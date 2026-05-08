//! Remove command implementation.
use anyhow::Result;

pub async fn run(remote: String, permanent: bool, server: &str) -> Result<()> {
    println!("TODO: Remove {remote} (permanent={permanent}, server={server})");
    Ok(())
}
