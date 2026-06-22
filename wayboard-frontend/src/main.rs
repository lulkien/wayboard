mod config;
mod winit;

use std::path::PathBuf;

use clap::Parser;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use wayboard_core::{NullShell, Shell, Wayboard};

#[derive(Parser)]
#[command(name = "wayboard", about = "A modular Wayland compositor")]
struct Cli {
    /// Path to config file
    #[arg(short = 'c', long = "config")]
    config: Option<PathBuf>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let cli = Cli::parse();
    let config = config::Config::load(cli.config.as_ref());

    let shell: Box<dyn Shell> = match config.shell.name.as_str() {
        "default" => Box::new(shell_default::DefaultShell::new()),
        _ => Box::new(NullShell),
    };

    let mut event_loop: EventLoop<Wayboard> = EventLoop::try_new()?;
    let display: Display<Wayboard> = Display::new()?;

    let mut wayboard = Wayboard::new(
        &mut event_loop,
        display,
        shell,
        std::env::var("RUST_LOG").unwrap_or_default(),
    );

    crate::winit::init_winit(&mut event_loop, &mut wayboard)?;

    unsafe { std::env::set_var("WAYLAND_DISPLAY", &wayboard.socket_name) };

    // Run startup commands from config
    for cmd in &config.startup {
        let mut child = std::process::Command::new(&cmd.command);
        child.args(&cmd.args);
        child.spawn().ok();
    }

    event_loop.run(None, &mut wayboard, move |_| {})?;

    Ok(())
}

fn init_logging() {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    } else {
        tracing_subscriber::fmt().init();
    }
}
