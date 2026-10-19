//! Initial memory map for the Game Boy Advance.
//!
//! This models address decoding and backing storage, not timing, DMA, open-bus
//! behavior, BIOS protection, or peripheral side effects. Those are added in
//! separate hardware-focused steps.

use crate::GbaCartridge;

/// EWRAM backing-store size (256 KiB).
pub const EWRAM_SIZE: usize = 256 * 1024;
/// IWRAM backing-store size (32 KiB).
pub const IWRAM_SIZE: usize = 32 * 1024;
/// Palette RAM backing-store size (1 KiB).
pub const PALETTE_RAM_SIZE: usize = 1024;
/// VRAM backing-store size (96 KiB).
pub const VRAM_SIZE: usize = 96 * 1024;
/// OAM backing-store size (1 KiB).
pub const OAM_SIZE: usize = 1024;
/// I/O register window size (1 KiB).
pub const IO_SIZE: usize = 1024;
/// SRAM backing-store size for the initial save-memory model (64 KiB).
pub const SRAM_SIZE: usize = 64 * 1024;

/// GBA bus and writable memory regions.
#[derive(Clone, Debug)]
pub struct MemoryBus {
    cartridge: GbaCartridge,
    ewram: Vec<u8>,
    iwram: Vec<u8>,
    io: Vec<u8>,
    palette: Vec<u8>,
    vram: Vec<u8>,
    oam: Vec<u8>,
    sram: Vec<u8>,
    /// Active-low KEYINPUT state is stored internally as a pressed-key bitmask.
    keys_pressed: u16,
}

impl MemoryBus {
    /// Creates a fresh bus with zeroed RAM and the supplied validated cartridge.
    pub fn new(cartridge: GbaCartridge) -> Self {
        Self {
            cartridge,
            ewram: vec![0; EWRAM_SIZE],
            iwram: vec![0; IWRAM_SIZE],
            io: vec![0; IO_SIZE],
            palette: vec![0; PALETTE_RAM_SIZE],
            vram: vec![0; VRAM_SIZE],
            oam: vec![0; OAM_SIZE],
            sram: vec![0xFF; SRAM_SIZE],
            keys_pressed: 0,
        }
    }

    /// Reads one byte using the current basic GBA address decoder.
    ///
    /// Unmapped and BIOS addresses return zero in this early bus model.
    pub fn read8(&self, address: u32) -> u8 {
        // KEYINPUT (0x04000130) is active-low and read-only. Bits 0..9 map
        // to A, B, Select, Start, Right, Left, Up, Down, R and L.
        if (0x0400_0130..=0x0400_0131).contains(&address) {
            let keyinput = (!self.keys_pressed) & 0x03FF;
            return ((keyinput >> ((address - 0x0400_0130) * 8)) & 0xFF) as u8;
        }

        match address {
            0x0200_0000..=0x02FF_FFFF => self.ewram[(address as usize - 0x0200_0000) % EWRAM_SIZE],
            0x0300_0000..=0x03FF_FFFF => self.iwram[(address as usize - 0x0300_0000) % IWRAM_SIZE],
            0x0400_0000..=0x0400_03FF => self.io[(address as usize - 0x0400_0000) % IO_SIZE],
            0x0500_0000..=0x05FF_FFFF => {
                self.palette[(address as usize - 0x0500_0000) % PALETTE_RAM_SIZE]
            }
            0x0600_0000..=0x06FF_FFFF => {
                let offset = (address as usize - 0x0600_0000) % 0x2_0000;
                let offset = if offset >= 0x1_8000 {
                    offset - 0x8_000
                } else {
                    offset
                };
                self.vram[offset % VRAM_SIZE]
            }
            0x0700_0000..=0x07FF_FFFF => self.oam[(address as usize - 0x0700_0000) % OAM_SIZE],
            0x0800_0000..=0x0DFF_FFFF => self
                .cartridge
                .read_rom_byte((address as usize - 0x0800_0000) % self.cartridge.len()),
            0x0E00_0000..=0x0FFF_FFFF => self.sram[(address as usize - 0x0E00_0000) % SRAM_SIZE],
            _ => 0,
        }
    }

    /// Sets the pressed GBA buttons using a bitmask (bit 0=A, 1=B, 2=Select,
    /// 3=Start, 4=Right, 5=Left, 6=Up, 7=Down, 8=R, 9=L).
    ///
    /// The hardware exposes these bits active-low through KEYINPUT; callers
    /// pass 1 for a pressed key and 0 for a released key.
    pub fn set_keys_pressed(&mut self, pressed: u16) {
        self.keys_pressed = pressed & 0x03FF;
    }

    /// Reads a little-endian halfword, aligned down to a two-byte boundary.
    pub fn read16(&self, address: u32) -> u16 {
        let address = address & !1;
        u16::from_le_bytes([self.read8(address), self.read8(address.wrapping_add(1))])
    }

    /// Reads a little-endian word, aligned down to a four-byte boundary.
    pub fn read32(&self, address: u32) -> u32 {
        let address = address & !3;
        u32::from_le_bytes([
            self.read8(address),
            self.read8(address.wrapping_add(1)),
            self.read8(address.wrapping_add(2)),
            self.read8(address.wrapping_add(3)),
        ])
    }

    /// Writes one byte to writable memory. Cartridge ROM and BIOS writes are ignored.
    pub fn write8(&mut self, address: u32, value: u8) {
        match address {
            0x0200_0000..=0x02FF_FFFF => {
                self.ewram[(address as usize - 0x0200_0000) % EWRAM_SIZE] = value
            }
            0x0300_0000..=0x03FF_FFFF => {
                self.iwram[(address as usize - 0x0300_0000) % IWRAM_SIZE] = value
            }
            0x0400_0000..=0x0400_03FF => {
                self.io[(address as usize - 0x0400_0000) % IO_SIZE] = value
            }
            0x0500_0000..=0x05FF_FFFF => {
                self.palette[(address as usize - 0x0500_0000) % PALETTE_RAM_SIZE] = value
            }
            0x0600_0000..=0x06FF_FFFF => {
                let offset = (address as usize - 0x0600_0000) % 0x2_0000;
                let offset = if offset >= 0x1_8000 {
                    offset - 0x8_000
                } else {
                    offset
                };
                self.vram[offset % VRAM_SIZE] = value;
            }
            0x0700_0000..=0x07FF_FFFF => {
                self.oam[(address as usize - 0x0700_0000) % OAM_SIZE] = value
            }
            0x0E00_0000..=0x0FFF_FFFF => {
                self.sram[(address as usize - 0x0E00_0000) % SRAM_SIZE] = value
            }
            _ => {}
        }
    }

    /// Writes a little-endian halfword, aligned down to a two-byte boundary.
    pub fn write16(&mut self, address: u32, value: u16) {
        let address = address & !1;
        let bytes = value.to_le_bytes();
        self.write8(address, bytes[0]);
        self.write8(address.wrapping_add(1), bytes[1]);
    }

    /// Writes a little-endian word, aligned down to a four-byte boundary.
    pub fn write32(&mut self, address: u32, value: u32) {
        let address = address & !3;
        for (offset, byte) in value.to_le_bytes().into_iter().enumerate() {
            self.write8(address.wrapping_add(offset as u32), byte);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GbaCartridge;

    fn cartridge() -> GbaCartridge {
        let mut rom = vec![0; 0xC0];
        let checksum = rom[0xA0..=0xBC]
            .iter()
            .fold(0x19_u8, |sum, byte| sum.wrapping_add(*byte))
            .wrapping_neg();
        rom[0xBD] = checksum;
        GbaCartridge::parse(&rom).expect("test ROM header should validate")
    }

    #[test]
    fn ram_regions_support_little_endian_access_and_mirroring() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write32(0x0200_0001, 0x1234_5678);
        assert_eq!(bus.read32(0x0200_0000), 0x1234_5678);
        assert_eq!(bus.read32(0x0204_0000), 0x1234_5678);

        bus.write16(0x0300_0002, 0xABCD);
        assert_eq!(bus.read16(0x0300_0002), 0xABCD);
        assert_eq!(bus.read16(0x0300_8002), 0xABCD);
    }

    #[test]
    fn rom_is_readable_and_writes_are_ignored() {
        let mut rom = vec![0; 0xC0];
        rom[0] = 0xEA;
        rom[0xBD] = rom[0xA0..=0xBC]
            .iter()
            .fold(0x19_u8, |sum, byte| sum.wrapping_add(*byte))
            .wrapping_neg();
        let cartridge = GbaCartridge::parse(&rom).unwrap();
        let mut bus = MemoryBus::new(cartridge);
        assert_eq!(bus.read8(0x0800_0000), 0xEA);
        bus.write8(0x0800_0000, 0x11);
        assert_eq!(bus.read8(0x0800_0000), 0xEA);
    }

    #[test]
    fn keyinput_is_active_low_read_only_and_masks_unused_bits() {
        let mut bus = MemoryBus::new(cartridge());
        assert_eq!(bus.read16(0x0400_0130), 0x03FF);

        bus.set_keys_pressed(0b10_0000_0001);
        assert_eq!(bus.read16(0x0400_0130), 0b01_1111_1110);
        assert_eq!(bus.read8(0x0400_0130), 0xFE);
        assert_eq!(bus.read8(0x0400_0131), 0x01);

        bus.write16(0x0400_0130, 0);
        assert_eq!(bus.read16(0x0400_0130), 0b01_1111_1110);
        bus.set_keys_pressed(u16::MAX);
        assert_eq!(bus.read16(0x0400_0130), 0);
    }

    #[test]
    fn vram_mirror_and_sram_backing_store_are_consistent() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write8(0x0601_8000, 0x5A);
        assert_eq!(bus.read8(0x0601_0000), 0x5A);
        bus.write8(0x0E00_1234, 0xA5);
        assert_eq!(bus.read8(0x0F00_1234), 0xA5);
        assert_eq!(bus.read8(0x0E00_0000), 0xFF);
    }
}
