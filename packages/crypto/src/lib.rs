//! # freebox-crypto
//!
//! All cryptographic operations for the FreeBox platform live here.
//! **No other crate should import raw crypto primitives directly** — all
//! cryptography is funnelled through this crate's public API. This makes
//! auditing trivial: reviewers only need to read one crate.
//!
//! ## Security Properties
//!
//! | Property | Mechanism |
//! |---|---|
//! | Confidentiality | AES-256-GCM (per-file unique key) |
//! | Integrity | AES-GCM authentication tag |
//! | Forward Secrecy | Signal Double Ratchet |
//! | Break-in Recovery | Double Ratchet ratchet-step after each message |
//! | Password Security | Argon2id (memory-hard, OWASP 2023 params) |
//! | Key Exchange | X3DH (Extended Triple Diffie-Hellman) |
//! | Content Dedup | BLAKE3 (plaintext-side hash, never sent to server) |
//! | Key Memory Safety | `zeroize` — wipes secrets on drop |
//!
//! ## Modules
//!
//! | Module | Responsibility |
//! |--------|---------------|
//! | [`keys`] | Key generation, serialization, Argon2id derivation |
//! | [`encryption`] | AES-256-GCM encrypt / decrypt with chunking |
//! | [`signal`] | X3DH key agreement + Double Ratchet state machine |

pub mod encryption;
pub mod keys;
pub mod signal;

pub use encryption::{decrypt_chunk, encrypt_chunk, ChunkCiphertext, FileKey};
pub use keys::{
    derive_keys_from_password, DerivedKeys, IdentityKeyPair, PrekeyBundle, SignedPrekey,
};
pub use signal::{x3dh_initiate, x3dh_respond, RatchetSession, X3dhInitiation};
