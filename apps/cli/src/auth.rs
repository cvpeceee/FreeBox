//! Authentication command handler for the FreeBox CLI.

use crate::AuthCommands;
use anyhow::Result;

/// Handle authentication subcommands (register, login, logout, whoami).
pub async fn handle(cmd: AuthCommands, server: &str) -> Result<()> {
    match cmd {
        AuthCommands::Register { username, email } => {
            println!("TODO: Register at {server} (username={username:?}, email={email:?})");
        }
        AuthCommands::Login { username } => {
            println!("TODO: Login to {server} (username={username:?})");
        }
        AuthCommands::Logout => {
            println!("TODO: Logout");
        }
        AuthCommands::Whoami => {
            println!("TODO: Whoami");
        }
    }
    Ok(())
}
