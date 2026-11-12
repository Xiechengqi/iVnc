//! Optional Wayland protocols used by native Chromium/Electron clients.
use super::Compositor;
use smithay::{
    delegate_cursor_shape, delegate_fractional_scale, delegate_pointer_constraints,
    delegate_relative_pointer, delegate_viewporter, delegate_xdg_activation, delegate_xdg_foreign,
    input::{pointer::PointerHandle, Seat},
    reexports::wayland_server::{protocol::wl_surface::WlSurface, Resource},
    utils::{Logical, Point},
    wayland::{
        compositor::with_states,
        fractional_scale::{with_fractional_scale, FractionalScaleHandler},
        pointer_constraints::{
            with_pointer_constraint, PointerConstraint, PointerConstraintsHandler,
        },
        tablet_manager::TabletSeatHandler,
        xdg_activation::{
            XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
        },
        xdg_foreign::{XdgForeignHandler, XdgForeignState},
    },
};
use std::time::Duration;

impl TabletSeatHandler for Compositor {}
delegate_cursor_shape!(Compositor);
delegate_viewporter!(Compositor);
delegate_relative_pointer!(Compositor);
impl XdgForeignHandler for Compositor {
    fn xdg_foreign_state(&mut self) -> &mut XdgForeignState {
        &mut self.xdg_foreign
    }
}
delegate_xdg_foreign!(Compositor);

impl FractionalScaleHandler for Compositor {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        // The video/output dimensions already represent physical pixels. Client
        // browser DPR must not be applied a second time to the Wayland output.
        with_states(&surface, |states| {
            with_fractional_scale(states, |scale| scale.set_preferred_scale(1.0))
        });
    }
}
delegate_fractional_scale!(Compositor);

impl XdgActivationHandler for Compositor {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.activation
    }

    fn token_created(&mut self, _: XdgActivationToken, data: XdgActivationTokenData) -> bool {
        self.activation
            .retain_tokens(|_, t| t.timestamp.elapsed() < Duration::from_secs(5));
        let keyboard = self.seat.get_keyboard().unwrap();
        let Some(focus) = keyboard.current_focus() else {
            return false;
        };
        let Some(client) = data.client_id else {
            return false;
        };
        if focus.client().is_none_or(|c| c.id() != client) {
            return false;
        }
        if data
            .surface
            .as_ref()
            .is_some_and(|s| !s.is_alive() || !s.id().same_client_as(&focus.id()))
        {
            return false;
        }
        let recent_input = self.last_user_serial.as_ref().filter(|(_, time, surface)| {
            time.elapsed() < Duration::from_secs(5) && surface.id().same_client_as(&focus.id())
        });
        // The protocol makes set_serial optional; Chromium/Electron frequently
        // supplies only set_surface. Permit that active client after recent
        // input, while still refusing background clients and stale requests.
        match data.serial {
            Some((serial, seat)) => {
                Seat::<Self>::from_resource(&seat).as_ref() == Some(&self.seat)
                    && (keyboard.last_enter() == Some(serial)
                        || recent_input.is_some_and(|(last, _, _)| *last == serial))
            }
            None => recent_input.is_some(),
        }
    }

    fn request_activation(
        &mut self,
        token: XdgActivationToken,
        data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        // Tokens are short lived and single use. A stale request cannot steal
        // focus after the user has switched to another application.
        self.activation.remove_token(&token);
        if data.timestamp.elapsed() >= Duration::from_secs(5) {
            return;
        }
        let Some(focus) = self.seat.get_keyboard().and_then(|kb| kb.current_focus()) else {
            return;
        };
        if data.client_id.as_ref() != focus.client().as_ref().map(|c| c.id()).as_ref() {
            return;
        }
        self.activate_window(&surface);
    }
}
delegate_xdg_activation!(Compositor);

impl Compositor {
    pub fn pointer_locked(&self) -> bool {
        let pointer = self.seat.get_pointer().unwrap();
        pointer.current_focus().is_some_and(|surface| {
            with_pointer_constraint(&surface, &pointer, |c| {
                c.is_some_and(|c| c.is_active() && matches!(&*c, PointerConstraint::Locked(_)))
            })
        })
    }

    pub fn activate_pointer_constraint(&mut self) {
        let pointer = self.seat.get_pointer().unwrap();
        let Some(surface) = pointer.current_focus() else {
            return;
        };
        let Some((under, origin)) = self.surface_under(pointer.current_location()) else {
            return;
        };
        if under != surface {
            return;
        }
        let keyboard_focus = self.seat.get_keyboard().and_then(|k| k.current_focus());
        if keyboard_focus
            .as_ref()
            .is_none_or(|s| !s.id().same_client_as(&surface.id()))
        {
            return;
        }
        let local = (pointer.current_location() - origin).to_i32_floor();
        with_pointer_constraint(&surface, &pointer, |constraint| {
            if let Some(constraint) = constraint {
                if constraint.region().is_none_or(|r| r.contains(local)) {
                    constraint.activate();
                }
            }
        });
    }

    /// Apply active locks/confinement in surface coordinates. Relative motion
    /// is delivered separately, including deltas beyond the output boundary.
    pub fn constrained_pointer_position(&self, next: Point<f64, Logical>) -> Point<f64, Logical> {
        let pointer = self.seat.get_pointer().unwrap();
        let old = pointer.current_location();
        let Some(surface) = pointer.current_focus() else {
            return next;
        };
        let Some((under, origin)) = self.surface_under(old) else {
            return next;
        };
        if under != surface {
            return next;
        }
        with_pointer_constraint(&surface, &pointer, |constraint| {
            let Some(c) = constraint.filter(|c| c.is_active()) else {
                return next;
            };
            match &*c {
                PointerConstraint::Locked(_) => old,
                PointerConstraint::Confined(_) => {
                    let same_surface = self.surface_under(next).is_some_and(|(s, _)| s == surface);
                    let in_region = c
                        .region()
                        .is_none_or(|r| r.contains((next - origin).to_i32_floor()));
                    if same_surface && in_region {
                        next
                    } else {
                        old
                    }
                }
            }
        })
    }

    pub fn release_pointer_constraint(&self) {
        let pointer = self.seat.get_pointer().unwrap();
        if let Some(surface) = pointer.current_focus() {
            with_pointer_constraint(&surface, &pointer, |c| {
                if let Some(c) = c {
                    c.deactivate();
                }
            });
        }
    }
}

impl PointerConstraintsHandler for Compositor {
    fn new_constraint(&mut self, _: &WlSurface, _: &PointerHandle<Self>) {
        self.activate_pointer_constraint();
    }
    // The hint is optional for compositors. Keeping the last known location
    // avoids warping the viewer's local hardware cursor on unlock.
    fn cursor_position_hint(
        &mut self,
        _: &WlSurface,
        _: &PointerHandle<Self>,
        _: Point<f64, Logical>,
    ) {
    }
}
delegate_pointer_constraints!(Compositor);
