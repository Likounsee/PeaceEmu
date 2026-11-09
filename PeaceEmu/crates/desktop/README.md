# PeaceEmu Desktop

The native PeaceEmu desktop application is built with Rust and eframe/egui. It is the user-facing host for the shared runtime and independently maintained console modules.

## Application responsibilities

- Browse and manage the local game library.
- Import content and report accepted or rejected files.
- Select an installed module and request a session launch.
- Manage session lifecycle actions and show per-session status.
- Persist user preferences, language and appearance settings.
- Present video frames and expose the audio/capability state reported by the active module.
- Keep console-specific hardware behavior outside the desktop UI.

The host does not bundle copyrighted game images, firmware or BIOS files. Importing a file does not establish that it is valid or playable. A console is not considered supported until its module passes reproducible compatibility tests.

## Code organization

- `main.rs` composes navigation and the top-level application lifecycle.
- Page rendering is split across `*_page.rs` modules.
- Input and video adapters live in `game_view.rs`.
- Preference persistence lives in `preferences.rs`.
- Session commands, summaries, bounded execution and media draining go through the shared `ApplicationController`.
- Console-specific CPU, bus, peripheral and timing behavior belongs in separate module repositories.

## Build and run

From the `PeaceEmu/` workspace directory:

```sh
cargo run --manifest-path crates/desktop/Cargo.toml
cargo test --manifest-path crates/desktop/Cargo.toml --all-targets
cargo clippy --manifest-path crates/desktop/Cargo.toml --all-targets -- -D warnings
```

The desktop crate is intentionally kept separate from the shared workspace membership because it has its own dependency graph. Its manifest is not redundant: it defines the actual desktop binary and its UI, media, localization and Windows integration dependencies.

## Validation

Check GitHub Actions for the exact commit and target operating system before describing a build as passing. A successful desktop build validates compilation and tests; it does not prove that a console module is compatible or that every imported game is playable.

## Language packs

French and English translation catalogs are stored in `assets/languages/`. The application loads the selected language pack next to the executable at runtime. Missing or invalid packs must not prevent startup.
