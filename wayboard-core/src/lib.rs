// wayboard-core — the compositor engine.
//
// Exports the Shell trait, NullShell, Wayboard state, and all protocol
// handlers. Shell plugins (like shell-default) depend on this crate.

pub mod grabs;
pub mod handlers;
pub mod input;
pub mod shell;
pub mod state;

pub use shell::{NullShell, Shell, ShellContext};
pub use state::Wayboard;
