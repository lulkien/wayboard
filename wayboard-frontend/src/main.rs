// wayboard — a modular Wayland compositor.
//
// BLOCK 0: Wayland socket + event loop. No protocols, no rendering.
//
// ── DATA FLOW ───────────────────────────────────────────────────────────
//
//   Client ──connect()──▶  Unix socket (wayland-0)
//                              │
//                              ▼ ListeningSocketSource → client_stream
//                              ▼ insert_client() → Display
//                              ▼ dispatch_clients() → (nothing yet)
//
// Detailed explanation: guide/01-skeleton.md

use smithay::{
    reexports::{
        calloop::{EventLoop, Interest, LoopSignal, Mode, PostAction, generic::Generic},
        wayland_server::Display,
    },
    wayland::socket::ListeningSocketSource,
};

#[allow(unused)]
struct Wayboard {
    /// Non-generic handle to the Display. Used for inserting clients,
    /// creating globals (Block 1), looking up clients by ID.
    display_handle: smithay::reexports::wayland_server::DisplayHandle,
    /// Handle to stop the event loop. Wired to winit close button in Block 2.
    loop_signal: LoopSignal,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Event loop — single-threaded, callback-driven, poll/epoll-based.
    let mut event_loop: EventLoop<Wayboard> = EventLoop::try_new()?;

    // 2. Wayland Display — client manager, protocol router, global registry.
    let display: Display<Wayboard> = Display::new()?;
    let dh = display.handle(); // clone before Display moves into Generic

    // 3. Unix socket at $XDG_RUNTIME_DIR/wayland-0 (or -1, -2...)
    let listening_socket = ListeningSocketSource::new_auto()?;
    let socket_name = listening_socket.socket_name().to_os_string();

    // 4. Accept new clients — fires callback on each connect()
    event_loop
        .handle()
        .insert_source(listening_socket, move |client_stream, _, state| {
            state
                .display_handle
                .insert_client(client_stream, std::sync::Arc::new(()))
                .unwrap();
        })?;

    // 5. Dispatch client messages — read+parse, call handler traits
    event_loop.handle().insert_source(
        Generic::new(display, Interest::READ, Mode::Level),
        |_, display, state| unsafe {
            display.get_mut().dispatch_clients(state).unwrap();
            Ok(PostAction::Continue)
        },
    )?;

    let loop_signal = event_loop.get_signal();

    let mut state = Wayboard {
        display_handle: dh,
        loop_signal,
    };

    // 6. Tell clients where the socket lives
    unsafe {
        std::env::set_var("WAYLAND_DISPLAY", &socket_name);
    }
    println!(
        "wayboard running on WAYLAND_DISPLAY={}",
        socket_name.to_string_lossy()
    );

    // 7. Run forever (or until loop_signal.stop())
    event_loop.run(None, &mut state, |_| {})?;

    Ok(())
}
