//! S3/R2 bucket management: create and list buckets.
//!
//! These operations use the S3 REST API directly via `reqwest` + SigV4, because
//! OpenDAL manages objects *within* a bucket and does not expose bucket-level
//! management APIs.

use freebox_core::error::{Error, Result};
use serde::Serialize;

use crate::sigv4;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Summary info about a single bucket.
#[derive(Debug, Clone, Serialize)]
pub struct BucketInfo {
    pub name: String,
    pub creation_date: Option<String>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Create a bucket. Returns `Ok(())` if the bucket already exists.
pub async fn create_bucket(
    endpoint: &str,
    bucket: &str,
    region: &str,
    access_key: &str,
    secret_key: &str,
) -> Result<()> {
    let endpoint = endpoint.trim_end_matches('/');
    let url = format!("{endpoint}/{bucket}");
    let parsed = url::Url::parse(&url)
        .map_err(|e| Error::config(format!("invalid endpoint URL: {e}")))?;
    let host = parsed.host_str().unwrap_or("").to_owned();
    let uri = format!("/{bucket}");

    let signed = sigv4::sign("PUT", &uri, "", &host, b"", access_key, secret_key, region);

    let client = reqwest::Client::new();
    let response = client
        .put(&url)
        .header("Host", &host)
        .header("x-amz-content-sha256", &signed.x_amz_content_sha256)
        .header("x-amz-date", &signed.x_amz_date)
        .header("Authorization", &signed.authorization)
        .send()
        .await
        .map_err(|e| Error::storage(format!("bucket create HTTP error: {e}")))?;

    let status = response.status();
    // 200 = created; 409 = BucketAlreadyExists / BucketAlreadyOwnedByYou — both fine.
    if status.is_success() || status.as_u16() == 409 {
        Ok(())
    } else {
        let body = response.text().await.unwrap_or_default();
        Err(Error::storage(format!(
            "failed to create bucket '{bucket}': HTTP {status} — {body}"
        )))
    }
}

/// List all buckets accessible with the given credentials.
///
/// Calls the S3 `ListBuckets` (`GET /`) operation at the account-level
/// endpoint (no bucket name in the URL).
pub async fn list_buckets(
    endpoint: &str,
    region: &str,
    access_key: &str,
    secret_key: &str,
) -> Result<Vec<BucketInfo>> {
    let endpoint = endpoint.trim_end_matches('/');
    let parsed = url::Url::parse(endpoint)
        .map_err(|e| Error::config(format!("invalid endpoint URL: {e}")))?;
    let host = parsed.host_str().unwrap_or("").to_owned();

    let signed = sigv4::sign("GET", "/", "", &host, b"", access_key, secret_key, region);

    let client = reqwest::Client::new();
    let response = client
        .get(endpoint)
        .header("Host", &host)
        .header("x-amz-content-sha256", &signed.x_amz_content_sha256)
        .header("x-amz-date", &signed.x_amz_date)
        .header("Authorization", &signed.authorization)
        .send()
        .await
        .map_err(|e| Error::storage(format!("list buckets HTTP error: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(Error::storage(format!(
            "failed to list buckets: HTTP {status} — {body}"
        )));
    }

    let body = response.text().await.unwrap_or_default();
    Ok(parse_list_buckets_xml(&body))
}

/// Auto-create a bucket when it does not yet exist.
/// Called from `S3Plugin::on_load` on a `NoSuchBucket` error.
pub async fn ensure_bucket_exists(
    endpoint: &str,
    bucket: &str,
    region: &str,
    access_key: &str,
    secret_key: &str,
) -> Result<()> {
    tracing::warn!(bucket = %bucket, "Bucket does not exist — creating automatically");
    create_bucket(endpoint, bucket, region, access_key, secret_key).await?;
    tracing::info!(bucket = %bucket, "Bucket created successfully");
    Ok(())
}

// ---------------------------------------------------------------------------
// Minimal XML parser for ListBuckets response
// ---------------------------------------------------------------------------

fn parse_list_buckets_xml(xml: &str) -> Vec<BucketInfo> {
    let mut buckets = Vec::new();
    let mut pos = 0;
    while let Some(rel) = xml[pos..].find("<Bucket>") {
        let start = pos + rel;
        let end = match xml[start..].find("</Bucket>") {
            Some(e) => start + e + "</Bucket>".len(),
            None => break,
        };
        let chunk = &xml[start..end];
        let name = extract_tag(chunk, "Name").unwrap_or_default().to_owned();
        let creation_date = extract_tag(chunk, "CreationDate").map(ToOwned::to_owned);
        if !name.is_empty() {
            buckets.push(BucketInfo { name, creation_date });
        }
        pos = end;
    }
    buckets
}

fn extract_tag<'a>(s: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = s.find(&open).map(|i| i + open.len())?;
    let end = s[start..].find(&close).map(|i| start + i)?;
    Some(&s[start..end])
}

#[cfg(test)]
mod tests;
