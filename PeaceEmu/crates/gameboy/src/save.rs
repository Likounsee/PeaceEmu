//! Cartridge backup-device identification and Flash command emulation.

use crate::memory::SramSizeError;

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
    fn reports_only_capacities_known_from_the_save_type() {
        assert_eq!(SaveType::Sram.capacity_bytes(), Some(32 * 1024));
        assert_eq!(SaveType::Flash64K.capacity_bytes(), Some(64 * 1024));
        assert_eq!(SaveType::Flash128K.capacity_bytes(), Some(128 * 1024));
        assert_eq!(SaveType::Eeprom.capacity_bytes(), None);
    }
}


/// Flash command-sequencer state for the GBA's byte-wide backup devices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FlashCommand {
    Ready,
    Unlock1,
    Unlock2,
    Program,
    EraseUnlock1,
    EraseUnlock2,
    EraseConfirm,
    BankSelect,
}

/// A GBA Flash backup device (64 KiB or 128 KiB).
///
/// Programming can only clear bits; sector/chip erase restores them to 1.
/// Command addresses are the low 16 bits of the Game Pak save-window address.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlashDevice {
    data: Vec<u8>,
    bank: usize,
    id_mode: bool,
    command: FlashCommand,
}

impl FlashDevice {
    /// Creates a blank Flash device for a supported Flash save type.
    pub fn new(save_type: SaveType) -> Option<Self> {
        let capacity = match save_type {
            SaveType::Flash64K => 64 * 1024,
            SaveType::Flash128K => 128 * 1024,
            _ => return None,
        };
        Some(Self {
            data: vec![0xFF; capacity],
            bank: 0,
            id_mode: false,
            command: FlashCommand::Ready,
        })
    }

    /// Returns the Flash capacity in bytes.
    pub fn capacity_bytes(&self) -> usize {
        self.data.len()
    }

    /// Returns the persistent Flash image.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Replaces the complete persistent image, rejecting mismatched sizes atomically.
    pub fn load_data(&mut self, data: &[u8]) -> Result<(), SramSizeError> {
        if data.len() != self.data.len() {
            return Err(SramSizeError {
                expected: self.data.len(),
                actual: data.len(),
            });
        }
        self.data.copy_from_slice(data);
        Ok(())
    }

    /// Reads one byte from the currently selected Flash bank.
    pub fn read(&self, address: u16) -> u8 {
        if self.id_mode {
            // Common Macronix-compatible IDs used by GBA Flash cartridges.
            return match (address, self.data.len()) {
                (0, 64_000..=65_536) => 0xC2,
                (1, 64_000..=65_536) => 0x1C,
                (0, _) => 0x62,
                (1, _) => 0x13,
                _ => 0xFF,
            };
        }
        self.data[self.bank * 0x1_0000 + usize::from(address)]
    }

    /// Feeds a byte write into the Flash command protocol.
    pub fn write(&mut self, address: u16, value: u8) {
        if value == 0xF0 {
            self.id_mode = false;
            self.command = FlashCommand::Ready;
            return;
        }

        let command = self.command;
        self.command = FlashCommand::Ready;
        match command {
            FlashCommand::Ready => {
                if address == 0x5555 && value == 0xAA {
                    self.command = FlashCommand::Unlock1;
                }
            }
            FlashCommand::Unlock1 => {
                if address == 0x2AAA && value == 0x55 {
                    self.command = FlashCommand::Unlock2;
                }
            }
            FlashCommand::Unlock2 if address == 0x5555 => match value {
                0xA0 => self.command = FlashCommand::Program,
                0x80 => self.command = FlashCommand::EraseUnlock1,
                0x90 => self.id_mode = true,
                0xB0 if self.data.len() == 128 * 1024 => {
                    self.command = FlashCommand::BankSelect;
                }
                _ => {}
            },
            FlashCommand::Program => {
                let index = self.bank * 0x1_0000 + usize::from(address);
                self.data[index] &= value;
            }
            FlashCommand::EraseUnlock1 => {
                if address == 0x5555 && value == 0xAA {
                    self.command = FlashCommand::EraseUnlock2;
                }
            }
            FlashCommand::EraseUnlock2 => {
                if address == 0x2AAA && value == 0x55 {
                    self.command = FlashCommand::EraseConfirm;
                }
            }
            FlashCommand::EraseConfirm => {
                if value == 0x10 && address == 0x5555 {
                    self.data.fill(0xFF);
                } else if value == 0x30 {
                    let start = self.bank * 0x1_0000 + (usize::from(address) & !0x0FFF);
                    self.data[start..start + 0x1000].fill(0xFF);
                }
            }
            FlashCommand::BankSelect => {
                if address == 0 {
                    self.bank = usize::from(value & 1);
                }
            }
            FlashCommand::Unlock2 | FlashCommand::EraseUnlock1 => {}
        }
    }
}

#[cfg(test)]
mod flash_tests {
    use super::*;

    fn unlock_flash(flash: &mut FlashDevice) {
        flash.write(0x5555, 0xAA);
        flash.write(0x2AAA, 0x55);
    }

    #[test]
    fn flash_programming_only_clears_bits_and_persists_data() {
        let mut flash = FlashDevice::new(SaveType::Flash64K).unwrap();
        unlock_flash(&mut flash);
        flash.write(0x5555, 0xA0);
        flash.write(0x0123, 0x5A);
        assert_eq!(flash.read(0x0123), 0x5A);

        unlock_flash(&mut flash);
        flash.write(0x5555, 0xA0);
        flash.write(0x0123, 0xFF);
        assert_eq!(flash.read(0x0123), 0x5A);
    }

    #[test]
    fn flash_sector_and_chip_erase_restore_erased_bytes() {
        let mut flash = FlashDevice::new(SaveType::Flash64K).unwrap();
        unlock_flash(&mut flash);
        flash.write(0x5555, 0xA0);
        flash.write(0x1234, 0x00);
        flash.write(0x2234, 0x00);

        unlock_flash(&mut flash);
        flash.write(0x5555, 0x80);
        flash.write(0x5555, 0xAA);
        flash.write(0x2AAA, 0x55);
        flash.write(0x1234, 0x30);
        assert_eq!(flash.read(0x1234), 0xFF);
        assert_eq!(flash.read(0x2234), 0x00);

        unlock_flash(&mut flash);
        flash.write(0x5555, 0x80);
        flash.write(0x5555, 0xAA);
        flash.write(0x2AAA, 0x55);
        flash.write(0x5555, 0x10);
        assert_eq!(flash.read(0x2234), 0xFF);
    }

    #[test]
    fn flash128k_bank_switching_isolated_and_64k_does_not_switch() {
        let mut flash = FlashDevice::new(SaveType::Flash128K).unwrap();
        unlock_flash(&mut flash);
        flash.write(0x5555, 0xA0);
        flash.write(0x0001, 0x12);
        unlock_flash(&mut flash);
        flash.write(0x5555, 0xB0);
        flash.write(0x0000, 1);
        assert_eq!(flash.read(1), 0xFF);
        unlock_flash(&mut flash);
        flash.write(0x5555, 0xA0);
        flash.write(0x0001, 0x34);
        assert_eq!(flash.read(1), 0x34);
        unlock_flash(&mut flash);
        flash.write(0x5555, 0xB0);
        flash.write(0x0000, 0);
        assert_eq!(flash.read(1), 0x12);
    }

    #[test]
    fn flash_id_mode_and_save_image_size_validation() {
        let mut flash = FlashDevice::new(SaveType::Flash64K).unwrap();
        unlock_flash(&mut flash);
        flash.write(0x5555, 0x90);
        assert_eq!(flash.read(0), 0xC2);
        assert_eq!(flash.read(1), 0x1C);
        flash.write(0, 0xF0);
        assert_eq!(flash.read(0), 0xFF);
        assert!(flash.load_data(&[0; 32]).is_err());
        assert_eq!(flash.data().len(), 64 * 1024);
    }
}
