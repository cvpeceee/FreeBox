//! Remove command implementation.

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::{
    auth::api_url,
    client::send_with_refresh,
    session::{effective_server, Session},
};

pub async fn run(remote: String, permanent: bool, server: &str) -> Result<()> {
    if permanent {
        anyhow::bail!(
            "--permanent hard delete is not yet supported; \
             files in trash are purged automatically after 30 days"
        );
    }

    let file_id = Uuid::parse_str(&remote)
        .context("remove expects a file UUID — use `fbx ls --long` to find the file ID")?;

    let mut session = Session::load()?;
    let server = effective_server(server, &session).to_owned();
    let http = reqwest::Client::new();
    let url = api_url(&server, &format!("/api/v1/files/{file_id}"));

    send_with_refresh(&mut session, |token| http.delete(&url).bearer_auth(token))
        .await?
        .error_for_status()?;

    println!("Moved {file_id} to trash (recoverable for 30 days).");
    Ok(())
}

#[cfg(test)]
mod tests;
