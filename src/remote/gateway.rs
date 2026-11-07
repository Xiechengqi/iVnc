use super::{
    media::Lease,
    store::{DesktopSettings, Target},
    worker, PROTOCOL,
};
use crate::{
    apps::{api::AppsState, app::AppType},
    web::SharedState,
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
    Json, Router,
};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as UpMessage};

#[derive(Clone)]
pub struct View {
    pub app: String,
    target: Source,
    created: Instant,
    lifetime: Arc<Lease>,
    lease: Option<Arc<Lease>>,
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
    pub fn revoke(&self, app: &str, remote_only: bool) {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, v| {
            let revoke = v.app == app && (!remote_only || matches!(v.target, Source::Remote(_)));
            if revoke {
                v.lifetime.stop();
                if let Some(l) = &v.lease {
                    l.stop();
                }
            }
            !revoke
        });
    }
}
struct Gateway {
    apps: Arc<AppsState>,
    shared: Arc<SharedState>,
    attempts: Mutex<HashMap<String, (Instant, u32)>>,
    password_jobs: Arc<tokio::sync::Semaphore>,
}
pub fn router(apps: Arc<AppsState>, shared: Arc<SharedState>) -> Router {
    let _ = apps.shared.set(Arc::downgrade(&shared));
    let state = Arc::new(Gateway {
        apps,
        shared,
        attempts: Mutex::new(HashMap::new()),
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
fn authorize(g: &Gateway, id: &str, headers: &HeaderMap) -> Result<(), Response> {
    desktop(g, id)?;
    if !g.apps.remote.verify(id, bearer(headers)) {
        return Err(error(
            StatusCode::UNAUTHORIZED,
            "Application credentials expired or remote access is disabled",
        ));
    }
    Ok(())
}
fn runtime(g: &Gateway, id: &str) -> Result<worker::Endpoint, Response> {
    if g.apps.process.status(id) != crate::apps::app::AppStatus::Running {
        return Err(error(StatusCode::CONFLICT, "Application is not running"));
    }
    let endpoint = worker::endpoint(id).map_err(|e| error(StatusCode::SERVICE_UNAVAILABLE, e))?;
    let policy = g
        .apps
        .remote
        .policy(id)
        .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if endpoint.desktop != policy.desktop {
        return Err(error(
            StatusCode::CONFLICT,
            "Restart this application on B to apply its desktop/network settings",
        ));
    }
    Ok(endpoint)
}
async fn get_policy(State(g): State<Arc<Gateway>>, Path(id): Path<String>) -> Response {
    if let Err(e) = desktop(&g, &id) {
        return e;
    }
    match g.apps.remote.policy(&id){Ok(p)=>Json(json!({"policy":p,"share_path":format!("/remote/v1/apps/{id}"),"node_id":g.apps.remote.node_id,"sessions":g.shared.media_router.sessions(&id)})).into_response(),Err(e)=>error(StatusCode::INTERNAL_SERVER_ERROR,e)}
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
        None => previous.desktop,
    };
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
    let result = tokio::task::spawn_blocking(move || {
        store.set_policy(&app, enabled, password.as_deref(), settings)
    })
    .await;
    match result {
        Ok(Ok(policy)) => {
            if policy.revision != old_revision {
                g.shared.media_router.revoke(&id, true);
            }
            let restart_required = worker::endpoint(&id).is_ok_and(|e| e.desktop != policy.desktop);
            Json(json!({"ok":true,"policy":policy,"restart_required":restart_required}))
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
    Json(body): Json<Value>,
) -> Response {
    // An application cannot be used as an unlimited Argon2 work queue.
    {
        let mut attempts = g.attempts.lock().unwrap();
        attempts.retain(|_, (t, _)| t.elapsed() < Duration::from_secs(60));
        if attempts.len() >= 1024 {
            return error(StatusCode::TOO_MANY_REQUESTS, "Try again later");
        }
        let entry = attempts.entry(id.clone()).or_insert((Instant::now(), 0));
        if entry.1 >= 10 {
            return error(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many authentication attempts; retry in one minute",
            );
        }
        entry.1 += 1;
    }
    if desktop(&g, &id).is_err() {
        return error(StatusCode::UNAUTHORIZED, "Invalid application credentials");
    }
    let Some(password) = body["password"].as_str().filter(|s| s.len() <= 256) else {
        return error(StatusCode::UNAUTHORIZED, "Invalid application credentials");
    };
    let permit = match g.password_jobs.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            return error(
                StatusCode::TOO_MANY_REQUESTS,
                "Authentication is busy; retry shortly",
            )
        }
    };
    let password = password.to_string();
    let store = g.apps.remote.clone();
    let app = id.clone();
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        store.authenticate(&app, &password)
    })
    .await
    {
        Ok(Ok(credential)) => {
            let app = match desktop(&g, &id) {
                Ok(a) => a,
                Err(e) => return e,
            };
            Json(json!({"protocol":PROTOCOL,"node_id":g.apps.remote.node_id,"app_id":id,"name":app.name,"credential":credential})).into_response()
        }
        _ => error(StatusCode::UNAUTHORIZED, "Invalid application credentials"),
    }
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
    Json(json!({"protocol":PROTOCOL,"node_id":g.apps.remote.node_id,"app_id":id,"name":app.name,"status":format!("{:?}",g.apps.app_status(&app)).to_lowercase()})).into_response()
}
pub fn http_client() -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30));
    for cert in trusted_certificates()? {
        builder = builder.add_root_certificate(
            reqwest::Certificate::from_der(&cert).map_err(|e| e.to_string())?,
        );
    }
    builder.build().map_err(|e| e.to_string())
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
pub async fn connect_target(body: &Value) -> Result<Target, String> {
    let (base_url, app_id) =
        parse_share_url(body["url"].as_str().ok_or("Missing application URL")?)?;
    let password = body["password"]
        .as_str()
        .filter(|p| !p.is_empty() && p.len() <= 256)
        .ok_or("Enter the application password")?;
    let response = http_client()?
        .post(format!("{base_url}/remote/v1/apps/{app_id}/authenticate"))
        .json(&json!({"password":password}))
        .send()
        .await
        .map_err(|e| format!("Cannot connect to B: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "B rejected application authentication ({})",
            response.status()
        ));
    }
    let value = bounded_json(response).await?;
    if value["protocol"].as_u64() != Some(PROTOCOL as u64)
        || value["app_id"].as_str() != Some(&app_id)
    {
        return Err("B does not support this application sharing protocol".into());
    }
    Ok(Target {
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
    })
}
async fn check_target(target: &Target) -> Result<Value, String> {
    let r = http_client()?
        .get(format!(
            "{}/remote/v1/apps/{}/info",
            target.base_url, target.app_id
        ))
        .bearer_auth(&target.credential)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err("B is unavailable or this connection has been revoked; enter the application password again".into());
    }
    let value = bounded_json(r).await?;
    if value["protocol"].as_u64() != Some(PROTOCOL as u64)
        || value["node_id"].as_str() != Some(&target.node_id)
        || value["app_id"].as_str() != Some(&target.app_id)
    {
        return Err("Remote instance identity or application protocol changed".into());
    }
    Ok(value)
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
            match check_target(&target).await {
                Ok(v) if v["status"] == "running" => {}
                Ok(_) => return error(StatusCode::CONFLICT, "Application is not running on B"),
                Err(e) => return error(StatusCode::BAD_GATEWAY, e),
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
        let keep = v.created.elapsed() < Duration::from_secs(8 * 3600);
        if !keep {
            v.lifetime.stop();
            if let Some(l) = &v.lease {
                l.stop();
            }
        }
        keep
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
            created: Instant::now(),
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
        .filter(|v| v.created.elapsed() < Duration::from_secs(8 * 3600))
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
    if let Some(v) = g.shared.app_views.entries.lock().unwrap().remove(&id) {
        v.lifetime.stop();
        if let Some(l) = v.lease {
            l.stop();
        }
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
    let response = tokio::select! {
        _ = guard.cancelled() => return error(StatusCode::GONE, "Application access ended"),
        result = request.timeout(Duration::from_secs(1800)).send() => result,
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
                        next = stream.next() => next,
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
    headers: HeaderMap,
) -> Response {
    if let Err(e) = authorize(&g, &id, &headers) {
        return e;
    }
    if !data_path(&path) {
        return error(StatusCode::NOT_FOUND, "Not found");
    }
    let endpoint = match runtime(&g, &id) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let lease = Lease::new(id.clone(), true, false, endpoint.generation.clone());
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
    {
        let mut entries = g.shared.app_views.entries.lock().unwrap();
        let Some(entry) = entries.get_mut(&id) else {
            return error(StatusCode::GONE, "View expired");
        };
        if let Some(old) = entry.lease.replace(lease.clone()) {
            old.stop();
        }
    }
    if let Err(e) = g.shared.media_router.track(&lease) {
        return error(StatusCode::TOO_MANY_REQUESTS, e);
    }
    ws.max_message_size(256 * 1024)
        .on_upgrade(move |socket| bridge(g, socket, v.target, lease, None))
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
    let endpoint = match runtime(&g, &id) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let lease = Lease::new(id.clone(), true, false, endpoint.generation.clone());
    let credential = bearer(&headers).to_string();
    if let Err(e) = g.shared.media_router.track(&lease) {
        return error(StatusCode::TOO_MANY_REQUESTS, e);
    }
    ws.max_message_size(256 * 1024)
        .on_upgrade(move |socket| {
            bridge(
                g,
                socket,
                Source::Local(endpoint),
                lease,
                Some((id, credential)),
            )
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
    let revoked = !lease.active();
    lease.stop();
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        if revoked {
            let _ = client
                .send(Message::Text(
                    json!({"ivnc_revoked":true}).to_string().into(),
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
            check_target(t).await?;
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
            Some(websocket_connector(trusted_certificates()?)?),
        ),
    )
    .await
    .map_err(|_| "Application connection timed out")?
    .map_err(|_| "Application connection failed or access was revoked")?;
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
            let connector = websocket_connector(extra).unwrap();
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
        let credential = store.authenticate("app", "password-one").unwrap();
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
        Json(json!({"ok":true})).into_response()
    } else {
        error(StatusCode::NOT_FOUND, "Session not found")
    }
}
async fn revoke_credential(
    State(g): State<Arc<Gateway>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = authorize(&g, &id, &headers) {
        return e;
    }
    match g.apps.remote.revoke_grant(&id, bearer(&headers)) {
        Ok(()) => Json(json!({"ok":true})).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}
async fn test_connection(Json(body): Json<Value>) -> Response {
    match connect_target(&body).await {
        Ok(target) => {
            let result = check_target(&target).await;
            forget_target(&target).await;
            match result {
                Ok(info) => Json(info).into_response(),
                Err(e) => error(StatusCode::BAD_GATEWAY, e),
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
fn websocket_connector(
    extra: Vec<rustls::pki_types::CertificateDer<'static>>,
) -> Result<tokio_tungstenite::Connector, String> {
    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs()
        .certs
        .into_iter()
        .chain(extra)
    {
        roots.add(cert).map_err(|e| e.to_string())?;
    }
    // TLS builds may enable both ring and aws-lc through dependencies. Always
    // choose a provider, including when only the system CA store is used.
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| e.to_string())?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(tokio_tungstenite::Connector::Rustls(Arc::new(config)))
}

struct AccessLease {
    lease: Arc<Lease>,
    owned: bool,
    authorization: Option<(Arc<super::store::RemoteStore>, String, String)>,
}
impl AccessLease {
    async fn cancelled(&self) {
        loop {
            if let Some((store, app, credential)) = &self.authorization {
                if !store.verify(app, credential) {
                    self.lease.stop();
                }
            }
            tokio::select! {
                _ = self.lease.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_secs(1)), if self.authorization.is_some() => {},
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
