//! Authoritative desktop geometry. Viewer viewports never change this state.
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tokio::sync::oneshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn validate(self) -> Result<Self, String> {
        // Bound compositor buffers as well as individual axes (maximum 4K).
        if self.width < 320
            || self.height < 200
            || self.width > 7680
            || self.height > 4320
            || u64::from(self.width) * u64::from(self.height) > 3840 * 2160
            || self.width % 2 != 0
            || self.height % 2 != 0
        {
            return Err(
                "Use even dimensions, width 320–7680, height 200–4320, at most 8,294,400 pixels"
                    .into(),
            );
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DisplayState {
    pub protocol: u8,
    /// Changes only when desktop coordinates change, never for an encoder rebuild.
    pub revision: u64,
    pub desktop: Size,
    pub encoded: Size,
    pub locked: bool,
    pub pending: Option<Size>,
    /// A previous resize failure is diagnostic state, not an HTTP request error.
    pub last_error: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Resize {
    pub width: u32,
    pub height: u32,
    pub revision: u64,
}

pub struct ResizeRequest {
    pub size: Size,
    reply: oneshot::Sender<Result<DisplayState, String>>,
}

struct Inner {
    state: DisplayState,
    queued: Option<ResizeRequest>,
}

pub struct DisplayController(Mutex<Inner>);

impl DisplayController {
    pub fn new(size: Size, locked: bool) -> Self {
        Self(Mutex::new(Inner {
            state: DisplayState {
                protocol: 1,
                revision: 1,
                desktop: size,
                encoded: size,
                locked,
                pending: None,
                last_error: None,
            },
            queued: None,
        }))
    }

    pub fn snapshot(&self) -> DisplayState {
        self.0.lock().unwrap().state.clone()
    }

    /// Only authenticated HTTP management handlers call this. No DC resize path.
    pub fn request(
        &self,
        resize: Resize,
    ) -> Result<oneshot::Receiver<Result<DisplayState, String>>, (u16, String)> {
        let size = Size::new(resize.width, resize.height)
            .validate()
            .map_err(|e| (400, e))?;
        let mut inner = self.0.lock().unwrap();
        if inner.state.locked {
            return Err((423, "Desktop size is locked by server configuration".into()));
        }
        if resize.revision != inner.state.revision || inner.state.pending.is_some() {
            return Err((
                409,
                "Desktop size changed or another change is pending; reload before applying".into(),
            ));
        }
        let (reply, rx) = oneshot::channel();
        inner.state.pending = Some(size);
        inner.state.last_error = None;
        inner.queued = Some(ResizeRequest { size, reply });
        Ok(rx)
    }

    pub fn take_request(&self) -> Option<ResizeRequest> {
        let mut inner = self.0.lock().unwrap();
        let request = inner.queued.take()?;
        if request.reply.is_closed() {
            inner.state.pending = None;
            return None;
        }
        Some(request)
    }

    /// Called after backend, encoder and persistent settings have all succeeded.
    pub fn complete(&self, request: ResizeRequest, result: Result<(), String>, encoded: Size) {
        let mut inner = self.0.lock().unwrap();
        inner.state.pending = None;
        if result.is_ok() {
            inner.state.encoded = encoded;
        }
        if result.is_ok() && inner.state.desktop != request.size {
            inner.state.desktop = request.size;
            inner.state.revision += 1;
        }
        inner.state.last_error = result.as_ref().err().cloned();
        let _ = request.reply.send(result.map(|_| inner.state.clone()));
    }

    pub fn set_encoded(&self, size: Size) -> bool {
        let mut inner = self.0.lock().unwrap();
        let changed = inner.state.encoded != size;
        inner.state.encoded = size;
        changed
    }
}

/// Locked config > locked environment settings > saved selection > display config.
pub fn initial_size(
    config: &crate::config::Config,
    ui: &crate::config::ui::UiConfig,
) -> Result<(Size, bool), String> {
    resolve_size(config, ui, crate::console_config::load().display)
}

fn resolve_size(
    config: &crate::config::Config,
    ui: &crate::config::ui::UiConfig,
    saved: Option<Size>,
) -> Result<(Size, bool), String> {
    let locked = ui.screen.manual_resolution.locked && ui.screen.manual_resolution.value;
    let configured = Size::new(config.display.width, config.display.height);
    if config.display.locked {
        return Ok((configured.validate()?, true));
    }
    if locked {
        return Ok((
            Size::new(ui.screen.width.value, ui.screen.height.value).validate()?,
            true,
        ));
    }
    if let Some(saved) = saved {
        match saved.validate() {
            Ok(size) => return Ok((size, false)),
            Err(err) => log::warn!("Ignoring invalid saved desktop size: {err}"),
        }
    }
    Ok((configured.validate()?, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_precedence_and_invalid_saved_values() {
        let mut config = crate::config::Config::default();
        let mut ui = crate::config::ui::UiConfig::from_env(&config);
        ui.screen.manual_resolution.locked = false;
        assert_eq!(
            resolve_size(&config, &ui, Some(Size::new(800, 600))).unwrap(),
            (Size::new(800, 600), false)
        );
        assert_eq!(
            resolve_size(&config, &ui, Some(Size::new(8000, 6000))).unwrap(),
            (Size::new(1920, 1080), false)
        );
        ui.screen.manual_resolution.value = true;
        ui.screen.manual_resolution.locked = true;
        ui.screen.width.value = 1280;
        ui.screen.height.value = 720;
        assert_eq!(
            resolve_size(&config, &ui, Some(Size::new(800, 600))).unwrap(),
            (Size::new(1280, 720), true)
        );
        config.display.locked = true;
        assert_eq!(
            resolve_size(&config, &ui, Some(Size::new(800, 600))).unwrap(),
            (Size::new(1920, 1080), true)
        );
    }
    #[test]
    fn resource_bounds() {
        assert!(Size::new(3840, 2160).validate().is_ok());
        for (w, h) in [(0, 600), (800, 0), (801, 600), (7680, 4320), (8000, 200)] {
            assert!(Size::new(w, h).validate().is_err());
        }
    }
    #[test]
    fn changes_are_transactional_and_revisioned() {
        let c = DisplayController::new(Size::new(1920, 1080), false);
        let mut rx = c
            .request(Resize {
                width: 1280,
                height: 720,
                revision: 1,
            })
            .unwrap();
        assert!(c
            .request(Resize {
                width: 800,
                height: 600,
                revision: 1
            })
            .is_err());
        assert_eq!(c.snapshot().desktop, Size::new(1920, 1080));
        let r = c.take_request().unwrap();
        c.complete(r, Err("allocation failed".into()), Size::new(1280, 720));
        assert!(rx.try_recv().unwrap().is_err());
        assert_eq!(c.snapshot().revision, 1);
        let snapshot = serde_json::to_value(c.snapshot()).unwrap();
        assert_eq!(snapshot["last_error"], "allocation failed");
        assert!(snapshot.get("error").is_none());
        let mut rx = c
            .request(Resize {
                width: 1280,
                height: 720,
                revision: 1,
            })
            .unwrap();
        let r = c.take_request().unwrap();
        c.complete(r, Ok(()), Size::new(1280, 720));
        assert_eq!(rx.try_recv().unwrap().unwrap().revision, 2);
        assert!(c.snapshot().last_error.is_none());
        assert!(c
            .request(Resize {
                width: 800,
                height: 600,
                revision: 1
            })
            .is_err());
        c.set_encoded(Size::new(640, 360));
        assert_eq!(c.snapshot().desktop, Size::new(1280, 720));
        assert_eq!(c.snapshot().revision, 2);
    }
    #[test]
    fn locked_and_cancelled_requests() {
        let c = DisplayController::new(Size::new(800, 600), true);
        assert_eq!(
            c.request(Resize {
                width: 1280,
                height: 720,
                revision: 1
            })
            .unwrap_err()
            .0,
            423
        );
        let c = DisplayController::new(Size::new(800, 600), false);
        drop(
            c.request(Resize {
                width: 1280,
                height: 720,
                revision: 1,
            })
            .unwrap(),
        );
        assert!(c.take_request().is_none());
        assert!(c.snapshot().pending.is_none());
    }
}
