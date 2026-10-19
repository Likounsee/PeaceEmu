#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Game Boy Advance hardware primitives for PeaceEmu.
//!
//! The package keeps the internal development codename `gameboy`, but targets
//! the Game Boy Advance only. It intentionally does not claim to be a playable
//! emulator yet: cartridge validation and the initial memory bus are being
//! built and tested before CPU/peripheral integration.

/// Game Boy Advance cartridge header parsing and validation.
pub mod cartridge;
/// Initial Game Boy Advance memory-bus implementation.
pub mod memory;

pub use cartridge::{CartridgeError, GbaCartridge};
pub use memory::MemoryBus;
