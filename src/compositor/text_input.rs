//! Browser IME commits delivered to the enabled text-input-v3 instance.
//!
//! Smithay's text-input manager forwards requests to an external input method.
//! iVnc is itself the input method, so it owns enable/commit state directly.
use super::Compositor;
use smithay::reexports::{
    wayland_protocols::wp::text_input::zv3::server::{
        zwp_text_input_manager_v3::{self, ZwpTextInputManagerV3},
        zwp_text_input_v3::{self, ZwpTextInputV3},
    },
    wayland_server::{
        backend::{ClientId, ObjectId},
        protocol::wl_surface::WlSurface,
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
    },
};
use std::sync::Mutex;

#[derive(Default, Debug)]
pub struct InputState {
    serial: u32,
    enabled: bool,
    pending_enabled: Option<bool>,
    purpose: u32,
    pending_purpose: Option<u32>,
    cursor: Option<[i32; 4]>,
    pending_cursor: Option<[i32; 4]>,
    pending_reset: bool,
}

impl InputState {
    fn commit(&mut self) {
        if std::mem::take(&mut self.pending_reset) {
            self.purpose = 0;
            self.cursor = None;
        }
        if let Some(enabled) = self.pending_enabled.take() {
            self.enabled = enabled;
        }
        if let Some(purpose) = self.pending_purpose.take() {
            self.purpose = purpose;
        }
        if let Some(cursor) = self.pending_cursor.take() {
            self.cursor = Some(cursor);
        }
        if !self.enabled {
            self.purpose = 0;
            self.cursor = None;
        }
    }
}

#[derive(Default)]
pub struct TextInputState {
    instances: Vec<ZwpTextInputV3>,
    focus: Option<WlSurface>,
    pub dirty: bool,
}

impl TextInputState {
    pub fn new(dh: &DisplayHandle) -> Self {
        dh.create_global::<Compositor, ZwpTextInputManagerV3, _>(1, ());
        Self::default()
    }

    pub fn set_focus(&mut self, surface: Option<&WlSurface>) {
        if self.focus.as_ref() == surface {
            return;
        }
        self.instances.retain(Resource::is_alive);
        for instance in &self.instances {
            if let Some(old) = self
                .focus
                .as_ref()
                .filter(|s| s.id().same_client_as(&instance.id()))
            {
                instance.leave(old);
                let mut state = instance
                    .data::<Mutex<InputState>>()
                    .unwrap()
                    .lock()
                    .unwrap();
                state.enabled = false;
                state.pending_enabled = None;
                state.pending_purpose = None;
                state.pending_cursor = None;
                state.pending_reset = false;
            }
        }
        self.focus = surface.cloned();
        self.dirty = true;
        for instance in &self.instances {
            if let Some(surface) = surface.filter(|s| s.id().same_client_as(&instance.id())) {
                instance.enter(surface);
            }
        }
    }

    /// False means the focused editor did not enable this protocol. Never infer
    /// support just from a client having bound the manager.
    pub fn commit_text(&self, text: &str) -> bool {
        let Some(focus) = self.focus.as_ref().filter(|s| s.is_alive()) else {
            return false;
        };
        // Keep every event below Wayland's maximum message size.
        if text.len() > 48 * 1024 || text.contains('\0') {
            return false;
        }
        for instance in self
            .instances
            .iter()
            .rev()
            .filter(|i| i.is_alive() && i.id().same_client_as(&focus.id()))
        {
            let state = instance
                .data::<Mutex<InputState>>()
                .unwrap()
                .lock()
                .unwrap();
            if state.enabled {
                instance.commit_string(Some(text.to_owned()));
                instance.done(state.serial);
                return true;
            }
        }
        false
    }

    fn focused_client(&self, object: &ObjectId) -> bool {
        self.focus
            .as_ref()
            .is_some_and(|s| s.is_alive() && s.id().same_client_as(object))
    }
}

impl Compositor {
    /// Only editor capabilities and caret geometry leave the compositor. Never
    /// expose surrounding text (especially passwords) to browser viewers.
    pub fn text_input_message(&self) -> String {
        let focus = self.text_input.focus.as_ref().filter(|s| s.is_alive());
        let top = focus.map(|s| self.toplevel_surface(s));
        let key = top
            .as_ref()
            .and_then(|s| self.window_keys.get(&s.id()))
            .map(uuid::Uuid::to_string);
        let mut enabled = false;
        let mut purpose = 0;
        let mut cursor = None;
        if let Some(surface) = focus {
            for instance in self
                .text_input
                .instances
                .iter()
                .rev()
                .filter(|i| i.is_alive() && i.id().same_client_as(&surface.id()))
            {
                let data = instance
                    .data::<Mutex<InputState>>()
                    .unwrap()
                    .lock()
                    .unwrap();
                if !data.enabled {
                    continue;
                }
                enabled = true;
                purpose = data.purpose;
                // Popup/subsurface coordinates require their own transforms;
                // omit those rather than send a misleading desktop position.
                if top.as_ref() == Some(surface) {
                    cursor = data.cursor.and_then(|mut rect| {
                        let window = self.window_for_surface(surface)?;
                        let location = self.space.element_location(&window)?;
                        rect[0] = rect[0].saturating_add(location.x);
                        rect[1] = rect[1].saturating_add(location.y);
                        Some(rect)
                    });
                }
                break;
            }
        }
        serde_json::json!({"type":"text_input", "data": {
            "protocol":1, "enabled":enabled, "focus":key, "purpose":purpose, "cursor":cursor
        }})
        .to_string()
    }
}

impl GlobalDispatch<ZwpTextInputManagerV3, ()> for Compositor {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ZwpTextInputManagerV3>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}

impl Dispatch<ZwpTextInputManagerV3, ()> for Compositor {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ZwpTextInputManagerV3,
        request: zwp_text_input_manager_v3::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let zwp_text_input_manager_v3::Request::GetTextInput { id, seat } = request {
            let input = init.init(id, Mutex::new(InputState::default()));
            if smithay::input::Seat::<Compositor>::from_resource(&seat).as_ref()
                != Some(&state.seat)
            {
                return;
            }
            if let Some(focus) = state
                .text_input
                .focus
                .as_ref()
                .filter(|s| s.id().same_client_as(&input.id()))
            {
                input.enter(focus);
            }
            state.text_input.instances.push(input);
        }
    }
}

impl Dispatch<ZwpTextInputV3, Mutex<InputState>> for Compositor {
    fn request(
        state: &mut Self,
        _: &Client,
        input: &ZwpTextInputV3,
        request: zwp_text_input_v3::Request,
        data: &Mutex<InputState>,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        let mut data = data.lock().unwrap();
        // Serial counts every commit, including commits made while unfocused.
        if matches!(request, zwp_text_input_v3::Request::Commit) {
            data.serial = data.serial.wrapping_add(1);
        }
        if !state.text_input.focused_client(&input.id()) {
            return;
        }
        match request {
            zwp_text_input_v3::Request::Enable => {
                data.pending_enabled = Some(true);
                data.pending_reset = true;
                data.pending_purpose = None;
                data.pending_cursor = None;
            }
            zwp_text_input_v3::Request::Disable => data.pending_enabled = Some(false),
            zwp_text_input_v3::Request::SetContentType {
                purpose: WEnum::Value(purpose),
                ..
            } => {
                data.pending_purpose = Some(purpose as u32);
            }
            zwp_text_input_v3::Request::SetCursorRectangle {
                x,
                y,
                width,
                height,
            } => {
                data.pending_cursor = Some([x, y, width.max(0), height.max(0)]);
            }
            zwp_text_input_v3::Request::Commit => {
                data.commit();
                state.text_input.dirty = true;
                // Acknowledge the surrounding-text/cursor updates too: Chromium
                // holds subsequent state changes until this exact serial arrives.
                input.done(data.serial);
            }
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _: ClientId, resource: &ZwpTextInputV3, _: &Mutex<InputState>) {
        state.text_input.instances.retain(|i| i != resource);
        state.text_input.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::InputState;

    #[test]
    fn editor_attributes_are_double_buffered_and_disabled_state_is_empty() {
        let mut state = InputState {
            pending_enabled: Some(true),
            pending_purpose: Some(6),
            pending_cursor: Some([20, 30, 1, 18]),
            ..Default::default()
        };
        assert!(!state.enabled);
        assert_eq!(state.purpose, 0);
        assert_eq!(state.cursor, None);
        state.commit();
        assert!(state.enabled);
        assert_eq!(state.purpose, 6);
        assert_eq!(state.cursor, Some([20, 30, 1, 18]));
        state.pending_enabled = Some(false);
        state.commit();
        assert!(!state.enabled);
        assert_eq!(state.purpose, 0);
        assert_eq!(state.cursor, None);
    }
}
