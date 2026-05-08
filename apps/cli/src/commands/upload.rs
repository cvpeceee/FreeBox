//! Upload command implementation.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use freebox_crypto::encryption::{encrypt_chunk, encrypt_file, ChunkCiphertext, FileKey};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::api_url,
    session::{effective_server, Session},
};

#[derive(Serialize)]
struct UploadInitRequest {
    total_chunks: u32,
    size_bytes: u64,
    encrypted_key_envelope: String,
    content_hash: String,
    encrypted_name: String,
}

#[derive(Deserialize)]
struct UploadInitResponse {
    upload_id: Uuid,
}

#[derive(Deserialize)]
struct UploadCompleteResponse {
    file_id: Uuid,
}

pub(crate) struct PreparedUpload {
    pub total_chunks: u32,
    pub size_bytes: u64,
    pub encrypted_key_envelope: String,
    pub content_hash: String,
    pub encrypted_name: String,
    pub chunks: Vec<ChunkCiphertext>,
}

pub(crate) struct UploadedFile {
    pub local_path: PathBuf,
    pub file_id: Uuid,
}

pub async fn run(
    files: Vec<PathBuf>,
    destination: String,
    parallelism: u8,
    server: &str,
) -> Result<()> {
    let session = Session::load()?;
    let server = effective_server(server, &session);
    let uploaded = upload_files(files, destination, parallelism, server, &session).await?;

    for file in uploaded {
        println!("Uploaded {} as {}", file.local_path.display(), file.file_id);
    }

    Ok(())
}

pub(crate) async fn upload_files(
    files: Vec<PathBuf>,
    destination: String,
    parallelism: u8,
    server: &str,
    session: &Session,
) -> Result<Vec<UploadedFile>> {
    if parallelism == 0 || parallelism > 16 {
        anyhow::bail!("parallelism must be between 1 and 16");
    }

    let client = reqwest::Client::new();
    let mut uploaded = Vec::with_capacity(files.len());

    for file in files {
        let plaintext = tokio::fs::read(&file)
            .await
            .with_context(|| format!("failed to read {}", file.display()))?;
        let remote_name = remote_name(&destination, &file)?;
        let prepared = prepare_upload(&remote_name, &plaintext)?;

        let init = UploadInitRequest {
            total_chunks: prepared.total_chunks,
            size_bytes: prepared.size_bytes,
            encrypted_key_envelope: prepared.encrypted_key_envelope,
            content_hash: prepared.content_hash,
            encrypted_name: prepared.encrypted_name,
        };

        let init_response = client
            .post(api_url(server, "/api/v1/files/upload/init"))
            .bearer_auth(&session.access_token)
            .json(&init)
            .send()
            .await?
            .error_for_status()?
            .json::<UploadInitResponse>()
            .await?;

        for chunk in &prepared.chunks {
            let body = serde_json::to_vec(chunk)?;
            client
                .put(api_url(
                    server,
                    &format!("/api/v1/files/upload/{}", init_response.upload_id),
                ))
                .bearer_auth(&session.access_token)
                .header("X-Chunk-Index", chunk.index.to_string())
                .body(body)
                .send()
                .await?
                .error_for_status()?;
        }

        let complete = client
            .post(api_url(
                server,
                &format!("/api/v1/files/upload/{}/complete", init_response.upload_id),
            ))
            .bearer_auth(&session.access_token)
            .send()
            .await?
            .error_for_status()?
            .json::<UploadCompleteResponse>()
            .await?;

        uploaded.push(UploadedFile {
            local_path: file,
            file_id: complete.file_id,
        });
    }

    Ok(uploaded)
}

pub(crate) fn prepare_upload(remote_name: &str, plaintext: &[u8]) -> Result<PreparedUpload> {
    let key = FileKey::generate();
    let chunks = encrypt_plaintext(&key, plaintext)?;

    Ok(PreparedUpload {
        total_chunks: chunks
            .len()
            .try_into()
            .context("file has too many chunks for upload protocol")?,
        size_bytes: plaintext.len() as u64,
        encrypted_key_envelope: STANDARD.encode(key.as_bytes()),
        content_hash: blake3::hash(plaintext).to_hex().to_string(),
        encrypted_name: STANDARD.encode(remote_name.as_bytes()),
        chunks,
    })
}

fn encrypt_plaintext(key: &FileKey, plaintext: &[u8]) -> Result<Vec<ChunkCiphertext>> {
    if plaintext.is_empty() {
        Ok(vec![encrypt_chunk(key, 0, b"")?])
    } else {
        encrypt_file(key, plaintext)
    }
}

pub(crate) fn remote_name(destination: &str, file: &Path) -> Result<String> {
    let file_name = file
        .file_name()
        .and_then(|name| name.to_str())
        .context("upload path must have a UTF-8 file name")?;

    let destination = destination.trim_end_matches('/');
    if destination.is_empty() || destination == "remote:" || destination == "remote://" {
        Ok(file_name.to_owned())
    } else {
        Ok(format!("{destination}/{file_name}"))
    }
}

#[cfg(test)]
mod tests;
