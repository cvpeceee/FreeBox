//! CLI configuration — loaded from `~/.config/freebox/config.toml` at startup.
//!
//! All fields have defaults so the config file is entirely optional.
//! The `--server` CLI flag always takes priority over the config file value.

use serde::{Deserialize, Serialize};

use crate::session::{config_dir, DEFAULT_SERVER};

/// A named storage source visible via `fbx datasource list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageSource {
    /// Human-readable name (e.g. "cloudflare-r2").
    pub name: String,
    /// Provider type: "s3", "local", "gcs", etc.
    pub provider: String,
    /// Endpoint URL (empty for AWS S3).
    pub endpoint: String,
    /// Bucket / root path.
    pub bucket: String,
    /// Region ("auto" for Cloudflare R2).
    pub region: String,
}

/// CLI configuration persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliConfig {
    /// Default FreeBox server URL.
    /// Overridden by the `--server` flag or `FREEBOX_SERVER` env var.
    pub server: String,

    /// Registered storage sources (populated by `fbx datasource add`).
    #[serde(default)]
    pub sources: Vec<StorageSource>,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            server: DEFAULT_SERVER.to_owned(),
            sources: Vec::new(),
        }
    }
}

impl CliConfig {
    /// Load config from `~/.config/freebox/config.toml`.
    /// Returns [`Default`] if the file does not exist or cannot be parsed.
    pub fn load() -> Self {
        let path = config_dir().join("config.toml");
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        toml::from_str(&raw).unwrap_or_default()
    }

    /// Save the current config to disk.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = config_dir().join("config.toml");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let toml = toml::to_string_pretty(self)?;
        std::fs::write(&path, toml)?;
        Ok(())
    }
}
