//! CLI configuration — server URL, credentials path, defaults.

/// CLI configuration loaded from `~/.config/freebox/config.toml`.
pub struct CliConfig {
    pub server: String,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            server: "https://freebox.io".into(),
        }
    }
}
