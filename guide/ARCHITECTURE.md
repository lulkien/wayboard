================================================================================
WAYBOARD — PLUGGABLE SHELL ARCHITECTURE
================================================================================

The previous `.so` plugin design was wrong for a Rust compositor. Smithay's
type system is generic over the compositor state — you can't swap trait
implementations at runtime via FFI without throwing away the type system.

The RIGHT question: what can be pluggable WITHOUT breaking Smithay's design?

Answer: the Shell. Everything the shell does — window placement, keyboard
shortcuts, layout, bar integration — is POLICY. The core provides MECHANISM.
This separation works because policies don't need to change protocol
dispatch; they just need hooks called at the right moments.

================================================================================
THE BOUNDARY: CORE vs SHELL
================================================================================

┌─────────────────────────────────────────────────────────────────┐
│                        WAYBOARD BINARY                          │
│                                                                 │
│  ┌──────────────────────┐    ┌──────────────────────────────┐   │
│  │        CORE           │    │          SHELL               │   │
│  │  (always compiled in) │    │  (selected at build or       │   │
│  │                       │    │   runtime, one at a time)    │   │
│  │  • Event loop         │    │                              │   │
│  │  • Client dispatch    │    │  • Window placement policy   │   │
│  │  • Protocol states:   │◄───│  • Keyboard shortcuts        │   │
│  │    compositor         │    │  • Layout algorithm          │   │
│  │    xdg-shell          │    │  • Bar/panel management      │   │
│  │    shm                │    │  • Workspace management      │   │
│  │    seat               │    │  • Move/resize behavior      │   │
│  │    data-device        │    │  • Focus policy              │   │
│  │    output             │    │                              │   │
│  │  • Renderer            │    │                              │   │
│  │  • Space               │    │                              │   │
│  │  • Input → Seat        │    │                              │   │
│  │  • Layer shell         │    │                              │   │
│  └──────────────────────┘    └──────────────────────────────┘   │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘

CORE = what every Wayland compositor MUST have to be protocol-compliant.
      Without these, clients can't connect, draw, or receive input.

SHELL = how the compositor chooses to arrange and control windows.
       Without it, windows would stack at (0,0) and accumulate forever.
       You could ship a null shell, a tiling shell, a floating shell —
       the core doesn't care.

================================================================================
WHAT GOES IN THE CORE (non-negotiable)
================================================================================

These are mandatory Wayland protocols. Every compositor must implement them,
or clients literally won't work. There is no benefit to making them
"pluggable" because they would always be loaded anyway.

┌────────────────┬──────────────────────────────────────────────────┐
│ Protocol       │ Purpose                                          │
├────────────────┼──────────────────────────────────────────────────┤
│ wl_compositor  │ Surface creation. Without this, no client can    │
│ wl_surface     │ create a drawing surface.                        │
├────────────────┼──────────────────────────────────────────────────┤
│ wl_shm         │ Shared memory buffers. The fallback for all      │
│                │ clients that don't use dmabuf. Mandatory.        │
├────────────────┼──────────────────────────────────────────────────┤
│ xdg_wm_base    │ XDG Shell. Every modern Wayland client uses this │
│ xdg_toplevel   │ for application windows. Mandatory.              │
│ xdg_popup      │                                                  │
├────────────────┼──────────────────────────────────────────────────┤
│ wl_seat        │ Input. Keyboard, pointer, touch. Mandatory.      │
├────────────────┼──────────────────────────────────────────────────┤
│ wl_data_device │ Clipboard + drag-and-drop. Mandatory.            │
├────────────────┼──────────────────────────────────────────────────┤
│ wl_output      │ Screen information. Mandatory.                   │
│ xdg_output     │                                                  │
└────────────────┴──────────────────────────────────────────────────┘

These live in the core as Smithay trait implementations on a CoreState
struct. They are compiled into the binary. Period.

================================================================================
WHAT CAN BE OPTIONAL (Cargo features)
================================================================================

These protocols are useful but not required for basic windowing.
They can be compiled in or out via Cargo features.

┌──────────────────────┬──────────────────────────────────────────┐
│ Feature flag         │ Protocol / Component                     │
├──────────────────────┼──────────────────────────────────────────┤
│ layer-shell          │ wlr_layer_shell (bars, wallpapers,       │
│                      │ panels, overlays)                        │
│ screencopy           │ wlr_screencopy (screenshots)             │
│ gamma-control        │ wlr_gamma_control (night light)          │
│ xdg-decoration       │ Server-side decorations negotiation      │
│ input-method         │ On-screen keyboard support               │
│ idle-inhibit         │ Inhibit screen blanking (video players)  │
│ session-lock         │ Screen lock protocol                     │
│ virtual-pointer      │ Synthetic input (xdotool equivalent)     │
└──────────────────────┴──────────────────────────────────────────┘

They work like this:

    // In Cargo.toml
    [features]
    default = ["layer-shell"]
    layer-shell = []
    screencopy = []

    // In the core:
    #[cfg(feature = "layer-shell")]
    let layer_shell_state = LayerShellState::new::<Wayboard>(&dh);

    #[cfg(feature = "layer-shell")]
    impl LayerShellHandler for Wayboard { ... }

    // If the feature is off: the state and handler don't exist.
    // Clients that try to bind wlr_layer_shell get a protocol error
    // ("global not found"). Perfectly valid — they just won't have bars.

================================================================================
THE SHELL TRAIT — THE PLUGGABLE BOUNDARY
================================================================================

This is where modularity actually works. The shell is a Rust trait with
clear hook points. Different shell implementations are different structs
that implement the trait. Selected at startup, compiled into the same
binary.

pub trait Shell {
    /// A client created a new toplevel. Where should it go?
    /// The core has already created the Window and attached it to the
    /// xdg_toplevel. The shell decides its position and size.
    fn new_window(&mut self, ctx: &mut ShellContext, window: Window);

    /// A toplevel was destroyed. Clean up any shell state.
    fn destroy_window(&mut self, _ctx: &mut ShellContext, _window: &Window) {}

    /// Called on every frame tick. Used for:
    ///   - Recalculating layouts after a window resize
    ///   - Animations (workspace transitions, window open/close)
    ///   - Updating bar content (clock, workspace indicators)
    fn tick(&mut self, _ctx: &mut ShellContext) {}

    /// A key was pressed. Return true if the shell consumed it
    /// (global shortcut like Super+Enter). Return false if it
    /// should be forwarded to the focused client.
    ///
    /// Default: forward everything. This means no compositor
    /// shortcuts — the null shell behavior.
    fn handle_key(
        &mut self,
        _ctx: &mut ShellContext,
        _keycode: u32,
        _modifiers: ModifiersState,
        _pressed: bool,
    ) -> bool {
        false // forward to client
    }

    /// The shell's name (for debugging/logging/config selection).
    fn name(&self) -> &'static str;

    /// If the shell wants to render a bar/panel, return the bar config.
    /// The core uses this to create a layer-shell surface for the bar.
    /// Return None for no bar.
    fn bar_config(&self) -> Option<BarConfig> { None }
}

pub struct ShellContext<'a> {
    /// The Space: read/write access to window positions and Z-order.
    /// This is the primary interface for the shell.
    pub space: &'a mut Space<Window>,

    /// The seat: for keyboard focus changes, pointer grab initiation.
    pub seat: &'a mut Seat<Wayboard>,

    /// For sending configure events to clients.
    pub display_handle: &'a mut DisplayHandle,

    /// Output geometries (for layout calculations).
    pub outputs: &'a [OutputGeometry],

    /// Layer surface state (for bar management).
    /// Only available with feature = "layer-shell".
    #[cfg(feature = "layer-shell")]
    pub layer_shell: Option<&'a LayerShellState>,
}

pub struct OutputGeometry {
    pub name: String,
    pub x: i32, pub y: i32,
    pub width: i32, pub height: i32,
    pub refresh: i32,
    pub scale: f64,
}

/// Bar configuration returned by the shell.
/// The core creates a layer-shell surface at Shell::bar_config().layer
/// with the given size and anchor, and calls Shell::render_bar() each
/// frame to populate it.
pub struct BarConfig {
    pub layer: Layer,          // Top or Bottom
    pub height: i32,           // pixels
    pub anchor: Anchor,        // which edges
    pub exclusive_zone: i32,   // space to reserve (pixels)
}

pub enum Layer { Background, Bottom, Top, Overlay }

================================================================================
HOW THE CORE CALLS THE SHELL
================================================================================

The core is the coordinator. It owns the event loop and calls shell hooks
at specific moments:

impl Wayboard {
    fn run(&mut self) {
        loop {
            // ── Dispatch client messages ──
            self.display.dispatch_clients(self)?;

            // ── Dispatch input events ──
            // (input arrives from the backend, gets translated to Seat events)

            // ── Tick the shell ──
            // Called once per logical frame (not per vblank — driven by
            // new events arriving, not display refresh).
            {
                let ctx = ShellContext {
                    space: &mut self.space,
                    seat: &mut self.seat,
                    display_handle: &mut self.display_handle,
                    outputs: &self.output_geometries,
                    #[cfg(feature = "layer-shell")]
                    layer_shell: self.layer_shell_state.as_ref(),
                };
                self.shell.tick(&mut ctx);
            }

            // ── Render ──
            // The backend composites the Space onto each output.
            // The shell's bar surface (if any) is just another element
            // in the Space — no special rendering path needed.
            self.backend.render(
                &self.space,
                &self.output_geometries,
                &mut self.damage_tracker,
            );

            // ── Flush ──
            self.display_handle.flush_clients()?;
        }
    }
}

================================================================================
EXAMPLE: TILING SHELL WITH BAR
================================================================================

struct TilingShell {
    /// BSP tree: the tiling layout
    tree: BspTree,

    /// Keybindings: shortcuts → actions
    bindings: KeybindingMap,

    /// Workspaces: each is a separate BspTree + Space
    workspaces: Vec<BspTree>,
    active_workspace: usize,

    /// Bar surface (managed by core via layer-shell, drawn by shell)
    bar: Option<BarState>,
}

struct BarState {
    /// The layer-shell surface for the bar
    surface: LayerSurface,
    /// Render state: cached text, workspace indicators
    /// (drawn as SHM buffers or GL textures by the shell)
}

impl Shell for TilingShell {
    fn new_window(&mut self, ctx: &mut ShellContext, window: Window) {
        // Insert into the BSP tree at the focused node
        // If focused node is a window, split it
        // Compute geometry from tree → set window position + size
        // Send configure to client
        let focused_id = self.tree.focused_id();

        if self.tree.is_leaf(focused_id) {
            self.tree.split(focused_id, SplitDirection::Vertical);
        }

        self.tree.insert(window, focused_id);
        self.apply_layout(ctx);
    }

    fn tick(&mut self, ctx: &mut ShellContext) {
        // Recalculate layout (windows might have resized)
        self.apply_layout(ctx);

        // Update bar content
        if let Some(ref mut bar) = self.bar {
            bar.redraw_workspace_indicators(self.active_workspace, self.workspaces.len());
            bar.redraw_clock();
        }
    }

    fn handle_key(&mut self, ctx: &mut ShellContext, keycode: u32, mods: ModifiersState, pressed: bool) -> bool {
        if !pressed { return false; }

        if let Some(action) = self.bindings.lookup(keycode, mods) {
            match action {
                Action::Focus(dir) => {
                    self.tree.focus_direction(dir);
                    self.apply_layout(ctx);
                }
                Action::Split(dir) => {
                    self.tree.split(self.tree.focused_id(), dir);
                }
                Action::Close => {
                    if let Some(window) = self.tree.remove_focused() {
                        window.toplevel().unwrap().send_close();
                    }
                    self.apply_layout(ctx);
                }
                Action::Workspace(n) => {
                    self.switch_workspace(n, ctx);
                }
                Action::Spawn(cmd) => {
                    // Launch a Wayland client
                    std::process::Command::new(cmd).spawn().ok();
                }
                Action::ToggleFloating => {
                    self.tree.toggle_floating(ctx.space);
                }
            }
            return true; // consumed
        }
        false // forward to client
    }

    fn bar_config(&self) -> Option<BarConfig> {
        Some(BarConfig {
            layer: Layer::Top,
            height: 30,
            anchor: Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
            exclusive_zone: 30, // tiled windows don't overlap the bar
        })
    }

    fn name(&self) -> &'static str { "tiling" }
}

impl TilingShell {
    fn apply_layout(&self, ctx: &mut ShellContext) {
        // Walk the BSP tree
        // For each leaf: compute its geometry from the tree split ratios
        // Subtract bar height from the available area
        // Call ctx.space.map_element(window, position, false)

        let mut usable_area = self.output_rect(ctx);

        // Reserve space for the bar
        if let Some(ref cfg) = self.bar_config() {
            usable_area.y += cfg.height;
            usable_area.height -= cfg.height;
        }

        for (node_id, geometry) in self.tree.layout(&usable_area) {
            if let Some(window) = self.tree.window_at(node_id) {
                ctx.space.map_element(
                    window.clone(),
                    (geometry.x, geometry.y).into(),
                    false, // don't send configure here — send it once after all positions
                );
            }
        }

        // Send configure events to all windows (so they know their new size)
        for window in ctx.space.elements() {
            window.toplevel().unwrap().send_pending_configure();
        }
    }
}

================================================================================
WHERE THE BAR LIVES
================================================================================

The bar is NOT a separate binary (like waybar). It's NOT an external client.
It's a layer-shell surface owned by the compositor itself, rendered by the
shell into the compositor's own Space.

This is important because:
  1. No IPC between bar and compositor — direct access to state
  2. Workspace indicators are always in sync (no polling)
  3. Clock updates are cheap (no protocol roundtrips)
  4. One less process to manage

The flow:
  Shell::bar_config() → core creates a wlr_layer_surface at Top layer
                      → core maps it into the Space
  Shell::tick()       → shell draws text/indicators into the bar surface
                      → core renders it as part of normal compositing

If you DO want an external bar (like waybar), that works too —
layer-shell is a protocol that external clients can use. But the
compositor's own shell can also use it internally for its own bar.
They coexist — the compositor bar at one anchor, waybar at another.

================================================================================
WORKSPACE LAYOUT
================================================================================

wayboard/
├── wayboard-core/             Everything in one crate (no workspace needed)
│   ├── main.rs
│   ├── state.rs               Wayboard struct (CoreState)
│   ├── core/
│   │   ├── mod.rs
│   │   ├── handlers/          Mandatory protocol implementations
│   │   │   ├── mod.rs         delegate_dispatch!
│   │   │   ├── compositor.rs
│   │   │   ├── xdg_shell.rs   Calls shell.new_window(), shell.handle_key()
│   │   │   ├── seat.rs
│   │   │   └── output.rs
│   │   ├── layer_shell.rs     #[cfg(feature = "layer-shell")] only
│   │   ├── screencopy.rs      #[cfg(feature = "screencopy")] only
│   │   └── ...
│   ├── backend/
│   │   ├── mod.rs             Backend trait
│   │   ├── winit.rs           Dev backend (host window)
│   │   └── drm.rs             Production backend (DRM/KMS + libinput)
│   ├── shell/
│   │   ├── mod.rs             Shell trait definition
│   │   ├── tiling.rs          BSP tiling with bar
│   │   ├── floating.rs        Stacking WM
│   │   └── null.rs            No-op shell (windows at 0,0, no shortcuts)
│   ├── grabs/
│   │   ├── mod.rs
│   │   ├── move_grab.rs
│   │   └── resize_grab.rs
│   └── config.rs              Keybinding parsing, config file loader

================================================================================
SELECTING THE SHELL
================================================================================

Two approaches, both valid:

APPROACH A: Cargo features (compile-time selection)

  [features]
  shell-tiling = []
  shell-floating = []
  default = ["shell-tiling"]

  // In main.rs:
  let shell: Box<dyn Shell> = {
      #[cfg(feature = "shell-tiling")]
      { Box::new(TilingShell::new(&config)) }

      #[cfg(feature = "shell-floating")]
      { Box::new(FloatingShell::new(&config)) }
  };

  Pro: dead code elimination — unused shells aren't compiled
  Con: changing shell requires recompilation

APPROACH B: Config file (runtime selection)

  # ~/.config/wayboard/config.toml
  [shell]
  type = "tiling"   # or "floating"

  // In main.rs:
  let shell: Box<dyn Shell> = match config.shell_type.as_str() {
      "tiling" => Box::new(TilingShell::new(&config)),
      "floating" => Box::new(FloatingShell::new(&config)),
      _ => Box::new(NullShell),
  };

  Pro: change shell without recompilation
  Con: all shells compiled into binary (larger binary, still small in practice)

Recommendation: Approach B by default (the binary is small anyway).
A Wayland compositor binary is ~5MB; adding a few shell impls is
negligible.

================================================================================
WHAT THIS DESIGN ALLOWS
================================================================================

1. A tiling shell with a built-in bar → one binary, zero external deps
2. A floating shell with window snapping → same binary, different config
3. A null shell for headless testing → no layout, just pass-through
4. Optional layer-shell support → bar works if compiled in, missing global if not
5. Optional screencopy → grim works if compiled in, "protocol not found" if not

================================================================================
WHAT THIS DESIGN DOES NOT ALLOW (and why that's fine)
================================================================================

- Hot-reloading a new shell at runtime without restarting the compositor
  → Minor: session logout/login or compositor restart is fast
  → If you really want this: embed Lua/Rhai for policy, keep Rust for engine

- Third-party .so shells loaded from arbitrary paths
  → The security model is different: a shell has full access to the
    compositor state. Loading arbitrary .so files is equivalent to
    running arbitrary code as the compositor user. If you trust the
    shell that much, you trust it enough to recompile.

================================================================================
IMPLEMENTATION ORDER
================================================================================

PHASE 1: Extract the Shell trait from current code
  Define Shell trait with new_window, handle_key, tick, name
  Move current xdg-shell behavior into NullShell (just place at 0,0)
  Verify: compositor still works, windows appear

PHASE 2: Add FloatingShell
  Smart window placement (cascade, center)
  Click-to-focus (already working)
  Window snapping (drag to edge → half-screen)
  Minimize/maximize

PHASE 3: Add TilingShell
  BSP tree implementation
  Split, focus, resize operations
  Keybinding system (config file → action dispatch)
  Workspace switching

PHASE 4: Add bar
  Enable layer-shell feature
  Render bar as internal layer surface
  Workspace indicators, clock, title display

PHASE 5: Production backend
  Replace winit backend with DRM/KMS + libinput
  TTY switching (Ctrl+Alt+F1..F7)
  Session management (logind integration)
