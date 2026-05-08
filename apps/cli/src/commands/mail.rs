//! Mail command implementation.
use crate::MailCommands;
use anyhow::Result;

pub async fn handle(cmd: MailCommands, server: &str) -> Result<()> {
    println!("TODO: Mail command at {server}");
    let _ = cmd;
    Ok(())
}
