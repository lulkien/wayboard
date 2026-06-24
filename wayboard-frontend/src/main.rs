// wayboard — a modular Wayland compositor.
//
// Block 1: Wayland socket + mandatory protocol globals.
// Clients can connect and see all expected globals (weston-info works).
// No rendering, no window placement yet — pure protocol compliance.
//
// ── DATA FLOW ───────────────────────────────────────────────────────────
//
//   Client ──connect()──▶  wayboard-core::Wayboard::init_wayland_listener
//                              │ ListeningSocketSource → client_stream
//                              │ insert_client(Arc<ClientState>)
//                              ▼ Display<Wayboard>
//                              │ dispatch_clients() → handler traits
//                              ▼ core::handlers::{compositor, xdg_shell, mod}
//
// Detailed explanation: guide/01-skeleton.md

use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
use tracing::info;
use wayboard_core::Wayboard;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    // 1. Event loop — single-threaded, callback-driven, poll/epoll-based.
    let mut event_loop: EventLoop<Wayboard> = EventLoop::try_new()?;

    // 2. Wayland Display — client manager, protocol router, global registry.
    let display: Display<Wayboard> = Display::new()?;

    // 3. Build compositor state, register all protocol globals, set up
    //    the listening socket and client dispatch. Wayboard::new does all of this.
    let state = Wayboard::new(display, &mut event_loop);

    // 4. Tell clients where the socket lives
    unsafe {
        std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);
    }
    info!(
        "wayboard running on WAYLAND_DISPLAY={}",
        state.socket_name.to_string_lossy()
    );

    // 5. Run forever (or until state.loop_signal.stop() is called)
    let mut state = state;
    event_loop.run(None, &mut state, |_| {})?;

    Ok(())
}

fn init_logging() {
    if let Ok(filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    } else {
        tracing_subscriber::fmt().init();
    }
}
