//! Server configuration — loaded from environment variables at startup.
//!
//! All config values have sensible defaults for local development.
//! In production, set these via environment variables or a secrets manager.
//!
//! See `.env.example` in the repository root for the full list of variables.

use anyhow::Context;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// OAuth provider configuration
// ---------------------------------------------------------------------------

/// Configuration for a single OAuth2 provider (e.g., GitHub, Google).
///
/// Providers are opt-in: if the environment variables are not set for a
/// provider, it will be `None` in [`OAuthConfig`] and the corresponding
/// `/auth/oauth/:provider` routes will return 404.
#[derive(Debug, Clone)]
pub struct OAuthProviderConfig {
    /// OAuth2 client ID (from the provider's developer console).
    pub client_id: String,
    /// OAuth2 client secret — **never expose this to clients**.
    pub client_secret: String,
    /// OAuth2 redirect URI (must match the value registered with the provider).
    /// Example: `https://app.freebox.io/api/v1/auth/oauth/github/callback`
    pub redirect_uri: String,
}

/// OAuth2 configuration for all supported providers.
///
/// Each provider is `Option<OAuthProviderConfig>` — only providers with
/// all three env vars set (CLIENT_ID, CLIENT_SECRET, REDIRECT_URI) are enabled.
#[derive(Debug, Clone, Default)]
pub struct OAuthConfig {
    pub github: Option<OAuthProviderConfig>,
    pub google: Option<OAuthProviderConfig>,
    pub microsoft: Option<OAuthProviderConfig>,
    pub apple: Option<OAuthProviderConfig>,
    pub facebook: Option<OAuthProviderConfig>,
}

impl OAuthConfig {
    /// Load OAuth config from environment variables.
    ///
    /// Each provider reads three env vars:
    /// - `OAUTH_{PROVIDER}_CLIENT_ID`
    /// - `OAUTH_{PROVIDER}_CLIENT_SECRET`
    /// - `OAUTH_{PROVIDER}_REDIRECT_URI`
    ///
    /// If any of the three is missing, the entire provider is disabled.
    pub fn from_env() -> Self {
        Self {
            github: load_provider_config("GITHUB"),
            google: load_provider_config("GOOGLE"),
            microsoft: load_provider_config("MICROSOFT"),
            apple: load_provider_config("APPLE"),
            facebook: load_provider_config("FACEBOOK"),
        }
    }

    /// Look up a provider config by name (case-insensitive).
    /// Returns `None` if the provider is not configured or unknown.
    pub fn get(&self, provider: &str) -> Option<&OAuthProviderConfig> {
        match provider.to_lowercase().as_str() {
            "github" => self.github.as_ref(),
            "google" => self.google.as_ref(),
            "microsoft" => self.microsoft.as_ref(),
            "apple" => self.apple.as_ref(),
            "facebook" => self.facebook.as_ref(),
            _ => None,
        }
    }
}

/// Try to load a single provider's config from env vars.
/// Returns `None` if any of the three required vars is missing.
fn load_provider_config(provider: &str) -> Option<OAuthProviderConfig> {
    let client_id = std::env::var(format!("OAUTH_{provider}_CLIENT_ID")).ok()?;
    let client_secret = std::env::var(format!("OAUTH_{provider}_CLIENT_SECRET")).ok()?;
    let redirect_uri = std::env::var(format!("OAUTH_{provider}_REDIRECT_URI")).ok()?;
    Some(OAuthProviderConfig {
        client_id,
        client_secret,
        redirect_uri,
    })
}

// ---------------------------------------------------------------------------
// Top-level server configuration
// ---------------------------------------------------------------------------

/// Top-level server configuration.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Config {
    // --- Network ---
    pub host: String,
    pub port: u16,

    // --- Database ---
    pub database_url: String,
    pub db_pool_size: u32,

    // --- Cache ---
    pub redis_url: String,

    // --- API protection ---
    pub rate_limit_requests: u32,
    pub rate_limit_window_secs: u64,
    /// Max age (days) for automatic OAuth reactivation of soft-deleted users.
    /// 0 disables the limit (reactivate regardless of deletion age).
    pub oauth_reactivation_max_age_days: u32,
    /// Comma-separated UUID allowlist for admin-only API routes.
    pub admin_user_ids: Vec<Uuid>,

    // --- JWT ---
    pub jwt_secret: String,
    /// Access token TTL in seconds (default: 15 minutes).
    pub jwt_access_ttl_secs: u64,
    /// Refresh token TTL in seconds (default: 30 days).
    pub jwt_refresh_ttl_secs: u64,

    // --- Storage ---
    pub storage_provider: String,
    pub storage_local_root: String,

    // --- S3 / Cloudflare R2 storage ---
    /// S3 bucket name (required when STORAGE_PROVIDER=s3).
    pub storage_s3_bucket: String,
    /// AWS region (default: us-east-1; for R2 use "auto").
    pub storage_s3_region: String,
    /// Custom endpoint URL for R2 / MinIO / etc.
    /// R2: `https://<ACCOUNT_ID>.r2.cloudflarestorage.com`
    pub storage_s3_endpoint: String,
    /// S3 access key ID (or R2 Access Key ID).
    pub storage_s3_access_key: String,
    /// S3 secret access key (or R2 Secret Access Key).
    pub storage_s3_secret_key: String,

    // --- Argon2id (password hashing) ---
    pub argon2_memory_kib: u32,
    pub argon2_iterations: u32,
    pub argon2_parallelism: u32,

    // --- OAuth2 ---
    pub oauth: OAuthConfig,
}

impl Config {
    /// Build configuration from environment variables.
    ///
    /// Returns an error with a descriptive message if a required variable
    /// is missing or cannot be parsed.
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            host: env_or("SERVER_HOST", "127.0.0.1"),
            port: env_parse("SERVER_PORT", 8080)?,
            database_url: env_require("DATABASE_URL")?,
            db_pool_size: env_parse("DB_POOL_SIZE", 10)?,
            redis_url: env_or("REDIS_URL", "redis://127.0.0.1:6379"),
            rate_limit_requests: env_parse("RATE_LIMIT_REQUESTS", 600)?,
            rate_limit_window_secs: env_parse("RATE_LIMIT_WINDOW_SECS", 60)?,
            oauth_reactivation_max_age_days: env_parse(
                "OAUTH_REACTIVATION_MAX_AGE_DAYS",
                30,
            )?,
            admin_user_ids: env_parse_uuid_list("ADMIN_USER_IDS")?,
            jwt_secret: env_require("JWT_SECRET")?,
            jwt_access_ttl_secs: env_parse("JWT_ACCESS_TTL_SECS", 900)?, // 15 min
            jwt_refresh_ttl_secs: env_parse("JWT_REFRESH_TTL_SECS", 2_592_000)?, // 30 days
            storage_provider: env_or("STORAGE_PROVIDER", "local"),
            storage_local_root: env_or("STORAGE_LOCAL_ROOT", "./data/freebox-storage"),
            storage_s3_bucket: env_or("STORAGE_S3_BUCKET", ""),
            storage_s3_region: env_or("STORAGE_S3_REGION", "auto"),
            storage_s3_endpoint: env_or("STORAGE_S3_ENDPOINT", ""),
            storage_s3_access_key: env_or("STORAGE_S3_ACCESS_KEY", ""),
            storage_s3_secret_key: env_or("STORAGE_S3_SECRET_KEY", ""),
            argon2_memory_kib: env_parse("ARGON2_MEMORY_KIB", 65_536)?,
            argon2_iterations: env_parse("ARGON2_ITERATIONS", 3)?,
            argon2_parallelism: env_parse("ARGON2_PARALLELISM", 4)?,
            oauth: OAuthConfig::from_env(),
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Read `key` from environment, falling back to `default` if absent.
fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

/// Read `key` from environment; error if absent.
fn env_require(key: &str) -> anyhow::Result<String> {
    std::env::var(key).with_context(|| format!("required environment variable `{key}` not set"))
}

/// Read `key` from environment and parse it as `T`; use `default` if absent.
fn env_parse<T>(key: &str, default: T) -> anyhow::Result<T>
where
    T: std::str::FromStr + Copy,
    T::Err: std::fmt::Display,
{
    match std::env::var(key) {
        Ok(val) => val
            .parse::<T>()
            .map_err(|e| anyhow::anyhow!("cannot parse `{key}={val}`: {e}")),
        Err(_) => Ok(default),
    }
}

fn env_parse_uuid_list(key: &str) -> anyhow::Result<Vec<Uuid>> {
    let raw = match std::env::var(key) {
        Ok(value) => value,
        Err(_) => return Ok(Vec::new()),
    };

    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            Uuid::parse_str(value)
                .map_err(|e| anyhow::anyhow!("cannot parse `{key}` UUID `{value}`: {e}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::env_parse_uuid_list;

    #[test]
    fn env_parse_uuid_list_returns_empty_when_missing() {
        std::env::remove_var("ADMIN_USER_IDS_TEST");

        let ids = env_parse_uuid_list("ADMIN_USER_IDS_TEST").unwrap();

        assert!(ids.is_empty());
    }

    #[test]
    fn env_parse_uuid_list_parses_comma_separated_values() {
        std::env::set_var(
            "ADMIN_USER_IDS_TEST",
            "11111111-1111-1111-1111-111111111111, 22222222-2222-2222-2222-222222222222",
        );

        let ids = env_parse_uuid_list("ADMIN_USER_IDS_TEST").unwrap();

        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0].to_string(), "11111111-1111-1111-1111-111111111111");
        assert_eq!(ids[1].to_string(), "22222222-2222-2222-2222-222222222222");

        std::env::remove_var("ADMIN_USER_IDS_TEST");
    }

    #[test]
    fn env_parse_uuid_list_rejects_invalid_uuid() {
        std::env::set_var("ADMIN_USER_IDS_TEST", "not-a-uuid");

        let err = env_parse_uuid_list("ADMIN_USER_IDS_TEST").unwrap_err();

        assert!(err.to_string().contains("cannot parse `ADMIN_USER_IDS_TEST`"));

        std::env::remove_var("ADMIN_USER_IDS_TEST");
    }
}
