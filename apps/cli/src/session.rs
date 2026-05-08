//! Session management for the FreeBox CLI.
//!
//! The CLI stores the current server session in a small JSON file under the
//! user config directory. This keeps the first functional client simple and
//! testable; the storage boundary can later be swapped for OS keychain-backed
//! secrets without changing command behavior.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const DEFAULT_SERVER: &str = "https://freebox.io";

/// An active session with a FreeBox server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    pub server: String,
    pub username: String,
    pub user_id: Uuid,
}

impl Session {
    pub fn save(&self) -> Result<()> {
        save_session_to_path(self, &session_path())
    }

    pub fn load() -> Result<Self> {
        load_session_from_path(&session_path())
    }

    pub fn clear() -> Result<()> {
        clear_session_at_path(&session_path())
    }
}

pub(crate) fn effective_server<'a>(server_arg: &'a str, session: &'a Session) -> &'a str {
    if server_arg == DEFAULT_SERVER {
        &session.server
    } else {
        server_arg
    }
}

pub(crate) fn session_path() -> PathBuf {
    config_dir().join("session.json")
}

pub(crate) fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("FREEBOX_CONFIG_DIR") {
        return PathBuf::from(dir);
    }

    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata).join("FreeBox");
    }

    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config").join("freebox");
    }

    if let Ok(profile) = std::env::var("USERPROFILE") {
        return PathBuf::from(profile).join(".freebox");
    }

    PathBuf::from(".freebox")
}

pub(crate) fn save_session_to_path(session: &Session, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }

    let bytes = serde_json::to_vec_pretty(session)?;
    std::fs::write(path, bytes)
        .with_context(|| format!("failed to write session file {}", path.display()))
}

pub(crate) fn load_session_from_path(path: &Path) -> Result<Session> {
    let bytes = std::fs::read(path).with_context(|| {
        format!(
            "not logged in; run `fbx auth login` first (missing {})",
            path.display()
        )
    })?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse session file {}", path.display()))
}

pub(crate) fn clear_session_at_path(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("failed to remove {}", path.display())),
    }
}

#[cfg(test)]
mod tests;
