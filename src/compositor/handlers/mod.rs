//! Wayland protocol handlers for the compositor

pub mod compositor;
pub mod xdg_shell;

use crate::compositor::Compositor;

use std::os::fd::OwnedFd;

use smithay::input::dnd::{DnDGrab, DndGrabHandler, GrabType, Source};
use smithay::input::pointer::Focus;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Resource;
use smithay::utils::Serial;
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::data_device::{
    set_data_device_focus,
    DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler,
};
use smithay::wayland::selection::{SelectionHandler, SelectionSource, SelectionTarget};
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::wayland::shell::xdg::ToplevelSurface;
use smithay::{delegate_data_device, delegate_output, delegate_seat, delegate_xdg_decoration};

impl SeatHandler for Compositor {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Compositor> {
        &mut self.seat_state
    }

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        image: smithay::input::pointer::CursorImageStatus,
    ) {
        self.cursor_status = image;
        self.cursor_dirty = true;
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        if let Some(pointer_surface) = self.seat.get_pointer().and_then(|p| p.current_focus()) {
            if focused.is_none_or(|f| {
                self.root_window_surface(f) != self.root_window_surface(&pointer_surface)
            }) {
                self.release_pointer_constraint();
            }
        }
        self.taskbar_dirty = true;
        let dh = &self.display_handle;
        let client = focused.and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client);

        self.text_input.set_focus(focused);

        // Set/unset xdg_toplevel Activated state so clients (e.g. Chromium)
        // know the window has keyboard focus and should process key events.
        for window in self
            .space
            .elements()
            .chain(self.minimized_windows.values().map(|(w, _)| w))
        {
            let toplevel = window.toplevel().unwrap();
            let is_focused = focused
                .map(|f| self.toplevel_surface(f) == *toplevel.wl_surface())
                .unwrap_or(false);
            toplevel.with_pending_state(|state| {
                if is_focused {
                    state.states.set(xdg_toplevel::State::Activated);
                } else {
                    state.states.unset(xdg_toplevel::State::Activated);
                }
            });
            if toplevel.is_initial_configure_sent() {
                toplevel.send_pending_configure();
            }
        }
    }
}

delegate_seat!(Compositor);

impl SelectionHandler for Compositor {
    type SelectionUserData = crate::compositor::clipboard::Snapshot;

    fn new_selection(
        &mut self,
        ty: SelectionTarget,
        source: Option<SelectionSource>,
        _seat: Seat<Self>,
    ) {
        log::info!(
            "new_selection called: ty={:?}, has_source={}",
            ty,
            source.is_some()
        );
        if ty != SelectionTarget::Clipboard {
            return;
        }
        self.clipboard_revision += 1;
        self.clipboard_pending_mimes.clear();
        self.clipboard_readback = None;
        let source = match source {
            Some(s) => s,
            None => {
                log::debug!("new_selection: no source (compositor-owned selection), skipping");
                return;
            }
        };

        let mime_types = source.mime_types();
        log::info!("new_selection: mime_types={:?}", mime_types);
        if let Some(mime) = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"]
            .into_iter()
            .find(|m| mime_types.iter().any(|t| t == m))
        {
            self.clipboard_pending_mimes.push(mime.to_owned());
        }
        if self.binary_clipboard_enabled {
            for mime in ["text/html", "image/png"] {
                if mime_types.iter().any(|t| t == mime) {
                    self.clipboard_pending_mimes.push(mime.into());
                }
            }
        }
    }

    fn send_selection(
        &mut self,
        _ty: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
        _seat: Seat<Self>,
        user_data: &Self::SelectionUserData,
    ) {
        if let Some(snapshot) = user_data.for_mime(&mime_type) {
            crate::compositor::clipboard::send_snapshot(fd, snapshot);
        }
    }
}

impl DataDeviceHandler for Compositor {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl DndGrabHandler for Compositor {}
impl WaylandDndGrabHandler for Compositor {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        _icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        type_: GrabType,
    ) {
        match type_ {
            GrabType::Pointer => {
                let ptr = seat.get_pointer().unwrap();
                let start_data = ptr.grab_start_data().unwrap();
                let grab = DnDGrab::new_pointer(&self.display_handle, start_data, source, seat);
                ptr.set_grab(self, grab, serial, Focus::Keep);
            }
            GrabType::Touch => {
                source.cancel();
            }
        }
    }
}

delegate_data_device!(Compositor);

impl OutputHandler for Compositor {}
delegate_output!(Compositor);

// Main surfaces stay borderless; dialogs need CSD for native close controls.
fn decoration_mode_for_dialog(is_dialog: bool) -> Mode {
    if is_dialog {
        Mode::ClientSide
    } else {
        Mode::ServerSide
    }
}

impl Compositor {
    pub(super) fn decoration_mode_for(&self, toplevel: &ToplevelSurface) -> Mode {
        let surface_id = toplevel.wl_surface().id();
        decoration_mode_for_dialog(self.dialog_surfaces.contains(&surface_id))
    }

    fn configure_toplevel_decoration(&self, toplevel: &ToplevelSurface) {
        let mode = self.decoration_mode_for(toplevel);
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(mode);
        });

        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }
}

impl XdgDecorationHandler for Compositor {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        self.configure_toplevel_decoration(&toplevel);
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: Mode) {
        self.configure_toplevel_decoration(&toplevel);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.configure_toplevel_decoration(&toplevel);
    }
}
delegate_xdg_decoration!(Compositor);

#[cfg(test)]
mod tests {
    use super::{decoration_mode_for_dialog, Mode};

    #[test]
    fn dialogs_use_client_side_decorations() {
        assert!(matches!(decoration_mode_for_dialog(true), Mode::ClientSide));
    }

    #[test]
    fn main_windows_keep_server_side_decorations() {
        assert!(matches!(
            decoration_mode_for_dialog(false),
            Mode::ServerSide
        ));
    }
}
