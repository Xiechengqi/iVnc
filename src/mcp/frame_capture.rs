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

    // First attempt at original resolution
    let jpeg = encode_jpeg(&img, quality)?;
    if jpeg.len() <= max_bytes {
        return Ok(encoded_frame(jpeg, width, height, width, height, quality));
    }

    // Downscale if too large
    let scale = (max_bytes as f64 / jpeg.len() as f64)
        .sqrt()
        .clamp(0.25, 1.0);
    let new_w = ((width as f64 * scale) as u32).max(1);
    let new_h = ((height as f64 * scale) as u32).max(1);

    let resized =
        image::imageops::resize(&img, new_w, new_h, image::imageops::FilterType::Triangle);
    let out_quality = quality.min(75);
    let jpeg = encode_jpeg(&resized, out_quality)?;
    Ok(encoded_frame(
        jpeg,
        width,
        height,
        new_w,
        new_h,
        out_quality,
    ))
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
}
