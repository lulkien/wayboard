// XdgShellHandler — handles xdg_wm_base, xdg_toplevel, xdg_popup.
//
// Called when clients create application windows. In Block 1 this is
// a stub — new_toplevel does nothing. Real window creation and placement
// arrive in Block 3 (via Space::map_element) and Block 5 (via Shell trait).

use crate::state::Wayboard;
use smithay::{
    utils::Serial,
    wayland::shell::xdg::{
        PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
    },
};

impl XdgShellHandler for Wayboard {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    /// A client created a new toplevel window. In Block 3 we'll create
    /// a Window and map it into the Space. In Block 5 we'll delegate to
    /// the Shell trait. For now: the client waits forever (no configure
    /// event sent). This is correct for a "protocols work" milestone —
    /// the client doesn't crash, it just doesn't draw.
    fn new_toplevel(&mut self, _surface: ToplevelSurface) {}

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {}

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
    }

    fn move_request(
        &mut self,
        _surface: ToplevelSurface,
        _seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
        _serial: Serial,
    ) {
    }

    fn resize_request(
        &mut self,
        _surface: ToplevelSurface,
        _seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
        _serial: Serial,
        _edges: smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    ) {
    }

    fn grab(
        &mut self,
        _surface: PopupSurface,
        _seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
        _serial: Serial,
    ) {
    }
}
