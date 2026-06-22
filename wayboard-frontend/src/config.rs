use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub shell: ShellConfig,

    /// Commands to run when the window manager starts.
    /// Example:
    ///   [[startup]]
    ///   command = "waybar"
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
        Self {
            name: "default".into(),
        }
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
            let home = std::env::var("HOME").unwrap_or_else(|_| "/home/user".into());
            PathBuf::from(home).join(".config")
        });

    base.join("wayboard").join("config.toml")
}
