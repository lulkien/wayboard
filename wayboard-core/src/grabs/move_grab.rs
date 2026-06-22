//! MoveSurfaceGrab — Drag a window to reposition it.
//!
//! Activated when the user clicks a window's titlebar and drags. The client
//! sends an xdg_toplevel.move request, and we start this grab.
//!
//! During the grab:
//!   - All pointer motion goes to the grab handler instead of the client
//!   - The window follows the cursor (position = initial + delta from click)
//!   - Other events (buttons, scroll, gestures) pass through to the client
//!   - When the left button is released, the grab ends
//!
//! The coordinate math: we record the cursor position at grab-start
//! (start_data.location) and the window's position at grab-start
//! (initial_window_location). On each motion event, we compute:
//!   delta = current_position - start_data.location
//!   new_position = initial_window_location + delta

use crate::Wayboard;
use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
        GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData as PointerGrabStartData,
        MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point},
};

pub struct MoveSurfaceGrab {
    /// The pointer state at the moment the grab started — cursor position,
    /// which surface was under the cursor, and the button serial. We use
    /// this to compute the delta from grab start.
    pub start_data: PointerGrabStartData<Wayboard>,

    /// The window being moved. We hold a reference so we can call
    /// space.map_element() to update its position on every motion event.
    pub window: Window,

    /// The window's position at grab start, in logical pixels within
    /// the Space. This is the baseline — we add the cursor delta to
    /// this to get the new position.
    pub initial_window_location: Point<i32, Logical>,
}

impl PointerGrab<Wayboard> for MoveSurfaceGrab {
    /// The core of the grab: pointer motion → window repositioning.
    ///
    /// Two things happen:
    ///   1. We forward the motion to handle.motion(data, None, event).
    ///      The None means "no surface has pointer focus" — this tells
    ///      Smithay to not send wl_pointer.motion to any client.
    ///   2. We update the window position in the Space.
    fn motion(
        &mut self,
        data: &mut Wayboard,
        handle: &mut PointerInnerHandle<'_, Wayboard>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // While the grab is active, no client has pointer focus.
        // This prevents the client under the cursor from receiving
        // motion events while we're dragging.
        handle.motion(data, None, event);

        // Compute the new window position: start position + cursor delta.
        // event.location is the current absolute cursor position.
        // start_data.location is where the cursor was when the grab started.
        let delta = event.location - self.start_data.location;
        let new_location = self.initial_window_location.to_f64() + delta;

        // Move the window. The `true` parameter means "send configure
        // events to the client." This is used internally by Space to
        // notify the client of position changes (via xdg_toplevel.configure
        // with bounds). For a drag, this isn't strictly necessary since
        // the client already knows it's being moved, but it's harmless.
        data.space
            .map_element(self.window.clone(), new_location.to_i32_round(), true);
    }

    // ── PASS-THROUGH EVENTS ────────────────────────────────────────────
    //
    // For everything other than motion (and button release), we just
    // forward the event to the normal handler. This means:
    //   - Relative motion works (for gaming mice)
    //   - Button events work (for detecting button release to end the grab)
    //   - Scroll events work (if the user scrolls while dragging)
    //   - Gestures work (touchpad gestures pass through)

    fn relative_motion(
        &mut self,
        data: &mut Wayboard,
        handle: &mut PointerInnerHandle<'_, Wayboard>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    /// Handle button events — specifically, detect button release to end
    /// the grab.
    ///
    /// When the user releases the left mouse button (BTN_LEFT = 0x110),
    /// the drag is over. We call handle.unset_grab() which:
    ///   - Restores normal pointer dispatch
    ///   - Sends wl_pointer.leave to the surface that had focus before
    ///     the grab, since we cleared focus during the grab
    ///   - Sends wl_pointer.enter if the cursor is now over a surface
    fn button(
        &mut self,
        data: &mut Wayboard,
        handle: &mut PointerInnerHandle<'_, Wayboard>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);

        // BTN_LEFT = 0x110 from linux/input-event-codes.h.
        //
        // We check current_pressed() rather than the event directly
        // because the event might be a button press of a different
        // button. current_pressed() is the bitmask of all currently
        // held buttons — if BTN_LEFT is no longer set, the user
        // released the left button.
        const BTN_LEFT: u32 = 0x110;

        if !handle.current_pressed().contains(&BTN_LEFT) {
            // No more buttons are pressed — end the grab.
            // The `true` parameter tells Smithay to restore the
            // pointer focus to whatever surface is now under the cursor.
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        data: &mut Wayboard,
        handle: &mut PointerInnerHandle<'_, Wayboard>,
        details: AxisFrame,
    ) {
        handle.axis(data, details)
    }

    fn frame(&mut self, data: &mut Wayboard, handle: &mut PointerInnerHandle<'_, Wayboard>) {
        handle.frame(data);
    }

    // ── GESTURES (all pass-through) ─────────────────────────────────────
    //
    // Touchpad gestures (swipe, pinch, hold) are forwarded without
    // modification. A real compositor might intercept these for
    // workspace switching (3-finger swipe) or expose (pinch to overview).

    fn gesture_swipe_begin(
        &mut self,
        data: &mut Wayboard,
        handle: &mut PointerInnerHandle<'_, Wayboard>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event)
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut Wayboard,
        handle: &mut PointerInnerHandle<'_, Wayboard>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event)
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut Wayboard,
        handle: &mut PointerInnerHandle<'_, Wayboard>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event)
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut Wayboard,
        handle: &mut PointerInnerHandle<'_, Wayboard>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event)
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut Wayboard,
        handle: &mut PointerInnerHandle<'_, Wayboard>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event)
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut Wayboard,
        handle: &mut PointerInnerHandle<'_, Wayboard>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event)
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut Wayboard,
        handle: &mut PointerInnerHandle<'_, Wayboard>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event)
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut Wayboard,
        handle: &mut PointerInnerHandle<'_, Wayboard>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event)
    }

    fn start_data(&self) -> &PointerGrabStartData<Wayboard> {
        &self.start_data
    }

    /// Called when the grab is forcefully unset (e.g., by another grab
    /// taking priority). No cleanup needed for a move grab — the window
    /// just stays where it was last placed.
    fn unset(&mut self, _data: &mut Wayboard) {}
}
