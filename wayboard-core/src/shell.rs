use smithay::{
    desktop::{Space, Window},
    input::Seat,
    reexports::wayland_server::DisplayHandle,
    wayland::shm::ShmState,
};

use crate::state::Wayboard;

pub struct ShellContext<'a> {
    pub space: &'a mut Space<Window>,
    pub seat: &'a mut Seat<Wayboard>,
    pub display_handle: &'a DisplayHandle,
    pub shm_state: &'a ShmState,
}

pub trait Shell {
    /// A client created a new toplevel. Place it.
    fn new_window(&mut self, ctx: &mut ShellContext, window: Window);

    /// A key was pressed. Return true if consumed (shortcut).
    fn handle_key(&mut self, _ctx: &mut ShellContext, _keycode: u32, _pressed: bool) -> bool {
        false
    }

    fn name(&self) -> &'static str;

    /// Called each frame.
    fn tick(&mut self, _ctx: &mut ShellContext) {}
}

pub struct NullShell;

impl Shell for NullShell {
    fn new_window(&mut self, ctx: &mut ShellContext, window: Window) {
        ctx.space.map_element(window, (0, 0), false);
    }

    fn name(&self) -> &'static str {
        "null"
    }
}
