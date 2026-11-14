//! Browser sessions are independent of HTTP Basic's browser credential cache.
//! Only token digests are retained, in memory; a restart ends every session.

use super::{http_server::HttpPeerAddr, SharedState};
use crate::config::HttpConfig;
use axum::{
    body::Body,
    extract::{Extension, State},
    http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    future::Future,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::watch;

const SESSION_LIMIT: usize = 64;
const LOGIN_WINDOW: Duration = Duration::from_secs(15 * 60);
const LOGIN_FAILURE_LIMIT: u32 = 8;
const LOGIN_SOURCE_LIMIT: usize = 4096;

type TokenDigest = [u8; 32];
fn digest(text: &str) -> TokenDigest {
    Sha256::digest(text.as_bytes()).into()
}
fn equal_secret(a: &str, b: &str) -> bool {
    digest(a)
        .iter()
        .zip(digest(b))
        .fold(0u8, |v, (a, b)| v | (*a ^ b))
        == 0
}

struct Session {
    created: Instant,
    used: Instant,
    csrf: String,
}
struct Attempts {
    since: Instant,
    failures: u32,
}
struct Inner {
    password: String,
    revision: u64,
    sessions: HashMap<TokenDigest, Session>,
    attempts: HashMap<IpAddr, Attempts>,
}

pub struct AuthStore {
    enabled: bool,
    username: String,
    cookie_name: String,
    idle: Duration,
    maximum: Duration,
    inner: Mutex<Inner>,
    changes: watch::Sender<u64>,
}

#[derive(Clone)]
enum Identity {
    Disabled,
    Basic(u64),
    Session(TokenDigest, u64),
}

/// Attached once by the HTTP middleware and carried into long-lived sockets.
#[derive(Clone)]
pub struct AuthContext {
    store: Arc<AuthStore>,
    identity: Identity,
}

/// Set by the accepted transport, never by untrusted Forwarded headers.
#[derive(Clone, Copy, Default)]
pub struct TransportSecurity(pub bool);

impl AuthStore {
    pub fn new(config: &HttpConfig) -> Self {
        Self {
            enabled: config.basic_auth_enabled,
            username: config.basic_auth_user.clone(),
            cookie_name: format!("ivnc_session_{}", config.port),
            idle: Duration::from_secs(config.auth_session_idle_seconds),
            maximum: Duration::from_secs(config.auth_session_max_seconds),
            inner: Mutex::new(Inner {
                password: config.basic_auth_password.clone(),
                revision: 0,
                sessions: HashMap::new(),
                attempts: HashMap::new(),
            }),
            changes: watch::channel(0).0,
        }
    }

    fn disabled(self: &Arc<Self>) -> AuthContext {
        AuthContext {
            store: self.clone(),
            identity: Identity::Disabled,
        }
    }

    fn session_valid(&self, session: &Session) -> bool {
        session.used.elapsed() < self.idle && session.created.elapsed() < self.maximum
    }

    fn cookie_name(&self, headers: &HeaderMap) -> String {
        // Host carries the published port, unlike config.port inside Docker.
        // Cookies do not have port isolation, so give each visible origin a name.
        headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .map(|host| {
                let hash = format!("{:x}", Sha256::digest(host.to_ascii_lowercase().as_bytes()));
                format!("ivnc_session_{}", &hash[..16])
            })
            .unwrap_or_else(|| self.cookie_name.clone())
    }

    fn cookie_token<'a>(&self, headers: &'a HeaderMap) -> Option<&'a str> {
        let cookie_name = self.cookie_name(headers);
        let mut found = None;
        for value in headers.get_all(header::COOKIE) {
            for pair in value.to_str().unwrap_or("").split(';') {
                if let Some((name, value)) = pair.trim().split_once('=') {
                    if name == cookie_name {
                        // Ambiguous cookies must not select whichever happens to come first.
                        if found.is_some() {
                            return Some("");
                        }
                        found = Some(value);
                    }
                }
            }
        }
        found
    }

    fn authenticate(self: &Arc<Self>, headers: &HeaderMap) -> Option<AuthContext> {
        if !self.enabled {
            return Some(self.disabled());
        }
        let mut inner = self.inner.lock().unwrap();
        let revision = inner.revision;
        if let Some(token) = self.cookie_token(headers) {
            // An expired or signed-out browser must never fall back to cached Basic credentials.
            if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let key = digest(token);
            let session = inner.sessions.get_mut(&key)?;
            if !self.session_valid(session) {
                inner.sessions.remove(&key);
                return None;
            }
            session.used = Instant::now();
            return Some(AuthContext {
                store: self.clone(),
                identity: Identity::Session(key, revision),
            });
        }
        let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
        let (scheme, encoded) = value.split_once(' ')?;
        if !scheme.eq_ignore_ascii_case("basic") || encoded.len() > 8192 {
            return None;
        }
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()?;
        let decoded = String::from_utf8(decoded).ok()?;
        let (username, password) = decoded.split_once(':')?;
        if !(equal_secret(username, &self.username) & equal_secret(password, &inner.password)) {
            return None;
        }
        Some(AuthContext {
            store: self.clone(),
            identity: Identity::Basic(revision),
        })
    }

    fn login(
        self: &Arc<Self>,
        ip: IpAddr,
        username: &str,
        password: &str,
    ) -> Result<(String, AuthContext), StatusCode> {
        let mut inner = self.inner.lock().unwrap();
        inner
            .attempts
            .retain(|_, a| a.since.elapsed() < LOGIN_WINDOW);
        if !inner.attempts.contains_key(&ip) && inner.attempts.len() >= LOGIN_SOURCE_LIMIT {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        let attempt = inner.attempts.entry(ip).or_insert(Attempts {
            since: Instant::now(),
            failures: 0,
        });
        if attempt.failures >= LOGIN_FAILURE_LIMIT {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        // Reserve the attempt before checking credentials, including concurrent attempts.
        attempt.failures += 1;
        if !(equal_secret(username, &self.username) & equal_secret(password, &inner.password)) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        inner.attempts.remove(&ip);
        inner.sessions.retain(|_, s| self.session_valid(s));
        if inner.sessions.len() >= SESSION_LIMIT {
            if let Some(oldest) = inner
                .sessions
                .iter()
                .min_by_key(|(_, s)| s.used)
                .map(|(k, _)| *k)
            {
                inner.sessions.remove(&oldest);
            }
            self.changes.send_modify(|v| *v = v.wrapping_add(1));
        }
        let token = crate::remote::token();
        let key = digest(&token);
        inner.sessions.insert(
            key,
            Session {
                created: Instant::now(),
                used: Instant::now(),
                csrf: crate::remote::token(),
            },
        );
        Ok((
            token,
            AuthContext {
                store: self.clone(),
                identity: Identity::Session(key, inner.revision),
            },
        ))
    }

    pub fn change_password(&self, password: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.password = password.to_string();
        inner.revision = inner.revision.wrapping_add(1);
        inner.sessions.clear();
        self.changes.send_modify(|v| *v = v.wrapping_add(1));
    }

    fn logout(&self, headers: &HeaderMap) {
        if let Some(token) = self.cookie_token(headers) {
            self.inner.lock().unwrap().sessions.remove(&digest(token));
        }
        self.changes.send_modify(|v| *v = v.wrapping_add(1));
    }

    fn set_cookie(&self, token: &str, secure: bool, headers: &HeaderMap) -> HeaderValue {
        HeaderValue::from_str(&format!(
            "{}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{}",
            self.cookie_name(headers),
            self.maximum.as_secs(),
            if secure { "; Secure" } else { "" }
        ))
        .unwrap()
    }

    /// iVnc's management session is never an upstream application's credential.
    pub fn strip_cookie(&self, headers: &mut HeaderMap) {
        let retained = headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(';'))
            .map(str::trim)
            .filter(|p| {
                p.split_once('=')
                    .is_some_and(|(name, _)| !name.starts_with("ivnc_session_"))
            })
            .collect::<Vec<_>>()
            .join("; ");
        headers.remove(header::COOKIE);
        if !retained.is_empty() {
            if let Ok(value) = retained.parse() {
                headers.insert(header::COOKIE, value);
            }
        }
    }

    pub fn is_own_cookie(&self, value: &HeaderValue) -> bool {
        value
            .to_str()
            .ok()
            .and_then(|v| v.split_once('='))
            .is_some_and(|(name, _)| name.trim().starts_with("ivnc_session_"))
    }
}

impl AuthContext {
    pub fn csrf(&self) -> Option<String> {
        let Identity::Session(key, _) = self.identity else {
            return None;
        };
        self.store
            .inner
            .lock()
            .unwrap()
            .sessions
            .get(&key)
            .filter(|s| self.store.session_valid(s))
            .map(|s| s.csrf.clone())
    }

    pub fn is_session(&self) -> bool {
        matches!(self.identity, Identity::Session(..))
    }

    fn remaining(&self) -> Option<Duration> {
        let inner = self.store.inner.lock().unwrap();
        match self.identity {
            Identity::Disabled => Some(Duration::MAX),
            Identity::Basic(revision) if revision == inner.revision => Some(Duration::MAX),
            Identity::Session(key, revision) if revision == inner.revision => {
                inner.sessions.get(&key).and_then(|s| {
                    let remaining = self
                        .store
                        .idle
                        .saturating_sub(s.used.elapsed())
                        .min(self.store.maximum.saturating_sub(s.created.elapsed()));
                    (!remaining.is_zero()).then_some(remaining)
                })
            }
            _ => None,
        }
    }

    pub async fn cancelled(&self) {
        let mut changes = self.store.changes.subscribe();
        loop {
            let Some(remaining) = self.remaining() else {
                return;
            };
            tokio::select! {
                _ = changes.changed() => {},
                _ = tokio::time::sleep(remaining.min(Duration::from_secs(3600))) => {},
            }
        }
    }

    pub async fn run<F: Future>(&self, future: F) -> Option<F::Output> {
        tokio::select! { biased; _ = self.cancelled() => None, value = future => Some(value) }
    }
}

/// Full origin comparison (including scheme/default port); no forwarded-header trust.
pub fn same_origin(headers: &HeaderMap, secure: bool, required: bool) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return !required;
    };
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let expected = reqwest::Url::parse(&format!(
        "{}://{host}",
        if secure { "https" } else { "http" }
    ));
    let actual = origin
        .to_str()
        .ok()
        .and_then(|v| reqwest::Url::parse(v).ok());
    matches!((expected, actual), (Ok(a), Some(b)) if a.origin() == b.origin() && b.username().is_empty() && b.password().is_none() && b.path() == "/" && b.query().is_none() && b.fragment().is_none())
}

fn csrf_matches(context: &AuthContext, headers: &HeaderMap) -> bool {
    match (
        context.csrf(),
        headers.get("x-ivnc-csrf").and_then(|v| v.to_str().ok()),
    ) {
        (Some(expected), Some(actual)) => equal_secret(actual, &expected),
        _ => false,
    }
}

fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    response
}

fn failure(status: StatusCode, code: &str) -> Response {
    no_store((status, Json(json!({"error":code}))).into_response())
}

fn public_path(path: &str) -> bool {
    matches!(
        path,
        "/health"
            | "/version"
            | "/manifest.json"
            | "/sw.js"
            | "/auth/login"
            | "/auth/session"
            | "/auth/logout"
            | "/login.html"
            | "/login.js"
            | "/auth-client.js"
    ) || path.starts_with("/icons/")
        || path.starts_with("/assets/")
}

fn browser_request(headers: &HeaderMap) -> bool {
    headers.contains_key("sec-fetch-mode")
        || headers.contains_key(header::ORIGIN)
        || headers.contains_key(header::COOKIE)
        // Fetch Metadata is absent on ordinary, untrusted HTTP IP origins.
        // A same-origin GET may also omit Origin, including after expiry.
        || headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("Mozilla/"))
        || headers
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("text/html"))
}

fn navigation(req: &Request<Body>) -> bool {
    matches!(*req.method(), Method::GET | Method::HEAD)
        && !req.headers().contains_key(header::UPGRADE)
        && (req
            .headers()
            .get("sec-fetch-mode")
            .is_some_and(|v| v == "navigate")
            || req
                .headers()
                .get(header::ACCEPT)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.contains("text/html")))
}

fn login_location(uri: &str) -> String {
    let mut url = reqwest::Url::parse("http://localhost/auth/login").unwrap();
    url.query_pairs_mut().append_pair("next", uri);
    format!("{}?{}", url.path(), url.query().unwrap())
}

pub async fn middleware(
    State(state): State<Arc<SharedState>>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    if req.uri().path().starts_with("/remote/v1/apps/") || public_path(req.uri().path()) {
        return next.run(req).await;
    }
    let secure = req
        .extensions()
        .get::<TransportSecurity>()
        .copied()
        .unwrap_or_default()
        .0;
    if let Some(context) = state.auth.authenticate(req.headers()) {
        let websocket = req
            .headers()
            .get(header::UPGRADE)
            .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"websocket"));
        let unsafe_method = !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
        if context.is_session() && (websocket || unsafe_method) {
            if !same_origin(req.headers(), secure, true) {
                return failure(StatusCode::FORBIDDEN, "origin_invalid");
            }
            // Miao has its own CSRF protocol, validated by its upstream server.
            let miao = super::http_server::is_miao_request(req.uri().path(), req.headers());
            if unsafe_method && !miao && !csrf_matches(&context, req.headers()) {
                return failure(StatusCode::FORBIDDEN, "csrf_invalid");
            }
        } else if state.config.http.basic_auth_enabled
            && (websocket || unsafe_method)
            && !same_origin(req.headers(), secure, false)
        {
            return failure(StatusCode::FORBIDDEN, "origin_invalid");
        }
        req.extensions_mut().insert(context);
        let response = next.run(req).await;
        return no_store(response);
    }
    if navigation(&req) {
        return no_store(
            (
                StatusCode::SEE_OTHER,
                [(header::LOCATION, login_location(&req.uri().to_string()))],
            )
                .into_response(),
        );
    }
    let mut response = failure(StatusCode::UNAUTHORIZED, "authentication_required");
    response
        .headers_mut()
        .insert("x-ivnc-auth", HeaderValue::from_static("required"));
    if !browser_request(req.headers()) {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Basic realm=\"ivnc\""),
        );
    }
    response
}

pub async fn login_page() -> Response {
    let mut response = if super::embedded_assets::has_embedded_assets()
        && std::env::var("IVNC_WEB_ROOT").is_err()
    {
        super::embedded_assets::get_embedded_file("login.html")
    } else {
        let root = std::env::var("IVNC_WEB_ROOT").unwrap_or_else(|_| "web/ivnc".into());
        match tokio::fs::read(std::path::Path::new(&root).join("login.html")).await {
            Ok(html) => {
                ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response()
            }
            Err(_) => StatusCode::NOT_FOUND.into_response(),
        }
    };
    response.headers_mut().insert("content-security-policy", HeaderValue::from_static("default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; img-src 'self'; connect-src 'self'; base-uri 'self'; frame-ancestors 'self'; form-action 'self'"));
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    no_store(response)
}

#[derive(Deserialize)]
pub struct Login {
    username: String,
    password: String,
}

pub(crate) async fn login(
    State(state): State<Arc<SharedState>>,
    Extension(transport): Extension<TransportSecurity>,
    Extension(peer): Extension<HttpPeerAddr>,
    headers: HeaderMap,
    Json(body): Json<Login>,
) -> Response {
    if !same_origin(&headers, transport.0, true) {
        return failure(StatusCode::FORBIDDEN, "origin_invalid");
    }
    if !state.config.http.basic_auth_enabled {
        return no_store(Json(json!({"ok":true})).into_response());
    }
    if body.username.len() > 256 || body.password.len() > 4096 {
        return failure(StatusCode::BAD_REQUEST, "credentials_too_long");
    }
    match state
        .auth
        .login(peer.0.ip(), &body.username, &body.password)
    {
        Ok((token, context)) => {
            // Replace a previous browser session without allowing session fixation.
            state.auth.logout(&headers);
            let mut response =
                no_store(Json(json!({"ok":true,"csrf":context.csrf()})).into_response());
            response.headers_mut().insert(
                header::SET_COOKIE,
                state.auth.set_cookie(&token, transport.0, &headers),
            );
            response
        }
        Err(status) => {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let mut response = failure(
                status,
                if status == StatusCode::TOO_MANY_REQUESTS {
                    "login_rate_limited"
                } else {
                    "invalid_credentials"
                },
            );
            if status == StatusCode::TOO_MANY_REQUESTS {
                response
                    .headers_mut()
                    .insert(header::RETRY_AFTER, HeaderValue::from_static("900"));
            }
            response
        }
    }
}

pub async fn status(State(state): State<Arc<SharedState>>, headers: HeaderMap) -> Response {
    let context = state.auth.authenticate(&headers);
    no_store(Json(json!({"enabled":state.config.http.basic_auth_enabled,"authenticated":context.is_some(),
        "kind":context.as_ref().map(|c| match c.identity { Identity::Disabled=>"disabled", Identity::Basic(_)=>"basic", Identity::Session(..)=>"session" }),
        "csrf":context.as_ref().and_then(AuthContext::csrf)})).into_response())
}

pub async fn logout(
    State(state): State<Arc<SharedState>>,
    Extension(transport): Extension<TransportSecurity>,
    headers: HeaderMap,
) -> Response {
    if !same_origin(&headers, transport.0, true) {
        return failure(StatusCode::FORBIDDEN, "origin_invalid");
    }
    if let Some(context) = state.auth.authenticate(&headers) {
        if context.is_session() && !csrf_matches(&context, &headers) {
            return failure(StatusCode::FORBIDDEN, "csrf_invalid");
        }
    }
    state.auth.logout(&headers);
    let mut response = no_store(Json(json!({"ok":true})).into_response());
    // Retain a signed-out marker so cached Basic credentials cannot undo logout.
    response.headers_mut().insert(
        header::SET_COOKIE,
        state.auth.set_cookie("signed-out", transport.0, &headers),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> Arc<AuthStore> {
        Arc::new(AuthStore::new(&HttpConfig::default()))
    }
    fn basic() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            header::AUTHORIZATION,
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode("user:mypasswd")
            )
            .parse()
            .unwrap(),
        );
        h
    }
    fn cookie(store: &AuthStore, token: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            format!("{}={token}", store.cookie_name).parse().unwrap(),
        );
        h
    }
    #[test]
    fn sessions_survive_requests_without_basic_and_do_not_store_raw_tokens() {
        let s = store();
        let (token, c) = s
            .login("127.0.0.1".parse().unwrap(), "user", "mypasswd")
            .unwrap();
        assert!(s.authenticate(&cookie(&s, &token)).is_some());
        assert!(c.csrf().is_some());
        assert!(s
            .inner
            .lock()
            .unwrap()
            .sessions
            .contains_key(&digest(&token)));
        assert!(s.authenticate(&HeaderMap::new()).is_none());
    }
    #[tokio::test]
    async fn logout_and_password_changes_revoke_connections_and_block_basic_fallback() {
        let s = store();
        let basic_context = s.authenticate(&basic()).unwrap();
        let (token, c) = s
            .login("127.0.0.1".parse().unwrap(), "user", "mypasswd")
            .unwrap();
        s.logout(&cookie(&s, &token));
        tokio::time::timeout(Duration::from_millis(100), c.cancelled())
            .await
            .unwrap();
        let mut h = basic();
        h.insert(
            header::COOKIE,
            format!("{}=signed-out", s.cookie_name).parse().unwrap(),
        );
        assert!(s.authenticate(&h).is_none());
        assert!(s.authenticate(&basic()).is_some());
        s.change_password("new-password");
        tokio::time::timeout(Duration::from_millis(100), basic_context.cancelled())
            .await
            .unwrap();
        assert!(s.authenticate(&basic()).is_none());
        assert!(s
            .login("127.0.0.1".parse().unwrap(), "user", "new-password")
            .is_ok());
    }
    #[test]
    fn expirations_are_idle_and_absolute_and_sessions_are_bounded() {
        let s = store();
        let ip = "127.0.0.1".parse().unwrap();
        let (old, c) = s.login(ip, "user", "mypasswd").unwrap();
        s.inner
            .lock()
            .unwrap()
            .sessions
            .get_mut(&digest(&old))
            .unwrap()
            .used = Instant::now() - s.idle;
        assert!(c.remaining().is_none());
        assert!(s.authenticate(&cookie(&s, &old)).is_none());
        let (old, c) = s.login(ip, "user", "mypasswd").unwrap();
        s.inner
            .lock()
            .unwrap()
            .sessions
            .get_mut(&digest(&old))
            .unwrap()
            .created = Instant::now() - s.maximum;
        assert!(c.remaining().is_none());
        for _ in 0..SESSION_LIMIT + 1 {
            s.login(ip, "user", "mypasswd").unwrap();
        }
        assert_eq!(s.inner.lock().unwrap().sessions.len(), SESSION_LIMIT);
    }
    #[test]
    fn failed_logins_are_source_limited_without_locking_out_other_sources() {
        let s = store();
        let ip = "127.0.0.1".parse().unwrap();
        for _ in 0..LOGIN_FAILURE_LIMIT {
            assert!(matches!(
                s.login(ip, "user", "wrong"),
                Err(StatusCode::UNAUTHORIZED)
            ));
        }
        assert!(matches!(
            s.login(ip, "user", "mypasswd"),
            Err(StatusCode::TOO_MANY_REQUESTS)
        ));
        assert!(s
            .login("127.0.0.2".parse().unwrap(), "user", "mypasswd")
            .is_ok());
    }
    #[test]
    fn origin_csrf_cookie_attributes_and_proxy_filtering() {
        let s = store();
        let (token, c) = s
            .login("127.0.0.1".parse().unwrap(), "user", "mypasswd")
            .unwrap();
        let mut h = cookie(&s, &token);
        h.insert(header::HOST, "example.test:8007".parse().unwrap());
        h.insert(header::ORIGIN, "http://example.test:8007".parse().unwrap());
        assert!(same_origin(&h, false, true));
        assert!(!same_origin(&h, true, true));
        assert!(!csrf_matches(&c, &h));
        h.insert("x-ivnc-csrf", c.csrf().unwrap().parse().unwrap());
        assert!(csrf_matches(&c, &h));
        h.insert(header::ORIGIN, "http://foreign.test:8007".parse().unwrap());
        assert!(!same_origin(&h, false, true));
        assert!(!s
            .set_cookie(&token, false, &h)
            .to_str()
            .unwrap()
            .contains("Secure"));
        assert!(s
            .set_cookie(&token, true, &h)
            .to_str()
            .unwrap()
            .contains("; Secure"));
        h.append(
            header::COOKIE,
            "miao_session=keep; unrelated=keep".parse().unwrap(),
        );
        s.strip_cookie(&mut h);
        assert_eq!(h[header::COOKIE], "miao_session=keep; unrelated=keep");
    }
    #[test]
    fn duplicate_cookies_and_new_process_sessions_are_rejected() {
        let s = store();
        let (token, _) = s
            .login("127.0.0.1".parse().unwrap(), "user", "mypasswd")
            .unwrap();
        let mut h = cookie(&s, &token);
        assert!(store().authenticate(&h).is_none());
        h.append(header::COOKIE, h[header::COOKIE].clone());
        assert!(s.authenticate(&h).is_none());
    }

    #[test]
    fn published_ports_have_independent_cookie_names() {
        let s = store();
        let mut a = HeaderMap::new();
        a.insert(header::HOST, "192.0.2.1:18007".parse().unwrap());
        let mut b = a.clone();
        b.insert(header::HOST, "192.0.2.1:28007".parse().unwrap());
        assert_ne!(s.cookie_name(&a), s.cookie_name(&b));
    }

    #[test]
    fn http_browsers_without_fetch_metadata_do_not_receive_basic_challenges() {
        let mut headers = HeaderMap::new();
        headers.insert(header::USER_AGENT, "curl/8.0".parse().unwrap());
        assert!(!browser_request(&headers));
        headers.insert(
            header::USER_AGENT,
            "Mozilla/5.0 (iPhone) AppleWebKit/605.1.15".parse().unwrap(),
        );
        assert!(browser_request(&headers));
        headers.remove(header::USER_AGENT);
        headers.insert(
            header::COOKIE,
            "ivnc_session_test=signed-out".parse().unwrap(),
        );
        assert!(browser_request(&headers));
    }

    #[tokio::test]
    async fn an_open_connection_expires_without_another_http_request() {
        let config = HttpConfig {
            auth_session_idle_seconds: 1,
            auth_session_max_seconds: 2,
            ..HttpConfig::default()
        };
        let s = Arc::new(AuthStore::new(&config));
        let (_, context) = s
            .login("127.0.0.1".parse().unwrap(), "user", "mypasswd")
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), context.cancelled())
            .await
            .unwrap();
    }
}
