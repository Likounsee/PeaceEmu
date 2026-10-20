# Official module catalogue and signing plan

## Decision

PeaceEmu uses a publisher-controlled Ed25519 signing identity for its official module catalogue. The private signing key is a release secret and must never be committed to this repository, included in the desktop installer, or exposed in build logs. The application pins only the corresponding public key and a stable key identifier.

## Trust bootstrap

1. Generate the Ed25519 key pair in a controlled release environment.
2. Store the private key in a protected CI secret with access restricted to trusted release workflows.
3. Review and pin the public key and key ID in the application source in a separately reviewed commit.
4. Publish the catalogue and detached signature as release artifacts on the official PeaceEmu repository.
5. Require signature verification over the exact catalogue bytes before parsing or using catalogue fields.
6. Persist the highest accepted catalogue revision durably and reject rollback.
7. Document a key-rotation procedure before the first production catalogue is shipped.

A placeholder key, test verifier, HTTPS-only URL, or SHA-256 digest is not a substitute for an authentic signature. The desktop crate contains a strict Ed25519 verifier backed by `ed25519-dalek`, a strict JSON parser for signed catalogues (unknown fields rejected, bounded to 1 MiB), and an adapter that routes signed envelopes through the core trust policy. Tests cover the RFC 8032 known vector, tampering, malformed inputs, strict JSON parsing, and verify-before-parse behavior. A basic worker-process launcher now connects standard input/output to the bounded IPC framing, kills/reaps the child when its handle is dropped, and offers a `spawn_verified` path that checks the executable SHA-256 against trusted metadata before startup. The launcher now also supports bounded graceful shutdown: it sends the shutdown message, waits only for the caller-specified duration, then kills and reaps an unresponsive worker. Invalid inbound payloads fail closed by terminating the worker. The hash check does not replace publisher signature validation and has a documented path-replacement race unless the install directory is protected. However, no production public key is pinned, no protected signing secret is configured, catalogue download and package installation are not wired to this path, and the worker launcher still does not enforce OS-level CPU/memory limits or provide a complete OS sandbox. External module installation must remain disabled.

## Package verification

A module package is accepted only when all checks pass:

- The catalogue signature validates against a pinned trusted key.
- The module ID, version, host API, target platform and dependencies pass schema and compatibility checks.
- The download uses HTTPS and an approved official distribution host.
- The downloaded byte count and SHA-256 digest match the signed catalogue.
- Archive paths are checked for traversal, absolute paths, platform-specific path tricks, and duplicate/conflicting entries before extraction.
- Extraction happens in a private staging directory; activation and installed-state persistence are journaled.
- Failed pre-activation operations clean up only their own staging files. Failures after activation begins require explicit journal recovery rather than blind deletion.

## Runtime isolation

Native Rust trait objects do not form a stable plugin ABI. PeaceEmu must not load arbitrary third-party Rust DLLs directly into the desktop process. External cores should use a versioned, bounded IPC protocol in a worker process. A framing layer exists in `module_worker_protocol.rs` (fixed header, protocol version, message kinds, and a 16 MiB payload cap), and the desktop crate now has a process launcher with a bounded IPC reader queue, caller-specified receive timeouts that kill and reap unresponsive workers, executable SHA-256 verification, strict per-message payload schemas, fail-closed handling of invalid inbound payloads, bounded graceful shutdown, and child cleanup. The host must still connect the launcher to core lifecycle management, enforce OS-level CPU/memory restrictions, and treat worker crashes as recoverable failures. Process separation alone is not a complete OS sandbox; stronger Windows isolation must be implemented and tested before being advertised as a security guarantee.

## Release gate

Do not publish a production module catalogue or advertise dynamic module installation until all of the following exist:

- A pinned production public key and reviewed key-rotation procedure.
- End-to-end wiring from the production Ed25519 verifier to a pinned public key and signed catalogue parser.
- Automated signature/tamper/unknown-key/rollback tests.
- A trusted catalogue publication workflow using a protected signing secret.
- A verified package installer and recovery tests.
- A worker-process launcher with complete message schemas, hard timeouts/resource limits, OS-level isolation, and executable isolation tests for external cores.

Console cores are not bundled in the host workspace. Each module repository is responsible for its implementation and validation; the host must report unavailable modules honestly until a verified package is installed.

## Worker IPC payload schemas

The desktop host validates each framed worker message against a per-operation schema before sending it and after receiving it. JSON control payloads are capped at 64 KiB and reject unknown fields. Initialization validates the module identifier, canonical app version, content digest, and a 32 MiB content-size ceiling; input batches are capped at 256 events; frame execution requires a non-zero instruction budget capped at one billion; shutdown requires an empty payload; and error reports have bounded fields. Video output uses a fixed RGBA8 binary envelope with exact dimensions and byte-count checks. These checks validate protocol shape only: they do not authenticate a worker, provide OS isolation, or make external module installation safe.
