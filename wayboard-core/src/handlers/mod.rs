// Protocol handler implementations for Wayboard.
//
// These handle the Wayland protocol wire format. Smithay parses
// raw bytes into typed messages, then calls these trait methods.
//
// In Block 1, most are stubs — they just return state references.
// Real logic arrives in Blocks 3-7.

mod compositor;
mod xdg_shell;

use crate::state::Wayboard;
use smithay::{
    input::{
        Seat, SeatHandler, SeatState,
        dnd::{DnDGrab, DndGrabHandler, GrabType, Source},
        pointer::Focus,
    },
    reexports::wayland_server::{Resource, protocol::wl_surface::WlSurface},
    utils::Serial,
    wayland::{
        output::OutputHandler,
        selection::{
            SelectionHandler,
            data_device::{
                DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler, set_data_device_focus,
            },
        },
    },
};

// ── Seat ──────────────────────────────────────────────────────────────
//
// SeatHandler manages input device focus (keyboard, pointer, touch).
// The focus types are WlSurface — "which surface receives input."

impl SeatHandler for Wayboard {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Wayboard> {
        &mut self.seat_state
    }

    /// Called when the pointer cursor image changes (e.g. client sets a cursor).
    /// We ignore it for now — no software cursor rendering in Block 1.
    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        _image: smithay::input::pointer::CursorImageStatus,
    ) {
    }

    /// Called when keyboard focus changes. We synchronize the data device
    /// focus (clipboard) — the focused client becomes the selection owner.
    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let dh = &self.display_handle;
        let client = focused.and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client);
    }
}

// ── Data Device (clipboard + Drag-and-Drop) ─────────────────────────

impl SelectionHandler for Wayboard {
    type SelectionUserData = ();
}

impl DataDeviceHandler for Wayboard {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl DndGrabHandler for Wayboard {}

impl WaylandDndGrabHandler for Wayboard {
    /// A client initiated a drag-and-drop. Start a pointer grab to
    /// track the drag. Touch-based DnD is cancelled (unsupported for now).
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

// ── Output ────────────────────────────────────────────────────────────

impl OutputHandler for Wayboard {}

// ── THE ROUTING TABLE ─────────────────────────────────────────────────
//
// This macro is CRITICAL. Without it, Smithay doesn't know which
// handler trait to call for which protocol opcode. It generates
// Dispatch impls that map wire messages → your trait methods.
//
// You call it exactly once, in exactly this file, after all your
// handler impls are defined.

smithay::delegate_dispatch2!(Wayboard);
