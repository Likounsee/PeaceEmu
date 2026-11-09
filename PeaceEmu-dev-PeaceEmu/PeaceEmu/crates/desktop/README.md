# PeaceEmu Desktop

The native desktop application uses eframe/egui and the shared `ApplicationController`. It is the main user-facing host; the console cores remain experimental and no playable system is currently claimed.

## Implemented flows

- Dashboard with library, session and system summaries.
- Multi-file content import with accepted/rejected summaries and duplicate protection.
- Persistent library paths, startup revalidation, search, sorting and Game Boy file filters.
- Content selection and launch, session management, and pause/resume/stop/reset controls.
- Persistent appearance preferences and explicit empty/error states.
- Bounded execution ticks and bounded RGBA frame conversion; failed ticks clear stale cached video.
- Semantic keyboard input with duplicate key-down suppression and release recovery when focus/page changes.
- Explicit per-session core errors and declared audio/video capability states.

Importing a file does not prove that it is valid or playable. Console cores are maintained in separate repositories; this host repository does not bundle a console implementation. Audio playback is not yet integrated.

## Boundaries

- `main.rs` composes navigation and the top-level application lifecycle.
- Page rendering is split across `*_page.rs` modules.
- Input/video adapters live in `game_view.rs`; preference persistence lives in `preferences.rs`.
- Session commands, read-only summaries, bounded execution and media draining go through `ApplicationController`.
- Console-specific behavior belongs in core modules, not desktop pages.

## Build and run

From the workspace root:

```sh
cargo run --manifest-path crates/desktop/Cargo.toml
cargo test --manifest-path crates/desktop/Cargo.toml
cargo clippy --manifest-path crates/desktop/Cargo.toml --all-targets -- -D warnings
```

## Validation

Tests in the repository are not considered validated until they have been run successfully. Check the GitHub Actions results for the exact commit and supported operating systems before claiming the desktop build is green.


## External language packs

French and English UI translation catalogs are stored as separate JSON files in `assets/languages/`. The desktop app loads `languages/<language>.json` next to the executable at runtime; the source-tree asset directory is used only for development. The Windows installer stages and installs the language directory separately, so these translation catalogs are not embedded in the executable. Invalid or missing packs do not crash startup.
