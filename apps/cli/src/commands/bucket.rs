//! `fbx bucket` — list and create storage buckets.

use anyhow::Result;
use clap::Subcommand;
use serde::Deserialize;

use crate::{
    auth::api_url,
    client::send_with_refresh,
    session::{effective_server, Session},
};

#[derive(Subcommand, Debug)]
pub enum BucketCommand {
    /// List all buckets available with the configured credentials.
    List,
    /// Create a new bucket.
    Create {
        /// Name of the bucket to create.
        name: String,
    },
}

pub async fn handle(cmd: BucketCommand, server: &str) -> Result<()> {
    let mut session = Session::load()?;
    let server = effective_server(server, &session).to_owned();
    match cmd {
        BucketCommand::List => list(&server, &mut session).await,
        BucketCommand::Create { name } => create(&name, &server, &mut session).await,
    }
}

// ---------------------------------------------------------------------------
// Implementations
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct BucketItem {
    name: String,
    creation_date: Option<String>,
}

#[derive(Deserialize)]
struct ListBucketsResponse {
    buckets: Vec<BucketItem>,
}

async fn list(server: &str, session: &mut Session) -> Result<()> {
    let http = reqwest::Client::new();
    let url = api_url(server, "/api/v1/storage/buckets");

    let resp = send_with_refresh(session, |token| http.get(&url).bearer_auth(token))
        .await?
        .error_for_status()?
        .json::<ListBucketsResponse>()
        .await?;

    if resp.buckets.is_empty() {
        println!("No buckets found.");
        return Ok(());
    }

    println!("{:<45} {}", "BUCKET NAME", "CREATED");
    println!("{}", "-".repeat(70));
    for b in &resp.buckets {
        println!(
            "{:<45} {}",
            b.name,
            b.creation_date.as_deref().unwrap_or("—")
        );
    }
    Ok(())
}

async fn create(name: &str, server: &str, session: &mut Session) -> Result<()> {
    let http = reqwest::Client::new();
    let url = api_url(server, "/api/v1/storage/buckets");

    let resp = send_with_refresh(session, |token| {
        http.post(&url)
            .bearer_auth(token)
            .json(&serde_json::json!({ "name": name }))
    })
    .await?;

    let status = resp.status();
    if status.is_success() {
        println!("Bucket '{}' created successfully.", name);
    } else {
        let err: serde_json::Value = resp.json().await.unwrap_or_default();
        anyhow::bail!(
            "Failed to create bucket: {}",
            err["message"].as_str().unwrap_or("unknown error")
        );
    }
    Ok(())
}
