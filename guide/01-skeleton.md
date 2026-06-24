# Journey to a Compositor — Chapter 1: The Skeleton

A Unix socket that accepts Wayland clients. Nothing else.
No protocol globals, no rendering, no input.
Clients can connect but see zero globals — `weston-info` is blank.

---

## Data Flow

```
Client  ──connect()──▶  Unix socket (wayland-0)
                           │
                           ▼
                       ListeningSocketSource ──generates──▶ client_stream
                           │
                           ▼ (insert_source callback)
                       Wayboard.display_handle.insert_client()
                           │
                           ▼
                       Display<Wayboard>
                           │ client sends messages on the socket
                           ▼
                       display.dispatch_clients() ──calls──▶ (nothing yet)
                           │ (in Block 1: calls handler traits)
                           ▼
                       Loop continues, waiting for more events.
```

---

## Key Types

### Display\<State\>

The Wayland server core. Manages ALL connected clients.

Generic over your state type — every callback gets `&mut State`.
Think of it as "the server database of clients + their objects."

### DisplayHandle

A **non-generic** handle to the Display. Cheap to clone.

**Why we need it:** Display gets MOVED into `Generic` (step 5 below),
so we can't hold `&mut Display` and also use it elsewhere.
DisplayHandle lets us register globals, create seats, etc.,
WITHOUT borrowing the Display itself.

### EventLoop\<State\>

calloop's single-threaded callback-driven event loop.

No async/await, no threads. You insert "event sources" (files,
sockets, timers, signals) and give each a callback. The loop
blocks on poll/epoll, wakes up when an event arrives, runs the
callback with `&mut State`, then goes back to sleep.

### LoopSignal

A handle to stop the event loop from anywhere. Call `.stop()` and
the loop exits cleanly at the next iteration. In Block 2 we'll
wire this to the window close button.

### ListeningSocketSource

Creates a Unix domain socket at `$XDG_RUNTIME_DIR/wayland-N`.

`new_auto()` picks the next free number (wayland-0, wayland-1...).
This is the "display socket" — the thing `$WAYLAND_DISPLAY` points to.

### Generic\<Display\<State\>\>

Wraps a Display into a calloop EventSource so the event loop can
poll it. Without this, the loop wouldn't know when clients send
data.

- `Interest::READ` = "wake me when the client fd is readable"
- `Mode::Level` = "keep waking me while data is available" (vs edge)

### client\_stream (UnixStream)

A connected client. Each `connect()` on the socket produces one.
Wrapped in `insert_client()` which assigns a `ClientId` and stores
per-client data (in Block 1: `ClientState`).

---

## The Display vs State Split

```
Display<Wayboard>  — owns client connections, dispatching machinery.
                      Generic because callbacks need &mut Wayboard.

Wayboard           — YOUR compositor state. Holds protocol states
                      (CompositorState, XdgShellState...), the Space,
                      the Seat, the Shell. What you "own."
```

In Block 0, Wayboard is nearly empty. It grows with every block.

---

## Step-by-Step Walkthrough

### Step 1: Create the event loop

```rust
let mut event_loop: EventLoop<Wayboard> = EventLoop::try_new()?;
```

`EventLoop<Wayboard>` is the heart of the compositor. It's a
single-threaded poll/epoll loop. Every callback you register
gets `&mut Wayboard` — no locks needed.

`try_new()` can fail if another event loop is already running
in this thread (calloop is per-thread singleton).

### Step 2: Create the Wayland Display

```rust
let display: Display<Wayboard> = Display::new()?;
let dh = display.handle();
```

`Display<Wayboard>` manages client connections, routes protocol
messages to handler traits, and owns the global registry
(wl_compositor, wl_seat, etc.).

`.handle()` gives us a `DisplayHandle` — cloneable and non-generic.
We take it NOW because Display will be MOVED into the `Generic`
event source in Step 5. After that move, we can't borrow it.
DisplayHandle gives us permanent access.

### Step 3: Create the listening socket

```rust
let listening_socket = ListeningSocketSource::new_auto()?;
let socket_name = listening_socket.socket_name().to_os_string();
```

Creates a socket at something like `/run/user/1000/wayland-0`.
Clients find it via `WAYLAND_DISPLAY`.

`ListeningSocketSource` is a calloop EventSource — when a client
connects, the callback fires with the client's `UnixStream`.

### Step 4: Accept new client connections

```rust
event_loop.handle().insert_source(listening_socket, move |client_stream, _, state| {
    state.display_handle.insert_client(client_stream, Arc::new(())).unwrap();
})?;
```

`insert_source()` registers the listening socket with the event
loop. Whenever a client calls `connect()`, calloop wakes up and
runs this callback.

The callback receives:
- `client_stream`: the client's UnixStream (connected fd)
- `_`: the ListeningSocketSource (unused here)
- `state`: `&mut Wayboard` (our compositor state)

`display_handle.insert_client()`:
- Assigns a `ClientId` to this connection
- Stores per-client data (the `Arc<()>` — empty placeholder,
  replaced in Block 1 with `ClientState`)
- Starts reading Wayland messages from the stream

### Step 5: Dispatch client messages

```rust
event_loop.handle().insert_source(
    Generic::new(display, Interest::READ, Mode::Level),
    |_, display, state| unsafe {
        display.get_mut().dispatch_clients(state).unwrap();
        Ok(PostAction::Continue)
    },
)?;
```

Display itself needs to be polled. When a connected client sends
Wayland messages, Display needs to read and parse them.

`Generic::new()` wraps Display into a calloop EventSource:
- `Interest::READ` = wake when client fd has data to read
- `Mode::Level` = keep waking while data is available

The callback fires when a client sent data:
- `display.get_mut()` — get `&mut Display` from the Generic wrapper
- `dispatch_clients(state)` — read+parse client messages, call
  handler traits (in later blocks)

**Why unsafe?** `dispatch_clients()` is marked unsafe because it
calls handler trait methods that take `&mut Wayboard`. The callback
ALSO gives us `&mut Wayboard` via `state`. Smithay ensures no
aliasing — the unsafe is just a contract calloop upholds.

`PostAction::Continue` = keep this source registered.

### Step 6: Get loop signal and assemble state

```rust
let loop_signal = event_loop.get_signal();
let mut state = Wayboard { display_handle: dh, loop_signal };
```

`loop_signal` is a handle to stop the event loop from anywhere
(signal handler, window close, IPC). Currently stored but unused.

### Step 7: Tell clients where to find us

```rust
unsafe { std::env::set_var("WAYLAND_DISPLAY", &socket_name); }
```

Wayland clients look at `$WAYLAND_DISPLAY` to find the socket path.
If `WAYLAND_DISPLAY=wayland-0`, they connect to:
`$XDG_RUNTIME_DIR/wayland-0`

Set before spawning any clients.

**Why unsafe?** `set_var` is not thread-safe (could race with other
threads reading env). Since we're single-threaded, this is safe.

### Step 8: Run the event loop

```rust
event_loop.run(None, &mut state, |_| {})?;
```

This blocks forever (or until `loop_signal.stop()` is called).
- `None` = no timeout (wait forever for events)
- `&mut state` = the compositor state, passed to every callback
- `|_| {}` = idle callback (called when no events — we ignore)

Under the hood, calloop does:

```
loop {
    poll(fds, timeout);           // wait for events
    for each ready fd:
        run its callback(&mut state);  // your code!
}
```

All your logic runs inside these callbacks. There is no "main
function" running alongside the loop — the loop IS your program.

---

## Next: Chapter 1 — Mandatory Protocols

In Block 1 (next), we register all mandatory Wayland protocol globals:
`wl_compositor`, `wl_shm`, `xdg_wm_base`, `wl_seat`, `wl_data_device`,
`wl_output`.

After that, clients see a real compositor — they can create surfaces
and toplevels, but nothing renders yet (that's Block 2).
