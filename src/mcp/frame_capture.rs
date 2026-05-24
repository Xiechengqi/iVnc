//! Frame capture for MCP screenshot tool.
//!
//! Bridges the calloop compositor thread (which renders frames) with
//! tokio-based MCP tool handlers via oneshot channels.

use base64::Engine;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::oneshot;

#[cfg(feature = "agent")]
use crate::agent::types::{DisplayMetadata, MonitorRect, Observation, ObservationFrame};
use crate::web::SharedState;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct EncodedFrame {
    pub bytes: Vec<u8>,
    pub image_width: u32,
    pub image_height: u32,
    pub quality: u8,
    pub image_to_screen_scale_x: f32,
    pub image_to_screen_scale_y: f32,
    pub sha256: String,
}

/// Request a frame capture from the compositor main loop.
/// Returns (width, height, xrgb8888_pixels).
pub async fn capture_frame(state: &Arc<SharedState>) -> Result<(u32, u32, Vec<u8>), String> {
    let (tx, rx) = oneshot::channel();
    state
        .frame_capture_tx
        .send(tx)
        .map_err(|_| "compositor not running")?;

    tokio::time::timeout(std::time::Duration::from_secs(2), rx)
        .await
        .map_err(|_| "frame capture timed out (2s)")?
        .map_err(|_| "compositor dropped frame capture request".to_string())
}

/// Convert XRGB8888 pixel buffer to JPEG, returning base64-encoded string.
/// If the result exceeds `max_bytes`, downscale and re-encode.
pub fn xrgb_to_jpeg_base64(
    width: u32,
    height: u32,
    xrgb: &[u8],
    quality: u8,
    max_bytes: usize,
) -> Result<String, String> {
    let encoded = xrgb_to_encoded_frame(width, height, xrgb, quality, max_bytes)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&encoded.bytes))
}

/// Convert XRGB8888 pixel buffer to JPEG and keep the metadata needed to map
/// provider-visible image coordinates back into compositor screen coordinates.
pub fn xrgb_to_encoded_frame(
    width: u32,
    height: u32,
    xrgb: &[u8],
    quality: u8,
    max_bytes: usize,
) -> Result<EncodedFrame, String> {
    use image::{ImageBuffer, RgbImage};

    // Convert XRGB8888 → RGB
    let mut rgb_buf: Vec<u8> = Vec::with_capacity((width * height * 3) as usize);
    for pixel in xrgb.chunks_exact(4) {
        rgb_buf.push(pixel[2]); // R  (XRGB8888 LE memory: [B, G, R, X])
        rgb_buf.push(pixel[1]); // G
        rgb_buf.push(pixel[0]); // B
    }

    let img: RgbImage =
        ImageBuffer::from_raw(width, height, rgb_buf).ok_or("failed to create image buffer")?;

    let (bytes, image_w, image_h, out_quality) = rgb_to_capped_jpeg(img, quality, max_bytes)?;
    Ok(encoded_frame(bytes, width, height, image_w, image_h, out_quality))
}

/// Encode an RgbImage as JPEG, downscaling once if the encoded bytes exceed
/// `max_bytes`. Returns `(bytes, image_width, image_height, quality_used)`.
/// Shared between the live-viewport XRGB path and the CDP full-page path.
pub(crate) fn rgb_to_capped_jpeg(
    img: image::RgbImage,
    quality: u8,
    max_bytes: usize,
) -> Result<(Vec<u8>, u32, u32, u8), String> {
    let (width, height) = (img.width(), img.height());

    // First attempt at original resolution.
    let jpeg = encode_jpeg(&img, quality)?;
    if jpeg.len() <= max_bytes {
        return Ok((jpeg, width, height, quality));
    }

    // Downscale if too large.
    let scale = (max_bytes as f64 / jpeg.len() as f64)
        .sqrt()
        .clamp(0.25, 1.0);
    let new_w = ((width as f64 * scale) as u32).max(1);
    let new_h = ((height as f64 * scale) as u32).max(1);

    let resized =
        image::imageops::resize(&img, new_w, new_h, image::imageops::FilterType::Triangle);
    let out_quality = quality.min(75);
    let jpeg = encode_jpeg(&resized, out_quality)?;
    Ok((jpeg, new_w, new_h, out_quality))
}

/// Full-page (read-only) capture via the built-in Chrome CDP. Returns an
/// Observation with `display.read_only=true` and `page_text` populated from
/// the page's `document.body.innerText`. Falls back to a normal live capture
/// (with a warning log) if CDP capture fails, so an agent run never breaks
/// solely because the browser is in a weird state.
#[cfg(feature = "agent")]
pub async fn capture_fullpage_observation(
    state: &Arc<SharedState>,
    options: &crate::agent::types::RunOptions,
    max_megapixels: f32,
) -> Result<Observation, String> {
    let port = crate::apps::api::chrome_devtools_port();
    let capture = match crate::apps::cdp::capture_full_page(
        port,
        max_megapixels,
        options.screenshot_max_bytes,
        options.fullpage_max_css_height,
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            log::warn!("capture_full_page failed, falling back to live viewport: {}", e);
            return capture_observation(state, 75, options.screenshot_max_bytes).await;
        }
    };

    let timestamp_ms = crate::agent::types::now_ms();
    let (screen_width, screen_height) = state.display_size();
    let mut hasher = Sha256::new();
    hasher.update(&capture.jpeg_bytes);
    let sha256 = format!("{:x}", hasher.finalize());
    let display = DisplayMetadata {
        screen_width,
        screen_height,
        image_width: capture.image_width,
        image_height: capture.image_height,
        // The full-page frame is read-only; to_screen is never called on it,
        // so the scale factors are placeholders.
        image_to_screen_scale_x: 1.0,
        image_to_screen_scale_y: 1.0,
        client_dpr: None,
        monitors: Vec::new(),
        read_only: true,
    };
    Ok(Observation {
        frame: ObservationFrame::JpegBytes {
            image_width: capture.image_width,
            image_height: capture.image_height,
            bytes: capture.jpeg_bytes,
            quality: 75,
            sha256,
            frame_path: None,
        },
        display,
        windows: state
            .last_taskbar_json
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|j| serde_json::from_str(j).ok()),
        clipboard_preview: state.clipboard_preview(256),
        page_text: capture.page_text,
        timestamp_ms,
    })
}

#[cfg(feature = "agent")]
pub async fn capture_observation(
    state: &Arc<SharedState>,
    quality: u8,
    max_bytes: usize,
) -> Result<Observation, String> {
    let (screen_width, screen_height, pixels) = capture_frame(state).await?;
    let encoded = xrgb_to_encoded_frame(screen_width, screen_height, &pixels, quality, max_bytes)?;
    let timestamp_ms = crate::agent::types::now_ms();
    let display = DisplayMetadata {
        screen_width,
        screen_height,
        image_width: encoded.image_width,
        image_height: encoded.image_height,
        image_to_screen_scale_x: encoded.image_to_screen_scale_x,
        image_to_screen_scale_y: encoded.image_to_screen_scale_y,
        client_dpr: None,
        monitors: vec![MonitorRect {
            x: 0,
            y: 0,
            width: screen_width,
            height: screen_height,
        }],
        read_only: false,
    };
    Ok(Observation {
        frame: ObservationFrame::JpegBytes {
            image_width: encoded.image_width,
            image_height: encoded.image_height,
            bytes: encoded.bytes,
            quality: encoded.quality,
            sha256: encoded.sha256,
            frame_path: None,
        },
        display,
        windows: state
            .last_taskbar_json
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|j| serde_json::from_str(j).ok()),
        clipboard_preview: state.clipboard_preview(256),
        page_text: None,
        timestamp_ms,
    })
}

fn encode_jpeg<P, C>(img: &image::ImageBuffer<P, C>, quality: u8) -> Result<Vec<u8>, String>
where
    P: image::Pixel<Subpixel = u8> + image::PixelWithColorType + 'static,
    C: std::ops::Deref<Target = [u8]>,
{
    use image::codecs::jpeg::JpegEncoder;
    use std::io::Cursor;

    let mut buf = Cursor::new(Vec::new());
    let encoder = JpegEncoder::new_with_quality(&mut buf, quality);
    img.write_with_encoder(encoder)
        .map_err(|e| format!("JPEG encode failed: {}", e))?;
    Ok(buf.into_inner())
}

fn encoded_frame(
    bytes: Vec<u8>,
    screen_width: u32,
    screen_height: u32,
    image_width: u32,
    image_height: u32,
    quality: u8,
) -> EncodedFrame {
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let sha256 = format!("{:x}", hasher.finalize());
    EncodedFrame {
        bytes,
        image_width,
        image_height,
        quality,
        image_to_screen_scale_x: screen_width as f32 / image_width.max(1) as f32,
        image_to_screen_scale_y: screen_height as f32 / image_height.max(1) as f32,
        sha256,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_frame_tracks_scale() {
        let frame = encoded_frame(vec![1, 2, 3], 1920, 1080, 960, 540, 75);
        assert_eq!(frame.image_to_screen_scale_x, 2.0);
        assert_eq!(frame.image_to_screen_scale_y, 2.0);
        assert_eq!(frame.image_width, 960);
        assert_eq!(frame.image_height, 540);
    }

    #[test]
    fn rgb_to_capped_jpeg_under_cap_keeps_dims() {
        let img = image::RgbImage::from_pixel(64, 64, image::Rgb([200, 100, 50]));
        let (bytes, w, h, q) = rgb_to_capped_jpeg(img, 80, 1_000_000).unwrap();
        assert_eq!((w, h), (64, 64));
        assert!(!bytes.is_empty());
        assert_eq!(q, 80);
    }

    #[test]
    fn rgb_to_capped_jpeg_over_cap_downscales() {
        // Random-ish gradient compresses poorly, so a generous original size
        // ensures the JPEG bytes exceed the tight cap on the first attempt.
        let mut img = image::RgbImage::new(512, 512);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgb([((x * 7) % 256) as u8, ((y * 11) % 256) as u8, ((x ^ y) % 256) as u8]);
        }
        let (bytes, w, h, q) = rgb_to_capped_jpeg(img, 95, 2_000).unwrap();
        assert!(w < 512 && h < 512, "expected downscale, got {}x{}", w, h);
        assert!(bytes.len() <= 2_000 || (w * h) < 512 * 512, "no resize");
        assert!(q <= 75);
    }
}
