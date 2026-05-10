//! List command implementation.

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::api_url,
    client::send_with_refresh,
    session::{effective_server, Session},
};

#[derive(Deserialize)]
struct FileMeta {
    file_id: Uuid,
    encrypted_name: String,
    size_bytes: u64,
    #[allow(dead_code)]
    total_chunks: u32,
    content_hash: String,
    created_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct FileListResponse {
    files: Vec<FileMeta>,
}

pub async fn run(path: String, long: bool, server: &str) -> Result<()> {
    let mut session = Session::load()?;
    let server = effective_server(server, &session).to_owned();
    let http = reqwest::Client::new();
    let url = api_url(&server, "/api/v1/files");

    let resp = send_with_refresh(&mut session, |token| http.get(&url).bearer_auth(token))
        .await?
        .error_for_status()?
        .json::<FileListResponse>()
        .await?;

    if resp.files.is_empty() {
        println!("No files.");
        return Ok(());
    }

    let prefix = path.trim_start_matches("remote://").trim_end_matches('/');

    let mut printed = 0usize;
    for file in &resp.files {
        let name = decode_name(&file.encrypted_name);
        if !prefix.is_empty() && !name.starts_with(prefix) {
            continue;
        }
        if long {
            println!(
                "{file_id}  {size:>10}  {date}  {hash:.12}  {name}",
                file_id = file.file_id,
                size = format_size(file.size_bytes),
                date = file.created_at.format("%Y-%m-%d %H:%M"),
                hash = file.content_hash,
            );
        } else {
            println!("{name}");
        }
        printed += 1;
    }

    if printed == 0 {
        println!("No files matching '{path}'.");
    }

    Ok(())
}

fn decode_name(encrypted_name: &str) -> String {
    STANDARD
        .decode(encrypted_name)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_else(|| encrypted_name.to_owned())
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1_024;
    const MB: u64 = KB * 1_024;
    const GB: u64 = MB * 1_024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}
