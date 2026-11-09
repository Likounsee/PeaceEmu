# PeaceEmu

<p align="center">
  <img src="PeaceEmu/crates/desktop/assets/branding/bird + text.png" alt="PeaceEmu" width="420">
</p>

<p align="center"><strong>A modular, Rust-based emulation application.</strong></p>

<p align="center">
  <a href="https://github.com/Likounsee/PeaceEmu/releases">Download stable releases</a> ·
  <a href="https://github.com/Likounsee/PeaceEmu-dev">Development repository</a>
</p>

PeaceEmu is a desktop application designed around a host-and-module architecture. The desktop host manages the game library, installed modules and emulation sessions, while console-specific emulation is provided through core/module implementations.

This repository is the **stable release repository**. Day-to-day development, experiments and unfinished changes belong in [PeaceEmu-dev](https://github.com/Likounsee/PeaceEmu-dev).

## Download

1. Open [GitHub Releases](https://github.com/Likounsee/PeaceEmu/releases).
2. Download the latest release that is not marked as a pre-release.
3. Follow the release notes and installation instructions attached to that version.
4. Verify the SHA-256 checksum when one is published.

If no stable release is listed, there is currently no official stable build available here.

## What PeaceEmu provides

- A native desktop interface built with Rust, eframe and egui.
- A local game library with content import and format validation.
- Module discovery, installation and management.
- Emulation session controls, including pause, resume, reset and stop.
- Input, video and audio interfaces shared between the host and emulation cores.
- Save-state and persistent-save contracts where the active core supports them.
- A bounded worker protocol for communicating with external core processes.
- French and English interface language packs.

**Important:** a feature being present in the host does not mean every console is fully emulated. Actual compatibility depends on the corresponding module and its tested implementation. Consult the release notes for the systems supported by a specific version.

## Project layout

- `PeaceEmu/crates/core` — platform-neutral emulation contracts, session lifecycle, content validation, module metadata and worker protocol.
- `PeaceEmu/crates/app` — frontend-independent application controller and game-library model.
- `PeaceEmu/crates/desktop` — native desktop application.
- `PeaceEmu/crates/windows-*` — Windows-specific integration helpers.
- `PeaceEmu/packaging` — Windows installer configuration.
- `PeaceEmu/update` — stable update-channel metadata.
- `.github/workflows` — automated build, test and quality checks.

For the internal design and the path from importing a game to running a session, read [ARCHITECTURE.md](ARCHITECTURE.md).

## Build from source

Install the Rust toolchain specified in `PeaceEmu/rust-toolchain.toml`.

From the repository root:

```sh
cargo test --manifest-path PeaceEmu/Cargo.toml --workspace
cargo run --manifest-path PeaceEmu/crates/desktop/Cargo.toml
cargo test --manifest-path PeaceEmu/crates/desktop/Cargo.toml --all-targets
```

The desktop crate is intentionally kept outside the shared Cargo workspace and has its own manifest. On Linux, the desktop build may require system libraries for the windowing and audio backends.

## Development and contributions

Please use [PeaceEmu-dev](https://github.com/Likounsee/PeaceEmu-dev) for development work. This repository should contain reviewed, release-ready source revisions and official release assets rather than experimental commits.

## Legal notice

PeaceEmu does not provide commercial game ROMs, copyrighted game images, firmware or BIOS files. Only use content that you are legally entitled to use. Third-party components remain subject to their respective licenses.

The project is distributed under the terms in [LICENSE-PROPRIETARY](LICENSE-PROPRIETARY).
