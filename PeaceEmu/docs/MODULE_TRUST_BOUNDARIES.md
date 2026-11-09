# Module trust boundaries

## What manifest validation guarantees

A valid manifest is a statement about metadata and compatibility, not proof that a module is safe to execute. Validation can establish that identifiers, versions, dependencies, declared capabilities, URLs, and API requirements satisfy the schema. It cannot prove that the implementation matches those declarations, avoids side effects, respects resource limits, or is free of malicious behavior.

## In-process modules

A native module loaded into the emulator host has the host process's effective privileges. Treat it as trusted code. A manifest, HTTPS artifact URL, digest match, or successful installation journal does not sandbox it. Do not load third-party native modules into the main process solely because metadata validation succeeds.

Before enabling third-party in-process modules, require a separate policy decision covering provenance/signature verification, supported API compatibility, review and update policy, and the consequences of arbitrary code execution. Capability declarations are routing and compatibility metadata; they are not operating-system permissions.

## Out-of-process modules

For untrusted or independently distributed modules, prefer a worker process with a versioned, narrow IPC protocol. The host should define explicit limits for message sizes, memory, CPU/time, file-system access, and lifecycle shutdown; validate all messages at the boundary; and treat worker crashes, malformed replies, timeouts, and protocol-version mismatches as recoverable module failures. The worker should receive only explicitly granted resources rather than inheriting ambient host handles.

Process separation alone is not a complete sandbox. OS-level identity/permissions and platform-specific isolation still need a concrete implementation and tests before making a security guarantee.

## Current scope

PeaceEmu currently validates module metadata and plans activation; this document does not claim that it sandboxes modules or verifies artifact signatures in every installation path. Until a sandboxed loader exists, module execution should be limited to trusted code. Keep trust decisions separate from schema validation and capability planning.
