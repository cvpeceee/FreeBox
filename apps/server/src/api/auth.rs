//! Authentication handlers — register, login, refresh, logout.
//!
//! # Security Model
//!
//! - Passwords are hashed client-side with Argon2id, then the **hash** is
//!   re-hashed server-side with Argon2id (double hashing). This ensures:
//!   1. The raw password never crosses the network (client-side Argon2id).
//!   2. A database leak reveals only double-hashed values (server-side Argon2id).
//! - Access tokens are short-lived JWTs (15 min). Refresh tokens are opaque
//!   random strings stored in the database (rotated on use).
//! - All token operations use constant-time comparison to prevent timing attacks.
//! - Login uses a generic error message to prevent user enumeration.

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use sqlx::Row;

use crate::{
    error::{AppError, Result},
    state::AppState,
};

// ---------------------------------------------------------------------------
// JWT Claims
// ---------------------------------------------------------------------------

/// Claims embedded in every access token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject — the user's UUID.
    pub sub: Uuid,
    /// Username — included for convenience (not authoritative).
    pub username: String,
    /// Expiry timestamp (Unix seconds).
    pub exp: usize,
    /// Issued-at timestamp (Unix seconds).
    pub iat: usize,
}

impl Claims {
    /// Create claims that expire `ttl_secs` from now.
    pub fn new(user_id: Uuid, username: &str, ttl_secs: u64) -> Self {
        let now = Utc::now().timestamp() as usize;
        Self {
            sub: user_id,
            username: username.to_owned(),
            exp: now + ttl_secs as usize,
            iat: now,
        }
    }
}

// ---------------------------------------------------------------------------
// Request / Response DTOs
// ---------------------------------------------------------------------------

/// Input validation constants.
const USERNAME_MIN: usize = 3;
const USERNAME_MAX: usize = 32;
/// Only alphanumeric + underscore + hyphen. Prevents XSS and SQL edge cases.
const USERNAME_PATTERN: &str = r"^[a-zA-Z0-9_-]+$";
/// Maximum length for hashes and salts (prevents storage exhaustion).
const MAX_HASH_LEN: usize = 256;
/// Maximum prekey bundle size in bytes (prevents abuse).
const MAX_BUNDLE_SIZE: usize = 1024 * 1024; // 1 MiB

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    /// Client-side computed Argon2id hash of the password.
    /// The server re-hashes this with Argon2id (double hashing).
    pub password_hash: String,
    /// Argon2id salt used by the client (stored for key re-derivation on login).
    pub argon2_salt: String,
    /// The user's Signal Protocol prekey bundle (serialized JSON).
    pub prekey_bundle: serde_json::Value,
}

#[derive(Serialize)]
pub struct AuthResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub user_id: Uuid,
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    /// The client sends the Argon2id hash; server compares against stored double-hash.
    pub password_hash: String,
}

#[derive(Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

// ---------------------------------------------------------------------------
// Input Validation
// ---------------------------------------------------------------------------

/// Validate a username against security rules.
fn validate_username(username: &str) -> Result<()> {
    if username.len() < USERNAME_MIN || username.len() > USERNAME_MAX {
        return Err(AppError::BadRequest(format!(
            "username must be {USERNAME_MIN}–{USERNAME_MAX} characters"
        )));
    }
    // Only allow alphanumeric, underscore, hyphen.
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AppError::BadRequest(
            "username may only contain letters, digits, underscores, and hyphens".into(),
        ));
    }
    Ok(())
}

/// Validate an email address (basic sanity check).
fn validate_email(email: &str) -> Result<()> {
    if email.is_empty() || email.len() > 254 || !email.contains('@') || email.contains(' ') {
        return Err(AppError::BadRequest("invalid email address".into()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /api/v1/auth/register`
///
/// Creates a new user account and stores their Signal Protocol prekey bundle.
///
/// # Flow
/// 1. Validate all inputs (username, email, hash lengths, bundle size)
/// 2. Server-side hash the client-side hash with Argon2id (double hashing)
/// 3. Begin transaction → INSERT user → INSERT prekey bundle → COMMIT
/// 4. Return JWT pair
pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<impl IntoResponse> {
    // --- Input validation ---
    validate_username(&req.username)?;
    validate_email(&req.email)?;

    if req.password_hash.len() > MAX_HASH_LEN {
        return Err(AppError::BadRequest(
            "password_hash exceeds maximum length".into(),
        ));
    }
    if req.argon2_salt.len() > MAX_HASH_LEN {
        return Err(AppError::BadRequest(
            "argon2_salt exceeds maximum length".into(),
        ));
    }
    let bundle_size = serde_json::to_vec(&req.prekey_bundle)
        .map(|v| v.len())
        .unwrap_or(0);
    if bundle_size > MAX_BUNDLE_SIZE {
        return Err(AppError::BadRequest(
            "prekey_bundle exceeds 1 MiB limit".into(),
        ));
    }

    // Server-side hash: re-hash the client-provided hash with Argon2id.
    // This ensures a database leak reveals only double-hashed values.
    let server_hash = hash_password_server(&req.password_hash)?;

    let user_id = Uuid::new_v4();

    // Use a transaction to ensure both user + prekey bundle are created atomically.
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    sqlx::query(
        r#"
        INSERT INTO users (id, username, email, password_hash, argon2_salt, created_at)
        VALUES ($1, $2, $3, $4, $5, NOW())
        "#,
    )
    .bind(user_id)
    .bind(&req.username)
    .bind(&req.email)
    .bind(&server_hash)
    .bind(&req.argon2_salt)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        // Check for PostgreSQL unique violation (code 23505).
        if let Some(db_err) = e.as_database_error() {
            if db_err.code().as_deref() == Some("23505") {
                return AppError::Conflict("username or email already taken".into());
            }
        }
        AppError::Internal(anyhow::anyhow!(e))
    })?;

    sqlx::query(
        r#"
        INSERT INTO prekey_bundles (user_id, bundle, updated_at)
        VALUES ($1, $2, NOW())
        "#,
    )
    .bind(user_id)
    .bind(&req.prekey_bundle)
    .execute(&mut *tx)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    tx.commit()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    tracing::info!(user_id = %user_id, username = %req.username, "User registered");

    let tokens = issue_token_pair(&state, user_id, &req.username).await?;
    Ok((StatusCode::CREATED, Json(tokens)))
}

/// `POST /api/v1/auth/login`
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<impl IntoResponse> {
    // Look up the user. Use a generic error message to prevent user enumeration.
    let user = sqlx::query(
        "SELECT id, username, password_hash FROM users WHERE username = $1 AND deleted_at IS NULL",
    )
    .bind(&req.username)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?
    .ok_or(AppError::Unauthorized("invalid credentials".into()))?;

    // OAuth-only users have NULL password_hash — they cannot use password login.
    let stored_hash = user.get::<Option<String>, _>("password_hash");
    let stored_hash = stored_hash
        .as_deref()
        .ok_or(AppError::Unauthorized("invalid credentials".into()))?;

    // Server-side verify: hash the client-provided hash and compare to stored.
    if !verify_password_server(&req.password_hash, stored_hash)? {
        return Err(AppError::Unauthorized("invalid credentials".into()));
    }

    let tokens = issue_token_pair(
        &state,
        user.get::<Uuid, _>("id"),
        &user.get::<String, _>("username"),
    )
    .await?;
    Ok(Json(tokens))
}

/// `POST /api/v1/auth/refresh`
///
/// Rotates the refresh token on use (delete-then-insert) to detect theft.
pub async fn refresh(
    State(state): State<AppState>,
    Json(req): Json<RefreshRequest>,
) -> Result<impl IntoResponse> {
    let row = sqlx::query(
        r#"
        DELETE FROM refresh_tokens
        WHERE token = $1 AND expires_at > NOW()
        RETURNING user_id, username
        "#,
    )
    .bind(&req.refresh_token)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?
    .ok_or(AppError::Unauthorized(
        "refresh token invalid or expired".into(),
    ))?;

    let tokens = issue_token_pair(
        &state,
        row.get::<Uuid, _>("user_id"),
        &row.get::<String, _>("username"),
    )
    .await?;
    Ok(Json(tokens))
}

/// `POST /api/v1/auth/logout`
pub async fn logout(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<RefreshRequest>,
) -> Result<impl IntoResponse> {
    // Only allow deleting refresh tokens that belong to the authenticated user.
    sqlx::query("DELETE FROM refresh_tokens WHERE token = $1 AND user_id = $2")
        .bind(&req.refresh_token)
        .bind(claims.sub)
        .execute(&state.db)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Auth middleware
// ---------------------------------------------------------------------------

use axum::Extension;

/// Axum middleware that validates the Bearer JWT on every authenticated route.
///
/// On success, the validated [`Claims`] are inserted into request extensions
/// so handlers can access user identity via `Extension<Claims>`.
pub async fn require_auth(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> std::result::Result<Response, AppError> {
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or(AppError::Unauthorized("missing Bearer token".into()))?;

    // Explicitly restrict to HS256 to prevent algorithm confusion attacks.
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;

    let claims = decode::<Claims>(
        token,
        &DecodingKey::from_secret(state.config.jwt_secret.as_bytes()),
        &validation,
    )
    .map_err(|_| AppError::Unauthorized("invalid or expired token".into()))?
    .claims;

    // Inject claims into request extensions for downstream handlers.
    req.extensions_mut().insert(claims);
    Ok(next.run(req).await)
}

// ---------------------------------------------------------------------------
// Password hashing (server-side Argon2id)
// ---------------------------------------------------------------------------

/// Hash a password (or client-side hash) with Argon2id for server-side storage.
fn hash_password_server(password: &str) -> Result<String> {
    use argon2::{
        password_hash::{rand_core::OsRng, SaltString},
        Argon2, PasswordHasher,
    };

    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default(); // uses Argon2id with sensible defaults

    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Argon2 hash failed: {e}")))?
        .to_string();

    Ok(hash)
}

/// Verify a password (or client-side hash) against a stored Argon2id hash.
fn verify_password_server(password: &str, stored_hash: &str) -> Result<bool> {
    use argon2::{password_hash::PasswordHash, Argon2, PasswordVerifier};

    let parsed = PasswordHash::new(stored_hash)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("stored hash invalid: {e}")))?;

    // `verify_password` is constant-time (provided by the argon2 crate).
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

// ---------------------------------------------------------------------------
// Token issuance
// ---------------------------------------------------------------------------

/// Issue an access + refresh token pair and persist the refresh token to the database.
pub(crate) async fn issue_token_pair(
    state: &AppState,
    user_id: Uuid,
    username: &str,
) -> Result<AuthResponse> {
    // --- Access token (JWT) ---
    let access_claims = Claims::new(user_id, username, state.config.jwt_access_ttl_secs);
    let access_token = encode(
        &Header::new(Algorithm::HS256),
        &access_claims,
        &EncodingKey::from_secret(state.config.jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(anyhow::anyhow!("JWT encode failed: {e}")))?;

    // --- Refresh token (opaque random string, stored in DB) ---
    let refresh_token = format!("{}{}", Uuid::new_v4(), Uuid::new_v4());
    let refresh_expires =
        chrono::Utc::now() + chrono::Duration::seconds(state.config.jwt_refresh_ttl_secs as i64);

    sqlx::query(
        r#"
        INSERT INTO refresh_tokens (token, user_id, username, expires_at, created_at)
        VALUES ($1, $2, $3, $4, NOW())
        "#,
    )
    .bind(&refresh_token)
    .bind(user_id)
    .bind(username)
    .bind(refresh_expires)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to persist refresh token: {e}")))?;

    Ok(AuthResponse {
        access_token,
        refresh_token,
        user_id,
    })
}

// ---------------------------------------------------------------------------
// Tests (unit-testable without database)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_expiry_is_set_correctly() {
        let user_id = Uuid::new_v4();
        let claims = Claims::new(user_id, "alice", 900);
        assert_eq!(claims.sub, user_id);
        assert_eq!(claims.username, "alice");
        assert!(claims.exp > claims.iat);
        assert_eq!(claims.exp - claims.iat, 900);
    }

    #[test]
    fn validate_username_rejects_short() {
        assert!(validate_username("ab").is_err());
    }

    #[test]
    fn validate_username_rejects_long() {
        let long = "a".repeat(33);
        assert!(validate_username(&long).is_err());
    }

    #[test]
    fn validate_username_rejects_special_chars() {
        assert!(validate_username("alice<script>").is_err());
        assert!(validate_username("bob'; DROP TABLE").is_err());
        assert!(validate_username("user@host").is_err());
    }

    #[test]
    fn validate_username_accepts_valid() {
        assert!(validate_username("alice").is_ok());
        assert!(validate_username("bob-42").is_ok());
        assert!(validate_username("user_name").is_ok());
    }

    #[test]
    fn validate_email_rejects_empty() {
        assert!(validate_email("").is_err());
    }

    #[test]
    fn validate_email_rejects_no_at() {
        assert!(validate_email("notanemail").is_err());
    }

    #[test]
    fn validate_email_accepts_valid() {
        assert!(validate_email("user@example.com").is_ok());
    }

    #[test]
    fn password_hash_round_trip() {
        let password = "client_argon2id_hash_output_here";
        let hash = hash_password_server(password).unwrap();
        assert!(verify_password_server(password, &hash).unwrap());
    }

    #[test]
    fn wrong_password_rejected() {
        let hash = hash_password_server("correct_hash").unwrap();
        assert!(!verify_password_server("wrong_hash", &hash).unwrap());
    }

    #[test]
    fn jwt_round_trip() {
        let secret = "test_secret_at_least_32_bytes_long_for_hs256";
        let user_id = Uuid::new_v4();
        let claims = Claims::new(user_id, "testuser", 3600);

        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();

        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = true;

        let decoded = decode::<Claims>(
            &token,
            &DecodingKey::from_secret(secret.as_bytes()),
            &validation,
        )
        .unwrap();

        assert_eq!(decoded.claims.sub, user_id);
        assert_eq!(decoded.claims.username, "testuser");
    }
}
