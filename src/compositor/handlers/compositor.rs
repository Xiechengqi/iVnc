//! Compositor and SHM buffer handlers

use crate::compositor::{grabs::resize_grab, state::ClientState, Compositor};
use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    delegate_compositor, delegate_shm,
    reexports::wayland_server::{
        protocol::{wl_buffer, wl_surface::WlSurface},
        Client, Resource,
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            get_parent, is_sync_subsurface, CompositorClientState, CompositorHandler,
            CompositorState,
        },
        shm::{ShmHandler, ShmState},
    },
};

use super::xdg_shell;

impl CompositorHandler for Compositor {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if self.cursor_status
            == smithay::input::pointer::CursorImageStatus::Surface(surface.clone())
        {
            self.cursor_dirty = true;
        }
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self.window_for_surface(&root) {
                window.on_commit();
            }
        };

        xdg_shell::handle_commit(
            &mut self.popups,
            &self.space,
            surface,
            &mut self.taskbar_dirty,
        );
        resize_grab::handle_commit(&mut self.space, surface);

        let id = surface.id();
        if self.dialog_surfaces.contains(&id)
            && !self.placed_dialogs.contains(&id)
            && !self.minimized_windows.contains_key(&id)
        {
            let window = self.window_for_surface(surface);
            if let Some(window) = window {
                let geometry = window.geometry();
                if geometry.size.w > 0 && geometry.size.h > 0 {
                    let parent = window.toplevel().unwrap().parent();
                    let parent_geo = parent
                        .as_ref()
                        .and_then(|s| self.window_for_surface(s))
                        .and_then(|w| self.space.element_geometry(&w));
                    let area = parent_geo.or_else(|| {
                        self.space
                            .outputs()
                            .next()
                            .and_then(|o| self.space.output_geometry(o))
                    });
                    if let Some(area) = area {
                        let x = area.loc.x + (area.size.w - geometry.size.w).max(0) / 2;
                        let y = area.loc.y + (area.size.h - geometry.size.h).max(0) / 2;
                        self.space.map_element(window, (x, y), false);
                        self.placed_dialogs.insert(id);
                    }
                }
            }
        }

        self.needs_redraw = true;
    }
}

impl BufferHandler for Compositor {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for Compositor {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

delegate_compositor!(Compositor);
delegate_shm!(Compositor);
