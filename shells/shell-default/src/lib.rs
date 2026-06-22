// shell-default — simple fullscreen shell.
//
// Every new window is requested to be fullscreen, covering the entire
// output. No layout, no keybindings yet — just maximum screen space.

use smithay::{
    desktop::Window,
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::Size,
};
use wayboard_core::shell::{Shell, ShellContext};

pub struct DefaultShell;

impl DefaultShell {
    pub fn new() -> Self {
        Self
    }
}

impl Shell for DefaultShell {
    fn new_window(&mut self, ctx: &mut ShellContext, window: Window) {
        // Get the output geometry to know how big the screen is.
        let output_size = ctx
            .space
            .outputs()
            .next()
            .and_then(|o| ctx.space.output_geometry(o))
            .map(|geo| geo.size)
            .unwrap_or(Size::from((800, 600)));

        // Tell the client it should be fullscreen at the output's size.
        let toplevel = window.toplevel().unwrap();
        toplevel.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Fullscreen);
            state.size = Some(output_size);
        });

        // Map the window at (0, 0) covering the entire output.
        ctx.space.map_element(window, (0, 0), false);
    }

    fn name(&self) -> &'static str {
        "default"
    }
}
