//! ResizeSurfaceGrab — Drag an edge or corner to resize a window.
//!
//! More complex than move_grab because resizing involves:
//!   - Computing new width/height from the drag direction
//!   - Respecting min/max size constraints from the client
//!   - For top/left edges: adjusting the window's position as well as size
//!   - Coordinating with the client via configure events
//!   - Handling the final commit to finalize position after resize
//!
//! EDGE SEMANTICS:
//!
//!   Dragging RIGHT edge:  width += delta.x  (position unchanged)
//!   Dragging LEFT edge:   width -= delta.x, x += delta.x minus new_width
//!   Dragging BOTTOM edge: height += delta.y  (position unchanged)
//!   Dragging TOP edge:    height -= delta.y, y += delta.y minus new_height
//!
//!   For corners, both axes apply simultaneously.
//!
//! Why does resizing the left/top edge change position? Because the
//! window's position is its top-left corner in the Space. If you drag
//! the left edge rightward by 50px, the window becomes 50px narrower
//! AND its x-position must move right by 50px so the right edge stays
//! in the same place.
//!
//! STATE MACHINE:
//!
//!   Idle ──(start resize)──→ Resizing
//!   Resizing ──(button release)──→ WaitingForLastCommit
//!   WaitingForLastCommit ──(surface commit)──→ Idle
//!
//! We need the WaitingForLastCommit state because when the user releases
//! the button, the client hasn't yet drawn at the final size. We tell
//! the client the new size via configure, then wait for the client to
//! commit a buffer at that size. Only then do we adjust the window
//! position (for top/left resizes). This prevents visual glitches where
//! the window position updates before the content matches.

use crate::Wayboard;
use smithay::{
    desktop::{Space, Window},
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
        GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData as PointerGrabStartData,
        MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Point, Rectangle, Size},
    wayland::{compositor, shell::xdg::SurfaceCachedState},
};
use std::cell::RefCell;

// ─────────────────────────────────────────────────────────────────────────
// RESIZE EDGE BITFLAGS
// ─────────────────────────────────────────────────────────────────────────
//
// We define our own ResizeEdge type (matching xdg_toplevel::ResizeEdge)
// for two reasons:
//   1. Our bitflag operations (intersects, combine) are more ergonomic
//      than matching against the xdg_toplevel enum variants
//   2. We can use it directly in the ResizeSurfaceState (which is stored
//      in the surface data map — having our own type avoids coupling
//      to smithay's internal types)

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct ResizeEdge: u32 {
        /// Top edge
        const TOP          = 0b0001;
        /// Bottom edge
        const BOTTOM       = 0b0010;
        /// Left edge
        const LEFT         = 0b0100;
        /// Right edge
        const RIGHT        = 0b1000;

        // Compound edges (corners):
        const TOP_LEFT     = Self::TOP.bits() | Self::LEFT.bits();
        const BOTTOM_LEFT  = Self::BOTTOM.bits() | Self::LEFT.bits();
        const TOP_RIGHT    = Self::TOP.bits() | Self::RIGHT.bits();
        const BOTTOM_RIGHT = Self::BOTTOM.bits() | Self::RIGHT.bits();
    }
}

/// Convert from the protocol's ResizeEdge enum to our bitflags.
/// The protocol enum values happen to match our bit layout.
impl From<xdg_toplevel::ResizeEdge> for ResizeEdge {
    #[inline]
    fn from(x: xdg_toplevel::ResizeEdge) -> Self {
        Self::from_bits(x as u32).unwrap()
    }
}

pub struct ResizeSurfaceGrab {
    start_data: PointerGrabStartData<Wayboard>,
    window: Window,

    /// Which edge(s) the user is dragging
    edges: ResizeEdge,

    /// The full window rectangle (position + size) at grab start.
    /// Used as the baseline — we add the cursor delta to compute new size.
    initial_rect: Rectangle<i32, Logical>,

    /// The most recently computed window size (clamped to min/max).
    /// Kept as a separate field so we can send it on button release.
    last_window_size: Size<i32, Logical>,
}

impl ResizeSurfaceGrab {
    /// Start a resize grab.
    ///
    /// This is called from XdgShellHandler::resize_request(). We:
    ///   1. Store the initial state in the surface's data map
    ///      (ResizeSurfaceState::Resizing) so the commit handler can
    ///      access it later
    ///   2. Record the initial rectangle for delta computation
    pub fn start(
        start_data: PointerGrabStartData<Wayboard>,
        window: Window,
        edges: ResizeEdge,
        initial_window_rect: Rectangle<i32, Logical>,
    ) -> Self {
        let initial_rect = initial_window_rect;

        // Store the resize state in the surface's data map.
        // This is how handle_commit() knows we're mid-resize
        // and need to adjust the window position after the final commit.
        ResizeSurfaceState::with(window.toplevel().unwrap().wl_surface(), |state| {
            *state = ResizeSurfaceState::Resizing {
                edges,
                initial_rect,
            };
        });

        Self {
            start_data,
            window,
            edges,
            initial_rect,
            last_window_size: initial_rect.size,
        }
    }
}

impl PointerGrab<Wayboard> for ResizeSurfaceGrab {
    /// The core: pointer motion → window resize.
    ///
    /// On each motion event:
    ///   1. Compute delta from grab start
    ///   2. Calculate new width/height based on which edges are active
    ///      (LEFT and TOP edges invert the delta sign)
    ///   3. Query the client's min/max size constraints
    ///   4. Clamp to [min, max]
    ///   5. Send a configure event with the new size
    ///
    /// We do NOT update the window position here — that happens
    /// during the final commit, after the client has drawn at the
    /// new size. This avoids a visual race between position and
    /// content updates.
    fn motion(
        &mut self,
        data: &mut Wayboard,
        handle: &mut PointerInnerHandle<'_, Wayboard>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // While the grab is active, no client has pointer focus
        handle.motion(data, None, event);

        let mut delta = event.location - self.start_data.location;

        let mut new_window_width = self.initial_rect.size.w;
        let mut new_window_height = self.initial_rect.size.h;

        // Horizontal resize: apply delta to width.
        // If dragging the left edge, the window grows leftward —
        // we invert the delta sign so dragging left = wider.
        if self.edges.intersects(ResizeEdge::LEFT | ResizeEdge::RIGHT) {
            if self.edges.intersects(ResizeEdge::LEFT) {
                delta.x = -delta.x;
            }

            new_window_width = (self.initial_rect.size.w as f64 + delta.x) as i32;
        }

        // Vertical resize: apply delta to height.
        // Same logic as horizontal — if dragging top edge, invert sign.
        if self.edges.intersects(ResizeEdge::TOP | ResizeEdge::BOTTOM) {
            if self.edges.intersects(ResizeEdge::TOP) {
                delta.y = -delta.y;
            }

            new_window_height = (self.initial_rect.size.h as f64 + delta.y) as i32;
        }

        // ── Min/Max Size Constraints ──────────────────────────────────
        //
        // Clients can specify min and max dimensions for their toplevel.
        // They're stored in the SurfaceCachedState. A value of 0 for max
        // means "no maximum" (unlimited).
        //
        // We read these via compositor::with_states, which gives us
        // access to the per-surface data map.
        let (min_size, max_size) =
            compositor::with_states(self.window.toplevel().unwrap().wl_surface(), |states| {
                let mut guard = states.cached_state.get::<SurfaceCachedState>();
                let data = guard.current();
                (data.min_size, data.max_size)
            });

        let min_width = min_size.w.max(1); // minimum 1px (zero-size
        let min_height = min_size.h.max(1); // windows don't make sense)

        let max_width = if max_size.w == 0 {
            i32::MAX
        } else {
            max_size.w
        };
        let max_height = if max_size.h == 0 {
            i32::MAX
        } else {
            max_size.h
        };

        self.last_window_size = Size::from((
            new_window_width.max(min_width).min(max_width),
            new_window_height.max(min_height).min(max_height),
        ));

        // Tell the client about the new size.
        // The client will (eventually) commit a buffer at this size,
        // at which point handle_commit() will adjust the window position.
        let xdg = self.window.toplevel().unwrap();
        xdg.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Resizing);
            state.size = Some(self.last_window_size);
        });

        xdg.send_pending_configure();
    }

    fn relative_motion(
        &mut self,
        data: &mut Wayboard,
        handle: &mut PointerInnerHandle<'_, Wayboard>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    /// Button release: end the resize grab.
    ///
    /// When the user releases the left button:
    ///   1. Send final configure with the Resizing state cleared
    ///   2. Transition to WaitingForLastCommit state
    ///
    /// We don't finalize the window position yet — we wait for the
    /// client to commit a buffer at the final size. See handle_commit().
    fn button(
        &mut self,
        data: &mut Wayboard,
        handle: &mut PointerInnerHandle<'_, Wayboard>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);

        const BTN_LEFT: u32 = 0x110;

        if !handle.current_pressed().contains(&BTN_LEFT) {
            // End the grab — restore normal pointer dispatch
            handle.unset_grab(self, data, event.serial, event.time, true);

            let xdg = self.window.toplevel().unwrap();
            xdg.with_pending_state(|state| {
                state.states.unset(xdg_toplevel::State::Resizing);
                state.size = Some(self.last_window_size);
            });

            xdg.send_pending_configure();

            // Transition to WaitingForLastCommit.
            // The next time this surface commits, handle_commit() will
            // finalize the window position and transition back to Idle.
            ResizeSurfaceState::with(xdg.wl_surface(), |state| {
                *state = ResizeSurfaceState::WaitingForLastCommit {
                    edges: self.edges,
                    initial_rect: self.initial_rect,
                };
            });
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

    fn unset(&mut self, _data: &mut Wayboard) {}
}

// ─────────────────────────────────────────────────────────────────────────
// RESIZE SURFACE STATE (stored in the surface data map)
// ─────────────────────────────────────────────────────────────────────────

/// Per-surface resize state.
///
/// Stored inside the WlSurface's DataMap via compositor::with_states.
/// This is how the commit handler (called asynchronously after the grab
/// ends) knows whether a resize is in progress and what to do.
///
/// ALTERNATIVE APPROACH: Store this in the ResizeSurfaceGrab and pass
/// it to the commit handler somehow. But the grab gets destroyed when
/// unset, and there's no direct channel from PointerGrab::unset to
/// CompositorHandler::commit. The surface data map is the natural
/// shared state — it persists for the surface's lifetime and is
/// accessible from both the grab and the commit handler.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
enum ResizeSurfaceState {
    /// No resize in progress
    #[default]
    Idle,

    /// Actively resizing — motion events are updating the size.
    /// The compositor is sending configure events; the client
    /// hasn't committed a buffer at the current size yet.
    Resizing {
        edges: ResizeEdge,
        /// The initial window size and location (for computing
        /// position adjustment on final commit).
        initial_rect: Rectangle<i32, Logical>,
    },

    /// Resize drag ended, waiting for the client's buffer commit
    /// at the final size. On commit: adjust position (for top/left
    /// resizes) and transition back to Idle.
    WaitingForLastCommit {
        edges: ResizeEdge,
        /// The initial window size and location.
        initial_rect: Rectangle<i32, Logical>,
    },
}

impl ResizeSurfaceState {
    /// Access or initialize the ResizeSurfaceState stored in a surface.
    ///
    /// This uses RefCell because Smithay's DataMap requires thread-local
    /// interior mutability — compositor operations are single-threaded,
    /// but Rust's type system doesn't know that the event loop enforces
    /// this. RefCell gives us runtime borrow checking that can't fail
    /// because we're always the only borrower.
    fn with<F, T>(surface: &WlSurface, cb: F) -> T
    where
        F: FnOnce(&mut Self) -> T,
    {
        compositor::with_states(surface, |states| {
            states.data_map.insert_if_missing(RefCell::<Self>::default);
            let state = states.data_map.get::<RefCell<Self>>().unwrap();

            cb(&mut state.borrow_mut())
        })
    }

    /// Consume the resize state on commit.
    ///
    /// Returns Some((edges, initial_rect)) if a resize was active,
    /// so the commit handler can adjust the window position.
    /// Returns None if idle.
    ///
    /// Transition:
    ///   Resizing → returns data, stays Resizing (more commits may come)
    ///   WaitingForLastCommit → returns data, transitions to Idle
    ///   Idle → returns None
    fn commit(&mut self) -> Option<(ResizeEdge, Rectangle<i32, Logical>)> {
        match *self {
            Self::Resizing {
                edges,
                initial_rect,
            } => Some((edges, initial_rect)),
            Self::WaitingForLastCommit {
                edges,
                initial_rect,
            } => {
                // The resize is done — go back to idle.
                *self = Self::Idle;
                Some((edges, initial_rect))
            }
            Self::Idle => None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// COMMIT-TIME POSITION ADJUSTMENT
// ─────────────────────────────────────────────────────────────────────────

/// Called from CompositorHandler::commit() after the client commits a
/// buffer during or after a resize.
///
/// This is where the window position finally gets updated for top/left
/// edge resizes. We delayed this until the client committed a buffer
/// at the new size so the position change and content update are atomic
/// from the user's perspective — no flash of misaligned content.
///
/// The math: for a left-edge resize, the window got narrower. Its
/// x-position needs to move right by the same amount so the right edge
/// stays fixed. For a top-edge resize, y-position moves down.
///
///   new_x = initial_rect.x + (initial_width - new_width)
///   new_y = initial_rect.y + (initial_height - new_height)
pub fn handle_commit(space: &mut Space<Window>, surface: &WlSurface) -> Option<()> {
    let window = space
        .elements()
        .find(|w| w.toplevel().unwrap().wl_surface() == surface)
        .cloned()?;

    let mut window_loc = space.element_location(&window)?;
    let geometry = window.geometry();

    let new_loc: Point<Option<i32>, Logical> = ResizeSurfaceState::with(surface, |state| {
        state
            .commit()
            .and_then(|(edges, initial_rect)| {
                // Only adjust position if a top or left edge is involved.
                // Bottom and right edges don't affect position.
                edges.intersects(ResizeEdge::TOP_LEFT).then(|| {
                    let new_x = edges
                        .intersects(ResizeEdge::LEFT)
                        .then_some(initial_rect.loc.x + (initial_rect.size.w - geometry.size.w));

                    let new_y = edges
                        .intersects(ResizeEdge::TOP)
                        .then_some(initial_rect.loc.y + (initial_rect.size.h - geometry.size.h));

                    (new_x, new_y).into()
                })
            })
            .unwrap_or_default()
    });

    if let Some(new_x) = new_loc.x {
        window_loc.x = new_x;
    }
    if let Some(new_y) = new_loc.y {
        window_loc.y = new_y;
    }

    if new_loc.x.is_some() || new_loc.y.is_some() {
        // Move the window to its final position.
        // `false` means don't send configure — we already sent the
        // final configure during button release.
        space.map_element(window, window_loc, false);
    }

    Some(())
}
