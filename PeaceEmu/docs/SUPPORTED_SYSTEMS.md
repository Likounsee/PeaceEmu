# Supported systems

## Current verified status

**NES is integrated as an experimental module, but no console is yet claimed as supported.** Integration tests prove the host contract is connected; they do not prove commercial-game compatibility.

| System | Status | Evidence |
| --- | --- | --- |
| Nintendo Entertainment System (NES) | Experimental | The desktop app registers the pinned `PeaceEmu-NES` module, detects iNES ROM headers from library launches, and adapts frame/audio/input/battery-RAM APIs. A synthetic looping ROM exercises adapter frame output; real-game compatibility remains unverified. |

## Status definitions

- **Planned:** selected as a future target; no working core.
- **Experimental:** core-related code exists but behavior is incomplete or unverified.
- **Boots tests:** starts some documented test content.
- **Partially functional:** defined test subset passes, with documented gaps.
- **Validated compatibility:** a published test suite and method pass reproducibly.
- **Extended compatibility:** broad representative corpus and regression suite results are published.

A crate name, successful compilation, header parsing, or GUI entry does not count as console support. Future entries must identify test-suite provenance, versions/hashes where licensing permits, expected results, actual results, and known failures.
