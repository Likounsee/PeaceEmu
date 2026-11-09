# Testing policy

## Current checks

From `PeaceEmu/`:

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo package --workspace --locked --no-verify
```

The CI workflow runs formatting, locked tests, warning-free documentation generation and package-content checks on Linux, Windows and macOS for pushes and pull requests targeting `PeaceEmu`. It also checks dependency advisories, licenses and source provenance with cargo-deny; Clippy runs on Linux. CI runs are not cancelled merely because another commit is pushed, so each revision can report its own result. A green build proves only that the checked-in code passes these checks on the listed runners; it does not prove console compatibility or native builds for every target architecture. The stable toolchain is declared in `rust-toolchain.toml`.

## Current test coverage

The core contract crate tests canonical and malformed identifiers (including whitespace, non-ASCII characters and invalid separators), default-false capability declarations, registry lookup and creation, duplicate registration without mutation, deterministic enumeration order, descriptor/key mismatch rejection, stable display messages for every public identifier, core-operation and registry error variant, empty-registry behavior, and explicit unsupported save/load-state behavior. Registry factory tests verify that the factory is invoked on each creation request. Console-specific test suites live in their respective module repositories. Host CI validates shared contracts and integration behavior; it does not establish console compatibility.

## Future core tests

Each core should add focused instruction/register tests, memory and bus boundary tests, interrupt/timer behavior, peripheral behavior, malformed-content tests, save/restore tests, and authorized reference programs with deterministic expected results. Keep fixtures small, redistributable and provenance-documented.

Do not remove a failing regression test merely to make CI green. Record unavailable checks explicitly and keep benchmark results separate from correctness tests. New tests should cover both the expected result and the unchanged-state guarantee when an operation is rejected. Cross-platform CI validates the current host-independent crate only; platform-specific adapters must add their own native tests.
