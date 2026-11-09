# Security baseline

All imported ROM images, firmware, save files, state files, metadata and configuration are untrusted input.

- Validate sizes, offsets, lengths and integer conversions before access or allocation.
- Put upper bounds on allocations driven by input data.
- Validate save-state version, target system ID and core version before restore.
- Parse into temporary structures and only mutate machine state after validation succeeds.
- Prefer explicit errors to panics on malformed external data.
- Keep unsafe Rust and FFI out of the shared core crate unless a specific, reviewed need exists.
- For native integrations, document pointer validity, ownership, buffer lengths, threading and error propagation.
- Avoid downloading or bundling copyrighted firmware or game images without permission.

This is a policy baseline, not a completed security audit. No hostile-input fuzzing or native boundary currently exists.


## Module catalogue trust boundary

The core provides `module_manifest_trust::ManifestTrustPolicy`, which requires a publisher key ID pinned by the application, calls an injected signature verifier on the exact payload bytes before parsing, validates the resulting manifest, and rejects revisions at or below the accepted revision floor. The accepted revision must be persisted durably by the application to prevent rollback across restarts. This module intentionally does not ship a cryptographic primitive or JSON parser: production must connect a reviewed signature implementation (for example Ed25519), a strict parser that parses exactly the verified bytes, and a carefully managed key-rotation policy. The test verifier is deliberately non-cryptographic and must never be used outside tests. A SHA-256 digest detects artifact mismatch but is not publisher authentication.
