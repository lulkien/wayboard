// wayboard-core — the compositor engine.
//
// Exports the compositor state and protocol handler implementations.
// Shell plugins and the frontend depend on this crate.

pub mod handlers;
pub mod state;

pub use state::Wayboard;
