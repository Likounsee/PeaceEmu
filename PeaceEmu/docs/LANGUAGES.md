# Language policy

## Default choices

- **Rust stable** is the default for new shared services, core orchestration, configuration, content validation, input, save management, diagnostics and new emulation implementations where the available references and testability justify it.
- **C++** is acceptable for a mature, legally reusable core or library where integration is safer and more complete than a rewrite. Prefer a narrow C ABI boundary where appropriate.
- **C** is primarily for stable native interfaces and libraries that already expose a C API.
- **Assembly** is exceptional and requires a reproducible benchmark demonstrating a meaningful improvement over optimized Rust, plus a portable fallback when feasible.
- **TypeScript/JavaScript/HTML/CSS** may be considered for a web or hybrid UI only when it brings concrete value; it must not host timing-critical emulation logic.
- **Python** may support tooling, fixture generation and benchmark analysis, not the emulation core.

## Decision checklist

For every new core or native dependency, compare performance measured end-to-end, maturity, hardware fidelity, test coverage, security history, portability, debugging, long-term maintenance, FFI complexity, license compatibility and redistribution obligations.

Do not rewrite a mature core merely to standardize language. Do not select a native dependency only because it is reputed to be fast. Keep foreign objects behind narrow interfaces and document ownership, thread safety, buffer lifetimes and error conversion.

## Current implementation

The current code is Rust only. There is no C/C++ core, native adapter, assembly, GUI framework or third-party emulation library yet. Therefore no FFI or native build toolchain is currently required. These decisions must be revisited against a concrete target platform rather than introducing languages speculatively.
