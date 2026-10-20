//! Cartridge backup-device identification.

/// Backup-device type inferred from ROM signatures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SaveType {
    /// 32 KiB byte-wide static RAM.
    Sram,
    /// 64 KiB flash memory.
    Flash64K,
    /// 128 KiB flash memory, indicated by the `FLASH1M_V` signature.
    Flash128K,
    /// Serial EEPROM with a capacity that must be determined separately.
    Eeprom,
}

impl SaveType {
    /// Detects a common GBA backup signature. Unknown images are not guessed.
    pub fn detect(rom: &[u8]) -> Option<Self> {
        let has = |needle: &[u8]| rom.windows(needle.len()).any(|window| window == needle);

        if has(b"FLASH1M_V") {
            Some(Self::Flash128K)
        } else if has(b"FLASH512_V") || has(b"FLASH_V") {
            Some(Self::Flash64K)
        } else if has(b"EEPROM_V") {
            Some(Self::Eeprom)
        } else if has(b"SRAM_V") {
            Some(Self::Sram)
        } else {
            None
        }
    }

    /// Returns the known capacity; EEPROM size is not guessed from its marker.
    pub const fn capacity_bytes(self) -> Option<usize> {
        match self {
            Self::Sram => Some(32 * 1024),
            Self::Flash64K => Some(64 * 1024),
            Self::Flash128K => Some(128 * 1024),
            Self::Eeprom => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_known_signatures_and_leaves_unknown_roms_unknown() {
        let cases = [
            (&b"SRAM_V"[..], SaveType::Sram),
            (&b"FLASH_V"[..], SaveType::Flash64K),
            (&b"FLASH512_V"[..], SaveType::Flash64K),
            (&b"FLASH1M_V"[..], SaveType::Flash128K),
            (&b"EEPROM_V"[..], SaveType::Eeprom),
        ];

        for (marker, expected) in cases {
            assert_eq!(SaveType::detect(marker), Some(expected));
        }

        assert_eq!(SaveType::detect(b"no save marker"), None);
    }

    #[test]
    fn does_not_guess_eeprom_capacity() {
        assert_eq!(SaveType::Eeprom.capacity_bytes(), None);
    }
}
