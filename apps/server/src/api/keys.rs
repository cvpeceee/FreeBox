//! Prekey bundle handlers for the Signal Protocol key server.
//!
//! # Role of the Key Server
//!
//! The key server is a **public bulletin board** for cryptographic public keys.
//! It stores no secret material. Clients upload their prekey bundles so that
//! peers can initiate E2EE sessions asynchronously (without the recipient being
//! online). This is the X3DH asynchronous key agreement model.
//!
//! # One-Time Prekey Consumption
//!
//! When Alice fetches Bob's prekey bundle to start a session, the server
//! removes one one-time prekey from Bob's supply. Bob's client periodically
//! replenishes the supply. If the supply runs out, the server returns a bundle
//! with no one-time prekey (slightly weaker — still secure, just without OTP
//! forward secrecy for that session).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use freebox_crypto::OneTimePrekey;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    api::auth::Claims,
    error::{AppError, Result},
    state::AppState,
};

const MAX_REPLENISH_ONE_TIME_PREKEYS: usize = 1_000;

/// `GET /api/v1/keys/:user_id` — fetch a peer's prekey bundle.
///
/// Returns the identity key, signed prekey, and **one** one-time prekey
/// (removed from the server's supply after this call).
pub async fn get_bundle(
    State(state): State<AppState>,
    Extension(_claims): Extension<Claims>,
    Path(user_id): Path<Uuid>,
) -> Result<impl IntoResponse> {
    // Fetch the bundle. `bundle` is a JSONB column containing the full
    // PrekeyBundle struct as serialized by the client.
    let row = sqlx::query("SELECT bundle FROM prekey_bundles WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?
        .ok_or(AppError::NotFound(format!(
            "no prekey bundle for user {user_id}"
        )))?;

    Ok(Json(row.get::<serde_json::Value, _>("bundle")))
}

/// `POST /api/v1/keys/one-time` — replenish the authenticated user's one-time prekeys.
///
/// The client generates a fresh batch of one-time prekeys locally and uploads
/// their public halves here. The server appends them to the user's supply.
pub async fn replenish_one_time(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(new_keys): Json<Vec<OneTimePrekey>>,
) -> Result<impl IntoResponse> {
    let user_id = claims.sub;
    validate_one_time_prekey_batch(&new_keys)?;
    let new_keys = serde_json::to_value(&new_keys)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to serialize prekeys: {e}")))?;

    // Merge new one-time prekeys into the existing bundle.
    sqlx::query(
        r#"
        UPDATE prekey_bundles
        SET bundle = jsonb_set(
            bundle,
            '{one_time_prekeys}',
            (bundle->'one_time_prekeys') || $2::jsonb,
            false
        ),
        updated_at = NOW()
        WHERE user_id = $1
        "#,
    )
    .bind(user_id)
    .bind(&new_keys)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    tracing::debug!(user_id = %user_id, "One-time prekeys replenished");
    Ok(StatusCode::NO_CONTENT)
}

fn validate_one_time_prekey_batch(keys: &[OneTimePrekey]) -> Result<()> {
    if keys.is_empty() {
        return Err(AppError::BadRequest(
            "one-time prekey batch must not be empty".into(),
        ));
    }
    if keys.len() > MAX_REPLENISH_ONE_TIME_PREKEYS {
        return Err(AppError::BadRequest(format!(
            "too many one-time prekeys: max {}, got {}",
            MAX_REPLENISH_ONE_TIME_PREKEYS,
            keys.len()
        )));
    }

    let mut seen_ids = std::collections::BTreeSet::new();
    for key in keys {
        key.validate_public()
            .map_err(|e| AppError::BadRequest(format!("invalid one-time prekey: {e}")))?;
        if !seen_ids.insert(key.id) {
            return Err(AppError::BadRequest(format!(
                "duplicate one-time prekey id {}",
                key.id
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_one_time_prekey_batch;
    use freebox_crypto::OneTimePrekey;

    fn prekey(id: u32) -> OneTimePrekey {
        OneTimePrekey {
            id,
            public_key: [id as u8 + 1; 32],
        }
    }

    #[test]
    fn validate_one_time_prekey_batch_accepts_valid_keys() {
        validate_one_time_prekey_batch(&[prekey(1), prekey(2)]).unwrap();
    }

    #[test]
    fn validate_one_time_prekey_batch_rejects_empty_batch() {
        assert!(validate_one_time_prekey_batch(&[]).is_err());
    }

    #[test]
    fn validate_one_time_prekey_batch_rejects_duplicate_ids() {
        assert!(validate_one_time_prekey_batch(&[prekey(1), prekey(1)]).is_err());
    }

    #[test]
    fn validate_one_time_prekey_batch_rejects_zero_public_key() {
        let key = OneTimePrekey {
            id: 1,
            public_key: [0; 32],
        };

        assert!(validate_one_time_prekey_batch(&[key]).is_err());
    }
}
