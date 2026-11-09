# PeaceEmu

<p align="center">
  <img src="PeaceEmu/crates/desktop/assets/branding/bird + text.png" alt="PeaceEmu" width="420">
</p>

<p align="center"><strong>A modular multi-console emulation application.</strong></p>

<p align="center">
  <a href="https://github.com/Likounsee/PeaceEmu/releases">Stable releases</a> ·
  <a href="https://github.com/Likounsee/PeaceEmu-dev">Development repository</a>
</p>

**PeaceEmu** is a Rust-based modular emulation application. This repository is the stable distribution and release source: tagged versions and their published assets are the official releases. Active development, experiments and in-progress changes take place in [PeaceEmu-dev](https://github.com/Likounsee/PeaceEmu-dev).

## Getting PeaceEmu

1. Open the [Releases page](https://github.com/Likounsee/PeaceEmu/releases).
2. Choose the latest non-prerelease version.
3. Download the installer or package attached to that release.
4. Verify the published SHA-256 checksum when one is provided.

If no release is listed yet, there is no official stable build published from this repository.

## Project structure

- `PeaceEmu/` — Rust workspace, application crates, packaging and technical documentation.
- `.github/workflows/` — automated quality gates and release-related workflows.
- `docs/` — project website.
- `README.md` — stable-repository overview.

Console implementations are maintained as separate modules. A system is only described as supported when its module and compatibility tests demonstrate that support; the presence of a module or a successful build alone is not sufficient.

## Building from source

The source tree is organized as a Cargo workspace plus a separate desktop application crate. For development instructions, architecture, testing requirements and current implementation status, see the [development repository](https://github.com/Likounsee/PeaceEmu-dev).

## Release policy

- Stable builds are published as GitHub Releases with versioned tags.
- Release assets should include checksums and only be published after the matching source revision has passed the required CI checks.
- Development commits are not automatically stable releases.
- Do not distribute copyrighted game images, firmware or BIOS files without authorization.

## License

PeaceEmu is distributed under the terms described in [LICENSE-PROPRIETARY](LICENSE-PROPRIETARY). Third-party components retain their respective licenses.
