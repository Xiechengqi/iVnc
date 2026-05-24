//! Chrome DevTools Protocol helpers for the built-in Chrome.
//!
//! Currently exposes a single high-level operation: capture a full-page
//! screenshot (including content below the fold) plus the page's readable
//! innerText. The agent loop runs this via `Action::CaptureFullPage` to give
//! the model a one-shot view of long pages instead of paying many
//! scroll+screenshot turns.

use serde_json::{json, Value};

/// Hard cap on the number of characters of extracted page text we return per
/// capture. Keeps the per-turn user prompt bounded even on huge pages.
const FULLPAGE_TEXT_MAX: usize = 20_000;

/// Compute the clipped capture height and the CDP `scale` factor that keeps
/// the resulting bitmap within `max_megapixels`. Returns
/// `(clipped_h, scale, truncated)`.
///
/// Pure / unit-testable so we can verify the math without standing up a Chrome.
pub fn fullpage_scale(
    css_w: f64,
    css_h: f64,
    max_megapixels: f32,
    max_css_height: f64,
) -> (f64, f64, bool) {
    let truncated = css_h > max_css_height;
    let clipped_h = if truncated { max_css_height } else { css_h };
    let natural_pixels = css_w.max(1.0) * clipped_h.max(1.0);
    let cap_pixels = (max_megapixels as f64).max(0.1) * 1_000_000.0;
    let raw_scale = if natural_pixels > cap_pixels {
        (cap_pixels / natural_pixels).sqrt()
    } else {
        1.0
    };
    (clipped_h, raw_scale.clamp(0.05, 1.0), truncated)
}

/// From a `/json/list` response array, pick the first real page tab and return
/// its websocket debugger URL. Skips `chrome://`, `devtools://`, and
/// `about:blank` so we capture user content rather than Chrome's own UI.
/// Chrome returns targets in MRU order, so the first survivor is the
/// foreground tab.
pub fn select_page_target(list: &[Value]) -> Option<String> {
    list.iter()
        .filter(|t| t.get("type").and_then(Value::as_str) == Some("page"))
        .filter(|t| {
            let url = t.get("url").and_then(Value::as_str).unwrap_or("");
            !url.starts_with("chrome://")
                && !url.starts_with("devtools://")
                && url != "about:blank"
        })
        .find_map(|t| {
            t.get("webSocketDebuggerUrl")
                .and_then(Value::as_str)
                .map(|s| s.to_string())
        })
}

/// Full-page capture result returned to the agent loop.
#[cfg(feature = "mcp")]
pub struct FullPageCapture {
    pub jpeg_bytes: Vec<u8>,
    pub image_width: u32,
    pub image_height: u32,
    pub css_content_width: f64,
    pub css_content_height: f64,
    pub truncated: bool,
    pub page_text: Option<String>,
}

/// Capture a full-page screenshot + innerText from the active built-in Chrome
/// tab via CDP. Returns a JPEG (re-encoded after PNG decode and shrunk if it
/// exceeds `max_bytes`) plus the extracted page text.
///
/// Caller must ensure the built-in Chrome is running on `debug_port` before
/// invoking — this function does not start the browser.
#[cfg(feature = "mcp")]
pub async fn capture_full_page(
    debug_port: u16,
    max_megapixels: f32,
    max_bytes: usize,
    max_css_height: f64,
) -> Result<FullPageCapture, String> {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| format!("build cdp http client: {e}"))?;

    let list_url = format!("http://127.0.0.1:{}/json/list", debug_port);
    let list_resp = client
        .get(&list_url)
        .send()
        .await
        .map_err(|e| format!("cdp /json/list failed: {e}"))?;
    if !list_resp.status().is_success() {
        return Err(format!(
            "cdp /json/list returned status {}",
            list_resp.status()
        ));
    }
    let list_json: Value = list_resp
        .json()
        .await
        .map_err(|e| format!("cdp /json/list parse: {e}"))?;
    let targets = list_json
        .as_array()
        .ok_or_else(|| "cdp /json/list was not an array".to_string())?;
    let ws_url = select_page_target(targets)
        .ok_or_else(|| "cdp /json/list contained no user-page tab".to_string())?;

    tokio::time::timeout(
        Duration::from_secs(20),
        capture_via_ws(ws_url, max_megapixels, max_bytes, max_css_height),
    )
    .await
    .map_err(|_| "cdp capture_full_page timed out (20s)".to_string())?
}

#[cfg(feature = "mcp")]
async fn capture_via_ws(
    ws_url: String,
    max_megapixels: f32,
    max_bytes: usize,
    max_css_height: f64,
) -> Result<FullPageCapture, String> {
    use base64::Engine;
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|e| format!("cdp connect_async failed: {e}"))?;

    async fn read_reply<W>(ws: &mut W, want_id: u64) -> Result<Value, String>
    where
        W: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
            + Unpin,
    {
        while let Some(msg) = ws.next().await {
            let msg = msg.map_err(|e| format!("cdp ws recv: {e}"))?;
            let text = match msg {
                Message::Text(t) => t.to_string(),
                Message::Close(_) => return Err("cdp ws closed before reply".to_string()),
                _ => continue,
            };
            let value: Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let Some(id) = value.get("id").and_then(Value::as_u64) else {
                continue;
            };
            if id != want_id {
                continue;
            }
            if let Some(err) = value.get("error") {
                return Err(format!("cdp error (id {id}): {err}"));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
        Err("cdp ws ended without reply".to_string())
    }

    async fn send_cmd<W>(ws: &mut W, id: u64, method: &str, params: Value) -> Result<(), String>
    where
        W: futures::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
    {
        let payload = json!({ "id": id, "method": method, "params": params }).to_string();
        ws.send(Message::Text(payload.into()))
            .await
            .map_err(|e| format!("cdp ws send: {e}"))
    }

    let mut id_counter: u64 = 0;
    let mut next_id = || {
        id_counter += 1;
        id_counter
    };

    // Page.enable establishes the lifecycle event channel and is the standard
    // CDP handshake before any Page.* call.
    let id = next_id();
    send_cmd(&mut ws, id, "Page.enable", json!({})).await?;
    let _ = read_reply(&mut ws, id).await?;

    let id = next_id();
    send_cmd(&mut ws, id, "Page.getLayoutMetrics", json!({})).await?;
    let metrics = read_reply(&mut ws, id).await?;
    let size = metrics
        .get("cssContentSize")
        .or_else(|| metrics.get("contentSize"))
        .ok_or_else(|| "cdp getLayoutMetrics: no contentSize".to_string())?;
    let css_w = size.get("width").and_then(Value::as_f64).unwrap_or(0.0);
    let css_h = size.get("height").and_then(Value::as_f64).unwrap_or(0.0);
    if css_w <= 0.0 || css_h <= 0.0 {
        let _ = ws.close(None).await;
        return Err(format!(
            "cdp getLayoutMetrics returned non-positive size: {css_w}x{css_h}"
        ));
    }

    let (clipped_h, scale, truncated) =
        fullpage_scale(css_w, css_h, max_megapixels, max_css_height);

    let id = next_id();
    send_cmd(
        &mut ws,
        id,
        "Page.captureScreenshot",
        json!({
            "format": "png",
            "captureBeyondViewport": true,
            "fromSurface": true,
            "clip": {
                "x": 0.0,
                "y": 0.0,
                "width": css_w,
                "height": clipped_h,
                "scale": scale,
            }
        }),
    )
    .await?;
    let shot = read_reply(&mut ws, id).await?;
    let data_b64 = shot
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| "cdp captureScreenshot: no data".to_string())?;
    let png_bytes = base64::engine::general_purpose::STANDARD
        .decode(data_b64)
        .map_err(|e| format!("cdp captureScreenshot base64 decode: {e}"))?;

    // Page text extraction — failures are non-fatal; the screenshot alone is
    // still useful, so we just leave page_text=None and log a warning.
    let page_text = {
        let id = next_id();
        let send_res = send_cmd(
            &mut ws,
            id,
            "Runtime.evaluate",
            json!({
                "expression": "(document.body && document.body.innerText) || ''",
                "returnByValue": true,
            }),
        )
        .await;
        match send_res {
            Err(e) => {
                log::warn!("cdp Runtime.evaluate send failed: {e}");
                None
            }
            Ok(()) => match read_reply(&mut ws, id).await {
                Ok(result) => result
                    .get("result")
                    .and_then(|r| r.get("value"))
                    .and_then(Value::as_str)
                    .map(|s| {
                        if s.chars().count() > FULLPAGE_TEXT_MAX {
                            s.chars().take(FULLPAGE_TEXT_MAX).collect::<String>()
                        } else {
                            s.to_string()
                        }
                    }),
                Err(e) => {
                    log::warn!("cdp Runtime.evaluate failed: {e}");
                    None
                }
            },
        }
    };

    let _ = ws.close(None).await;

    let img = image::load_from_memory_with_format(&png_bytes, image::ImageFormat::Png)
        .map_err(|e| format!("cdp png decode failed: {e}"))?
        .to_rgb8();
    let (jpeg_bytes, image_width, image_height, _q) =
        crate::mcp::frame_capture::rgb_to_capped_jpeg(img, 75, max_bytes)?;

    Ok(FullPageCapture {
        jpeg_bytes,
        image_width,
        image_height,
        css_content_width: css_w,
        css_content_height: css_h,
        truncated,
        page_text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fullpage_scale_no_cap() {
        let (h, s, truncated) = fullpage_scale(1280.0, 800.0, 100.0, 20_000.0);
        assert_eq!(h, 800.0);
        assert!((s - 1.0).abs() < 1e-9);
        assert!(!truncated);
    }

    #[test]
    fn fullpage_scale_megapixel_cap() {
        let (h, s, truncated) = fullpage_scale(1280.0, 20_000.0, 4.0, 25_000.0);
        assert_eq!(h, 20_000.0);
        assert!(!truncated);
        // nat = 1280*20000 = 25.6M; cap = 4M. scale = sqrt(4/25.6) ≈ 0.395.
        let expected = (4.0_f64 / 25.6_f64).sqrt();
        assert!((s - expected).abs() < 1e-6, "got {s}, expected ~{expected}");
    }

    #[test]
    fn fullpage_scale_truncates_long_page() {
        let (h, _s, truncated) = fullpage_scale(1280.0, 30_000.0, 4.0, 20_000.0);
        assert_eq!(h, 20_000.0);
        assert!(truncated);
    }

    #[test]
    fn fullpage_scale_clamps_lower_bound() {
        // Pathological 1M-by-1M page would otherwise underflow to ~0.002 scale.
        let (_h, s, _t) = fullpage_scale(1_000_000.0, 1_000_000.0, 4.0, 1_000_000.0);
        assert!(s >= 0.05, "scale {s} not clamped to >= 0.05");
    }

    #[test]
    fn select_page_target_skips_internal_pages() {
        let list = serde_json::json!([
            {"type":"page","url":"chrome://newtab","webSocketDebuggerUrl":"ws://x/a"},
            {"type":"page","url":"devtools://devtools/x","webSocketDebuggerUrl":"ws://x/b"},
            {"type":"background_page","url":"https://e/","webSocketDebuggerUrl":"ws://x/c"},
            {"type":"page","url":"https://example.com/foo","webSocketDebuggerUrl":"ws://x/d"},
            {"type":"page","url":"https://example.com/bar","webSocketDebuggerUrl":"ws://x/e"},
        ]);
        let url = select_page_target(list.as_array().unwrap()).unwrap();
        assert_eq!(url, "ws://x/d");
    }

    #[test]
    fn select_page_target_none_when_only_internal() {
        let list = serde_json::json!([
            {"type":"page","url":"chrome://newtab","webSocketDebuggerUrl":"ws://x/a"},
            {"type":"page","url":"about:blank","webSocketDebuggerUrl":"ws://x/b"},
        ]);
        assert!(select_page_target(list.as_array().unwrap()).is_none());
    }
}
