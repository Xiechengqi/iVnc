//! XDG Shell handler for toplevel windows and popups

use smithay::{
    delegate_xdg_shell,
    desktop::{
        find_popup_root_surface, get_popup_toplevel_coords, PopupKeyboardGrab, PopupKind,
        PopupManager, PopupPointerGrab, PopupUngrabStrategy, Space, Window,
    },
    input::{
        pointer::{Focus, GrabStartData as PointerGrabStartData},
        Seat,
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            protocol::{wl_output::WlOutput, wl_seat, wl_surface::WlSurface},
            Resource,
        },
    },
    utils::{Rectangle, Serial},
    wayland::{
        compositor::with_states,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            XdgToplevelSurfaceData,
        },
    },
};

use crate::compositor::{
    grabs::{MoveSurfaceGrab, ResizeSurfaceGrab},
    Compositor,
};

impl XdgShellHandler for Compositor {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        self.window_keys
            .insert(surface.wl_surface().id(), uuid::Uuid::new_v4());
        let is_dialog = surface.parent().is_some();
        let window = Window::new_wayland_window(surface.clone());
        self.space.map_element(window, (0, 0), false);
        if is_dialog {
            self.dialog_surfaces.insert(surface.wl_surface().id());
        } else {
            // Fill the desktop without telling Electron it entered fullscreen.
            if let Some(geometry) = self
                .space
                .outputs()
                .next()
                .and_then(|o| self.space.output_geometry(o))
            {
                surface.with_pending_state(|state| {
                    state.states.set(xdg_toplevel::State::Maximized);
                    state.size = Some(geometry.size);
                });
            }
            self.window_registry.push(surface.wl_surface().clone());
        }
        // Initial configure is sent on the first commit, after set_parent,
        // min/max size and decoration requests have been processed.
        self.taskbar_dirty = true;
        self.activate_window(surface.wl_surface());
    }

    fn parent_changed(&mut self, surface: ToplevelSurface) {
        if surface.parent().is_some() {
            let proto_id = surface.wl_surface().id();
            log::info!("parent_changed: dialog detected (sid={:?})", proto_id);
            self.dialog_surfaces.insert(proto_id.clone());
            self.placed_dialogs.remove(&proto_id);

            // Remove from taskbar if it was previously registered as a main window
            let surf_id = surface.wl_surface().id();
            let was_registered = self.window_registry.iter().any(|w| w.id() == surf_id);
            if was_registered {
                self.window_registry.retain(|w| w.id() != surf_id);
                self.taskbar_dirty = true;
                log::info!("parent_changed: removed sid={:?} from taskbar", proto_id);
            }

            // Undo fullscreen: let the app use its preferred dialog size
            let decoration_mode = self.decoration_mode_for(&surface);
            surface.with_pending_state(|state| {
                state.states.unset(xdg_toplevel::State::Fullscreen);
                state.states.unset(xdg_toplevel::State::Maximized);
                state.size = None;
                state.decoration_mode = Some(decoration_mode);
            });
            if surface.is_initial_configure_sent() {
                surface.send_pending_configure();
            }
        } else if self.dialog_surfaces.remove(&surface.wl_surface().id()) {
            self.placed_dialogs.remove(&surface.wl_surface().id());
            if !self.window_registry.contains(surface.wl_surface()) {
                self.window_registry.push(surface.wl_surface().clone());
            }
            self.configure_toplevel_decoration(&surface);
            self.taskbar_dirty = true;
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            let geometry = positioner.get_geometry();
            state.geometry = geometry;
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        if surface.with_pending_state(|s| {
            s.states.contains(xdg_toplevel::State::Maximized)
                || s.states.contains(xdg_toplevel::State::Fullscreen)
        }) {
            return;
        }
        let seat = Seat::from_resource(&seat).unwrap();
        let wl_surface = surface.wl_surface();

        if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            let pointer = seat.get_pointer().unwrap();
            let Some(window) = self
                .space
                .elements()
                .find(|w| w.toplevel().unwrap().wl_surface() == wl_surface)
                .cloned()
            else {
                return;
            };
            let initial_window_location = self.space.element_location(&window).unwrap();

            let grab = MoveSurfaceGrab {
                start_data,
                window,
                initial_window_location,
            };
            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        if surface.with_pending_state(|s| {
            s.states.contains(xdg_toplevel::State::Maximized)
                || s.states.contains(xdg_toplevel::State::Fullscreen)
        }) {
            return;
        }
        let seat = Seat::from_resource(&seat).unwrap();
        let wl_surface = surface.wl_surface();

        if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            let pointer = seat.get_pointer().unwrap();
            let Some(window) = self
                .space
                .elements()
                .find(|w| w.toplevel().unwrap().wl_surface() == wl_surface)
                .cloned()
            else {
                return;
            };
            let initial_window_location = self.space.element_location(&window).unwrap();
            let initial_window_size = window.geometry().size;

            surface.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Resizing);
            });
            if surface.is_initial_configure_sent() {
                surface.send_pending_configure();
            }

            let grab = ResizeSurfaceGrab::start(
                start_data,
                window,
                edges.into(),
                Rectangle::new(initial_window_location, initial_window_size),
            );
            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        if self.dialog_surfaces.contains(&surface.wl_surface().id()) {
            return;
        }
        self.save_normal_geometry(surface.wl_surface());
        let output_geometry = self
            .space
            .outputs()
            .next()
            .and_then(|o| self.space.output_geometry(o));
        if let Some(geometry) = output_geometry {
            surface.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Maximized);
                state.size = Some(geometry.size);
            });
            if surface.is_initial_configure_sent() {
                surface.send_pending_configure();
            }
            if let Some(window) = self.window_for_surface(surface.wl_surface()) {
                if self.space.element_location(&window).is_some() {
                    self.space.map_element(window, geometry.loc, false);
                }
            }
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        self.restore_window_geometry(surface.wl_surface(), false);
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, _output: Option<WlOutput>) {
        self.save_normal_geometry(surface.wl_surface());
        let output_geometry = self
            .space
            .outputs()
            .next()
            .and_then(|o| self.space.output_geometry(o));
        if let Some(geometry) = output_geometry {
            surface.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Fullscreen);
                state.size = Some(geometry.size);
            });
            if surface.is_initial_configure_sent() {
                surface.send_pending_configure();
            }
            if let Some(window) = self.window_for_surface(surface.wl_surface()) {
                if self.space.element_location(&window).is_some() {
                    self.space.map_element(window, geometry.loc, false);
                }
            }
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        self.restore_window_geometry(surface.wl_surface(), true);
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        self.minimize_window(surface.wl_surface());
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let id = surface.wl_surface().id();
        // Direct xdg parents are stored as wl_surface references by Smithay.
        // Destroying a toplevel does not clear those references automatically.
        // Per xdg-shell, surviving children inherit the destroyed parent's
        // parent; an orphaned child must regain its own toolbar entry.
        let children: Vec<_> = self
            .space
            .elements()
            .chain(self.minimized_windows.values().map(|(w, _)| w))
            .filter_map(|w| {
                w.toplevel()
                    .filter(|t| t.parent().as_ref() == Some(surface.wl_surface()))
                    .cloned()
            })
            .collect();
        for child in children {
            with_states(child.wl_surface(), |states| {
                states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .parent = surface.parent();
            });
            self.parent_changed(child);
        }
        self.dialog_surfaces.remove(&id);
        self.placed_dialogs.remove(&id);
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.space.unmap_elem(&window);
        }
        self.minimized_windows.remove(&id);
        self.minimized_window_order.retain(|item| item != &id);
        self.normal_geometry.remove(&id);
        self.window_registry.retain(|wl| wl.id() != id);
        self.window_keys.remove(&id);
        let focused = self.seat.get_keyboard().and_then(|kb| kb.current_focus());
        if focused.as_ref() == Some(surface.wl_surface()) {
            let next = self.space.elements().rev().find_map(|w| {
                let surface = w.toplevel()?.wl_surface();
                (surface.id() != id && surface.is_alive()).then(|| surface.clone())
            });
            let keyboard = self.seat.get_keyboard().unwrap();
            keyboard.set_focus(self, next, smithay::utils::SERIAL_COUNTER.next_serial());
        }
        self.taskbar_dirty = true;
        self.needs_redraw = true;
    }

    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let seat: Seat<Compositor> = Seat::from_resource(&seat).unwrap();
        let kind = PopupKind::Xdg(surface);

        // Find the root toplevel surface for this popup chain
        let root = find_popup_root_surface(&kind).ok().and_then(|root| {
            self.space
                .elements()
                .find(|w| w.toplevel().unwrap().wl_surface() == &root)
                .map(|w| w.toplevel().unwrap().wl_surface().clone())
        });

        let Some(root) = root else { return };

        let ret = self.popups.grab_popup(root, kind, &seat, serial);

        if let Ok(mut grab) = ret {
            if let Some(keyboard) = seat.get_keyboard() {
                if keyboard.is_grabbed()
                    && !(keyboard.has_grab(serial)
                        || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
                {
                    grab.ungrab(PopupUngrabStrategy::All);
                    return;
                }
                keyboard.set_focus(self, grab.current_grab(), serial);
                keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
            }
            if let Some(pointer) = seat.get_pointer() {
                if pointer.is_grabbed()
                    && !(pointer.has_grab(serial)
                        || pointer
                            .has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial())))
                {
                    grab.ungrab(PopupUngrabStrategy::All);
                    return;
                }
                pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
            }
        }
    }
}

delegate_xdg_shell!(Compositor);

fn check_grab(
    seat: &Seat<Compositor>,
    surface: &WlSurface,
    serial: Serial,
) -> Option<PointerGrabStartData<Compositor>> {
    let pointer = seat.get_pointer()?;
    if !pointer.has_grab(serial) {
        return None;
    }
    let start_data = pointer.grab_start_data()?;
    let (focus, _) = start_data.focus.as_ref()?;
    if !focus.id().same_client_as(&surface.id()) {
        return None;
    }
    Some(start_data)
}

/// Should be called on `WlSurface::commit`
pub fn handle_commit(
    popups: &mut PopupManager,
    space: &Space<Window>,
    surface: &WlSurface,
    taskbar_dirty: &mut bool,
) {
    if let Some(window) = space
        .elements()
        .find(|w| w.toplevel().unwrap().wl_surface() == surface)
        .cloned()
    {
        let (initial_configure_sent, identity_available) = with_states(surface, |states| {
            let data = states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .unwrap()
                .lock()
                .unwrap();
            (
                data.initial_configure_sent,
                data.title.is_some() || data.app_id.is_some(),
            )
        });
        if !initial_configure_sent {
            // Fixed-size Electron windows (including late set_parent requests)
            // cannot obey a desktop-sized configure. Read committed limits.
            let fixed = with_states(surface, |states| {
                let mut cache = states
                    .cached_state
                    .get::<smithay::wayland::shell::xdg::SurfaceCachedState>();
                let limits = cache.current();
                limits.min_size.w > 0 && limits.min_size.h > 0 && limits.min_size == limits.max_size
            });
            if fixed {
                window.toplevel().unwrap().with_pending_state(|state| {
                    if !state.states.contains(xdg_toplevel::State::Fullscreen) {
                        state.states.unset(xdg_toplevel::State::Maximized);
                        state.size = None;
                    }
                });
            }
            window.toplevel().unwrap().send_configure();
        }
        // Mark dirty on every commit that has title/app_id — the main loop
        // will deduplicate by comparing the serialized JSON.
        if identity_available {
            *taskbar_dirty = true;
        }
    }

    popups.commit(surface);
    if let Some(popup) = popups.find_popup(surface) {
        match popup {
            PopupKind::Xdg(ref xdg) => {
                if !xdg.is_initial_configure_sent() {
                    xdg.send_configure().expect("initial configure failed");
                }
            }
            PopupKind::InputMethod(ref _input_method) => {}
        }
    }
}

impl Compositor {
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            return;
        };
        let Some(window) = self
            .space
            .elements()
            .find(|w| w.toplevel().unwrap().wl_surface() == &root)
        else {
            return;
        };

        let output = self.space.outputs().next().unwrap();
        let output_geo = self.space.output_geometry(output).unwrap();
        let window_geo = self.space.element_geometry(window).unwrap();

        let mut target = output_geo;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= window_geo.loc;

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}
