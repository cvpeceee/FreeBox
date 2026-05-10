//! Shared HTTP utilities: token refresh and authenticated request helpers.
//!
//! # Token Refresh
//!
//! Access tokens expire after 15 minutes. Any command that makes authenticated
//! API calls should use [`send_with_refresh`] so that an expiring token is
//! transparently refreshed and the operation retried without bothering the user.
//!
//! # Usage
//!
//! ```rust,no_run
//! let mut session = Session::load()?;
//! let http = reqwest::Client::new();
//! let url = api_url(&server, "/api/v1/files");
//!
//! let resp = send_with_refresh(&mut session, |token| {
//!     http.get(&url).bearer_auth(token)
//! })
//! .await?
//! .error_for_status()?;
//! ```

use anyhow::{Context, Result};
use reqwest::{RequestBuilder, Response, StatusCode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{auth::api_url, session::Session};

// ---------------------------------------------------------------------------
// Internal DTOs (mirror the server's auth response)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct RefreshRequest {
    refresh_token: String,
}

#[derive(Deserialize)]
struct AuthTokenResponse {
    access_token: String,
    refresh_token: String,
    user_id: Uuid,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Exchange the stored refresh token for a new access+refresh token pair.
///
/// Persists the updated session to disk and returns it.
/// Returns an error if the refresh token is expired or revoked —
/// the user will need to run `fbx auth login` again.
pub async fn refresh_session(session: &Session) -> Result<Session> {
    let http = reqwest::Client::new();
    let resp = http
        .post(api_url(&session.server, "/api/v1/auth/refresh"))
        .json(&RefreshRequest {
            refresh_token: session.refresh_token.clone(),
        })
        .send()
        .await
        .context("failed to reach server during token refresh")?
        .error_for_status()
        .context("session expired — please run `fbx auth login` again")?
        .json::<AuthTokenResponse>()
        .await
        .context("invalid token refresh response from server")?;

    let updated = Session {
        access_token: resp.access_token,
        refresh_token: resp.refresh_token,
        user_id: resp.user_id,
        server: session.server.clone(),
        username: session.username.clone(),
    };
    updated.save()?;
    Ok(updated)
}

/// Send an authenticated request and, on a 401, refresh the token once and retry.
///
/// `build` is a closure that receives the current bearer token and returns a
/// `RequestBuilder` ready to send. It is called a second time only if the
/// first attempt gets a 401 response.
///
/// ```rust,no_run
/// send_with_refresh(&mut session, |token| {
///     http.get(&url).bearer_auth(token)
/// }).await?;
/// ```
pub async fn send_with_refresh<F>(session: &mut Session, build: F) -> Result<Response>
where
    F: Fn(&str) -> RequestBuilder,
{
    let resp = build(&session.access_token).send().await?;
    if resp.status() == StatusCode::UNAUTHORIZED {
        *session = refresh_session(session).await?;
        let resp = build(&session.access_token).send().await?;
        return Ok(resp);
    }
    Ok(resp)
}
