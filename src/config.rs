use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Name of the config file inside the config directory.
pub const CONFIG_FILE: &str = "config.toml";

/// The on-disk configuration, stored in `~/.config/vibe/config.toml`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// OpenRouter API key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// The model id the user picked, e.g. `openai/gpt-4o`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl Config {
    /// The config directory: `~/.config/vibe`.
    ///
    /// Respects `XDG_CONFIG_HOME` when set, otherwise falls back to
    /// `$HOME/.config`.
    pub fn dir() -> PathBuf {
        let base = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .unwrap_or_else(|| PathBuf::from(".config"));

        base.join("vibe")
    }

    /// Creates the config directory (`~/.config/vibe`) if it does not exist yet.
    pub fn ensure_dir() -> io::Result<()> {
        fs::create_dir_all(Self::dir())
    }

    /// Path to the config file: `~/.config/vibe/config.toml`.
    pub fn path() -> PathBuf {
        Self::dir().join(CONFIG_FILE)
    }

    /// Loads the config, or returns an empty config if it does not exist yet.
    pub fn load() -> io::Result<Self> {
        let path = Self::path();

        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(&path)?;

        toml::from_str(&contents).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to parse {}: {e}", path.display()),
            )
        })
    }

    /// Writes the config back to `~/.config/vibe/config.toml`.
    pub fn save(&self) -> io::Result<()> {
        let path = Self::path();

        let contents = toml::to_string_pretty(self).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to serialize config: {e}"),
            )
        })?;

        fs::write(&path, contents)
    }

    /// Returns the API key, or an error telling the user how to set one.
    pub fn require_api_key(&self) -> io::Result<&str> {
        self.api_key.as_deref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "no API key set. Run `vibe api-key \"<your-key>\"` first",
            )
        })
    }

    /// Returns the selected model, or an error telling the user how to pick one.
    pub fn require_model(&self) -> io::Result<&str> {
        self.model.as_deref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "no model selected. Run `vibe model` first",
            )
        })
    }
}
