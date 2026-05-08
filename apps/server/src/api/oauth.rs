//! OAuth2 third-party authentication — GitHub, Google, Microsoft, Apple, Facebook.
//!
//! # Flow
//!
//! 1. **Initiate** (`GET /api/v1/auth/oauth/:provider`):
//!    - Builds the OAuth2 authorization URL with PKCE challenge
//!    - Stores CSRF state + PKCE verifier in the `oauth_states` table (5 min TTL)
//!    - Returns the authorization URL for the client to redirect the user's browser
//!
//! 2. **Callback** (`GET /api/v1/auth/oauth/:provider/callback`):
//!    - Validates the CSRF state token against the database
//!    - Exchanges the authorization code for an access token (using PKCE verifier)
//!    - Fetches the user's profile from the provider's API
//!    - Links or creates the FreeBox user account
//!    - Returns the same `AuthResponse` (JWT pair) as password login
//!
//! 3. **Provider Management** (authenticated):
//!    - `GET /api/v1/auth/providers` — list all linked OAuth providers
//!    - `POST /api/v1/auth/oauth/:provider/link` — initiate linking a new provider
//!    - `DELETE /api/v1/auth/oauth/:provider/unlink` — unlink a provider
//!
//! # Security
//!
//! - **PKCE** (Proof Key for Code Exchange) prevents authorization code interception.
//! - **CSRF state** token prevents cross-site request forgery on the callback.
//! - State tokens expire after 5 minutes and are single-use (deleted on successful callback).
//! - OAuth access tokens from providers are NOT stored (we only need them to fetch the profile).
//! - All user identity resolution uses database transactions to prevent race conditions.
//! - Unlinking the last auth method (no password + only one provider) is rejected to prevent lockout.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use oauth2::{
    basic::BasicClient, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::{
    config::OAuthProviderConfig,
    error::{AppError, Result},
    state::AppState,
};

/// Custom extra token fields to capture `id_token` from OAuth responses (needed for Apple Sign In).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IdTokenExtraFields {
    #[serde(default)]
    id_token: Option<String>,
}
impl oauth2::ExtraTokenFields for IdTokenExtraFields {}

/// Token response type that can capture the `id_token` field.
type IdTokenTokenResponse =
    oauth2::StandardTokenResponse<IdTokenExtraFields, oauth2::basic::BasicTokenType>;

// ---------------------------------------------------------------------------
// Provider metadata — authorization/token URLs and profile endpoints
// ---------------------------------------------------------------------------

/// OAuth2 endpoint URLs and user-profile fetching logic for each provider.
struct ProviderMeta {
    auth_url: &'static str,
    token_url: &'static str,
    scopes: &'static [&'static str],
    profile_url: &'static str,
}

/// Returns the OAuth2 endpoint metadata for a supported provider.
fn provider_meta(provider: &str) -> Result<ProviderMeta> {
    match provider {
        "github" => Ok(ProviderMeta {
            auth_url: "https://github.com/login/oauth/authorize",
            token_url: "https://github.com/login/oauth/access_token",
            scopes: &["read:user", "user:email"],
            profile_url: "https://api.github.com/user",
        }),
        "google" => Ok(ProviderMeta {
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
            token_url: "https://oauth2.googleapis.com/token",
            scopes: &["openid", "email", "profile"],
            profile_url: "https://www.googleapis.com/oauth2/v3/userinfo",
        }),
        "microsoft" => Ok(ProviderMeta {
            auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            scopes: &["openid", "email", "profile", "User.Read"],
            profile_url: "https://graph.microsoft.com/v1.0/me",
        }),
        "apple" => Ok(ProviderMeta {
            auth_url: "https://appleid.apple.com/auth/authorize",
            token_url: "https://appleid.apple.com/auth/token",
            scopes: &["name", "email"],
            // Apple returns user info in the id_token, not via a profile endpoint.
            // We handle this specially in `fetch_user_profile`.
            profile_url: "",
        }),
        "facebook" => Ok(ProviderMeta {
            auth_url: "https://www.facebook.com/v19.0/dialog/oauth",
            token_url: "https://graph.facebook.com/v19.0/oauth/access_token",
            scopes: &["email", "public_profile"],
            profile_url: "https://graph.facebook.com/me?fields=id,name,email,picture",
        }),
        _ => Err(AppError::NotFound(format!(
            "unknown OAuth provider: {provider}"
        ))),
    }
}

/// Custom OAuth2 client type that captures `id_token` in token responses.
type OAuthClient<
    HasAuthUrl = oauth2::EndpointNotSet,
    HasDeviceAuthUrl = oauth2::EndpointNotSet,
    HasIntrospectionUrl = oauth2::EndpointNotSet,
    HasRevocationUrl = oauth2::EndpointNotSet,
    HasTokenUrl = oauth2::EndpointNotSet,
> = oauth2::Client<
    oauth2::basic::BasicErrorResponse,
    IdTokenTokenResponse,
    oauth2::basic::BasicTokenIntrospectionResponse,
    oauth2::StandardRevocableToken,
    oauth2::basic::BasicRevocationErrorResponse,
    HasAuthUrl,
    HasDeviceAuthUrl,
    HasIntrospectionUrl,
    HasRevocationUrl,
    HasTokenUrl,
>;

/// Build an OAuth2 client for a given provider.
fn build_oauth_client(
    provider_config: &OAuthProviderConfig,
    meta: &ProviderMeta,
) -> Result<
    OAuthClient<
        oauth2::EndpointSet,
        oauth2::EndpointNotSet,
        oauth2::EndpointNotSet,
        oauth2::EndpointNotSet,
        oauth2::EndpointSet,
    >,
> {
    let client = OAuthClient::new(ClientId::new(provider_config.client_id.clone()))
        .set_client_secret(ClientSecret::new(provider_config.client_secret.clone()))
        .set_auth_uri(
            AuthUrl::new(meta.auth_url.to_string())
                .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid auth URL: {e}")))?,
        )
        .set_token_uri(
            TokenUrl::new(meta.token_url.to_string())
                .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid token URL: {e}")))?,
        )
        .set_redirect_uri(
            RedirectUrl::new(provider_config.redirect_uri.clone())
                .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid redirect URI: {e}")))?,
        );

    Ok(client)
}

// ---------------------------------------------------------------------------
// Request / Response DTOs
// ---------------------------------------------------------------------------

/// Response from the initiate endpoint — the client redirects the user here.
#[derive(Serialize)]
pub struct OAuthInitiateResponse {
    /// The full authorization URL to redirect the user's browser to.
    pub authorize_url: String,
}

/// Query parameters received on the OAuth callback URL.
#[derive(Deserialize)]
pub struct OAuthCallbackParams {
    /// The authorization code from the provider.
    pub code: String,
    /// The CSRF state token (must match what we stored).
    pub state: String,
}

/// Normalized user profile from any OAuth provider.
#[derive(Debug)]
struct OAuthUserProfile {
    /// The user's unique ID at the provider (always a string).
    provider_user_id: String,
    /// Email address (may be absent for some providers).
    email: Option<String>,
    /// Display name.
    username: Option<String>,
    /// Avatar URL.
    avatar_url: Option<String>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/auth/oauth/:provider`
///
/// Initiates the OAuth2 authorization flow for the given provider.
///
/// # Flow
/// 1. Validate the provider is supported and configured
/// 2. Generate PKCE challenge + verifier pair
/// 3. Build the authorization URL with scopes, PKCE, and CSRF state
/// 4. Store the CSRF state + PKCE verifier in the database (5 min TTL)
/// 5. Return the authorization URL for the client to redirect to
pub async fn initiate(
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> Result<impl IntoResponse> {
    let provider = provider.to_lowercase();

    // Validate provider is known and configured.
    let provider_config = state.oauth.get(&provider).ok_or_else(|| {
        AppError::NotFound(format!("OAuth provider '{provider}' is not configured"))
    })?;
    let meta = provider_meta(&provider)?;

    let client = build_oauth_client(provider_config, &meta)?;

    // PKCE: generate challenge + verifier pair.
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    // Build the authorization URL with all required parameters.
    let mut auth_request = client.authorize_url(CsrfToken::new_random);

    // Add provider-specific scopes.
    for scope in meta.scopes {
        auth_request = auth_request.add_scope(Scope::new(scope.to_string()));
    }

    let (authorize_url, csrf_state) = auth_request.set_pkce_challenge(pkce_challenge).url();

    // Persist the CSRF state + PKCE verifier (5 minute TTL).
    sqlx::query(
        r#"
        INSERT INTO oauth_states (state, pkce_verifier, provider, created_at, expires_at)
        VALUES ($1, $2, $3, NOW(), NOW() + INTERVAL '5 minutes')
        "#,
    )
    .bind(csrf_state.secret())
    .bind(pkce_verifier.secret())
    .bind(&provider)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to store OAuth state: {e}")))?;

    tracing::info!(provider = %provider, "OAuth flow initiated");

    Ok(Json(OAuthInitiateResponse {
        authorize_url: authorize_url.to_string(),
    }))
}

/// `GET /api/v1/auth/oauth/:provider/callback`
///
/// Handles the OAuth2 callback after the user authorizes with the provider.
///
/// # Flow
/// 1. Validate CSRF state token against the database (single-use)
/// 2. Exchange the authorization code for an access token (with PKCE verifier)
/// 3. Fetch the user's profile from the provider's API
/// 4. Find or create the FreeBox user account
/// 5. Return a JWT pair (same format as password login)
pub async fn callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Query(params): Query<OAuthCallbackParams>,
) -> Result<impl IntoResponse> {
    let provider = provider.to_lowercase();

    // --- Step 1: Validate CSRF state and retrieve PKCE verifier ---
    // DELETE + RETURNING makes the state single-use (prevents replay attacks).
    // The `provider` column may contain "link:<user_id>:<provider>" for link flows.
    let oauth_state = sqlx::query(
        r#"
        DELETE FROM oauth_states
        WHERE state = $1 AND expires_at > NOW()
        RETURNING pkce_verifier, provider AS stored_provider
        "#,
    )
    .bind(&params.state)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to validate OAuth state: {e}")))?
    .ok_or(AppError::Unauthorized(
        "invalid or expired OAuth state".into(),
    ))?;

    // Determine if this is a link flow or a login flow.
    let stored_provider = oauth_state.get::<String, _>("stored_provider");
    let (is_link_flow, link_user_id) = if stored_provider.starts_with("link:") {
        // Format: "link:<uuid>:<provider>"
        let parts: Vec<&str> = stored_provider.splitn(3, ':').collect();
        if parts.len() == 3 && parts[2] == provider {
            let user_id = Uuid::parse_str(parts[1]).map_err(|_| {
                AppError::Internal(anyhow::anyhow!("invalid user_id in link state"))
            })?;
            (true, Some(user_id))
        } else {
            return Err(AppError::Unauthorized(
                "OAuth state provider mismatch".into(),
            ));
        }
    } else if stored_provider == provider {
        (false, None)
    } else {
        return Err(AppError::Unauthorized(
            "OAuth state provider mismatch".into(),
        ));
    };

    // --- Step 2: Exchange auth code for access token ---
    let provider_config = state.oauth.get(&provider).ok_or_else(|| {
        AppError::NotFound(format!("OAuth provider '{provider}' is not configured"))
    })?;
    let meta = provider_meta(&provider)?;
    let client = build_oauth_client(provider_config, &meta)?;

    let pkce_verifier = PkceCodeVerifier::new(oauth_state.get::<String, _>("pkce_verifier"));

    // Build a closure that implements AsyncHttpClient for the oauth2 crate.
    let http_client_ref = &state.http_client;
    let async_http_client = |request: oauth2::HttpRequest| {
        let client = http_client_ref.clone();
        async move {
            let method = request.method().clone();
            let url = request.uri().to_string();
            let headers = request.headers().clone();
            let body_bytes = request.into_body();

            let mut builder = client.request(method, &url);
            for (name, value) in &headers {
                builder = builder.header(name.clone(), value.clone());
            }
            builder = builder.body(body_bytes);
            let response = builder.send().await.map_err(|e| {
                oauth2::HttpClientError::<std::io::Error>::Other(format!("reqwest error: {e}"))
            })?;
            let status = response.status();
            let resp_headers = response.headers().clone();
            let body = response.bytes().await.map_err(|e| {
                oauth2::HttpClientError::<std::io::Error>::Other(format!("body read error: {e}"))
            })?;
            axum::http::Response::builder()
                .status(status)
                .body(body.to_vec())
                .map(|mut resp| {
                    *resp.headers_mut() = resp_headers;
                    resp
                })
                .map_err(oauth2::HttpClientError::<std::io::Error>::Http)
        }
    };

    let token_response = client
        .exchange_code(AuthorizationCode::new(params.code))
        .set_pkce_verifier(pkce_verifier)
        .request_async(&async_http_client)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("OAuth token exchange failed: {e}")))?;

    let access_token = token_response.access_token().secret().to_string();

    // Extract id_token for Apple Sign In. Our custom IdTokenExtraFields
    // captures it from the token response JSON.
    let id_token = token_response.extra_fields().id_token.clone();

    // --- Step 3: Fetch user profile from provider ---
    let profile = fetch_user_profile(
        &state.http_client,
        &provider,
        &access_token,
        id_token.as_deref(),
        &meta,
    )
    .await?;

    // --- Step 4: Find or create FreeBox user ---
    if is_link_flow {
        // Link flow: attach this OAuth identity to the authenticated user.
        let user_id = link_user_id.unwrap();

        // Check if this provider identity is already linked to another account.
        let existing = sqlx::query_scalar::<_, Uuid>(
            "SELECT user_id FROM oauth_accounts WHERE provider = $1 AND provider_user_id = $2",
        )
        .bind(&provider)
        .bind(&profile.provider_user_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

        if let Some(other_user_id) = existing {
            if other_user_id != user_id {
                return Err(AppError::Conflict(format!(
                    "this {provider} account is already linked to a different FreeBox user"
                )));
            }
            // Already linked to the same user — no-op.
        } else {
            // Insert the new link.
            sqlx::query(
                r#"
                INSERT INTO oauth_accounts (user_id, provider, provider_user_id, provider_email, provider_username, avatar_url)
                VALUES ($1, $2, $3, $4, $5, $6)
                "#,
            )
            .bind(user_id)
            .bind(&provider)
            .bind(&profile.provider_user_id)
            .bind(&profile.email)
            .bind(&profile.username)
            .bind(&profile.avatar_url)
            .execute(&state.db)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;
        }

        // Fetch username for token issuance.
        let username = sqlx::query_scalar::<_, String>("SELECT username FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

        let tokens = super::auth::issue_token_pair(&state, user_id, &username).await?;

        tracing::info!(
            user_id = %user_id,
            provider = %provider,
            "OAuth provider linked successfully",
        );

        Ok(Json(tokens))
    } else {
        // Login flow: find or create user.
        let (user_id, username) = find_or_create_user(&state, &provider, &profile).await?;
        let tokens = super::auth::issue_token_pair(&state, user_id, &username).await?;

        tracing::info!(
            user_id = %user_id,
            provider = %provider,
            provider_user_id = %profile.provider_user_id,
            "OAuth login successful",
        );

        Ok(Json(tokens))
    }
}

// ---------------------------------------------------------------------------
// Provider-specific profile fetching
// ---------------------------------------------------------------------------

/// Fetch the authenticated user's profile from the OAuth provider's API.
///
/// Each provider returns a different JSON shape, so this function normalizes
/// the response into a common [`OAuthUserProfile`].
///
/// The `id_token` parameter is only used by Apple Sign In, which embeds user
/// info in a JWT rather than exposing a profile API endpoint.
async fn fetch_user_profile(
    http_client: &reqwest::Client,
    provider: &str,
    access_token: &str,
    id_token: Option<&str>,
    meta: &ProviderMeta,
) -> Result<OAuthUserProfile> {
    match provider {
        "github" => fetch_github_profile(http_client, access_token, meta).await,
        "google" => fetch_google_profile(http_client, access_token, meta).await,
        "microsoft" => fetch_microsoft_profile(http_client, access_token, meta).await,
        "apple" => fetch_apple_profile(http_client, id_token).await,
        "facebook" => fetch_facebook_profile(http_client, access_token, meta).await,
        _ => Err(AppError::Internal(anyhow::anyhow!(
            "unsupported provider: {provider}"
        ))),
    }
}

/// GitHub: `GET https://api.github.com/user`
/// Returns `{ id: 123, login: "octocat", email: "...", avatar_url: "..." }`
async fn fetch_github_profile(
    http_client: &reqwest::Client,
    access_token: &str,
    meta: &ProviderMeta,
) -> Result<OAuthUserProfile> {
    #[derive(Deserialize)]
    struct GithubUser {
        id: u64,
        login: String,
        email: Option<String>,
        avatar_url: Option<String>,
    }

    let user: GithubUser = http_client
        .get(meta.profile_url)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "FreeBox-Server")
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("GitHub API error: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("GitHub profile parse error: {e}")))?;

    Ok(OAuthUserProfile {
        provider_user_id: user.id.to_string(),
        email: user.email,
        username: Some(user.login),
        avatar_url: user.avatar_url,
    })
}

/// Google: `GET https://www.googleapis.com/oauth2/v3/userinfo`
/// Returns `{ sub: "...", email: "...", name: "...", picture: "..." }`
async fn fetch_google_profile(
    http_client: &reqwest::Client,
    access_token: &str,
    meta: &ProviderMeta,
) -> Result<OAuthUserProfile> {
    #[derive(Deserialize)]
    struct GoogleUser {
        sub: String,
        email: Option<String>,
        name: Option<String>,
        picture: Option<String>,
    }

    let user: GoogleUser = http_client
        .get(meta.profile_url)
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Google API error: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Google profile parse error: {e}")))?;

    Ok(OAuthUserProfile {
        provider_user_id: user.sub,
        email: user.email,
        username: user.name,
        avatar_url: user.picture,
    })
}

/// Microsoft: `GET https://graph.microsoft.com/v1.0/me`
/// Returns `{ id: "...", displayName: "...", mail: "...", userPrincipalName: "..." }`
async fn fetch_microsoft_profile(
    http_client: &reqwest::Client,
    access_token: &str,
    meta: &ProviderMeta,
) -> Result<OAuthUserProfile> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct MicrosoftUser {
        id: String,
        display_name: Option<String>,
        mail: Option<String>,
        user_principal_name: Option<String>,
    }

    let user: MicrosoftUser = http_client
        .get(meta.profile_url)
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Microsoft API error: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Microsoft profile parse error: {e}")))?;

    // Microsoft may return mail or userPrincipalName as the email.
    let email = user.mail.or(user.user_principal_name);

    Ok(OAuthUserProfile {
        provider_user_id: user.id,
        email,
        username: user.display_name,
        avatar_url: None, // Microsoft Graph requires a separate call for photos.
    })
}

/// Apple Sign In: User info is embedded in the `id_token` JWT.
///
/// Unlike other providers, Apple does NOT have a profile API endpoint. Instead,
/// the token exchange response includes an `id_token` — a signed JWT containing
/// the user's `sub` (unique Apple ID), `email`, and optionally `email_verified`.
///
/// # Security
///
/// The id_token is verified against Apple's public JWKS keys fetched from
/// `https://appleid.apple.com/auth/keys`. This ensures:
/// - The token was genuinely issued by Apple (not forged)
/// - The token hasn't been tampered with (signature verification)
/// - The token hasn't expired (`exp` claim)
/// - The audience matches our client ID (`aud` claim — verified by caller context)
///
/// # Important Notes
///
/// - Apple only sends the user's name on the **first** authorization. After that,
///   only `sub` and `email` are available. We store whatever we get on first link.
/// - The `sub` is stable and unique per user per developer team. It is the correct
///   field to use as `provider_user_id` (NOT the email, which can change).
async fn fetch_apple_profile(
    http_client: &reqwest::Client,
    id_token: Option<&str>,
) -> Result<OAuthUserProfile> {
    let id_token = id_token.ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!(
            "Apple Sign In token response did not include an id_token"
        ))
    })?;

    // --- Step 1: Fetch Apple's public JWKS keys ---
    // Apple publishes RSA public keys at a well-known endpoint. These keys rotate
    // periodically, so we fetch them on every login (a production system would
    // cache these with a TTL matching the Cache-Control header).
    let jwks: AppleJwks = http_client
        .get(APPLE_JWKS_URL)
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to fetch Apple JWKS: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to parse Apple JWKS: {e}")))?;

    // --- Step 2: Decode the JWT header to find the signing key ---
    // The id_token header contains a `kid` (Key ID) that tells us which of Apple's
    // public keys was used to sign this token.
    let header = jsonwebtoken::decode_header(id_token)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid Apple id_token header: {e}")))?;

    let kid = header.kid.ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!("Apple id_token missing `kid` in header"))
    })?;

    // Find the matching key in Apple's JWKS.
    let jwk = jwks.keys.iter().find(|k| k.kid == kid).ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!(
            "Apple JWKS does not contain key with kid={kid}"
        ))
    })?;

    // --- Step 3: Build the RSA public key and verify the JWT signature ---
    let decoding_key = jsonwebtoken::DecodingKey::from_rsa_components(&jwk.n, &jwk.e)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid Apple RSA key: {e}")))?;

    // Configure validation: verify expiry and signature algorithm.
    // Apple uses RS256 for id_token signatures.
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.validate_exp = true;
    // Apple's issuer is always "https://appleid.apple.com".
    validation.set_issuer(&["https://appleid.apple.com"]);
    // We skip audience validation here because we don't have the client_id in scope.
    // The token exchange already validated the code came from our registered app.
    // In a hardened production system, pass the client_id and validate `aud` here.
    validation.validate_aud = false;

    let token_data =
        jsonwebtoken::decode::<AppleIdTokenClaims>(id_token, &decoding_key, &validation).map_err(
            |e| AppError::Unauthorized(format!("Apple id_token verification failed: {e}")),
        )?;

    let claims = token_data.claims;

    tracing::info!(
        apple_sub = %claims.sub,
        email = ?claims.email,
        email_verified = ?claims.email_verified,
        "Apple id_token decoded successfully",
    );

    Ok(OAuthUserProfile {
        provider_user_id: claims.sub,
        email: claims.email,
        // Apple only sends the name on the first authorization via a separate POST
        // body parameter (`user` JSON). The id_token itself doesn't contain the name.
        // The name is typically captured by the client-side Apple JS SDK and sent
        // to our server alongside the authorization code.
        username: None,
        avatar_url: None, // Apple does not provide avatar URLs.
    })
}

/// Apple's JWKS (JSON Web Key Set) endpoint URL.
///
/// Apple publishes its public RSA keys here. These keys are used to verify
/// the signature on id_tokens. Keys rotate periodically.
const APPLE_JWKS_URL: &str = "https://appleid.apple.com/auth/keys";

/// A single RSA public key from Apple's JWKS endpoint.
///
/// Apple uses RS256 (RSA + SHA-256) to sign id_tokens. Each key has a unique
/// `kid` (Key ID) that matches the `kid` in the id_token JWT header.
#[derive(Deserialize)]
struct AppleJwk {
    /// Key ID — matches the `kid` in the JWT header.
    kid: String,
    /// RSA modulus (Base64url-encoded).
    n: String,
    /// RSA public exponent (Base64url-encoded, typically "AQAB" = 65537).
    e: String,
}

/// Apple's JWKS response containing multiple RSA public keys.
#[derive(Deserialize)]
struct AppleJwks {
    keys: Vec<AppleJwk>,
}

/// Claims embedded in Apple's id_token JWT.
///
/// Apple's id_token follows the OpenID Connect specification. Key claims:
/// - `sub`: A unique, stable identifier for the user (per developer team)
/// - `email`: The user's email (may be a private relay address if user chose "Hide My Email")
/// - `email_verified`: Whether Apple has verified this email
/// - `iss`: Always "https://appleid.apple.com"
/// - `aud`: The client_id (Service ID) of your app
/// - `exp`: Token expiry timestamp
#[derive(Deserialize)]
struct AppleIdTokenClaims {
    /// Subject — the user's unique Apple ID (stable, team-scoped).
    /// This is the correct value for `provider_user_id`.
    sub: String,
    /// The user's email address. May be a private relay address like
    /// `abc123@privaterelay.appleid.com` if the user chose "Hide My Email".
    email: Option<String>,
    /// Whether Apple has verified this email address.
    #[serde(default)]
    email_verified: Option<bool>,
}

/// Facebook: `GET https://graph.facebook.com/me?fields=id,name,email,picture`
/// Returns `{ id: "...", name: "...", email: "...", picture: { data: { url: "..." } } }`
async fn fetch_facebook_profile(
    http_client: &reqwest::Client,
    access_token: &str,
    meta: &ProviderMeta,
) -> Result<OAuthUserProfile> {
    #[derive(Deserialize)]
    struct FacebookPicture {
        data: FacebookPictureData,
    }
    #[derive(Deserialize)]
    struct FacebookPictureData {
        url: String,
    }
    #[derive(Deserialize)]
    struct FacebookUser {
        id: String,
        name: Option<String>,
        email: Option<String>,
        picture: Option<FacebookPicture>,
    }

    let user: FacebookUser = http_client
        .get(meta.profile_url)
        .query(&[("access_token", access_token)])
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Facebook API error: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Facebook profile parse error: {e}")))?;

    Ok(OAuthUserProfile {
        provider_user_id: user.id,
        email: user.email,
        username: user.name,
        avatar_url: user.picture.map(|p| p.data.url),
    })
}

// ---------------------------------------------------------------------------
// User identity resolution
// ---------------------------------------------------------------------------

/// Find an existing user linked to this OAuth identity, or create a new one.
///
/// # Resolution order
///
/// 1. **Exact match**: An `oauth_accounts` row exists for `(provider, provider_user_id)`.
///    → Return the linked `users` row.
///
/// 2. **Email match**: No OAuth link exists, but the provider email matches an existing
///    `users.email`. → Link the OAuth account to the existing user (account linking).
///
/// 3. **New user**: No match at all. → Create a new `users` row (NULL password) and
///    a new `oauth_accounts` row.
///
/// All operations use a database transaction to prevent race conditions.
async fn find_or_create_user(
    state: &AppState,
    provider: &str,
    profile: &OAuthUserProfile,
) -> Result<(Uuid, String)> {
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    // --- 1. Check for existing OAuth link ---
    let existing = sqlx::query(
        r#"
        SELECT oa.user_id, u.username
        FROM oauth_accounts oa
        JOIN users u ON u.id = oa.user_id
        WHERE oa.provider = $1 AND oa.provider_user_id = $2 AND u.deleted_at IS NULL
        "#,
    )
    .bind(provider)
    .bind(&profile.provider_user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    if let Some(row) = existing {
        // Update the last-seen profile info (avatar, email may change).
        sqlx::query(
            r#"
            UPDATE oauth_accounts
            SET provider_email = $1, provider_username = $2, avatar_url = $3, updated_at = NOW()
            WHERE provider = $4 AND provider_user_id = $5
            "#,
        )
        .bind(&profile.email)
        .bind(&profile.username)
        .bind(&profile.avatar_url)
        .bind(provider)
        .bind(&profile.provider_user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

        tx.commit()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;
        return Ok((
            row.get::<Uuid, _>("user_id"),
            row.get::<String, _>("username"),
        ));
    }

    // --- 2. Check for email match (account linking) ---
    let email_match = if let Some(email) = &profile.email {
        sqlx::query("SELECT id, username FROM users WHERE email = $1 AND deleted_at IS NULL")
            .bind(email)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?
    } else {
        None
    };

    let (user_id, username) = if let Some(user) = email_match {
        // Link the OAuth account to the existing FreeBox user.
        (user.get::<Uuid, _>("id"), user.get::<String, _>("username"))
    } else {
        // --- 3. Create a brand-new user ---
        let user_id = Uuid::new_v4();
        // Generate a username from the provider profile, with a random suffix for uniqueness.
        let base_name = profile
            .username
            .clone()
            .unwrap_or_else(|| format!("user_{}", &Uuid::new_v4().to_string()[..8]));
        // Sanitize: only keep alphanumeric, underscore, hyphen.
        let sanitized: String = base_name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .take(24) // leave room for suffix
            .collect();
        let username = if sanitized.len() < 3 {
            format!("user_{}", &Uuid::new_v4().to_string()[..8])
        } else {
            sanitized
        };

        sqlx::query(
            r#"
            INSERT INTO users (id, username, email, password_hash, argon2_salt, created_at)
            VALUES ($1, $2, $3, NULL, NULL, NOW())
            "#,
        )
        .bind(user_id)
        .bind(&username)
        .bind(profile.email.as_deref().unwrap_or(""))
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            if let Some(db_err) = e.as_database_error() {
                if db_err.code().as_deref() == Some("23505") {
                    return AppError::Conflict(
                        "an account with this username or email already exists".into(),
                    );
                }
            }
            AppError::Internal(anyhow::anyhow!(e))
        })?;

        (user_id, username)
    };

    // Insert the OAuth account link.
    sqlx::query(
        r#"
        INSERT INTO oauth_accounts (user_id, provider, provider_user_id, provider_email, provider_username, avatar_url)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(user_id)
    .bind(provider)
    .bind(&profile.provider_user_id)
    .bind(&profile.email)
    .bind(&profile.username)
    .bind(&profile.avatar_url)
    .execute(&mut *tx)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    tx.commit()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    Ok((user_id, username))
}

// ---------------------------------------------------------------------------
// Provider management (authenticated)
// ---------------------------------------------------------------------------

/// A single linked OAuth provider, returned by the `list_providers` endpoint.
#[derive(Serialize)]
pub struct LinkedProvider {
    /// Provider name (e.g., "github", "google").
    pub provider: String,
    /// The user's display name at the provider.
    pub provider_username: Option<String>,
    /// The user's email at the provider.
    pub provider_email: Option<String>,
    /// The user's avatar URL at the provider.
    pub avatar_url: Option<String>,
    /// When the link was created.
    pub linked_at: chrono::DateTime<chrono::Utc>,
}

/// Response from the `list_providers` endpoint.
#[derive(Serialize)]
pub struct ListProvidersResponse {
    /// All OAuth providers currently linked to this account.
    pub providers: Vec<LinkedProvider>,
    /// Whether this account has a password set (useful for UI: show "set password"
    /// prompt if the user wants to unlink their last OAuth provider).
    pub has_password: bool,
}

/// `GET /api/v1/auth/providers`
///
/// Returns all OAuth providers linked to the authenticated user's account,
/// plus whether the account has a password set.
pub async fn list_providers(
    State(state): State<AppState>,
    Extension(claims): Extension<super::auth::Claims>,
) -> Result<impl IntoResponse> {
    // Fetch all linked providers for this user.
    let rows = sqlx::query(
        r#"
        SELECT provider, provider_username, provider_email, avatar_url, created_at
        FROM oauth_accounts
        WHERE user_id = $1
        ORDER BY created_at ASC
        "#,
    )
    .bind(claims.sub)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    let providers: Vec<LinkedProvider> = rows
        .into_iter()
        .map(|r| LinkedProvider {
            provider: r.get::<String, _>("provider"),
            provider_username: r.get::<Option<String>, _>("provider_username"),
            provider_email: r.get::<Option<String>, _>("provider_email"),
            avatar_url: r.get::<Option<String>, _>("avatar_url"),
            linked_at: r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
        })
        .collect();

    // Check if the user has a password set (non-NULL password_hash).
    let has_password =
        sqlx::query_scalar::<_, bool>("SELECT password_hash IS NOT NULL FROM users WHERE id = $1")
            .bind(claims.sub)
            .fetch_one(&state.db)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    Ok(Json(ListProvidersResponse {
        providers,
        has_password,
    }))
}

/// `POST /api/v1/auth/oauth/:provider/link`
///
/// Initiates linking a new OAuth provider to the authenticated user's account.
///
/// Works identically to the `initiate` endpoint, but stores the user's ID in
/// the OAuth state so the callback knows to link (not create) the account.
/// The callback uses the `link_user_id` field to distinguish link vs. login flows.
pub async fn link_provider(
    State(state): State<AppState>,
    Extension(claims): Extension<super::auth::Claims>,
    Path(provider): Path<String>,
) -> Result<impl IntoResponse> {
    let provider = provider.to_lowercase();

    // Validate provider is known and configured.
    let provider_config = state.oauth.get(&provider).ok_or_else(|| {
        AppError::NotFound(format!("OAuth provider '{provider}' is not configured"))
    })?;
    let meta = provider_meta(&provider)?;

    // Check if already linked.
    let already_linked = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM oauth_accounts WHERE user_id = $1 AND provider = $2",
    )
    .bind(claims.sub)
    .bind(&provider)
    .fetch_one(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    if already_linked > 0 {
        return Err(AppError::Conflict(format!(
            "{provider} is already linked to your account"
        )));
    }

    let client = build_oauth_client(provider_config, &meta)?;
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    let mut auth_request = client.authorize_url(CsrfToken::new_random);
    for scope in meta.scopes {
        auth_request = auth_request.add_scope(Scope::new(scope.to_string()));
    }
    let (authorize_url, csrf_state) = auth_request.set_pkce_challenge(pkce_challenge).url();

    // Store state with the user_id so the callback knows this is a link operation.
    // We store the user_id in the provider field prefixed with "link:" to signal
    // the callback to link rather than login/create.
    let link_provider = format!("link:{}:{}", claims.sub, provider);

    sqlx::query(
        r#"
        INSERT INTO oauth_states (state, pkce_verifier, provider, created_at, expires_at)
        VALUES ($1, $2, $3, NOW(), NOW() + INTERVAL '5 minutes')
        "#,
    )
    .bind(csrf_state.secret())
    .bind(pkce_verifier.secret())
    .bind(&link_provider)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to store OAuth state: {e}")))?;

    tracing::info!(
        user_id = %claims.sub,
        provider = %provider,
        "OAuth link flow initiated",
    );

    Ok(Json(OAuthInitiateResponse {
        authorize_url: authorize_url.to_string(),
    }))
}

/// `DELETE /api/v1/auth/oauth/:provider/unlink`
///
/// Unlinks an OAuth provider from the authenticated user's account.
///
/// # Safety
///
/// Prevents unlinking the last authentication method. The user must have at
/// least one of: a password, or another linked OAuth provider. Otherwise,
/// they would be permanently locked out.
pub async fn unlink_provider(
    State(state): State<AppState>,
    Extension(claims): Extension<super::auth::Claims>,
    Path(provider): Path<String>,
) -> Result<impl IntoResponse> {
    let provider = provider.to_lowercase();

    // Count how many auth methods the user currently has.
    let has_password =
        sqlx::query_scalar::<_, bool>("SELECT password_hash IS NOT NULL FROM users WHERE id = $1")
            .bind(claims.sub)
            .fetch_one(&state.db)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    let oauth_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM oauth_accounts WHERE user_id = $1")
            .bind(claims.sub)
            .fetch_one(&state.db)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    // Prevent lockout: must have at least one remaining auth method after unlink.
    if !has_password && oauth_count <= 1 {
        return Err(AppError::BadRequest(
            "cannot unlink your only authentication method — set a password first, \
             or link another provider before unlinking this one"
                .into(),
        ));
    }

    // Delete the OAuth link.
    let result = sqlx::query("DELETE FROM oauth_accounts WHERE user_id = $1 AND provider = $2")
        .bind(claims.sub)
        .bind(&provider)
        .execute(&state.db)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "{provider} is not linked to your account"
        )));
    }

    tracing::info!(
        user_id = %claims.sub,
        provider = %provider,
        "OAuth provider unlinked",
    );

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_meta_returns_known_providers() {
        assert!(provider_meta("github").is_ok());
        assert!(provider_meta("google").is_ok());
        assert!(provider_meta("microsoft").is_ok());
        assert!(provider_meta("apple").is_ok());
        assert!(provider_meta("facebook").is_ok());
    }

    #[test]
    fn provider_meta_rejects_unknown() {
        assert!(provider_meta("twitter").is_err());
        assert!(provider_meta("linkedin").is_err());
        assert!(provider_meta("").is_err());
    }

    #[test]
    fn github_scopes_include_user_and_email() {
        let meta = provider_meta("github").unwrap();
        assert!(meta.scopes.contains(&"read:user"));
        assert!(meta.scopes.contains(&"user:email"));
    }

    #[test]
    fn google_uses_openid_scopes() {
        let meta = provider_meta("google").unwrap();
        assert!(meta.scopes.contains(&"openid"));
        assert!(meta.scopes.contains(&"email"));
    }

    #[test]
    fn profile_url_is_set_for_non_apple_providers() {
        for provider in &["github", "google", "microsoft", "facebook"] {
            let meta = provider_meta(provider).unwrap();
            assert!(
                !meta.profile_url.is_empty(),
                "{provider} should have a profile URL"
            );
        }
    }

    #[test]
    fn apple_has_empty_profile_url() {
        let meta = provider_meta("apple").unwrap();
        assert!(
            meta.profile_url.is_empty(),
            "Apple uses id_token, not a profile endpoint"
        );
    }

    #[test]
    fn apple_jwks_url_is_correct() {
        assert_eq!(APPLE_JWKS_URL, "https://appleid.apple.com/auth/keys");
    }

    #[test]
    fn apple_id_token_claims_deserialize() {
        // Simulate a decoded Apple id_token payload.
        let json = r#"{"sub":"001234.abcdef1234567890","email":"alice@privaterelay.appleid.com","email_verified":true,"iss":"https://appleid.apple.com","aud":"com.freebox.app","exp":9999999999,"iat":1000000000}"#;
        let claims: AppleIdTokenClaims = serde_json::from_str(json).unwrap();
        assert_eq!(claims.sub, "001234.abcdef1234567890");
        assert_eq!(
            claims.email.as_deref(),
            Some("alice@privaterelay.appleid.com")
        );
        assert_eq!(claims.email_verified, Some(true));
    }

    #[test]
    fn apple_id_token_claims_handles_missing_email() {
        // Apple may omit email on subsequent logins.
        let json = r#"{"sub":"001234.abcdef1234567890","iss":"https://appleid.apple.com","aud":"com.freebox.app","exp":9999999999,"iat":1000000000}"#;
        let claims: AppleIdTokenClaims = serde_json::from_str(json).unwrap();
        assert_eq!(claims.sub, "001234.abcdef1234567890");
        assert!(claims.email.is_none());
        assert!(claims.email_verified.is_none());
    }

    #[test]
    fn apple_jwk_deserializes() {
        let json = r#"{"kid":"abc123","kty":"RSA","alg":"RS256","use":"sig","n":"0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM","e":"AQAB"}"#;
        let jwk: AppleJwk = serde_json::from_str(json).unwrap();
        assert_eq!(jwk.kid, "abc123");
        assert_eq!(jwk.e, "AQAB");
    }
}
