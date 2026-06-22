//! Pointer grabs for interactive window manipulation (move and resize).
//!
//! These are mechanism, not policy. They work the same regardless of
//! which shell is active.

pub mod move_grab;
pub use move_grab::MoveSurfaceGrab;

pub mod resize_grab;
pub use resize_grab::ResizeSurfaceGrab;
