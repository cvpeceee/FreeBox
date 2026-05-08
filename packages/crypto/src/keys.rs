//! Key management — generation, derivation, serialization, and secure erasure.
//!
//! # Key Hierarchy
//!
//! ```text
//! User Password
//!     └──[Argon2id]──► Master Secret (256-bit)
//!                            ├──► Identity Key Pair   (Ed25519  — signing)
//!                            ├──► Signed Prekey       (X25519   — key exchange)
//!                            └──► One-Time Prekeys    (X25519 × N, uploaded to server)
//! ```
//!
//! # Security Notes
//!
//! - All key material implements [`zeroize::Zeroize`]. Values are **wiped from
//!   memory** when dropped, preventing leakage through heap inspection or swap.
//! - The Master Secret is never stored. It is re-derived from the password on
//!   each login. This means a password change can invalidate all derived keys.
//! - One-time prekeys are single-use. The server deletes a prekey after it has
//!   been handed to a peer. If the supply runs low the client replenishes them.

use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

use argon2::{
    password_hash::{rand_core::OsRng as Argon2OsRng, SaltString},
    Argon2, Params, Version,
};

// ---------------------------------------------------------------------------
// Master secret
// ---------------------------------------------------------------------------

/// 256-bit master secret derived from the user's password via Argon2id.
///
/// # Security
///
/// This type is `ZeroizeOnDrop` — the secret bytes are overwritten when this
/// value goes out of scope. Never clone this value unnecessarily.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct MasterSecret([u8; 32]);

impl MasterSecret {
    /// Derive a master secret from a password and a stored salt.
    ///
    /// Uses **Argon2id** with OWASP 2023 recommended parameters:
    /// - Memory: 64 MiB
    /// - Iterations: 3
    /// - Parallelism: 4
    ///
    /// The `salt` argument must be the same salt that was stored at
    /// registration time. It is **not** secret but must not be reused
    /// across users.
    pub fn derive(password: &[u8], salt: &[u8]) -> anyhow::Result<Self> {
        // OWASP 2023 recommended Argon2id parameters.
        let params = Params::new(
            64 * 1024, // 64 MiB memory
            3,         // 3 iterations
            4,         // 4 parallel lanes
            Some(32),  // 256-bit output
        )
        .map_err(|e| anyhow::anyhow!("Argon2 params error: {}", e))?;

        let argon2 = Argon2::new(argon2::Algorithm::Argon2id, Version::V0x13, params);

        let mut output = [0u8; 32];
        argon2
            .hash_password_into(password, salt, &mut output)
            .map_err(|e| anyhow::anyhow!("Argon2 KDF failed: {}", e))?;

        Ok(Self(output))
    }

    /// Generate a new random salt for use with [`MasterSecret::derive`].
    /// Store this salt alongside the user record (it is NOT secret).
    pub fn generate_salt() -> String {
        SaltString::generate(&mut Argon2OsRng).to_string()
    }

    /// Access the raw secret bytes.
    ///
    /// Prefer keeping this reference short-lived and within this module.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

// ---------------------------------------------------------------------------
// Identity key pair (Ed25519)
// ---------------------------------------------------------------------------

/// The user's long-term identity key pair.
///
/// The **signing key** (private) never leaves the client. The **verifying key**
/// (public) is uploaded to the key server so peers can verify the user's
/// identity and authenticate their prekey bundles.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct IdentityKeyPair {
    signing: [u8; 32],   // Ed25519 secret scalar
    verifying: [u8; 32], // Ed25519 public key (not secret but stored alongside)
}

impl IdentityKeyPair {
    /// Generate a fresh identity key pair from OS randomness.
    pub fn generate() -> Self {
        let key = SigningKey::generate(&mut OsRng);
        let verifying = key.verifying_key().to_bytes();
        Self {
            signing: key.to_bytes(),
            verifying,
        }
    }

    /// Reconstruct from stored bytes (e.g. from encrypted local storage).
    pub fn from_bytes(signing: [u8; 32]) -> anyhow::Result<Self> {
        let key = SigningKey::from_bytes(&signing);
        let verifying = key.verifying_key().to_bytes();
        Ok(Self { signing, verifying })
    }

    /// The public verifying key — safe to share with any peer.
    pub fn verifying_key_bytes(&self) -> &[u8; 32] {
        &self.verifying
    }

    /// Sign `message` using the private identity key.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        use ed25519_dalek::Signer;
        let key = SigningKey::from_bytes(&self.signing);
        key.sign(message).to_bytes()
    }
}

// ---------------------------------------------------------------------------
// Signed prekey (X25519)
// ---------------------------------------------------------------------------

/// A medium-term X25519 key pair, signed by the identity key.
///
/// Rotated periodically (e.g. every 7 days). The signature allows peers to
/// verify that the prekey was generated by the legitimate owner.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SignedPrekey {
    /// X25519 public key bytes.
    pub public_key: [u8; 32],
    /// Ed25519 signature over `public_key` by the identity key.
    #[serde(with = "serde_big_array::BigArray")]
    pub signature: [u8; 64],
    /// Unix timestamp (seconds) when this prekey was generated.
    pub created_at: u64,
}

impl SignedPrekey {
    /// Generate a new signed prekey, signed with `identity`.
    pub fn generate(identity: &IdentityKeyPair) -> (Self, StaticSecret) {
        let secret = StaticSecret::random_from_rng(OsRng);
        let public = X25519PublicKey::from(&secret);
        let pub_bytes = public.to_bytes();

        let signature = identity.sign(&pub_bytes);
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        (
            Self {
                public_key: pub_bytes,
                signature,
                created_at,
            },
            secret,
        )
    }
}

// ---------------------------------------------------------------------------
// One-time prekeys (X25519)
// ---------------------------------------------------------------------------

/// A single-use X25519 public key for X3DH initiation.
///
/// After a peer uses this key to initiate a session, it is discarded. This
/// provides **forward secrecy at the session level**: even if the signed
/// prekey is later compromised, past session initiations remain secure.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OneTimePrekey {
    /// Sequential ID — sent to the server with the bundle.
    pub id: u32,
    /// X25519 public key bytes.
    pub public_key: [u8; 32],
}

// ---------------------------------------------------------------------------
// Prekey bundle (uploaded to key server)
// ---------------------------------------------------------------------------

/// The full public key bundle uploaded to the key server on registration.
///
/// Peers download this bundle to initiate a secure session without the
/// recipient being online (asynchronous key agreement — the X3DH algorithm).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PrekeyBundle {
    /// User's identity verifying key (Ed25519 public).
    pub identity_key: [u8; 32],
    /// Current signed prekey.
    pub signed_prekey: SignedPrekey,
    /// Batch of single-use prekeys (client generates 100 at a time).
    pub one_time_prekeys: Vec<OneTimePrekey>,
}

impl PrekeyBundle {
    /// Generate a complete prekey bundle.
    ///
    /// `one_time_count` is typically 100. The server stores these and
    /// provides one per new incoming session request.
    pub fn generate(
        identity: &IdentityKeyPair,
        one_time_count: u32,
    ) -> (Self, StaticSecret, Vec<StaticSecret>) {
        let (signed_prekey, signed_prekey_secret) = SignedPrekey::generate(identity);

        let mut one_time_prekeys = Vec::with_capacity(one_time_count as usize);
        let mut one_time_secrets = Vec::with_capacity(one_time_count as usize);

        for id in 0..one_time_count {
            let secret = StaticSecret::random_from_rng(OsRng);
            let public_key = X25519PublicKey::from(&secret).to_bytes();
            one_time_prekeys.push(OneTimePrekey { id, public_key });
            one_time_secrets.push(secret);
        }

        let bundle = Self {
            identity_key: *identity.verifying_key_bytes(),
            signed_prekey,
            one_time_prekeys,
        };

        (bundle, signed_prekey_secret, one_time_secrets)
    }
}

// ---------------------------------------------------------------------------
// Convenience: derive all keys from a password in one call
// ---------------------------------------------------------------------------

/// All keys derived from a single login operation.
///
/// This struct holds secret material and is `ZeroizeOnDrop`.
#[derive(ZeroizeOnDrop)]
pub struct DerivedKeys {
    pub master_secret: MasterSecret,
    pub identity: IdentityKeyPair,
}

/// Derive the full key hierarchy from a password and salt.
///
/// Used during login. At registration, additionally generate a [`PrekeyBundle`]
/// via [`PrekeyBundle::generate`].
pub fn derive_keys_from_password(password: &str, salt: &str) -> anyhow::Result<DerivedKeys> {
    let master_secret = MasterSecret::derive(password.as_bytes(), salt.as_bytes())?;

    // Deterministically derive the identity key from the master secret so
    // that the same password always produces the same identity key pair.
    // This allows key recovery without a separate backup mechanism.
    let mut identity_seed = [0u8; 32];
    identity_seed.copy_from_slice(&blake3::derive_key(
        "freebox identity key v1",
        master_secret.as_bytes(),
    ));

    let identity = IdentityKeyPair::from_bytes(identity_seed)?;
    identity_seed.zeroize(); // wipe the intermediate seed immediately

    Ok(DerivedKeys {
        master_secret,
        identity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_keys_is_deterministic() {
        let k1 = derive_keys_from_password("hunter2", "saltsaltsalt").unwrap();
        let k2 = derive_keys_from_password("hunter2", "saltsaltsalt").unwrap();
        // Same password + salt → same identity verifying key.
        assert_eq!(
            k1.identity.verifying_key_bytes(),
            k2.identity.verifying_key_bytes()
        );
    }

    #[test]
    fn different_password_different_keys() {
        let k1 = derive_keys_from_password("password1", "saltsaltsalt").unwrap();
        let k2 = derive_keys_from_password("password2", "saltsaltsalt").unwrap();
        assert_ne!(
            k1.identity.verifying_key_bytes(),
            k2.identity.verifying_key_bytes()
        );
    }

    #[test]
    fn prekey_bundle_generation() {
        let identity = IdentityKeyPair::generate();
        let (bundle, _, _) = PrekeyBundle::generate(&identity, 10);
        assert_eq!(bundle.one_time_prekeys.len(), 10);
        assert_eq!(bundle.identity_key, *identity.verifying_key_bytes());
    }
}
