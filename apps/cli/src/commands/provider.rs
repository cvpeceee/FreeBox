//! Provider management command implementation.
use crate::ProviderCommands;
use anyhow::Result;

pub async fn handle(cmd: ProviderCommands, server: &str) -> Result<()> {
    println!("TODO: Provider command at {server}");
    let _ = cmd;
    Ok(())
}
