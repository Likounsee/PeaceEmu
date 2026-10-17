# PeaceEmu

<p align="center">
  <img src="PeaceEmu/crates/desktop/assets/branding/bird + text.png" alt="PeaceEmu" width="420">
</p>

<p align="center"><strong>A modular multi-console emulation project.</strong></p>

**PeaceEmu is an in-development, modular multi-console emulation project.** Its long-term goal is to host independently testable emulation cores behind a shared application and tooling layer. The repository is at the foundation stage; no console is claimed as supported until a real core and reproducible tests demonstrate that support.

## Goals

- Build shared components primarily in stable Rust, with explicit errors and testable boundaries.
- Integrate C or C++ only when a mature, legally reusable library or a concrete interoperability/performance need justifies it.
- Keep cores independent of the UI and make audio/video ownership and timing explicit.
- For consoles with software interfaces, aim to reproduce or independently recreate their dashboards, system menus, launchers and settings interfaces when feasible.
- Provide optional, reversible visual enhancements for older systems, while retaining a faithful original-presentation mode and clearly distinguishing recreated interfaces from original firmware.
- Validate platform behavior with reproducible tests rather than treating compilation as compatibility.
- Treat game images, firmware, configuration and save files as untrusted input.

## Repository layout

- `PeaceEmu/` — Cargo workspace, source crates and technical documentation.
- `.github/workflows/` — GitHub Actions quality gates.
- `PeaceEmu/docs/` — architecture, interface, roadmap, testing, security and legal notes.

## Current implementation

The shared Rust core crate defines validated system identifiers, capability metadata, a core registry, an independent-session lifecycle controller, bounded execution requests, common media payloads, and host-side input state with duplicate-transition suppression and explicit release recovery. Module selection/state persistence and host performance planning are also present as foundations. Runtime tests cover lifecycle transitions, error recovery, independent session creation, and shared input-state behavior.

The Game Boy module is one unified `game-boy` target with cartridge-header classification for DMG-only, dual-compatible, and CGB-only titles. The core now has an initial LR35902 instruction subset connected to a deliberately limited memory bus for ROM-only (mapper 0) cartridges. It can execute simple instructions from the cartridge entry point, but has no PPU/frame output, timer/interrupt controller, audio, joypad hardware, mapper banking, or complete opcode coverage. This is a development milestone, not playable-game support; no system is currently claimed as supported.

## Build and test

Run from the project directory:

```sh
cd PeaceEmu
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
```

GitHub Actions runs formatting, locked workspace tests, Clippy with warnings denied, desktop-crate tests and Clippy, and warning-free workspace documentation generation on Linux, macOS and Windows. It runs on relevant source/documentation changes pushed to `PeaceEmu`, pull requests targeting that branch, or manual dispatch. The workflow configuration is checked in; the latest run must still be verified in GitHub Actions before these checks can be called green.

## Roadmap

Development priority is the **global emulator framework**, not expanding one console core. Current priorities are:

1. **P0 — Reliable shared runtime:** lifecycle and error semantics, module contracts and capability enforcement, generic content validation, and regression tests.
2. **P1 — Complete launch path:** multi-session orchestration, common input/video/audio contracts, host timing, and validated persistent saves/save states.
3. **P1 — Minimal host application:** select content and module, launch/pause/stop sessions, and surface diagnostics.
4. **P2 — Security and diagnostics:** trusted module/catalogue handling, safe installation and recovery, structured errors, fuzzing, and dependency/package checks.
5. **P3 — Measured quality:** conformance tests, long-run stability, reproducible benchmarks, and profiling-led optimization.
6. **P4 — Expand systems:** add independently tested cores only after shared interfaces are stable. Game Boy remains a validation target, not the focus of the global roadmap.
7. **P5 — UI and release:** content library, optional presentation enhancements, legal review, and reproducible packages.

See [the detailed roadmap](PeaceEmu/docs/ROADMAP.md) for current estimates, acceptance criteria and remaining work. Progress is measured against checked-in implementation and verified tests—not planned features or directory names.

## Development and legal policy

Development changes belong on the `PeaceEmu` branch; do not commit project changes to `main`. Do not distribute copyrighted game images, BIOS files or firmware without authorization. Third-party dependencies must be reviewed for license compatibility, redistribution terms, security, maintenance and platform support before integration.
