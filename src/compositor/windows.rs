//! Window lifetime and geometry are independent of whether a window is mapped.
use super::Compositor;
use smithay::{
    desktop::Window,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel::State,
        wayland_server::{protocol::wl_surface::WlSurface, Resource},
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
};

impl Compositor {
    pub fn toplevel_surface(&self, surface: &WlSurface) -> WlSurface {
        let mut root = surface.clone();
        while let Some(parent) = smithay::wayland::compositor::get_parent(&root) {
            root = parent;
        }
        if let Some(popup) = self.popups.find_popup(&root) {
            if let Ok(parent) = smithay::desktop::find_popup_root_surface(&popup) {
                return parent;
            }
        }
        root
    }

    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.space
            .elements()
            .find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == surface))
            .cloned()
            .or_else(|| {
                self.minimized_windows
                    .get(&surface.id())
                    .map(|(window, _)| window.clone())
            })
    }

    /// Explicit xdg parents, never PID relationships, define a window family.
    pub fn root_window_surface(&self, surface: &WlSurface) -> WlSurface {
        let mut root = self.toplevel_surface(surface);
        for _ in 0..32 {
            let parent = self
                .window_for_surface(&root)
                .and_then(|w| w.toplevel().unwrap().parent());
            match parent {
                Some(parent) if parent != root => root = parent,
                _ => break,
            }
        }
        root
    }

    pub fn activate_window(&mut self, surface: &WlSurface) {
        let root = self.root_window_surface(surface);
        let restore: Vec<_> = self
            .minimized_window_order
            .iter()
            .filter(|id| {
                let s = self
                    .minimized_windows
                    .get(*id)
                    .unwrap()
                    .0
                    .toplevel()
                    .unwrap()
                    .wl_surface();
                self.root_window_surface(s) == root
            })
            .cloned()
            .collect();
        for id in restore {
            if let Some((window, location)) = self.minimized_windows.remove(&id) {
                let location = self.reachable_window_location(&window, location);
                self.space.map_element(window, location, false);
            }
        }
        self.minimized_window_order
            .retain(|id| self.minimized_windows.contains_key(id));
        let Some(requested) = self.window_for_surface(surface) else {
            return;
        };
        // Raise every member, parents before children. Stable ordering retains
        // sibling stacking both across activation and a minimize/restore cycle.
        let mut family: Vec<_> = self
            .space
            .elements()
            .filter(|w| self.root_window_surface(w.toplevel().unwrap().wl_surface()) == root)
            .cloned()
            .collect();
        family.sort_by_key(|window| {
            let mut depth = 0;
            let mut surface = window.toplevel().unwrap().wl_surface().clone();
            while depth < 32 {
                let Some(parent) = self
                    .window_for_surface(&surface)
                    .and_then(|w| w.toplevel().unwrap().parent())
                else {
                    break;
                };
                surface = parent;
                depth += 1;
            }
            depth
        });
        let mut window = requested;
        for member in family {
            self.space.raise_element(&member, false);
            let mut current = member.toplevel().unwrap().wl_surface().clone();
            for _ in 0..32 {
                if current == *surface {
                    window = member.clone();
                    break;
                }
                let Some(parent) = self
                    .window_for_surface(&current)
                    .and_then(|w| w.toplevel().unwrap().parent())
                else {
                    break;
                };
                current = parent;
            }
        }
        self.space.raise_element(&window, true);
        let keyboard = self.seat.get_keyboard().unwrap();
        keyboard.set_focus(
            self,
            Some(window.toplevel().unwrap().wl_surface().clone()),
            SERIAL_COUNTER.next_serial(),
        );
        self.taskbar_dirty = true;
        self.needs_redraw = true;
    }

    pub fn minimize_window(&mut self, surface: &WlSurface) {
        let root = self.root_window_surface(surface);
        let family: Vec<_> = self
            .space
            .elements()
            .filter(|w| self.root_window_surface(w.toplevel().unwrap().wl_surface()) == root)
            .cloned()
            .collect();
        for window in family {
            let location = self.space.element_location(&window).unwrap_or_default();
            let id = window.toplevel().unwrap().wl_surface().id();
            self.space.unmap_elem(&window);
            self.minimized_window_order.push(id.clone());
            self.minimized_windows.insert(id, (window, location));
        }
        let keyboard = self.seat.get_keyboard().unwrap();
        if keyboard
            .current_focus()
            .is_some_and(|s| self.root_window_surface(&s) == root)
        {
            let next = self
                .space
                .elements()
                .next_back()
                .map(|w| w.toplevel().unwrap().wl_surface().clone());
            keyboard.set_focus(self, next, SERIAL_COUNTER.next_serial());
        }
        self.taskbar_dirty = true;
        self.needs_redraw = true;
    }

    pub fn save_normal_geometry(&mut self, surface: &WlSurface) {
        if self.normal_geometry.contains_key(&surface.id()) {
            return;
        }
        if let Some(window) = self.window_for_surface(surface) {
            let special = window.toplevel().unwrap().with_pending_state(|s| {
                s.states.contains(State::Maximized) || s.states.contains(State::Fullscreen)
            });
            let size = window.geometry().size;
            if !special && size.w > 0 && size.h > 0 {
                let location = self
                    .space
                    .element_location(&window)
                    .or_else(|| {
                        self.minimized_windows
                            .get(&surface.id())
                            .map(|(_, loc)| *loc)
                    })
                    .unwrap_or_default();
                self.normal_geometry
                    .insert(surface.id(), Rectangle::new(location, size));
            }
        }
    }

    /// Leaving fullscreen restores maximization, if it was present on entry.
    /// A pending unmaximize during fullscreen only clears that saved state.
    pub fn restore_window_geometry(&mut self, surface: &WlSurface, fullscreen: bool) {
        let Some(window) = self.window_for_surface(surface) else {
            return;
        };
        let toplevel = window.toplevel().unwrap();
        let output = self
            .space
            .outputs()
            .next()
            .and_then(|o| self.space.output_geometry(o));
        let saved = self.normal_geometry.get(&surface.id()).copied();
        let special = toplevel.with_pending_state(|state| {
            state.states.unset(if fullscreen {
                State::Fullscreen
            } else {
                State::Maximized
            });
            let special =
                state.states.contains(State::Maximized) || state.states.contains(State::Fullscreen);
            state.size = if special {
                output.map(|g| g.size)
            } else {
                saved.map(|g| g.size)
            };
            special
        });
        toplevel.send_pending_configure();
        if !special {
            self.normal_geometry.remove(&surface.id());
        }
        // A state request must not silently unminimize the application.
        if self.space.element_location(&window).is_some() {
            if let Some(geometry) = if special { output } else { saved } {
                let location = self.reachable_window_location(&window, geometry.loc);
                self.space.map_element(window, location, false);
            }
        }
        self.needs_redraw = true;
    }

    fn reachable_window_location(
        &self,
        window: &Window,
        location: Point<i32, Logical>,
    ) -> Point<i32, Logical> {
        let Some(area) = self
            .space
            .outputs()
            .next()
            .and_then(|o| self.space.output_geometry(o))
        else {
            return location;
        };
        let size = window.geometry().size;
        (
            location
                .x
                .clamp(area.loc.x, area.loc.x + (area.size.w - size.w).max(0)),
            location
                .y
                .clamp(area.loc.y, area.loc.y + (area.size.h - size.h).max(0)),
        )
            .into()
    }

    pub fn resize_windows(&mut self, width: u32, height: u32) {
        let size = (width as i32, height as i32).into();
        for window in self
            .space
            .elements()
            .chain(self.minimized_windows.values().map(|(w, _)| w))
        {
            let toplevel = window.toplevel().unwrap();
            toplevel.with_pending_state(|state| {
                if state.states.contains(State::Maximized)
                    || state.states.contains(State::Fullscreen)
                {
                    state.size = Some(size);
                }
            });
            if toplevel.is_initial_configure_sent() {
                toplevel.send_pending_configure();
            }
        }
        // Keep floating/dialog windows reachable after a viewer shrinks. Their
        // preferred sizes remain the application's decision.
        let reposition: Vec<_> = self
            .space
            .elements()
            .filter_map(|window| {
                let geometry = self.space.element_geometry(window)?;
                let location = self.space.element_location(window)?;
                let x = geometry
                    .loc
                    .x
                    .clamp(0, (width as i32 - geometry.size.w).max(0));
                let y = geometry
                    .loc
                    .y
                    .clamp(0, (height as i32 - geometry.size.h).max(0));
                Some((
                    window.clone(),
                    location
                        + smithay::utils::Point::from((x - geometry.loc.x, y - geometry.loc.y)),
                ))
            })
            .collect();
        for (window, location) in reposition {
            self.space.map_element(window, location, false);
        }
    }
}
