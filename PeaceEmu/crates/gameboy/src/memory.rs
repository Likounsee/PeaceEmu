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
    /// Timer reload values (TM0CNT_L through TM3CNT_L).
    timer_reload: [u16; 4],
    /// Current timer counters, advanced by `advance_cycles`.
    timer_counter: [u16; 4],
    /// Timer control bits (prescaler, cascade, IRQ and enable).
    timer_control: [u16; 4],
    /// Input cycles accumulated toward each timer's selected prescaler.
    timer_prescaler_phase: [u16; 4],
}

/// Error returned when loading save data whose size does not match the SRAM backing store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SramSizeError {
    /// Number of bytes required by this SRAM model.
    pub expected: usize,
    /// Number of bytes supplied by the caller.
    pub actual: usize,
}

impl core::fmt::Display for SramSizeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            formatter,
            "invalid SRAM save size: expected {} bytes, got {}",
            self.expected, self.actual
        )
    }
}

impl std::error::Error for SramSizeError {}

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
            timer_reload: [0; 4],
            timer_counter: [0; 4],
            timer_control: [0; 4],
            timer_prescaler_phase: [0; 4],
        }
    }

    /// Returns the SRAM save data for persistence by the frontend.
    ///
    /// The returned slice is the complete 64 KiB backing store, including
    /// mirrored address regions only once. The slice is read-only so callers
    /// cannot bypass the bus's memory semantics.
    pub fn sram_data(&self) -> &[u8] {
        &self.sram
    }

    /// Loads a complete SRAM save image.
    ///
    /// The size is validated before any bytes are changed, so a malformed save
    /// cannot partially overwrite the currently loaded save.
    pub fn load_sram_data(&mut self, data: &[u8]) -> Result<(), SramSizeError> {
        if data.len() != SRAM_SIZE {
            return Err(SramSizeError {
                expected: SRAM_SIZE,
                actual: data.len(),
            });
        }
        self.sram.copy_from_slice(data);
        Ok(())
    }

    /// Reads one byte using the current basic GBA address decoder.
    ///
    /// Unmapped and BIOS addresses return zero in this early bus model.
    pub fn read8(&self, address: u32) -> u8 {
        // Timer counters are live hardware state; control registers expose only
        // the documented low three bits plus IRQ and enable.
        if (0x0400_0100..=0x0400_010F).contains(&address) {
            let offset = address - 0x0400_0100;
            let timer = (offset / 4) as usize;
            let register_offset = offset % 4;
            let value = if register_offset < 2 {
                self.timer_counter[timer]
            } else {
                self.timer_control[timer]
            };
            return ((value >> ((register_offset & 1) * 8)) & 0xFF) as u8;
        }
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
            // Each 32 MiB Game Pak wait-state region mirrors the ROM
            // independently: regions 0x08/0x09, 0x0A/0x0B and 0x0C/0x0D
            // all begin at ROM offset zero.
            0x0800_0000..=0x09FF_FFFF => self
                .cartridge
                .read_rom_byte((address as usize - 0x0800_0000) % self.cartridge.len()),
            0x0A00_0000..=0x0BFF_FFFF => self
                .cartridge
                .read_rom_byte((address as usize - 0x0A00_0000) % self.cartridge.len()),
            0x0C00_0000..=0x0DFF_FFFF => self
                .cartridge
                .read_rom_byte((address as usize - 0x0C00_0000) % self.cartridge.len()),
            // The 64 KiB save SRAM is mirrored throughout the 32 MiB
            // Game Pak SRAM window (0x0E00_0000..=0x0FFF_FFFF).
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

    /// Advances the four GBA timers by a number of CPU clock cycles.
    ///
    /// Timer overflow reloads the configured value and raises the matching IF
    /// bit when enabled. Timers 1–3 can count overflows from the previous timer
    /// in cascade mode. The caller must supply elapsed CPU cycles; this core
    /// does not yet derive instruction-specific bus wait states.
    pub fn advance_cycles(&mut self, cycles: u32) {
        for _ in 0..cycles {
            let mut previous_overflow = false;
            for timer in 0..4 {
                let control = self.timer_control[timer];
                if control & 0x0080 == 0 {
                    previous_overflow = false;
                    continue;
                }

                let cascade = timer != 0 && control & 0x0004 != 0;
                let tick = if cascade {
                    previous_overflow
                } else {
                    let divisor = match control & 3 {
                        0 => 1,
                        1 => 64,
                        2 => 256,
                        _ => 1024,
                    };
                    self.timer_prescaler_phase[timer] += 1;
                    if self.timer_prescaler_phase[timer] >= divisor {
                        self.timer_prescaler_phase[timer] = 0;
                        true
                    } else {
                        false
                    }
                };

                previous_overflow = false;
                if tick {
                    let (next, overflow) = self.timer_counter[timer].overflowing_add(1);
                    if overflow {
                        self.timer_counter[timer] = self.timer_reload[timer];
                        if control & 0x0040 != 0 {
                            self.interrupt_flags |= 1 << (3 + timer);
                        }
                        previous_overflow = true;
                    } else {
                        self.timer_counter[timer] = next;
                    }
                }
            }
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
        // SRAM is byte-wide and uses the original byte address even for an
        // unsupported wider access; unlike ordinary halfword RAM, do not align it.
        if (0x0E00_0000..=0x0FFF_FFFF).contains(&address) {
            let byte = u16::from(self.read8(address));
            return byte | (byte << 8);
        }
        let address = address & !1;
        u16::from_le_bytes([self.read8(address), self.read8(address.wrapping_add(1))])
    }

    /// Reads a little-endian word, aligned down to a four-byte boundary.
    pub fn read32(&self, address: u32) -> u32 {
        // The 8-bit SRAM data bus repeats the byte selected by the original
        // address for wider reads, including addresses that are not word-aligned.
        if (0x0E00_0000..=0x0FFF_FFFF).contains(&address) {
            return u32::from(self.read8(address)) * 0x0101_0101;
        }
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
        // Timer registers are 16-bit. Byte writes update the corresponding
        // half of the reload/control register without bypassing timer state.
        if (0x0400_0100..=0x0400_010F).contains(&address) {
            let offset = address - 0x0400_0100;
            let timer = (offset / 4) as usize;
            let shift = (offset & 1) * 8;
            if offset % 4 < 2 {
                let mask = 0xFF_u16 << shift;
                self.timer_reload[timer] =
                    (self.timer_reload[timer] & !mask) | ((value as u16) << shift);
                let io_offset = (address - 0x0400_0000) as usize;
                self.io[io_offset] = value;
            } else {
                let old_control = self.timer_control[timer];
                let mask = 0x00C7_u16 & (0xFF_u16 << shift);
                self.timer_control[timer] =
                    (old_control & !mask) | (((value as u16) << shift) & mask);
                if old_control & 0x0080 == 0 && self.timer_control[timer] & 0x0080 != 0 {
                    self.timer_counter[timer] = self.timer_reload[timer];
                    self.timer_prescaler_phase[timer] = 0;
                }
                let io_offset = (address - 0x0400_0000) as usize;
                self.io[io_offset] = ((self.timer_control[timer] >> shift) & 0xFF) as u8;
            }
            return;
        }
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
                // Palette RAM byte writes replicate the byte across its halfword.
                let offset = (address as usize - 0x0500_0000) % PALETTE_RAM_SIZE;
                let offset = offset & !1;
                self.palette[offset] = value;
                self.palette[offset + 1] = value;
            }
            0x0600_0000..=0x06FF_FFFF => {
                // VRAM byte writes replicate across a halfword. Apply the GBA's
                // 96 KiB mirroring rule before aligning the physical offset.
                let offset = (address as usize - 0x0600_0000) % 0x2_0000;
                let offset = if offset >= 0x1_8000 {
                    offset - 0x8_000
                } else {
                    offset
                };
                let offset = (offset % VRAM_SIZE) & !1;
                self.vram[offset] = value;
                self.vram[offset + 1] = value;
            }
            // OAM does not support byte writes on GBA hardware.
            0x0700_0000..=0x07FF_FFFF => {}
            0x0E00_0000..=0x0FFF_FFFF => {
                self.sram[(address as usize - 0x0E00_0000) % SRAM_SIZE] = value
            }
            _ => {}
        }
    }

    /// Writes a little-endian halfword, aligned down to a two-byte boundary.
    pub fn write16(&mut self, address: u32, value: u16) {
        if (0x0400_0100..=0x0400_010F).contains(&address) {
            let address = address & !1;
            let offset = address - 0x0400_0100;
            let timer = (offset / 4) as usize;
            if offset % 4 == 0 {
                self.timer_reload[timer] = value;
                let io_offset = (address - 0x0400_0000) as usize;
                let bytes = value.to_le_bytes();
                self.io[io_offset] = bytes[0];
                self.io[io_offset + 1] = bytes[1];
            } else {
                let old_control = self.timer_control[timer];
                self.timer_control[timer] = value & 0x00C7;
                if old_control & 0x0080 == 0 && self.timer_control[timer] & 0x0080 != 0 {
                    self.timer_counter[timer] = self.timer_reload[timer];
                    self.timer_prescaler_phase[timer] = 0;
                } else if self.timer_control[timer] & 0x0080 == 0 {
                    self.timer_prescaler_phase[timer] = 0;
                }
                let io_offset = (address - 0x0400_0000) as usize;
                let bytes = self.timer_control[timer].to_le_bytes();
                self.io[io_offset] = bytes[0];
                self.io[io_offset + 1] = bytes[1];
            }
            return;
        }
        // SRAM is byte-wide. The bus selects the addressed byte lane, so an
        // odd-address halfword store presents the source's high byte.
        if (0x0E00_0000..=0x0FFF_FFFF).contains(&address) {
            let byte = value.rotate_right((address & 1) * 8) as u8;
            self.write8(address, byte);
            return;
        }
        let address = address & !1;

        // KEYCNT is a single 16-bit register. Apply a halfword write atomically
        // so intermediate byte values cannot spuriously latch a keypad IRQ.
        if address == 0x0400_0132 {
            let was_requested = self.keypad_irq_requested();
            self.key_control = value & 0xC3FF;
            self.latch_keypad_irq_on_rising_edge(was_requested);
            return;
        }

        let bytes = value.to_le_bytes();
        // These video-memory regions have width-specific write semantics, so
        // halfword stores must bypass write8's byte-write behavior.
        match address {
            0x0500_0000..=0x05FF_FFFF => {
                let offset = (address as usize - 0x0500_0000) % PALETTE_RAM_SIZE;
                self.palette[offset] = bytes[0];
                self.palette[offset + 1] = bytes[1];
            }
            0x0600_0000..=0x06FF_FFFF => {
                let offset = (address as usize - 0x0600_0000) % 0x2_0000;
                let offset = if offset >= 0x1_8000 { offset - 0x8_000 } else { offset };
                let offset = (offset % VRAM_SIZE) & !1;
                self.vram[offset] = bytes[0];
                self.vram[offset + 1] = bytes[1];
            }
            0x0700_0000..=0x07FF_FFFF => {
                let offset = (address as usize - 0x0700_0000) % OAM_SIZE;
                self.oam[offset] = bytes[0];
                self.oam[offset + 1] = bytes[1];
            }
            _ => {
                self.write8(address, bytes[0]);
                self.write8(address.wrapping_add(1), bytes[1]);
            }
        }
    }

    /// Writes a little-endian word, aligned down to a four-byte boundary.
    pub fn write32(&mut self, address: u32, value: u32) {
        // SRAM is byte-wide. Rotate by the original address so the selected
        // byte lane reaches SRAM, and perform only one physical byte write.
        if (0x0E00_0000..=0x0FFF_FFFF).contains(&address) {
            let byte = value.rotate_right((address & 3) * 8) as u8;
            self.write8(address, byte);
            return;
        }
        let address = address & !3;
        let bytes = value.to_le_bytes();
        self.write16(address, u16::from_le_bytes([bytes[0], bytes[1]]));
        self.write16(address.wrapping_add(2), u16::from_le_bytes([bytes[2], bytes[3]]));
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
    fn rom_mirrors_from_the_start_in_each_game_pak_region() {
        let mut rom = vec![0; 0xC0];
        rom[0] = 0xEA;
        rom[1] = 0x12;
        rom[0xBD] = rom[0xA0..=0xBC]
            .iter()
            .fold(0x19_u8, |sum, byte| sum.wrapping_add(*byte))
            .wrapping_neg();
        let cartridge = GbaCartridge::parse(&rom).unwrap();
        let bus = MemoryBus::new(cartridge);

        for address in [0x0800_0000, 0x0A00_0000, 0x0C00_0000] {
            assert_eq!(bus.read8(address), 0xEA, "ROM base at {address:#010X}");
            assert_eq!(bus.read8(address + 1), 0x12, "ROM byte at {:#010X}", address + 1);
        }
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
    fn keycnt_halfword_write_does_not_latch_irq_from_intermediate_value() {
        let mut bus = MemoryBus::new(cartridge());
        bus.set_keys_pressed(1);

        // IRQ enabled but no keys selected: condition remains inactive.
        bus.write16(0x0400_0132, 0x4000);
        assert_eq!(bus.read16(0x0400_0202), 0);

        // Final value selects A but disables IRQ. A byte-at-a-time write
        // would transiently combine A with the old IRQ-enable bit.
        bus.write16(0x0400_0132, 0x0001);
        assert_eq!(bus.read16(0x0400_0132), 0x0001);
        assert_eq!(bus.read16(0x0400_0202), 0);
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
    fn timers_reload_raise_irq_and_cascade() {
        let mut bus = MemoryBus::new(cartridge());

        // Timer 0 overflows after two ticks and raises IRQ 3.
        bus.write16(0x0400_0100, 0xFFFE);
        bus.write16(0x0400_0102, 0x00C0); // enable + IRQ, divisor 1
        assert_eq!(bus.read16(0x0400_0100), 0xFFFE);
        bus.advance_cycles(1);
        assert_eq!(bus.read16(0x0400_0100), 0xFFFF);
        assert_eq!(bus.read16(0x0400_0102), 0x00C0);
        bus.advance_cycles(1);
        assert_eq!(bus.read16(0x0400_0100), 0xFFFE);
        assert_eq!(bus.read16(0x0400_0202) & (1 << 3), 1 << 3);

        // Timer 1 in cascade mode ticks once for timer 0's overflow.
        bus.write16(0x0400_0100, 0xFFFF);
        bus.write16(0x0400_0102, 0x0080);
        bus.write16(0x0400_0104, 0x1234);
        bus.write16(0x0400_0106, 0x0084); // enable + cascade
        bus.advance_cycles(1);
        assert_eq!(bus.read16(0x0400_0104), 0x1235);
    }

    #[test]
    fn timers_apply_prescalers_and_mask_reserved_control_bits() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write16(0x0400_0100, 0);
        bus.write16(0x0400_0102, 0xFFFF); // divisor 1024; reserved bits ignored
        assert_eq!(bus.read16(0x0400_0102), 0x00C7);
        bus.advance_cycles(1023);
        assert_eq!(bus.read16(0x0400_0100), 0);
        bus.advance_cycles(1);
        assert_eq!(bus.read16(0x0400_0100), 1);
    }

    #[test]
    fn video_memory_obeys_byte_and_halfword_write_widths() {
        let mut bus = MemoryBus::new(cartridge());

        // Palette and VRAM byte writes replicate across the aligned halfword.
        bus.write8(0x0500_0001, 0x12);
        assert_eq!(bus.read16(0x0500_0000), 0x1212);
        bus.write16(0x0500_0000, 0x3456);
        assert_eq!(bus.read16(0x0500_0000), 0x3456);

        bus.write8(0x0600_0003, 0xAB);
        assert_eq!(bus.read16(0x0600_0002), 0xABAB);
        bus.write16(0x0600_0002, 0x1234);
        assert_eq!(bus.read16(0x0600_0002), 0x1234);

        // OAM byte stores are ignored, while halfword stores are accepted.
        bus.write16(0x0700_0000, 0xFFFF);
        bus.write8(0x0700_0000, 0x12);
        assert_eq!(bus.read16(0x0700_0000), 0xFFFF);
        bus.write32(0x0700_0000, 0x1234_5678);
        assert_eq!(bus.read32(0x0700_0000), 0x1234_5678);
    }

    #[test]
    fn sram_save_data_round_trips_and_rejects_wrong_sizes_atomically() {
        let mut bus = MemoryBus::new(cartridge());
        assert_eq!(bus.sram_data().len(), SRAM_SIZE);
        assert!(bus.sram_data().iter().all(|byte| *byte == 0xFF));

        let mut save = vec![0xFF; SRAM_SIZE];
        save[0x1234] = 0x5A;
        save[0xFFFF] = 0xA5;
        bus.load_sram_data(&save).unwrap();
        assert_eq!(bus.read8(0x0E00_1234), 0x5A);
        assert_eq!(bus.read8(0x0E00_FFFF), 0xA5);
        assert_eq!(bus.sram_data(), save.as_slice());

        let before = bus.sram_data().to_vec();
        let error = bus.load_sram_data(&save[..save.len() - 1]).unwrap_err();
        assert_eq!(error.expected, SRAM_SIZE);
        assert_eq!(error.actual, SRAM_SIZE - 1);
        assert_eq!(bus.sram_data(), before.as_slice());

        let error = bus.load_sram_data(&[0; SRAM_SIZE + 1]).unwrap_err();
        assert_eq!(error.actual, SRAM_SIZE + 1);
        assert_eq!(bus.sram_data(), before.as_slice());
    }

    #[test]
    fn sram_wide_accesses_follow_the_eight_bit_data_bus() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write8(0x0E00_0000, 0x12);
        bus.write8(0x0E00_0001, 0x34);

        assert_eq!(bus.read16(0x0E00_0000), 0x1212);
        assert_eq!(bus.read32(0x0E00_0000), 0x1212_1212);

        bus.write16(0x0E00_0000, 0xABCD);
        assert_eq!(bus.read8(0x0E00_0000), 0xCD);
        assert_eq!(bus.read8(0x0E00_0001), 0x34);
        bus.write16(0x0E00_0001, 0xABCD);
        assert_eq!(bus.read8(0x0E00_0001), 0xAB);

        bus.write32(0x0E00_0000, 0x7654_3210);
        assert_eq!(bus.read8(0x0E00_0000), 0x10);
        assert_eq!(bus.read8(0x0E00_0001), 0x34);
        bus.write32(0x0E00_0003, 0x7654_3210);
        assert_eq!(bus.read8(0x0E00_0003), 0x76);
    }

    #[test]
    fn vram_mirror_and_sram_address_window_are_consistent() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write8(0x0601_8000, 0x5A);
        assert_eq!(bus.read8(0x0601_0000), 0x5A);

        // The 64 KiB save chip mirrors throughout the 32 MiB SRAM window.
        bus.write8(0x0E00_1234, 0xA5);
        assert_eq!(bus.read8(0x0E00_1234), 0xA5);
        assert_eq!(bus.read8(0x0E01_1234), 0xA5);
        assert_eq!(bus.read8(0x0F00_1234), 0xA5);

        bus.write8(0x0F00_1234, 0x3C);
        assert_eq!(bus.read8(0x0E00_1234), 0x3C);
        assert_eq!(bus.read8(0x0F00_1234), 0x3C);
    }
}
