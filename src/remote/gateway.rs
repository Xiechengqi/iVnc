use super::{
    media::{Lease, StopReason},
    store::{invalid_credentials, Target},
    worker, PROTOCOL,
};
use crate::{
    apps::{
        api::AppsState,
        app::{AppStatus, AppType},
        error::ApiError,
    },
    web::{http_server::HttpPeerAddr, SharedState},
};
use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket},
        Path, State, WebSocketUpgrade,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as UpMessage};

/// Views without an active connection expire after this idle period.
const VIEW_IDLE: Duration = Duration::from_secs(8 * 3600);

#[derive(Clone)]
pub struct View {
    pub app: String,
    target: Source,
    touched: Arc<Mutex<Instant>>,
    lifetime: Arc<Lease>,
    lease: Option<Arc<Lease>>,
}
impl View {
    fn expired(&self) -> bool {
        !self.lease.as_ref().is_some_and(|l| l.active())
            && self.touched.lock().unwrap().elapsed() >= VIEW_IDLE
    }
    fn stop(&self, reason: StopReason) {
        self.lifetime.stop_with(reason);
        if let Some(l) = &self.lease {
            l.stop_with(reason);
        }
    }
}
#[derive(Clone)]
enum Source {
    Local(worker::Endpoint),
    /// A saved connection and its epoch; see `RemoteHealthCache`.
    Remote(u64, Target),
}
#[derive(Default)]
pub struct Views {
    entries: Mutex<HashMap<String, View>>,
}
impl Views {
    pub fn revoke(&self, app: &str, remote_only: bool, reason: StopReason) {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, v| {
            let revoke = v.app == app && (!remote_only || matches!(v.target, Source::Remote(..)));
            if revoke {
                v.stop(reason);
            }
            !revoke
        });
    }
}

/// Last known state of a remote application, as seen from A.
#[derive(Clone, Debug)]
pub struct RemoteHealth {
    pub state: &'static str,
    /// Why the application is not connected.
    pub detail: Option<ApiError>,
    at: Instant,
}
/// What A knows about a remote application while one saved connection lasts.
struct Link {
    epoch: u64,
    health: Option<RemoteHealth>,
    /// The session token B issued, so that checks do not cost B a password hash each time.
    token: Option<String>,
    /// Why B refused the saved password. Until the connection is saved again,
    /// only the rare health check authenticates again.
    refused: Option<ApiError>,
    /// Serializes authentication, so that concurrent requests share one new token.
    login: Arc<tokio::sync::Mutex<()>>,
}

/// Remote applications as seen from A. Saving a connection, starting or stopping
/// the application begins a new epoch; work begun in an earlier epoch can no
/// longer record a state or a token.
#[derive(Default)]
pub struct RemoteHealthCache {
    links: Mutex<HashMap<String, Link>>,
    epochs: std::sync::atomic::AtomicU64,
    refreshing: Mutex<HashSet<String>>,
}
impl RemoteHealthCache {
    pub fn get(&self, app: &str) -> Option<RemoteHealth> {
        self.links.lock().unwrap().get(app)?.health.clone()
    }
    /// Runs `f` on the current link, beginning an epoch if there is none.
    fn link<R>(&self, app: &str, f: impl FnOnce(&mut Link) -> R) -> R {
        let mut links = self.links.lock().unwrap();
        let link = links.entry(app.to_owned()).or_insert_with(|| Link {
            epoch: self
                .epochs
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            health: None,
            token: None,
            refused: None,
            login: Default::default(),
        });
        f(link)
    }
    /// Runs `f` on the link only while it is still in `epoch`.
    fn current<R>(&self, app: &str, epoch: u64, f: impl FnOnce(&mut Link) -> R) -> Option<R> {
        self.links
            .lock()
            .unwrap()
            .get_mut(app)
            .filter(|l| l.epoch == epoch)
            .map(f)
    }
    fn epoch(&self, app: &str) -> u64 {
        self.link(app, |l| l.epoch)
    }
    /// Ends the epoch, e.g. when the connection changes.
    pub fn forget(&self, app: &str) {
        self.links.lock().unwrap().remove(app);
    }
    /// Keeps the token of a connection just saved.
    pub fn remember_token(&self, app: &str, token: String) {
        self.link(app, |l| l.token = Some(token));
    }
    fn token(&self, app: &str, epoch: u64) -> Option<String> {
        self.current(app, epoch, |l| l.token.clone()).flatten()
    }
    fn refused(&self, app: &str, epoch: u64) -> Option<ApiError> {
        self.current(app, epoch, |l| l.refused.clone()).flatten()
    }
    /// Returns whether the state changed.
    fn record(&self, app: &str, epoch: u64, state: &'static str, detail: Option<ApiError>) -> bool {
        self.current(app, epoch, |l| {
            let previous = l.health.replace(RemoteHealth {
                state,
                detail,
                at: Instant::now(),
            });
            previous.is_none_or(|h| h.state != state)
        })
        .unwrap_or(false)
    }
}

/// The saved connection of a remote application, with the epoch it belongs to.
/// The epoch is taken first: a connection saved meanwhile begins a later one.
fn connection(apps: &AppsState, id: &str) -> (u64, Result<Target, ApiError>) {
    let epoch = apps.remote_health.epoch(id);
    (epoch, apps.remote.target(id))
}

/// Refreshes stale remote states in the background; nothing waits on B. States
/// that need the user to act are checked rarely, so that a wrong saved password
/// does not exhaust B's limit on failed attempts. Only this check authenticates
/// again after B refused the saved password.
pub fn refresh_remote_health(apps: Arc<AppsState>, ids: Vec<String>) {
    for id in ids {
        let stale = apps.remote_health.get(&id).is_none_or(|h| {
            let interval = match h.state {
                "auth_required" | "incompatible" => Duration::from_secs(300),
                _ => Duration::from_secs(30),
            };
            h.at.elapsed() > interval
        });
        if !stale
            || !apps
                .remote_health
                .refreshing
                .lock()
                .unwrap()
                .insert(id.clone())
        {
            continue;
        }
        let apps = apps.clone();
        tokio::spawn(async move {
            let (epoch, target) = connection(&apps, &id);
            match target {
                Ok(target) => {
                    let _ = probe(&apps, &id, epoch, &target, true).await;
                }
                Err(e) => record(&apps, &id, epoch, "auth_required", Some(e)),
            }
            apps.remote_health.refreshing.lock().unwrap().remove(&id);
        });
    }
}

/// Records a remote state; the taskbar follows state changes.
fn record(apps: &AppsState, id: &str, epoch: u64, state: &'static str, detail: Option<ApiError>) {
    if apps.remote_health.record(id, epoch, state, detail) {
        apps.refresh_stream_apps();
    }
}

/// Failed-authentication windows per source and per source/application. IPv6
/// sources are counted per /64, since one host usually controls the whole prefix.
/// Attempts count as failures until they succeed, so concurrent guesses from one
/// source are bounded too. One source never locks out another.
#[derive(Default)]
struct Limiter {
    entries: Mutex<HashMap<String, (Instant, u32)>>,
}
impl Limiter {
    const WINDOW: Duration = Duration::from_secs(15 * 60);
    const CAPACITY: usize = 8192;
    fn source(ip: &str) -> String {
        match ip.parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V6(v6)) => {
                let s = v6.segments();
                format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
            }
            _ => ip.to_owned(),
        }
    }
    fn keys(ip: &str, app: &str) -> [(String, u32); 2] {
        let source = Self::source(ip);
        [
            (format!("ip {source}"), 30),
            (format!("app {source} {app}"), 8),
        ]
    }
    /// Records an attempt; returns false when the source must wait.
    fn begin(&self, ip: &str, app: &str) -> bool {
        let mut entries = self.entries.lock().unwrap();
        let keys = Self::keys(ip, app);
        let blocked = keys.iter().any(|(key, limit)| {
            entries
                .get(key)
                .is_some_and(|(start, count)| start.elapsed() < Self::WINDOW && count >= limit)
        });
        if blocked {
            return false;
        }
        if entries.len() >= Self::CAPACITY {
            entries.retain(|_, (start, _)| start.elapsed() < Self::WINDOW);
            while entries.len() >= Self::CAPACITY {
                let oldest = entries
                    .iter()
                    .min_by_key(|(_, (start, _))| *start)
                    .map(|(k, _)| k.clone());
                match oldest {
                    Some(key) => entries.remove(&key),
                    None => break,
                };
            }
        }
        for (key, _) in keys {
            let entry = entries.entry(key).or_insert((Instant::now(), 0));
            if entry.0.elapsed() >= Self::WINDOW {
                *entry = (Instant::now(), 0);
            }
            entry.1 += 1;
        }
        true
    }
    /// A successful attempt is not a failure and clears the source/application count.
    fn succeed(&self, ip: &str, app: &str) {
        let [(source_key, _), (app_key, _)] = Self::keys(ip, app);
        let mut entries = self.entries.lock().unwrap();
        entries.remove(&app_key);
        if let Some(entry) = entries.get_mut(&source_key) {
            entry.1 = entry.1.saturating_sub(1);
        }
    }
}

/// Serializes password checks per application and spaces out failures, bounding
/// guesses against one application from any number of sources to about one per second.
struct Pacing([tokio::sync::Mutex<()>; 64]);
impl Default for Pacing {
    fn default() -> Self {
        Self(std::array::from_fn(|_| tokio::sync::Mutex::new(())))
    }
}
impl Pacing {
    fn slot(&self, app: &str) -> &tokio::sync::Mutex<()> {
        let index = app
            .bytes()
            .fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize));
        &self.0[index % self.0.len()]
    }
}

struct Gateway {
    apps: Arc<AppsState>,
    shared: Arc<SharedState>,
    limiter: Limiter,
    pacing: Pacing,
    password_jobs: Arc<tokio::sync::Semaphore>,
}
pub fn router(apps: Arc<AppsState>, shared: Arc<SharedState>) -> Router {
    let _ = apps.shared.set(Arc::downgrade(&shared));
    let state = Arc::new(Gateway {
        apps,
        shared,
        limiter: Limiter::default(),
        pacing: Pacing::default(),
        password_jobs: Arc::new(tokio::sync::Semaphore::new(4)),
    });
    Router::new()
        .route("/api/apps/{id}/sharing", get(get_policy).put(set_policy))
        .route("/api/apps/{id}/view", post(create_view))
        .route("/api/remote/test", post(test_connection))
        .route("/remote/v1/apps/{id}/authenticate", post(authenticate))
        .route("/remote/v1/apps/{id}/session", delete(close_session))
        .route("/remote/v1/apps/{id}/info", get(remote_info))
        .route("/remote/v1/apps/{id}/socket", get(remote_socket))
        .route("/remote/v1/apps/{id}/data/{*path}", get(remote_data))
        .route("/app-view/{id}/", get(view_index))
        .route("/app-view/{id}/webrtc", get(view_socket))
        .route("/app-view/{id}/close", post(close_view))
        .route("/app-view/{id}/{*path}", get(view_data))
        .with_state(state)
}
fn error(status: StatusCode, error: impl Into<ApiError>) -> Response {
    (status, Json(error.into().body())).into_response()
}
fn bearer(headers: &HeaderMap) -> &str {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
}
fn peer(addr: &Option<Extension<HttpPeerAddr>>) -> String {
    addr.as_ref()
        .map(|a| a.0 .0.ip().to_canonical().to_string())
        .unwrap_or_default()
}

fn desktop(g: &Gateway, id: &str) -> Result<crate::apps::app::ManagedApp, Response> {
    let app = g.apps.store.get(id).map_err(|_| {
        error(
            StatusCode::NOT_FOUND,
            ApiError::new("app_not_found", "Application not found"),
        )
    })?;
    if app.app_type != AppType::DesktopApp {
        return Err(error(
            StatusCode::BAD_REQUEST,
            ApiError::new(
                "sharing_desktop_only",
                "Only local desktop applications support remote access",
            ),
        ));
    }
    Ok(app)
}
/// Verifies a session token of the application.
fn authorize(g: &Gateway, id: &str, headers: &HeaderMap) -> Result<(), Response> {
    desktop(g, id)?;
    if !g.apps.remote.verify(id, bearer(headers)) {
        return Err(error(
            StatusCode::UNAUTHORIZED,
            ApiError::new(
                "session_invalid",
                "Application session expired or remote access is disabled",
            ),
        ));
    }
    Ok(())
}
/// The runtime of an application that runs on its own. Access from another
/// instance also needs sharing to be enabled.
fn runtime_check(
    g: &Gateway,
    id: &str,
    remote: bool,
) -> Result<worker::Endpoint, (StatusCode, ApiError)> {
    match g.apps.process.status(id) {
        AppStatus::Running => {}
        AppStatus::Starting => {
            return Err((
                StatusCode::CONFLICT,
                ApiError::new("app_starting", "Application is starting"),
            ))
        }
        _ => {
            return Err((
                StatusCode::CONFLICT,
                ApiError::new("app_not_running", "Application is not running"),
            ))
        }
    }
    let enabled = !remote
        || g.apps
            .remote
            .policy(id)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.into()))?
            .enabled;
    match g.apps.process.in_worker(id) {
        Some(true) if enabled => worker::endpoint(id).map_err(|e| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiError::new("app_runtime_unavailable", e),
            )
        }),
        Some(false) if remote => Err((
            StatusCode::CONFLICT,
            ApiError::new(
                "app_restart_required",
                "Restart this application on B to apply its remote access setting",
            ),
        )),
        _ => Err((
            StatusCode::CONFLICT,
            ApiError::new(
                "sharing_disabled",
                "Remote access is not enabled for this application",
            ),
        )),
    }
}
fn runtime(g: &Gateway, id: &str, remote: bool) -> Result<worker::Endpoint, Response> {
    runtime_check(g, id, remote).map_err(|(status, message)| error(status, message))
}
async fn get_policy(State(g): State<Arc<Gateway>>, Path(id): Path<String>) -> Response {
    if let Err(e) = desktop(&g, &id) {
        return e;
    }
    let (policy, password) = match g
        .apps
        .remote
        .policy(&id)
        .and_then(|p| Ok((p, g.apps.remote.policy_password(&id)?)))
    {
        Ok(v) => v,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    Json(json!({
        "enabled": policy.enabled,
        "password_set": policy.password_set,
        "password_viewable": password.is_some(),
        "password": password,
        "share_path": super::share_path(&id),
        "restart_required": g.apps.restart_required(&id, policy.enabled),
    }))
    .into_response()
}
async fn set_policy(
    State(g): State<Arc<Gateway>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    if let Err(e) = desktop(&g, &id) {
        return e;
    }
    let enabled = match body["enabled"].as_bool() {
        Some(v) => v,
        None => {
            return error(
                StatusCode::BAD_REQUEST,
                ApiError::new("sharing_enabled_invalid", "enabled must be a boolean"),
            )
        }
    };
    let previous = match g.apps.remote.policy(&id) {
        Ok(policy) => policy,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let store = g.apps.remote.clone();
    let app = id.clone();
    let password = body["password"].as_str().map(str::to_owned);
    let result =
        tokio::task::spawn_blocking(move || store.set_policy(&app, enabled, password.as_deref()))
            .await;
    match result {
        Ok(Ok(policy)) => {
            // A new revision has already ended every session token; end their media too.
            let changed = policy.revision != previous.revision;
            if changed {
                g.shared.media_router.revoke(&id, true, StopReason::Revoked);
                log::info!("Remote access to {id} changed: enabled={}", policy.enabled);
            }
            let restart_required = g.apps.restart_required(&id, policy.enabled);
            Json(json!({"ok":true,"revoked":changed && previous.enabled,"restart_required":restart_required}))
                .into_response()
        }
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, e),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Password processing failed",
        ),
    }
}
async fn authenticate(
    State(g): State<Arc<Gateway>>,
    Path(id): Path<String>,
    addr: Option<Extension<HttpPeerAddr>>,
    Json(body): Json<Value>,
) -> Response {
    let ip = peer(&addr);
    if !super::valid_id(&id) {
        return error(StatusCode::UNAUTHORIZED, invalid_credentials());
    }
    if !g.limiter.begin(&ip, &id) {
        log::warn!("Remote authentication for {id} from {ip} is rate limited");
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            ApiError::new(
                "auth_rate_limited",
                "Too many failed attempts from this address; retry later",
            ),
        );
    }
    let password = body["password"].as_str().unwrap_or("").to_string();
    let busy = || {
        error(
            StatusCode::TOO_MANY_REQUESTS,
            ApiError::new("auth_busy", "Authentication is busy; retry shortly"),
        )
    };
    let Ok(_paced) = tokio::time::timeout(Duration::from_secs(10), g.pacing.slot(&id).lock()).await
    else {
        return busy();
    };
    // Argon2 work is bounded; requests wait briefly instead of failing at once.
    let permit = match tokio::time::timeout(
        Duration::from_secs(10),
        g.password_jobs.clone().acquire_owned(),
    )
    .await
    {
        Ok(Ok(p)) => p,
        _ => return busy(),
    };
    let store = g.apps.remote.clone();
    let app = id.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        store.authenticate(&app, &password)
    })
    .await;
    match (result, desktop(&g, &id)) {
        (Ok(Ok(token)), Ok(app)) => {
            g.limiter.succeed(&ip, &id);
            Json(json!({"protocol":PROTOCOL,"node_id":g.apps.remote.node_id,"app_id":id,"name":app.name,"token":token})).into_response()
        }
        (Ok(Ok(_)), Err(_)) => error(StatusCode::UNAUTHORIZED, invalid_credentials()),
        (Ok(Err(e)), _) => {
            if e.code == "auth_invalid" {
                log::info!("Remote authentication for {id} from {ip} failed");
                // Keep this application's slot busy so failures stay spaced out.
                tokio::time::sleep(Duration::from_secs(1)).await;
                error(StatusCode::UNAUTHORIZED, e)
            } else {
                error(StatusCode::CONFLICT, e)
            }
        }
        (Err(_), _) => error(StatusCode::INTERNAL_SERVER_ERROR, "Authentication failed"),
    }
}
/// Ends the caller's own session, e.g. one opened only to test a connection.
async fn close_session(
    State(g): State<Arc<Gateway>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    g.apps.remote.close_session(&id, bearer(&headers));
    Json(json!({"ok":true})).into_response()
}
async fn remote_info(
    State(g): State<Arc<Gateway>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = authorize(&g, &id, &headers) {
        return e;
    }
    let app = match desktop(&g, &id) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let status = format!("{:?}", g.apps.app_status(&app)).to_lowercase();
    let (ready, detail) = match runtime_check(&g, &id, true) {
        Ok(_) => (true, String::new()),
        Err((_, e)) => (false, e.message),
    };
    Json(json!({"protocol":PROTOCOL,"node_id":g.apps.remote.node_id,"app_id":id,"name":app.name,"status":status,"ready":ready,"detail":detail})).into_response()
}

/// One shared TLS client configuration: system roots plus `IVNC_REMOTE_CA_FILE`,
/// used by both HTTPS and WSS so they always trust the same certificates. It is
/// rebuilt when the CA file changes; failures are returned, never cached.
fn tls_client() -> Result<(Arc<rustls::ClientConfig>, reqwest::Client), String> {
    type Cached = (
        Option<std::time::SystemTime>,
        Arc<rustls::ClientConfig>,
        reqwest::Client,
    );
    static CACHE: Mutex<Option<Cached>> = Mutex::new(None);
    let version = std::env::var_os("IVNC_REMOTE_CA_FILE")
        .and_then(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok());
    let mut cache = CACHE.lock().unwrap();
    if let Some((cached, config, client)) = cache.as_ref() {
        if *cached == version {
            return Ok((config.clone(), client.clone()));
        }
    }
    let config = build_tls_config(trusted_certificates()?)?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .use_preconfigured_tls(config.clone())
        .build()
        .map_err(|e| e.to_string())?;
    let config = Arc::new(config);
    *cache = Some((version, config.clone(), client.clone()));
    Ok((config, client))
}
fn tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
    tls_client().map(|(config, _)| config)
}
pub fn http_client() -> Result<reqwest::Client, ApiError> {
    tls_client()
        .map(|(_, client)| client)
        .map_err(|e| ApiError::from(e).or("remote_tls_config"))
}
pub fn parse_share_url(value: &str) -> Result<(String, String), ApiError> {
    let url = reqwest::Url::parse(value.trim())
        .map_err(|_| ApiError::new("remote_url_invalid", "Invalid application connection URL"))?;
    if !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ApiError::new(
            "remote_url_unsupported",
            "Use an HTTP(S) application URL without credentials, query or fragment",
        ));
    }
    let id = url
        .path()
        .trim_end_matches('/')
        .strip_prefix("/remote/v1/apps/")
        .filter(|s| super::valid_id(s))
        .ok_or_else(|| {
            ApiError::new(
                "remote_url_not_share",
                "Use the application sharing URL copied from B",
            )
        })?
        .to_owned();
    Ok((url.origin().ascii_serialization(), id))
}
fn remote_url(target: &Target, path: &str) -> String {
    format!("{}/{path}", target.share_url())
}

/// Why B could not serve a remote application, in a form A can show and cache.
#[derive(Debug)]
pub struct Probe {
    pub state: &'static str,
    pub error: ApiError,
}
fn unavailable(error: ApiError) -> Probe {
    Probe {
        state: "unreachable",
        error,
    }
}
type Cause<'a> = &'a (dyn std::error::Error + 'static);
/// A request to B that failed before B answered. reqwest's own text names only
/// the URL, so the message and the `detail` give every cause; the code tells
/// common causes apart.
fn request_failed(context: &str, e: reqwest::Error) -> Probe {
    let causes = causes(&e);
    let mut detail = String::new();
    for cause in &causes {
        // Some errors repeat the text of their cause.
        let text = cause.to_string();
        if !detail.contains(&text) {
            if !detail.is_empty() {
                detail.push_str(": ");
            }
            detail.push_str(&text);
        }
    }
    let any = |f: fn(Cause) -> bool| causes.iter().any(|c| f(*c));
    let code = if any(|c| matches!(c.downcast_ref(), Some(rustls::Error::InvalidCertificate(_)))) {
        "remote_certificate"
    } else if e.is_timeout() {
        "remote_timeout"
    } else if any(|c| {
        c.downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::ConnectionRefused)
    }) {
        "remote_refused"
    } else if any(|c| c.to_string().starts_with("dns error")) {
        "remote_dns"
    } else {
        "remote_unreachable"
    };
    unavailable(ApiError::new(code, format!("{context}: {detail}")).param("detail", detail))
}
/// An error and its causes, outermost first. I/O errors are opened with
/// `get_ref`: their `source` skips the error they wrap, e.g. a rustls error.
fn causes(e: Cause) -> Vec<Cause> {
    let mut causes = vec![e];
    let mut current = e;
    while let Some(next) = match current.downcast_ref::<std::io::Error>() {
        Some(io) => io.get_ref().map(|e| e as _),
        None => current.source(),
    } {
        causes.push(next);
        current = next;
    }
    causes
}

/// A session on B, as issued by its authentication endpoint.
struct Session {
    token: String,
    node_id: String,
    name: String,
}
async fn open_session(base_url: &str, app_id: &str, password: &str) -> Result<Session, Probe> {
    let response = http_client()
        .map_err(unavailable)?
        .post(format!(
            "{base_url}{}/authenticate",
            super::share_path(app_id)
        ))
        .json(&json!({ "password": password }))
        .send()
        .await
        .map_err(|e| request_failed("Cannot connect to B", e))?;
    let status = response.status();
    if !status.is_success() {
        let detail = bounded_json(response)
            .await
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_owned))
            .unwrap_or_default();
        let message = format!("B rejected application authentication ({status}): {detail}");
        let refused = status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN;
        let error = if refused {
            ApiError::new("remote_auth_failed", message)
        } else if status == StatusCode::TOO_MANY_REQUESTS {
            ApiError::new("remote_rate_limited", message)
        } else {
            ApiError::new("remote_rejected", message)
                .param("status", status.as_u16())
                .param("detail", detail)
        };
        return Err(Probe {
            state: if refused {
                "auth_required"
            } else {
                "unreachable"
            },
            error,
        });
    }
    let value = bounded_json(response).await.map_err(unavailable)?;
    let field = |name: &str| {
        value[name]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    match (
        value["protocol"].as_u64() == Some(PROTOCOL as u64)
            && value["app_id"].as_str() == Some(app_id),
        field("token").filter(|t| t.len() == 64),
        field("node_id"),
        field("name"),
    ) {
        (true, Some(token), Some(node_id), Some(name)) => Ok(Session {
            token,
            node_id,
            name,
        }),
        _ => Err(Probe {
            state: "incompatible",
            error: ApiError::new("remote_incompatible", "B does not support this application sharing protocol; run the same iVnc version on both instances"),
        }),
    }
}
/// Authenticates against B for a new or edited connection. `own_node` prevents
/// mapping an instance onto itself, which would register the same media session
/// twice in one router. Returns the connection and its first session token.
pub async fn connect_target(body: &Value, own_node: &str) -> Result<(Target, String), ApiError> {
    let (base_url, app_id) = parse_share_url(
        body["url"]
            .as_str()
            .ok_or_else(|| ApiError::new("remote_url_required", "Missing application URL"))?,
    )?;
    let password = body["password"]
        .as_str()
        .filter(|p| !p.is_empty() && p.len() <= 256)
        .ok_or_else(|| {
            ApiError::new("remote_password_required", "Enter the application password")
        })?;
    let session = open_session(&base_url, &app_id, password)
        .await
        .map_err(|e| e.error)?;
    if session.node_id == own_node {
        return Err(ApiError::new(
            "remote_self",
            "This address is the current instance; open the local application directly",
        ));
    }
    let target = Target {
        base_url,
        app_id,
        node_id: session.node_id,
        name: session.name,
        password: password.to_owned(),
    };
    Ok((target, session.token))
}
/// A session token for B: the cached one, or a new one when there is none or
/// B no longer accepts `rejected`. Concurrent callers share one authentication.
/// After B refused the saved password, only `retry` authenticates again; other
/// requests fail at once instead of spending B's limit on failed attempts.
/// Returns whether the token is new.
async fn session_token(
    apps: &AppsState,
    id: &str,
    epoch: u64,
    target: &Target,
    rejected: Option<&str>,
    retry: bool,
) -> Result<(String, bool), Probe> {
    let cache = &apps.remote_health;
    let cached = || {
        cache
            .token(id, epoch)
            .filter(|t| Some(t.as_str()) != rejected)
    };
    if let Some(token) = cached() {
        return Ok((token, false));
    }
    let login = cache.link(id, |l| l.login.clone());
    let _login = login.lock().await;
    // Another request may have authenticated meanwhile.
    if let Some(token) = cached() {
        return Ok((token, true));
    }
    let refused = |error: ApiError| Probe {
        state: "auth_required",
        error,
    };
    if target.password.is_empty() {
        return Err(refused(ApiError::new("remote_legacy_password", "This connection was saved by an earlier version; edit it and enter the application password again")));
    }
    if let Some(error) = cache.refused(id, epoch).filter(|_| !retry) {
        return Err(refused(error));
    }
    let session = match open_session(&target.base_url, &target.app_id, &target.password).await {
        Ok(session) => session,
        Err(e) => {
            if e.state == "auth_required" {
                cache.current(id, epoch, |l| {
                    l.token = None;
                    l.refused = Some(e.error.clone());
                });
                // The health check now waits before it tries the password again.
                record(apps, id, epoch, e.state, Some(e.error.clone()));
            }
            return Err(e);
        }
    };
    if session.node_id != target.node_id {
        return Err(Probe {
            state: "incompatible",
            error: ApiError::new(
                "remote_identity_changed",
                "Remote instance identity changed; edit the connection to trust it again",
            ),
        });
    }
    cache.current(id, epoch, |l| {
        l.token = Some(session.token.clone());
        l.refused = None;
    });
    Ok((session.token, true))
}
/// Sends a request to B with a session token, authenticating again once when B
/// no longer accepts the cached one.
async fn send_remote(
    apps: &AppsState,
    id: &str,
    epoch: u64,
    target: &Target,
    retry: bool,
    request: impl Fn(&reqwest::Client, &str) -> reqwest::RequestBuilder,
) -> Result<reqwest::Response, Probe> {
    let client = http_client().map_err(unavailable)?;
    let (mut token, mut fresh) = session_token(apps, id, epoch, target, None, retry).await?;
    loop {
        let response = request(&client, &token)
            .send()
            .await
            .map_err(|e| request_failed("B is unavailable", e))?;
        if response.status() != StatusCode::UNAUTHORIZED || fresh {
            return Ok(response);
        }
        (token, fresh) = session_token(apps, id, epoch, target, Some(&token), retry).await?;
    }
}
/// Interprets B's answer to an application information request.
async fn read_info(response: reqwest::Response, target: &Target) -> Result<Value, Probe> {
    if response.status() == StatusCode::UNAUTHORIZED || response.status() == StatusCode::NOT_FOUND {
        return Err(Probe {
            state: "auth_required",
            error: ApiError::new("remote_access_revoked", "B disabled sharing or changed the application password; edit the connection and enter the password again"),
        });
    }
    if !response.status().is_success() {
        return Err(unavailable(
            ApiError::new(
                "remote_http_error",
                format!("B returned {}", response.status()),
            )
            .param("status", response.status().as_u16()),
        ));
    }
    let value = bounded_json(response).await.map_err(unavailable)?;
    if value["protocol"].as_u64() != Some(PROTOCOL as u64)
        || value["node_id"].as_str() != Some(&target.node_id)
        || value["app_id"].as_str() != Some(&target.app_id)
    {
        return Err(Probe {
            state: "incompatible",
            error: ApiError::new(
                "remote_identity_changed",
                "Remote instance identity or application protocol changed",
            ),
        });
    }
    Ok(value)
}
/// Checks B and records the result for the application list and the taskbar.
/// Anything but a ready application is an error. See `session_token` for `retry`.
async fn probe(
    apps: &AppsState,
    id: &str,
    epoch: u64,
    target: &Target,
    retry: bool,
) -> Result<(), Probe> {
    // A refusal already recorded stays recorded as it was, so that the health
    // check keeps its interval.
    if let Some(error) = apps.remote_health.refused(id, epoch).filter(|_| !retry) {
        return Err(Probe {
            state: "auth_required",
            error,
        });
    }
    let failed = |state, error: ApiError| Err(Probe { state, error });
    let result = match send_remote(apps, id, epoch, target, retry, |client, token| {
        client.get(remote_url(target, "info")).bearer_auth(token)
    })
    .await
    {
        Ok(response) => read_info(response, target).await,
        Err(e) => Err(e),
    }
    .and_then(|v| {
        if v["status"] == "starting" {
            failed(
                "starting",
                ApiError::new("remote_app_starting", "The application is starting on B"),
            )
        } else if v["status"] != "running" {
            failed(
                "stopped",
                ApiError::new("remote_app_stopped", "The application is not running on B"),
            )
        } else if v["ready"] != true {
            // B explains in English why its application is not ready.
            let detail = v["detail"].as_str().unwrap_or("");
            failed(
                "not_ready",
                ApiError::new("remote_app_not_ready", detail).param("detail", detail),
            )
        } else {
            Ok(())
        }
    });
    match &result {
        Ok(()) => record(apps, id, epoch, "connected", None),
        Err(e) => record(apps, id, epoch, e.state, Some(e.error.clone())),
    }
    result
}
/// Ends A's views of a remote application whose saved password B refuses, so
/// that they stop reconnecting and polling until the connection is saved again.
fn end_refused_views(g: &Gateway, app: &str) {
    g.shared.app_views.revoke(app, true, StopReason::Revoked);
}
async fn create_view(State(g): State<Arc<Gateway>>, Path(id): Path<String>) -> Response {
    let app = match g.apps.store.get(&id) {
        Ok(a) => a,
        Err(e) => return error(StatusCode::NOT_FOUND, e),
    };
    let source = match app.app_type {
        AppType::DesktopApp => match runtime(&g, &id, false) {
            Ok(e) => Source::Local(e),
            Err(e) => return e,
        },
        AppType::RemoteApp => {
            if !g.apps.remote_started(&id) {
                return error(
                    StatusCode::CONFLICT,
                    ApiError::new("remote_not_started", "Start this remote application first"),
                );
            }
            let (epoch, target) = connection(&g.apps, &id);
            let target = match target {
                Ok(t) => t,
                Err(e) => return error(StatusCode::BAD_REQUEST, e),
            };
            if let Err(e) = probe(&g.apps, &id, epoch, &target, false).await {
                let status = match e.state {
                    "stopped" | "starting" | "not_ready" => StatusCode::CONFLICT,
                    _ => StatusCode::BAD_GATEWAY,
                };
                return error(status, e.error);
            }
            Source::Remote(epoch, target)
        }
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                ApiError::new(
                    "view_desktop_only",
                    "Only desktop applications have remote views",
                ),
            )
        }
    };
    let view = super::token();
    let mut entries = g.shared.app_views.entries.lock().unwrap();
    entries.retain(|_, v| {
        let expired = v.expired();
        if expired {
            v.stop(StopReason::Closed);
        }
        !expired
    });
    if entries.len() >= 128 {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            ApiError::new("too_many_views", "Too many application views"),
        );
    }
    entries.insert(
        view.clone(),
        View {
            lifetime: Lease::new(id.clone(), false),
            app: id,
            target: source,
            touched: Arc::new(Mutex::new(Instant::now())),
            lease: None,
        },
    );
    Json(json!({"url":format!("/app-view/{view}/"),"name":app.name})).into_response()
}
fn view(g: &Gateway, id: &str) -> Result<View, Response> {
    g.shared
        .app_views
        .entries
        .lock()
        .unwrap()
        .get(id)
        .filter(|v| !v.expired() && v.lifetime.active())
        .inspect(|v| *v.touched.lock().unwrap() = Instant::now())
        .cloned()
        .ok_or_else(|| {
            error(
                StatusCode::GONE,
                ApiError::new(
                    "view_expired",
                    "Application view expired; open it again from the application list",
                ),
            )
        })
}
async fn view_index(State(g): State<Arc<Gateway>>, Path(id): Path<String>) -> Response {
    if let Err(e) = view(&g, &id) {
        return e;
    }
    crate::web::embedded_assets::get_embedded_file("index.html")
}
async fn close_view(State(g): State<Arc<Gateway>>, Path(id): Path<String>) -> Response {
    let removed = g.shared.app_views.entries.lock().unwrap().remove(&id);
    if let Some(v) = removed {
        v.stop(StopReason::Closed);
    }
    Json(json!({"ok":true})).into_response()
}
/// Runtime documents a view loads from the application's own server.
fn data_path(path: &str) -> bool {
    matches!(path, "ui-config" | "ws-config" | "api/render-status")
}
/// Relays one small runtime document. The request is bounded and ends early
/// when access ends.
async fn fetch_data(
    guard: AccessLease,
    request: impl Future<Output = Result<reqwest::Response, String>>,
) -> Response {
    let result = tokio::select! {
        _ = guard.cancelled() => {
            return error(
                StatusCode::GONE,
                ApiError::new("access_ended", "Application access ended"),
            )
        }
        result = tokio::time::timeout(Duration::from_secs(30), async {
            let response = request.await?;
            let status = response.status();
            let content_type = response.headers().get(header::CONTENT_TYPE).cloned();
            Ok::<_, String>((status, content_type, bounded_body(response, 1 << 20).await?))
        }) => result,
    };
    match result {
        Ok(Ok((status, content_type, body))) => {
            let mut builder = Response::builder()
                .status(status)
                .header(header::CACHE_CONTROL, "no-store");
            if let Some(value) = content_type {
                builder = builder.header(header::CONTENT_TYPE, value);
            }
            builder.body(Body::from(body)).unwrap()
        }
        Ok(Err(_)) => error(
            StatusCode::BAD_GATEWAY,
            ApiError::new(
                "app_runtime_unavailable",
                "Application runtime is unavailable",
            ),
        ),
        Err(_) => error(
            StatusCode::GATEWAY_TIMEOUT,
            ApiError::new(
                "app_runtime_no_response",
                "Application runtime did not respond",
            ),
        ),
    }
}
fn local_data(
    endpoint: &worker::Endpoint,
    path: &str,
) -> impl Future<Output = Result<reqwest::Response, String>> {
    let request = http_client().map(|client| {
        client
            .get(format!("http://127.0.0.1:{}/{path}", endpoint.port))
            .basic_auth("worker", Some(&endpoint.password))
    });
    async move { request?.send().await.map_err(|e| e.to_string()) }
}
async fn view_data(
    State(g): State<Arc<Gateway>>,
    Path((id, path)): Path<(String, String)>,
) -> Response {
    let v = match view(&g, &id) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if data_path(&path) {
        let guard = AccessLease {
            lease: v.lifetime.clone(),
            owned: false,
            authorization: None,
        };
        return match &v.target {
            Source::Local(endpoint) => fetch_data(guard, local_data(endpoint, &path)).await,
            Source::Remote(epoch, target) => {
                let url = remote_url(target, &format!("data/{path}"));
                let request = async {
                    let result =
                        send_remote(&g.apps, &v.app, *epoch, target, false, |client, token| {
                            client.get(&url).bearer_auth(token)
                        })
                        .await;
                    // Ending the view also ends this request with 410, which
                    // stops the page from polling.
                    match result {
                        Ok(r) if r.status() == StatusCode::UNAUTHORIZED => {
                            end_refused_views(&g, &v.app);
                            Ok(r)
                        }
                        Err(e) if e.state == "auth_required" => {
                            end_refused_views(&g, &v.app);
                            Err(e.error.message)
                        }
                        result => result.map_err(|e| e.error.message),
                    }
                };
                fetch_data(guard, request).await
            }
        };
    }
    // Only trusted A assets are embedded. No HTML or JavaScript is loaded from B.
    if path.contains("..")
        || path.starts_with("api/")
        || path == "console"
        || path == "console.html"
        || path == "console.js"
    {
        return error(
            StatusCode::NOT_FOUND,
            ApiError::new("not_found", "Not found"),
        );
    }
    crate::web::embedded_assets::get_embedded_file(&path)
}
async fn remote_data(
    State(g): State<Arc<Gateway>>,
    Path((id, path)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = authorize(&g, &id, &headers) {
        return e;
    }
    if !data_path(&path) {
        return error(
            StatusCode::NOT_FOUND,
            ApiError::new("not_found", "Not found"),
        );
    }
    let endpoint = match runtime(&g, &id, true) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let lease = Lease::new(id.clone(), true);
    if let Err(e) = g.shared.media_router.track(&lease) {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            ApiError::new("too_many_sessions", e),
        );
    }
    fetch_data(
        AccessLease {
            lease,
            owned: true,
            authorization: Some((g.apps.remote.clone(), id, bearer(&headers).to_owned())),
        },
        local_data(&endpoint, &path),
    )
    .await
}
async fn view_socket(
    State(g): State<Arc<Gateway>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !same_origin(&headers) {
        return error(
            StatusCode::FORBIDDEN,
            ApiError::new("origin_invalid", "Invalid WebSocket origin"),
        );
    }
    let v = match view(&g, &id) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if let Source::Local(e) = &v.target {
        match runtime(&g, &v.app, false) {
            Ok(now) if now.generation == e.generation => {}
            _ => {
                return error(
                    StatusCode::CONFLICT,
                    ApiError::new("view_restarted", "Application restarted; open a new view"),
                )
            }
        }
    }
    let lease = Lease::new(v.app.clone(), matches!(v.target, Source::Remote(..)));
    if let Err(e) = g.shared.media_router.track(&lease) {
        lease.stop();
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            ApiError::new("too_many_sessions", e),
        );
    }
    {
        let mut entries = g.shared.app_views.entries.lock().unwrap();
        let Some(entry) = entries.get_mut(&id) else {
            lease.stop();
            return error(
                StatusCode::GONE,
                ApiError::new("view_expired", "View expired"),
            );
        };
        if let Some(old) = entry.lease.replace(lease.clone()) {
            old.stop();
        }
    }
    // If the upgrade never completes, the callback is dropped and so is the guard.
    let pending = StopOnDrop(lease.clone());
    ws.max_message_size(256 * 1024)
        .on_upgrade(move |socket| async move {
            let _pending = pending;
            bridge(g, socket, v.target, lease, None).await
        })
        .into_response()
}
async fn remote_socket(
    State(g): State<Arc<Gateway>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if let Err(e) = authorize(&g, &id, &headers) {
        return e;
    }
    let endpoint = match runtime(&g, &id, true) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let lease = Lease::new(id.clone(), true);
    let token = bearer(&headers).to_string();
    if let Err(e) = g.shared.media_router.track(&lease) {
        lease.stop();
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            ApiError::new("too_many_sessions", e),
        );
    }
    // If the upgrade never completes, the callback is dropped and so is the guard.
    let pending = StopOnDrop(lease.clone());
    ws.max_message_size(256 * 1024)
        .on_upgrade(move |socket| async move {
            let _pending = pending;
            bridge(g, socket, Source::Local(endpoint), lease, Some((id, token))).await
        })
        .into_response()
}
pub fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    reqwest::Url::parse(origin)
        .ok()
        .and_then(|u| {
            u.host_str().map(|h| match u.port() {
                Some(p) => format!("{h}:{p}"),
                None => h.to_string(),
            })
        })
        .is_some_and(|h| h == host)
}
async fn bridge(
    g: Arc<Gateway>,
    mut client: WebSocket,
    source: Source,
    lease: Arc<Lease>,
    authorization: Option<(String, String)>,
) {
    let guard = AccessLease {
        lease: lease.clone(),
        owned: true,
        authorization: authorization.map(|(app, token)| (g.apps.remote.clone(), app, token)),
    };
    let result = tokio::select! {
        result = bridge_inner(&g, &mut client, &source, &lease) => result,
        _ = guard.cancelled() => Ok(()),
    };
    let reason = lease.stop_reason();
    lease.stop();
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        // Clients reconnect automatically only after an ordinary close.
        if let Some(reason) = reason.filter(|r| *r != StopReason::Closed) {
            let _ = client
                .send(Message::Text(
                    json!({"ivnc_revoked":true,"reason":reason.as_str()})
                        .to_string()
                        .into(),
                ))
                .await;
        }
        if let Err(message) = result {
            let _ = client
                .send(Message::Text(format!("ERROR {message}").into()))
                .await;
        }
        let _ = client.close().await;
    })
    .await;
}

async fn send_signal<S, M>(socket: &mut S, message: M) -> Result<(), String>
where
    S: futures::Sink<M> + Unpin,
    S::Error: std::fmt::Display,
{
    tokio::time::timeout(Duration::from_secs(10), socket.send(message))
        .await
        .map_err(|_| "Application signaling timed out")?
        .map_err(|e| e.to_string())
}

/// Explains a refused upstream WebSocket handshake using B's status and message.
fn handshake_error(error: tokio_tungstenite::tungstenite::Error) -> String {
    use tokio_tungstenite::tungstenite::Error;
    match error {
        Error::Http(response) => {
            let detail = response
                .body()
                .as_deref()
                .and_then(|b| serde_json::from_slice::<Value>(b).ok())
                .and_then(|v| v["error"].as_str().map(str::to_owned));
            let fallback = match response.status() {
                StatusCode::UNAUTHORIZED => "B ended this session or disabled sharing",
                StatusCode::CONFLICT => "the application is not running or must be restarted on B",
                StatusCode::TOO_MANY_REQUESTS => "B has reached its session limit",
                _ => "B refused the application connection",
            };
            format!(
                "{} ({})",
                detail.as_deref().unwrap_or(fallback),
                response.status()
            )
        }
        other => format!("Application connection failed: {other}"),
    }
}
async fn bridge_inner(
    g: &Gateway,
    client: &mut WebSocket,
    source: &Source,
    lease: &Arc<Lease>,
) -> Result<(), String> {
    let (url, auth, destination) = match source {
        Source::Local(e) => {
            use base64::Engine;
            (
                format!("ws://127.0.0.1:{}/webrtc", e.port),
                format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD
                        .encode(format!("worker:{}", e.password))
                ),
                format!("127.0.0.1:{}", e.port),
            )
        }
        Source::Remote(epoch, t) => {
            // The check also renews the session token when B no longer accepts it.
            let checked = match probe(&g.apps, &lease.app, *epoch, t, false).await {
                Ok(()) => session_token(&g.apps, &lease.app, *epoch, t, None, false)
                    .await
                    .map(|(token, _)| token),
                Err(e) => Err(e),
            };
            let token = checked.map_err(|e| {
                if e.state == "auth_required" {
                    // The client is told the view was revoked and stops reconnecting.
                    end_refused_views(g, &lease.app);
                }
                e.error.message
            })?;
            let mut url =
                reqwest::Url::parse(&remote_url(t, "socket")).map_err(|e| e.to_string())?;
            let destination = format!(
                "{}:{}",
                url.host_str().ok_or("Missing remote host")?,
                url.port_or_known_default().unwrap_or(443)
            );
            url.set_scheme(if url.scheme() == "https" { "wss" } else { "ws" })
                .map_err(|_| "Invalid scheme")?;
            (url.to_string(), format!("Bearer {token}"), destination)
        }
    };
    let mut request = url.into_client_request().map_err(|e| e.to_string())?;
    request.headers_mut().insert(
        header::AUTHORIZATION,
        auth.parse().map_err(|_| "Invalid upstream credentials")?,
    );
    let (mut upstream, _) = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::connect_async_tls_with_config(
            request,
            Some(tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
                max_message_size: Some(256 * 1024),
                max_frame_size: Some(256 * 1024),
                ..Default::default()
            }),
            true,
            Some(tokio_tungstenite::Connector::Rustls(tls_config()?)),
        ),
    )
    .await
    .map_err(|_| "Application connection timed out")?
    .map_err(handshake_error)?;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _=lease.cancelled()=>break,
            _=tick.tick()=>{
                if let Source::Local(e)=source{if worker::endpoint(&lease.app).map_or(true,|now|now.generation!=e.generation){return Err("Application stopped or restarted".into());}}
            },
            msg=client.recv()=>match msg{
                Some(Ok(Message::Text(text)))=>{send_signal(&mut upstream, UpMessage::Text(text.to_string())).await?;},
                Some(Ok(Message::Ping(p)))=>{send_signal(client, Message::Pong(p)).await?;},
                Some(Ok(Message::Close(_)))|None|Some(Err(_))=>break,
                _=>{},
            },
            msg=upstream.next()=>match msg{
                Some(Ok(UpMessage::Text(text)))=>{
                    if let Ok(value)=serde_json::from_str::<Value>(&text){
                        let sdp=value["sdp"].as_str().or_else(||value["sdp"]["sdp"].as_str());
                        if let Some(sdp)=sdp{g.shared.media_router.register_sdp(sdp,&destination,lease)?;}
                    }
                    send_signal(client, Message::Text(text.into())).await?;
                },
                Some(Ok(UpMessage::Ping(p)))=>{send_signal(&mut upstream, UpMessage::Pong(p)).await?;},
                Some(Ok(UpMessage::Close(_)))|None|Some(Err(_))=>break,
                _=>{},
            },
        }
    }
    Ok(())
}

async fn test_connection(State(g): State<Arc<Gateway>>, Json(body): Json<Value>) -> Response {
    let (target, token) = match connect_target(&body, &g.apps.remote.node_id).await {
        Ok(v) => v,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    let client = match http_client() {
        Ok(c) => c,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let result = match client
        .get(remote_url(&target, "info"))
        .bearer_auth(&token)
        .send()
        .await
    {
        Ok(response) => read_info(response, &target).await,
        Err(e) => Err(request_failed("B is unavailable", e)),
    };
    // The session was only for this test; B need not keep it.
    let _ = client
        .delete(remote_url(&target, "session"))
        .bearer_auth(&token)
        .timeout(Duration::from_secs(5))
        .send()
        .await;
    match result {
        Ok(info) => Json(info).into_response(),
        Err(e) => error(StatusCode::BAD_GATEWAY, e.error),
    }
}

fn trusted_certificates() -> Result<Vec<rustls::pki_types::CertificateDer<'static>>, String> {
    use rustls::pki_types::pem::PemObject;
    let Some(path) = std::env::var_os("IVNC_REMOTE_CA_FILE") else {
        return Ok(Vec::new());
    };
    let certs = rustls::pki_types::CertificateDer::pem_file_iter(path)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if certs.is_empty() {
        return Err("IVNC_REMOTE_CA_FILE contains no certificates".into());
    }
    Ok(certs)
}
fn build_tls_config(
    extra: Vec<rustls::pki_types::CertificateDer<'static>>,
) -> Result<rustls::ClientConfig, String> {
    let mut roots = rustls::RootCertStore::empty();
    let (_, ignored) =
        roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
    if ignored > 0 {
        log::debug!("Ignored {ignored} unusable system certificates");
    }
    for cert in extra {
        roots.add(cert).map_err(|e| e.to_string())?;
    }
    // TLS builds may enable both ring and aws-lc through dependencies. Always
    // choose a provider, including when only the system CA store is used.
    Ok(rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| e.to_string())?
    .with_root_certificates(roots)
    .with_no_client_auth())
}

/// Ends a lease when dropped, e.g. with a WebSocket callback that never ran.
struct StopOnDrop(Arc<Lease>);
impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop();
    }
}

struct AccessLease {
    lease: Arc<Lease>,
    owned: bool,
    authorization: Option<(Arc<super::store::RemoteStore>, String, String)>,
}
impl AccessLease {
    /// Completes when access ends, including when its session token is no
    /// longer valid. Access changes are signalled by the store; the slow timer
    /// only catches expiry.
    async fn cancelled(&self) {
        let Some((store, app, token)) = &self.authorization else {
            return self.lease.cancelled().await;
        };
        let mut changes = store.subscribe();
        loop {
            if !store.verify(app, token) {
                self.lease.stop_with(StopReason::Revoked);
                return;
            }
            tokio::select! {
                _ = self.lease.cancelled() => return,
                _ = changes.changed() => {},
                _ = tokio::time::sleep(Duration::from_secs(300)) => {},
            }
        }
    }
}
impl Drop for AccessLease {
    fn drop(&mut self) {
        if self.owned {
            self.lease.stop();
        }
    }
}
async fn bounded_body(response: reqwest::Response, limit: usize) -> Result<Vec<u8>, ApiError> {
    let mut stream = response.bytes_stream();
    let mut data = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| ApiError::from(e.to_string()).or("remote_unreachable"))?;
        if data.len() + chunk.len() > limit {
            return Err(ApiError::new(
                "remote_response_too_large",
                "Remote application response is too large",
            ));
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}
async fn bounded_json(response: reqwest::Response) -> Result<Value, ApiError> {
    let data = bounded_body(response, 65536).await?;
    serde_json::from_slice(&data).map_err(|_| {
        ApiError::new(
            "remote_invalid_response",
            "Invalid remote application metadata",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A TLS server identity for 127.0.0.1 that no system root trusts.
    #[cfg(feature = "tls")]
    fn self_signed() -> (
        tokio_rustls::TlsAcceptor,
        rustls::pki_types::CertificateDer<'static>,
    ) {
        let certificate = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
        let cert = certificate.cert.der().clone();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(certificate.key_pair.serialize_der())
                .into(),
        )
        .unwrap();
        (tokio_rustls::TlsAcceptor::from(Arc::new(config)), cert)
    }

    #[cfg(feature = "tls")]
    #[tokio::test]
    async fn wss_uses_explicit_crypto_provider_and_validates_trust() {
        let (acceptor, cert) = self_signed();

        // No private CA must produce a certificate error, even when both ring
        // and aws-lc are enabled. Adding the CA must permit a real WSS exchange.
        for trust_certificate in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("wss://{}/", listener.local_addr().unwrap());
            let extra = if trust_certificate {
                vec![cert.clone()]
            } else {
                Vec::new()
            };
            let connector =
                tokio_tungstenite::Connector::Rustls(Arc::new(build_tls_config(extra).unwrap()));
            let server = async {
                let (stream, _) = listener.accept().await.unwrap();
                let stream = acceptor.accept(stream).await;
                if trust_certificate {
                    let mut socket = tokio_tungstenite::accept_async(stream.unwrap())
                        .await
                        .unwrap();
                    socket
                        .send(UpMessage::Text("trusted".into()))
                        .await
                        .unwrap();
                } else {
                    assert!(stream.is_err());
                }
            };
            let client = async {
                let result = tokio_tungstenite::connect_async_tls_with_config(
                    url,
                    None,
                    true,
                    Some(connector),
                )
                .await;
                if trust_certificate {
                    let (mut socket, _) = result.unwrap();
                    assert_eq!(
                        socket.next().await.unwrap().unwrap(),
                        UpMessage::Text("trusted".into())
                    );
                } else {
                    let tokio_tungstenite::tungstenite::Error::Io(error) = result.unwrap_err()
                    else {
                        panic!("untrusted certificate should fail TLS validation");
                    };
                    assert!(matches!(
                        error
                            .get_ref()
                            .and_then(|e| e.downcast_ref::<rustls::Error>()),
                        Some(rustls::Error::InvalidCertificate(
                            rustls::CertificateError::UnknownIssuer
                        ))
                    ));
                }
            };
            tokio::time::timeout(Duration::from_secs(5), async {
                tokio::join!(server, client);
            })
            .await
            .unwrap();
        }
    }

    #[cfg(feature = "tls")]
    #[tokio::test]
    async fn untrusted_certificates_are_told_apart_from_unreachable_hosts() {
        let (acceptor, _) = self_signed();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("https://{}/", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let _ = acceptor.accept(stream).await;
        });
        let client = reqwest::Client::builder()
            .use_preconfigured_tls(build_tls_config(Vec::new()).unwrap())
            .build()
            .unwrap();
        let e = request_failed(
            "Cannot connect to B",
            client.get(url).send().await.unwrap_err(),
        );
        assert_eq!(e.error.code, "remote_certificate");
        assert!(e.error.message.starts_with("Cannot connect to B: "));
    }

    #[tokio::test]
    async fn failed_requests_name_their_cause() {
        // A port nobody listens on.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        drop(listener);
        let e = http_client().unwrap().get(url).send().await.unwrap_err();
        let e = request_failed("Cannot connect to B", e).error;
        assert_eq!(e.code, "remote_refused");
        let detail = e.params["detail"].as_str().unwrap();
        assert!(detail.contains("Connection refused"), "{detail}");
        assert_eq!(e.message, format!("Cannot connect to B: {detail}"));
    }

    #[tokio::test]
    async fn closing_a_view_cancels_a_request_before_response_headers() {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = worker::Endpoint {
            port: listener.local_addr().unwrap().port(),
            password: "test".into(),
            generation: "one".into(),
            supervisor: std::process::id(),
        };
        let lease = Lease::new("app".into(), false);
        let guard = AccessLease {
            lease: lease.clone(),
            owned: true,
            authorization: None,
        };
        let request =
            tokio::spawn(
                async move { fetch_data(guard, local_data(&endpoint, "ui-config")).await },
            );
        let (mut upstream, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut bytes = [0; 4096];
        assert!(upstream.read(&mut bytes).await.unwrap() > 0);
        lease.stop();
        let response = tokio::time::timeout(Duration::from_secs(2), request)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.status(), StatusCode::GONE);
    }

    #[tokio::test]
    async fn access_changes_end_sessions_without_a_websocket() {
        let store = Arc::new(super::super::store::RemoteStore::in_memory());
        store.set_policy("app", true, Some("password-one")).unwrap();
        let token = store.authenticate("app", "password-one").unwrap();
        let lease = Lease::new("app".into(), true);
        let guard = AccessLease {
            lease: lease.clone(),
            owned: true,
            authorization: Some((store.clone(), "app".into(), token.clone())),
        };
        assert!(lease.active());
        let waiting = tokio::spawn(async move { guard.cancelled().await });
        tokio::task::yield_now().await;
        assert!(lease.active());
        store.set_policy("app", true, Some("password-two")).unwrap();
        tokio::time::timeout(Duration::from_secs(2), waiting)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(lease.stop_reason(), Some(StopReason::Revoked));
        assert!(!store.verify("app", &token));
    }

    #[tokio::test]
    async fn failed_data_requests_release_session_capacity() {
        let lease = Lease::new("app".into(), true);
        let guard = AccessLease {
            lease: lease.clone(),
            owned: true,
            authorization: None,
        };
        let endpoint = worker::Endpoint {
            port: 0,
            password: "test".into(),
            generation: "one".into(),
            supervisor: std::process::id(),
        };
        let response = fetch_data(guard, local_data(&endpoint, "ui-config")).await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert!(!lease.active());
    }

    #[test]
    fn work_of_an_earlier_connection_records_nothing() {
        let cache = RemoteHealthCache::default();
        let old = cache.epoch("app");
        assert_eq!(cache.epoch("app"), old);
        assert!(cache.record("app", old, "connected", None));
        // The connection is saved again while a check of the old one runs.
        cache.forget("app");
        cache.remember_token("app", "new-token".into());
        let new = cache.epoch("app");
        assert_ne!(new, old);
        assert!(!cache.record("app", old, "auth_required", Some("stale".into())));
        assert!(cache.get("app").is_none());
        assert!(cache
            .current("app", old, |l| l.token = Some("stale".into()))
            .is_none());
        assert_eq!(cache.token("app", new).as_deref(), Some("new-token"));
        assert_eq!(cache.token("app", old), None);
    }

    /// A stand-in for B that counts authentications, answers them after a
    /// delay, and accepts `password-one` only.
    async fn fake_b() -> (Target, Arc<std::sync::atomic::AtomicUsize>) {
        let logins = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = logins.clone();
        let app = Router::new().route(
            "/remote/v1/apps/app/authenticate",
            post(move |Json(body): Json<Value>| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    if body["password"] != "password-one" {
                        return error(StatusCode::UNAUTHORIZED, "Invalid application credentials");
                    }
                    Json(json!({"protocol":PROTOCOL,"node_id":"node","app_id":"app","name":"App","token":super::super::token()})).into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = Target {
            base_url: format!("http://{}", listener.local_addr().unwrap()),
            app_id: "app".into(),
            node_id: "node".into(),
            name: "App".into(),
            password: "password-one".into(),
        };
        tokio::spawn(async move { axum::serve(listener, app).await });
        (target, logins)
    }

    #[tokio::test]
    async fn concurrent_requests_share_one_authentication() {
        use std::sync::atomic::Ordering::SeqCst;
        let apps = AppsState::for_test(None, None);
        let (target, logins) = fake_b().await;
        let epoch = apps.remote_health.epoch("app");
        let tokens = futures::future::join_all(
            (0..8).map(|_| session_token(&apps, "app", epoch, &target, None, false)),
        )
        .await;
        let tokens: Vec<String> = tokens.into_iter().map(|t| t.unwrap().0).collect();
        assert_eq!(logins.load(SeqCst), 1);
        assert!(tokens.iter().all(|t| *t == tokens[0]));
        // B no longer accepts it: requests that saw it rejected renew it once.
        let renewed = futures::future::join_all(
            (0..4).map(|_| session_token(&apps, "app", epoch, &target, Some(&tokens[0]), false)),
        )
        .await;
        assert_eq!(logins.load(SeqCst), 2);
        let renewed = renewed[0].as_ref().unwrap();
        assert!(renewed.1 && renewed.0 != tokens[0]);
    }

    #[tokio::test]
    async fn a_refused_password_is_tried_again_only_by_the_health_check() {
        use std::sync::atomic::Ordering::SeqCst;
        let apps = AppsState::for_test(None, None);
        let (target, logins) = fake_b().await;
        let target = Target {
            password: "password-old".into(),
            ..target
        };
        let epoch = apps.remote_health.epoch("app");
        let refused = session_token(&apps, "app", epoch, &target, None, false)
            .await
            .unwrap_err();
        assert_eq!(refused.state, "auth_required");
        assert_eq!(logins.load(SeqCst), 1);
        assert_eq!(
            apps.remote_health.get("app").unwrap().state,
            "auth_required"
        );
        // Views and their data requests fail at once.
        for _ in 0..3 {
            let e = session_token(&apps, "app", epoch, &target, None, false)
                .await
                .unwrap_err();
            assert_eq!(e.state, "auth_required");
            let e = probe(&apps, "app", epoch, &target, false)
                .await
                .unwrap_err();
            assert_eq!(e.state, "auth_required");
        }
        assert_eq!(logins.load(SeqCst), 1);
        // The health check tries again.
        assert!(session_token(&apps, "app", epoch, &target, None, true)
            .await
            .is_err());
        assert_eq!(logins.load(SeqCst), 2);
        // So does a connection saved again.
        apps.remote_health.forget("app");
        let epoch = apps.remote_health.epoch("app");
        let target = Target {
            password: "password-one".into(),
            ..target
        };
        assert!(session_token(&apps, "app", epoch, &target, None, false)
            .await
            .is_ok());
        assert_eq!(logins.load(SeqCst), 3);
    }

    #[test]
    fn failures_are_limited_per_source_and_prefix() {
        let limiter = Limiter::default();
        for _ in 0..8 {
            assert!(limiter.begin("2001:db8:1:2::5", "app"));
        }
        // Rotating addresses inside the same /64 does not reset the count.
        assert!(!limiter.begin("2001:db8:1:2::9", "app"));
        // Other sources and applications are unaffected.
        assert!(limiter.begin("2001:db8:1:3::1", "app"));
        assert!(limiter.begin("2001:db8:1:2::5", "other"));
        // Success clears the source/application count.
        let limiter = Limiter::default();
        for _ in 0..20 {
            assert!(limiter.begin("192.0.2.1", "app"));
            limiter.succeed("192.0.2.1", "app");
        }
        assert_eq!(Limiter::source("192.0.2.1"), "192.0.2.1");
    }

    #[test]
    fn share_addresses_cannot_be_arbitrary_proxy_targets() {
        assert_eq!(
            parse_share_url("https://b.example/remote/v1/apps/builtin-chrome").unwrap(),
            ("https://b.example".into(), "builtin-chrome".into())
        );
        for url in [
            "file:///etc/passwd",
            "https://user:secret@b.example/remote/v1/apps/app",
            "https://b.example/api/apps",
            "https://b.example/remote/v1/apps/app?password=x",
            "https://b.example/remote/v1/apps/app/other",
        ] {
            assert!(parse_share_url(url).is_err(), "{url}");
        }
        assert!(parse_share_url("http://127.0.0.1:8008/remote/v1/apps/app").is_ok());
        for (url, code) in [
            ("not a url", "remote_url_invalid"),
            ("file:///etc/passwd", "remote_url_unsupported"),
            (
                "https://b.example/remote/v1/apps/app?password=x",
                "remote_url_unsupported",
            ),
            ("https://b.example/api/apps", "remote_url_not_share"),
        ] {
            assert_eq!(parse_share_url(url).unwrap_err().code, code, "{url}");
        }
        assert!(parse_share_url("http://b.example:8008/remote/v1/apps/app").is_ok());
        assert!(data_path("ui-config"));
        assert!(!data_path("api/apps"));
        assert!(!data_path("app/files"));
        assert!(!data_path("api/console/display"));
    }

    fn gateway(apps: Arc<AppsState>) -> Arc<Gateway> {
        let config = crate::config::Config::default();
        let shared = SharedState::new(
            config.clone(),
            crate::config::ui::UiConfig::from_env(&config),
            tokio::sync::mpsc::unbounded_channel().0,
            Arc::new(crate::runtime_settings::RuntimeSettings::new(&config)),
            None,
            None,
        );
        Arc::new(Gateway {
            apps,
            shared: Arc::new(shared),
            limiter: Limiter::default(),
            pacing: Pacing::default(),
            password_jobs: Arc::new(tokio::sync::Semaphore::new(4)),
        })
    }

    /// The status and the `code` of an error response.
    async fn code(response: Response) -> (StatusCode, String) {
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert!(body["error"].as_str().is_some_and(|e| !e.is_empty()));
        (status, body["code"].as_str().unwrap_or_default().to_owned())
    }

    #[tokio::test]
    async fn console_errors_carry_codes() {
        let apps = AppsState::for_test(None, None);
        for (id, app_type) in [
            ("desktop", AppType::DesktopApp),
            ("remote", AppType::RemoteApp),
        ] {
            let app = crate::apps::app::ManagedApp {
                name: id.into(),
                ..crate::apps::app::ManagedApp::for_test(id, app_type)
            };
            apps.store.add(&app).unwrap();
        }
        let g = gateway(apps);
        let view = |id: &str| create_view(State(g.clone()), Path(id.into()));
        assert_eq!(
            code(view("remote").await).await,
            (StatusCode::CONFLICT, "remote_not_started".into())
        );
        assert_eq!(
            code(view("desktop").await).await,
            (StatusCode::CONFLICT, "app_not_running".into())
        );
        assert_eq!(
            code(view("missing").await).await,
            (StatusCode::NOT_FOUND, "app_not_found".into())
        );
        let sharing = |body| set_policy(State(g.clone()), Path("desktop".into()), Json(body));
        for (body, expected) in [
            (json!({"enabled": "yes"}), "sharing_enabled_invalid"),
            (json!({"enabled": true}), "share_password_required"),
            (
                json!({"enabled": true, "password": "short"}),
                "share_password_length",
            ),
        ] {
            assert_eq!(
                code(sharing(body).await).await,
                (StatusCode::BAD_REQUEST, expected.into())
            );
        }
        for (body, expected) in [
            (json!({}), "remote_url_required"),
            (
                json!({"url": "https://b.example/remote/v1/apps/app"}),
                "remote_password_required",
            ),
        ] {
            assert_eq!(
                connect_target(&body, "node").await.unwrap_err().code,
                expected
            );
        }
    }
}
