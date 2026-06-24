# Building a Wayland Compositor in Rust (with Smithay)

A block-by-block journey. Each block produces a **runnable** compositor.
You build understanding incrementally — no "vibe coding" where you type
lines you don't understand.

## Project Structure (your design, preserved)

```
my-compositor/
├── Cargo.toml                 workspace: members = ["core", "frontend", "shells/*"]
├── core/                      lib crate — mechanism
│   └── src/
│       ├── lib.rs             re-exports Shell trait, Wayboard state
│       ├── shell.rs           Shell trait + NullShell
│       ├── state.rs           Wayboard struct (all protocol states)
│       ├── input.rs           input event → seat dispatch
│       ├── handlers/
│       │   ├── mod.rs         delegate_dispatch2! + Seat/DataDevice/Output impls
│       │   ├── compositor.rs  CompositorHandler + ShmHandler + BufferHandler
│       │   └── xdg_shell.rs   XdgShellHandler (new_toplevel → shell.new_window)
│       └── grabs/
│           ├── mod.rs
│           ├── move_grab.rs   PointerGrab for window dragging
│           └── resize_grab.rs PointerGrab for window resizing
├── frontend/                  bin crate — brings it to life
│   └── src/
│       ├── main.rs            clap, config load, shell selection, event loop
│       ├── winit.rs           winit backend init + render loop
│       └── config.rs          TOML config parsing
└── shells/                    shell crates — policy (depends ONLY on core)
    └── shell-default/
        └── src/lib.rs         impl Shell for DefaultShell
```

```
┌────────────────────────────────────────────────────┐
│                    FRONTEND (binary)               │
│  main.rs: parse args → load config → pick shell    │
│  winit.rs: host window + render loop + input       │
│  config.rs: [shell], [[startup]]                   │
├────────────────────────────────────────────────────┤
│                    CORE (library)                  │
│  ┌──────────┐  ┌────────────┐  ┌────────────────┐  │
│  │ Shell    │  │ Handlers   │  │ Grabs          │  │
│  │ trait    │  │ compositor │  │ move_grab      │  │
│  │ NullShell│  │ xdg_shell  │  │ resize_grab    │  │
│  │ Context  │  │ seat etc   │  │                │  │
│  └──────────┘  └────────────┘  └────────────────┘  │
│  ┌──────────────────────────────────────────────┐  │
│  │ state.rs: Wayboard struct                    │  │
│  │   DisplayHandle, Space, Seat,                │  │
│  │   CompositorState, XdgShellState, etc.       │  │
│  └──────────────────────────────────────────────┘  │
│  ┌──────────────────────────────────────────────┐  │
│  │ input.rs: InputEvent → shell.handle_key()    │  │
│  │           → seat keyboard/pointer dispatch   │  │
│  └──────────────────────────────────────────────┘  │
├────────────────────────────────────────────────────┤
│                    SHELLS (plugins)                │
│  shell-default: new_window → fullscreen            │
│  (future) shell-tiling: BSP tree layout            │
│  (future) shell-floating: stacking WM              │
└────────────────────────────────────────────────────┘
```

## Prerequisite Knowledge

Before Block 0, understand these concepts. You don't need mastery — just
enough to recognize them when they appear in code.

### 1. The Wayland Protocol (mental model)

Wayland is an **asynchronous object-oriented protocol** over a Unix socket.
The compositor is the server; applications are clients.

```
Client                           Server (you)
  │                                  │
  │── wl_compositor.create_surface ─▶│  "I need a surface"
  │◀─────── wl_surface (id=7) ───────│  "Here you go"
  │── xdg_wm_base.get_xdg_surface ──▶│  "Make it an app window"
  │◀──── xdg_toplevel (id=12) ───────│  "Here's your toplevel"
  │─── wl_surface.attach(buffer) ───▶│  "Here's pixels"
  │─── wl_surface.commit ───────────▶│  "Show it now!"
```

Key insight: the protocol is just messages. The compositor receives them
and decides what to do. There is no "window manager" process — you ARE the
window manager.

### 2. Smithay's Role

Smithay handles the **wire protocol** so you don't have to parse Wayland
messages. You implement **handler traits** and Smithay calls them when
clients send requests.

```
Client message ──▶ Smithay parses wire ──▶ Your XdgShellHandler::new_toplevel()
                                             Your CompositorHandler::commit()
                                             Your SeatHandler::focus_changed()
```

### 3. Key Smithay Types You'll Use

| Type | What it does |
|------|-------------|
| `Display<State>` | Manages client connections. Generic over your state. |
| `DisplayHandle` | Non-generic handle. Use for creating globals. |
| `EventLoop<State>` | calloop event loop. All callbacks get `&mut State`. |
| `Space<Window>` | 2D plane where windows live. Handles Z-order, hit-testing. |
| `Window` | Wraps an xdg_toplevel. Has geometry, surface, activation. |
| `Output` | A "screen" — resolution, refresh, position in Space. |
| `Seat<State>` | Keyboard + pointer + touch for one user. |
| `CompositorState` | Tracks wl_compositor globals (surfaces, regions). |
| `XdgShellState` | Tracks xdg_wm_base globals (toplevels, popups). |

### 4. The Generic Pattern

Smithay types are generic over YOUR state struct:

```rust
Display<Wayboard>       // client manager for your compositor
Seat<Wayboard>          // input devices for your compositor
PointerGrab<Wayboard>   // grab handler for your compositor
```

This is compile-time monomorphization. Every `State` gets its own copy of
Smithay's internals. This is **why .so/dynamic plugins don't work** — the
types are baked into the binary at compile time.

### 5. The delegate_dispatch2! Macro

```rust
smithay::delegate_dispatch2!(Wayboard);
```

Without this, NONE of your handler trait impls are called. It generates
the `Dispatch` trait implementations that map Wayland protocol opcodes
to your handler methods. Think of it as the routing table.

### 6. The Event Loop (calloop)

Smithay uses `calloop`, not async/tokio. It's a single-threaded callback-based
event loop:

```rust
let mut event_loop: EventLoop<Wayboard> = EventLoop::try_new()?;

// Insert event sources (wayland socket, winit window)
event_loop.handle().insert_source(source, callback);

// Run forever
event_loop.run(None, &mut state, |_| {})?;
```

All your code runs on one thread. No locks needed for your state.
Every callback receives `&mut Wayboard`.

### 7. What "Runnable" Means

At minimum, a runnable compositor:
- Creates a Wayland socket (`WAYLAND_DISPLAY`)
- Accepts client connections
- Dispatches client messages (even if it ignores them)
- Runs an event loop that can be cleanly exited

No rendering needed. No window placement needed. Just the skeleton.

---

## Block 0: Skeleton — "Hello, I'm a Wayland socket"

**Goal:** A binary that creates a Wayland socket, accepts client connections,
and runs an event loop. Exits on Ctrl+C. No rendering, no windows.

**What you'll learn:**
- Cargo workspace setup
- Display, EventLoop, ListeningSocketSource
- `WAYLAND_DISPLAY` environment variable
- The concept that "the compositor IS the display server"

### The Code

```
my-compositor/
├── Cargo.toml
├── core/
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
└── frontend/
    ├── Cargo.toml
    └── src/
        └── main.rs
```

**core/Cargo.toml:**
```toml
[package]
name = "my-compositor-core"
version = "0.1.0"
edition = "2024"

[dependencies.smithay]
git = "https://github.com/Smithay/smithay.git"
default-features = false
features = ["wayland_frontend"]
```

**core/src/lib.rs:**
```rust
// Empty for now — we'll add state and handlers in later blocks.
```

**frontend/Cargo.toml:**
```toml
[package]
name = "my-compositor"
version = "0.1.0"
edition = "2024"

[dependencies]
my-compositor-core = { path = "../core" }
smithay = { git = "https://github.com/Smithay/smithay.git", default-features = false, features = ["wayland_frontend"] }
```

**frontend/src/main.rs:**
```rust
use smithay::reexports::calloop::{EventLoop, Interest, Mode, PostAction, generic::Generic};
use smithay::reexports::wayland_server::Display;
use smithay::wayland::socket::ListeningSocketSource;

// The state struct — will grow in later blocks. For now, it just
// holds what we need to accept connections.
struct MyCompositor {
    display_handle: smithay::reexports::wayland_server::DisplayHandle,
    loop_signal: smithay::reexports::calloop::LoopSignal,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Create the event loop
    let mut event_loop: EventLoop<MyCompositor> = EventLoop::try_new()?;

    // 2. Create the Wayland display
    let display: Display<MyCompositor> = Display::new()?;
    let dh = display.handle();

    // 3. Create a Wayland socket (wayland-0, wayland-1, etc.)
    let listening_socket = ListeningSocketSource::new_auto()?;
    let socket_name = listening_socket.socket_name().to_os_string();

    // 4. Accept new client connections
    event_loop
        .handle()
        .insert_source(listening_socket, |client_stream, _, state| {
            state
                .display_handle
                .insert_client(client_stream, std::sync::Arc::new(()))
                .unwrap();
        })?;

    // 5. Dispatch client messages (even though we ignore them)
    event_loop
        .handle()
        .insert_source(
            Generic::new(display, Interest::READ, Mode::Level),
            |_, display, state| unsafe {
                display.get_mut().dispatch_clients(state).unwrap();
                Ok(PostAction::Continue)
            },
        )?;

    let loop_signal = event_loop.get_signal();

    // 6. Tell clients where to find us
    let mut state = MyCompositor {
        display_handle: dh,
        loop_signal,
    };

    unsafe { std::env::set_var("WAYLAND_DISPLAY", &socket_name); }

    println!("Compositor running on WAYLAND_DISPLAY={}", socket_name.to_string_lossy());

    // 7. Run!
    event_loop.run(None, &mut state, |_| {})?;

    Ok(())
}
```

### Verify

```bash
cd my-compositor
cargo build
# Run in a TTY (not inside an existing Wayland/X11 session):
cargo run
# In another terminal:
WAYLAND_DISPLAY=wayland-0 weston-info    # should connect but see no globals
# Ctrl+C to exit
```

### What happened

You created a Wayland display server. It listens on a Unix socket.
Clients can connect, but they can't DO anything because you haven't
registered any protocol globals (no wl_compositor, no xdg_wm_base).

### Key insight

The compositor IS the server. There's no separate "display server" process.
This surprises people coming from X11 where Xorg is separate from the WM.
In Wayland, you are both.

---

## Block 1: Mandatory Protocols — "I speak Wayland"

**Goal:** Register all mandatory Wayland protocol globals. Clients can
connect and create surfaces/toplevels. Still no rendering — but now
clients see a real Wayland compositor.

**What you'll learn:**
- Protocol state types (CompositorState, XdgShellState, ...)
- Seat creation (keyboard + pointer)
- Per-client state via ClientData
- The full `MyCompositor` state struct pattern
- `delegate_dispatch2!`

### Add to core/src/lib.rs

```rust
pub mod state;
pub mod handlers;
```

### New file: core/src/state.rs

```rust
use smithay::{
    desktop::Space,
    desktop::Window,
    input::{Seat, SeatState},
    reexports::{
        calloop::LoopSignal,
        wayland_server::{
            Display, DisplayHandle,
            backend::{ClientData, ClientId, DisconnectReason},
        },
    },
    wayland::{
        compositor::{CompositorState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::xdg::XdgShellState,
        shm::ShmState,
    },
};
use std::sync::Arc;

pub struct MyCompositor {
    pub display_handle: DisplayHandle,
    pub loop_signal: LoopSignal,
    pub socket_name: std::ffi::OsString,

    // Mandatory protocol states
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<MyCompositor>,
    pub data_device_state: DataDeviceState,

    // Spatial
    pub space: Space<Window>,

    // Input
    pub seat: Seat<Self>,
}

impl MyCompositor {
    pub fn new(display: Display<Self>) -> Self {
        let dh = display.handle();

        // Create all mandatory protocol globals.
        // Each .new::<Self>() registers the protocol with the Display,
        // creating a "global" that clients can bind to.
        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);

        // Create a seat with one keyboard + one pointer
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "seat0");
        seat.add_keyboard(Default::default(), 200, 25).unwrap();
        seat.add_pointer();

        Self {
            display_handle: dh,
            loop_signal: LoopSignal, // placeholder, set by main after event_loop creation
            socket_name: Default::default(),
            compositor_state,
            xdg_shell_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            space: Space::default(),
            seat,
        }
    }
}

// Per-client state. Each connected client gets one of these.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: smithay::wayland::compositor::CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
```

### New file: core/src/handlers/mod.rs

```rust
mod compositor;
mod xdg_shell;

use crate::state::MyCompositor;

// ── Seat ────────────────────────────────────────────────────────

use smithay::{
    input::{Seat, SeatHandler, SeatState},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
};

impl SeatHandler for MyCompositor {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<MyCompositor> {
        &mut self.seat_state
    }

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        _image: smithay::input::pointer::CursorImageStatus,
    ) {}

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let dh = &self.display_handle;
        let client = focused.and_then(|s| dh.get_client(s.id()).ok());
        smithay::wayland::selection::data_device::set_data_device_focus(dh, seat, client);
    }
}

// ── Data Device (clipboard + DnD) ─────────────────────────────

use smithay::{
    input::{
        Seat,
        dnd::{DnDGrab, DndGrabHandler, GrabType, Source},
        pointer::Focus,
    },
    utils::Serial,
    wayland::selection::{
        SelectionHandler,
        data_device::{
            DataDeviceHandler, DataDeviceState,
            WaylandDndGrabHandler, set_data_device_focus,
        },
    },
};

impl SelectionHandler for MyCompositor {
    type SelectionUserData = ();
}

impl DataDeviceHandler for MyCompositor {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl DndGrabHandler for MyCompositor {}

impl WaylandDndGrabHandler for MyCompositor {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        _icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        type_: GrabType,
    ) {
        match type_ {
            GrabType::Pointer => {
                let ptr = seat.get_pointer().unwrap();
                let start_data = ptr.grab_start_data().unwrap();
                let grab = DnDGrab::new_pointer(&self.display_handle, start_data, source, seat);
                ptr.set_grab(self, grab, serial, Focus::Keep);
            }
            GrabType::Touch => {
                source.cancel();
            }
        }
    }
}

// ── Output ─────────────────────────────────────────────────────

use smithay::wayland::output::OutputHandler;
impl OutputHandler for MyCompositor {}

// ── THE CRITICAL LINE ─────────────────────────────────────────
// Without this, none of your handler impls are ever called.
smithay::delegate_dispatch2!(MyCompositor);
```

### New file: core/src/handlers/compositor.rs

```rust
use crate::state::{ClientState, MyCompositor};
use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    reexports::wayland_server::{
        Client,
        protocol::wl_surface::WlSurface,
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            CompositorClientState, CompositorHandler, CompositorState,
        },
        shm::{ShmHandler, ShmState},
    },
};

impl CompositorHandler for MyCompositor {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        // In later blocks: handle window commits, popups, resizes
    }
}

impl BufferHandler for MyCompositor {
    fn buffer_destroyed(&mut self, _buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer) {}
}

impl ShmHandler for MyCompositor {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}
```

### New file: core/src/handlers/xdg_shell.rs

```rust
use crate::state::MyCompositor;
use smithay::{
    wayland::shell::xdg::{
        PopupSurface, PositionerState, ToplevelSurface,
        XdgShellHandler, XdgShellState,
    },
};

impl XdgShellHandler for MyCompositor {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, _surface: ToplevelSurface) {
        // In Block 3, we'll create a Window and place it.
        // For now: do nothing. The client gets no configure → it waits forever.
        // This is fine for a "protocols work" milestone.
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {
        // Popups need a parent. We'll handle them later.
    }

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {}

    fn move_request(
        &mut self,
        _surface: ToplevelSurface,
        _seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
        _serial: smithay::utils::Serial,
    ) {}

    fn resize_request(
        &mut self,
        _surface: ToplevelSurface,
        _seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
        _serial: smithay::utils::Serial,
        _edges: smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    ) {}

    fn grab(&mut self, _surface: PopupSurface, _seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat, _serial: smithay::utils::Serial) {}
}
```

### Update frontend/src/main.rs

Now use `MyCompositor` from core instead of the inline struct:

```rust
use smithay::reexports::calloop::{EventLoop, Interest, Mode, PostAction, generic::Generic};
use smithay::reexports::wayland_server::Display;
use smithay::wayland::socket::ListeningSocketSource;
use std::sync::Arc;

use my_compositor_core::state::{ClientState, MyCompositor};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut event_loop: EventLoop<MyCompositor> = EventLoop::try_new()?;
    let display: Display<MyCompositor> = Display::new()?;

    // Wayland socket
    let listening_socket = ListeningSocketSource::new_auto()?;
    let socket_name = listening_socket.socket_name().to_os_string();

    event_loop
        .handle()
        .insert_source(listening_socket, move |client_stream, _, state| {
            state
                .display_handle
                .insert_client(client_stream, Arc::new(ClientState::default()))
                .unwrap();
        })?;

    // Client dispatch
    event_loop
        .handle()
        .insert_source(
            Generic::new(display, Interest::READ, Mode::Level),
            |_, display, state| unsafe {
                display.get_mut().dispatch_clients(state).unwrap();
                Ok(PostAction::Continue)
            },
        )?;

    let mut state = MyCompositor::new(display); // use core's constructor
    state.socket_name = socket_name.clone();
    state.loop_signal = event_loop.get_signal();

    unsafe { std::env::set_var("WAYLAND_DISPLAY", &state.socket_name); }
    println!("Compositor running. Socket: {}", state.socket_name.to_string_lossy());

    event_loop.run(None, &mut state, |_| {})?;
    Ok(())
}
```

### Verify

```bash
cargo build
cargo run &
WAYLAND_DISPLAY=wayland-0 weston-info | head -30
# You should see globals: wl_compositor, wl_seat, wl_shm,
# xdg_wm_base, wl_data_device_manager, wl_output
```

### What happened

You registered all mandatory Wayland protocols. Clients now see a
fully-featured Wayland compositor — they can create surfaces, bind shm,
and open toplevel windows. But you haven't told them WHERE to draw
those windows (no output/rendering), and you haven't sent any configure
events (no window size). So windows won't appear yet. That's Block 3.

### Mandatory vs Optional Protocols

```
MANDATORY (every compositor):
  wl_compositor   — surface creation
  wl_shm          — shared memory buffers
  xdg_wm_base     — application windows
  wl_seat         — input devices
  wl_data_device  — clipboard + drag-drop
  wl_output       — screen info

OPTIONAL (Cargo features):
  wlr_layer_shell — bars, wallpapers, overlays
  wlr_screencopy  — screenshots
  xdg_decoration  — server-side window decorations
  session_lock    — screen lock
  input_method    — on-screen keyboard
```

---

## Block 2: Winit Backend + Rendering — "I can see windows!"

**Goal:** Create a winit window that acts as the compositor's "screen."
Render client windows into it. Now `kitty` or `weston-terminal` actually
appears inside your compositor window.

**What you'll learn:**
- winit backend (the dev backend — runs as a window inside your existing session)
- Output creation and management
- Space: the 2D plane where windows live
- render_output: turning the Space into pixels
- Frame callbacks: telling clients when to draw the next frame
- The render loop pattern

### New dependency for core

**core/Cargo.toml** — add `desktop` feature:
```toml
[dependencies.smithay]
git = "https://github.com/Smithay/smithay.git"
default-features = false
features = ["wayland_frontend", "desktop"]
```

### New dependency for frontend

**frontend/Cargo.toml** — add `backend_winit`:
```toml
[dependencies.smithay]
git = "https://github.com/Smithay/smithay.git"
default-features = false
features = ["wayland_frontend", "desktop", "backend_winit"]
```

### New file: frontend/src/winit.rs

```rust
use std::time::Duration;

use smithay::{
    backend::{
        renderer::{
            damage::OutputDamageTracker,
            element::surface::WaylandSurfaceRenderElement,
            gles::GlesRenderer,
        },
        winit::{self, WinitEvent},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::EventLoop,
    utils::{Rectangle, Transform},
};

use my_compositor_core::state::MyCompositor;

pub fn init_winit(
    event_loop: &mut EventLoop<MyCompositor>,
    state: &mut MyCompositor,
) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Create the winit backend
    let (mut backend, winit) = winit::init()?;

    // 2. Create an Output matching the winit window size
    let mode = Mode {
        size: backend.window_size(),
        refresh: 60_000,
    };

    let output = Output::new(
        "winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Smithay".into(),
            model: "Winit".into(),
            serial_number: "Unknown".into(),
        },
    );
    let _global = output.create_global::<MyCompositor>(&state.display_handle);
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180), // winit renders flipped by default
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);

    // 3. Map the output into the Space (it's now a "viewport" into the Space)
    state.space.map_output(&output, (0, 0));

    let mut damage_tracker = OutputDamageTracker::from_output(&output);

    // 4. Insert winit events into the event loop
    event_loop
        .handle()
        .insert_source(winit, move |event, _, state| {
            match event {
                WinitEvent::Resized { size, .. } => {
                    output.change_current_state(
                        Some(Mode { size, refresh: 60_000 }),
                        None, None, None,
                    );
                }
                WinitEvent::Input(event) => {
                    // Block 4 will process input here
                    // state.process_input_event(event);
                }
                WinitEvent::Redraw => {
                    // ── RENDER ────────────────────────────────────
                    let size = backend.window_size();
                    let damage = Rectangle::from_size(size);

                    {
                        // bind() gives us a GlesRenderer + GlesFrame
                        let (renderer, mut framebuffer) = backend.bind().unwrap();

                        // render_output composites the Space onto the output
                        smithay::desktop::space::render_output::<
                            _,
                            WaylandSurfaceRenderElement<GlesRenderer>,
                            _,
                            _,
                        >(
                            &output,
                            renderer,
                            1.0,   // scale
                            0,     // age
                            &mut framebuffer,
                            [&state.space],
                            &[],
                            &mut damage_tracker,
                            [0.1, 0.1, 0.1, 1.0],  // clear color (dark gray)
                        )
                        .unwrap();
                    }

                    // Submit the rendered frame to the winit window
                    backend.submit(Some(&[damage])).unwrap();

                    // ── FRAME CALLBACKS ─────────────────────────
                    // Tell every window "we drew a frame" so clients
                    // know they can draw the next one.
                    state.space.elements().for_each(|window| {
                        window.send_frame(
                            &output,
                            state.start_time.elapsed(),
                            Some(Duration::ZERO),
                            |_, _| Some(output.clone()),
                        )
                    });

                    state.space.refresh();
                    let _ = state.display_handle.flush_clients();

                    // Keep the render loop going
                    backend.window().request_redraw();
                }
                WinitEvent::CloseRequested => {
                    state.loop_signal.stop();
                }
                _ => (),
            }
        })?;

    Ok(())
}
```

### Update core/src/state.rs

Add `start_time` and `popups`:

```rust
use smithay::desktop::PopupManager;
use std::time;

pub struct MyCompositor {
    pub start_time: time::Instant,  // ADD THIS
    // ... existing fields ...
    pub popups: PopupManager,       // ADD THIS
}

impl MyCompositor {
    pub fn new(display: Display<Self>) -> Self {
        // ... existing setup ...

        Self {
            start_time: time::Instant::now(),   // ADD
            // ... existing fields ...
            popups: PopupManager::default(),     // ADD
        }
    }
}
```

### Update frontend/src/main.rs

```rust
mod winit;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ... existing setup ...
    let mut state = MyCompositor::new(display);
    // ... socket setup ...

    // === ADD: init winit backend ===
    crate::winit::init_winit(&mut event_loop, &mut state)?;

    unsafe { std::env::set_var("WAYLAND_DISPLAY", &state.socket_name); }
    event_loop.run(None, &mut state, |_| {})?;
    Ok(())
}
```

### Verify

```bash
cargo build
# Run INSIDE an existing Wayland/X11 session (this opens a window):
cargo run &
WAYLAND_DISPLAY=wayland-0 kitty
# kitty should appear inside your compositor window!
```

### What happened

You now have a screen (the winit window) and a rendering pipeline.
`render_output` walks through every window in the Space, composites
their surfaces into a single frame, and `submit` sends it to the screen.

But windows still don't position themselves — `new_toplevel` does nothing.
Let's fix that.

---

## Block 3: XDG Shell — "Windows appear and get positioned"

**Goal:** When a client creates a toplevel, your compositor creates a
`Window`, maps it into the Space, and sends a configure event telling
the client what size to draw. Now apps actually work.

**What you'll learn:**
- Window creation from ToplevelSurface
- Space::map_element — putting windows on the 2D plane
- Configure events — telling clients their size
- The commit cycle: client commits → you handle it → send configure

### Update core/src/handlers/xdg_shell.rs

```rust
use smithay::{
    desktop::{PopupKind, PopupManager, Space, Window, find_popup_root_surface, get_popup_toplevel_coords},
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            Resource,
            protocol::{wl_seat, wl_surface::WlSurface},
        },
    },
    utils::{Rectangle, Serial},
    wayland::{
        compositor::with_states,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface,
            XdgShellHandler, XdgShellState, XdgToplevelSurfaceData,
        },
    },
};

impl XdgShellHandler for MyCompositor {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Create a Window from the toplevel surface
        let window = Window::new_wayland_window(surface);

        // Place it at (0, 0) in the Space — this is our "null" layout.
        // In later blocks, the Shell trait will decide placement.
        self.space.map_element(window, (0, 0), false);
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, _surface: ToplevelSurface, _seat: wl_seat::WlSeat, _serial: Serial) {}
    fn resize_request(&mut self, _surface: ToplevelSurface, _seat: wl_seat::WlSeat, _serial: Serial, _edges: xdg_toplevel::ResizeEdge) {}
    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {}
}

impl MyCompositor {
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else { return; };
        let Some(window) = self.space.elements().find(|w| {
            w.toplevel().unwrap().wl_surface() == &root
        }) else { return; };

        let output = self.space.outputs().next().unwrap();
        let output_geo = self.space.output_geometry(output).unwrap();
        let window_geo = self.space.element_geometry(window).unwrap();

        let mut target = output_geo;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= window_geo.loc;

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}

// Handle the initial commit — send the first configure event
pub fn handle_commit(popups: &mut PopupManager, space: &Space<Window>, surface: &WlSurface) {
    if let Some(window) = space
        .elements()
        .find(|w| w.toplevel().unwrap().wl_surface() == surface)
        .cloned()
    {
        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .unwrap()
                .lock()
                .unwrap()
                .initial_configure_sent
        });

        if !initial_configure_sent {
            window.toplevel().unwrap().send_configure();
        }
    }

    popups.commit(surface);
    if let Some(popup) = popups.find_popup(surface) {
        if let PopupKind::Xdg(ref xdg) = popup {
            if !xdg.is_initial_configure_sent() {
                xdg.send_configure().expect("initial configure failed");
            }
        }
    }
}
```

### Update core/src/handlers/compositor.rs — commit handler

```rust
fn commit(&mut self, surface: &WlSurface) {
    on_commit_buffer_handler::<Self>(surface);

    // Handle subsurface parent traversal
    if !is_sync_subsurface(surface) {
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        if let Some(window) = self
            .space
            .elements()
            .find(|w| w.toplevel().unwrap().wl_surface() == &root)
        {
            window.on_commit();
        }
    }

    // Handle initial configure for new windows
    super::xdg_shell::handle_commit(&mut self.popups, &self.space, surface);
}
```

Add imports at top of compositor.rs:
```rust
use smithay::wayland::compositor::{get_parent, is_sync_subsurface};
```

### Verify

```bash
cargo build && cargo run &
WAYLAND_DISPLAY=wayland-0 kitty
# kitty now appears as a window in your compositor!
# All windows stack at (0,0) — they overlap each other.
# But they're real, interactive Wayland windows!
```

### The Commit → Configure Cycle

```
Client                          Compositor
  │                                  │
  │── xdg_toplevel(nil)─────────────▶│  new_toplevel() → Window, map_element
  │── wl_surface.attach(buffer)─────▶│
  │── wl_surface.commit─────────────▶│  commit() → initial configure sent?
  │◀── xdg_toplevel.configure(0,0)──│  "Draw at this size"
  │── wl_surface.attach(buffer)─────▶│
  │── wl_surface.commit─────────────▶│  commit() → on_commit()
  │                                  │  (window now has pixels to render)
  │◀── wl_callback.done─────────────│  frame callback
```

The initial configure is critical. Without it, the client doesn't know
what size to draw and will sit there with an empty buffer forever.

---

## Block 4: Input — "I can click and type"

**Goal:** Route keyboard and pointer events from the winit backend through
the Seat to the focused client. Now you can type in kitty and click buttons.

**What you'll learn:**
- InputBackend event processing
- Keyboard focus management
- Pointer motion, button, and axis (scroll) events
- Click-to-focus: raise window on click, set keyboard focus
- `surface_under` — raycasting through the Space

### New file: core/src/input.rs

```rust
use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState,
        Event, InputBackend, InputEvent,
        KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
    },
    input::{
        keyboard::FilterResult,
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, SERIAL_COUNTER},
};

use crate::state::MyCompositor;

impl MyCompositor {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event, .. } => {
                let keycode = event.key_code();
                let pressed = event.state() == smithay::backend::input::KeyState::Pressed;

                // Forward to the client (no compositor shortcuts yet)
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);
                self.seat.get_keyboard().unwrap().input::<(), _>(
                    self,
                    keycode,
                    event.state(),
                    serial,
                    time,
                    |_, _, _| FilterResult::Forward,
                );
            }

            InputEvent::PointerMotion { .. } => {}

            InputEvent::PointerMotionAbsolute { event, .. } => {
                let output = self.space.outputs().next().unwrap();
                let output_geo = self.space.output_geometry(output).unwrap();
                let pos = event.position_transformed(output_geo.size) + output_geo.loc.to_f64();
                let serial = SERIAL_COUNTER.next_serial();

                let pointer = self.seat.get_pointer().unwrap();
                let under = self.surface_under(pos);

                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }

            InputEvent::PointerButton { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                let keyboard = self.seat.get_keyboard().unwrap();
                let serial = SERIAL_COUNTER.next_serial();
                let button_state = event.state();

                if ButtonState::Pressed == button_state && !pointer.is_grabbed() {
                    // Click-to-focus: raise the clicked window, set keyboard focus
                    if let Some((window, _loc)) = self
                        .space
                        .element_under(pointer.current_location())
                        .map(|(w, l)| (w.clone(), l))
                    {
                        self.space.raise_element(&window, true);
                        keyboard.set_focus(
                            self,
                            Some(window.toplevel().unwrap().wl_surface().clone()),
                            serial,
                        );
                        // Update activation state for all windows
                        self.space.elements().for_each(|window| {
                            window.toplevel().unwrap().send_pending_configure();
                        });
                    } else {
                        // Clicked on empty space — defocus
                        self.space.elements().for_each(|window| {
                            window.set_activated(false);
                            window.toplevel().unwrap().send_pending_configure();
                        });
                        keyboard.set_focus(self, Option::<WlSurface>::None, serial);
                    }
                }

                pointer.button(
                    self,
                    &ButtonEvent {
                        button: event.button_code(),
                        state: button_state,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }

            InputEvent::PointerAxis { event, .. } => {
                let source = event.source();

                let horizontal_amount = event.amount(Axis::Horizontal).unwrap_or_else(|| {
                    event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.
                });
                let vertical_amount = event.amount(Axis::Vertical).unwrap_or_else(|| {
                    event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.
                });
                let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal);
                let vertical_amount_discrete = event.amount_v120(Axis::Vertical);

                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                if horizontal_amount != 0.0 {
                    frame = frame.value(Axis::Horizontal, horizontal_amount);
                    if let Some(discrete) = horizontal_amount_discrete {
                        frame = frame.v120(Axis::Horizontal, discrete as i32);
                    }
                }
                if vertical_amount != 0.0 {
                    frame = frame.value(Axis::Vertical, vertical_amount);
                    if let Some(discrete) = vertical_amount_discrete {
                        frame = frame.v120(Axis::Vertical, discrete as i32);
                    }
                }
                if source == AxisSource::Finger {
                    if event.amount(Axis::Horizontal) == Some(0.0) {
                        frame = frame.stop(Axis::Horizontal);
                    }
                    if event.amount(Axis::Vertical) == Some(0.0) {
                        frame = frame.stop(Axis::Vertical);
                    }
                }

                let pointer = self.seat.get_pointer().unwrap();
                pointer.axis(self, frame);
                pointer.frame(self);
            }

            _ => {}
        }
    }

    fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.space
            .element_under(pos)
            .and_then(|(window, location)| {
                window
                    .surface_under(pos - location.to_f64(), smithay::desktop::WindowSurfaceType::ALL)
                    .map(|(s, p)| (s, (p + location).to_f64()))
            })
    }
}
```

### Update frontend/src/winit.rs

Uncomment the input processing line:
```rust
WinitEvent::Input(event) => {
    state.process_input_event(event);
}
```

### Verify

```bash
cargo build && cargo run &
WAYLAND_DISPLAY=wayland-0 kitty
# Now you can TYPE in kitty!
# Click another kitty window — it comes to front, gets keyboard focus.
# Scroll works.
# You have a basic working compositor!
```

---

## Block 5: Shell Trait — "Separate mechanism from policy"

**Goal:** Extract window placement and key handling into a `Shell` trait.
The core provides mechanism (Space, Seat, protocols); the shell provides
policy (where windows go, what keys do). This is your wayboard-style
architecture.

**What you'll learn:**
- Trait-based plugin architecture
- `ShellContext` — loan pattern (the shell borrows, doesn't own)
- NullShell — minimal working shell
- How the core calls the shell

### New file: core/src/shell.rs

```rust
use smithay::{
    desktop::{Space, Window},
    input::Seat,
    reexports::wayland_server::DisplayHandle,
    wayland::shm::ShmState,
};

use crate::state::MyCompositor;

/// Context the shell receives during hook calls.
/// The shell borrows these — it CANNOT store references.
pub struct ShellContext<'a> {
    pub space: &'a mut Space<Window>,
    pub seat: &'a mut Seat<MyCompositor>,
    pub display_handle: &'a DisplayHandle,
    pub shm_state: &'a ShmState,
}

pub trait Shell {
    /// A client created a new toplevel. Place it in the space.
    fn new_window(&mut self, ctx: &mut ShellContext, window: Window);

    /// A key was pressed. Return true if the shell consumed it
    /// (global shortcut). Return false → forward to client.
    fn handle_key(&mut self, _ctx: &mut ShellContext, _keycode: u32, _pressed: bool) -> bool {
        false
    }

    /// Called each frame. Use for layout recalculation, animations.
    fn tick(&mut self, _ctx: &mut ShellContext) {}

    /// Human-readable name for config/debugging.
    fn name(&self) -> &'static str;
}

// ── NullShell — does the minimum ───────────────────────────────

pub struct NullShell;

impl Shell for NullShell {
    fn new_window(&mut self, ctx: &mut ShellContext, window: Window) {
        // Place every window at (0, 0). They'll overlap.
        ctx.space.map_element(window, (0, 0), false);
    }

    fn name(&self) -> &'static str {
        "null"
    }
}
```

### Update core/src/lib.rs

```rust
pub mod grabs;
pub mod handlers;
pub mod input;
pub mod shell;
pub mod state;

pub use shell::{NullShell, Shell, ShellContext};
pub use state::MyCompositor;
```

### Update core/src/state.rs

Add `shell` field and change constructor:

```rust
use crate::shell::Shell;

pub struct MyCompositor {
    // ... existing fields ...
    pub shell: Box<dyn Shell>,   // ADD
}

impl MyCompositor {
    pub fn new(display: Display<Self>, shell: Box<dyn Shell>) -> Self {
        // ... existing protocol setup ...

        Self {
            // ... existing fields ...
            shell,               // ADD
        }
    }
}
```

### Update core/src/handlers/xdg_shell.rs

Replace the inline placement with shell delegation:

```rust
fn new_toplevel(&mut self, surface: ToplevelSurface) {
    let window = Window::new_wayland_window(surface);

    let mut ctx = crate::shell::ShellContext {
        space: &mut self.space,
        seat: &mut self.seat,
        display_handle: &self.display_handle,
        shm_state: &self.shm_state,
    };

    self.shell.new_window(&mut ctx, window);
}
```

### Update core/src/input.rs

Before forwarding keys to the client, ask the shell:

```rust
InputEvent::Keyboard { event, .. } => {
    let keycode = event.key_code();
    let pressed = event.state() == smithay::backend::input::KeyState::Pressed;

    // Ask the shell if it wants this key
    let consumed = {
        let mut ctx = crate::shell::ShellContext {
            space: &mut self.space,
            seat: &mut self.seat,
            display_handle: &self.display_handle,
            shm_state: &self.shm_state,
        };
        self.shell.handle_key(&mut ctx, keycode.into(), pressed)
    };

    if !consumed {
        let serial = SERIAL_COUNTER.next_serial();
        let time = Event::time_msec(&event);
        self.seat.get_keyboard().unwrap().input::<(), _>(
            self, keycode, event.state(), serial, time,
            |_, _, _| FilterResult::Forward,
        );
    }
}
```

### Update frontend/src/main.rs

Pass a shell to the state:

```rust
use my_compositor_core::{NullShell, Shell};

let shell: Box<dyn Shell> = Box::new(NullShell);
let mut state = MyCompositor::new(display, shell);
```

### Verify

```bash
cargo build && cargo run &
WAYLAND_DISPLAY=wayland-0 kitty
# Same behavior as before, but now the placement logic
# lives in NullShell, not in the handler.
```

### The Architecture Boundary

```
CORE (mechanism)                    SHELL (policy)
─────────────────                  ──────────────
Event loop                          Window placement
Protocol dispatch                   Keybindings
Space (2D plane)                   Layout algorithm
Seat (input devices)               Focus policy
Renderer                           Workspace management
Grabs (move/resize)                Bar/panel logic
```

The core calls the shell at specific hook points:
- `shell.new_window()` — when a client creates a toplevel
- `shell.handle_key()` — when a key is pressed (shell gets first dibs)
- `shell.tick()` — each frame, for layout updates

---

## Block 6: Shell Plugin — "Different shells, same core"

**Goal:** Create a separate crate with a different shell that places
windows fullscreen. Select shell at startup via config file.

**What you'll learn:**
- Shell as a separate crate depending only on core
- Runtime shell selection via config
- Startup commands (launch waybar, wallpaper, terminal)
- Config-driven compositor

### New crate: shells/shell-default/

**shells/shell-default/Cargo.toml:**
```toml
[package]
name = "shell-default"
version = "0.1.0"
edition = "2024"

[dependencies]
my-compositor-core = { path = "../../core" }
smithay = { git = "https://github.com/Smithay/smithay.git", default-features = false, features = ["wayland_frontend", "desktop"] }
```

**shells/shell-default/src/lib.rs:**
```rust
use smithay::{
    desktop::Window,
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::Size,
};
use my_compositor_core::shell::{Shell, ShellContext};

pub struct DefaultShell;

impl DefaultShell {
    pub fn new() -> Self { Self }
}

impl Shell for DefaultShell {
    fn new_window(&mut self, ctx: &mut ShellContext, window: Window) {
        // Get output size
        let output_size = ctx
            .space
            .outputs()
            .next()
            .and_then(|o| ctx.space.output_geometry(o))
            .map(|geo| geo.size)
            .unwrap_or(Size::from((800, 600)));

        // Tell the client to be fullscreen
        let toplevel = window.toplevel().unwrap();
        toplevel.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Fullscreen);
            state.size = Some(output_size);
        });

        ctx.space.map_element(window, (0, 0), false);
    }

    fn name(&self) -> &'static str {
        "default"
    }
}
```

### Add config support

**frontend/src/config.rs:**
```rust
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub shell: ShellConfig,
    #[serde(default)]
    pub startup: Vec<StartupCommand>,
}

#[derive(Deserialize)]
#[serde(default)]
pub struct ShellConfig {
    pub name: String,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self { name: "default".into() }
    }
}

#[derive(Deserialize)]
pub struct StartupCommand {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

impl Config {
    pub fn load(path: Option<&PathBuf>) -> Self {
        let path = path.cloned().unwrap_or_else(default_config_path);
        match std::fs::read_to_string(&path) {
            Ok(contents) => toml::from_str(&contents).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }
}

fn default_config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap()).join(".config")
        });
    base.join("my-compositor").join("config.toml")
}
```

### Update frontend/src/main.rs

```rust
mod config;
mod winit;

use clap::Parser;
use my_compositor_core::{NullShell, Shell, state::MyCompositor};
use smithay::reexports::{calloop::EventLoop, wayland_server::Display};

#[derive(Parser)]
struct Cli {
    #[arg(short = 'c', long = "config")]
    config: Option<std::path::PathBuf>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let config = config::Config::load(cli.config.as_ref());

    // Select shell by config
    let shell: Box<dyn Shell> = match config.shell.name.as_str() {
        "default" => Box::new(shell_default::DefaultShell::new()),
        _ => Box::new(NullShell),
    };

    let mut event_loop: EventLoop<MyCompositor> = EventLoop::try_new()?;
    let display: Display<MyCompositor> = Display::new()?;

    // ... socket setup (same as before) ...

    let mut state = MyCompositor::new(display, shell);
    // ... socket name + loop_signal ...

    crate::winit::init_winit(&mut event_loop, &mut state)?;
    unsafe { std::env::set_var("WAYLAND_DISPLAY", &state.socket_name); }

    // Run startup commands
    for cmd in &config.startup {
        let mut child = std::process::Command::new(&cmd.command);
        child.args(&cmd.args);
        child.spawn().ok();
    }

    event_loop.run(None, &mut state, |_| {})?;
    Ok(())
}
```

### Config file (~/.config/my-compositor/config.toml):

```toml
[shell]
name = "default"

[[startup]]
command = "kitty"
```

### Verify

```bash
cargo build && cargo run
# kitty auto-starts, fullscreen in the compositor window.
# Edit config → change name to "null" → kitty appears at (0,0).
```

---

## Block 7: Move/Resize Grabs — "Drag windows around"

**Goal:** Implement pointer grabs so the user can drag windows to move
them and drag edges to resize them. These are mechanism, not policy —
they work the same in any shell.

**What you'll learn:**
- PointerGrab trait — full-screen modal input capture
- Grab lifecycle: start → motion → button release → unset
- Resize edge semantics: LEFT/TOP edges affect position AND size
- Surface data map for cross-handler state sharing (resize state)

### New file: core/src/grabs/mod.rs

```rust
pub mod move_grab;
pub mod resize_grab;

pub use move_grab::MoveSurfaceGrab;
pub use resize_grab::ResizeSurfaceGrab;
```

### New file: core/src/grabs/move_grab.rs

```rust
use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent,
        GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
        GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent,
        GrabStartData as PointerGrabStartData, MotionEvent, PointerGrab,
        PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point},
};
use crate::state::MyCompositor;

pub struct MoveSurfaceGrab {
    pub start_data: PointerGrabStartData<MyCompositor>,
    pub window: Window,
    pub initial_window_location: Point<i32, Logical>,
}

impl PointerGrab<MyCompositor> for MoveSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut MyCompositor,
        handle: &mut PointerInnerHandle<'_, MyCompositor>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // No client gets pointer focus during the grab
        handle.motion(data, None, event);

        // Move the window by the cursor delta from grab start
        let delta = event.location - self.start_data.location;
        let new_location = self.initial_window_location.to_f64() + delta;
        data.space.map_element(self.window.clone(), new_location.to_i32_round(), true);
    }

    fn button(
        &mut self,
        data: &mut MyCompositor,
        handle: &mut PointerInnerHandle<'_, MyCompositor>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        const BTN_LEFT: u32 = 0x110;
        if !handle.current_pressed().contains(&BTN_LEFT) {
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    // Pass-through for everything else
    fn relative_motion(&mut self, data: &mut MyCompositor, handle: &mut PointerInnerHandle<'_, MyCompositor>, focus: Option<(WlSurface, Point<f64, Logical>)>, event: &RelativeMotionEvent) { handle.relative_motion(data, focus, event); }
    fn axis(&mut self, data: &mut MyCompositor, handle: &mut PointerInnerHandle<'_, MyCompositor>, details: AxisFrame) { handle.axis(data, details); }
    fn frame(&mut self, data: &mut MyCompositor, handle: &mut PointerInnerHandle<'_, MyCompositor>) { handle.frame(data); }
    fn gesture_swipe_begin(&mut self, data: &mut MyCompositor, handle: &mut PointerInnerHandle<'_, MyCompositor>, event: &GestureSwipeBeginEvent) { handle.gesture_swipe_begin(data, event); }
    fn gesture_swipe_update(&mut self, data: &mut MyCompositor, handle: &mut PointerInnerHandle<'_, MyCompositor>, event: &GestureSwipeUpdateEvent) { handle.gesture_swipe_update(data, event); }
    fn gesture_swipe_end(&mut self, data: &mut MyCompositor, handle: &mut PointerInnerHandle<'_, MyCompositor>, event: &GestureSwipeEndEvent) { handle.gesture_swipe_end(data, event); }
    fn gesture_pinch_begin(&mut self, data: &mut MyCompositor, handle: &mut PointerInnerHandle<'_, MyCompositor>, event: &GesturePinchBeginEvent) { handle.gesture_pinch_begin(data, event); }
    fn gesture_pinch_update(&mut self, data: &mut MyCompositor, handle: &mut PointerInnerHandle<'_, MyCompositor>, event: &GesturePinchUpdateEvent) { handle.gesture_pinch_update(data, event); }
    fn gesture_pinch_end(&mut self, data: &mut MyCompositor, handle: &mut PointerInnerHandle<'_, MyCompositor>, event: &GesturePinchEndEvent) { handle.gesture_pinch_end(data, event); }
    fn gesture_hold_begin(&mut self, data: &mut MyCompositor, handle: &mut PointerInnerHandle<'_, MyCompositor>, event: &GestureHoldBeginEvent) { handle.gesture_hold_begin(data, event); }
    fn gesture_hold_end(&mut self, data: &mut MyCompositor, handle: &mut PointerInnerHandle<'_, MyCompositor>, event: &GestureHoldEndEvent) { handle.gesture_hold_end(data, event); }
    fn start_data(&self) -> &PointerGrabStartData<MyCompositor> { &self.start_data }
    fn unset(&mut self, _data: &mut MyCompositor) {}
}
```

### The Resize Grab

This is the most complex piece. The key insight: resizing the left/top
edge means changing BOTH the window's size AND its position. And position
changes must wait until the client commits a buffer at the new size.

**core/src/grabs/resize_grab.rs** — see your wayboard's resize_grab.rs
for the full implementation (~550 lines). The critical parts:

1. Edge bitflags: `TOP`, `BOTTOM`, `LEFT`, `RIGHT`, `TOP_LEFT`, etc.
2. Motion: compute new size from delta, clamp to min/max, send configure
3. Button release: send final configure, transition to `WaitingForLastCommit`
4. `handle_commit()`: on surface commit, adjust position for TOP/LEFT edges

### Wire up move/resize in xdg_shell handler

```rust
fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
    let seat = Seat::from_resource(&seat).unwrap();
    let wl_surface = surface.wl_surface();

    if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
        let pointer = seat.get_pointer().unwrap();
        let window = self.space.elements()
            .find(|w| w.toplevel().unwrap().wl_surface() == wl_surface)
            .unwrap().clone();
        let initial_window_location = self.space.element_location(&window).unwrap();

        let grab = MoveSurfaceGrab {
            start_data,
            window,
            initial_window_location,
        };
        pointer.set_grab(self, grab, serial, Focus::Clear);
    }
}

fn resize_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial, edges: xdg_toplevel::ResizeEdge) {
    // Similar: create ResizeSurfaceGrab, call pointer.set_grab()
}
```

### Verify

```bash
cargo build && cargo run &
WAYLAND_DISPLAY=wayland-0 kitty
# Hold Super (or whatever your client uses) + left-click: drag the window!
# Drag from edges: resize!
```

### How Grabs Work

```
Normal mode:
  Input → pointer.motion() → client under cursor gets wl_pointer.motion

Grab mode:
  Input → PointerGrab::motion() → YOUR CODE (move/resize logic)
         → handle.motion(data, None, ...) — NO client gets events
  Button release → handle.unset_grab() → back to normal mode
```

A PointerGrab is modal — while active, it intercepts ALL pointer events.
No client receives pointer events during the grab.

---

## Block 8+: What's Next (the roadmap from here)

At this point you have a functional compositor. Here's what you can add:

### Layer Shell (bars, wallpapers, overlays)

```toml
# Cargo feature
layer-shell = []
```

```rust
#[cfg(feature = "layer-shell")]
let layer_shell_state = LayerShellState::new::<MyCompositor>(&dh);

#[cfg(feature = "layer-shell")]
impl LayerShellHandler for MyCompositor { ... }
```

### Server-Side Decorations (window titlebars)

Let the compositor draw titlebars so windows look native without
requiring client-side decorations.

### Workspaces

Multiple virtual desktops. Each workspace is a different set of windows
mapped/unmapped from the Space. Switch workspaces by keybinding.

```rust
struct WorkspaceShell {
    workspaces: Vec<Vec<Window>>,
    active: usize,
}

impl Shell for WorkspaceShell {
    fn handle_key(&mut self, ctx: &mut ShellContext, keycode: u32, pressed: bool) -> bool {
        // Super+1 → workspace 1, Super+2 → workspace 2, etc.
    }
}
```

### Tiling Layout (BSP tree)

```rust
struct TilingShell {
    tree: BspTree,
    bindings: HashMap<(u32, ModifiersState), Action>,
}

impl Shell for TilingShell {
    fn new_window(&mut self, ctx: &mut ShellContext, window: Window) {
        self.tree.insert(window, focused_node);
        self.apply_layout(ctx);
    }
}
```

### DRM/KMS Backend (bare metal)

Replace the winit dev backend with a real DRM/KMS + libinput backend
that runs directly on the hardware (no host compositor needed):

```
Backend options:
  Winit:     development, nested in existing session
  DRM/KMS:   production, bare metal (like sway, hyprland)
  Headless:  CI/testing, no display
```

### XWayland

Run X11 applications inside your Wayland compositor. Requires:
- `xwayland` Cargo feature on smithay
- An XWayland server process spawned by your compositor
- Additional protocol handlers

---

## Pitfalls We've Hit (learn from our scars)

### 1. Raw GL after render_output → EGL BAD_ALLOC

NEVER call `renderer.with_context()` (raw GL) after `render_output()` in
the same `bind()` scope. It corrupts EGL surface state.

**Fix:** One-pass rendering. For internal bars, use `clear_color` + window
offset — the bar is just the clear color where no windows cover it.

### 2. backend.submit() can fail with ContextLost

On some Wayland hosts, `backend.submit()` fails with
`ContextLost(EGLCreateSurface(BadAlloc))`. Not a code bug — an EGL-level issue.

**Fix:** handle gracefully:
```rust
if let Err(e) = backend.submit(Some(&[damage])) {
    eprintln!("submit failed: {e:?}, skipping frame");
    backend.window().request_redraw();
    return;
}
```

### 3. Borrow scope in render loop

`backend.bind()` borrows `backend` mutably until the framebuffer is dropped.
You CANNOT call `backend.window().request_redraw()` inside that scope.

**Fix:** bind in a tight block:
```rust
{
    let (renderer, mut framebuffer) = backend.bind().unwrap();
    // ... render ...
} // borrow released here

backend.submit(...)?;
backend.window().request_redraw(); // OK now
```

### 4. .so plugins don't work with Smithay

Smithay monomorphizes at compile time. You can't put `PointerGrab<State>`
behind FFI. Use `Box<dyn Shell>` for policy, Cargo features for optional
protocols — all compiled into the same binary.

### 5. The initial_configure_sent flag

Without sending the first configure event, the client waits forever.
This happens in `commit()` after the client's first `wl_surface.commit()`.

### 6. listen+Fds aren't Signalfd-safe with SIGUSR1

If you add signal handling (for SIGUSR1 to reload config), FDs like
inotify watches won't work alongside signalfd.

---

## Reference Compositors to Study

These are the compositors in `~/Projects/wayland/` we analyzed:

| Compositor | Lines | Style | What to learn from it |
|-----------|-------|-------|----------------------|
| **wayboard** (yours) | ~2K | Core/shell split, winit | Clean architecture, good docs |
| **wprs** (Google) | ~30K | Network-transparent proxy | Production patterns, serialization |
| **niri** | ~50K+ | Scrollable tiling, DRM/KMS | Production GPU rendering, animations |
| **strata** | ~10K | Layouts, bindings, decorations | Plugin-like layout system |
| **MagmaWM** | ~5K | Config-driven, backends | Multi-backend architecture |

### niri's approach (for reference)

niri is a production compositor with:
- DRM/KMS backend (no winit dev mode)
- Its own GPU abstraction layer over Vulkan/EGL
- Scrollable tiling: windows live on a horizontal strip
- Real-time animations (gesture-driven workspace switching)
- IPC protocol for external control (niri-ipc crate)

It's the "pinnacle" of what a Smithay compositor can be — but start
with wayboard's simplicity.

---

## Key Takeaways

1. **The compositor IS the display server.** You're not writing a WM
   that sits on top of something else. You ARE the thing.

2. **Smithay handles the wire protocol.** You implement handler traits
   that get called when clients send messages.

3. **Mechanism vs Policy.** The core handles Wayland protocols, the
   Space, rendering, and input. The shell decides where windows go
   and what keys do. Keep them separate.

4. **Build incrementally.** Every block adds one concept and produces
   a working binary. You never have to debug 1000 lines at once.

5. **Start with winit, graduate to DRM/KMS.** Winit lets you develop
   inside your existing session. DRM/KMS is for when you're ready to
   run on bare metal.

6. **The Space is your canvas.** Everything lives on a 2D plane.
   Outputs are viewports into it. Windows are rectangles on it.
   Rendering composites it all into a frame.

---

*Document written from analysis of wayboard, wprs, niri, strata, and MagmaWM
compositors — plus the Smithay smallvil example that inspired them all.*
