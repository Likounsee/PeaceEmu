# Module catalogue and first-launch setup

## Goals

PeaceEmu is installed as a small, usable host application. Console emulation implementations are optional modules selected during first launch and managed later from Settings. The host must remain launchable when no console module is installed or when the network is unavailable.

The core crate now provides a platform-neutral, deterministic dependency planner. It deliberately does not perform network requests, execute downloaded code, modify the host operating system, or claim that a console is emulated. Those responsibilities belong to a future application/installer layer.

## Package model

1. **Base application** — UI, library, user settings, logging, module manager, update coordination and core interfaces. Installed with PeaceEmu.
2. **Console module** — implementation for one exact system, with its own version and declared capabilities.
3. **Shared runtime package** — reusable libraries or assets that can be dependencies of several console modules.
4. **User-owned content** — game images, saves, firmware/BIOS and configuration. This is never removed as a side effect of removing an emulator module.

A console being listed in the catalogue is not proof that its emulator works. The catalogue status must distinguish `installable`, `experimental`, and `in-development`. `in-development` modules must not be offered as installable. An experimental module can be installed only with an explicit, honest compatibility label.

## Catalogue and IDs

Every package has a stable, lowercase ASCII ID (`game-boy`, `shared-audio-runtime`), display name, readiness status, package version, compatibility range, platform/architecture constraints, declared dependencies and a trusted source identity. IDs are machine-facing and must not be renamed casually once used in user configuration.

The first implementation uses owned, runtime-validated module IDs, typed descriptors and a pure dependency planner in `emulation_core::module_management`. The `emulation_core::module_manifest` module now validates a versioned manifest model: strict numeric package/app versions, minimum and exclusive maximum app compatibility, explicit OS/CPU targets, duplicate and missing IDs, dependency cycles, and basic artifact metadata (HTTPS URL, nonzero size, SHA-256 syntax and signing-key ID). This is structural validation only, not signature verification; no remote manifest or package is trusted until a future signed-catalogue verifier authenticates it. The target-filter API currently returns an error if any installable/experimental module in the manifest is incompatible with the selected host, so callers should supply a target-specific manifest or split per-target records before presenting choices. User intent and installed versions are modeled separately in `emulation_core::module_state`, with version-1 disk persistence in `emulation_core::module_state_store`. The store bounds input size, validates IDs and records, hex-encodes version strings, stages writes, retains a backup and attempts recovery from a corrupt primary file. The store exposes a per-user default path resolver for Windows (APPDATA), macOS (Application Support), and Unix/XDG configuration directories. The application still needs to own the store lifecycle and serialize concurrent writes. It validates duplicate IDs, missing dependencies and dependency cycles. Planning resolves transitive dependencies, emits dependencies before dependants, avoids duplicate shared packages and omits already-installed packages. It rejects unknown modules and modules still in development.

The core now also exposes `emulation_core::module_manifest_trust`: a pinned-key policy, signature-verifier interface, signature-before-parse ordering, structural validation and monotonically increasing revision check. This is a trust boundary, not a complete production signature stack: a reviewed cryptographic verifier and strict parser for the exact signed bytes still need to be integrated, and the accepted revision must be persisted across restarts. The test verifier is explicitly non-cryptographic. The next step is a strict serialized external manifest format and production cryptographic integration. The format should include:
- schema version and catalogue revision;
- package ID, package version and compatible PeaceEmu version range;
- target OS and CPU architecture;
- artifact URL, exact byte size and cryptographic digest;
- signature/key identity and signing policy;
- dependency IDs and version constraints;
- readiness status and human-readable release notes.

Do not deserialize remote manifests directly into trusted executable paths. Validate schema, IDs, version constraints, dependency graph, target platform and signature before offering an installation.

## First launch

1. Start the base application without requiring network access.
2. If the setup-complete setting is absent, show a welcome screen explaining that console modules are optional.
3. Fetch and verify the catalogue when online. If unavailable, show a retry option and allow the user to continue with the base application.
4. Display only modules available for this application version and platform. Show compatibility state, approximate download size and required shared packages when known.
5. Let the user select multiple modules, skip setup, or choose “configure later”.
6. Resolve the full dependency plan and show a confirmation summary before downloads.
7. Download to a temporary staging directory; enforce size limits and timeouts; verify digest and signature; unpack without path traversal; validate package metadata; then atomically activate the package.
8. Persist the user's selected modules and installed package versions only after each successful activation. Record failures and allow retry without repeating completed work.
9. Open the main library. Missing, broken or offline modules must not prevent the host from starting.

The setup-complete flag must mean the wizard was deliberately completed or skipped, not that every selected package installed successfully. Failed installs remain visible and retryable.

## Later module management

Settings exposes the same catalogue and installation planner after first launch. Users can add, update, repair or remove console modules independently of the base application.

- Check dependency use before removal; do not remove shared packages while any installed module requires them.
- Keep package versions and install state in a transactional registry.
- Stage updates side-by-side and activate them atomically; retain a known-good version for rollback.
- Never remove saves, game files, user settings or user-supplied firmware when removing a module.
- A failed download, signature check or extraction must leave the active installation unchanged.
- Do not automatically elevate privileges or invoke arbitrary shell commands to install dependencies.
- Prefer dependencies bundled as verified application packages over system-wide dependency changes.

## Security and trust

A digest detects corruption but does not establish publisher authenticity. Production downloads therefore need a signed catalogue/artifact policy with pinned trusted keys and a documented key-rotation procedure. Enforce HTTPS, archive extraction path checks, package size limits and compatibility checks. Never execute post-install scripts from an unsigned or untrusted package.

The initial module planner is not itself an installer or security boundary. The future downloader/activator must implement these controls and test them on Windows, macOS and Linux.

## Current truth about supported systems

Console-specific code is developed in separate module repositories and is not bundled in the host workspace. A module must remain marked `in-development` until its implementation and reproducible execution tests justify a stronger status. No placeholder package should be labelled as a working emulator.

## Implementation sequence

- **P0:** dependency-aware planner and tests (implemented in core); truthful readiness states; documented trust model.
- **P0:** persistent first-launch/selection/installed-version state with a versioned format, size limit, validation, staged writes and backup recovery (model and store implemented; per-user path integration remains).\n- **P0:** version compatibility rules and conflict handling during concurrent application updates.
- **P1:** first-launch wizard and settings page, including skip/offline paths.
- **P1:** signed catalogue format and secure staged installer.
- **P1/P2:** platform-specific packaging, atomic activation, repair, removal and rollback.
- **P2:** user-facing compatibility reporting backed by reproducible tests for each console.
