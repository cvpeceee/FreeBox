//! Download command implementation.

use std::path::PathBuf;

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use freebox_crypto::encryption::{decrypt_file, ChunkCiphertext, FileKey};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::api_url,
    client::send_with_refresh,
    session::{effective_server, Session},
};

#[derive(Deserialize)]
struct FileMetaResponse {
    encrypted_name: String,
    total_chunks: u32,
    encrypted_key_envelope: String,
}

#[derive(Deserialize)]
struct FileSummary {
    file_id: Uuid,
    encrypted_name: String,
}

#[derive(Deserialize)]
struct FileListResponse {
    files: Vec<FileSummary>,
}

pub async fn run(remote: String, output: Option<PathBuf>, server: &str) -> Result<()> {
    let mut session = Session::load()?;
    let server = effective_server(server, &session).to_owned();

    // Accept either a UUID or a plain file name / path.
    let file_id = match Uuid::parse_str(&remote) {
        Ok(id) => id,
        Err(_) => resolve_name_to_id(&remote, &server, &mut session).await?,
    };

    let output = download_file(file_id, output, &server, &mut session).await?;
    println!("Downloaded to {}", output.display());
    Ok(())
}

/// Look up a file ID by (decoded) remote name.
/// Performs a prefix / suffix match so `report.pdf`, `docs/report.pdf`, and
/// `remote://docs/report.pdf` all find the same file.
async fn resolve_name_to_id(name: &str, server: &str, session: &mut Session) -> Result<Uuid> {
    let http = reqwest::Client::new();
    let url = api_url(server, "/api/v1/files");

    let list = send_with_refresh(session, |token| http.get(&url).bearer_auth(token))
        .await?
        .error_for_status()?
        .json::<FileListResponse>()
        .await?;

    // Normalise the search term: strip remote:// prefix and leading slash.
    let needle = name.trim_start_matches("remote://").trim_start_matches('/');

    // Find the first file whose decoded name ends with the needle.
    let matched = list.files.into_iter().find(|f| {
        let decoded = STANDARD
            .decode(&f.encrypted_name)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_default();
        let decoded_norm = decoded
            .trim_start_matches("remote://")
            .trim_start_matches('/');
        // Accept exact match or suffix match (e.g. "report.pdf" matches "docs/report.pdf").
        decoded_norm == needle || decoded_norm.ends_with(&format!("/{needle}"))
    });

    matched
        .map(|f| f.file_id)
        .ok_or_else(|| anyhow::anyhow!("no file named '{name}' found — use `fbx ls` to list files"))
}

pub(crate) async fn download_file(
    file_id: Uuid,
    output: Option<PathBuf>,
    server: &str,
    session: &mut Session,
) -> Result<PathBuf> {
    let http = reqwest::Client::new();
    let meta_url = api_url(server, &format!("/api/v1/files/{file_id}"));

    let meta = send_with_refresh(session, |token| http.get(&meta_url).bearer_auth(token))
        .await?
        .error_for_status()?
        .json::<FileMetaResponse>()
        .await?;

    let key = file_key_from_envelope(&meta.encrypted_key_envelope)?;
    let mut chunks = Vec::with_capacity(meta.total_chunks as usize);

    let pb = ProgressBar::new(meta.total_chunks as u64);
    pb.set_style(
        ProgressStyle::with_template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} chunks")
            .unwrap()
            .progress_chars("=> "),
    );

    for chunk_index in 0..meta.total_chunks {
        let chunk_url = api_url(
            server,
            &format!("/api/v1/files/{file_id}/chunk/{chunk_index}"),
        );
        let body = send_with_refresh(session, |token| http.get(&chunk_url).bearer_auth(token))
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        chunks.push(serde_json::from_slice::<ChunkCiphertext>(&body)?);
        pb.inc(1);
    }

    pb.finish_and_clear();

    let plaintext = decrypt_file(&key, &chunks)?;
    let output = resolve_output_path(output, &meta.encrypted_name)?;
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }
    tokio::fs::write(&output, plaintext)
        .await
        .with_context(|| format!("failed to write {}", output.display()))?;

    Ok(output)
}

pub(crate) fn file_key_from_envelope(envelope: &str) -> Result<FileKey> {
    let key_bytes: [u8; 32] = STANDARD
        .decode(envelope)
        .context("file key envelope is not valid base64")?
        .try_into()
        .map_err(|bytes: Vec<u8>| {
            anyhow::anyhow!(
                "file key envelope must decode to 32 bytes, got {}",
                bytes.len()
            )
        })?;
    Ok(FileKey::from_bytes(key_bytes))
}

pub(crate) fn resolve_output_path(
    output: Option<PathBuf>,
    encrypted_name: &str,
) -> Result<PathBuf> {
    let default_name = default_file_name(encrypted_name)?;
    match output {
        Some(path) if path.is_dir() => Ok(path.join(default_name)),
        Some(path) => Ok(path),
        None => Ok(PathBuf::from(default_name)),
    }
}

pub(crate) fn default_file_name(encrypted_name: &str) -> Result<String> {
    let decoded = STANDARD
        .decode(encrypted_name)
        .context("encrypted_name is not valid base64")?;
    let remote_name = String::from_utf8(decoded).context("encrypted_name is not UTF-8")?;
    let file_name = remote_name
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or("download.bin");
    Ok(file_name.to_owned())
}

#[cfg(test)]
mod tests;
