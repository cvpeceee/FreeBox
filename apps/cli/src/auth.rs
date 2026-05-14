//! Authentication command handler for the FreeBox CLI.

use anyhow::{Context, Result};
use dialoguer::{Input, Password};
use freebox_crypto::{
    derive_keys_from_password,
    keys::{MasterSecret, PrekeyBundle},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use freebox_crypto::auth_password_hash;

use crate::{session::Session, AuthCommands};

#[derive(Serialize)]
struct RegisterRequest {
    username: String,
    email: String,
    password_hash: String,
    argon2_salt: String,
    prekey_bundle: serde_json::Value,
}

#[derive(Serialize)]
struct LoginRequest {
    username: String,
    password_hash: String,
}

#[derive(Serialize)]
struct RefreshRequest {
    refresh_token: String,
}

#[derive(Deserialize)]
struct AuthResponse {
    access_token: String,
    refresh_token: String,
    user_id: Uuid,
}

#[derive(Deserialize)]
struct SaltResponse {
    argon2_salt: String,
}

/// Handle authentication subcommands (register, login, logout, whoami).
pub async fn handle(cmd: AuthCommands, server: &str) -> Result<()> {
    match cmd {
        AuthCommands::Register { username, email } => register(username, email, server).await,
        AuthCommands::Login { username } => login(username, server).await,
        AuthCommands::Logout => logout(server).await,
        AuthCommands::Whoami => whoami(),
    }
}

async fn register(username: Option<String>, email: Option<String>, server: &str) -> Result<()> {
    let username = prompt_if_missing(username, "Username")?;
    let email = prompt_if_missing(email, "Email")?;
    let password = Password::new()
        .with_prompt("Password")
        .with_confirmation("Confirm password", "Passwords do not match")
        .interact()
        .context("failed to read password")?;

    let salt = MasterSecret::generate_salt();
    let password_hash = client_password_hash(&password, &salt)?;
    let prekey_bundle = registration_prekey_bundle(&password, &salt)?;

    let req = RegisterRequest {
        username: username.clone(),
        email,
        password_hash,
        argon2_salt: salt,
        prekey_bundle,
    };

    let client = reqwest::Client::new();
    let auth = client
        .post(api_url(server, "/api/v1/auth/register"))
        .json(&req)
        .send()
        .await?
        .error_for_status()?
        .json::<AuthResponse>()
        .await?;

    save_session(server, &username, auth)?;
    println!("Registered and logged in as {username}");
    Ok(())
}

async fn login(username: Option<String>, server: &str) -> Result<()> {
    let username = prompt_if_missing(username, "Username")?;
    let password = Password::new()
        .with_prompt("Password")
        .interact()
        .context("failed to read password")?;

    let client = reqwest::Client::new();
    let salt = client
        .get(api_url(server, &format!("/api/v1/auth/salt/{username}")))
        .send()
        .await?
        .error_for_status()?
        .json::<SaltResponse>()
        .await?
        .argon2_salt;

    let req = LoginRequest {
        username: username.clone(),
        password_hash: client_password_hash(&password, &salt)?,
    };

    let auth = client
        .post(api_url(server, "/api/v1/auth/login"))
        .json(&req)
        .send()
        .await?
        .error_for_status()?
        .json::<AuthResponse>()
        .await?;

    save_session(server, &username, auth)?;
    println!("Logged in as {username}");
    Ok(())
}

async fn logout(server: &str) -> Result<()> {
    if let Ok(session) = Session::load() {
        let client = reqwest::Client::new();
        let _ = client
            .post(api_url(server, "/api/v1/auth/logout"))
            .bearer_auth(&session.access_token)
            .json(&RefreshRequest {
                refresh_token: session.refresh_token,
            })
            .send()
            .await;
    }

    Session::clear()?;
    println!("Logged out");
    Ok(())
}

fn whoami() -> Result<()> {
    let session = Session::load()?;
    println!(
        "{} ({}) on {}",
        session.username, session.user_id, session.server
    );
    Ok(())
}

fn save_session(server: &str, username: &str, auth: AuthResponse) -> Result<()> {
    Session {
        access_token: auth.access_token,
        refresh_token: auth.refresh_token,
        server: normalize_server_url(server),
        username: username.to_owned(),
        user_id: auth.user_id,
    }
    .save()
}

fn prompt_if_missing(value: Option<String>, label: &str) -> Result<String> {
    match value {
        Some(value) => Ok(value),
        None => Input::<String>::new()
            .with_prompt(label)
            .interact_text()
            .with_context(|| format!("failed to read {label}")),
    }
}

pub(crate) fn client_password_hash(password: &str, salt: &str) -> Result<String> {
    Ok(auth_password_hash(password, salt))
}

pub(crate) fn registration_prekey_bundle(password: &str, salt: &str) -> Result<serde_json::Value> {
    let keys = derive_keys_from_password(password, salt)?;
    let (bundle, _, _) = PrekeyBundle::generate(&keys.identity, 100);
    Ok(serde_json::to_value(bundle)?)
}

pub(crate) fn api_url(server: &str, path: &str) -> String {
    format!(
        "{}/{}",
        normalize_server_url(server),
        path.trim_start_matches('/')
    )
}

pub(crate) fn normalize_server_url(server: &str) -> String {
    server.trim_end_matches('/').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn random_salt() -> String {
        // Generate a random 22-char alphanumeric salt for tests
        use std::time::{SystemTime, UNIX_EPOCH};
        let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
        format!("testsalt{t:014}")
    }

    #[test]
    fn api_url_normalizes_slashes() {
        assert_eq!(
            api_url("http://localhost:8080/", "/api/v1/auth/login"),
            "http://localhost:8080/api/v1/auth/login"
        );
    }

    #[test]
    fn client_password_hash_is_deterministic_for_same_salt() {
        let salt = random_salt();

        let h1 = client_password_hash("secret", &salt).unwrap();
        let h2 = client_password_hash("secret", &salt).unwrap();

        assert_eq!(h1, h2);
    }

    #[test]
    fn client_password_hash_changes_with_password() {
        let salt = random_salt();

        let h1 = client_password_hash("secret", &salt).unwrap();
        let h2 = client_password_hash("different", &salt).unwrap();

        assert_ne!(h1, h2);
    }

    #[test]
    fn registration_prekey_bundle_has_expected_shape() {
        let salt = random_salt();

        let bundle = registration_prekey_bundle("secret", &salt).unwrap();

        assert!(bundle.get("identity_key").is_some());
        assert!(bundle.get("signed_prekey").is_some());
        assert_eq!(bundle["one_time_prekeys"].as_array().unwrap().len(), 100);
    }
}
