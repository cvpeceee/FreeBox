# Crypto Types

> Last synced: 2026-05-11

Class diagram and data flow for `packages/crypto`.

## Key Hierarchy & Types

```mermaid
classDiagram
    direction TB

    class MasterSecret {
        <<ZeroizeOnDrop>>
        -[u8; 32]
        +derive(password, salt) Result~MasterSecret~$
        +generate_salt() String$
        +as_bytes() [u8; 32]
    }

    class IdentityKeyPair {
        <<ZeroizeOnDrop>>
        -signing: [u8; 32]
        -verifying: [u8; 32]
        +generate() IdentityKeyPair$
        +from_bytes(signing) Result~IdentityKeyPair~$
        +verifying_key_bytes() [u8; 32]
        +sign(message) [u8; 64]
    }

    class SignedPrekey {
        +public_key: [u8; 32]
        +signature: [u8; 64]
        +created_at: u64
        +generate(identity) (Self, StaticSecret)$
    }

    class OneTimePrekey {
        +id: u32
        +public_key: [u8; 32]
        +validate_public() Result~()~
    }

    class PrekeyBundle {
        +identity_key: [u8; 32]
        +signed_prekey: SignedPrekey
        +one_time_prekeys: Vec~OneTimePrekey~
        +validate_public() Result~()~
    }

    class DerivedKeys {
        +identity: IdentityKeyPair
        +signed_prekey: SignedPrekey
        +signed_prekey_secret: StaticSecret
        +one_time_prekeys: Vec~OneTimePrekey~
        +one_time_secrets: Vec~StaticSecret~
    }

    class FileKey {
        <<ZeroizeOnDrop>>
        -[u8; 32]
        +generate() FileKey$
        +from_bytes(bytes) FileKey$
        +as_bytes() [u8; 32]
    }

    class ChunkCiphertext {
        +index: u64
        +nonce: Vec~u8~
        +ciphertext: Vec~u8~
    }

    class X3dhInitiation {
        +shared_secret: [u8; 32]
        +ephemeral_public_key: [u8; 32]
        +one_time_prekey_id: Option~u32~
    }

    class RatchetSession {
        <<ZeroizeOnDrop, Serialize>>
        -root_key: [u8; 32]
        -send_chain_key: [u8; 32]
        -recv_chain_key: [u8; 32]
        -send_count: u32
        -recv_count: u32
        +new(shared_secret, is_initiator) RatchetSession$
        +next_send_key() Result~[u8; 32]~
        +next_recv_key() Result~[u8; 32]~
        +seal(plaintext) Result~Vec~u8~~
        +open(ciphertext) Result~Vec~u8~~
    }

    MasterSecret ..> IdentityKeyPair : derives
    IdentityKeyPair --> SignedPrekey : signs
    PrekeyBundle --> SignedPrekey : contains
    PrekeyBundle --> OneTimePrekey : contains 0..N
    DerivedKeys --> IdentityKeyPair : contains
    DerivedKeys --> SignedPrekey : contains
    DerivedKeys --> OneTimePrekey : contains

    X3dhInitiation ..> PrekeyBundle : computed from
    X3dhInitiation ..> RatchetSession : seeds

    FileKey --> ChunkCiphertext : produces via encrypt_chunk
```

## Encryption Pipeline

```mermaid
flowchart LR
    subgraph Client Side
        PW[User Password]
        SALT[Argon2id Salt]
        PW -->|Argon2id 64MiB/3iter/4p| MS[MasterSecret 256-bit]
        SALT --> MS

        MS --> IKP[IdentityKeyPair Ed25519]
        MS --> SPK[SignedPrekey X25519]
        MS --> OTP[OneTimePrekeys X25519 × N]

        FILE[Plaintext File] --> SPLIT[Split 4 MiB chunks]
        FK[FileKey::generate] --> ENC[AES-256-GCM per chunk]
        SPLIT --> ENC
        ENC --> CT[ChunkCiphertext × N]

        FK -->|Seal with session key| ENV[Encrypted Key Envelope]
    end

    subgraph Server
        CT -->|PUT /files/upload/:id| STORE[(S3 / Local)]
        ENV -->|POST /files/upload/init| DB[(PostgreSQL)]
    end
```

## X3DH Key Agreement Flow

```mermaid
sequenceDiagram
    participant A as Alice (Initiator)
    participant KS as Key Server
    participant B as Bob (Responder)

    B->>KS: Upload PrekeyBundle (identity + signed + OTPs)

    A->>KS: Fetch Bob's PrekeyBundle
    KS-->>A: {identity_key, signed_prekey, one_time_prekeys}

    Note over A: Verify Ed25519 signature on signed_prekey

    A->>A: DH1 = DH(alice_identity, bob_signed_prekey)
    A->>A: DH2 = DH(alice_ephemeral, bob_identity)
    A->>A: DH3 = DH(alice_ephemeral, bob_signed_prekey)
    A->>A: DH4 = DH(alice_ephemeral, bob_one_time) [if available]
    A->>A: shared_secret = BLAKE3(DH1 || DH2 || DH3 [|| DH4])
    A->>A: RatchetSession::new(shared_secret, true)

    A->>B: {ephemeral_public_key, otp_id, initial_ciphertext}

    B->>B: DH1..DH4 (mirrored), derive same shared_secret
    B->>B: RatchetSession::new(shared_secret, false)

    Note over A,B: Double Ratchet advances with each message
```

## Domain Separation Constants

| Constant | Value | Used For |
|----------|-------|----------|
| `X3DH` | `"freebox x3dh v1"` | X3DH KDF |
| `RATCHET_ROOT` | `"freebox ratchet root v1"` | Root key derivation |
| `RATCHET_SEND` | `"freebox ratchet send v1"` | Send chain init |
| `RATCHET_RECV` | `"freebox ratchet recv v1"` | Recv chain init |
| `MSG_KEY` | `"freebox msg key v1"` | Per-message key |
| `CHAIN_ADVANCE` | `"freebox chain advance v1"` | Chain ratchet step |
