// Wayboard — the global compositor state.
//
// Holds every protocol state machine Smithay needs. Generic over
// Wayboard itself — Smithay monomorphizes dispatch for this struct.
//
// In Block 0 this was inline in main.rs. Now it lives in core so
// protocol handlers, input, grabs, and the shell can all share it.

use std::{ffi::OsString, sync::Arc, time};

use smithay::{
    desktop::{PopupManager, Space, Window},
    input::{Seat, SeatState},
    reexports::{
        calloop,
        calloop::LoopSignal,
        wayland_server::{
            Display, DisplayHandle,
            backend::{ClientData, ClientId, DisconnectReason},
        },
    },
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::xdg::XdgShellState,
        shm::ShmState,
        socket::ListeningSocketSource,
    },
};

// =============================================================================
// Wayboard — the compositor state
// =============================================================================

pub struct Wayboard {
    // ── Infrastructure ─────────────────────────────────────────────────
    pub start_time: time::Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,
    pub loop_signal: LoopSignal,

    // ── Spatial (not used until Block 3) ───────────────────────────────
    pub space: Space<Window>,
    pub popups: PopupManager,

    // ── Mandatory protocol states ──────────────────────────────────────
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Wayboard>,
    pub data_device_state: DataDeviceState,

    // ── Input (not used until Block 4) ─────────────────────────────────
    pub seat: Seat<Self>,
}

impl Wayboard {
    /// Build a Wayboard. Creates all mandatory protocol globals and the
    /// default seat. The event loop and listening socket are set up by
    /// the caller (frontend).
    pub fn new(display: Display<Self>, event_loop: &mut calloop::EventLoop<Self>) -> Self {
        let start_time = time::Instant::now();
        let dh = display.handle();

        // ── Register all mandatory protocols ─────────────────────────
        //
        // Each ::new::<Self>(&dh) creates a "global" — a known name that
        // clients bind to. Without these, clients see nothing and crash.
        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);

        // ── Create the default seat ──────────────────────────────────
        //
        // A seat represents one user with input devices. Every compositor
        // needs at least seat0 with a keyboard and pointer. The numbers
        // 200, 25 are key repeat rate (ms) and delay (ms).
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "seat0");
        seat.add_keyboard(Default::default(), 200, 25).unwrap();
        seat.add_pointer();

        let space = Space::default();
        let popups = PopupManager::default();

        // ── Set up the Wayland socket ────────────────────────────────
        let socket_name = Self::init_wayland_listener(display, event_loop);
        let loop_signal = event_loop.get_signal();

        Self {
            start_time,
            display_handle: dh,
            space,
            popups,
            loop_signal,
            socket_name,
            compositor_state,
            xdg_shell_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            seat,
        }
    }

    /// Create the listening socket and register it + client dispatch
    /// with the event loop. Returns the socket name so the caller can
    /// set WAYLAND_DISPLAY.
    fn init_wayland_listener(
        display: Display<Self>,
        event_loop: &mut calloop::EventLoop<Self>,
    ) -> OsString {
        use smithay::reexports::calloop::{Interest, Mode, PostAction, generic::Generic};

        let listening_socket = ListeningSocketSource::new_auto().unwrap();
        let socket_name = listening_socket.socket_name().to_os_string();
        let loop_handle = event_loop.handle();

        // Accept new client connections
        loop_handle
            .insert_source(listening_socket, move |client_stream, _, state| {
                let client = state
                    .display_handle
                    .insert_client(client_stream, Arc::new(ClientState::default()))
                    .unwrap();
                tracing::debug!(client_id = ?client.id(), "client connected");
            })
            .expect("Failed to init Wayland listener");

        // Dispatch client messages — read + parse, call handler traits
        loop_handle
            .insert_source(
                {
                    let source: Generic<Display<Self>> =
                        Generic::new(display, Interest::READ, Mode::Level);
                    source
                },
                |_, display, state| unsafe {
                    display.get_mut().dispatch_clients(state).unwrap();
                    Ok(PostAction::Continue)
                },
            )
            .unwrap();

        socket_name
    }
}

// =============================================================================
// Per-client state
// =============================================================================

/// Each connected Wayland client gets one of these. Smithay requires
/// CompositorClientState per-client to track the client's surfaces.
/// Additional client state (e.g. for layer shell) is added in later blocks.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, client_id: ClientId) {
        tracing::debug!(client_id = ?client_id, "client initialized");
    }
    fn disconnected(&self, client_id: ClientId, reason: DisconnectReason) {
        tracing::debug!(client_id = ?client_id, ?reason, "client disconnected");
    }
}
