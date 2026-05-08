//! Messaging command implementation.
use crate::MsgCommands;
use anyhow::Result;

pub async fn handle(cmd: MsgCommands, server: &str) -> Result<()> {
    println!("TODO: Msg command at {server}");
    let _ = cmd;
    Ok(())
}
