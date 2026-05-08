//! Upload command implementation.
use anyhow::Result;
use std::path::PathBuf;

pub async fn run(
    files: Vec<PathBuf>,
    destination: String,
    parallelism: u8,
    server: &str,
) -> Result<()> {
    println!("TODO: Upload {files:?} → {destination} (parallelism={parallelism}, server={server})");
    Ok(())
}
