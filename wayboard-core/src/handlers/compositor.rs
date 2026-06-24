// CompositorHandler — handles wl_surface creation and commits.
//
// Every surface commit (when a client publishes a new frame) goes
// through here. In Block 1, we only handle buffer lifecycle. Window
// commit logic (initial configure) arrives in Block 3.

use crate::state::{ClientState, Wayboard};
use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    reexports::wayland_server::{
        Client,
        protocol::{wl_buffer, wl_surface::WlSurface},
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        shm::{ShmHandler, ShmState},
    },
};

impl CompositorHandler for Wayboard {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    /// Return the per-client CompositorClientState. Smithay needs this
    /// to track which surfaces belong to which client.
    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    /// Called when a client calls wl_surface.commit(). The client says
    /// "I'm done — show this frame."
    ///
    /// on_commit_buffer_handler: processes buffer attach/detach, damage.
    /// In Block 3 we add: initial configure for new windows, popup commits,
    /// and resize grab finalization.
    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
    }
}

impl BufferHandler for Wayboard {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for Wayboard {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}
