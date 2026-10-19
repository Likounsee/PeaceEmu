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
    key_control: u16,
    interrupt_enable: u16,
    interrupt_flags: u16,
    interrupt_master_enable: u16,
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
            key_control: 0,
            interrupt_enable: 0,
            interrupt_flags: 0,
            interrupt_master_enable: 0,
        }
    }

    /// Reads one byte using the current basic GBA address decoder.
    ///
    /// Unmapped and BIOS addresses return zero in this early bus model.
    pub fn read8(&self, address: u32) -> u8 {
        // Interrupt controller: IE, IF and IME. Reserved bits read as zero.
        for (base, value) in [
            (0x0400_0200, self.interrupt_enable),
            (0x0400_0202, self.interrupt_flags),
            (0x0400_0208, self.interrupt_master_enable),
        ] {
            if (base..=base + 1).contains(&address) {
                return ((value >> ((address - base) * 8)) & 0xFF) as u8;
            }
        }

        // KEYINPUT (0x04000130) is active-low and read-only. Bits 0..9 map
        // to A, B, Select, Start, Right, Left, Up, Down, R and L.
        if (0x0400_0130..=0x0400_0131).contains(&address) {
            let keyinput = (!self.keys_pressed) & 0x03FF;
            return ((keyinput >> ((address - 0x0400_0130) * 8)) & 0xFF) as u8;
        }
        if (0x0400_0132..=0x0400_0133).contains(&address) {
            return ((self.key_control >> ((address - 0x0400_0132) * 8)) & 0xFF) as u8;
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
        let was_requested = self.keypad_irq_requested();
        self.keys_pressed = pressed & 0x03FF;
        self.latch_keypad_irq_on_rising_edge(was_requested);
    }

    /// Returns whether KEYCNT's configured keypad interrupt condition is met.
    pub fn keypad_irq_requested(&self) -> bool {
        if self.key_control & (1 << 14) == 0 {
            return false;
        }
        let selected = self.key_control & 0x03FF;
        if selected == 0 {
            return false;
        }
        if self.key_control & (1 << 15) == 0 {
            self.keys_pressed & selected != 0
        } else {
            self.keys_pressed & selected == selected
        }
    }

    /// Returns true when IME is enabled and an enabled interrupt is pending.
    pub fn interrupt_pending(&self) -> bool {
        self.interrupt_master_enable & 1 != 0
            && self.interrupt_enable & self.interrupt_flags & 0x3FFF != 0
    }

    /// Latches the keypad IRQ (interrupt bit 12) when its condition changes
    /// from inactive to active. IF remains latched until software writes one to clear it.
    fn latch_keypad_irq_on_rising_edge(&mut self, was_requested: bool) {
        if !was_requested && self.keypad_irq_requested() {
            self.interrupt_flags |= 1 << 12;
        }
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
        // IE: only IRQ sources 0..13 are implemented by the GBA interrupt controller.
        if (0x0400_0200..=0x0400_0201).contains(&address) {
            let shift = (address - 0x0400_0200) * 8;
            let byte_mask = 0xFF_u16 << shift;
            self.interrupt_enable =
                ((self.interrupt_enable & !byte_mask) | ((value as u16) << shift)) & 0x3FFF;
            return;
        }
        // IF is write-one-to-clear; writing zero leaves pending flags unchanged.
        if (0x0400_0202..=0x0400_0203).contains(&address) {
            let shift = (address - 0x0400_0202) * 8;
            self.interrupt_flags &= !(((value as u16) << shift) & 0x3FFF);
            return;
        }
        // IME exposes only bit 0.
        if (0x0400_0208..=0x0400_0209).contains(&address) {
            if address == 0x0400_0208 {
                self.interrupt_master_enable = (value as u16) & 1;
            }
            return;
        }

        // KEYINPUT is read-only; KEYCNT accepts only key-select, IRQ-enable and
        // condition-mode bits (0..9, 14 and 15).
        if (0x0400_0130..=0x0400_0131).contains(&address) {
            return;
        }
        if (0x0400_0132..=0x0400_0133).contains(&address) {
            let was_requested = self.keypad_irq_requested();
            let shift = (address - 0x0400_0132) * 8;
            let mask = 0xC3FF_u16;
            let byte_mask = 0xFF_u16 << shift;
            let incoming = (value as u16) << shift;
            self.key_control =
                (self.key_control & !(byte_mask & mask)) | (incoming & byte_mask & mask);
            self.latch_keypad_irq_on_rising_edge(was_requested);
            return;
        }

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
    fn keycnt_masks_reserved_bits_and_evaluates_or_and_conditions() {
        let mut bus = MemoryBus::new(cartridge());
        assert_eq!(bus.read16(0x0400_0132), 0);
        assert!(!bus.keypad_irq_requested());

        // IRQ enabled, A or B pressed: OR mode triggers for either selected key.
        bus.write16(0x0400_0132, 0x4003);
        bus.set_keys_pressed(0b10);
        assert!(bus.keypad_irq_requested());
        bus.set_keys_pressed(0b100);
        assert!(!bus.keypad_irq_requested());

        // AND mode requires both selected keys to be pressed simultaneously.
        bus.write16(0x0400_0132, 0xC003);
        bus.set_keys_pressed(0b1);
        assert!(!bus.keypad_irq_requested());
        bus.set_keys_pressed(0b11);
        assert!(bus.keypad_irq_requested());

        // Reserved bits are ignored on writes and never appear on reads.
        bus.write16(0x0400_0132, 0x3C00);
        assert_eq!(bus.read16(0x0400_0132), 0);
        assert!(!bus.keypad_irq_requested());
    }

    #[test]
    fn interrupt_registers_latch_keypad_irq_and_obey_masks_and_acknowledgement() {
        let mut bus = MemoryBus::new(cartridge());

        // Select A, enable keypad IRQ, then press A: IF bit 12 must latch.
        bus.write16(0x0400_0132, 0x4001);
        bus.set_keys_pressed(1);
        assert_eq!(bus.read16(0x0400_0202), 1 << 12);
        assert!(!bus.interrupt_pending());

        // IE and IME jointly gate the CPU-visible pending interrupt.
        bus.write16(0x0400_0200, 1 << 12);
        assert!(!bus.interrupt_pending());
        bus.write16(0x0400_0208, 0xFFFF);
        assert_eq!(bus.read16(0x0400_0208), 1);
        assert!(bus.interrupt_pending());

        // IF is write-one-to-clear, including byte writes; zero doesn't clear.
        bus.write8(0x0400_0202, 0);
        assert_eq!(bus.read16(0x0400_0202), 1 << 12);
        bus.write8(0x0400_0203, 0x10);
        assert_eq!(bus.read16(0x0400_0202), 0);
        assert!(!bus.interrupt_pending());

        // A still-held key does not immediately relatch after IF is cleared;
        // release and press again creates a new keypad condition edge.
        bus.set_keys_pressed(0);
        bus.set_keys_pressed(1);
        assert_eq!(bus.read16(0x0400_0202), 1 << 12);

        // IE reserved bits and IME high bits are masked.
        bus.write16(0x0400_0200, 0xFFFF);
        assert_eq!(bus.read16(0x0400_0200), 0x3FFF);
        bus.write16(0x0400_0208, 0x0100);
        assert_eq!(bus.read16(0x0400_0208), 0);
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
