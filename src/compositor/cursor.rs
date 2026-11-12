//! Convert SHM cursor surfaces to the browser's existing PNG cursor protocol.
use super::Compositor;
use base64::Engine;
use smithay::{
    backend::renderer::utils::RendererSurfaceStateUserData,
    input::pointer::{CursorImageStatus, CursorImageSurfaceData},
    reexports::wayland_server::{protocol::wl_shm::Format, Resource},
    utils::Transform,
    wayland::{compositor::with_states, shm::with_buffer_contents},
};

impl Compositor {
    pub fn cursor_message(&self) -> String {
        let override_message =
            |name: &str| format!("cursor,{}", serde_json::json!({"override": name}));
        let surface = match &self.cursor_status {
            CursorImageStatus::Hidden => return override_message("none"),
            CursorImageStatus::Named(icon) => return override_message(icon.name()),
            CursorImageStatus::Surface(s) if s.is_alive() => s,
            _ => return override_message("default"),
        };
        let image = with_states(surface, |states| {
            let hotspot = states
                .data_map
                .get::<CursorImageSurfaceData>()?
                .lock()
                .ok()?
                .hotspot;
            let data = states
                .data_map
                .get::<RendererSurfaceStateUserData>()?
                .lock()
                .ok()?;
            // Cursor transforms/viewport crops are uncommon. Do not read their
            // buffers with an incorrect geometry; use the default in that case.
            if data.buffer_transform() != Transform::Normal {
                return None;
            }
            let buffer_size = data.buffer_size()?;
            let view = data.view()?;
            if buffer_size != data.surface_size()?
                || view.src.loc != (0.0, 0.0).into()
                || view.src.size != buffer_size.to_f64()
            {
                return None;
            }
            let buffer = data.buffer()?;
            let scale = data.buffer_scale().max(1) as usize;
            let image = with_buffer_contents(buffer, |ptr, len, meta| {
                if meta.width <= 0
                    || meta.height <= 0
                    || meta.offset < 0
                    || meta.stride < meta.width.checked_mul(4)?
                {
                    return None;
                }
                let (width, height) = (meta.width as usize / scale, meta.height as usize / scale);
                if width == 0 || height == 0 || width > 128 || height > 128 {
                    return None;
                }
                if !matches!(meta.format, Format::Argb8888 | Format::Xrgb8888) {
                    return None;
                }
                let end = (meta.offset as usize)
                    .checked_add((meta.stride as usize).checked_mul(meta.height as usize)?)?;
                if end > len {
                    return None;
                }
                let mut rgba = Vec::with_capacity(width * height * 4);
                // Read only inside Smithay's guarded SHM mapping callback.
                for y in 0..height {
                    for x in 0..width {
                        let offset =
                            meta.offset as usize + y * scale * meta.stride as usize + x * scale * 4;
                        let pixel = u32::from_ne_bytes(unsafe {
                            std::ptr::read_volatile(ptr.add(offset).cast::<[u8; 4]>())
                        });
                        let a = if meta.format == Format::Xrgb8888 {
                            255
                        } else {
                            (pixel >> 24) as u8
                        };
                        // Wayland ARGB is premultiplied; PNG uses straight alpha.
                        let unpremultiply = |v: u8| {
                            if a == 0 {
                                0
                            } else {
                                ((v as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8
                            }
                        };
                        rgba.extend_from_slice(&[
                            unpremultiply((pixel >> 16) as u8),
                            unpremultiply((pixel >> 8) as u8),
                            unpremultiply(pixel as u8),
                            a,
                        ]);
                    }
                }
                let mut png = Vec::new();
                let mut encoder = png::Encoder::new(&mut png, width as u32, height as u32);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder.write_header().ok()?.write_image_data(&rgba).ok()?;
                Some((png, width as i32, height as i32))
            })
            .ok()??;
            Some((image, hotspot))
        });
        match image {
            Some(((png, width, height), hotspot)) => format!(
                "cursor,{}",
                serde_json::json!({
                    "handle": 1, "curdata": base64::engine::general_purpose::STANDARD.encode(png),
                    "hotx": hotspot.x.clamp(0, width - 1), "hoty": hotspot.y.clamp(0, height - 1)
                })
            ),
            None => override_message("default"),
        }
    }
}
