//! Plugin management command implementation.
use crate::PluginCommands;
use anyhow::Result;

pub async fn handle(cmd: PluginCommands, server: &str) -> Result<()> {
    println!("TODO: Plugin command at {server}");
    let _ = cmd;
    Ok(())
}
