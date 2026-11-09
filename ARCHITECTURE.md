# PeaceEmu architecture

This document describes how the current PeaceEmu source tree is organized and how its host, core contracts and modules work together. It describes the architecture implemented in this repository; it is not a claim that every console or feature is already fully supported.

## 1. High-level design

PeaceEmu separates the user interface from the emulation implementation:

```text
User
 |
 v
Desktop application (crates/desktop)
 |  navigation, library UI, preferences, sessions, input and media presentation
 v
Application controller (crates/app)
 |  library model, content import, session orchestration
 v
Emulation host and contracts (crates/core)
 |                         |
 | in-process core         | external module worker
 v                         v
EmulationCore / Session    Worker process + bounded IPC protocol
 |                         |
 +-----------+-------------+
             |
             v
       Video / audio / save data
```

The desktop application is the host, not a console emulator by itself. Console-specific CPU, memory, graphics, audio, timing and peripheral behavior belongs in the relevant emulation core/module.

## 2. Rust crates

### `emulation-core` — shared contracts

Located in `PeaceEmu/crates/core`, this crate contains platform-neutral types and services used by the host and cores:

- Core descriptors, identifiers, capabilities and registry contracts.
- Content envelopes, bounded reads and format-probe interfaces.
- Session lifecycle and input, video and audio data contracts.
- Save-state envelopes and compatibility metadata.
- Host runtime and session ownership.
- Module manifests, capability metadata and dependency planning.
- Module artifact staging, archive path checks and installation journals.
- Versioned, bounded messages for external workers.

It deliberately does not implement a particular console. A declared capability is a contract; it is not proof of compatibility.

### `host-app` — application logic

Located in `PeaceEmu/crates/app`, this crate keeps application state independent of the desktop UI. Its controller coordinates the content library, core registry, session creation and session commands. This separation allows frontends to use the same application behavior without owning console-specific logic.

### `desktop-app` — native user interface

Located in `PeaceEmu/crates/desktop`, this is the user-facing application built with eframe/egui. It provides the library, systems/modules, sessions, settings, controls, language and appearance screens. It also handles desktop concerns such as audio output, displaying video frames, launching external workers and persisting preferences.

The desktop crate has its own `Cargo.toml` and is intentionally excluded from the shared workspace membership. Its manifest is required; it is not a duplicate of the workspace manifest.

### Windows helper crates

- `windows-app-identity` contains Windows application identity integration.
- `windows-durability` contains Windows-specific durability support used by the shared core where applicable.

These crates isolate platform-specific code from the platform-neutral contracts.

## 3. From game import to an emulation session

The normal host-side flow is:

1. **Import:** the desktop UI asks the application controller to add a user-selected content file to the library.
2. **Inspect:** the host checks that the path is readable and records content metadata. A file existing on disk does not mean it is a valid or playable game.
3. **Bounded read:** at launch, content is read through size-limited APIs into an owned content image. This prevents a file from growing without bound during validation.
4. **Format detection:** registered format probes inspect the bounded data. The host must not guess when detection is missing, incomplete or ambiguous.
5. **Core selection:** the registry resolves the detected format to a compatible registered core, or the module/worker manager selects the installed external module according to its metadata.
6. **Validation and load:** the selected implementation validates the content and initializes its machine state. A failed validation must fail the launch rather than create a misleading running session.
7. **Execution:** the host advances the session in bounded steps, sends input events and collects the media produced by the core.
8. **Presentation:** the desktop displays the newest available video frame and sends audio buffers to its audio output backend.
9. **Shutdown:** stopping or closing a session releases held input and stops the core or worker cleanly.

## 4. Session lifecycle and media

The shared runtime models session states such as empty, running, paused, halted and stopped. The application controller exposes typed operations such as pause, resume, reset and stop instead of letting UI pages manipulate a core directly.

A core advertises whether it supports content loading, input, video, audio, persistent saves and complete save states. The host checks those capabilities before offering operations that depend on them.

Video frames and audio buffers cross the core/host boundary as owned data. The desktop is responsible for presenting that data; the core is responsible for producing it according to its emulated machine timing.

## 5. Modules and external workers

PeaceEmu is designed so console implementations can be maintained separately from the desktop host.

Module metadata describes identity, version, compatibility, capabilities and dependencies. Installation code stages package artifacts, validates paths and digests, and uses a journal to recover from interrupted installation operations. Signed-manifest trust checks use configured trust information and anti-rollback policy.

An external core can run as a separate worker process. The host communicates with it using the versioned, size-bounded worker protocol in `module_worker_protocol`. The desktop worker launcher validates the executable path and expected SHA-256 before starting it, and uses explicit standard-input/standard-output pipes rather than loading the module as a library into the host.

**Process separation is not an operating-system sandbox.** Hash verification identifies the expected executable bytes; it does not, by itself, prove publisher identity or prevent a compromised worker from accessing resources allowed to its OS account. Installation-directory permissions and any additional platform security controls remain important.

## 6. Save data

PeaceEmu distinguishes two concepts:

- **Save states** capture a restorable machine snapshot. Their envelope includes version and compatibility information so incompatible snapshots can be rejected.
- **Persistent game saves** represent data maintained by the emulated system, such as battery-backed RAM. They are separate from a full machine snapshot.

Availability depends on the capabilities implemented by the active core. The host must not promise save-state or persistent-save support when the module does not advertise and implement it.

## 7. Cargo workspace

The shared workspace is defined by `PeaceEmu/Cargo.toml` and includes `core`, `app`, `windows-durability` and `windows-app-identity`.

The desktop application is intentionally excluded and is built using `PeaceEmu/crates/desktop/Cargo.toml`. Keep both the workspace manifest and the desktop manifest: they describe different build units.

## 8. Compatibility and release discipline

A successful build proves that the selected source compiles for that target. It does not prove that a console is accurately emulated or that every game works.

System support should be documented only when the corresponding module has reproducible tests and compatibility evidence. Stable releases are published from the `PeaceEmu` repository; ongoing implementation and experiments belong in [PeaceEmu-dev](https://github.com/Likounsee/PeaceEmu-dev).
