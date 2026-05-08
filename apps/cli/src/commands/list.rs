//! List command implementation.
use anyhow::Result;

pub async fn run(path: String, long: bool, server: &str) -> Result<()> {
    println!("TODO: List {path} (long={long}, server={server})");
    Ok(())
}
