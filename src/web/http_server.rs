//! HTTP server with same-port ICE-TCP protocol splitting
//!
//! Provides health check endpoints, metrics, WebRTC signaling WebSocket,
//! and same-port HTTP + ICE-TCP multiplexing. Incoming TCP connections are
//! classified by peeking the first byte:
//! - ASCII letters (HTTP methods) → axum HTTP handler
//! - 0x00-0x03 (ICE) or 0x14-0x17 (DTLS) → WebRTC ICE-TCP session

#![allow(dead_code)]

use crate::web::embedded_assets::{get_embedded_file, has_embedded_assets};
use crate::web::shared::SharedState;
#[cfg(feature = "agent")]
use axum::extract::Path;
use axum::routing::put;
use axum::{
    body::{to_bytes, Body, Bytes},
    extract::ws::{Message as AxumWsMessage, WebSocket as AxumWebSocket},
    extract::{Query, State, WebSocketUpgrade},
    http::{header, HeaderMap, Method, Request, StatusCode, Uri},
    middleware,
    response::{IntoResponse, Response},
    routing::{any, get, post},
    Router,
};

use base64::Engine;
use hyper_util::rt::TokioIo;
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
#[cfg(feature = "tls")]
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tower::Service;
use tower_http::services::{ServeDir, ServeFile};

use crate::apps::api::AppsState;
use crate::webrtc::SessionManager;

/// Classify a TCP connection by its first bytes.
fn classify_first_bytes(buf: &[u8]) -> ConnectionType {
    if buf.is_empty() {
        return ConnectionType::Unknown;
    }
    if looks_like_http(buf) {
        return ConnectionType::Http;
    }
    let b0 = buf[0];
    // DTLS/TLS handshake record type (0x16) needs version disambiguation.
    if b0 == 0x16 {
        if buf.len() >= 2 {
            let b1 = buf[1];
            if b1 == 0x03 {
                return ConnectionType::Tls; // TLS record
            }
            if b1 == 0xFE {
                return ConnectionType::IceTcp; // DTLS record
            }
        }
        // Ambiguous: default to ICE/DTLS to avoid misrouting DTLS to HTTPS.
        return ConnectionType::IceTcp;
    }
    // ICE/DTLS record types (ChangeCipherSpec, Alert, Handshake, ApplicationData)
    if (0x00..=0x03).contains(&b0) || (0x14..=0x17).contains(&b0) {
        return ConnectionType::IceTcp;
    }
    ConnectionType::Unknown
}

fn looks_like_http(buf: &[u8]) -> bool {
    const METHODS: [&[u8]; 9] = [
        b"GET", b"POST", b"HEAD", b"PUT", b"PATCH", b"DELETE", b"OPTIONS", b"CONNECT", b"TRACE",
    ];
    if METHODS.iter().any(|m| buf.starts_with(m)) {
        return true;
    }
    // If we only have a few bytes, treat an initial ASCII letter as a partial HTTP method.
    buf.len() < 3 && buf.first().is_some_and(|b| b.is_ascii_alphabetic())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionType {
    Http,
    IceTcp,
    Tls,
    Unknown,
}

/// Run the HTTP server with WebRTC signaling support and same-port ICE-TCP
pub async fn run_http_server_with_webrtc(
    port: u16,
    state: Arc<SharedState>,
    session_manager: Option<Arc<SessionManager>>,
    enable_tls: bool,
    apps_state: Option<Arc<AppsState>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr = format!("0.0.0.0:{}", port);

    // Check for embedded assets first, then fall back to filesystem
    let use_embedded = has_embedded_assets() && std::env::var("IVNC_WEB_ROOT").is_err();

    if use_embedded {
        info!("Serving web UI from embedded assets");
    } else {
        let static_root = std::env::var("IVNC_WEB_ROOT").unwrap_or_else(|_| "web/ivnc".to_string());
        let cwd = std::env::current_dir().ok();
        let index_path = PathBuf::from(&static_root).join("index.html");
        info!("Serving web UI from {:?} (cwd: {:?})", static_root, cwd);
        if !index_path.exists() {
            info!("Web UI index not found at {:?}", index_path);
        }
    }

    // Build router
    let mut app = Router::new()
        .route("/", get(index_handler))
        .route("/index.html", get(index_handler))
        .route("/connect", get(index_handler))
        .route("/health", get(health_handler))
        .route("/version", get(public_version_handler))
        .route("/metrics", get(metrics_handler))
        .route("/clients", get(clients_handler))
        .route("/api/render-status", get(render_status_handler))
        .route("/ui-config", get(ui_config_handler))
        .route("/ws-config", get(ws_config_handler))
        .route("/api/change-password", post(change_password_handler))
        .route("/api/version", get(get_version_handler))
        .route("/api/restart", post(restart_handler))
        .route("/api/upgrade/ws", get(upgrade_ws_handler))
        .route("/api/connections", get(connections_handler))
        .route("/api/connections/{id}/disconnect", post(disconnect_handler))
        .route("/api/ipv4", get(ipv4_handler))
        .route("/api/proxy/status", get(proxy_status_handler))
        .route("/api/proxy/restart", post(proxy_restart_handler))
        .route("/proxy", get(proxy_panel_root_handler))
        .route("/proxy/", any(proxy_panel_handler))
        .route("/proxy/{*path}", any(proxy_panel_handler))
        .route("/proxy-ws/{*path}", get(proxy_panel_ws_handler))
        .route("/terminal/ws", get(terminal_ws_handler));

    app = app
        .route("/api/console/overview", get(console_overview_handler))
        .route(
            "/api/console/settings",
            get(console_settings_get_handler).put(console_settings_put_handler),
        )
        .route("/api/console/keyframe", post(console_keyframe_handler))
        .route(
            "/api/console/agent-config",
            get(console_agent_get_handler).put(console_agent_put_handler),
        )
        .route("/api/console/agent-stop", post(console_agent_stop_handler))
        .route("/api/console/agent-start", post(console_agent_start_handler))
        .route(
            "/api/console/schedules",
            get(console_schedules_get_handler).post(console_schedules_post_handler),
        )
        .route(
            "/api/console/schedules/{id}",
            put(console_schedule_put_handler).delete(console_schedule_delete_handler),
        )
        .route(
            "/api/console/schedules/{id}/run-now",
            post(console_schedule_run_now_handler),
        )
        .route("/api/console/providers", get(console_providers_get_handler))
        .route(
            "/api/console/providers/{name}",
            put(console_provider_put_handler),
        )
        .route("/api/console/agent-runs", get(console_agent_runs_handler))
        .route(
            "/api/console/agent-runs/{run_id}/steps",
            get(console_agent_run_steps_handler),
        )
        .route(
            "/api/console/agent-runs/{run_id}/frames/{name}",
            get(console_agent_run_frame_handler),
        );

    // Add WebRTC signaling endpoint if session manager is provided
    if let Some(ref manager) = session_manager {
        info!("Adding WebRTC signaling endpoint at /webrtc");
        let state_clone = state.clone();
        let manager_clone = manager.clone();
        let signaling_handler = move |headers: axum::http::HeaderMap, ws: WebSocketUpgrade| {
            let state = state_clone.clone();
            let manager = manager_clone.clone();
            let host_str = headers
                .get(axum::http::header::HOST)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            async move {
                ws.on_upgrade(move |socket| async move {
                    crate::transport::handle_signaling_connection(socket, state, manager, host_str)
                        .await;
                })
            }
        };
        app = app
            .route("/webrtc", get(signaling_handler.clone()))
            .route("/webrtc/signaling", get(signaling_handler.clone()))
            .route("/webrtc/signaling/", get(signaling_handler.clone()))
            .route("/{app}/signaling", get(signaling_handler.clone()))
            .route("/{app}/signaling/", get(signaling_handler));
    }

    // MCP Streamable HTTP endpoint
    #[cfg(feature = "mcp")]
    {
        let mcp_state = state.clone();
        let mcp_session_mgr = Arc::new(
            rmcp::transport::streamable_http_server::session::local::LocalSessionManager::default(),
        );
        let mcp_config = rmcp::transport::streamable_http_server::StreamableHttpServerConfig {
            stateful_mode: true,
            ..Default::default()
        };
        let mcp_service = rmcp::transport::streamable_http_server::StreamableHttpService::new(
            move || Ok(crate::mcp::McpServer::new(mcp_state.clone())),
            mcp_session_mgr,
            mcp_config,
        );
        app = app.route_service("/mcp", mcp_service);
        info!("MCP Streamable HTTP endpoint enabled at /mcp");
    }

    // Apps management routes
    if let Some(_apps) = &apps_state {
        app = app.route("/console", get(console_handler));
        info!("Apps console enabled at /console");
    }

    // Set up fallback for static files
    let auth_state = state.clone();
    let metrics_state = state.clone(); // keep a copy for the accept loop (metrics)
    let mut app: Router<()> = if use_embedded {
        app.fallback(embedded_fallback_handler).with_state(state)
    } else {
        let static_root = std::env::var("IVNC_WEB_ROOT").unwrap_or_else(|_| "web/ivnc".to_string());
        let index_path = PathBuf::from(&static_root).join("index.html");
        let static_service = ServeDir::new(&static_root).fallback(ServeFile::new(index_path));
        app.fallback_service(static_service).with_state(state)
    };

    // Merge apps routes after with_state (both are Router<()> now)
    if let Some(ref apps) = apps_state {
        app = app.merge(crate::apps::api::router(apps.clone()));
    }

    let app = app
        .layer(middleware::from_fn_with_state(
            auth_state.clone(),
            proxy_panel_absolute_path_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            auth_state,
            basic_auth_middleware,
        ));

    let listener = TcpListener::bind(&addr).await?;
    let local_addr = listener.local_addr()?;

    // TLS setup
    #[cfg(feature = "tls")]
    let tls_acceptor = if enable_tls {
        let acceptor = create_tls_acceptor()?;
        info!("HTTPS+ICE-TCP server listening on https://{}", local_addr);
        Some(acceptor)
    } else {
        info!("HTTP+ICE-TCP server listening on http://{}", local_addr);
        None
    };
    #[cfg(not(feature = "tls"))]
    {
        let _ = enable_tls;
        info!("HTTP+ICE-TCP server listening on http://{}", local_addr);
    }

    if session_manager.is_some() {
        info!("Same-port ICE-TCP multiplexing enabled on :{}", port);
    }

    // Start IPv4 fetching task
    let ipv4_state = metrics_state.clone();
    tokio::spawn(async move {
        let mut last_ip = String::new();
        loop {
            match fetch_ipv4().await {
                Ok(ip) if ip != last_ip => {
                    *ipv4_state.ipv4_address.write().await = ip.clone();
                    ipv4_state.send_text(format!("ipv4,{}", ip));
                    info!("IPv4 address updated: {}", ip);
                    last_ip = ip;
                }
                Err(e) => debug!("Failed to fetch IPv4: {}", e),
                _ => {}
            }
            tokio::time::sleep(Duration::from_secs(300)).await;
        }
    });

    // Accept loop with first-byte protocol splitting
    loop {
        let (tcp_stream, peer_addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                warn!("TCP accept error: {}", e);
                continue;
            }
        };

        let app = app.clone();
        let sm = session_manager.clone();
        let conn_state = metrics_state.clone();
        #[cfg(feature = "tls")]
        let tls_acceptor = tls_acceptor.clone();

        tokio::spawn(async move {
            let mut first_bytes = vec![0u8; 8];
            let peek_result = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                tcp_stream.peek(&mut first_bytes),
            )
            .await;
            let n = match peek_result {
                Ok(Ok(0)) | Err(_) => return,
                Ok(Ok(n)) => n,
                Ok(Err(e)) => {
                    debug!("Peek error from {}: {}", peer_addr, e);
                    return;
                }
            };
            first_bytes.truncate(n);
            // If we only got the first byte and it's a TLS/DTLS handshake marker,
            // try a quick re-peek to disambiguate version (0x03 vs 0xFE).
            if first_bytes.len() < 2 && first_bytes.first() == Some(&0x16) {
                let mut retry_buf = vec![0u8; 8];
                if let Ok(Ok(n2)) = tokio::time::timeout(
                    std::time::Duration::from_millis(50),
                    tcp_stream.peek(&mut retry_buf),
                )
                .await
                {
                    if n2 > 0 {
                        retry_buf.truncate(n2);
                        first_bytes = retry_buf;
                    }
                }
            }
            let kind = classify_first_bytes(&first_bytes);
            debug!(
                "Connection from {} classified as {:?} (first_bytes={:02x?})",
                peer_addr, kind, &first_bytes
            );

            // Record protocol classification metric
            conn_state.record_protocol_classification(match kind {
                ConnectionType::Http => "http",
                ConnectionType::IceTcp => "ice_tcp",
                ConnectionType::Tls => "tls",
                ConnectionType::Unknown => "unknown",
            });

            // In TLS mode: disambiguate TLS vs DTLS by version bytes
            #[cfg(feature = "tls")]
            if let Some(ref acceptor) = tls_acceptor {
                if kind == ConnectionType::Tls {
                    // TLS handshake
                    match acceptor.accept(tcp_stream).await {
                        Ok(tls_stream) => {
                            serve_http(TokioIo::new(tls_stream), app).await;
                        }
                        Err(e) => {
                            debug!("TLS handshake error from {}: {}", peer_addr, e);
                        }
                    }
                    return;
                }
                // Non-TLS: route HTTP if detected, otherwise ICE-TCP
                match kind {
                    ConnectionType::Http => {
                        debug!("Rejecting plaintext HTTP on TLS port from {}", peer_addr);
                        let body = "HTTPS required";
                        let response = format!(
                            "HTTP/1.1 426 Upgrade Required\r\nConnection: close\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = tcp_stream.write_all(response.as_bytes()).await;
                        let _ = tcp_stream.shutdown().await;
                    }
                    ConnectionType::IceTcp | ConnectionType::Unknown | ConnectionType::Tls => {
                        handle_ice_connection(tcp_stream, peer_addr, sm).await;
                    }
                }
                return;
            }

            // Non-TLS mode: first-byte classification
            match kind {
                ConnectionType::IceTcp => handle_ice_connection(tcp_stream, peer_addr, sm).await,
                ConnectionType::Http | ConnectionType::Tls => {
                    serve_http(TokioIo::new(tcp_stream), app).await;
                }
                ConnectionType::Unknown => {
                    warn!(
                        "Unrecognized protocol from {} (first_bytes={:02x?}), closing",
                        peer_addr, &first_bytes
                    );
                }
            }
        });
    }
}

/// Serve HTTP over a generic IO stream
async fn serve_http<I>(io: TokioIo<I>, app: Router<()>)
where
    I: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let service = hyper::service::service_fn(move |req| {
        let mut app = app.clone();
        async move { app.call(req).await }
    });
    let _ = hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
        .serve_connection_with_upgrades(io, service)
        .await;
}

/// Handle an ICE-TCP connection
async fn handle_ice_connection(
    mut stream: tokio::net::TcpStream,
    peer_addr: std::net::SocketAddr,
    sm: Option<Arc<SessionManager>>,
) {
    if let Some(sm) = sm {
        let ice_local_addr = sm.listen_addr();
        let mut buf = vec![0u8; 8192];
        match stream.read(&mut buf).await {
            Ok(0) => return,
            Ok(n) => {
                buf.truncate(n);
                info!(
                    "ICE-TCP connection from {} ({} bytes, first16={:02x?})",
                    peer_addr,
                    n,
                    &buf[..n.min(16)]
                );
                if let Err(e) = sm
                    .handle_ice_tcp_connection(stream, peer_addr, ice_local_addr, &buf)
                    .await
                {
                    warn!("ICE-TCP session match failed from {}: {}", peer_addr, e);
                }
            }
            Err(e) => {
                debug!("ICE-TCP read error from {}: {}", peer_addr, e);
            }
        }
    } else {
        debug!(
            "ICE-TCP connection from {} but no session manager",
            peer_addr
        );
    }
}

#[cfg(feature = "tls")]
fn create_tls_acceptor() -> Result<tokio_rustls::TlsAcceptor, Box<dyn std::error::Error>> {
    use rustls::ServerConfig;
    use std::sync::Arc as StdArc;

    let cert = rcgen::generate_simple_self_signed(vec![
        "localhost".to_string(),
        "ivnc.local".to_string(),
    ])?;
    let cert_der = rustls::pki_types::CertificateDer::from(cert.cert);
    let key_der = rustls::pki_types::PrivateKeyDer::try_from(cert.key_pair.serialize_der())
        .map_err(|e| format!("TLS key error: {}", e))?;

    let config =
        ServerConfig::builder_with_provider(StdArc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der)?;

    info!("TLS enabled with self-signed certificate");
    Ok(tokio_rustls::TlsAcceptor::from(StdArc::new(config)))
}

/// Health check handler
async fn health_handler(State(state): State<Arc<SharedState>>) -> String {
    let uptime = state.uptime();
    let clients = state.connection_count();

    format!(
        r#"{{
  "status": "healthy",
  "uptime_seconds": {:.2},
  "connections": {},
  "version": "{}"
}}"#,
        uptime.as_secs_f64(),
        clients,
        env!("CARGO_PKG_VERSION")
    )
}

/// Metrics handler (Prometheus format)
async fn metrics_handler(State(state): State<Arc<SharedState>>) -> String {
    let uptime = state.uptime().as_secs_f64();
    let clients = state.connection_count();
    let stats = state.stats.lock().unwrap().clone();

    format!(
        r#"# HELP ivnc_uptime_seconds Server uptime in seconds
# TYPE ivnc_uptime_seconds counter
ivnc_uptime_seconds {}
# HELP ivnc_connections Current number of connections
# TYPE ivnc_connections gauge
ivnc_connections {}
# HELP ivnc_cpu_percent Process CPU usage percent
# TYPE ivnc_cpu_percent gauge
ivnc_cpu_percent {}
# HELP ivnc_mem_bytes Process RSS in bytes
# TYPE ivnc_mem_bytes gauge
ivnc_mem_bytes {}
# HELP ivnc_client_latency_ms Client-reported latency in ms
# TYPE ivnc_client_latency_ms gauge
ivnc_client_latency_ms {}
# HELP ivnc_client_fps Client-reported FPS
# TYPE ivnc_client_fps gauge
ivnc_client_fps {}
# HELP ivnc_proto_connections_total Protocol classification counters
# TYPE ivnc_proto_connections_total counter
ivnc_proto_connections_total{{protocol="http"}} {}
ivnc_proto_connections_total{{protocol="ice_tcp"}} {}
ivnc_proto_connections_total{{protocol="tls"}} {}
ivnc_proto_connections_total{{protocol="unknown"}} {}
"#,
        uptime,
        clients,
        stats.cpu_percent,
        stats.mem_used,
        stats.client_latency_ms,
        stats.client_fps,
        stats.proto_http,
        stats.proto_ice_tcp,
        stats.proto_tls,
        stats.proto_unknown
    )
}

async fn render_status_handler(
    State(state): State<Arc<SharedState>>,
) -> axum::Json<serde_json::Value> {
    let stats = state.stats.lock().unwrap().clone();
    let client_viewers = state.client_viewer_summary();
    axum::Json(serde_json::json!({
        "render_state": state.render_state().as_str(),
        "ivnc_cpu_percent": stats.cpu_percent,
        "mem_used": stats.mem_used,
        "viewer_count": state.webrtc_sessions(),
        "rtp_receiver_count": state.rtp_receiver_count(),
        "connections": state.connection_count(),
        "has_active_viewer": client_viewers.has_active_viewer,
        "all_clients_inactive": client_viewers.all_clients_inactive,
        "has_recent_client_wakeup": client_viewers.has_recent_client_wakeup,
        "uptime_seconds": state.uptime().as_secs_f64()
    }))
}

async fn basic_auth_middleware(
    State(state): State<Arc<SharedState>>,
    req: Request<Body>,
    next: middleware::Next,
) -> Response {
    if !state.config.http.basic_auth_enabled {
        return next.run(req).await;
    }

    let path = req.uri().path();
    if path == "/health"
        || path == "/version"
        || path == "/manifest.json"
        || path == "/sw.js"
        || path.starts_with("/icons/")
    {
        return next.run(req).await;
    }

    // Read password override; clone to release the RwLock guard immediately
    let expected_password = {
        let guard = state.password_override.read().await;
        match guard.as_deref() {
            Some(overridden) => overridden.to_string(),
            None => state.config.http.basic_auth_password.clone(),
        }
    };

    match req.headers().get(header::AUTHORIZATION) {
        Some(value) => {
            if let Ok(value_str) = value.to_str() {
                if let Some(encoded) = value_str.strip_prefix("Basic ") {
                    if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded) {
                        if let Ok(decoded_str) = String::from_utf8(decoded) {
                            if let Some((user, pass)) = decoded_str.split_once(':') {
                                if user == state.config.http.basic_auth_user
                                    && pass == expected_password
                                {
                                    return next.run(req).await;
                                }
                            }
                        }
                    }
                }
            }
        }
        None => {}
    }

    let mut response = Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .body(Body::from("Unauthorized"))
        .unwrap_or_else(|_| Response::new(Body::empty()));
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        header::HeaderValue::from_static("Basic realm=\"ivnc\""),
    );
    response
}

/// Clients handler - returns WebRTC session count
async fn clients_handler(State(state): State<Arc<SharedState>>) -> String {
    let webrtc_sessions = state.webrtc_sessions();

    format!(
        r#"{{
  "webrtc_sessions": {}
}}"#,
        webrtc_sessions
    )
}

/// UI configuration handler
async fn ui_config_handler(State(state): State<Arc<SharedState>>) -> String {
    state.ui_config_json()
}

/// WebSocket configuration handler
async fn ws_config_handler(State(state): State<Arc<SharedState>>) -> Response {
    let payload = json!({
        "ws_port": state.config.http.port,
        "tcp_only": state.config.webrtc.tcp_only
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap()
}

/// Connections handler - returns all active connections
async fn connections_handler(State(state): State<Arc<SharedState>>) -> Response {
    let json = state.get_connections_json();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json))
        .unwrap()
}

/// Disconnect handler - disconnects a specific connection
async fn disconnect_handler(
    State(state): State<Arc<SharedState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    // Send force disconnect notification to client before closing
    state.send_text(format!("force_disconnect,{}", id));

    // Wait briefly for message to be sent
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Get the connection and send shutdown signal
    let shutdown_tx = {
        let mut conns = state.connections.lock().unwrap();
        conns.remove(&id).and_then(|conn| conn.shutdown_tx)
    };

    if let Some(tx) = shutdown_tx {
        let _ = tx.send(());
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"success":true}"#))
        .unwrap()
}

/// IPv4 handler - returns current IPv4 address
async fn ipv4_handler(State(state): State<Arc<SharedState>>) -> Response {
    let ip = state.ipv4_address.read().await.clone();
    let json = json!({"ipv4": if ip.is_empty() { None } else { Some(ip) }}).to_string();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json))
        .unwrap()
}

/// Browser terminal WebSocket endpoint.
async fn terminal_ws_handler(
    State(state): State<Arc<SharedState>>,
    ws: WebSocketUpgrade,
) -> Response {
    if !state.config.terminal.enabled {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"error":"terminal disabled"}"#))
            .unwrap();
    }

    ws.on_upgrade(move |socket| async move {
        crate::terminal::ws::handle_terminal_websocket(socket, state).await;
    })
    .into_response()
}

/// Fetch IPv4 address from ipv4.im
async fn fetch_ipv4() -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("curl/7.68.0")
        .build()?;
    let ip = client
        .get("https://ipv4.im")
        .send()
        .await?
        .text()
        .await?
        .trim()
        .to_string();
    Ok(ip)
}

async fn index_handler(State(_state): State<Arc<SharedState>>) -> Response {
    // Check for embedded assets first, then fall back to filesystem
    let use_embedded = has_embedded_assets() && std::env::var("IVNC_WEB_ROOT").is_err();

    if use_embedded {
        return get_embedded_file("index.html");
    }

    // Fallback to filesystem
    let static_root = std::env::var("IVNC_WEB_ROOT").unwrap_or_else(|_| "web/ivnc".to_string());
    let index_path = PathBuf::from(&static_root).join("index.html");
    match tokio::fs::read(&index_path).await {
        Ok(data) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-store, max-age=0")
            .body(Body::from(data))
            .unwrap(),
        Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("index.html not found"))
            .unwrap(),
    }
}

/// Handler for serving embedded static files
async fn embedded_fallback_handler(uri: Uri) -> Response {
    get_embedded_file(uri.path())
}

/// Change password handler
async fn change_password_handler(
    State(state): State<Arc<SharedState>>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Response {
    let new_password = match body.get("new_password").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => {
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"error":"missing new_password field"}"#))
                .unwrap();
        }
    };

    if new_password.len() < 4 {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                r#"{"error":"password must be at least 4 characters"}"#,
            ))
            .unwrap();
    }

    let mut pw = state.password_override.write().await;
    *pw = Some(new_password.to_string());
    info!("Password changed via /api/change-password");

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"ok":true}"#))
        .unwrap()
}

/// Console page handler - serves the Apps management UI
async fn console_handler(State(_state): State<Arc<SharedState>>) -> Response {
    // Check for embedded assets first, then fall back to filesystem
    let use_embedded = has_embedded_assets() && std::env::var("IVNC_WEB_ROOT").is_err();

    if use_embedded {
        return get_embedded_file("console.html");
    }

    // Fallback to filesystem
    let static_root = std::env::var("IVNC_WEB_ROOT").unwrap_or_else(|_| "web/ivnc".to_string());
    let path = PathBuf::from(&static_root).join("console.html");
    match tokio::fs::read(&path).await {
        Ok(data) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-store, max-age=0")
            .body(Body::from(data))
            .unwrap(),
        Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("console.html not found"))
            .unwrap(),
    }
}

#[cfg(feature = "agent")]
async fn console_overview_handler(State(state): State<Arc<SharedState>>) -> Response {
    let (screen_width, screen_height) = state.display_size();
    let stats = state.stats.lock().unwrap().clone();
    let running_run_id = state.agent_runs.running_run_id();
    json_response(
        StatusCode::OK,
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "build": {
                "commit": option_env!("IVNC_BUILD_GIT_COMMIT").unwrap_or("unknown"),
                "message": option_env!("IVNC_BUILD_GIT_MESSAGE").unwrap_or("unknown"),
                "features": {
                    "mcp": cfg!(feature = "mcp"),
                    "agent": cfg!(feature = "agent"),
                    "agent_all": cfg!(feature = "agent-all"),
                }
            },
            "endpoints": {
                "mcp": "/mcp",
                "console": "/console",
                "desktop": "/",
            },
            "display": {
                "width": screen_width,
                "height": screen_height,
                "configured_width": state.config.display.width,
                "configured_height": state.config.display.height,
                "refresh_rate": state.config.display.refresh_rate,
            },
            "runtime": {
                "fps": stats.fps,
                "bandwidth_bps": stats.bandwidth,
                "cpu_percent": stats.cpu_percent,
                "mem_bytes": stats.mem_used,
                "target_fps": state.runtime_settings.target_fps(),
                "video_bitrate_kbps": state.runtime_settings.video_bitrate_kbps(),
                "audio_bitrate": state.runtime_settings.audio_bitrate(),
                "keyframe_interval": state.runtime_settings.keyframe_interval(),
            },
            "connections": {
                "webrtc_sessions": state.webrtc_sessions(),
            },
            "agent": {
                "exclusive": state.agent_exclusive(),
                "stop_requested": state.agent_stop_requested(),
                "running_run_id": running_run_id,
            },
            "providers": crate::agent::registry::provider_infos(),
            "config_path": crate::console_config::config_path(),
        }),
    )
}

#[cfg(not(feature = "agent"))]
async fn console_overview_handler(State(state): State<Arc<SharedState>>) -> Response {
    let (screen_width, screen_height) = state.display_size();
    let stats = state.stats.lock().unwrap().clone();
    json_response(
        StatusCode::OK,
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "build": {
                "commit": option_env!("IVNC_BUILD_GIT_COMMIT").unwrap_or("unknown"),
                "message": option_env!("IVNC_BUILD_GIT_MESSAGE").unwrap_or("unknown"),
                "features": {
                    "mcp": cfg!(feature = "mcp"),
                    "agent": false,
                    "agent_all": false,
                }
            },
            "endpoints": {
                "mcp": if cfg!(feature = "mcp") { "/mcp" } else { "" },
                "console": "/console",
                "desktop": "/",
            },
            "display": {
                "width": screen_width,
                "height": screen_height,
                "configured_width": state.config.display.width,
                "configured_height": state.config.display.height,
                "refresh_rate": state.config.display.refresh_rate,
            },
            "runtime": {
                "fps": stats.fps,
                "bandwidth_bps": stats.bandwidth,
                "cpu_percent": stats.cpu_percent,
                "mem_bytes": stats.mem_used,
                "target_fps": state.runtime_settings.target_fps(),
                "video_bitrate_kbps": state.runtime_settings.video_bitrate_kbps(),
                "audio_bitrate": state.runtime_settings.audio_bitrate(),
                "keyframe_interval": state.runtime_settings.keyframe_interval(),
            },
            "connections": {
                "webrtc_sessions": state.webrtc_sessions(),
            },
            "agent": {
                "enabled": false,
                "exclusive": false,
                "stop_requested": false,
                "running_run_id": null,
            },
            "providers": [],
            "config_path": null,
        }),
    )
}

#[cfg(not(feature = "agent"))]
#[derive(Debug, Deserialize)]
struct RuntimeConsoleUpdate {
    video_bitrate_kbps: Option<u32>,
    audio_bitrate: Option<u32>,
    keyframe_interval: Option<u32>,
    target_fps: Option<u32>,
    binary_clipboard_enabled: Option<bool>,
}

#[cfg(feature = "agent")]
async fn console_settings_get_handler(State(state): State<Arc<SharedState>>) -> Response {
    let saved = crate::console_config::load().runtime;
    json_response(
        StatusCode::OK,
        json!({
            "saved": saved,
            "current": {
                "target_fps": state.runtime_settings.target_fps(),
                "video_bitrate_kbps": state.runtime_settings.video_bitrate_kbps(),
                "audio_bitrate": state.runtime_settings.audio_bitrate(),
                "keyframe_interval": state.runtime_settings.keyframe_interval(),
                "binary_clipboard_enabled": state.runtime_settings.binary_clipboard_enabled(),
            },
            "restart_required_fields": [
                "http.host",
                "http.port",
                "display.width",
                "display.height",
                "tls",
                "web_root",
                "features"
            ]
        }),
    )
}

#[cfg(not(feature = "agent"))]
async fn console_settings_get_handler(State(state): State<Arc<SharedState>>) -> Response {
    let current = json!({
        "target_fps": state.runtime_settings.target_fps(),
        "video_bitrate_kbps": state.runtime_settings.video_bitrate_kbps(),
        "audio_bitrate": state.runtime_settings.audio_bitrate(),
        "keyframe_interval": state.runtime_settings.keyframe_interval(),
        "binary_clipboard_enabled": state.runtime_settings.binary_clipboard_enabled(),
    });
    json_response(
        StatusCode::OK,
        json!({
            "saved": current,
            "current": current,
            "restart_required_fields": [],
        }),
    )
}

#[cfg(feature = "agent")]
async fn console_settings_put_handler(
    State(state): State<Arc<SharedState>>,
    axum::extract::Json(body): axum::extract::Json<crate::console_config::RuntimeConsoleConfig>,
) -> Response {
    if let Some(fps) = body.target_fps {
        state.runtime_settings.set_target_fps(fps);
    }
    if let Some(bitrate) = body.video_bitrate_kbps {
        state.runtime_settings.set_video_bitrate_kbps(bitrate);
    }
    if let Some(bitrate) = body.audio_bitrate {
        state.runtime_settings.set_audio_bitrate(bitrate);
    }
    if let Some(interval) = body.keyframe_interval {
        state.runtime_settings.set_keyframe_interval(interval);
    }
    if let Some(enabled) = body.binary_clipboard_enabled {
        state
            .runtime_settings
            .apply_settings_json(&json!({"enable_binary_clipboard": enabled}).to_string());
    }

    let mut config = crate::console_config::load();
    config.runtime = body;
    match crate::console_config::save(&config) {
        Ok(()) => json_response(StatusCode::OK, json!({"ok": true, "applied": "runtime"})),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": err})),
    }
}

#[cfg(not(feature = "agent"))]
async fn console_settings_put_handler(
    State(state): State<Arc<SharedState>>,
    axum::extract::Json(body): axum::extract::Json<RuntimeConsoleUpdate>,
) -> Response {
    if let Some(fps) = body.target_fps {
        state.runtime_settings.set_target_fps(fps);
    }
    if let Some(bitrate) = body.video_bitrate_kbps {
        state.runtime_settings.set_video_bitrate_kbps(bitrate);
    }
    if let Some(bitrate) = body.audio_bitrate {
        state.runtime_settings.set_audio_bitrate(bitrate);
    }
    if let Some(interval) = body.keyframe_interval {
        state.runtime_settings.set_keyframe_interval(interval);
    }
    if let Some(enabled) = body.binary_clipboard_enabled {
        state
            .runtime_settings
            .apply_settings_json(&json!({"enable_binary_clipboard": enabled}).to_string());
    }

    json_response(StatusCode::OK, json!({"ok": true, "applied": "runtime"}))
}

#[cfg(feature = "agent")]
async fn console_keyframe_handler(State(state): State<Arc<SharedState>>) -> Response {
    state.runtime_settings.request_keyframe();
    json_response(StatusCode::OK, json!({"ok": true}))
}

#[cfg(not(feature = "agent"))]
async fn console_keyframe_handler(State(state): State<Arc<SharedState>>) -> Response {
    state.runtime_settings.request_keyframe();
    json_response(StatusCode::OK, json!({"ok": true}))
}

#[cfg(feature = "agent")]
async fn console_agent_get_handler() -> Response {
    json_response(
        StatusCode::OK,
        json!(crate::console_config::agent_defaults()),
    )
}

#[cfg(not(feature = "agent"))]
async fn console_agent_get_handler() -> Response {
    json_response(
        StatusCode::OK,
        json!({
            "default_provider": "",
            "options": {
                "budget": {
                    "max_steps": 50,
                    "max_input_tokens": 200000,
                    "max_output_tokens": 20000,
                    "max_wall_seconds": 300,
                    "max_screenshots": 60,
                },
                "screenshot_max_bytes": 800000,
                "record_trajectory": true,
                "dry_run": false,
            },
            "disabled": true,
        }),
    )
}

#[cfg(feature = "agent")]
async fn console_agent_put_handler(
    axum::extract::Json(body): axum::extract::Json<crate::console_config::AgentConsoleConfig>,
) -> Response {
    if body.default_provider.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "default_provider is required"}),
        );
    }
    if body.options.budget.max_steps == 0 || body.options.max_actions_per_step == 0 {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "max_steps and max_actions_per_step must be greater than 0"}),
        );
    }
    let mut config = crate::console_config::load();
    config.agent = body;
    match crate::console_config::save(&config) {
        Ok(()) => json_response(
            StatusCode::OK,
            json!({"ok": true, "applies": "next_agent_run"}),
        ),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": err})),
    }
}

#[cfg(not(feature = "agent"))]
async fn console_agent_put_handler() -> Response {
    json_response(
        StatusCode::NOT_IMPLEMENTED,
        json!({"error": "agent feature is not enabled"}),
    )
}

#[cfg(feature = "agent")]
async fn console_agent_stop_handler(State(state): State<Arc<SharedState>>) -> Response {
    state.request_agent_stop();
    state.agent_runs.mark_interrupted(None);
    json_response(StatusCode::OK, json!({"ok": true}))
}

#[cfg(not(feature = "agent"))]
async fn console_agent_stop_handler() -> Response {
    json_response(
        StatusCode::OK,
        json!({"ok": false, "disabled": true, "reason": "agent feature is not enabled"}),
    )
}

#[cfg(feature = "agent")]
#[derive(serde::Deserialize)]
struct AgentStartHttpRequest {
    task: String,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    budget: Option<crate::agent::types::Budget>,
    #[serde(default)]
    options: Option<crate::agent::types::RunOptions>,
}

#[cfg(feature = "agent")]
async fn console_agent_start_handler(
    State(state): State<Arc<SharedState>>,
    axum::extract::Json(body): axum::extract::Json<AgentStartHttpRequest>,
) -> Response {
    if body.task.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "task is required"}),
        );
    }
    let provider = body
        .provider
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| crate::console_config::agent_defaults().default_provider);
    if provider.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "provider is required"}),
        );
    }
    let req = crate::agent::launch::LaunchRequest {
        task: body.task,
        provider,
        model: body.model,
        budget: body.budget,
        options: body.options,
        replay_actions: None,
        source: None,
    };
    match crate::agent::launch::launch_agent_run(state.clone(), req, false).await {
        Ok(report) => json_response(
            StatusCode::OK,
            json!({
                "ok": true,
                "run_id": report.run_id,
                "started_at_ms": report.started_at_ms,
            }),
        ),
        Err(crate::agent::launch::LaunchError::AlreadyActive(id)) => json_response(
            StatusCode::CONFLICT,
            json!({"error": "agent_run_already_active", "run_id": id}),
        ),
        Err(crate::agent::launch::LaunchError::ProviderUnavailable(name)) => json_response(
            StatusCode::BAD_REQUEST,
            json!({
                "error": format!("provider '{}' is not available in this build", name),
            }),
        ),
        Err(crate::agent::launch::LaunchError::InvalidRequest(msg)) => {
            json_response(StatusCode::BAD_REQUEST, json!({"error": msg}))
        }
        Err(crate::agent::launch::LaunchError::Internal(msg)) => {
            json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": msg}))
        }
    }
}

#[cfg(not(feature = "agent"))]
async fn console_agent_start_handler() -> Response {
    json_response(
        StatusCode::NOT_IMPLEMENTED,
        json!({"error": "agent feature is not enabled"}),
    )
}

#[cfg(feature = "agent")]
#[derive(serde::Deserialize)]
struct ScheduleInput {
    name: String,
    cron: String,
    task: String,
    provider: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    budget: Option<crate::agent::types::Budget>,
    #[serde(default)]
    options: Option<crate::agent::types::RunOptions>,
    #[serde(default = "default_true_bool")]
    enabled: bool,
}

#[cfg(feature = "agent")]
fn default_true_bool() -> bool {
    true
}

#[cfg(feature = "agent")]
fn validate_schedule_input(body: &ScheduleInput) -> Result<(), String> {
    if body.name.trim().is_empty() {
        return Err("name is required".into());
    }
    if body.task.trim().is_empty() {
        return Err("task is required".into());
    }
    if body.provider.trim().is_empty() {
        return Err("provider is required".into());
    }
    use std::str::FromStr;
    let normalized = crate::agent::schedule::normalize_cron(&body.cron);
    if let Err(e) = cron::Schedule::from_str(&normalized) {
        return Err(format!("invalid cron expression: {}", e));
    }
    if let Some(b) = &body.budget {
        if b.max_steps == 0 {
            return Err("budget.max_steps must be > 0".into());
        }
        if b.max_wall_seconds == 0 {
            return Err("budget.max_wall_seconds must be > 0".into());
        }
    }
    Ok(())
}

#[cfg(feature = "agent")]
fn next_fire_preview(cron_expr: &str) -> Option<u64> {
    use std::str::FromStr;
    let sched = cron::Schedule::from_str(&crate::agent::schedule::normalize_cron(cron_expr)).ok()?;
    sched
        .upcoming(chrono::Local)
        .next()
        .map(|dt| dt.timestamp_millis() as u64)
}

#[cfg(feature = "agent")]
fn merge_schedule_json(
    st: &crate::console_config::ScheduledTask,
    state: &Arc<SharedState>,
) -> serde_json::Value {
    let rt = state.schedule_state.lock().unwrap().get(&st.id).cloned();
    let computed_next = next_fire_preview(&st.cron);
    let runtime_next = rt.as_ref().map(|r| r.next_fire_ms);
    json!({
        "id": st.id,
        "name": st.name,
        "cron": st.cron,
        "task": st.task,
        "provider": st.provider,
        "model": st.model,
        "budget": st.budget,
        "options": st.options,
        "enabled": st.enabled,
        "next_fire_ms": runtime_next.or(computed_next),
        "last_run_ms": rt.as_ref().and_then(|r| r.last_run_ms),
        "last_run_id": rt.as_ref().and_then(|r| r.last_run_id.clone()),
        "last_outcome": rt.as_ref().and_then(|r| r.last_outcome.clone()),
        "last_skip_reason": rt.as_ref().and_then(|r| r.last_skip_reason.clone()),
    })
}

#[cfg(feature = "agent")]
async fn console_schedules_get_handler(State(state): State<Arc<SharedState>>) -> Response {
    let cfg = crate::console_config::load();
    let items: Vec<_> = cfg
        .schedules
        .iter()
        .map(|st| merge_schedule_json(st, &state))
        .collect();
    json_response(StatusCode::OK, json!({ "schedules": items }))
}

#[cfg(not(feature = "agent"))]
async fn console_schedules_get_handler() -> Response {
    json_response(
        StatusCode::NOT_IMPLEMENTED,
        json!({"error": "agent feature is not enabled"}),
    )
}

#[cfg(feature = "agent")]
async fn console_schedules_post_handler(
    State(state): State<Arc<SharedState>>,
    axum::extract::Json(body): axum::extract::Json<ScheduleInput>,
) -> Response {
    if let Err(e) = validate_schedule_input(&body) {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": e}));
    }
    let id = format!("sch_{}", uuid::Uuid::new_v4());
    let st = crate::console_config::ScheduledTask {
        id: id.clone(),
        name: body.name,
        cron: body.cron,
        task: body.task,
        provider: body.provider,
        model: body.model,
        budget: body.budget.unwrap_or_default(),
        options: body.options,
        enabled: body.enabled,
    };
    let mut cfg = crate::console_config::load();
    cfg.schedules.push(st.clone());
    if let Err(e) = crate::console_config::save(&cfg) {
        return json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e}));
    }
    crate::agent::schedule::invalidate(&state, &id);
    json_response(StatusCode::OK, json!({"ok": true, "schedule": merge_schedule_json(&st, &state)}))
}

#[cfg(not(feature = "agent"))]
async fn console_schedules_post_handler() -> Response {
    json_response(
        StatusCode::NOT_IMPLEMENTED,
        json!({"error": "agent feature is not enabled"}),
    )
}

#[cfg(feature = "agent")]
async fn console_schedule_put_handler(
    State(state): State<Arc<SharedState>>,
    Path(id): Path<String>,
    axum::extract::Json(body): axum::extract::Json<ScheduleInput>,
) -> Response {
    if let Err(e) = validate_schedule_input(&body) {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": e}));
    }
    let mut cfg = crate::console_config::load();
    let Some(idx) = cfg.schedules.iter().position(|s| s.id == id) else {
        return json_response(StatusCode::NOT_FOUND, json!({"error": "schedule not found"}));
    };
    let updated = crate::console_config::ScheduledTask {
        id: id.clone(),
        name: body.name,
        cron: body.cron,
        task: body.task,
        provider: body.provider,
        model: body.model,
        budget: body.budget.unwrap_or_default(),
        options: body.options,
        enabled: body.enabled,
    };
    cfg.schedules[idx] = updated.clone();
    if let Err(e) = crate::console_config::save(&cfg) {
        return json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e}));
    }
    crate::agent::schedule::invalidate(&state, &id);
    json_response(StatusCode::OK, json!({"ok": true, "schedule": merge_schedule_json(&updated, &state)}))
}

#[cfg(not(feature = "agent"))]
async fn console_schedule_put_handler() -> Response {
    json_response(
        StatusCode::NOT_IMPLEMENTED,
        json!({"error": "agent feature is not enabled"}),
    )
}

#[cfg(feature = "agent")]
async fn console_schedule_delete_handler(
    State(state): State<Arc<SharedState>>,
    Path(id): Path<String>,
) -> Response {
    let mut cfg = crate::console_config::load();
    let len_before = cfg.schedules.len();
    cfg.schedules.retain(|s| s.id != id);
    if cfg.schedules.len() == len_before {
        return json_response(StatusCode::NOT_FOUND, json!({"error": "schedule not found"}));
    }
    if let Err(e) = crate::console_config::save(&cfg) {
        return json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e}));
    }
    crate::agent::schedule::invalidate(&state, &id);
    json_response(StatusCode::OK, json!({"ok": true}))
}

#[cfg(not(feature = "agent"))]
async fn console_schedule_delete_handler() -> Response {
    json_response(
        StatusCode::NOT_IMPLEMENTED,
        json!({"error": "agent feature is not enabled"}),
    )
}

#[cfg(feature = "agent")]
async fn console_schedule_run_now_handler(
    State(state): State<Arc<SharedState>>,
    Path(id): Path<String>,
) -> Response {
    let cfg = crate::console_config::load();
    let Some(st) = cfg.schedules.iter().find(|s| s.id == id).cloned() else {
        return json_response(StatusCode::NOT_FOUND, json!({"error": "schedule not found"}));
    };
    let req = crate::agent::launch::LaunchRequest {
        task: st.task,
        provider: st.provider,
        model: st.model,
        budget: Some(st.budget),
        options: st.options,
        replay_actions: None,
        source: Some(crate::agent::types::RunSource::Schedule {
            id: st.id.clone(),
            name: Some(st.name.clone()),
        }),
    };
    match crate::agent::launch::launch_agent_run(state.clone(), req, false).await {
        Ok(report) => {
            let mut map = state.schedule_state.lock().unwrap();
            let rt = map.entry(st.id.clone()).or_default();
            rt.last_run_ms = Some(crate::agent::types::now_ms());
            rt.last_run_id = Some(report.run_id.clone());
            rt.last_outcome = Some("started".into());
            rt.last_skip_reason = None;
            drop(map);
            json_response(StatusCode::OK, json!({"ok": true, "run_id": report.run_id}))
        }
        Err(crate::agent::launch::LaunchError::AlreadyActive(active_id)) => json_response(
            StatusCode::CONFLICT,
            json!({"error": "agent_run_already_active", "run_id": active_id}),
        ),
        Err(crate::agent::launch::LaunchError::ProviderUnavailable(name)) => json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": format!("provider '{}' is not available in this build", name)}),
        ),
        Err(crate::agent::launch::LaunchError::InvalidRequest(m)) => {
            json_response(StatusCode::BAD_REQUEST, json!({"error": m}))
        }
        Err(crate::agent::launch::LaunchError::Internal(m)) => {
            json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": m}))
        }
    }
}

#[cfg(not(feature = "agent"))]
async fn console_schedule_run_now_handler() -> Response {
    json_response(
        StatusCode::NOT_IMPLEMENTED,
        json!({"error": "agent feature is not enabled"}),
    )
}


#[cfg(feature = "agent")]
async fn console_providers_get_handler() -> Response {
    let saved = crate::console_config::load();
    let providers: Vec<_> = crate::agent::registry::provider_presets()
        .into_iter()
        .map(|preset| {
            let cfg = saved.providers.get(preset.name).cloned().unwrap_or_default();
            json!({
                "name": preset.name,
                "default_model": preset.default_model,
                "default_endpoint": preset.default_endpoint,
                "api_key_env": preset.api_key_env,
                "configured": crate::agent::registry::provider_infos().iter().find(|p| p.name == preset.name).map(|p| p.configured).unwrap_or(false),
                "capabilities": preset.capabilities,
                "settings": {
                    "endpoint": cfg.endpoint,
                    "model": cfg.model,
                    "api_format": cfg.api_format,
                    "api_key_configured": cfg.api_key.as_ref().is_some_and(|v| !v.is_empty())
                        || preset.api_key_env.is_some_and(|env| std::env::var_os(env).is_some()),
                    "coord_space": cfg.coord_space,
                    "system_prompt": cfg.system_prompt,
                }
            })
        })
        .collect();
    json_response(StatusCode::OK, json!({ "providers": providers }))
}

#[cfg(not(feature = "agent"))]
async fn console_providers_get_handler() -> Response {
    json_response(StatusCode::OK, json!({ "providers": [] }))
}

#[cfg(feature = "agent")]
#[derive(Debug, Deserialize)]
struct ProviderConsoleUpdate {
    endpoint: Option<String>,
    model: Option<String>,
    api_format: Option<String>,
    api_key: Option<String>,
    clear_api_key: Option<bool>,
    coord_space: Option<String>,
    system_prompt: Option<String>,
}

#[cfg(feature = "agent")]
async fn console_provider_put_handler(
    Path(name): Path<String>,
    axum::extract::Json(body): axum::extract::Json<ProviderConsoleUpdate>,
) -> Response {
    let known = crate::agent::registry::provider_presets()
        .into_iter()
        .any(|preset| preset.name == name);
    if !known {
        return json_response(StatusCode::NOT_FOUND, json!({"error": "unknown provider"}));
    }

    let mut config = crate::console_config::load();
    let entry = config.providers.entry(name).or_default();
    entry.endpoint = clean_optional(body.endpoint);
    entry.model = clean_optional(body.model);
    entry.api_format = clean_optional(body.api_format);
    entry.coord_space = clean_optional(body.coord_space);
    if body.system_prompt.is_some() {
        entry.system_prompt = clean_optional(body.system_prompt);
    }
    if body.clear_api_key.unwrap_or(false) {
        entry.api_key = None;
    } else if let Some(api_key) = clean_optional(body.api_key) {
        if api_key != "********" {
            entry.api_key = Some(api_key);
        }
    }

    match crate::console_config::save(&config) {
        Ok(()) => json_response(
            StatusCode::OK,
            json!({"ok": true, "applies": "next_agent_run"}),
        ),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": err})),
    }
}

#[cfg(not(feature = "agent"))]
async fn console_provider_put_handler() -> Response {
    json_response(
        StatusCode::NOT_IMPLEMENTED,
        json!({"error": "agent feature is not enabled"}),
    )
}

#[cfg(feature = "agent")]
async fn console_agent_runs_handler(State(state): State<Arc<SharedState>>) -> Response {
    json_response(StatusCode::OK, json!({ "runs": state.agent_runs.list() }))
}

#[cfg(not(feature = "agent"))]
async fn console_agent_runs_handler() -> Response {
    json_response(StatusCode::OK, json!({ "runs": [] }))
}

#[cfg(feature = "agent")]
async fn console_agent_run_steps_handler(
    State(state): State<Arc<SharedState>>,
    Path(run_id): Path<String>,
) -> Response {
    let report = state.agent_runs.get(&run_id);
    let path = report
        .as_ref()
        .and_then(|r| r.trajectory_path.clone())
        .unwrap_or_else(|| crate::agent::trajectory::default_trajectory_path(&run_id));
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => {
            let steps: Vec<serde_json::Value> = content
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
                .collect();
            json_response(
                StatusCode::OK,
                json!({ "run_id": run_id, "report": report, "steps": steps }),
            )
        }
        Err(_) => json_response(
            StatusCode::NOT_FOUND,
            json!({ "run_id": run_id, "report": report, "steps": [], "error": "trajectory not found" }),
        ),
    }
}

#[cfg(not(feature = "agent"))]
async fn console_agent_run_steps_handler(Path(_run_id): Path<String>) -> Response {
    json_response(StatusCode::OK, json!({ "steps": [] }))
}

#[cfg(feature = "agent")]
async fn console_agent_run_frame_handler(Path((run_id, name)): Path<(String, String)>) -> Response {
    // Only allow a bare image basename — reject any path traversal.
    let safe = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        && !name.contains("..");
    if !safe || name.is_empty() {
        return json_response(StatusCode::BAD_REQUEST, json!({ "error": "invalid frame name" }));
    }
    let frame_dir = crate::agent::trajectory::default_trajectory_path(&run_id)
        .with_file_name(format!("{}_frames", run_id));
    let path = frame_dir.join(&name);
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let ctype = if name.ends_with(".png") {
                "image/png"
            } else {
                "image/jpeg"
            };
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, ctype)
                .header(header::CACHE_CONTROL, "private, max-age=86400")
                .body(Body::from(bytes))
                .unwrap()
        }
        Err(_) => json_response(StatusCode::NOT_FOUND, json!({ "error": "frame not found" })),
    }
}

#[cfg(not(feature = "agent"))]
async fn console_agent_run_frame_handler(Path((_run_id, _name)): Path<(String, String)>) -> Response {
    json_response(StatusCode::NOT_FOUND, json!({ "error": "agent feature is not enabled" }))
}

fn proxy_json_response(status: StatusCode, body: serde_json::Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn json_response(status: StatusCode, body: serde_json::Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

async fn proxy_status_handler(State(state): State<Arc<SharedState>>) -> Response {
    proxy_json_response(StatusCode::OK, json!(state.proxy_panel.status()))
}

async fn proxy_restart_handler(State(state): State<Arc<SharedState>>) -> Response {
    match state.proxy_panel.restart().await {
        Ok(pid) => proxy_json_response(StatusCode::OK, json!({"ok": true, "pid": pid})),
        Err(err) => proxy_json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"ok": false, "error": err}),
        ),
    }
}

async fn proxy_panel_root_handler() -> Response {
    Response::builder()
        .status(StatusCode::TEMPORARY_REDIRECT)
        .header(header::LOCATION, "/proxy/")
        .body(Body::empty())
        .unwrap()
}

async fn proxy_panel_handler(
    State(state): State<Arc<SharedState>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    proxy_to_miao(state, method, uri, headers, body, true).await
}

async fn proxy_panel_ws_handler(
    State(state): State<Arc<SharedState>>,
    ws: WebSocketUpgrade,
    uri: Uri,
) -> Response {
    if let Err(err) = state.proxy_panel.ensure_running().await {
        return proxy_json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"ok": false, "error": err}),
        );
    }

    let path_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    let upstream_path = path_query.strip_prefix("/proxy-ws").unwrap_or(path_query);
    let upstream_path = if upstream_path.is_empty() {
        "/"
    } else {
        upstream_path
    };
    let upstream = format!(
        "ws://127.0.0.1:{}{}",
        crate::proxy_panel::MIAO_PORT,
        upstream_path
    );

    ws.on_upgrade(move |socket| async move {
        proxy_miao_websocket(socket, upstream).await;
    })
    .into_response()
}

async fn proxy_panel_absolute_path_middleware(
    State(state): State<Arc<SharedState>>,
    req: Request<Body>,
    next: middleware::Next,
) -> Response {
    let path = req.uri().path();
    let referer_from_proxy = req
        .headers()
        .get(header::REFERER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<Uri>().ok())
        .map(|uri| uri.path().starts_with("/proxy"))
        .unwrap_or(false);
    let should_proxy = path.starts_with("/_next/")
        || path.starts_with("/miao-inject/")
        || (path.starts_with("/api/") && referer_from_proxy);

    if !should_proxy {
        return next.run(req).await;
    }

    let (parts, body) = req.into_parts();
    proxy_to_miao(state, parts.method, parts.uri, parts.headers, body, false).await
}

async fn proxy_to_miao(
    state: Arc<SharedState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
    strip_proxy_prefix: bool,
) -> Response {
    if let Err(err) = state.proxy_panel.ensure_running().await {
        return proxy_json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"ok": false, "error": err}),
        );
    }

    let path_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    let upstream_path = if strip_proxy_prefix {
        path_query.strip_prefix("/proxy").unwrap_or(path_query)
    } else {
        path_query
    };
    let upstream_path = if upstream_path.is_empty() {
        "/"
    } else {
        upstream_path
    };
    let upstream = format!(
        "http://127.0.0.1:{}{}",
        crate::proxy_panel::MIAO_PORT,
        upstream_path
    );

    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(60))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            return proxy_json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"ok": false, "error": format!("proxy client error: {}", err)}),
            )
        }
    };

    let body_bytes = match to_bytes(body, 32 * 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(err) => {
            return proxy_json_response(
                StatusCode::BAD_REQUEST,
                json!({"ok": false, "error": format!("request body error: {}", err)}),
            )
        }
    };

    let mut req = client.request(method, upstream).body(body_bytes);
    for (name, value) in headers.iter() {
        if name == header::HOST
            || name == header::CONTENT_LENGTH
            || name == header::CONNECTION
            || name == header::ACCEPT_ENCODING
        {
            continue;
        }
        req = req.header(name, value);
    }

    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let headers = resp.headers().clone();
            let mut bytes = match resp.bytes().await {
                Ok(bytes) => bytes,
                Err(err) => {
                    return proxy_json_response(
                        StatusCode::BAD_GATEWAY,
                        json!({"ok": false, "error": format!("upstream read error: {}", err)}),
                    )
                }
            };
            let is_html = headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.contains("text/html"))
                .unwrap_or(false);
            if strip_proxy_prefix && is_html {
                bytes = inject_proxy_panel_rewrite(bytes);
            }
            let mut builder = Response::builder().status(status);
            for (name, value) in headers.iter() {
                if name == header::CONTENT_LENGTH
                    || name == header::CONNECTION
                    || name == header::TRANSFER_ENCODING
                {
                    continue;
                }
                builder = builder.header(name, value);
            }
            builder.body(Body::from(bytes)).unwrap()
        }
        Err(err) => proxy_json_response(
            StatusCode::BAD_GATEWAY,
            json!({"ok": false, "error": format!("upstream proxy error: {}", err)}),
        ),
    }
}

async fn proxy_miao_websocket(client_socket: AxumWebSocket, upstream: String) {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;

    let upstream_socket = match tokio_tungstenite::connect_async(&upstream).await {
        Ok((socket, _)) => socket,
        Err(err) => {
            warn!("failed to connect miao websocket {}: {}", upstream, err);
            return;
        }
    };

    let (mut client_tx, mut client_rx) = client_socket.split();
    let (mut upstream_tx, mut upstream_rx) = upstream_socket.split();

    loop {
        tokio::select! {
            msg = client_rx.next() => {
                let Some(Ok(msg)) = msg else { break; };
                let upstream_msg = match msg {
                    AxumWsMessage::Text(text) => TungsteniteMessage::Text(text.to_string().into()),
                    AxumWsMessage::Binary(data) => TungsteniteMessage::Binary(data.to_vec().into()),
                    AxumWsMessage::Ping(data) => TungsteniteMessage::Ping(data.to_vec().into()),
                    AxumWsMessage::Pong(data) => TungsteniteMessage::Pong(data.to_vec().into()),
                    AxumWsMessage::Close(frame) => {
                        let _ = upstream_tx.send(TungsteniteMessage::Close(frame.map(|frame| {
                            tokio_tungstenite::tungstenite::protocol::CloseFrame {
                                code: frame.code.into(),
                                reason: frame.reason.to_string().into(),
                            }
                        }))).await;
                        break;
                    }
                };
                if upstream_tx.send(upstream_msg).await.is_err() {
                    break;
                }
            }
            msg = upstream_rx.next() => {
                let Some(Ok(msg)) = msg else { break; };
                let client_msg = match msg {
                    TungsteniteMessage::Text(text) => AxumWsMessage::Text(text.to_string().into()),
                    TungsteniteMessage::Binary(data) => AxumWsMessage::Binary(data.to_vec().into()),
                    TungsteniteMessage::Ping(data) => AxumWsMessage::Ping(data.to_vec().into()),
                    TungsteniteMessage::Pong(data) => AxumWsMessage::Pong(data.to_vec().into()),
                    TungsteniteMessage::Close(frame) => {
                        let _ = client_tx.send(AxumWsMessage::Close(frame.map(|frame| {
                            axum::extract::ws::CloseFrame {
                                code: frame.code.into(),
                                reason: frame.reason.to_string().into(),
                            }
                        }))).await;
                        break;
                    }
                    TungsteniteMessage::Frame(_) => continue,
                };
                if client_tx.send(client_msg).await.is_err() {
                    break;
                }
            }
        }
    }
}

fn inject_proxy_panel_rewrite(bytes: Bytes) -> Bytes {
    const SCRIPT: &str = r#"<script>
(() => {
  if (window.__ivncProxyPanelRewrite) return;
  window.__ivncProxyPanelRewrite = true;
  const mapPath = (value) => {
    if (typeof value !== "string") return value;
    if (value.startsWith("/api/") || value === "/login" || value.startsWith("/dashboard")) {
      return `/proxy${value}`;
    }
    return value;
  };
  const nativeFetch = window.fetch.bind(window);
  window.fetch = (input, init) => {
    if (typeof input === "string") return nativeFetch(mapPath(input), init);
    if (input instanceof Request) {
      const url = new URL(input.url);
      const mapped = mapPath(`${url.pathname}${url.search}`);
      if (mapped !== `${url.pathname}${url.search}`) {
        input = new Request(mapped, input);
      }
    }
    return nativeFetch(input, init);
  };
  const NativeWebSocket = window.WebSocket;
  window.WebSocket = function(url, protocols) {
    try {
      const parsed = new URL(String(url), window.location.href);
      if (parsed.pathname.startsWith("/api/")) {
        const scheme = window.location.protocol === "https:" ? "wss:" : "ws:";
        url = `${scheme}//${window.location.host}/proxy-ws${parsed.pathname}${parsed.search}`;
      }
    } catch (_) {}
    return protocols ? new NativeWebSocket(url, protocols) : new NativeWebSocket(url);
  };
  window.WebSocket.prototype = NativeWebSocket.prototype;
})();
</script>"#;

    let Ok(mut html) = String::from_utf8(bytes.to_vec()) else {
        return bytes;
    };
    if html.contains("__ivncProxyPanelRewrite") {
        return Bytes::from(html);
    }
    if let Some(index) = html.find("<head>") {
        html.insert_str(index + "<head>".len(), SCRIPT);
    } else {
        html.insert_str(0, SCRIPT);
    }
    Bytes::from(html)
}

// ============================================================================
// Force Update Feature
// ============================================================================

/// Version information
#[derive(Serialize, Clone)]
struct VersionInfo {
    current: String,
    latest: String,
    has_update: bool,
    download_url: String,
}

/// Public build version information
#[derive(Serialize, Clone)]
struct PublicVersionInfo {
    version: String,
    git_commit: String,
    git_commit_short: String,
    git_message: String,
}

/// Upgrade log entry for WebSocket streaming
#[derive(Clone, Serialize)]
struct UpgradeLogEntry {
    step: u8,
    total_steps: u8,
    message: String,
    level: String, // "info" | "error" | "success" | "progress"
    #[serde(skip_serializing_if = "Option::is_none")]
    progress: Option<u8>, // 0-100
}

/// GitHub Release response
#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
}

/// WebSocket auth query
#[derive(Deserialize)]
struct WsAuthQuery {
    token: Option<String>,
}

/// GET /version - Public build version information
async fn public_version_handler() -> axum::Json<PublicVersionInfo> {
    let git_commit = option_env!("IVNC_BUILD_GIT_COMMIT")
        .unwrap_or("unknown")
        .to_string();
    let git_commit_short = if git_commit.len() >= 7 {
        git_commit[..7].to_string()
    } else {
        git_commit.clone()
    };
    axum::Json(PublicVersionInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        git_commit,
        git_commit_short,
        git_message: option_env!("IVNC_BUILD_GIT_MESSAGE")
            .unwrap_or("unknown")
            .to_string(),
    })
}

/// GET /api/version - Check for updates
async fn get_version_handler() -> axum::Json<VersionInfo> {
    let current = env!("CARGO_PKG_VERSION").to_string();

    // Detect architecture
    let arch = if cfg!(target_arch = "x86_64") {
        "amd64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        return axum::Json(VersionInfo {
            current: current.clone(),
            latest: current,
            has_update: false,
            download_url: String::new(),
        });
    };

    // Fetch latest version from GitHub
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("ivnc-updater")
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            return axum::Json(VersionInfo {
                current: current.clone(),
                latest: current,
                has_update: false,
                download_url: String::new(),
            });
        }
    };

    let latest_version = match client
        .get("https://api.github.com/repos/Xiechengqi/iVnc/releases/latest")
        .send()
        .await
    {
        Ok(resp) => match resp.json::<GitHubRelease>().await {
            Ok(release) => release.tag_name.trim_start_matches('v').to_string(),
            Err(_) => current.clone(),
        },
        Err(_) => current.clone(),
    };

    let has_update = latest_version != current;
    let download_url = format!(
        "https://github.com/Xiechengqi/iVnc/releases/download/latest/ivnc-linux-{}",
        arch
    );

    axum::Json(VersionInfo {
        current,
        latest: latest_version,
        has_update,
        download_url,
    })
}

/// POST /api/restart - Restart iVNC without upgrading
async fn restart_handler() -> axum::Json<serde_json::Value> {
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Err(err) = restart_current_process().await {
            log::error!("Failed to restart iVNC: {}", err);
        }
    });
    axum::Json(json!({"ok": true}))
}

/// GET /api/upgrade/ws - WebSocket upgrade endpoint
async fn upgrade_ws_handler(
    State(state): State<Arc<SharedState>>,
    Query(_query): Query<WsAuthQuery>,
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, StatusCode> {
    // Verify authentication if basic auth is enabled
    if state.config.http.basic_auth_enabled {
        // Check Authorization header
        if let Some(auth_header) = headers.get(header::AUTHORIZATION) {
            if let Ok(auth_str) = auth_header.to_str() {
                if let Some(encoded) = auth_str.strip_prefix("Basic ") {
                    if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded) {
                        if let Ok(decoded_str) = String::from_utf8(decoded) {
                            if let Some((user, pass)) = decoded_str.split_once(':') {
                                // Read password override
                                let expected_password = {
                                    let guard = state.password_override.read().await;
                                    match guard.as_deref() {
                                        Some(overridden) => overridden.to_string(),
                                        None => state.config.http.basic_auth_password.clone(),
                                    }
                                };

                                if user == state.config.http.basic_auth_user
                                    && pass == expected_password
                                {
                                    return Ok(ws.on_upgrade(handle_upgrade_websocket));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Authentication failed
        return Err(StatusCode::UNAUTHORIZED);
    }

    // No auth required, proceed
    Ok(ws.on_upgrade(handle_upgrade_websocket))
}

/// Handle WebSocket connection for upgrade
async fn handle_upgrade_websocket(mut socket: axum::extract::ws::WebSocket) {
    use futures::SinkExt;
    use tokio::sync::mpsc;

    let (log_tx, mut log_rx) = mpsc::channel::<UpgradeLogEntry>(32);

    // Spawn upgrade task
    let mut upgrade_task = tokio::spawn(async move { perform_upgrade_with_logs(log_tx).await });

    // Forward logs to WebSocket
    loop {
        tokio::select! {
            Some(entry) = log_rx.recv() => {
                let json = serde_json::to_string(&entry).unwrap_or_default();
                if socket.send(axum::extract::ws::Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
            _ = &mut upgrade_task => {
                break;
            }
        }
    }

    // Drain remaining logs
    while let Ok(entry) = log_rx.try_recv() {
        let json = serde_json::to_string(&entry).unwrap_or_default();
        let _ = socket
            .send(axum::extract::ws::Message::Text(json.into()))
            .await;
    }

    let _ = socket.close().await;
}

/// Perform upgrade with real-time logging
async fn perform_upgrade_with_logs(log_tx: tokio::sync::mpsc::Sender<UpgradeLogEntry>) {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let send_log = |step: u8, message: &str, level: &str, progress: Option<u8>| {
        let entry = UpgradeLogEntry {
            step,
            total_steps: 10,
            message: message.to_string(),
            level: level.to_string(),
            progress,
        };
        let tx = log_tx.clone();
        async move {
            let _ = tx.send(entry).await;
        }
    };

    // Step 0: Save running apps state (before upgrade)
    send_log(0, "保存运行中的应用状态...", "info", None).await;
    if let Ok(apps_state) = crate::apps::api::AppsState::new() {
        if let Err(e) = apps_state.save_running_state() {
            log::warn!("Failed to save running apps state: {}", e);
            send_log(0, &format!("保存应用状态失败: {}", e), "error", None).await;
        } else {
            send_log(0, "应用状态已保存", "success", None).await;
        }
    }

    // Step 1: Detect architecture
    send_log(1, "检测系统架构...", "info", None).await;
    let arch = if cfg!(target_arch = "x86_64") {
        "amd64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        send_log(1, "不支持的系统架构", "error", None).await;
        return;
    };
    send_log(1, &format!("系统架构: {}", arch), "success", None).await;

    // Step 2: Build download URL
    send_log(2, "准备下载最新版本...", "info", None).await;
    let download_url = format!(
        "https://github.com/Xiechengqi/iVnc/releases/download/latest/ivnc-linux-{}",
        arch
    );
    send_log(2, &format!("下载地址: {}", download_url), "success", None).await;

    // Step 3: Download new version
    send_log(3, "开始下载...", "info", None).await;
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent("ivnc-updater")
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            send_log(3, &format!("创建 HTTP 客户端失败: {}", e), "error", None).await;
            return;
        }
    };

    let mut response = match client.get(&download_url).send().await {
        Ok(r) => {
            if !r.status().is_success() {
                send_log(3, &format!("下载失败: HTTP {}", r.status()), "error", None).await;
                return;
            }
            r
        }
        Err(e) => {
            send_log(3, &format!("下载失败: {}", e), "error", None).await;
            return;
        }
    };

    let total_size = response.content_length().unwrap_or(0);
    let temp_path = format!(
        "/tmp/ivnc-new-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );

    let mut file = match tokio::fs::File::create(&temp_path).await {
        Ok(f) => f,
        Err(e) => {
            send_log(3, &format!("创建临时文件失败: {}", e), "error", None).await;
            return;
        }
    };

    let mut downloaded = 0u64;
    let mut last_progress = 0u8;

    while let Some(chunk_result) = response.chunk().await.transpose() {
        let chunk = match chunk_result {
            Ok(c) => c,
            Err(e) => {
                send_log(3, &format!("下载数据失败: {}", e), "error", None).await;
                let _ = tokio::fs::remove_file(&temp_path).await;
                return;
            }
        };

        if tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .is_err()
        {
            send_log(3, "写入文件失败", "error", None).await;
            let _ = tokio::fs::remove_file(&temp_path).await;
            return;
        }

        downloaded += chunk.len() as u64;
        if total_size > 0 {
            let progress = ((downloaded as f64 / total_size as f64) * 100.0) as u8;
            if progress != last_progress && (progress % 5 == 0 || progress == 100) {
                send_log(
                    3,
                    &format!(
                        "下载中... {}/{} MB",
                        downloaded / 1024 / 1024,
                        total_size / 1024 / 1024
                    ),
                    "progress",
                    Some(progress),
                )
                .await;
                last_progress = progress;
            }
        } else {
            // No content-length, show downloaded bytes only
            let current_mb = downloaded / 1024 / 1024;
            if current_mb > 0 && current_mb % 5 == 0 && current_mb as u8 != last_progress {
                send_log(3, &format!("下载中... {} MB", current_mb), "info", None).await;
                last_progress = current_mb as u8;
            }
        }
    }
    drop(file);
    send_log(3, "下载完成", "success", None).await;

    // Step 4: Verify download
    send_log(4, "验证下载文件...", "info", None).await;
    let metadata = match tokio::fs::metadata(&temp_path).await {
        Ok(m) => m,
        Err(e) => {
            send_log(4, &format!("读取文件信息失败: {}", e), "error", None).await;
            let _ = tokio::fs::remove_file(&temp_path).await;
            return;
        }
    };

    if metadata.len() < 1024 * 1024 {
        send_log(4, "下载文件过小，可能损坏", "error", None).await;
        let _ = tokio::fs::remove_file(&temp_path).await;
        return;
    }
    send_log(
        4,
        &format!("文件大小: {} MB", metadata.len() / 1024 / 1024),
        "success",
        None,
    )
    .await;

    // Step 5: Backup current version
    send_log(5, "备份当前版本...", "info", None).await;
    let current_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            send_log(5, &format!("获取当前程序路径失败: {}", e), "error", None).await;
            let _ = tokio::fs::remove_file(&temp_path).await;
            return;
        }
    };

    let backup_path = format!(
        "{}.backup-{}",
        current_exe.display(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );

    if let Err(e) = tokio::fs::copy(&current_exe, &backup_path).await {
        send_log(5, &format!("备份失败: {}", e), "error", None).await;
        let _ = tokio::fs::remove_file(&temp_path).await;
        return;
    }
    send_log(5, "备份完成", "success", None).await;

    // Cleanup old backups (keep only the 3 most recent)
    cleanup_old_backups(&current_exe).await;

    // Step 6: Set permissions
    send_log(6, "设置执行权限...", "info", None).await;
    if let Err(e) = fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o755)) {
        send_log(6, &format!("设置权限失败: {}", e), "error", None).await;
        let _ = tokio::fs::remove_file(&temp_path).await;
        return;
    }
    send_log(6, "权限设置完成", "success", None).await;

    // Step 7: Replace binary
    send_log(7, "替换程序文件...", "info", None).await;
    if let Err(e) = tokio::fs::remove_file(&current_exe).await {
        send_log(7, &format!("删除旧文件失败: {}", e), "error", None).await;
        return;
    }

    if let Err(e) = tokio::fs::rename(&temp_path, &current_exe).await {
        send_log(7, &format!("移动新文件失败: {}", e), "error", None).await;
        // Try to restore backup
        let _ = tokio::fs::copy(&backup_path, &current_exe).await;
        return;
    }
    send_log(7, "文件替换完成", "success", None).await;

    // Step 7.5: Verify new binary
    send_log(7, "验证新版本...", "info", None).await;
    match tokio::process::Command::new(&current_exe)
        .arg("--version")
        .output()
        .await
    {
        Ok(output) if output.status.success() => {
            send_log(7, "新版本验证通过", "success", None).await;
        }
        _ => {
            send_log(7, "新版本验证失败，恢复备份", "error", None).await;
            // Restore backup
            let _ = tokio::fs::copy(&backup_path, &current_exe).await;
            return;
        }
    }

    // Step 8: Cleanup
    send_log(8, "清理临时文件...", "info", None).await;
    let _ = tokio::fs::remove_file(&temp_path).await;
    send_log(8, "清理完成", "success", None).await;

    // Step 9: Prepare restart
    send_log(9, "准备重启服务...", "info", None).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    send_log(9, "即将重启", "success", None).await;

    // Step 10: Restart
    send_log(10, "重启服务...", "info", None).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    if let Err(err) = restart_current_process().await {
        let err = err.to_string();
        send_log(10, &format!("重启失败: {}", err), "error", None).await;
    }

    // Try to restore backup
    let _ = fs::remove_file(&current_exe);
    let _ = fs::copy(&backup_path, &current_exe);
    let _ = fs::set_permissions(&current_exe, fs::Permissions::from_mode(0o755));
}

async fn restart_current_process() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if try_restart_systemd("ivnc").await.is_ok() {
        return Ok(());
    }

    let current_exe = std::env::current_exe()?;
    let args: Vec<String> = std::env::args().collect();
    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new(&current_exe)
        .args(&args[1..])
        .exec();
    Err(Box::new(err))
}

/// Try to restart via systemd
async fn try_restart_systemd(service_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let output = tokio::process::Command::new("systemctl")
        .args(&["restart", service_name])
        .output()
        .await?;

    if output.status.success() {
        Ok(())
    } else {
        Err("systemctl restart failed".into())
    }
}

/// Cleanup old backup files, keeping only the most recent N backups
async fn cleanup_old_backups(exe_path: &std::path::Path) {
    use std::path::PathBuf;

    let parent_dir = match exe_path.parent() {
        Some(dir) => dir,
        None => return,
    };

    let exe_name = match exe_path.file_name() {
        Some(name) => name.to_string_lossy(),
        None => return,
    };

    // Find all backup files
    let mut backups: Vec<(PathBuf, u64)> = Vec::new();

    if let Ok(mut entries) = tokio::fs::read_dir(parent_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Ok(file_name) = entry.file_name().into_string() {
                // Match pattern: {exe_name}.backup-{timestamp}
                if file_name.starts_with(&format!("{}.backup-", exe_name)) {
                    if let Some(timestamp_str) =
                        file_name.strip_prefix(&format!("{}.backup-", exe_name))
                    {
                        if let Ok(timestamp) = timestamp_str.parse::<u64>() {
                            backups.push((entry.path(), timestamp));
                        }
                    }
                }
            }
        }
    }

    // Keep only the 3 most recent backups
    const KEEP_COUNT: usize = 3;
    if backups.len() > KEEP_COUNT {
        // Sort by timestamp (newest first)
        backups.sort_by(|a, b| b.1.cmp(&a.1));

        // Delete old backups
        for (path, _) in backups.iter().skip(KEEP_COUNT) {
            let _ = tokio::fs::remove_file(path).await;
            info!("Cleaned up old backup: {:?}", path);
        }
    }
}
