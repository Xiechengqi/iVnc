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
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    },
};
use std::sync::Mutex;

#[derive(Default, Debug)]
pub struct InputState {
    serial: u32,
    enabled: bool,
    pending_enabled: Option<bool>,
}

#[derive(Default)]
pub struct TextInputState {
    instances: Vec<ZwpTextInputV3>,
    focus: Option<WlSurface>,
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
            }
        }
        self.focus = surface.cloned();
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
            zwp_text_input_v3::Request::Enable => data.pending_enabled = Some(true),
            zwp_text_input_v3::Request::Disable => data.pending_enabled = Some(false),
            zwp_text_input_v3::Request::Commit => {
                if let Some(enabled) = data.pending_enabled.take() {
                    data.enabled = enabled;
                }
                // Acknowledge the surrounding-text/cursor updates too: Chromium
                // holds subsequent state changes until this exact serial arrives.
                input.done(data.serial);
            }
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _: ClientId, resource: &ZwpTextInputV3, _: &Mutex<InputState>) {
        state.text_input.instances.retain(|i| i != resource);
    }
}
