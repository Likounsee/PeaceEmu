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

## Definition of done for P1

A user must be able to install and start PeaceEmu without a Rust toolchain, Git checkout or build directory. Verify first launch, application-data/configuration paths, import and launch errors, settings persistence, session closure, uninstall behavior and update failure recovery. The application must not present a placeholder or a successful ROM import as proof that a console is playable.

## Immediate work

- [ ] Finish renaming repository paths and fix all workflow triggers, working directories, links and update-manifest URLs.
- [ ] Run workspace formatting, tests, Clippy and documentation generation; inspect CI results rather than assuming success.
- [ ] Run the desktop workflow on Linux, macOS and Windows and fix failures.
- [ ] Test the Windows installer and uninstall/update paths in a clean environment.
- [ ] On version tags matching the desktop package, publish the Windows installer and atomically update the stable manifest with its release URL, byte size and SHA-256; keep install/replace as a separate, explicit next step.
- [ ] Review the desktop's library, session, settings, error and empty-state flows.
- [ ] Keep Game Boy work limited to fixes needed for shared contracts until P1 is complete.

## Progress policy

Progress percentages are estimates, not measurements of time spent. Update them only after reviewing code and observable acceptance criteria. Mark a task complete only when its stated checks have actually run and passed. CI status, platform support and playable-console compatibility must never be inferred from code being present.
