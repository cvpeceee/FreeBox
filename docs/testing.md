# FreeBox Testing Layout

FreeBox uses Rust's crate-level test model. Tests should live close to the crate
or module they exercise so each crate can be run independently with Cargo.

## Unit Tests

Use unit tests for small behavior, validation, formatting, helper functions, and
security invariants that do not require external services.

Preferred layout:

```text
crate/
  src/
    feature.rs
    feature/
      tests/
        mod.rs
        case_group.rs
```

Examples:

```text
apps/server/src/api/tests/files.rs
packages/crypto/src/encryption.rs
packages/core/src/event.rs
```

Inline `#[cfg(test)] mod tests` blocks are still acceptable for tiny modules or
when tests need direct access to private implementation details. Once a test
block grows beyond a few cases, move it into a dedicated `tests/` module folder.

## Integration Tests

Use each crate's top-level `tests/` folder for public API behavior and flows that
should only depend on exported crate APIs.

Preferred layout:

```text
crate/
  tests/
    storage_provider.rs
    crypto_round_trip.rs
```

Avoid putting Cargo tests in the workspace root because the root is a virtual
workspace, not a package.

## End-to-End Tests

End-to-end tests that need Postgres, storage, and a running server should live in
a future `tests/e2e/` harness with explicit Docker setup. Keep these separate
from unit tests so local feedback stays fast.

## Current Coverage Map

- `packages/crypto`: chunk encryption, key derivation, X3DH, ratchet behavior.
- `packages/core`: error formatting, event contracts, plugin registry.
- `apps/server`: auth validation, JWTs, OAuth helpers, file upload helper logic.

