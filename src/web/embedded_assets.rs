//! Embedded web assets using rust-embed
//!
//! This module embeds the web UI assets directly into the binary,
//! eliminating the need for external file dependencies.

use axum::{
    body::Body,
    http::{header, StatusCode},
    response::Response,
};
use rust_embed::RustEmbed;

/// Embedded web UI assets from the Vite build output
#[derive(RustEmbed)]
#[folder = "web/selkies/dist"]
pub struct WebAssets;

/// Get an embedded file and return it as an Axum response
pub fn get_embedded_file(path: &str) -> Response {
    // Normalize path: remove leading slash, default to index.html
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    match WebAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                .header(header::CACHE_CONTROL, cache_control_for_path(path))
                .body(Body::from(content.data.into_owned()))
                .unwrap()
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("Not found"))
            .unwrap(),
    }
}

/// Get index.html with WebSocket port injection
pub fn get_index_html_with_port(ws_port: u16) -> Response {
    match WebAssets::get("index.html") {
        Some(content) => {
            let html = match std::str::from_utf8(&content.data) {
                Ok(s) => s.to_string(),
                Err(_) => {
                    return Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::from("Invalid UTF-8 in index.html"))
                        .unwrap()
                }
            };

            // Inject WebSocket port
            let html = html.replace("__SELKIES_INJECTED_PORT__", &ws_port.to_string());

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CACHE_CONTROL, "no-store, max-age=0")
                .body(Body::from(html))
                .unwrap()
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("index.html not found"))
            .unwrap(),
    }
}

/// Check if embedded assets are available
pub fn has_embedded_assets() -> bool {
    WebAssets::get("index.html").is_some()
}

/// List all embedded files (for debugging)
#[allow(dead_code)]
pub fn list_embedded_files() -> Vec<String> {
    WebAssets::iter().map(|s| s.to_string()).collect()
}

/// Determine cache control header based on file type
fn cache_control_for_path(path: &str) -> &'static str {
    if path == "index.html" {
        // Never cache index.html to ensure fresh content
        "no-store, max-age=0"
    } else if path.ends_with(".js") || path.ends_with(".css") {
        // Cache JS/CSS for 1 year (they have hashed filenames)
        "public, max-age=31536000, immutable"
    } else if path.ends_with(".woff2") || path.ends_with(".woff") || path.ends_with(".ttf") {
        // Cache fonts for 1 year
        "public, max-age=31536000, immutable"
    } else {
        // Default: cache for 1 hour
        "public, max-age=3600"
    }
}
