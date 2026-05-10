//! Data source management - add, list, remove, and test storage sources.
//!
//! Storage sources are saved in `~/.config/freebox/config.toml` under
//! the `[[sources]]` array. The server reads its active storage backend from
//! environment variables; this command manages the CLI-side registry so users
//! can see which storage data sources are available and test connectivity
//! before pointing the server at one.

use anyhow::{Context, Result};
use dialoguer::{Input, Password, Select};
use serde::Deserialize;

use crate::{
    auth::api_url,
    config::{CliConfig, StorageSource},
    session::DEFAULT_SERVER,
    ProviderCommands,
};

pub async fn handle(cmd: ProviderCommands, server: &str) -> Result<()> {
    match cmd {
        ProviderCommands::Add { provider } => add(provider).await,
        ProviderCommands::List => list(server).await,
        ProviderCommands::Remove { provider_id } => remove(&provider_id),
        ProviderCommands::Test { provider_id } => test(&provider_id).await,
    }
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

async fn list(server: &str) -> Result<()> {
    let rows = server_datasource_rows(server)
        .await
        .unwrap_or_else(local_datasource_rows);

    println!(
        "{:<20} {:<15} {:<12} {:<10} {}",
        "NAME", "TYPE", "STATUS", "REGION", "LOCATION"
    );
    println!("{}", "-".repeat(110));
    for row in &rows {
        println!(
            "{:<20} {:<15} {:<12} {:<10} {}",
            row.name, row.provider, row.status, row.region, row.location
        );
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct DataSourceRow {
    name: String,
    provider: String,
    location: String,
    region: String,
    status: String,
}

#[derive(Deserialize)]
struct ServerDatasourceResponse {
    datasources: Vec<DataSourceRow>,
}

async fn server_datasource_rows(server: &str) -> Option<Vec<DataSourceRow>> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .ok()?;

    for candidate in server_candidates(server) {
        let url = api_url(&candidate, "/api/v1/storage/datasources");
        let Ok(response) = http.get(&url).send().await else {
            continue;
        };
        let Ok(response) = response.error_for_status() else {
            continue;
        };
        let Ok(resp) = response.json::<ServerDatasourceResponse>().await else {
            continue;
        };

        if !resp.datasources.is_empty() {
            return Some(resp.datasources);
        }
    }

    None
}

fn server_candidates(server: &str) -> Vec<String> {
    if server == DEFAULT_SERVER {
        vec![
            "http://127.0.0.1:8080".into(),
            "http://localhost:8080".into(),
        ]
    } else {
        vec![server.to_owned()]
    }
}

fn datasource_rows(configured: &[StorageSource]) -> Vec<DataSourceRow> {
    let mut rows = Vec::with_capacity(configured.len() + 1);
    let has_local_named_source = configured
        .iter()
        .any(|source| source.name.eq_ignore_ascii_case("local"));

    if !has_local_named_source {
        rows.push(DataSourceRow {
            name: "local".into(),
            provider: "local".into(),
            location: "server local filesystem".into(),
            region: "-".into(),
            status: "available".into(),
        });
    }

    rows.extend(configured.iter().map(configured_source_row));
    rows
}

fn local_datasource_rows() -> Vec<DataSourceRow> {
    datasource_rows(&[])
}

fn configured_source_row(source: &StorageSource) -> DataSourceRow {
    DataSourceRow {
        name: source.name.clone(),
        provider: source.provider.clone(),
        location: source_location(source),
        region: if source.region.is_empty() {
            "-".into()
        } else {
            source.region.clone()
        },
        status: if source.provider == "local" {
            "available".into()
        } else {
            "configured".into()
        },
    }
}

fn source_location(source: &StorageSource) -> String {
    if source.provider == "local" {
        if source.bucket.is_empty() {
            "server local filesystem".into()
        } else {
            source.bucket.clone()
        }
    } else if source.endpoint.is_empty() {
        format!("s3://{}", source.bucket)
    } else {
        format!(
            "{}/{}",
            source.endpoint.trim_end_matches('/'),
            source.bucket
        )
    }
}

// ---------------------------------------------------------------------------
// add
// ---------------------------------------------------------------------------

async fn add(provider_type: String) -> Result<()> {
    // Supported provider types with friendly display labels.
    let types = ["cloudflare-r2", "aws-s3", "minio", "backblaze-b2", "local"];

    // If the caller passed a specific type use it, otherwise prompt.
    let chosen = if types.contains(&provider_type.as_str()) {
        provider_type.clone()
    } else if provider_type.is_empty() || provider_type == "?" {
        let idx = Select::new()
            .with_prompt("Data source type")
            .items(&types)
            .default(0)
            .interact()
            .context("failed to read data source type")?;
        types[idx].to_owned()
    } else {
        provider_type.clone()
    };

    let name: String = Input::new()
        .with_prompt("Source name (e.g. cloudflare-r2)")
        .default(chosen.clone())
        .interact_text()?;

    let (endpoint_hint, region_default, needs_endpoint) = match chosen.as_str() {
        "cloudflare-r2" => (
            "https://<ACCOUNT_ID>.r2.cloudflarestorage.com",
            "auto",
            true,
        ),
        "minio" => ("http://localhost:9000", "us-east-1", true),
        "backblaze-b2" => (
            "https://s3.us-west-004.backblazeb2.com",
            "us-west-004",
            true,
        ),
        "local" => ("", "", false),
        _ => ("", "us-east-1", false), // aws-s3 uses no custom endpoint
    };

    let endpoint = if needs_endpoint {
        Input::<String>::new()
            .with_prompt(format!("Endpoint URL (e.g. {endpoint_hint})"))
            .allow_empty(false)
            .interact_text()?
    } else {
        String::new()
    };

    let bucket: String = if chosen == "local" {
        Input::new()
            .with_prompt("Local root directory path")
            .interact_text()?
    } else {
        Input::new().with_prompt("Bucket name").interact_text()?
    };

    let region: String = if chosen == "local" {
        String::new()
    } else {
        Input::new()
            .with_prompt("Region")
            .default(region_default.to_owned())
            .interact_text()?
    };

    let access_key: String = if chosen != "local" {
        Input::new().with_prompt("Access Key ID").interact_text()?
    } else {
        String::new()
    };

    // Show the secret key prompt only when an access key was given.
    let _secret_key: String = if !access_key.is_empty() {
        Password::new()
            .with_prompt("Secret Access Key")
            .interact()?
    } else {
        String::new()
    };

    let source = StorageSource {
        name: name.clone(),
        provider: chosen,
        endpoint,
        bucket,
        region,
    };

    let mut cfg = CliConfig::load();
    // Replace if a source with the same name already exists.
    cfg.sources.retain(|s| s.name != name);
    cfg.sources.push(source);
    cfg.save()?;

    println!("Source '{name}' saved.");
    println!("To use it, set these environment variables before starting the server:");
    print_env_hint(cfg.sources.last().unwrap());
    Ok(())
}

// ---------------------------------------------------------------------------
// remove
// ---------------------------------------------------------------------------

fn remove(provider_id: &str) -> Result<()> {
    let mut cfg = CliConfig::load();
    let before = cfg.sources.len();
    cfg.sources.retain(|s| s.name != provider_id);
    if cfg.sources.len() == before {
        anyhow::bail!("no source named '{provider_id}' found - run `fbx datasource list`");
    }
    cfg.save()?;
    println!("Source '{provider_id}' removed.");
    Ok(())
}

// ---------------------------------------------------------------------------
// test
// ---------------------------------------------------------------------------

async fn test(provider_id: &str) -> Result<()> {
    let cfg = CliConfig::load();
    let source = cfg
        .sources
        .iter()
        .find(|s| s.name == provider_id)
        .ok_or_else(|| {
            anyhow::anyhow!("no source named '{provider_id}' found - run `fbx datasource list`")
        })?;

    if source.provider == "local" {
        let exists = std::path::Path::new(&source.bucket).exists();
        if exists {
            println!(
                "OK Local path '{}' exists and is accessible.",
                source.bucket
            );
        } else {
            anyhow::bail!("local path '{}' does not exist", source.bucket);
        }
        return Ok(());
    }

    // For S3-compatible sources: perform a lightweight HEAD request to the
    // endpoint to check reachability. A full S3 connectivity test requires
    // credentials which we don't store in the config file - we only verify
    // that the endpoint URL is reachable.
    let url = if source.endpoint.is_empty() {
        format!("https://s3.amazonaws.com/{}", source.bucket)
    } else {
        format!(
            "{}/{}",
            source.endpoint.trim_end_matches('/'),
            source.bucket
        )
    };

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    match http.head(&url).send().await {
        Ok(resp) => {
            // 200, 403 (auth needed but endpoint reached), 404 (bucket wrong)
            // are all proof the endpoint is reachable.
            println!(
                "OK Endpoint reachable (HTTP {}). Source '{}' looks good.",
                resp.status().as_u16(),
                provider_id
            );
        }
        Err(e) => {
            anyhow::bail!("Could not reach endpoint '{}': {e}", url);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn print_env_hint(s: &StorageSource) {
    println!();
    if s.provider == "local" {
        println!("  STORAGE_PROVIDER=local");
        println!("  STORAGE_LOCAL_ROOT={}", s.bucket);
        return;
    }

    println!("  STORAGE_PROVIDER=s3");
    println!("  STORAGE_S3_BUCKET={}", s.bucket);
    println!("  STORAGE_S3_REGION={}", s.region);
    if !s.endpoint.is_empty() {
        println!("  STORAGE_S3_ENDPOINT={}", s.endpoint);
    }
    println!("  STORAGE_S3_ACCESS_KEY=<your-access-key>");
    println!("  STORAGE_S3_SECRET_KEY=<your-secret-key>");
}

#[cfg(test)]
mod tests {
    use super::{datasource_rows, server_candidates, source_location};
    use crate::config::StorageSource;

    fn r2_source() -> StorageSource {
        StorageSource {
            name: "cloudflare".into(),
            provider: "cloudflare-r2".into(),
            endpoint: "https://example.r2.cloudflarestorage.com".into(),
            bucket: "freebox".into(),
            region: "auto".into(),
        }
    }

    #[test]
    fn datasource_rows_always_include_builtin_local() {
        let rows = datasource_rows(&[]);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "local");
        assert_eq!(rows[0].provider, "local");
        assert_eq!(rows[0].status, "available");
    }

    #[test]
    fn datasource_rows_include_configured_cloud_sources_after_local() {
        let rows = datasource_rows(&[r2_source()]);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "local");
        assert_eq!(rows[1].name, "cloudflare");
        assert_eq!(rows[1].provider, "cloudflare-r2");
        assert_eq!(
            rows[1].location,
            "https://example.r2.cloudflarestorage.com/freebox"
        );
        assert_eq!(rows[1].region, "auto");
        assert_eq!(rows[1].status, "configured");
    }

    #[test]
    fn datasource_rows_do_not_duplicate_configured_local_name() {
        let local = StorageSource {
            name: "local".into(),
            provider: "local".into(),
            endpoint: String::new(),
            bucket: "C:/freebox/data".into(),
            region: String::new(),
        };
        let rows = datasource_rows(&[local]);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "local");
        assert_eq!(rows[0].location, "C:/freebox/data");
        assert_eq!(rows[0].status, "available");
    }

    #[test]
    fn source_location_uses_s3_uri_when_endpoint_is_empty() {
        let aws = StorageSource {
            name: "aws".into(),
            provider: "aws-s3".into(),
            endpoint: String::new(),
            bucket: "freebox".into(),
            region: "us-east-1".into(),
        };

        assert_eq!(source_location(&aws), "s3://freebox");
    }

    #[test]
    fn server_candidates_try_local_dev_server_for_default_server() {
        let candidates = server_candidates("https://freebox.io");

        assert_eq!(candidates[0], "http://127.0.0.1:8080");
        assert_eq!(candidates[1], "http://localhost:8080");
        assert_eq!(candidates.len(), 2);
    }

    #[test]
    fn server_candidates_preserve_explicit_server() {
        let candidates = server_candidates("http://example.test:9000");

        assert_eq!(candidates, vec!["http://example.test:9000"]);
    }
}
