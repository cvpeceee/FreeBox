//! Storage management API - data source and bucket operations.
//!
//! Routes (all require Bearer auth):
//!
//! ```
//! GET  /api/v1/storage/datasources      - list available storage data sources
//! GET  /api/v1/storage/buckets          - list all buckets
//! POST /api/v1/storage/buckets          - create a new bucket
//! ```

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};

use crate::{config::Config, error::AppError, state::AppState};

// ---------------------------------------------------------------------------
// Response / request types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct BucketItem {
    pub name: String,
    pub creation_date: Option<String>,
}

#[derive(Serialize)]
pub struct ListBucketsResponse {
    pub buckets: Vec<BucketItem>,
}

#[derive(Deserialize)]
pub struct CreateBucketRequest {
    /// Name for the new bucket.
    pub name: String,
}

#[derive(Serialize)]
pub struct CreateBucketResponse {
    pub name: String,
    pub created: bool,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct DatasourceItem {
    pub name: String,
    pub provider: String,
    pub location: String,
    pub region: String,
    pub status: String,
}

#[derive(Serialize)]
pub struct ListDatasourcesResponse {
    pub datasources: Vec<DatasourceItem>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/storage/datasources` - list built-in and configured data sources.
pub async fn list_datasources(
    State(state): State<AppState>,
) -> Result<Json<ListDatasourcesResponse>, AppError> {
    let cfg = &state.config;
    let mut datasources = local_datasources_from_config(cfg);

    if uses_s3_compatible_storage(cfg) {
        let buckets = freebox_storage_s3::list_buckets(
            &cfg.storage_s3_endpoint,
            &cfg.storage_s3_region,
            &cfg.storage_s3_access_key,
            &cfg.storage_s3_secret_key,
        )
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

        datasources.extend(s3_datasources_from_config_and_buckets(cfg, buckets));
    }

    Ok(Json(ListDatasourcesResponse { datasources }))
}

/// `GET /api/v1/storage/buckets` - list all buckets for the configured backend.
pub async fn list_buckets(
    State(state): State<AppState>,
) -> Result<Json<ListBucketsResponse>, AppError> {
    let cfg = &state.config;

    if !uses_s3_compatible_storage(cfg) {
        // Local storage has no cloud buckets to list.
        return Ok(Json(ListBucketsResponse { buckets: vec![] }));
    }

    let buckets = freebox_storage_s3::list_buckets(
        &cfg.storage_s3_endpoint,
        &cfg.storage_s3_region,
        &cfg.storage_s3_access_key,
        &cfg.storage_s3_secret_key,
    )
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

    Ok(Json(ListBucketsResponse {
        buckets: buckets
            .into_iter()
            .map(|b| BucketItem {
                name: b.name,
                creation_date: b.creation_date,
            })
            .collect(),
    }))
}

/// `POST /api/v1/storage/buckets` - create a new bucket.
pub async fn create_bucket(
    State(state): State<AppState>,
    Json(body): Json<CreateBucketRequest>,
) -> Result<Json<CreateBucketResponse>, AppError> {
    let cfg = &state.config;

    if !uses_s3_compatible_storage(cfg) {
        return Err(AppError::BadRequest(
            "bucket management is only available for S3-compatible backends".into(),
        ));
    }

    if body.name.is_empty() {
        return Err(AppError::BadRequest("bucket name cannot be empty".into()));
    }

    freebox_storage_s3::create_bucket(
        &cfg.storage_s3_endpoint,
        &body.name,
        &cfg.storage_s3_region,
        &cfg.storage_s3_access_key,
        &cfg.storage_s3_secret_key,
    )
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

    tracing::info!(bucket = %body.name, "Bucket created via API");

    Ok(Json(CreateBucketResponse {
        name: body.name,
        created: true,
    }))
}

fn local_datasources_from_config(cfg: &Config) -> Vec<DatasourceItem> {
    vec![DatasourceItem {
        name: "local".into(),
        provider: "local".into(),
        location: cfg.storage_local_root.clone(),
        region: "-".into(),
        status: "available".into(),
    }]
}

fn s3_datasources_from_config_and_buckets(
    cfg: &Config,
    buckets: Vec<freebox_storage_s3::BucketInfo>,
) -> Vec<DatasourceItem> {
    let provider_name = s3_provider_name(cfg);
    let active_bucket = cfg.storage_s3_bucket.trim();
    let mut bucket_names: Vec<String> = buckets.into_iter().map(|bucket| bucket.name).collect();
    bucket_names.sort();
    bucket_names.dedup();

    if bucket_names.is_empty() {
        return vec![DatasourceItem {
            name: if active_bucket.is_empty() {
                provider_name.clone()
            } else {
                active_bucket.to_owned()
            },
            provider: provider_name,
            location: s3_location_for_bucket(cfg, active_bucket),
            region: empty_as_dash(&cfg.storage_s3_region),
            status: if active_bucket.is_empty() {
                "misconfigured".into()
            } else {
                "configured".into()
            },
        }];
    }

    bucket_names
        .into_iter()
        .map(|bucket| DatasourceItem {
            status: if bucket == active_bucket {
                "configured".into()
            } else {
                "available".into()
            },
            location: s3_location_for_bucket(cfg, &bucket),
            name: bucket,
            provider: provider_name.clone(),
            region: empty_as_dash(&cfg.storage_s3_region),
        })
        .collect()
}

fn uses_s3_compatible_storage(cfg: &Config) -> bool {
    let provider = cfg.storage_provider.trim().to_ascii_lowercase();
    provider == "s3" || provider == "r2"
}

fn s3_provider_name(cfg: &Config) -> String {
    let provider = cfg.storage_provider.trim().to_ascii_lowercase();
    let endpoint = cfg.storage_s3_endpoint.to_ascii_lowercase();

    if provider == "r2" || endpoint.contains(".r2.cloudflarestorage.com") {
        "cloudflare-r2".into()
    } else if endpoint.contains("backblazeb2.com") {
        "backblaze-b2".into()
    } else if endpoint.contains("localhost")
        || endpoint.contains("127.0.0.1")
        || endpoint.contains("minio")
    {
        "minio".into()
    } else if endpoint.is_empty() {
        "aws-s3".into()
    } else {
        "s3-compatible".into()
    }
}

fn s3_location_for_bucket(cfg: &Config, bucket: &str) -> String {
    let bucket = bucket.trim();
    let endpoint = cfg.storage_s3_endpoint.trim().trim_end_matches('/');

    match (endpoint.is_empty(), bucket.is_empty()) {
        (true, true) => "s3://<bucket-not-configured>".into(),
        (true, false) => format!("s3://{bucket}"),
        (false, true) => endpoint.to_owned(),
        (false, false) => format!("{endpoint}/{bucket}"),
    }
}

fn empty_as_dash(value: &str) -> String {
    if value.trim().is_empty() {
        "-".into()
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{local_datasources_from_config, s3_datasources_from_config_and_buckets};
    use crate::config::{Config, OAuthConfig};
    use freebox_storage_s3::BucketInfo;

    fn test_config() -> Config {
        Config {
            host: "127.0.0.1".into(),
            port: 8080,
            database_url: "postgres://freebox:freebox@localhost/freebox".into(),
            db_pool_size: 10,
            redis_url: "redis://127.0.0.1:6379".into(),
            rate_limit_requests: 600,
            rate_limit_window_secs: 60,
            oauth_reactivation_max_age_days: 30,
            admin_user_ids: Vec::new(),
            jwt_secret: "test-secret".into(),
            jwt_access_ttl_secs: 900,
            jwt_refresh_ttl_secs: 2_592_000,
            storage_provider: "local".into(),
            storage_local_root: "./data/freebox-storage".into(),
            storage_s3_bucket: String::new(),
            storage_s3_region: "auto".into(),
            storage_s3_endpoint: String::new(),
            storage_s3_access_key: String::new(),
            storage_s3_secret_key: String::new(),
            argon2_memory_kib: 65_536,
            argon2_iterations: 3,
            argon2_parallelism: 4,
            oauth: OAuthConfig::default(),
        }
    }

    #[test]
    fn datasources_always_include_local() {
        let cfg = test_config();
        let datasources = local_datasources_from_config(&cfg);

        assert_eq!(datasources.len(), 1);
        assert_eq!(datasources[0].name, "local");
        assert_eq!(datasources[0].status, "available");
        assert_eq!(datasources[0].location, "./data/freebox-storage");
    }

    #[test]
    fn datasources_expand_cloudflare_r2_buckets_from_endpoint() {
        let mut cfg = test_config();
        cfg.storage_provider = "s3".into();
        cfg.storage_s3_endpoint = "https://example.r2.cloudflarestorage.com".into();
        cfg.storage_s3_bucket = "knowledge-vault".into();

        let datasources = s3_datasources_from_config_and_buckets(
            &cfg,
            vec![
                BucketInfo {
                    name: "freebox".into(),
                    creation_date: None,
                },
                BucketInfo {
                    name: "knowledge-vault".into(),
                    creation_date: None,
                },
            ],
        );

        assert_eq!(datasources.len(), 2);
        assert_eq!(datasources[0].name, "freebox");
        assert_eq!(datasources[0].provider, "cloudflare-r2");
        assert_eq!(
            datasources[0].location,
            "https://example.r2.cloudflarestorage.com/freebox"
        );
        assert_eq!(datasources[0].region, "auto");
        assert_eq!(datasources[0].status, "available");
        assert_eq!(datasources[1].name, "knowledge-vault");
        assert_eq!(datasources[1].status, "configured");
    }

    #[test]
    fn datasources_detect_aws_s3_when_endpoint_is_empty() {
        let mut cfg = test_config();
        cfg.storage_provider = "s3".into();
        cfg.storage_s3_bucket = "freebox".into();
        cfg.storage_s3_region = "us-east-1".into();

        let datasources = s3_datasources_from_config_and_buckets(
            &cfg,
            vec![BucketInfo {
                name: "freebox".into(),
                creation_date: None,
            }],
        );

        assert_eq!(datasources[0].name, "freebox");
        assert_eq!(datasources[0].provider, "aws-s3");
        assert_eq!(datasources[0].location, "s3://freebox");
        assert_eq!(datasources[0].region, "us-east-1");
        assert_eq!(datasources[0].status, "configured");
    }
}
