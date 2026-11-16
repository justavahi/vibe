use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Name of the config file inside the config directory.
pub const CONFIG_FILE: &str = "config.toml";

/// An LLM provider that vibe can talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    /// [OpenRouter](https://openrouter.ai).
    OpenRouter,
    /// [Venice AI](https://venice.ai).
    Venice,
}

impl Vendor {
    /// Every vendor, in the order shown by `vibe api-key choose`.
    pub const ALL: [Vendor; 2] = [Vendor::OpenRouter, Vendor::Venice];

    /// The lowercase identifier stored in the config file.
    pub fn as_str(self) -> &'static str {
        match self {
            Vendor::OpenRouter => "openrouter",
            Vendor::Venice => "venice",
        }
    }

    /// The `vibe api-key` flag that targets this vendor.
    pub fn flag(self) -> &'static str {
        match self {
            Vendor::OpenRouter => "-o",
            Vendor::Venice => "-v",
        }
    }

    /// Parses a vendor identifier, case-insensitively.
    pub fn parse(value: &str) -> Option<Vendor> {
        match value.trim().to_ascii_lowercase().as_str() {
            "openrouter" => Some(Vendor::OpenRouter),
            "venice" => Some(Vendor::Venice),
            _ => None,
        }
    }
}

impl std::fmt::Display for Vendor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The on-disk configuration, stored in `~/.config/vibe/config.toml`.
///
/// The file has two sections:
///
/// ```toml
/// [API]
/// vendor = "openrouter"
/// openrouter_api_key = "sk-or-..."
/// venice_api_key = "..."
///
/// [LLM]
/// model = "deepseek/deepseek-v4.1-flash"
/// ```
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// The `[API]` section: provider choice and API keys.
    #[serde(rename = "API", default, skip_serializing_if = "ApiSection::is_empty")]
    pub api: ApiSection,

    /// The `[LLM]` section: the selected model.
    #[serde(rename = "LLM", default, skip_serializing_if = "LlmSection::is_empty")]
    pub llm: LlmSection,

    /// Legacy top-level `api_key`, migrated into `[API]` on load.
    #[serde(default, skip_serializing)]
    pub api_key: Option<String>,

    /// Legacy top-level `model`, migrated into `[LLM]` on load.
    #[serde(default, skip_serializing)]
    pub model: Option<String>,
}

/// The `[API]` section of the config file.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ApiSection {
    /// The selected provider, e.g. `"openrouter"` or `"venice"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,

    /// OpenRouter API key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openrouter_api_key: Option<String>,

    /// Venice AI API key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venice_api_key: Option<String>,
}

impl ApiSection {
    /// Whether every field is unset.
    fn is_empty(&self) -> bool {
        self.vendor.is_none()
            && self.openrouter_api_key.is_none()
            && self.venice_api_key.is_none()
    }
}

/// The `[LLM]` section of the config file.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LlmSection {
    /// The model id the user picked, e.g. `openai/gpt-4o`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl LlmSection {
    /// Whether the model is unset.
    fn is_empty(&self) -> bool {
        self.model.is_none()
    }
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
    ///
    /// Legacy flat files (with top-level `api_key` / `model`) are migrated into
    /// the sectioned layout and rewritten.
    pub fn load() -> io::Result<Self> {
        let path = Self::path();

        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(&path)?;

        let mut config: Self = toml::from_str(&contents).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to parse {}: {e}", path.display()),
            )
        })?;

        let migrated = config.migrate_legacy();

        if migrated {
            // Best effort: if rewriting fails, the in-memory config is still usable.
            let _ = config.save();
        }

        Ok(config)
    }

    /// Moves legacy top-level fields into their sections. Returns `true` if
    /// anything changed.
    fn migrate_legacy(&mut self) -> bool {
        let mut changed = false;

        if self.api.openrouter_api_key.is_none()
            && let Some(key) = self.api_key.take()
        {
            self.api.openrouter_api_key = Some(key);
            changed = true;
        }

        if self.llm.model.is_none()
            && let Some(model) = self.model.take()
        {
            self.llm.model = Some(model);
            changed = true;
        }

        changed
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

    /// The selected vendor.
    ///
    /// Falls back to [`Vendor::OpenRouter`] when the field is missing or holds
    /// an unknown value.
    pub fn vendor(&self) -> Vendor {
        self.api
            .vendor
            .as_deref()
            .and_then(Vendor::parse)
            .unwrap_or(Vendor::OpenRouter)
    }

    /// Whether the stored `vendor` field is present and valid.
    pub fn has_valid_vendor(&self) -> bool {
        self.api
            .vendor
            .as_deref()
            .and_then(Vendor::parse)
            .is_some()
    }

    /// The API key stored for `vendor`, if any.
    pub fn api_key_for(&self, vendor: Vendor) -> Option<&str> {
        match vendor {
            Vendor::OpenRouter => self.api.openrouter_api_key.as_deref(),
            Vendor::Venice => self.api.venice_api_key.as_deref(),
        }
    }

    /// Stores `key` for `vendor`.
    pub fn set_api_key_for(&mut self, vendor: Vendor, key: &str) {
        let key = key.trim().to_string();
        match vendor {
            Vendor::OpenRouter => self.api.openrouter_api_key = Some(key),
            Vendor::Venice => self.api.venice_api_key = Some(key),
        }
    }

    /// Sets the selected vendor.
    pub fn set_vendor(&mut self, vendor: Vendor) {
        self.api.vendor = Some(vendor.as_str().to_string());
    }

    /// Returns the API key for the selected vendor, or an error telling the
    /// user how to set one.
    pub fn require_api_key(&self) -> io::Result<&str> {
        let vendor = self.vendor();

        self.api_key_for(vendor).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "no {vendor} API key set. Run `vibe api-key {} \"<key>\"` first",
                    vendor.flag()
                ),
            )
        })
    }

    /// Returns the selected model, or an error telling the user how to pick one.
    pub fn require_model(&self) -> io::Result<&str> {
        self.llm.model.as_deref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "no model selected. Run `vibe model` first",
            )
        })
    }
}
