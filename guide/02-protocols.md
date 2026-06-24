# Journey to a Compositor — Chapter 2: Speaking Wayland

Block 0 gave us a socket. Block 1 makes it a real Wayland compositor.
Clients now see protocol globals, create surfaces and toplevels, and
don't crash on connect. But nothing renders yet — that's Block 2.

---

## What Changed

```
Block 0                           Block 1
─────────                         ─────────
main.rs: inline Wayboard struct   core/state.rs: Wayboard + ClientState
main.rs: socket + dispatch        state.rs: init_wayland_listener()
no handlers                       handlers/{mod,compositor,xdg_shell}.rs
no delegate_dispatch2!            delegate_dispatch2!(Wayboard)
no protocol globals               8 mandatory globals registered
no seat                           seat0 with keyboard + pointer
```

---

## New Concepts

### 1. Protocol State Objects

Every Wayland protocol needs a "state object" in your compositor.
These are Smithay types that track the protocol's internal bookkeeping.

| Field | Type | Protocol | What It Tracks |
|---|---|---|---|
| `compositor_state` | `CompositorState` | wl_compositor | All surfaces, regions, subsurfaces |
| `xdg_shell_state` | `XdgShellState` | xdg_wm_base | All toplevel windows + popups |
| `shm_state` | `ShmState` | wl_shm | Shared memory buffer formats |
| `output_manager_state` | `OutputManagerState` | wl_output + xdg_output | Screen info for clients |
| `seat_state` | `SeatState<Self>` | wl_seat | Input device registration |
| `data_device_state` | `DataDeviceState` | wl_data_device | Clipboard + drag-drop |

**How they're created:**

```rust
let compositor_state = CompositorState::new::<Self>(&display_handle);
```

`::new::<Self>(&dh)` does two things:
1. Creates the state object
2. **Registers a global** — the protocol name (e.g. `wl_compositor`)
   becomes visible to clients at a known object ID

Without this call, the global doesn't exist. A client that tries to
bind `wl_compositor` gets a protocol error and disconnects. This is
why Block 0 clients crash immediately.

**The `::<Self>` turbofish:** Smithay monomorphizes the protocol
state for YOUR compositor struct. When a client message arrives,
Smithay calls your handler with `&mut Wayboard` — the generic
parameter is how it knows your type at compile time.

### 2. Per-Client State — ClientData

Each connected Wayland client gets its own `ClientState`:

```rust
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}
```

`CompositorClientState` is MANDATORY. Smithay needs it to track which
surfaces belong to which client. Without it, `CompositorHandler::commit`
can't look up the client's surfaces.

The `ClientData` trait wires it in:

```rust
impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
```

`initialized` fires when a client finishes the handshake. `disconnected`
fires when a client drops the connection. Both are no-ops for now.

When a client connects, we wrap their state in an `Arc`:

```rust
display_handle.insert_client(client_stream, Arc::new(ClientState::default()))
```

The `Arc` is required by Smithay — it may need to access client state
from multiple internal callbacks.

### 3. Handler Traits — "When X happens, call my function"

For each protocol state, you implement a handler trait. Smithay calls
these when clients send messages:

```
Client sends xdg_wm_base.get_xdg_surface
  → Smithay parses the wire format
  → Looks up the XdgShellHandler impl for Wayboard
  → Calls XdgShellHandler::new_toplevel(&mut self, surface)
```

| Trait | Key method | When called |
|---|---|---|
| `CompositorHandler` | `commit()` | Client calls `wl_surface.commit()` |
| `XdgShellHandler` | `new_toplevel()` | Client creates a window |
| `SeatHandler` | `focus_changed()` | Keyboard focus moves |
| `DataDeviceHandler` | `data_device_state()` | Smithay needs the state ref |
| `ShmHandler` | `shm_state()` | Smithay needs the state ref |
| `BufferHandler` | `buffer_destroyed()` | Client destroys a pixel buffer |
| `OutputHandler` | (none required) | Marker trait |
| `SelectionHandler` | (none required) | Clipboard selection |

Most are one-liners that just return `&mut self.xxx_state`. The real
logic lives in `commit()`, `new_toplevel()`, and `focus_changed()`.

**In Block 1, `new_toplevel()` is intentionally empty.** The client
calls it, we do nothing, the client waits forever for a configure
event. It doesn't crash — it just doesn't draw. This is correct
for a "protocols work" milestone.

### 4. The Seat — "Here is your keyboard and mouse"

```rust
let mut seat_state = SeatState::new();
let mut seat = seat_state.new_wl_seat(&dh, "seat0");
seat.add_keyboard(Default::default(), 200, 25).unwrap();
seat.add_pointer();
```

A `Seat` is a collection of input devices for one user. Every Wayland
compositor needs at least one.

- `"seat0"` — the seat name. Multi-seat setups would have `seat1`, etc.
- `add_keyboard(repeat_info, rate_ms, delay_ms)` — key repeat: 200ms
  initial delay, 25ms between repeats. `Default::default()` for keymap
  (US QWERTY).
- `add_pointer()` — a pointing device (mouse/trackpad).

Without a seat, clients can't receive keyboard or pointer events.
They'd crash on `wl_seat.get_keyboard()`.

`SeatState` and `Seat` are both generic over `Wayboard` because input
dispatch calls your handler methods with `&mut Wayboard`.

### 5. delegate_dispatch2! — The Routing Table

```rust
smithay::delegate_dispatch2!(Wayboard);
```

This macro is **the most important line in the compositor.** Without it,
Smithay has no way to know which handler trait to call for which
protocol opcode.

**What it generates:** `Dispatch` trait implementations for every
Wayland protocol. Each impl maps a protocol opcode (like
`wl_surface::Request::Commit`) to your handler method (like
`CompositorHandler::commit()`).

**Where it goes:** In `handlers/mod.rs`, AFTER all your handler
impls are defined. The macro reads the current scope and finds
your `impl XxxHandler for Wayboard` blocks.

**Why exactly once:** Calling it twice would generate duplicate
trait impls (compile error). Calling it zero times means no
dispatch (clients connect but no handlers fire — Block 0 behavior).

### 6. The Listening Socket — Now in core

In Block 0, the socket setup was in `main.rs`. In Block 1, it moves
to `Wayboard::init_wayland_listener()`. This keeps `main.rs` minimal:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut event_loop: EventLoop<Wayboard> = EventLoop::try_new()?;
    let display: Display<Wayboard> = Display::new()?;
    let state = Wayboard::new(display, &mut event_loop);
    unsafe { std::env::set_var("WAYLAND_DISPLAY", &state.socket_name); }
    let mut state = state;
    event_loop.run(None, &mut state, |_| {})?;
    Ok(())
}
```

The difference from Block 0: `insert_client` now passes
`Arc::new(ClientState::default())` instead of `Arc::new(())`.
This gives each client a proper `CompositorClientState`.

### 7. The commit() Cycle (Preview)

When a client wants to show something:

```
1. Client creates a wl_buffer (allocates pixels via shm or dmabuf)
2. Client calls wl_surface.attach(buffer) — links buffer to surface
3. Client calls wl_surface.commit() — publishes the frame
```

Step 3 triggers `CompositorHandler::commit()`. In Block 1, `commit()`
only calls `on_commit_buffer_handler` — which tells Smithay "this
buffer exists, track it." In Block 3, `commit()` will also send the
first `xdg_toplevel.configure` event, telling the client what size
to draw. Without that configure, the client stalls.

### 8. DnD (Drag and Drop)

The DnD handler is mostly boilerplate. When a client starts a drag:

```rust
impl WaylandDndGrabHandler for Wayboard {
    fn dnd_requested(...) {
        match type_ {
            GrabType::Pointer => {
                // Start a pointer grab to track the drag
                let grab = DnDGrab::new_pointer(&self.display_handle, ...);
                ptr.set_grab(self, grab, serial, Focus::Keep);
            }
            GrabType::Touch => {
                source.cancel(); // Touch DnD not supported yet
            }
        }
    }
}
```

This is identical across all Smithay compositors. It's part of the
mandatory protocol set because `wl_data_device` is mandatory.

---

## The Wayboard Struct (Full)

```rust
pub struct Wayboard {
    // Infrastructure
    pub start_time: Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,
    pub loop_signal: LoopSignal,

    // Spatial (not used until Block 3)
    pub space: Space<Window>,
    pub popups: PopupManager,

    // Mandatory protocol states
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Wayboard>,
    pub data_device_state: DataDeviceState,

    // Input (not used until Block 4)
    pub seat: Seat<Self>,
}
```

16 fields. In Block 0: 2 fields. The compositor grows one chunk at a time.

---

## What Still Doesn't Work

| Feature | Why not yet | Fixed in |
|---|---|---|
| Windows appear | No output, no rendering, no configure | Block 2+3 |
| Input (keyboard/mouse) | No backend, no input processing | Block 2+4 |
| Window placement | new_toplevel() is empty | Block 3+5 |
| Cursor changes | cursor_image() is no-op | Block 4 |
| Shell plugins | No Shell trait | Block 5 |
| Move/resize | No grabs | Block 7 |

---

## Verification

```bash
cargo run &
WAYLAND_DISPLAY=wayland-0 weston-info | head -20
```

Expected output:
```
interface: 'wl_compositor', version: 5
interface: 'wl_subcompositor', version: 1
interface: 'wl_shm', version: 1
interface: 'wl_seat', version: 8
interface: 'xdg_wm_base', version: 5
interface: 'wl_data_device_manager', version: 3
interface: 'wl_output', version: 4
interface: 'zxdg_output_manager_v1', version: 3
```

If any of these are missing, the corresponding `XxxState::new::<Self>(&dh)`
call is missing from `Wayboard::new()`. If all are present but `weston-info`
shows nothing, `delegate_dispatch2!` might be missing or in the wrong place.

---

## Next: Chapter 3 — Winit Backend + Rendering

Block 2 adds a visible window (winit), an Output (screen), and the
render loop. For the first time, you'll see client windows actually
draw inside your compositor.
