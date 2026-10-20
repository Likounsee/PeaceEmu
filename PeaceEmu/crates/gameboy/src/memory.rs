//! Initial memory map for the Game Boy Advance.
//!
//! This models address decoding, backing storage, timer progression, scanline/DISPSTAT
//! timing, immediate/HBlank/VBlank DMA, and timer-triggered sound FIFO DMA.
//! Open-bus behavior, BIOS protection, and additional peripheral side effects remain
//! in development.

use std::collections::VecDeque;

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
/// SRAM backup-device capacity (32 KiB); the 64 KiB bus window mirrors it.
pub const SRAM_SIZE: usize = 32 * 1024;

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
    /// Game Pak wait-state configuration (WAITCNT, with reserved bits masked).
    wait_control: u16,
    /// Timer reload values (TM0CNT_L through TM3CNT_L).
    timer_reload: [u16; 4],
    /// Current timer counters, advanced by `advance_cycles`.
    timer_counter: [u16; 4],
    /// Timer control bits (prescaler, cascade, IRQ and enable).
    timer_control: [u16; 4],
    /// Input cycles accumulated toward each timer's selected prescaler.
    timer_prescaler_phase: [u16; 4],
    /// CPU cycles elapsed in the current 1232-cycle video scanline.
    video_cycles_in_line: u16,
    /// Current visible/blanking scanline counter (0..227).
    vcount: u16,
    /// DISPSTAT writable controls plus live VBlank/HBlank/VCOUNT-match status.
    dispstat: u16,
    /// Sound FIFO A/B byte queues (hardware capacity: 32 bytes each).
    audio_fifos: [VecDeque<u8>; 2],
    /// Last signed PCM sample held when a sound FIFO underflows.
    audio_last_samples: [i8; 2],
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
            wait_control: 0,
            timer_reload: [0; 4],
            timer_counter: [0; 4],
            timer_control: [0; 4],
            timer_prescaler_phase: [0; 4],
            video_cycles_in_line: 0,
            vcount: 0,
            dispstat: 0,
            audio_fifos: std::array::from_fn(|_| VecDeque::with_capacity(32)),
            audio_last_samples: [0; 2],
        }
    }

    /// Returns the backup-device type inferred from the loaded cartridge ROM.
    ///
    /// This is signature-based metadata only; Flash and EEPROM command protocols
    /// are not implemented by the current SRAM bus path.
    pub fn cartridge_save_type(&self) -> Option<crate::SaveType> {
        self.cartridge.save_type()
    }

    /// Returns the most recently latched signed PCM sample for FIFO A (0) or B (1).
    pub fn audio_fifo_last_sample(&self, fifo: usize) -> Option<i8> {
        self.audio_last_samples.get(fifo).copied()
    }

    /// Returns the number of queued bytes in FIFO A (0) or B (1).
    pub fn audio_fifo_len(&self, fifo: usize) -> Option<usize> {
        self.audio_fifos.get(fifo).map(VecDeque::len)
    }

    fn write_audio_fifo_word(&mut self, fifo: usize, value: u32) {
        if let Some(queue) = self.audio_fifos.get_mut(fifo) {
            for byte in value.to_le_bytes() {
                if queue.len() == 32 {
                    break;
                }
                queue.push_back(byte);
            }
        }
    }

    fn clock_audio_fifo(&mut self, timer: usize) {
        if timer > 1 || self.read16(0x0400_0084) & 0x0080 == 0 {
            return;
        }
        let sound_control = self.read16(0x0400_0082);
        for fifo in 0..2 {
            let route_mask = if fifo == 0 {
                (1 << 8) | (1 << 9)
            } else {
                (1 << 12) | (1 << 13)
            };
            let timer_bit = if fifo == 0 { 10 } else { 14 };
            if sound_control & route_mask == 0
                || usize::from((sound_control >> timer_bit) & 1) != timer
            {
                continue;
            }
            // Direct Sound holds the last DAC sample if the FIFO runs dry;
            // only a successful pop changes the output latch.
            if let Some(sample) = self.audio_fifos[fifo].pop_front() {
                self.audio_last_samples[fifo] = sample as i8;
            }
            if self.audio_fifos[fifo].len() <= 16 {
                self.trigger_audio_fifo_dma(fifo);
            }
        }
    }

    fn trigger_audio_fifo_dma(&mut self, fifo: usize) {
        // FIFO A is serviced by DMA1, FIFO B by DMA2; each request transfers four words.
        let channel = fifo + 1;
        let base = 0xB0 + channel * 12;
        let control_offset = base + 10;
        let control = u16::from_le_bytes([self.io[control_offset], self.io[control_offset + 1]]);
        let destination = u32::from_le_bytes(self.io[base + 4..base + 8].try_into().unwrap());
        let fifo_address = if fifo == 0 { 0x0400_00A0 } else { 0x0400_00A4 };
        // Sound FIFO DMA is supported only on DMA1/2, with word-sized
        // transfers, repeat enabled, incrementing source and fixed FIFO target.
        // Reject malformed configurations instead of silently applying FIFO
        // semantics to a channel programmed for an incompatible transfer.
        let source_mode = (control >> 7) & 3;
        let destination_mode = (control >> 5) & 3;
        let word_transfer = control & (1 << 10) != 0;
        let repeat = control & (1 << 9) != 0;
        if control & 0x8000 == 0
            || (control >> 12) & 3 != 3
            || !word_transfer
            || !repeat
            || source_mode != 0
            || destination_mode != 2
            || destination != fifo_address
        {
            return;
        }
        let source = u32::from_le_bytes(self.io[base..base + 4].try_into().unwrap()) & !3;
        for index in 0..4 {
            let word = self.read32(source.wrapping_add(index * 4));
            self.write_audio_fifo_word(fifo, word);
        }
        self.io[base..base + 4].copy_from_slice(&source.wrapping_add(16).to_le_bytes());
        if control & (1 << 14) != 0 {
            self.request_interrupt(1 << (8 + channel));
        }
    }

    /// Returns the SRAM save data for persistence by the frontend.
    ///
    /// The returned slice is the complete 32 KiB backing store, including
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
        // Video timing registers expose live status rather than the raw I/O bytes.
        if (0x0400_0004..=0x0400_0005).contains(&address) {
            return ((self.dispstat >> ((address - 0x0400_0004) * 8)) & 0xFF) as u8;
        }
        if (0x0400_0006..=0x0400_0007).contains(&address) {
            return ((self.vcount >> ((address - 0x0400_0006) * 8)) & 0xFF) as u8;
        }
        // Interrupt controller: IE, IF and IME. Reserved bits read as zero.
        for (base, value) in [
            (0x0400_0200, self.interrupt_enable),
            (0x0400_0202, self.interrupt_flags),
            (0x0400_0204, self.wait_control),
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
            // GBA save SRAM is mapped only in the first 64 KiB of region 0x0E.
            // The rest of regions 0x0E/0x0F are not SRAM aliases.
            0x0E00_0000..=0x0E00_FFFF => self.sram[(address as usize - 0x0E00_0000) % SRAM_SIZE],
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

    /// Advances timers and video timing by a number of CPU clock cycles.
    ///
    /// Timer overflow reloads the configured value and raises the matching IF
    /// bit when enabled. Timers 1–3 can count overflows from the previous timer
    /// in cascade mode. Video timing advances 1232 cycles per scanline and updates
    /// DISPSTAT/VCOUNT and enabled HBlank, VBlank, and VCOUNT-match IRQs. The caller
    /// supplies elapsed CPU cycles; instruction-specific bus wait states are not yet modeled.
    pub fn advance_cycles(&mut self, cycles: u32) {
        for _ in 0..cycles {
            self.advance_video_cycle();
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
                            self.request_interrupt(1 << (3 + timer));
                        }
                        self.clock_audio_fifo(timer);
                        previous_overflow = true;
                    } else {
                        self.timer_counter[timer] = next;
                    }
                }
            }
        }
    }

    /// Writes a DMA control register and performs immediate-timing transfers.
    ///
    /// HBlank and VBlank requests are driven by video timing; sound FIFO
    /// requests are serviced by the timer-overflow path. The external Game Pak
    /// DRQ request line is not modeled yet.
    fn write_dma_control(&mut self, channel: usize, value: u16) {
        if channel >= 4 {
            return;
        }
        let base = 0xB0 + channel as u32 * 12;
        let control_offset = (base + 10) as usize;
        let old = u16::from_le_bytes([self.io[control_offset], self.io[control_offset + 1]]);
        // DMA3 alone implements bit 11 (Game Pak DRQ); it is reserved on DMA0–2.
        let control_mask = if channel == 3 { 0xFFE0 } else { 0xF7E0 };
        let control = value & control_mask;
        self.io[control_offset..control_offset + 2].copy_from_slice(&control.to_le_bytes());
        if control & 0x8000 == 0 || old & 0x8000 != 0 {
            return;
        }
        // DMA3's Game Pak DRQ bit selects an external request line regardless
        // of the timing-selector bits. Do not mistake selector 0 for an
        // immediate request while DRQ mode is enabled.
        let game_pak_drq = channel == 3 && control & (1 << 11) != 0;
        if !game_pak_drq && (control >> 12) & 3 == 0 {
            self.perform_dma_transfer(channel, control);
        }
    }

    /// Fires enabled DMA channels whose timing selector matches a peripheral event.
    fn trigger_dma_timing(&mut self, timing: u16) {
        for channel in 0..4 {
            let control_offset = 0xB0 + channel * 12 + 10;
            let control = u16::from_le_bytes([
                self.io[control_offset as usize],
                self.io[control_offset as usize + 1],
            ]);
            // DMA3 Game Pak DRQ selects an external request source, not
            // the normal VBlank/HBlank/FIFO timing events. Until the cartridge
            // DRQ line is modeled, leave such transfers pending rather than
            // incorrectly firing them from the video scheduler.
            let game_pak_drq = channel == 3 && control & (1 << 11) != 0;
            if control & 0x8000 != 0 && !game_pak_drq && (control >> 12) & 3 == timing {
                self.perform_dma_transfer(channel as usize, control);
            }
        }
    }

    /// Executes one configured DMA transfer and updates its address registers.
    fn perform_dma_transfer(&mut self, channel: usize, control: u16) {
        let base = 0xB0 + channel as u32 * 12;
        let source_offset = base as usize;
        let destination_offset = (base + 4) as usize;
        let count_offset = (base + 8) as usize;
        let source = u32::from_le_bytes(
            self.io[source_offset..source_offset + 4]
                .try_into()
                .unwrap(),
        );
        let destination = u32::from_le_bytes(
            self.io[destination_offset..destination_offset + 4]
                .try_into()
                .unwrap(),
        );
        let raw_count = u16::from_le_bytes([self.io[count_offset], self.io[count_offset + 1]]);
        let count_mask = if channel == 3 { 0xFFFF } else { 0x3FFF };
        let programmed_count = raw_count & count_mask;
        let count = if programmed_count == 0 {
            if channel == 3 {
                0x1_0000
            } else {
                0x4000
            }
        } else {
            u32::from(programmed_count)
        };
        let word = control & (1 << 10) != 0;
        let step = if word { 4_u32 } else { 2_u32 };
        let source_mode = (control >> 7) & 3;
        let destination_mode = (control >> 5) & 3;
        let mut src = source & !(step - 1);
        let mut dst = destination & !(step - 1);

        for _ in 0..count {
            if word {
                let data = self.read32(src);
                self.write32(dst, data);
            } else {
                let data = self.read16(src);
                self.write16(dst, data);
            }
            src = match source_mode {
                1 => src.wrapping_sub(step),
                2 => src,
                _ => src.wrapping_add(step),
            };
            dst = match destination_mode {
                1 => dst.wrapping_sub(step),
                2 => dst,
                _ => dst.wrapping_add(step),
            };
        }

        self.io[source_offset..source_offset + 4].copy_from_slice(&src.to_le_bytes());
        let timing = (control >> 12) & 3;
        let repeat = control & (1 << 9) != 0 && timing != 0;
        // Increment/reload only reloads the destination between repeated
        // timed transfers. An immediate transfer leaves the advanced address.
        let next_destination = if destination_mode == 3 && repeat {
            destination & !(step - 1)
        } else {
            dst
        };
        self.io[destination_offset..destination_offset + 4]
            .copy_from_slice(&next_destination.to_le_bytes());

        if control & (1 << 14) != 0 {
            self.request_interrupt(1 << (8 + channel));
        }
        if !repeat {
            let control_offset = (base + 10) as usize;
            self.io[control_offset..control_offset + 2]
                .copy_from_slice(&(control & !(1 << 15)).to_le_bytes());
        }
    }

    /// Advances the PPU timing state by one CPU cycle.
    ///
    /// A scanline lasts 1232 cycles: HBlank begins at cycle 960, and VBlank
    /// begins at scanline 160. DISPSTAT status bits and IRQ flags are updated
    /// on their hardware transition edges.
    fn advance_video_cycle(&mut self) {
        self.video_cycles_in_line += 1;
        if self.video_cycles_in_line == 960 {
            self.dispstat |= 1 << 1;
            // HBlank DMA transfers are for visible scanlines only. HBlank
            // status and its IRQ still occur during VBlank, but the DMA
            // request must not start another display transfer there.
            if self.vcount < 160 {
                self.trigger_dma_timing(2);
            }
            if self.dispstat & (1 << 4) != 0 {
                self.request_interrupt(1 << 1);
            }
        }
        if self.video_cycles_in_line < 1232 {
            return;
        }

        self.video_cycles_in_line = 0;
        self.vcount += 1;
        if self.vcount == 228 {
            self.vcount = 0;
        }
        self.dispstat &= !(1 << 1);
        if self.vcount == 160 {
            self.dispstat |= 1;
            self.trigger_dma_timing(1);
            if self.dispstat & (1 << 3) != 0 {
                self.request_interrupt(1);
            }
        } else if self.vcount == 0 {
            self.dispstat &= !1;
        }

        let was_match = self.dispstat & (1 << 2) != 0;
        let matches = self.vcount == (self.dispstat >> 8);
        if matches {
            self.dispstat |= 1 << 2;
            if !was_match && self.dispstat & (1 << 5) != 0 {
                self.request_interrupt(1 << 2);
            }
        } else {
            self.dispstat &= !(1 << 2);
        }
    }

    /// Requests one or more GBA interrupt sources by setting their IF bits.
    ///
    /// Only the 14 defined interrupt sources are retained. Peripheral models
    /// such as video, DMA, and serial can use this without bypassing IF semantics.
    pub fn request_interrupt(&mut self, sources: u16) {
        self.interrupt_flags |= sources & 0x3FFF;
    }

    /// Returns enabled interrupt sources, independent of IME.
    ///
    /// This is the IE/IF intersection used by CPU exception entry once IRQ
    /// modes and interrupt masking are implemented.
    pub fn pending_interrupts(&self) -> u16 {
        self.interrupt_enable & self.interrupt_flags & 0x3FFF
    }

    /// Returns true when IME is enabled and an enabled interrupt is pending.
    pub fn interrupt_pending(&self) -> bool {
        self.interrupt_master_enable & 1 != 0 && self.pending_interrupts() != 0
    }

    /// Latches the keypad IRQ (interrupt bit 12) when its condition changes
    /// from inactive to active. IF remains latched until software writes one to clear it.
    fn latch_keypad_irq_on_rising_edge(&mut self, was_requested: bool) {
        if !was_requested && self.keypad_irq_requested() {
            self.request_interrupt(1 << 12);
        }
    }

    /// Reads a little-endian halfword, aligned down to a two-byte boundary.
    pub fn read16(&self, address: u32) -> u16 {
        // SRAM is byte-wide and uses the original byte address even for an
        // unsupported wider access; unlike ordinary halfword RAM, do not align it.
        if (0x0E00_0000..=0x0E00_FFFF).contains(&address) {
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
        if (0x0E00_0000..=0x0E00_FFFF).contains(&address) {
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
        // DMA control's high byte contains the enable bit; support STRB writes.
        if (0x0400_00B0..=0x0400_00DF).contains(&address) {
            let offset = address - 0x0400_0000;
            let register_offset = offset - 0x00B0;
            if register_offset % 12 == 10 || register_offset % 12 == 11 {
                let control_address = address & !1;
                let current = self.read16(control_address);
                let shift = (address & 1) * 8;
                let byte_mask = 0x00FF_u16 << shift;
                let next = (current & !byte_mask) | (u16::from(value) << shift);
                self.write_dma_control((register_offset / 12) as usize, next);
                return;
            }
        }
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
                } else if self.timer_control[timer] & 0x0080 == 0 {
                    // Keep byte-wide control writes consistent with write16:
                    // a disabled timer must not retain a partial prescaler phase.
                    self.timer_prescaler_phase[timer] = 0;
                }
                let io_offset = (address - 0x0400_0000) as usize;
                self.io[io_offset] = ((self.timer_control[timer] >> shift) & 0xFF) as u8;
            }
            return;
        }
        // DISPSTAT: status bits 0..2 are read-only; IRQ enables and compare
        // scanline are writable. VCOUNT itself is read-only.
        if (0x0400_0004..=0x0400_0005).contains(&address) {
            let old_match = self.dispstat & (1 << 2) != 0;
            let shift = (address - 0x0400_0004) * 8;
            let writable_mask = 0xFF_u16 << shift & 0xFF38;
            self.dispstat =
                (self.dispstat & !writable_mask) | ((u16::from(value) << shift) & writable_mask);
            let matches = self.vcount == (self.dispstat >> 8);
            if matches {
                self.dispstat |= 1 << 2;
                if !old_match && self.dispstat & (1 << 5) != 0 {
                    self.request_interrupt(1 << 2);
                }
            } else {
                self.dispstat &= !(1 << 2);
            }
            return;
        }
        if (0x0400_0006..=0x0400_0007).contains(&address) {
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
        // WAITCNT exposes SRAM/Game Pak wait-state, PHI, and prefetch controls;
        // reserved bits 2 and 15 read as zero; bits 12–14 are valid controls.
        if (0x0400_0204..=0x0400_0205).contains(&address) {
            let shift = (address - 0x0400_0204) * 8;
            let byte_mask = 0xFF_u16 << shift;
            self.wait_control =
                ((self.wait_control & !byte_mask) | ((value as u16) << shift & byte_mask)) & 0x7FFB;
            return;
        }
        // IME exposes only bit 0.
        if (0x0400_0208..=0x0400_0209).contains(&address) {
            if address == 0x0400_0208 {
                self.interrupt_master_enable = (value as u16) & 1;
            }
            return;
        }

        // Sound FIFO ports are write-only and only accept 32-bit pushes.
        if (0x0400_00A0..=0x0400_00A7).contains(&address) {
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
            0x0E00_0000..=0x0E00_FFFF => {
                self.sram[(address as usize - 0x0E00_0000) % SRAM_SIZE] = value
            }
            _ => {}
        }
    }

    /// Writes a little-endian halfword, aligned down to a two-byte boundary.
    pub fn write16(&mut self, address: u32, value: u16) {
        if (0x0400_00BA..=0x0400_00DF).contains(&address) && (address - 0x0400_00B0) % 12 == 10 {
            self.write_dma_control(((address - 0x0400_00B0) / 12) as usize, value);
            return;
        }
        if (0x0400_0004..=0x0400_0005).contains(&address) {
            let old_match = self.dispstat & (1 << 2) != 0;
            self.dispstat = (self.dispstat & 0x0007) | (value & 0xFF38);
            let matches = self.vcount == (self.dispstat >> 8);
            if matches {
                self.dispstat |= 1 << 2;
                if !old_match && self.dispstat & (1 << 5) != 0 {
                    self.request_interrupt(1 << 2);
                }
            } else {
                self.dispstat &= !(1 << 2);
            }
            return;
        }
        if (0x0400_0006..=0x0400_0007).contains(&address) {
            return;
        }
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
        if (0x0400_0204..=0x0400_0205).contains(&address) {
            self.wait_control = value & 0x7FFB;
            return;
        }
        if address == 0x0400_0082 {
            let mut control = value & 0x770F;
            if value & (1 << 11) != 0 {
                self.audio_fifos[0].clear();
                self.audio_last_samples[0] = 0;
            }
            if value & (1 << 15) != 0 {
                self.audio_fifos[1].clear();
                self.audio_last_samples[1] = 0;
            }
            control &= !((1 << 11) | (1 << 15));
            self.io[0x82..0x84].copy_from_slice(&control.to_le_bytes());
            return;
        }
        if (0x0400_00A0..=0x0400_00A7).contains(&address) {
            return;
        }
        // SRAM is byte-wide. The bus selects the addressed byte lane, so an
        // odd-address halfword store presents the source's high byte.
        if (0x0E00_0000..=0x0E00_FFFF).contains(&address) {
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
                let offset = if offset >= 0x1_8000 {
                    offset - 0x8_000
                } else {
                    offset
                };
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
        if (0x0400_00A0..=0x0400_00A7).contains(&address) {
            let fifo = usize::from(address >= 0x0400_00A4);
            self.write_audio_fifo_word(fifo, value);
            return;
        }
        // SRAM is byte-wide. Rotate by the original address so the selected
        // byte lane reaches SRAM, and perform only one physical byte write.
        if (0x0E00_0000..=0x0E00_FFFF).contains(&address) {
            let byte = value.rotate_right((address & 3) * 8) as u8;
            self.write8(address, byte);
            return;
        }
        let address = address & !3;
        let bytes = value.to_le_bytes();
        self.write16(address, u16::from_le_bytes([bytes[0], bytes[1]]));
        self.write16(
            address.wrapping_add(2),
            u16::from_le_bytes([bytes[2], bytes[3]]),
        );
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
            assert_eq!(
                bus.read8(address + 1),
                0x12,
                "ROM byte at {:#010X}",
                address + 1
            );
        }
    }

    #[test]
    fn interrupt_requests_are_masked_and_pending_sources_ignore_ime() {
        let mut bus = MemoryBus::new(cartridge());

        bus.write16(0x0400_0200, 0xFFFF); // IE: reserved bits are ignored.
        bus.request_interrupt(0xFFFF); // IF accepts only defined sources.
        assert_eq!(bus.read16(0x0400_0202), 0x3FFF);
        assert_eq!(bus.pending_interrupts(), 0x3FFF);
        assert!(!bus.interrupt_pending()); // IME remains disabled.

        bus.write16(0x0400_0208, 1);
        assert!(bus.interrupt_pending());
        bus.write16(0x0400_0202, 0x3FFF); // IF is write-one-to-clear.
        assert_eq!(bus.pending_interrupts(), 0);
        assert!(!bus.interrupt_pending());
    }

    #[test]
    fn waitcnt_masks_reserved_bits_and_supports_byte_writes() {
        let mut bus = MemoryBus::new(cartridge());

        // WAITCNT has writable timing/PHI/prefetch fields; reserved bits 2 and 15
        // must be cleared while the valid high-byte controls are preserved.
        bus.write16(0x0400_0204, 0xFFFF);
        assert_eq!(bus.read16(0x0400_0204), 0x7FFB);

        bus.write16(0x0400_0204, 0);
        bus.write8(0x0400_0204, 0xA5);
        bus.write8(0x0400_0205, 0x7F);
        assert_eq!(bus.read16(0x0400_0204), 0x7FA1);
        assert_eq!(bus.read8(0x0400_0204), 0xA1);
        assert_eq!(bus.read8(0x0400_0205), 0x7F);
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
        bus.write16(0x0400_0102, 0x0000); // disable and reset prescaler
        bus.write16(0x0400_0100, 0xFFFF);
        bus.write16(0x0400_0102, 0x0080); // re-enable to load FFFF
        bus.write16(0x0400_0104, 0x1234);
        bus.write16(0x0400_0106, 0x0084); // enable + cascade
        bus.advance_cycles(1);
        assert_eq!(bus.read16(0x0400_0104), 0x1235);
    }

    #[test]
    fn timer_cascade_propagates_overflow_through_multiple_timers() {
        let mut bus = MemoryBus::new(cartridge());

        // Each timer is one tick from overflow. Timer 0's overflow must tick
        // timer 1, whose own overflow must then tick timer 2 in the same cycle.
        bus.write16(0x0400_0100, 0xFFFF);
        bus.write16(0x0400_0102, 0x0080);
        bus.write16(0x0400_0104, 0xFFFF);
        bus.write16(0x0400_0106, 0x0084);
        bus.write16(0x0400_0108, 0x1234);
        bus.write16(0x0400_010A, 0x0084);

        bus.advance_cycles(1);

        assert_eq!(bus.read16(0x0400_0100), 0xFFFF);
        assert_eq!(bus.read16(0x0400_0104), 0xFFFF);
        assert_eq!(bus.read16(0x0400_0108), 0x1235);
    }

    #[test]
    fn disabling_timer_with_byte_write_resets_prescaler_phase() {
        let mut bus = MemoryBus::new(cartridge());

        // Divisor 64: accumulate a partial prescaler phase, then disable the
        // timer through its low control byte (the byte-wide register path).
        bus.write16(0x0400_0100, 0);
        bus.write16(0x0400_0102, 0x0081);
        bus.advance_cycles(32);
        assert_eq!(bus.read16(0x0400_0100), 0);

        bus.write8(0x0400_0102, 0x01); // Disable, retain divisor 64.
        bus.write8(0x0400_0102, 0x81); // Re-enable with divisor 64.
        bus.advance_cycles(63);
        assert_eq!(bus.read16(0x0400_0100), 0);
        bus.advance_cycles(1);
        assert_eq!(bus.read16(0x0400_0100), 1);
    }

    #[test]
    fn timer_byte_writes_update_reload_without_changing_live_counter() {
        let mut bus = MemoryBus::new(cartridge());

        bus.write8(0x0400_0100, 0xFE);
        bus.write8(0x0400_0101, 0xFF);
        bus.write8(0x0400_0102, 0x80); // Enable timer 0, divisor 1.
        bus.advance_cycles(1);
        assert_eq!(bus.read16(0x0400_0100), 0xFFFF);

        // Updating the reload register while enabled must not overwrite the
        // live counter; the new reload value takes effect at the next overflow.
        bus.write8(0x0400_0100, 0x10);
        assert_eq!(bus.read16(0x0400_0100), 0xFFFF);
        bus.advance_cycles(1);
        assert_eq!(bus.read16(0x0400_0100), 0xFF10);

        // Disabling and re-enabling loads the latest reload value immediately.
        bus.write8(0x0400_0102, 0);
        bus.write8(0x0400_0102, 0x80);
        assert_eq!(bus.read16(0x0400_0100), 0xFF10);
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
    fn video_timing_updates_hblank_vblank_and_vcount_irq_flags() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write16(0x0400_0004, (1 << 3) | (1 << 4) | (160 << 8));

        bus.advance_cycles(959);
        assert_eq!(bus.read16(0x0400_0004) & 0x0003, 0);
        bus.advance_cycles(1);
        assert_eq!(bus.read16(0x0400_0004) & 0x0002, 0x0002);
        assert_eq!(bus.read16(0x0400_0202) & (1 << 1), 1 << 1);

        bus.advance_cycles(272); // finish line 0
        assert_eq!(bus.read16(0x0400_0006), 1);
        assert_eq!(bus.read16(0x0400_0004) & 0x0002, 0);

        bus.advance_cycles(1232 * 159);
        assert_eq!(bus.read16(0x0400_0006), 160);
        assert_eq!(bus.read16(0x0400_0004) & 0x0001, 1);
        assert_eq!(bus.read16(0x0400_0202) & 1, 1);
        assert_eq!(bus.read16(0x0400_0004) & 0x0004, 0x0004);
    }

    #[test]
    fn vcount_register_is_read_only_and_dispstat_status_bits_are_read_only() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write16(0x0400_0006, 123);
        bus.write16(0x0400_0004, 0xFFFF);
        assert_eq!(bus.read16(0x0400_0006), 0);
        assert_eq!(bus.read16(0x0400_0004) & 0x0007, 0x0000);
        assert_eq!(bus.read16(0x0400_0004) & 0xFF38, 0xFF38);
    }

    #[test]
    fn vcount_compare_irq_fires_on_scanline_transition() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write16(0x0400_0004, (1 << 5) | (2 << 8));
        bus.advance_cycles(1232);
        assert_eq!(bus.read16(0x0400_0006), 1);
        assert_eq!(bus.read16(0x0400_0202) & (1 << 2), 0);
        bus.advance_cycles(1232);
        assert_eq!(bus.read16(0x0400_0006), 2);
        assert_eq!(bus.read16(0x0400_0004) & (1 << 2), 1 << 2);
        assert_eq!(bus.read16(0x0400_0202) & (1 << 2), 1 << 2);
    }

    #[test]
    fn immediate_dma_copies_halfwords_and_raises_optional_irq() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write16(0x0200_0000, 0x1234);
        bus.write16(0x0200_0002, 0xABCD);
        bus.write32(0x0400_00B0, 0x0200_0000);
        bus.write32(0x0400_00B4, 0x0300_0000);
        bus.write16(0x0400_00B8, 2);
        bus.write16(0x0400_00BA, 0xC000);
        assert_eq!(bus.read16(0x0300_0000), 0x1234);
        assert_eq!(bus.read16(0x0300_0002), 0xABCD);
        assert_eq!(bus.read16(0x0400_0202) & (1 << 8), 1 << 8);
        assert_eq!(bus.read16(0x0400_00BA) & 0x8000, 0);
    }

    #[test]
    fn immediate_dma_supports_word_transfers_and_fixed_destination() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write32(0x0200_0000, 0x1234_5678);
        bus.write32(0x0200_0004, 0xAABB_CCDD);
        bus.write32(0x0400_00BC, 0x0200_0000);
        bus.write32(0x0400_00C0, 0x0300_0000);
        bus.write16(0x0400_00C4, 2);
        bus.write16(0x0400_00C6, 0x8440);
        assert_eq!(bus.read32(0x0300_0000), 0xAABB_CCDD);
        assert_eq!(bus.read16(0x0400_00C6) & 0x8000, 0);
    }

    #[test]
    fn dma_enable_bit_written_as_byte_starts_immediate_transfer() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write16(0x0200_0000, 0x5AA5);
        bus.write32(0x0400_00B0, 0x0200_0000);
        bus.write32(0x0400_00B4, 0x0300_0000);
        bus.write16(0x0400_00B8, 1);
        bus.write8(0x0400_00BA, 0x00);
        bus.write8(0x0400_00BB, 0x80);

        assert_eq!(bus.read16(0x0300_0000), 0x5AA5);
        assert_eq!(bus.read16(0x0400_00BA) & 0x8000, 0);
    }

    #[test]
    fn dma_decrement_modes_walk_source_and_destination_backwards() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write16(0x0200_0000, 0x1111);
        bus.write16(0x0200_0002, 0x2222);
        bus.write16(0x0200_0004, 0x3333);
        bus.write32(0x0400_00B0, 0x0200_0004);
        bus.write32(0x0400_00B4, 0x0300_0004);
        bus.write16(0x0400_00B8, 3);
        bus.write16(0x0400_00BA, 0x80A0); // decrement source and destination

        assert_eq!(bus.read16(0x0300_0004), 0x3333);
        assert_eq!(bus.read16(0x0300_0002), 0x2222);
        assert_eq!(bus.read16(0x0300_0000), 0x1111);
        assert_eq!(bus.read32(0x0400_00B0), 0x01FF_FFFE);
        assert_eq!(bus.read32(0x0400_00B4), 0x02FF_FFFE);
    }

    #[test]
    fn dma_count_is_limited_by_channel_register_width() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write16(0x0200_0000, 0xCAFE);
        bus.write16(0x0200_0002, 0xBABE);
        bus.write32(0x0400_00B0, 0x0200_0000);
        bus.write32(0x0400_00B4, 0x0300_0000);
        bus.write16(0x0400_00B8, 0x4001); // DMA0 count is only 14 bits
        bus.write16(0x0400_00BA, 0x8000);

        // 0x4001 is masked to 0x0001 for DMA0, so only one halfword moves.
        assert_eq!(bus.read16(0x0300_0000), 0xCAFE);
        assert_eq!(bus.read16(0x0300_0002), 0);
        assert_eq!(bus.read16(0x0300_7FFE), 0);
    }

    #[test]
    fn immediate_dma_increment_reload_mode_keeps_advanced_destination() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write32(0x0200_0000, 0x1111_2222);
        bus.write32(0x0200_0004, 0x3333_4444);
        bus.write32(0x0400_00B0, 0x0200_0000);
        bus.write32(0x0400_00B4, 0x0300_0000);
        bus.write16(0x0400_00B8, 2);
        bus.write16(0x0400_00BA, 0x8460); // immediate, 32-bit, increment/reload destination

        assert_eq!(bus.read32(0x0300_0000), 0x1111_2222);
        assert_eq!(bus.read32(0x0300_0004), 0x3333_4444);
        assert_eq!(bus.read32(0x0400_00B4), 0x0300_0008);
    }

    #[test]
    fn dma_control_low_byte_writes_apply_reserved_bit_mask() {
        let mut bus = MemoryBus::new(cartridge());
        // Only control bits 5–7 are writable in the low byte; bits 0–4
        // are reserved and must be masked even for STRB writes.
        bus.write8(0x0400_00BA, 0xFF);
        assert_eq!(bus.read16(0x0400_00BA) & 0x00FF, 0x00E0);

        // The low-byte path must preserve the high byte, including enable.
        bus.write8(0x0400_00BB, 0x90); // enable on VBlank, not immediate
        bus.write8(0x0400_00BA, 0x00);
        assert_eq!(bus.read16(0x0400_00BA) & 0x8000, 0x8000);
    }

    #[test]
    fn dma3_preserves_game_pak_drq_control_bit_but_dma0_masks_it() {
        let mut bus = MemoryBus::new(cartridge());

        bus.write16(0x0400_00BA, 0x9800); // DMA0: reserved bit 11, VBlank, enabled
        assert_eq!(bus.read16(0x0400_00BA) & 0x0800, 0);

        bus.write16(0x0400_00DE, 0x9800); // DMA3: Game Pak DRQ, VBlank, enabled
        assert_eq!(bus.read16(0x0400_00DE) & 0x0800, 0x0800);
    }

    #[test]
    fn dma_control_high_byte_masks_drq_by_channel() {
        let mut bus = MemoryBus::new(cartridge());
        // A high-byte STRB write must preserve DMA3's DRQ selector but mask
        // the reserved bit on DMA0. VBlank timing avoids an immediate transfer.
        bus.write8(0x0400_00BB, 0x98);
        bus.write8(0x0400_00DF, 0x98);
        assert_eq!(bus.read16(0x0400_00BA), 0x9000);
        assert_eq!(bus.read16(0x0400_00DE), 0x9800);
    }

    #[test]
    fn dma3_game_pak_drq_waits_for_external_request_not_vblank() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write16(0x0200_0000, 0xCAFE);
        bus.write32(0x0400_00D4, 0x0200_0000);
        bus.write32(0x0400_00D8, 0x0300_0000);
        bus.write16(0x0400_00DC, 1);
        bus.write16(0x0400_00DE, 0x9800); // DMA3 DRQ + VBlank selector + enable

        bus.advance_cycles(1232 * 160);

        assert_eq!(bus.read16(0x0300_0000), 0);
        assert_eq!(bus.read16(0x0400_00DE) & 0x8000, 0x8000);
    }

    #[test]
    fn dma3_game_pak_drq_does_not_start_as_immediate_dma() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write16(0x0200_0000, 0xCAFE);
        bus.write32(0x0400_00D4, 0x0200_0000);
        bus.write32(0x0400_00D8, 0x0300_0000);
        bus.write16(0x0400_00DC, 1);
        // DRQ is an external request source; selector 0 must not override it.
        bus.write16(0x0400_00DE, 0x8800);
        assert_eq!(bus.read16(0x0300_0000), 0);
        assert_eq!(bus.read16(0x0400_00DE) & 0x8000, 0x8000);
    }

    #[test]
    fn timed_dma_stays_enabled_until_its_trigger_exists() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write16(0x0200_0000, 0xCAFE);
        bus.write16(0x0200_0002, 0xFACE);
        bus.write32(0x0400_00B0, 0x0200_0000);
        bus.write32(0x0400_00B4, 0x0300_0000);
        bus.write16(0x0400_00B8, 1);
        bus.write16(0x0400_00BA, 0x9000);
        assert_eq!(bus.read16(0x0300_0000), 0);
        assert_eq!(bus.read16(0x0400_00BA) & 0x8000, 0x8000);
    }
    #[test]
    fn hblank_dma_does_not_trigger_during_vblank_scanlines() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write16(0x0200_0000, 0xCAFE);

        // Reach VBlank before enabling HBlank DMA so earlier visible-line
        // HBlank requests cannot consume the one-word transfer.
        bus.advance_cycles(1232 * 160);
        assert_eq!(bus.read16(0x0400_0006), 160);
        bus.write32(0x0400_00B0, 0x0200_0000);
        bus.write32(0x0400_00B4, 0x0300_0000);
        bus.write16(0x0400_00B8, 1);
        bus.write16(0x0400_00BA, 0xA000); // enable HBlank DMA
        bus.advance_cycles(960);

        assert_eq!(bus.read16(0x0300_0000), 0);
        assert_eq!(bus.read16(0x0400_00BA) & 0x8000, 0x8000);
        // DISPSTAT still reports HBlank during VBlank.
        assert_eq!(bus.read16(0x0400_0004) & 2, 2);
    }

    #[test]
    fn vblank_dma_transfers_only_when_vblank_begins_and_repeats_when_configured() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write16(0x0200_0000, 0xCAFE);
        bus.write16(0x0200_0002, 0xFACE);
        bus.write32(0x0400_00B0, 0x0200_0000);
        bus.write32(0x0400_00B4, 0x0300_0000);
        bus.write16(0x0400_00B8, 1);
        bus.write16(0x0400_00BA, 0x9260); // enable, repeat, VBlank, destination reload

        assert_eq!(bus.read16(0x0300_0000), 0);
        bus.advance_cycles(1232 * 159);
        assert_eq!(bus.read16(0x0300_0000), 0);
        bus.advance_cycles(1232);
        assert_eq!(bus.read16(0x0300_0000), 0xCAFE);
        assert_eq!(bus.read32(0x0400_00B4), 0x0300_0000);
        assert_eq!(bus.read16(0x0400_00BA) & 0x8000, 0x8000);
        assert_eq!(bus.read16(0x0400_0202) & (1 << 8), 0);

        // A repeated VBlank transfer reuses the count and continues the source
        // address on the following frame.
        bus.advance_cycles(1232 * 228);
        assert_eq!(bus.read16(0x0300_0000), 0xFACE);
        assert_eq!(bus.read16(0x0300_0002), 0);
        assert_eq!(bus.read32(0x0400_00B4), 0x0300_0000);
        assert_eq!(bus.read16(0x0400_00BA) & 0x8000, 0x8000);
    }

    #[test]
    fn hblank_dma_fires_at_the_hblank_boundary() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write16(0x0200_0000, 0xBEEF);
        bus.write32(0x0400_00B0, 0x0200_0000);
        bus.write32(0x0400_00B4, 0x0300_0000);
        bus.write16(0x0400_00B8, 1);
        bus.write16(0x0400_00BA, 0xA000); // enable, HBlank timing

        bus.advance_cycles(959);
        assert_eq!(bus.read16(0x0300_0000), 0);
        bus.advance_cycles(1);
        assert_eq!(bus.read16(0x0300_0000), 0xBEEF);
        assert_eq!(bus.read16(0x0400_00BA) & 0x8000, 0);
    }

    #[test]
    fn sram_save_data_round_trips_and_rejects_wrong_sizes_atomically() {
        let mut bus = MemoryBus::new(cartridge());
        assert_eq!(bus.sram_data().len(), SRAM_SIZE);
        assert!(bus.sram_data().iter().all(|byte| *byte == 0xFF));

        let mut save = vec![0xFF; SRAM_SIZE];
        save[0x1234] = 0x5A;
        save[0x7FFF] = 0xA5;
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
        assert_eq!(bus.read8(0x0E00_0001), 0xAB);
        bus.write32(0x0E00_0003, 0x7654_3210);
        assert_eq!(bus.read8(0x0E00_0003), 0x76);
    }

    #[test]
    fn sound_fifo_waits_for_master_enable_and_selected_timer() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write32(0x0400_00A0, 0x4433_2211);
        bus.write16(0x0400_0082, 1 << 8); // FIFO A uses timer 0 (select bit clear).

        // Timer 1 overflow must not clock a FIFO assigned to timer 0.
        bus.write16(0x0400_0104, 0xFFFF);
        bus.write16(0x0400_0106, 0x0080);
        bus.advance_cycles(1);
        assert_eq!(bus.audio_fifo_len(0), Some(4));
        assert_eq!(bus.audio_fifo_last_sample(0), Some(0));

        // Timer 0 is selected, but FIFO playback is still gated by SOUNDCNT_X.
        bus.write16(0x0400_0100, 0xFFFF);
        bus.write16(0x0400_0102, 0x0080);
        bus.advance_cycles(1);
        assert_eq!(bus.audio_fifo_len(0), Some(4));
        assert_eq!(bus.audio_fifo_last_sample(0), Some(0));

        // Enabling the sound master allows the selected timer to consume one byte.
        bus.write16(0x0400_0084, 0x0080);
        bus.advance_cycles(1);
        assert_eq!(bus.audio_fifo_len(0), Some(3));
        assert_eq!(bus.audio_fifo_last_sample(0), Some(0x11));
    }

    #[test]
    fn sound_fifo_underflow_holds_the_last_output_sample() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write16(0x0400_0084, 0x0080); // Sound master enabled.
        bus.write16(0x0400_0082, 1 << 8); // FIFO A uses timer 0.
        bus.write32(0x0400_00A0, 0x0000_0080); // First sample is -128.
        bus.write16(0x0400_0100, 0xFFFF);
        bus.write16(0x0400_0102, 0x0080); // Timer 0 enabled, divisor 1.

        bus.advance_cycles(1);
        assert_eq!(bus.audio_fifo_last_sample(0), Some(-128));
        assert_eq!(bus.audio_fifo_len(0), Some(3));

        // Drain the remaining bytes, then clock the empty FIFO. The DAC latch
        // must retain the most recent sample instead of snapping to zero.
        bus.advance_cycles(3);
        assert_eq!(bus.audio_fifo_len(0), Some(0));
        assert_eq!(bus.audio_fifo_last_sample(0), Some(0));
        bus.write32(0x0400_00A0, 0x0000_007F);
        bus.advance_cycles(1);
        assert_eq!(bus.audio_fifo_last_sample(0), Some(127));
        bus.advance_cycles(1);
        assert_eq!(bus.audio_fifo_last_sample(0), Some(0));
    }

    #[test]
    fn sound_fifo_accepts_word_writes_caps_at_32_bytes_and_resets() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write8(0x0400_00A0, 0xAA);
        bus.write16(0x0400_00A0, 0xBBBB);
        assert_eq!(bus.audio_fifo_len(0), Some(0));

        for word in 0..10_u32 {
            bus.write32(0x0400_00A0, 0x0302_0100 + word * 0x0404_0404);
        }
        assert_eq!(bus.audio_fifo_len(0), Some(32));
        bus.write16(0x0400_0082, (1 << 8) | (1 << 11));
        assert_eq!(bus.audio_fifo_len(0), Some(0));
        assert_eq!(bus.read16(0x0400_0082) & (1 << 11), 0);
    }

    #[test]
    fn sound_timer_overflow_requests_fifo_dma_in_four_word_burst() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write16(0x0400_0084, 0x0080);
        bus.write16(0x0400_0082, 1 << 8); // FIFO A uses timer 0.
        for i in 0..4_u32 {
            bus.write32(0x0200_0000 + i * 4, 0x1111_1111 * (i + 1));
            bus.write32(0x0400_00A0, 0x8081_8283 + i * 0x0404_0404);
        }
        assert_eq!(bus.audio_fifo_len(0), Some(16));

        // DMA1: EWRAM -> FIFO A, fixed destination, 32-bit, repeat, FIFO timing.
        bus.write32(0x0400_00BC, 0x0200_0000);
        bus.write32(0x0400_00C0, 0x0400_00A0);
        bus.write16(0x0400_00C4, 1);
        bus.write16(0x0400_00C6, 0xB640);
        bus.write16(0x0400_0100, 0xFFFF);
        bus.write16(0x0400_0102, 0x0080);
        bus.advance_cycles(1);

        assert_eq!(bus.audio_fifo_len(0), Some(31));
        assert_eq!(bus.audio_fifo_last_sample(0), Some(0x83_u8 as i8));
        assert_eq!(bus.read32(0x0400_00BC), 0x0200_0010);
        assert_eq!(bus.read16(0x0400_00C6) & 0x8000, 0x8000);
    }

    #[test]
    fn sound_fifo_dma_requests_the_matching_channel_interrupt() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write16(0x0400_0084, 0x0080);
        bus.write16(0x0400_0082, 1 << 8);
        for i in 0..4_u32 {
            bus.write32(0x0200_0200 + i * 4, 0x1111_1111 * (i + 1));
            bus.write32(0x0400_00A0, 0x0101_0101 * (i + 1));
        }
        bus.write32(0x0400_00BC, 0x0200_0200);
        bus.write32(0x0400_00C0, 0x0400_00A0);
        bus.write16(0x0400_00C4, 1);
        bus.write16(0x0400_00C6, 0xF640); // DMA1 FIFO mode with IRQ enabled.
        bus.write16(0x0400_0100, 0xFFFF);
        bus.write16(0x0400_0102, 0x0080);
        bus.advance_cycles(1);

        assert_eq!(bus.read16(0x0400_0202) & (1 << 9), 1 << 9);
        assert_eq!(bus.read32(0x0400_00BC), 0x0200_0210);
    }

    #[test]
    fn sound_fifo_dma_rejects_incompatible_channel_configuration() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write16(0x0400_0084, 0x0080);
        bus.write16(0x0400_0082, 1 << 8);
        bus.write32(0x0200_0000, 0x4433_2211);
        bus.write32(0x0400_00A0, 0x8877_6655);

        // DMA1 has the FIFO destination and special timing, but no word/repeat
        // configuration. It must not perform a forced four-word FIFO burst.
        bus.write32(0x0400_00BC, 0x0200_0000);
        bus.write32(0x0400_00C0, 0x0400_00A0);
        bus.write16(0x0400_00C4, 1);
        bus.write16(0x0400_00C6, 0xB000);
        bus.write16(0x0400_0100, 0xFFFF);
        bus.write16(0x0400_0102, 0x0080);
        bus.advance_cycles(1);

        assert_eq!(bus.audio_fifo_len(0), Some(3));
        assert_eq!(bus.audio_fifo_last_sample(0), Some(0x55));
        assert_eq!(bus.read32(0x0400_00BC), 0x0200_0000);
        assert_eq!(bus.read16(0x0400_00C6) & 0x8000, 0x8000);
    }

    #[test]
    fn sound_fifo_b_uses_timer_one_and_dma_channel_two() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write16(0x0400_0084, 0x0080);
        bus.write16(0x0400_0082, (1 << 12) | (1 << 14)); // FIFO B, timer 1.
        for i in 0..4_u32 {
            bus.write32(0x0200_0100 + i * 4, 0xA3A2_A1A0 + i * 0x0404_0404);
            bus.write32(0x0400_00A4, 0xA3A2_A1A0 + i * 0x0404_0404);
        }

        // DMA2: EWRAM -> FIFO B, fixed destination, word-sized FIFO burst.
        bus.write32(0x0400_00C8, 0x0200_0100);
        bus.write32(0x0400_00CC, 0x0400_00A4);
        bus.write16(0x0400_00D0, 1);
        bus.write16(0x0400_00D2, 0xB640);
        bus.write16(0x0400_0104, 0xFFFF);
        bus.write16(0x0400_0106, 0x0080);
        bus.advance_cycles(1);

        assert_eq!(bus.audio_fifo_len(1), Some(31));
        assert_eq!(bus.audio_fifo_last_sample(1), Some(0xA0_u8 as i8));
        assert_eq!(bus.read32(0x0400_00C8), 0x0200_0110);
        assert_eq!(bus.read16(0x0400_00D2) & 0x8000, 0x8000);
    }

    #[test]
    fn vram_mirror_and_sram_address_window_are_consistent() {
        let mut bus = MemoryBus::new(cartridge());
        bus.write8(0x0601_8000, 0x5A);
        assert_eq!(bus.read8(0x0601_0000), 0x5A);

        // Only 0x0E00_0000..=0x0E00_FFFF is mapped to save SRAM.
        bus.write8(0x0E00_1234, 0xA5);
        assert_eq!(bus.read8(0x0E00_1234), 0xA5);
        assert_eq!(bus.read8(0x0E01_1234), 0);
        assert_eq!(bus.read8(0x0F00_1234), 0);

        // Writes outside the SRAM window must not alias into the save.
        bus.write8(0x0F00_1234, 0x3C);
        assert_eq!(bus.read8(0x0E00_1234), 0xA5);
        assert_eq!(bus.read8(0x0F00_1234), 0);
    }
}
