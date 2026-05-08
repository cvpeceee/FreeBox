//! Download command implementation.
use anyhow::Result;
use std::path::PathBuf;

pub async fn run(remote: String, output: Option<PathBuf>, server: &str) -> Result<()> {
    println!("TODO: Download {remote} → {output:?} (server={server})");
    Ok(())
}
