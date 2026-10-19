# PeaceEmu — roadmap

## Direction

**Priority: finish the generic desktop application before expanding console emulation.** Keep shared runtime fixes focused on issues that block the application. Do not count planned features, fake cores, successful compilation, or an unverified CI run as proof of a working emulator.

## Current priorities

| Priority | Focus | Exit criteria |
| --- | --- | --- |
| **P0 — maintenance** | Shared runtime, content validation, core contracts, session lifecycle and persistence | Regressions fixed; contracts covered by repeatable tests |
| **P1 — current** | A usable, locally installable desktop application | Reliable library/import/launch flows, sessions, settings, diagnostics, media states, tests and a working Windows installer |
| **P1.1** | UI/controller boundary | UI uses explicit application-controller APIs; pages do not manipulate runtime internals |
| **P1.2** | Library and launch | Import, search, select, validate, launch and error recovery behave consistently |
| **P1.3** | Sessions and media | Pause/resume/stop/reset and close paths are safe; video/audio support is reported honestly |
| **P1.4** | Preferences and localization | Settings persist safely; visible strings and language behavior are consistent |
| **P1.5** | Quality gates | Formatting, tests, Clippy, docs and supported-platform CI runs are green |
| **P1.6** | Packaging and updates | Installer works on a clean Windows environment; update artifacts are verified before use |
| **P2 — after P1** | Module trust, installation and diagnostics | Provenance, validation, recovery and trust boundaries are tested end to end |
| **P3 — after P1** | First playable console | One real core passes reproducible CPU, bus, peripheral and end-to-end tests |
| **P4** | Additional systems | New cores remain isolated behind the shared contracts |
| **P5** | Release and maintenance | Reproducible releases, clear compatibility claims and maintained security/legal policy |

## P1 status snapshot — 2026-10-19

### Confirmed complete

- [x] **Version 0.2.3 released.** The GitHub release contains `PeaceEmu-Setup-0.2.3.exe` and its SHA-256 sidecar. The stable update manifest identifies version `0.2.3` and matches the published installer size and digest.
- [x] **Startup update prompt implemented.** On Windows, PeaceEmu checks for a newer stable version in the background and shows a small non-blocking prompt with “Install now” / “Later”. Installation is user initiated; the downloaded installer is size/SHA-256 verified before the existing install launcher runs.
- [x] **CI and desktop quality gates passed for the v0.2.3 source commit** (`9fd5a1db390db181700b555b8871e9cd2ff72216`): workspace and desktop formatting, tests, Clippy, docs, package checks and dependency/license audits where configured passed across the supported CI matrix.
- [x] **Desktop smoke test passed**, including building and starting the application on the CI runner.
- [x] **Windows installer smoke test passed** on a clean hosted Windows runner: installer/checksum verification, installation, installed-binary CLI checks, shortcut/icon checks, in-place reinstall, user-settings preservation, uninstall, and checking that the executable and shortcut are removed.
- [x] **Release automation exercised by v0.2.3.** The tagged release has the installer asset, and the stable manifest was published with its matching size and SHA-256. The updater does not silently install an update.
- [x] **P1.1 — UI/controller boundary hardened and guarded.** Production `ApplicationController` no longer exposes `host()` or `host_mut()`; those raw runtime accessors are compiled only for the controller crate's unit tests. The CLI uses the narrow `max_sessions()` API, and a desktop regression test rejects direct `HostRuntime`/`EmulationSession` access and raw host/session accessors from page, shell and display-adapter modules. Workspace CI passed on Linux, macOS and Windows for commit `4e695f5ba1a4f72b388f44b699dbb80926be2e01`; the desktop smoke test also passed.
- [x] **Use the product name for the Windows process executable.** The desktop Cargo binary target is `peaceemu`; the installer installs it as `PeaceEmu.exe` rather than `desktop-app.exe`, and the installer smoke test rejects the old filename.
- [ ] **Embed Windows executable branding and verify it in CI.** The Windows build now embeds the official bird artwork as the PE executable icon and sets `FileDescription`, `ProductName`, `InternalName`, `OriginalFilename`, and `CompanyName`. The Windows installer workflow checks those PE metadata fields and extracts the associated executable icon. Do not mark this complete until the exact commit's Windows workflow passes and a visual Task Manager check is recorded.

### Remaining before P1 can be closed

- [ ] **P1.2 — complete end-to-end library/launch acceptance.** Verify first-run and empty states, search/select, missing and unsupported files, import errors, launch failures/cancellation, and clear recovery guidance using real files in the installed application. Existing unit tests cover important library-path and search cases, but do not replace this acceptance pass.
- [ ] **P1.2 — custom game cover artwork.** In the library's selected-game details, let users choose a local image file as that game's cover, and search online for artwork and preview/select a result to apply as the cover icon. Persist cover choices per game across restarts, show them in both library tiles and the detail panel, provide replace/remove/fallback actions, validate image formats and size limits, and keep online search/download failures non-fatal. Match artwork to the game title/platform, show the source for online results, and never download or execute ROMs as part of cover lookup.
- [ ] **P1.3 — session lifecycle, media, and per-session game windows.** Exercise pause/resume/stop/reset and closing one or all sessions, including error paths and input/audio cleanup; confirm video/audio availability and limitations are communicated honestly. Current tests cover some state handling, but the complete user workflow still needs sign-off. New user requirement: launching a session should open its own native game window while the main library/settings window remains available; each window must display and control the correct session, support fullscreen independently, and close without terminating other sessions. The current desktop frontend renders one active session in one eframe window, so this needs an explicit multi-viewport/session-to-window design and acceptance tests. This is a windowing requirement, not a requirement to create a separate OS process per session; do not introduce process-per-session/IPC unless separately justified.
- [ ] **P1.4 — finish the installed-app preferences/localization pass.** Confirm language, appearance/accent settings and library paths survive restart; review visible strings and error/empty states in French and English. Preference storage already has substantial automated test coverage.
- [ ] **P1.6 — test updater failure/recovery paths end to end.** Cover offline/invalid manifest, interrupted or size-mismatched download, bad SHA-256, an installer that fails or is cancelled, and retry behavior. Confirm a failure does not delete the usable current installation or destroy a previously verified staged installer. The normal release/update path is implemented, but these negative paths must be explicitly verified before P1 sign-off.
- [ ] **Final acceptance on an ordinary Windows installation.** Record a full pass for first launch, configuration/data location, settings persistence, and visually confirm that the window, taskbar, and Task Manager use PeaceEmu branding instead of egui’s default white “e” icon. A second visual row is not proof of a second process; verify process IDs before claiming a process was removed. The user has confirmed that the normal update path works; the remaining acceptance gap is recovery after a failed update. CI install/reinstall/uninstall tests pass, but do not by themselves prove every interactive self-update failure scenario.

P1 remains open until these acceptance items are checked. A green CI run proves the tested revision passed its automated checks; it is not a substitute for user-flow and update-failure acceptance.

## Definition of done for P1

A user must be able to install and start PeaceEmu without a Rust toolchain, Git checkout or build directory. Verify first launch, application-data/configuration paths, import and launch errors, settings persistence, session closure, uninstall behavior and update failure recovery. The application must not present a placeholder or a successful ROM import as proof that a console is playable.

## Immediate work

- [x] Finish renaming repository paths and fix workflow triggers, working directories, links and update-manifest URLs.
- [x] Run workspace formatting, tests, Clippy and documentation generation; inspect CI results rather than assuming success.
- [x] Run the desktop quality workflow on Linux, macOS and Windows and fix the formatting/CI failures encountered while adding the update prompt.
- [x] Test Windows installer installation, in-place reinstall and uninstall in a clean hosted runner; verify user settings are preserved.
- [x] On version tags matching the desktop package, publish the Windows installer and update the stable manifest with its release URL, byte size and SHA-256; keep install/replace as a separate, explicit user action.
- [ ] Review the desktop's library, session, settings, error and empty-state flows (see the remaining P1 acceptance items above).
- [ ] Keep Game Boy work limited to fixes needed for shared contracts until P1 is complete.

## Progress policy

Progress percentages are estimates, not measurements of time spent. Update them only after reviewing code and observable acceptance criteria. Mark a task complete only when its stated checks have actually run and passed. CI status, platform support and playable-console compatibility must never be inferred from code being present.
