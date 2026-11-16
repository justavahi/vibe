use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Name of the local config file, kept next to wherever `vibe` is run from.
pub const CONFIG_FILE: &str = "vibe.toml";

/// The on-disk configuration, stored locally as `vibe.toml`.
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
    /// Path to the config file in the current working directory.
    pub fn path() -> PathBuf {
        Path::new(CONFIG_FILE).to_path_buf()
    }

    /// Loads the config from `vibe.toml`, or returns an empty config if it
    /// does not exist yet.
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

    /// Writes the config back to `vibe.toml`.
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
