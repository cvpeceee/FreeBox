//! Download command implementation.

use std::path::PathBuf;

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use freebox_crypto::encryption::{decrypt_file, ChunkCiphertext, FileKey};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::api_url,
    session::{effective_server, Session},
};

#[derive(Deserialize)]
struct FileMetaResponse {
    encrypted_name: String,
    total_chunks: u32,
    encrypted_key_envelope: String,
}

pub async fn run(remote: String, output: Option<PathBuf>, server: &str) -> Result<()> {
    let file_id = Uuid::parse_str(&remote).context("download currently expects a file UUID")?;
    let session = Session::load()?;
    let server = effective_server(server, &session);
    let output = download_file(file_id, output, server, &session).await?;

    println!("Downloaded {file_id} to {}", output.display());
    Ok(())
}

pub(crate) async fn download_file(
    file_id: Uuid,
    output: Option<PathBuf>,
    server: &str,
    session: &Session,
) -> Result<PathBuf> {
    let client = reqwest::Client::new();

    let meta = client
        .get(api_url(server, &format!("/api/v1/files/{file_id}")))
        .bearer_auth(&session.access_token)
        .send()
        .await?
        .error_for_status()?
        .json::<FileMetaResponse>()
        .await?;

    let key = file_key_from_envelope(&meta.encrypted_key_envelope)?;
    let mut chunks = Vec::with_capacity(meta.total_chunks as usize);

    for chunk_index in 0..meta.total_chunks {
        let body = client
            .get(api_url(
                server,
                &format!("/api/v1/files/{file_id}/chunk/{chunk_index}"),
            ))
            .bearer_auth(&session.access_token)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        chunks.push(serde_json::from_slice::<ChunkCiphertext>(&body)?);
    }

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
