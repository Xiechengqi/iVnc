use super::{
    media::{Kind, Lease, Origin, StopReason},
    store::{Client, DesktopSettings, Target},
    worker, PROTOCOL,
};
use crate::{
    apps::{api::AppsState, app::AppType},
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
    routing::{get, post},
    Extension, Json, Router,
};
use futures::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, OnceLock},
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
    Remote(Target),
}
#[derive(Default)]
pub struct Views {
    entries: Mutex<HashMap<String, View>>,
}
impl Views {
    pub fn connection_status(&self, app: &str) -> &'static str {
        let entries = self.entries.lock().unwrap();
        let leases: Vec<_> = entries
            .values()
            .filter(|v| v.app == app)
            .filter_map(|v| v.lease.as_ref())
            .filter(|l| l.active())
            .collect();
        if leases.iter().any(|l| l.connected()) {
            "connected"
        } else if !leases.is_empty() {
            "connecting"
        } else {
            "disconnected"
        }
    }
    pub fn revoke(&self, app: &str, remote_only: bool, reason: StopReason) {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, v| {
            let revoke = v.app == app && (!remote_only || matches!(v.target, Source::Remote(_)));
            if revoke {
                v.stop(reason);
            }
            !revoke
        });
    }
}

/// Last known state of each remote application, as seen from A.
#[derive(Clone, Debug, Serialize)]
pub struct RemoteHealth {
    pub state: &'static str,
    pub detail: String,
    #[serde(skip)]
    at: Instant,
}
#[derive(Default)]
pub struct RemoteHealthCache {
    entries: Mutex<HashMap<String, RemoteHealth>>,
    refreshing: Mutex<HashSet<String>>,
}
impl RemoteHealthCache {
    pub fn get(&self, app: &str) -> Option<RemoteHealth> {
        self.entries.lock().unwrap().get(app).cloned()
    }
    pub fn forget(&self, app: &str) {
        self.entries.lock().unwrap().remove(app);
    }
    fn record(&self, app: &str, state: &'static str, detail: String) {
        self.entries.lock().unwrap().insert(
            app.to_owned(),
            RemoteHealth {
                state,
                detail,
                at: Instant::now(),
            },
        );
    }
}

/// Refreshes stale remote states in the background; listing never waits on B.
pub fn refresh_remote_health(apps: Arc<AppsState>, ids: Vec<String>) {
    for id in ids {
        let stale = apps
            .remote_health
            .get(&id)
            .is_none_or(|h| h.at.elapsed() > Duration::from_secs(30));
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
            if let Ok(target) = apps.remote.target(&id) {
                let _ = probe(&apps, &id, &target).await;
            }
            apps.remote_health.refreshing.lock().unwrap().remove(&id);
        });
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
        .route("/api/desktop/services", get(services))
        .route("/api/remote/test", post(test_connection))
        .route(
            "/api/apps/{id}/sessions/{session}",
            axum::routing::delete(disconnect_session),
        )
        .route(
            "/api/apps/{id}/grants/{grant}",
            axum::routing::delete(revoke_grant),
        )
        .route(
            "/remote/v1/apps/{id}/credential",
            axum::routing::delete(revoke_credential),
        )
        .route("/remote/v1/apps/{id}/authenticate", post(authenticate))
        .route("/remote/v1/apps/{id}/info", get(remote_info))
        .route("/remote/v1/apps/{id}/socket", get(remote_socket))
        .route("/remote/v1/apps/{id}/data/{*path}", get(remote_data))
        .route("/app-view/{id}/", get(view_index))
        .route("/app-view/{id}/webrtc", get(view_socket))
        .route("/app-view/{id}/close", post(close_view))
        .route("/app-view/{id}/{*path}", get(view_data))
        .with_state(state)
}
fn error(status: StatusCode, message: impl ToString) -> Response {
    (status, Json(json!({"error":message.to_string()}))).into_response()
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

/// Security-relevant remote access events, kept separately from the service log.
/// A dedicated thread writes them so request handlers never wait on the disk.
fn audit(event: &str, app: &str, detail: Value) {
    static WRITER: OnceLock<std::sync::mpsc::Sender<Value>> = OnceLock::new();
    log::info!("remote access {event}: app={app} {detail}");
    let line = json!({
        "time": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        "event": event,
        "app": app,
        "detail": detail,
    });
    let writer = WRITER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Value>();
        let path = crate::paths::ivnc_home().join("remote-audit.log");
        std::thread::spawn(move || {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            for line in rx {
                if std::fs::metadata(&path).is_ok_and(|m| m.len() > 4 * 1024 * 1024) {
                    let _ = std::fs::rename(&path, path.with_extension("log.1"));
                }
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .mode(0o600)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(&path)
                {
                    let _ = writeln!(file, "{line}");
                }
            }
        });
        tx
    });
    let _ = writer.send(line);
}

fn desktop(g: &Gateway, id: &str) -> Result<crate::apps::app::ManagedApp, Response> {
    let app = g
        .apps
        .store
        .get(id)
        .map_err(|_| error(StatusCode::NOT_FOUND, "Application not found"))?;
    if app.app_type != AppType::DesktopApp {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Only local desktop applications support remote access",
        ));
    }
    Ok(app)
}
/// Verifies an application credential and returns who is using it.
fn authorize(g: &Gateway, id: &str, headers: &HeaderMap, ip: &str) -> Result<Origin, Response> {
    desktop(g, id)?;
    let credential = bearer(headers);
    let Some((grant, label)) = g.apps.remote.grant(id, credential) else {
        return Err(error(
            StatusCode::UNAUTHORIZED,
            "Application credentials expired or remote access is disabled",
        ));
    };
    g.apps.remote.touch(id, credential, ip);
    Ok(Origin {
        grant,
        label,
        peer: ip.to_owned(),
    })
}
fn runtime_check(g: &Gateway, id: &str) -> Result<worker::Endpoint, (StatusCode, String)> {
    if g.apps.process.status(id) != crate::apps::app::AppStatus::Running {
        return Err((StatusCode::CONFLICT, "Application is not running".into()));
    }
    let endpoint = worker::endpoint(id).map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;
    let policy = g
        .apps
        .remote
        .policy(id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if endpoint.desktop != policy.desktop {
        return Err((
            StatusCode::CONFLICT,
            "Restart this application on B to apply its isolation settings".into(),
        ));
    }
    Ok(endpoint)
}
fn runtime(g: &Gateway, id: &str) -> Result<worker::Endpoint, Response> {
    runtime_check(g, id).map_err(|(status, message)| error(status, message))
}
async fn get_policy(State(g): State<Arc<Gateway>>, Path(id): Path<String>) -> Response {
    if let Err(e) = desktop(&g, &id) {
        return e;
    }
    let policy = match g.apps.remote.policy(&id) {
        Ok(p) => p,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let sessions = g.shared.media_router.sessions(&id);
    let grants: Vec<Value> = match g.apps.remote.grants(&id) {
        Ok(grants) => grants
            .into_iter()
            .map(|grant| {
                let active = sessions
                    .iter()
                    .filter(|s| s["grant"] == grant.id.as_str())
                    .count();
                let mut value = json!(grant);
                value["active_sessions"] = json!(active);
                value
            })
            .collect(),
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let restart_required = worker::endpoint(&id).is_ok_and(|e| e.desktop != policy.desktop);
    Json(json!({
        "policy": policy,
        "share_path": format!("/remote/v1/apps/{id}"),
        "node_id": g.apps.remote.node_id,
        "sessions": sessions,
        "grants": grants,
        "restart_required": restart_required,
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
        None => return error(StatusCode::BAD_REQUEST, "enabled must be a boolean"),
    };
    if enabled {
        let override_password = g.shared.password_override.read().await;
        let password = override_password
            .as_deref()
            .unwrap_or(&g.shared.config.http.basic_auth_password);
        if !g.shared.config.http.basic_auth_enabled || password == "mypasswd" || password.is_empty()
        {
            return error(
                StatusCode::BAD_REQUEST,
                "Set a private instance management password before sharing applications",
            );
        }
    }
    let previous = match g.apps.remote.policy(&id) {
        Ok(policy) => policy,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let settings = match body.get("desktop") {
        Some(value) => match serde_json::from_value::<DesktopSettings>(value.clone()) {
            Ok(settings) => settings,
            Err(e) => return error(StatusCode::BAD_REQUEST, e),
        },
        None => previous.desktop.clone(),
    };
    if let Err(e) = settings.validate() {
        return error(StatusCode::BAD_REQUEST, e);
    }
    if let Err(e) = worker::resolve_mounts(&settings.mounts, &worker::protected_paths()) {
        return error(StatusCode::BAD_REQUEST, e);
    }
    for service in &settings.services {
        if !g.apps.store.get(service).is_ok_and(|a| {
            a.app_type == AppType::BackgroundApp && worker::service_url(&a).is_some()
        }) {
            return error(StatusCode::BAD_REQUEST, "Unknown background service");
        }
    }
    let old_revision = previous.revision;
    let store = g.apps.remote.clone();
    let app = id.clone();
    let password = body["password"].as_str().map(str::to_owned);
    let password_changed = password.as_deref().is_some_and(|p| !p.is_empty());
    let result = tokio::task::spawn_blocking(move || {
        store.set_policy(&app, enabled, password.as_deref(), settings)
    })
    .await;
    match result {
        Ok(Ok(policy)) => {
            let revoked = policy.revision != old_revision;
            if revoked {
                g.shared.media_router.revoke(&id, true, StopReason::Revoked);
                audit(
                    "policy_changed",
                    &id,
                    json!({"enabled": enabled, "password_changed": password_changed}),
                );
            }
            let restart_required = worker::endpoint(&id).is_ok_and(|e| e.desktop != policy.desktop);
            Json(json!({"ok":true,"policy":policy,"restart_required":restart_required,"revoked":revoked}))
                .into_response()
        }
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, e),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Password processing failed",
        ),
    }
}
async fn services(State(g): State<Arc<Gateway>>) -> Response {
    match g.apps.store.list(){Ok(apps)=>Json(json!({"services":apps.into_iter().filter(|a|a.app_type==AppType::BackgroundApp && worker::service_url(a).is_some()).map(|a|json!({"id":a.id,"name":a.name,"url":worker::service_url(&a)})).collect::<Vec<_>>()})).into_response(),Err(e)=>error(StatusCode::INTERNAL_SERVER_ERROR,e)}
}
async fn authenticate(
    State(g): State<Arc<Gateway>>,
    Path(id): Path<String>,
    addr: Option<Extension<HttpPeerAddr>>,
    Json(body): Json<Value>,
) -> Response {
    let ip = peer(&addr);
    if !super::valid_id(&id) {
        return error(StatusCode::UNAUTHORIZED, "Invalid application credentials");
    }
    if !g.limiter.begin(&ip, &id) {
        audit("auth_limited", &id, json!({"ip": ip}));
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many failed attempts from this address; retry later",
        );
    }
    let password = body["password"].as_str().unwrap_or("").to_string();
    let client = Client {
        node: body["client"]["node_id"].as_str().unwrap_or("").into(),
        label: body["client"]["name"].as_str().unwrap_or("").into(),
        ip: ip.clone(),
    };
    let busy = || {
        error(
            StatusCode::TOO_MANY_REQUESTS,
            "Authentication is busy; retry shortly",
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
        store.authenticate(&app, &password, &client)
    })
    .await;
    match (result, desktop(&g, &id)) {
        (Ok(Ok(credential)), Ok(app)) => {
            g.limiter.succeed(&ip, &id);
            audit(
                "auth_success",
                &id,
                json!({"ip": ip, "client": body["client"]["name"], "node": body["client"]["node_id"]}),
            );
            Json(json!({"protocol":PROTOCOL,"node_id":g.apps.remote.node_id,"app_id":id,"name":app.name,"credential":credential})).into_response()
        }
        (Ok(Ok(credential)), Err(_)) => {
            let _ = g.apps.remote.revoke_grant(&id, &credential);
            error(StatusCode::UNAUTHORIZED, "Invalid application credentials")
        }
        (Ok(Err(e)), _) => {
            if e.starts_with("Invalid") {
                audit("auth_failure", &id, json!({"ip": ip}));
                // Keep this application's slot busy so failures stay spaced out.
                tokio::time::sleep(Duration::from_secs(1)).await;
                error(StatusCode::UNAUTHORIZED, "Invalid application credentials")
            } else {
                error(StatusCode::CONFLICT, e)
            }
        }
        (Err(_), _) => error(StatusCode::INTERNAL_SERVER_ERROR, "Authentication failed"),
    }
}
async fn remote_info(
    State(g): State<Arc<Gateway>>,
    Path(id): Path<String>,
    addr: Option<Extension<HttpPeerAddr>>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = authorize(&g, &id, &headers, &peer(&addr)) {
        return e;
    }
    let app = match desktop(&g, &id) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let status = format!("{:?}", g.apps.app_status(&app)).to_lowercase();
    let (ready, detail) = match runtime_check(&g, &id) {
        Ok(_) => (true, String::new()),
        Err((_, message)) => (false, message),
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
pub fn http_client() -> Result<reqwest::Client, String> {
    tls_client().map(|(_, client)| client)
}
pub fn parse_share_url(value: &str) -> Result<(String, String), String> {
    let url =
        reqwest::Url::parse(value.trim()).map_err(|_| "Invalid application connection URL")?;
    if !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Use an HTTP(S) application URL without credentials, query or fragment".into());
    }
    // Plain HTTP is restricted to a local/private transport, never public hosts.
    if url.scheme() == "http" {
        let host = url.host_str().unwrap_or("").trim_matches(['[', ']']);
        let trusted = host == "localhost"
            || host.parse::<std::net::IpAddr>().is_ok_and(|ip| match ip {
                std::net::IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
                std::net::IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local(),
            });
        if !trusted {
            return Err(
                "Remote public hosts require HTTPS; configure a trusted certificate".into(),
            );
        }
    }
    let id = url
        .path()
        .trim_end_matches('/')
        .strip_prefix("/remote/v1/apps/")
        .filter(|s| super::valid_id(s))
        .ok_or("Use the application sharing URL copied from B")?
        .to_owned();
    Ok((url.origin().ascii_serialization(), id))
}
fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}
/// Authenticates against B. `own_node` prevents mapping an instance onto itself,
/// which would register the same media session twice in one router.
pub async fn connect_target(body: &Value, own_node: &str) -> Result<Target, String> {
    let (base_url, app_id) =
        parse_share_url(body["url"].as_str().ok_or("Missing application URL")?)?;
    let password = body["password"]
        .as_str()
        .filter(|p| !p.is_empty() && p.len() <= 256)
        .ok_or("Enter the application password")?;
    let response = http_client()?
        .post(format!("{base_url}/remote/v1/apps/{app_id}/authenticate"))
        .json(&json!({"password":password,"client":{"node_id":own_node,"name":hostname()}}))
        .send()
        .await
        .map_err(|e| format!("Cannot connect to B: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = bounded_json(response)
            .await
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_owned))
            .unwrap_or_default();
        return Err(format!(
            "B rejected application authentication ({status}): {detail}"
        ));
    }
    let value = bounded_json(response).await?;
    if value["protocol"].as_u64() != Some(PROTOCOL as u64)
        || value["app_id"].as_str() != Some(&app_id)
    {
        return Err("B does not support this application sharing protocol".into());
    }
    let target = Target {
        base_url,
        app_id,
        node_id: value["node_id"]
            .as_str()
            .ok_or("Missing B identity")?
            .into(),
        name: value["name"]
            .as_str()
            .ok_or("Missing application name")?
            .into(),
        credential: value["credential"]
            .as_str()
            .filter(|s| s.len() == 64)
            .ok_or("Invalid application credential")?
            .into(),
    };
    if target.node_id == own_node {
        forget_target(&target).await;
        return Err(
            "This address is the current instance; open the local application directly".into(),
        );
    }
    Ok(target)
}

/// Why B could not serve a remote application, in a form A can show and cache.
#[derive(Debug)]
pub struct Probe {
    pub state: &'static str,
    pub message: String,
}
async fn check_target(target: &Target) -> Result<Value, Probe> {
    let unreachable = |message: String| Probe {
        state: "unreachable",
        message,
    };
    let client = http_client().map_err(unreachable)?;
    let r = client
        .get(format!(
            "{}/remote/v1/apps/{}/info",
            target.base_url, target.app_id
        ))
        .bearer_auth(&target.credential)
        .send()
        .await
        .map_err(|e| unreachable(format!("B is unavailable: {e}")))?;
    if r.status() == StatusCode::UNAUTHORIZED || r.status() == StatusCode::NOT_FOUND {
        return Err(Probe {
            state: "auth_required",
            message: "B revoked this connection or disabled sharing; edit it and enter the application password again".into(),
        });
    }
    if !r.status().is_success() {
        return Err(unreachable(format!("B returned {}", r.status())));
    }
    let value = bounded_json(r).await.map_err(unreachable)?;
    if value["protocol"].as_u64() != Some(PROTOCOL as u64)
        || value["node_id"].as_str() != Some(&target.node_id)
        || value["app_id"].as_str() != Some(&target.app_id)
    {
        return Err(Probe {
            state: "incompatible",
            message: "Remote instance identity or application protocol changed".into(),
        });
    }
    Ok(value)
}
/// Checks B and records the result for the application list.
async fn probe(apps: &AppsState, id: &str, target: &Target) -> Result<Value, Probe> {
    let result = check_target(target).await;
    match &result {
        Ok(value) if value["status"] != "running" => {
            apps.remote_health
                .record(id, "stopped", "The application is not running on B".into())
        }
        // B versions without readiness information are treated as ready.
        Ok(value) if value["ready"] == false => apps.remote_health.record(
            id,
            "not_ready",
            value["detail"].as_str().unwrap_or("").to_owned(),
        ),
        Ok(_) => apps.remote_health.record(id, "running", String::new()),
        Err(probe) => apps
            .remote_health
            .record(id, probe.state, probe.message.clone()),
    }
    result
}
async fn create_view(State(g): State<Arc<Gateway>>, Path(id): Path<String>) -> Response {
    let app = match g.apps.store.get(&id) {
        Ok(a) => a,
        Err(e) => return error(StatusCode::NOT_FOUND, e),
    };
    let source = match app.app_type {
        AppType::DesktopApp => match runtime(&g, &id) {
            Ok(e) => Source::Local(e),
            Err(e) => return e,
        },
        AppType::RemoteApp => {
            let target = match g.apps.remote.target(&id) {
                Ok(t) => t,
                Err(e) => return error(StatusCode::BAD_REQUEST, e),
            };
            match probe(&g.apps, &id, &target).await {
                Ok(v) if v["status"] != "running" => {
                    return error(StatusCode::CONFLICT, "Application is not running on B")
                }
                Ok(v) if v["ready"] == false => {
                    return error(
                        StatusCode::CONFLICT,
                        format!(
                            "B: {}",
                            v["detail"].as_str().unwrap_or("application is not ready")
                        ),
                    )
                }
                Ok(_) => {}
                Err(e) => return error(StatusCode::BAD_GATEWAY, e.message),
            }
            Source::Remote(target)
        }
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "Only desktop applications have remote views",
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
        return error(StatusCode::TOO_MANY_REQUESTS, "Too many application views");
    }
    entries.insert(
        view.clone(),
        View {
            lifetime: Lease::new(id.clone(), false, false, String::new()),
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
                "Application view expired; open it again from the application list",
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
fn data_path(path: &str) -> bool {
    matches!(
        path,
        "ui-config" | "ws-config" | "api/render-status" | "app/files"
    ) || (path.starts_with("app/files/") && !path.contains("..") && !path.contains('\\'))
}
async fn fetch_data(source: &Source, path: &str, guard: AccessLease) -> Response {
    let client = match http_client() {
        Ok(c) => c,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let escaped = path
        .split('/')
        .map(|part| {
            let mut url = reqwest::Url::parse("http://local/").unwrap();
            url.path_segments_mut().unwrap().push(part);
            url.path()[1..].to_owned()
        })
        .collect::<Vec<_>>()
        .join("/");
    let request = match source {
        Source::Local(e) => client
            .get(format!("http://127.0.0.1:{}/{escaped}", e.port))
            .basic_auth("worker", Some(&e.password)),
        Source::Remote(t) => client
            .get(format!(
                "{}/remote/v1/apps/{}/data/{escaped}",
                t.base_url, t.app_id
            ))
            .bearer_auth(&t.credential),
    };
    // Large downloads are bounded by inactivity rather than total duration.
    let response = tokio::select! {
        _ = guard.cancelled() => return error(StatusCode::GONE, "Application access ended"),
        result = request.timeout(Duration::from_secs(24 * 3600)).send() => result,
    };
    match response {
        Ok(response) => {
            let status = response.status();
            let mut builder = Response::builder()
                .status(status)
                .header(header::CACHE_CONTROL, "no-store");
            for name in [
                header::CONTENT_TYPE,
                header::CONTENT_DISPOSITION,
                header::CONTENT_LENGTH,
            ] {
                if let Some(value) = response.headers().get(&name) {
                    builder = builder.header(name, value);
                }
            }
            let stream = futures::stream::unfold(
                (response.bytes_stream().boxed(), guard),
                |(mut stream, guard)| async move {
                    let next = tokio::select! {
                        _ = guard.cancelled() => None,
                        next = tokio::time::timeout(Duration::from_secs(120), stream.next()) => match next {
                            Ok(item) => item.map(|r| r.map_err(std::io::Error::other)),
                            Err(_) => Some(Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "download stalled"))),
                        },
                    };
                    next.map(|data| (data, (stream, guard)))
                },
            );
            builder.body(Body::from_stream(stream)).unwrap()
        }
        Err(_) => error(
            StatusCode::BAD_GATEWAY,
            "Application runtime is unavailable",
        ),
    }
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
        return fetch_data(
            &v.target,
            &path,
            AccessLease {
                lease: v.lifetime.clone(),
                owned: false,
                authorization: None,
            },
        )
        .await;
    }
    // Only trusted A assets are embedded. No HTML or JavaScript is loaded from B.
    if path.contains("..")
        || path.starts_with("api/")
        || path == "console"
        || path == "console.html"
        || path == "console.js"
    {
        return error(StatusCode::NOT_FOUND, "Not found");
    }
    crate::web::embedded_assets::get_embedded_file(&path)
}
async fn remote_data(
    State(g): State<Arc<Gateway>>,
    Path((id, path)): Path<(String, String)>,
    addr: Option<Extension<HttpPeerAddr>>,
    headers: HeaderMap,
) -> Response {
    let origin = match authorize(&g, &id, &headers, &peer(&addr)) {
        Ok(o) => o,
        Err(e) => return e,
    };
    if !data_path(&path) {
        return error(StatusCode::NOT_FOUND, "Not found");
    }
    let endpoint = match runtime(&g, &id) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let lease = Lease::remote(id.clone(), Kind::Data, endpoint.generation.clone(), origin);
    if let Err(e) = g.shared.media_router.track(&lease) {
        return error(StatusCode::TOO_MANY_REQUESTS, e);
    }
    fetch_data(
        &Source::Local(endpoint),
        &path,
        AccessLease {
            lease,
            owned: true,
            authorization: Some((g.apps.remote.clone(), id, bearer(&headers).to_owned())),
        },
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
        return error(StatusCode::FORBIDDEN, "Invalid WebSocket origin");
    }
    let v = match view(&g, &id) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if let Source::Local(e) = &v.target {
        match runtime(&g, &v.app) {
            Ok(now) if now.generation == e.generation => {}
            _ => {
                return error(
                    StatusCode::CONFLICT,
                    "Application restarted; open a new view",
                )
            }
        }
    }
    let owner = matches!(v.target, Source::Local(_));
    let generation = match &v.target {
        Source::Local(e) => e.generation.clone(),
        Source::Remote(t) => t.node_id.clone(),
    };
    let lease = Lease::new(v.app.clone(), !owner, owner, generation);
    if let Err(e) = g.shared.media_router.track(&lease) {
        lease.stop();
        return error(StatusCode::TOO_MANY_REQUESTS, e);
    }
    {
        let mut entries = g.shared.app_views.entries.lock().unwrap();
        let Some(entry) = entries.get_mut(&id) else {
            lease.stop();
            return error(StatusCode::GONE, "View expired");
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
    addr: Option<Extension<HttpPeerAddr>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let origin = match authorize(&g, &id, &headers, &peer(&addr)) {
        Ok(o) => o,
        Err(e) => return e,
    };
    let endpoint = match runtime(&g, &id) {
        Ok(e) => e,
        Err(e) => return e,
    };
    audit(
        "connect",
        &id,
        json!({"ip": origin.peer, "grant": origin.grant, "client": origin.label}),
    );
    let lease = Lease::remote(
        id.clone(),
        Kind::Signal,
        endpoint.generation.clone(),
        origin,
    );
    let credential = bearer(&headers).to_string();
    if let Err(e) = g.shared.media_router.track(&lease) {
        lease.stop();
        return error(StatusCode::TOO_MANY_REQUESTS, e);
    }
    // If the upgrade never completes, the callback is dropped and so is the guard.
    let pending = StopOnDrop(lease.clone());
    ws.max_message_size(256 * 1024)
        .on_upgrade(move |socket| async move {
            let _pending = pending;
            bridge(
                g,
                socket,
                Source::Local(endpoint),
                lease,
                Some((id, credential)),
            )
            .await
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
        authorization: authorization
            .map(|(app, credential)| (g.apps.remote.clone(), app, credential)),
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
                StatusCode::UNAUTHORIZED => "B revoked this connection or disabled sharing",
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
        Source::Remote(t) => {
            match probe(&g.apps, &lease.app, t).await {
                Ok(v) if v["ready"] == false => {
                    return Err(format!(
                        "B: {}",
                        v["detail"].as_str().unwrap_or("not ready")
                    ))
                }
                Ok(_) => {}
                Err(e) => return Err(e.message),
            }
            let mut url = reqwest::Url::parse(&format!(
                "{}/remote/v1/apps/{}/socket",
                t.base_url, t.app_id
            ))
            .map_err(|e| e.to_string())?;
            let destination = format!(
                "{}:{}",
                url.host_str().ok_or("Missing remote host")?,
                url.port_or_known_default().unwrap_or(443)
            );
            url.set_scheme(if url.scheme() == "https" { "wss" } else { "ws" })
                .map_err(|_| "Invalid scheme")?;
            (
                url.to_string(),
                format!("Bearer {}", t.credential),
                destination,
            )
        }
    };
    let mut request = url.into_client_request().map_err(|e| e.to_string())?;
    request.headers_mut().insert(
        header::AUTHORIZATION,
        auth.parse().map_err(|_| "Invalid upstream credentials")?,
    );
    if matches!(source, Source::Local(_)) {
        request.headers_mut().insert(
            "x-ivnc-owner",
            if lease.owner { "true" } else { "false" }.parse().unwrap(),
        );
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "tls")]
    #[tokio::test]
    async fn wss_uses_explicit_crypto_provider_and_validates_trust() {
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
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));

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

    #[tokio::test]
    async fn closing_a_view_cancels_a_request_before_response_headers() {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = worker::Endpoint {
            port: listener.local_addr().unwrap().port(),
            password: "test".into(),
            generation: "one".into(),
            desktop: DesktopSettings::default(),
            supervisor: std::process::id(),
        };
        let lease = Lease::new("app".into(), false, true, "one".into());
        let guard = AccessLease {
            lease: lease.clone(),
            owned: true,
            authorization: None,
        };
        let request =
            tokio::spawn(
                async move { fetch_data(&Source::Local(endpoint), "app/files", guard).await },
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
    async fn credential_revocation_cancels_downloads_without_a_websocket() {
        let store = Arc::new(super::super::store::RemoteStore::in_memory());
        store
            .set_policy(
                "app",
                true,
                Some("password-one"),
                DesktopSettings::default(),
            )
            .unwrap();
        let credential = store
            .authenticate("app", "password-one", &Default::default())
            .unwrap();
        let lease = Lease::new("app".into(), true, false, "one".into());
        let guard = AccessLease {
            lease: lease.clone(),
            owned: true,
            authorization: Some((store.clone(), "app".into(), credential.clone())),
        };
        assert!(lease.active());
        store.revoke_grant("app", &credential).unwrap();
        tokio::time::timeout(Duration::from_secs(2), guard.cancelled())
            .await
            .unwrap();
        assert!(!lease.active());
    }

    #[tokio::test]
    async fn failed_data_requests_release_session_capacity() {
        let lease = Lease::new("app".into(), true, false, "one".into());
        let guard = AccessLease {
            lease: lease.clone(),
            owned: true,
            authorization: None,
        };
        let endpoint = worker::Endpoint {
            port: 0,
            password: "test".into(),
            generation: "one".into(),
            desktop: DesktopSettings::default(),
            supervisor: std::process::id(),
        };
        let response = fetch_data(&Source::Local(endpoint), "app/files", guard).await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert!(!lease.active());
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
            "http://b.example/remote/v1/apps/app",
            "https://user:secret@b.example/remote/v1/apps/app",
            "https://b.example/api/apps",
            "https://b.example/remote/v1/apps/app?password=x",
            "https://b.example/remote/v1/apps/app/other",
        ] {
            assert!(parse_share_url(url).is_err(), "{url}");
        }
        assert!(parse_share_url("http://127.0.0.1:8008/remote/v1/apps/app").is_ok());
        assert!(!data_path("api/apps"));
        assert!(!data_path("app/files/../../etc/passwd"));
    }
}

async fn disconnect_session(
    State(g): State<Arc<Gateway>>,
    Path((app, session)): Path<(String, String)>,
) -> Response {
    if let Err(e) = desktop(&g, &app) {
        return e;
    }
    if g.shared.media_router.disconnect(&app, &session) {
        audit("disconnect", &app, json!({"session": session}));
        Json(json!({"ok":true})).into_response()
    } else {
        error(StatusCode::NOT_FOUND, "Session not found")
    }
}
/// Revokes one saved connection on B and ends every session that used it.
async fn revoke_grant(
    State(g): State<Arc<Gateway>>,
    Path((app, grant)): Path<(String, String)>,
) -> Response {
    if let Err(e) = desktop(&g, &app) {
        return e;
    }
    match g.apps.remote.revoke_grant_id(&app, &grant) {
        Ok(true) => {
            g.shared.media_router.revoke_grant(&app, &grant);
            audit("revoke_grant", &app, json!({"grant": grant}));
            Json(json!({"ok":true})).into_response()
        }
        Ok(false) => error(StatusCode::NOT_FOUND, "Connection not found"),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}
async fn revoke_credential(
    State(g): State<Arc<Gateway>>,
    Path(id): Path<String>,
    addr: Option<Extension<HttpPeerAddr>>,
    headers: HeaderMap,
) -> Response {
    let origin = match authorize(&g, &id, &headers, &peer(&addr)) {
        Ok(o) => o,
        Err(e) => return e,
    };
    match g.apps.remote.revoke_grant(&id, bearer(&headers)) {
        Ok(()) => {
            g.shared.media_router.revoke_grant(&id, &origin.grant);
            Json(json!({"ok":true})).into_response()
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}
async fn test_connection(State(g): State<Arc<Gateway>>, Json(body): Json<Value>) -> Response {
    match connect_target(&body, &g.apps.remote.node_id).await {
        Ok(target) => {
            let result = check_target(&target).await;
            forget_target(&target).await;
            match result {
                Ok(info) => Json(info).into_response(),
                Err(e) => error(StatusCode::BAD_GATEWAY, e.message),
            }
        }
        Err(e) => error(StatusCode::BAD_REQUEST, e),
    }
}

/// Best effort: removing a mapping must still work when B is unavailable.
pub async fn forget_target(target: &Target) {
    if let Ok(client) = http_client() {
        let _ = client
            .delete(format!(
                "{}/remote/v1/apps/{}/credential",
                target.base_url, target.app_id
            ))
            .bearer_auth(&target.credential)
            .timeout(Duration::from_secs(3))
            .send()
            .await;
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
    /// Completes when access ends, including when its credential is revoked.
    /// Credential changes are signalled by the store; the slow timer only
    /// catches expiry.
    async fn cancelled(&self) {
        let Some((store, app, credential)) = &self.authorization else {
            return self.lease.cancelled().await;
        };
        let mut changes = store.subscribe();
        loop {
            if !store.verify(app, credential) {
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
async fn bounded_json(response: reqwest::Response) -> Result<Value, String> {
    let mut stream = response.bytes_stream();
    let mut data = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        if data.len() + chunk.len() > 65536 {
            return Err("Remote application metadata is too large".into());
        }
        data.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&data).map_err(|_| "Invalid remote application metadata".into())
}
