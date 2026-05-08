//! Sync command implementation.
use crate::SyncDirection;
use anyhow::Result;
use std::path::PathBuf;

pub async fn run(
    local: PathBuf,
    remote: String,
    watch: bool,
    direction: SyncDirection,
    server: &str,
) -> Result<()> {
    println!("TODO: Sync {local:?} ↔ {remote} (watch={watch}, server={server})");
    let _ = direction;
    Ok(())
}
