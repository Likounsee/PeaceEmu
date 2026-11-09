# State persistence policy

PeaceEmu distinguishes three different kinds of data. They must not share a format or be advertised as interchangeable.

## Emulator save states

A save state captures the emulated machine at a point in time so execution can resume later. The core owns the internal payload and must validate it before mutating machine state. The common envelope records the system ID and core implementation version, enforces a size limit, and detects accidental corruption with CRC-32; it does not authenticate data.

The host persists envelopes through `SaveStateStore`, which writes a synced temporary file and retains a recoverable backup. The terminal frontend exposes this operation as `save-state <path>` and `load-state <path>`. Both commands check the active core's `save_states` capability. They report unsupported capability or I/O/compatibility errors without treating the operation as successful.

## Persistent game saves

Persistent game saves are data written by the emulated software to its own save medium (for example, cartridge RAM or another platform-specific storage device). They are not emulator save states and must use a separately defined core/host contract and persistence policy. The `persistent_saves` capability describes this separately from `save_states`.

## Application configuration

Frontend preferences (such as interface behavior and paths) belong to application configuration. They must not be stored inside game-save data or emulator-state payloads. Configuration persistence is a separate future feature.

## Current implementation boundary

The host provides generic save-state storage, but no console core is bundled in this repository. A module must explicitly advertise save-state and persistent-game-save support before the host enables those operations. Its capability flags remain authoritative; the frontend must display the unsupported result instead of implying that saving is available. The current envelope checksum is an integrity check, not a security boundary.
