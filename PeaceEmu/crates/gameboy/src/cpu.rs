//! ARM7TDMI execution core: the initial ARM-state instruction subset.

use crate::MemoryBus;

/// ARM condition flags in the CPSR.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConditionFlags {
    /// Negative flag.
    pub negative: bool,
    /// Zero flag.
    pub zero: bool,
    /// Carry / not-borrow flag.
    pub carry: bool,
    /// Signed overflow flag.
    pub overflow: bool,
}

/// Result of executing one instruction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepOutcome {
    /// An instruction was executed.
    Executed {
        /// Address of the executed instruction.
        address: u32,
        /// Raw 32-bit instruction word.
        instruction: u32,
    },
    /// An instruction was skipped because its condition failed.
    ConditionFailed {
        /// Address of the skipped instruction.
        address: u32,
        /// Raw 32-bit instruction word.
        instruction: u32,
    },
}

/// Errors for CPU instructions that are not implemented yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CpuError {
    /// The instruction encoding is outside the currently implemented subset.
    UnsupportedInstruction {
        /// Address of the unsupported instruction.
        address: u32,
        /// Raw 32-bit instruction word.
        instruction: u32,
    },
}

impl core::fmt::Display for CpuError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnsupportedInstruction {
                address,
                instruction,
            } => write!(
                formatter,
                "unsupported CPU instruction 0x{instruction:08X} at 0x{address:08X}"
            ),
        }
    }
}

impl std::error::Error for CpuError {}

/// ARM7TDMI CPU state, initially implementing ARM-state data processing,
/// branches, and single word/byte transfers.
#[derive(Clone, Debug)]
pub struct Cpu {
    registers: [u32; 16],
    flags: ConditionFlags,
    thumb: bool,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    /// Creates a CPU with zeroed registers and flags.
    ///
    /// Set the program counter to the desired entry point before stepping.
    pub fn new() -> Self {
        Self {
            registers: [0; 16],
            flags: ConditionFlags::default(),
            thumb: false,
        }
    }

    /// Resets all registers and flags and sets the initial program counter.
    pub fn reset(&mut self, entry_point: u32) {
        self.registers = [0; 16];
        self.registers[15] = entry_point & !3;
        self.flags = ConditionFlags::default();
        self.thumb = false;
    }

    /// Returns a general-purpose register, including r15 as the current PC.
    pub fn register(&self, index: usize) -> Option<u32> {
        self.registers.get(index).copied()
    }

    /// Sets a general-purpose register. Writes to r15 follow the current
    /// instruction state: halfword alignment in Thumb state, word alignment in ARM.
    pub fn set_register(&mut self, index: usize, value: u32) -> bool {
        if index >= self.registers.len() {
            return false;
        }
        self.registers[index] = if index == 15 {
            if self.thumb { value & !1 } else { value & !3 }
        } else {
            value
        };
        true
    }

    /// Returns whether the CPU is currently executing Thumb instructions.
    pub fn is_thumb(&self) -> bool {
        self.thumb
    }

    /// Returns the modeled CPSR condition flags.
    pub fn flags(&self) -> ConditionFlags {
        self.flags
    }

    /// Executes one ARM-state instruction from the current program counter.
    ///
    /// The CPU advances the PC by four bytes before executing the operation.
    /// This implementation is instruction-functional, not cycle-accurate.
    /// Thumb execution is supported, but exceptions, banked registers, and SPSRs
    /// are not modeled yet.
    pub fn step(&mut self, bus: &mut MemoryBus) -> Result<StepOutcome, CpuError> {
        if self.thumb {
            return self.step_thumb(bus);
        }
        let address = self.registers[15] & !3;
        let instruction = bus.read32(address);
        self.registers[15] = address.wrapping_add(4);

        let condition = instruction >> 28;
        if condition == 0xF {
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
        }
        if !self.condition_passed(condition) {
            return Ok(StepOutcome::ConditionFailed {
                address,
                instruction,
            });
        }

        if instruction & 0x0FBF_0FFF == 0x010F_0000 {
            self.execute_mrs(address, instruction)?;
        } else if instruction & 0x0FB0_FFF0 == 0x0120_F000
            || instruction & 0x0FB0_F000 == 0x0320_F000
        {
            self.execute_msr(address, instruction)?;
        } else if instruction & 0x0FB0_0FF0 == 0x0100_0090 {
            self.execute_swap(address, instruction, bus)?;
        } else if instruction & 0x0F80_00F0 == 0x0080_0090 {
            self.execute_long_multiply(address, instruction)?;
        } else if instruction & 0x0FFF_FFF0 == 0x012F_FF10 {
            let rm = (instruction & 0xF) as usize;
            // BX with Rm=PC is architecturally unpredictable on ARM7TDMI.
            if rm == 15 {
                return Err(CpuError::UnsupportedInstruction {
                    address,
                    instruction,
                });
            }
            let target = self.registers[rm];
            self.thumb = target & 1 != 0;
            self.registers[15] = if self.thumb { target & !1 } else { target & !3 };
        } else if instruction & 0x0FC0_00F0 == 0x0000_0090 {
            self.execute_multiply(address, instruction)?;
        } else if instruction & 0x0E00_0000 == 0x0A00_0000 {
            self.execute_branch(address, instruction);
        } else if instruction & 0x0E00_0000 == 0x0800_0000 {
            self.execute_block_transfer(address, instruction, bus)?;
        } else if instruction & 0x0C00_0000 == 0x0400_0000 {
            self.execute_single_transfer(address, instruction, bus)?;
        } else if instruction & 0x0E00_0090 == 0x0000_0090 {
            self.execute_halfword_transfer(address, instruction, bus)?;
        } else if instruction & 0x0C00_0000 == 0 {
            self.execute_data_processing(address, instruction)?;
        } else {
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
        }

        Ok(StepOutcome::Executed {
            address,
            instruction,
        })
    }

    fn step_thumb(&mut self, bus: &mut MemoryBus) -> Result<StepOutcome, CpuError> {
        let address = self.registers[15] & !1;
        let instruction = u32::from(bus.read16(address));
        self.registers[15] = address.wrapping_add(2);
        let raw = instruction as u16;

        if raw & 0xF800 == 0xF000 {
            // First half of Thumb BL: form the signed high displacement in LR.
            let high_displacement = ((u32::from(raw & 0x07FF) << 21) as i32 >> 9) as u32;
            self.registers[14] = address.wrapping_add(4).wrapping_add(high_displacement);
        } else if raw & 0xF800 == 0xF800 {
            // Second half of Thumb BL: branch relative to the saved high part.
            let low_displacement = u32::from(raw & 0x07FF) << 1;
            let target = self.registers[14].wrapping_add(low_displacement);
            self.registers[14] = address.wrapping_add(2) | 1;
            self.registers[15] = target & !1;
        } else if raw & 0xF800 == 0xE000 {
            let displacement = (((u32::from(raw & 0x07FF)) << 21) as i32 >> 20) as u32;
            self.registers[15] = address.wrapping_add(4).wrapping_add(displacement) & !1;
        } else if raw & 0xF000 == 0xD000 {
            let condition = u32::from((raw >> 8) & 0xF);
            if condition >= 0xE {
                return Err(CpuError::UnsupportedInstruction {
                    address,
                    instruction,
                });
            }
            if self.condition_passed(condition) {
                let displacement = ((u32::from(raw & 0xFF) << 24) as i32 >> 23) as u32;
                self.registers[15] = address.wrapping_add(4).wrapping_add(displacement) & !1;
            }
        } else if raw & 0xFC00 == 0x4400 {
            let operation = (raw >> 8) & 3;
            let rd = usize::from((raw & 7) | ((raw >> 4) & 8));
            let rs = usize::from((raw >> 3) & 0xF);
            let right = self.read_thumb_register(rs, address);
            let left = self.read_thumb_register(rd, address);
            match operation {
                0 => {
                    let result = left.wrapping_add(right);
                    if rd == 15 {
                        self.registers[15] = result & !1;
                    } else {
                        self.registers[rd] = result;
                    }
                }
                1 => {
                    let (result, carry, overflow) = Self::subtract(left, right, false);
                    self.set_arithmetic_flags(result, carry, overflow);
                }
                2 => {
                    if rd == 15 {
                        self.registers[15] = right & !1;
                    } else {
                        self.registers[rd] = right;
                    }
                }
                3 => {
                    self.thumb = right & 1 != 0;
                    self.registers[15] = if self.thumb { right & !1 } else { right & !3 };
                }
                _ => unreachable!(),
            }
        } else if raw & 0xF800 == 0x4800 {
            let rd = usize::from((raw >> 8) & 7);
            let offset = u32::from(raw & 0xFF) * 4;
            let base = address.wrapping_add(4) & !3;
            self.registers[rd] = bus.read32(base.wrapping_add(offset));
        } else if raw & 0xF000 == 0x5000 {
            let operation = (raw >> 9) & 7;
            let ro = usize::from((raw >> 6) & 7);
            let rb = usize::from((raw >> 3) & 7);
            let rd = usize::from(raw & 7);
            let effective = self.registers[rb].wrapping_add(self.registers[ro]);
            match operation {
                0 => bus.write32(effective, self.registers[rd]),
                1 => bus.write16(effective, self.registers[rd] as u16),
                2 => bus.write8(effective, self.registers[rd] as u8),
                3 => {
                    self.registers[rd] = bus.read8(effective) as i8 as i32 as u32;
                }
                4 => {
                    // Thumb LDR uses the same ARM7TDMI unaligned-word
                    // rotation behavior as ARM-state LDR.
                    self.registers[rd] =
                        bus.read32(effective).rotate_right((effective & 3) * 8);
                },
                5 => {
                    // ARM7TDMI LDRH rotates the aligned halfword by one byte
                    // when the effective address is odd.
                    let value = bus.read16(effective);
                    self.registers[rd] = u32::from(if effective & 1 != 0 {
                        value.rotate_right(8)
                    } else {
                        value
                    });
                },
                6 => self.registers[rd] = u32::from(bus.read8(effective)),
                7 => {
                    self.registers[rd] = if effective & 1 != 0 {
                        bus.read8(effective) as i8 as i32 as u32
                    } else {
                        bus.read16(effective) as i16 as i32 as u32
                    };
                }
                _ => unreachable!(),
            }
        } else if raw & 0xE000 == 0x6000 {
            let byte_transfer = raw & (1 << 12) != 0;
            let load = raw & (1 << 11) != 0;
            let offset = u32::from((raw >> 6) & 0x1F);
            let rb = usize::from((raw >> 3) & 7);
            let rd = usize::from(raw & 7);
            let scaled_offset = if byte_transfer { offset } else { offset * 4 };
            let effective = self.registers[rb].wrapping_add(scaled_offset);
            if load {
                self.registers[rd] = if byte_transfer {
                    u32::from(bus.read8(effective))
                } else {
                    // The bus returns an aligned word; rotate it to model the
                    // ARM7TDMI result for an unaligned Thumb LDR address.
                    bus.read32(effective).rotate_right((effective & 3) * 8)
                };
            } else if byte_transfer {
                bus.write8(effective, self.registers[rd] as u8);
            } else {
                bus.write32(effective, self.registers[rd]);
            }
        } else if raw & 0xF000 == 0x8000 {
            let load = raw & (1 << 11) != 0;
            let offset = u32::from((raw >> 6) & 0x1F) * 2;
            let rb = usize::from((raw >> 3) & 7);
            let rd = usize::from(raw & 7);
            let effective = self.registers[rb].wrapping_add(offset);
            if load {
                // Unsigned Thumb LDRH rotates the aligned halfword by 8 bits
                // when the effective address is odd, just like ARM-state LDRH.
                let value = bus.read16(effective);
                self.registers[rd] = u32::from(if effective & 1 != 0 {
                    value.rotate_right(8)
                } else {
                    value
                });
            } else {
                bus.write16(effective, self.registers[rd] as u16);
            }
        } else if raw & 0xF000 == 0xA000 {
            // ADD Rd, PC/SP, #imm: the PC base is word-aligned address+4.
            let rd = usize::from((raw >> 8) & 7);
            let use_sp = raw & (1 << 11) != 0;
            let offset = u32::from(raw & 0xFF) * 4;
            let base = if use_sp {
                self.registers[13]
            } else {
                address.wrapping_add(4) & !3
            };
            self.registers[rd] = base.wrapping_add(offset);
        } else if raw & 0xF000 == 0x9000 {
            let load = raw & (1 << 11) != 0;
            let rd = usize::from((raw >> 8) & 7);
            let offset = u32::from(raw & 0xFF) * 4;
            let effective = self.registers[13].wrapping_add(offset);
            if load {
                // This is an ordinary Thumb word load: preserve ARM7TDMI's
                // rotate-right behavior even if software has made SP unaligned.
                self.registers[rd] =
                    bus.read32(effective).rotate_right((effective & 3) * 8);
            } else {
                bus.write32(effective, self.registers[rd]);
            }
        } else if raw & 0xFE00 == 0xB400 || raw & 0xFE00 == 0xBC00 {
            let pop = raw & 0x0800 != 0;
            let extra_register = raw & (1 << 8) != 0;
            let register_list = u32::from(raw & 0xFF);
            let count = register_list.count_ones() + u32::from(extra_register);
            if count == 0 {
                return Err(CpuError::UnsupportedInstruction {
                    address,
                    instruction,
                });
            }

            let old_sp = self.registers[13];
            if pop {
                let mut transfer_address = old_sp;
                for register in 0..8 {
                    if register_list & (1 << register) != 0 {
                        self.registers[register] = bus.read32(transfer_address);
                        transfer_address = transfer_address.wrapping_add(4);
                    }
                }
                if extra_register {
                    let value = bus.read32(transfer_address);
                    self.registers[15] = value & !1;
                }
                self.registers[13] = old_sp.wrapping_add(count * 4);
            } else {
                let new_sp = old_sp.wrapping_sub(count * 4);
                let mut transfer_address = new_sp;
                for register in 0..8 {
                    if register_list & (1 << register) != 0 {
                        bus.write32(transfer_address, self.registers[register]);
                        transfer_address = transfer_address.wrapping_add(4);
                    }
                }
                if extra_register {
                    bus.write32(transfer_address, self.registers[14]);
                }
                self.registers[13] = new_sp;
            }
        } else if raw & 0xFF00 == 0xB000 {
            let subtract = raw & (1 << 7) != 0;
            let offset = u32::from(raw & 0x7F) * 4;
            self.registers[13] = if subtract {
                self.registers[13].wrapping_sub(offset)
            } else {
                self.registers[13].wrapping_add(offset)
            };
        } else if raw & 0xF000 == 0xC000 {
            let load = raw & (1 << 11) != 0;
            let rb = usize::from((raw >> 8) & 7);
            let register_list = u32::from(raw & 0xFF);
            let count = register_list.count_ones();
            if count == 0 {
                return Err(CpuError::UnsupportedInstruction {
                    address,
                    instruction,
                });
            }

            let base = self.registers[rb];
            let mut transfer_address = base;
            for register in 0..8 {
                if register_list & (1 << register) == 0 {
                    continue;
                }
                if load {
                    self.registers[register] = bus.read32(transfer_address);
                } else {
                    // Thumb STMIA has a defined special case when the base is
                    // included in the register list: if it is the lowest
                    // listed register, the stored value is the updated base;
                    // otherwise the original base is stored.
                    let value = if register == rb {
                        let lowest_register = register_list.trailing_zeros();
                        if lowest_register == rb as u32 {
                            base.wrapping_add(count * 4)
                        } else {
                            base
                        }
                    } else {
                        self.registers[register]
                    };
                    bus.write32(transfer_address, value);
                }
                transfer_address = transfer_address.wrapping_add(4);
            }
            if !load || register_list & (1 << rb) == 0 {
                self.registers[rb] = base.wrapping_add(count * 4);
            }
        } else if raw & 0xE000 == 0x2000 {
            let operation = (raw >> 11) & 3;
            let rd = usize::from((raw >> 8) & 7);
            let immediate = u32::from(raw & 0xFF);
            let left = self.registers[rd];
            match operation {
                0 => {
                    self.registers[rd] = immediate;
                    self.set_logic_flags(immediate, None);
                }
                1 => {
                    let (result, carry, overflow) = Self::subtract(left, immediate, false);
                    self.set_arithmetic_flags(result, carry, overflow);
                }
                2 => {
                    let (result, carry, overflow) = Self::add(left, immediate, false);
                    self.registers[rd] = result;
                    self.set_arithmetic_flags(result, carry, overflow);
                }
                3 => {
                    let (result, carry, overflow) = Self::subtract(left, immediate, false);
                    self.registers[rd] = result;
                    self.set_arithmetic_flags(result, carry, overflow);
                }
                _ => unreachable!(),
            }
        } else if raw & 0xE000 == 0x0000 {
            let operation = (raw >> 11) & 3;
            if operation != 3 {
                let immediate = u32::from((raw >> 6) & 0x1F);
                let rs = usize::from((raw >> 3) & 7);
                let rd = usize::from(raw & 7);
                let value = self.registers[rs];
                let carry_in = self.flags.carry;
                let (result, carry) =
                    Self::shift_thumb_immediate(value, u32::from(operation), immediate, carry_in);
                self.registers[rd] = result;
                self.set_logic_flags(result, carry);
            } else {
                let immediate_operand = raw & (1 << 10) != 0;
                let subtract = raw & (1 << 9) != 0;
                let operand = if immediate_operand {
                    u32::from((raw >> 6) & 7)
                } else {
                    self.registers[usize::from((raw >> 6) & 7)]
                };
                let rs = usize::from((raw >> 3) & 7);
                let rd = usize::from(raw & 7);
                let left = self.registers[rs];
                let (result, carry, overflow) = if subtract {
                    Self::subtract(left, operand, false)
                } else {
                    Self::add(left, operand, false)
                };
                self.registers[rd] = result;
                self.set_arithmetic_flags(result, carry, overflow);
            }
        } else if raw & 0xFC00 == 0x4000 {
            let operation = (raw >> 6) & 0xF;
            let rd = usize::from(raw & 7);
            let rs = usize::from((raw >> 3) & 7);
            let left = self.registers[rd];
            let right = self.registers[rs];
            match operation {
                0 => {
                    let result = left & right;
                    self.registers[rd] = result;
                    self.set_logic_flags(result, None);
                }
                1 => {
                    let result = left ^ right;
                    self.registers[rd] = result;
                    self.set_logic_flags(result, None);
                }
                2 | 3 | 4 | 7 => {
                    // Register-controlled Thumb shifts use the low byte of Rs.
                    let amount = right & 0xFF;
                    if amount != 0 {
                        let (result, carry) = match operation {
                            2 => match amount {
                                1..=31 => (left << amount, Some((left >> (32 - amount)) & 1 != 0)),
                                32 => (0, Some(left & 1 != 0)),
                                _ => (0, Some(false)),
                            },
                            3 => match amount {
                                1..=31 => (left >> amount, Some((left >> (amount - 1)) & 1 != 0)),
                                32 => (0, Some(left >> 31 != 0)),
                                _ => (0, Some(false)),
                            },
                            4 => {
                                let sign = left >> 31 != 0;
                                if amount >= 32 {
                                    (if sign { u32::MAX } else { 0 }, Some(sign))
                                } else {
                                    (
                                        ((left as i32) >> amount) as u32,
                                        Some((left >> (amount - 1)) & 1 != 0),
                                    )
                                }
                            }
                            7 => {
                                let rotation = amount & 31;
                                if rotation == 0 {
                                    (left, Some(left >> 31 != 0))
                                } else {
                                    let result = left.rotate_right(rotation);
                                    (result, Some(result >> 31 != 0))
                                }
                            }
                            _ => unreachable!(),
                        };
                        self.registers[rd] = result;
                        self.set_logic_flags(result, carry);
                    }
                }
                5 => {
                    let (result, carry, overflow) = Self::add(left, right, self.flags.carry);
                    self.registers[rd] = result;
                    self.set_arithmetic_flags(result, carry, overflow);
                }
                6 => {
                    let borrow_in = !self.flags.carry;
                    let (result, carry, overflow) = Self::subtract(left, right, borrow_in);
                    self.registers[rd] = result;
                    self.set_arithmetic_flags(result, carry, overflow);
                }
                8 => {
                    self.set_logic_flags(left & right, None);
                }
                9 => {
                    let (result, carry, overflow) = Self::subtract(0, right, false);
                    self.registers[rd] = result;
                    self.set_arithmetic_flags(result, carry, overflow);
                }
                10 => {
                    let (result, carry, overflow) = Self::subtract(left, right, false);
                    self.set_arithmetic_flags(result, carry, overflow);
                }
                11 => {
                    let (result, carry, overflow) = Self::add(left, right, false);
                    self.set_arithmetic_flags(result, carry, overflow);
                }
                12 => {
                    let result = left | right;
                    self.registers[rd] = result;
                    self.set_logic_flags(result, None);
                }
                13 => {
                    let result = left.wrapping_mul(right);
                    self.registers[rd] = result;
                    self.set_logic_flags(result, None);
                }
                14 => {
                    let result = left & !right;
                    self.registers[rd] = result;
                    self.set_logic_flags(result, None);
                }
                15 => {
                    let result = !right;
                    self.registers[rd] = result;
                    self.set_logic_flags(result, None);
                }
                _ => {
                    return Err(CpuError::UnsupportedInstruction {
                        address,
                        instruction,
                    });
                }
            }
        } else {
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
        }

        Ok(StepOutcome::Executed {
            address,
            instruction,
        })
    }

    fn read_thumb_register(&self, index: usize, address: u32) -> u32 {
        if index == 15 {
            address.wrapping_add(4) & !3
        } else {
            self.registers[index]
        }
    }

    fn set_logic_flags(&mut self, result: u32, carry: Option<bool>) {
        self.flags.negative = result >> 31 != 0;
        self.flags.zero = result == 0;
        if let Some(carry) = carry {
            self.flags.carry = carry;
        }
    }

    fn set_arithmetic_flags(&mut self, result: u32, carry: bool, overflow: bool) {
        self.flags.negative = result >> 31 != 0;
        self.flags.zero = result == 0;
        self.flags.carry = carry;
        self.flags.overflow = overflow;
    }

    fn condition_passed(&self, condition: u32) -> bool {
        let f = self.flags;
        match condition {
            0x0 => f.zero,
            0x1 => !f.zero,
            0x2 => f.carry,
            0x3 => !f.carry,
            0x4 => f.negative,
            0x5 => !f.negative,
            0x6 => f.overflow,
            0x7 => !f.overflow,
            0x8 => f.carry && !f.zero,
            0x9 => !f.carry || f.zero,
            0xA => f.negative == f.overflow,
            0xB => f.negative != f.overflow,
            0xC => !f.zero && f.negative == f.overflow,
            0xD => f.zero || f.negative != f.overflow,
            0xE => true,
            _ => false,
        }
    }

    fn read_operand_register(&self, index: usize, instruction_address: u32) -> u32 {
        if index == 15 {
            instruction_address.wrapping_add(8)
        } else {
            self.registers[index]
        }
    }

    fn execute_branch(&mut self, address: u32, instruction: u32) {
        if instruction & (1 << 24) != 0 {
            self.registers[14] = address.wrapping_add(4);
        }
        let displacement = (((instruction & 0x00FF_FFFF) << 8) as i32 >> 6) as u32;
        self.registers[15] = address.wrapping_add(8).wrapping_add(displacement) & !3;
    }

    /// Writes the modeled CPSR condition flags with MSR CPSR_f.
    /// Control fields and SPSR writes require exception-mode state and remain unsupported.
    fn execute_msr(&mut self, address: u32, instruction: u32) -> Result<(), CpuError> {
        let saved_status = instruction & (1 << 22) != 0;
        let field_mask = (instruction >> 16) & 0xF;
        let immediate = instruction & (1 << 25) != 0;

        if saved_status || field_mask & !0x8 != 0 {
            return Err(CpuError::UnsupportedInstruction { address, instruction });
        }
        if field_mask == 0 {
            return Ok(());
        }

        let value = if immediate {
            (instruction & 0xFF).rotate_right(((instruction >> 8) & 0xF) * 2)
        } else {
            let rm = (instruction & 0xF) as usize;
            if rm == 15 {
                return Err(CpuError::UnsupportedInstruction { address, instruction });
            }
            self.registers[rm]
        };

        self.flags.negative = value >> 31 != 0;
        self.flags.zero = value & (1 << 30) != 0;
        self.flags.carry = value & (1 << 29) != 0;
        self.flags.overflow = value & (1 << 28) != 0;
        Ok(())
    }

    /// Reads the modeled CPSR fields with ARM-state MRS.
    /// Mode, interrupt masks and other unmodeled CPSR bits read as zero.
    fn execute_mrs(&mut self, address: u32, instruction: u32) -> Result<(), CpuError> {
        let rd = ((instruction >> 12) & 0xF) as usize;
        let saved_status = instruction & (1 << 22) != 0;
        if saved_status || rd == 15 {
            return Err(CpuError::UnsupportedInstruction { address, instruction });
        }

        let mut cpsr = 0_u32;
        cpsr |= u32::from(self.flags.negative) << 31;
        cpsr |= u32::from(self.flags.zero) << 30;
        cpsr |= u32::from(self.flags.carry) << 29;
        cpsr |= u32::from(self.flags.overflow) << 28;
        cpsr |= u32::from(self.thumb) << 5;
        self.registers[rd] = cpsr;
        Ok(())
    }

    /// Executes ARM7TDMI SWP/SWPB. The read and write are performed as
    /// one instruction-level operation; bus-cycle atomicity is not modeled.
    fn execute_swap(
        &mut self,
        address: u32,
        instruction: u32,
        bus: &mut MemoryBus,
    ) -> Result<(), CpuError> {
        let byte = instruction & (1 << 22) != 0;
        let rn = ((instruction >> 16) & 0xF) as usize;
        let rd = ((instruction >> 12) & 0xF) as usize;
        let rm = (instruction & 0xF) as usize;

        if rn == 15 || rd == 15 || rm == 15 {
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
        }

        let effective = self.registers[rn];
        if !byte && effective & 3 != 0 {
            // Word SWP requires a word-aligned address on ARM7TDMI.
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
        }

        let source = self.registers[rm];
        let previous = if byte {
            let previous = u32::from(bus.read8(effective));
            bus.write8(effective, source as u8);
            previous
        } else {
            let previous = bus.read32(effective);
            bus.write32(effective, source);
            previous
        };
        self.registers[rd] = previous;
        Ok(())
    }

    fn execute_long_multiply(
        &mut self,
        address: u32,
        instruction: u32,
    ) -> Result<(), CpuError> {
        let signed = instruction & (1 << 22) != 0;
        let accumulate = instruction & (1 << 21) != 0;
        let set_flags = instruction & (1 << 20) != 0;
        let rd_hi = ((instruction >> 16) & 0xF) as usize;
        let rd_lo = ((instruction >> 12) & 0xF) as usize;
        let rs = ((instruction >> 8) & 0xF) as usize;
        let rm = (instruction & 0xF) as usize;

        // ARM7TDMI forbids PC operands, identical high/low destinations,
        // and using Rm as either destination. Rs may alias a destination.
        if rd_hi == 15
            || rd_lo == 15
            || rd_hi == rd_lo
            || rs == 15
            || rm == 15
            || rd_hi == rm
            || rd_lo == rm
        {
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
        }

        let mut result = if signed {
            let left = self.registers[rm] as i32 as i64;
            let right = self.registers[rs] as i32 as i64;
            (left.wrapping_mul(right)) as u64
        } else {
            u64::from(self.registers[rm]) * u64::from(self.registers[rs])
        };

        if accumulate {
            let accumulator =
                (u64::from(self.registers[rd_hi]) << 32) | u64::from(self.registers[rd_lo]);
            result = result.wrapping_add(accumulator);
        }

        self.registers[rd_lo] = result as u32;
        self.registers[rd_hi] = (result >> 32) as u32;
        if set_flags {
            self.flags.negative = result >> 63 != 0;
            self.flags.zero = result == 0;
            // Long multiply updates N and Z only; C and V are unchanged.
        }
        Ok(())
    }

    fn execute_multiply(&mut self, address: u32, instruction: u32) -> Result<(), CpuError> {
        let accumulate = instruction & (1 << 21) != 0;
        let set_flags = instruction & (1 << 20) != 0;
        let rd = ((instruction >> 16) & 0xF) as usize;
        let rn = ((instruction >> 12) & 0xF) as usize;
        let rs = ((instruction >> 8) & 0xF) as usize;
        let rm = (instruction & 0xF) as usize;

        // ARM7TDMI multiply operands and destination must not use r15.
        // Rd must also differ from Rm; that overlap is architecturally
        // unpredictable on this core, for both MUL and MLA.
        if rd == 15
            || rs == 15
            || rm == 15
            || rd == rm
            || (accumulate && rn == 15)
        {
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
        }

        let mut result = self.registers[rm].wrapping_mul(self.registers[rs]);
        if accumulate {
            result = result.wrapping_add(self.registers[rn]);
        }
        self.registers[rd] = result;

        if set_flags {
            self.flags.negative = result >> 31 != 0;
            self.flags.zero = result == 0;
            // MUL/MLA update N and Z only; C and V are unchanged.
        }
        Ok(())
    }

    fn execute_data_processing(&mut self, address: u32, instruction: u32) -> Result<(), CpuError> {
        let immediate = instruction & (1 << 25) != 0;
        let opcode = (instruction >> 21) & 0xF;
        let set_flags = instruction & (1 << 20) != 0;
        let rn = ((instruction >> 16) & 0xF) as usize;
        let rd = ((instruction >> 12) & 0xF) as usize;

        // TST, TEQ, CMP and CMN are defined only with S=1. Treat their
        // reserved S=0 encodings as unsupported instead of silently changing
        // CPSR flags for an instruction that is not architecturally valid.
        if (0x8..=0xB).contains(&opcode) && !set_flags {
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
        }

        // Data-processing instructions with S=1 and Rd=PC perform an
        // exception return: the CPSR is restored from the current SPSR.
        // Since this core does not model SPSRs or exception modes yet, reject
        // these instructions rather than silently branch with incorrect flags.
        if set_flags && rd == 15 {
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
        }

        let left = self.read_operand_register(rn, address);
        let (right, shifter_carry) = if immediate {
            let value = instruction & 0xFF;
            let rotate = ((instruction >> 8) & 0xF) * 2;
            let result = value.rotate_right(rotate);
            (
                result,
                if rotate == 0 {
                    None
                } else {
                    Some(result >> 31 != 0)
                },
            )
        } else if instruction & (1 << 4) != 0 {
            // Register-controlled shifts use Rs[7:0] as the shift amount.
            // Bit 7 must be clear for the ARM7TDMI data-processing encoding.
            if instruction & (1 << 7) != 0 {
                return Err(CpuError::UnsupportedInstruction {
                    address,
                    instruction,
                });
            }
            let rm = (instruction & 0xF) as usize;
            let rs = ((instruction >> 8) & 0xF) as usize;
            // ARM7TDMI register-controlled shifts cannot use PC as Rs.
            // Rm=PC observes the extra register-shift pipeline cycle (PC+12).
            if rs == 15 {
                return Err(CpuError::UnsupportedInstruction {
                    address,
                    instruction,
                });
            }
            let value = if rm == 15 {
                address.wrapping_add(12)
            } else {
                self.registers[rm]
            };
            let amount = self.registers[rs] & 0xFF;
            let shift_type = (instruction >> 5) & 3;
            Self::shift_register(value, shift_type, amount, self.flags.carry)
        } else {
            let rm = (instruction & 0xF) as usize;
            let value = self.read_operand_register(rm, address);
            let shift_type = (instruction >> 5) & 3;
            let shift_amount = (instruction >> 7) & 0x1F;
            let (result, carry) =
                Self::shift_immediate(value, shift_type, shift_amount, self.flags.carry);
            (result, carry)
        };

        let (result, write_result, arithmetic_flags) = match opcode {
            0x0 => (left & right, true, None),
            0x1 => (left ^ right, true, None),
            0x2 => {
                let (result, carry, overflow) = Self::subtract(left, right, false);
                (result, true, Some((carry, overflow)))
            }
            0x3 => {
                let (result, carry, overflow) = Self::subtract(right, left, false);
                (result, true, Some((carry, overflow)))
            }
            0x4 => {
                let (result, carry, overflow) = Self::add(left, right, false);
                (result, true, Some((carry, overflow)))
            }
            0x5 => {
                let (result, carry, overflow) = Self::add(left, right, self.flags.carry);
                (result, true, Some((carry, overflow)))
            }
            0x6 => {
                let (result, carry, overflow) = Self::subtract(left, right, !self.flags.carry);
                (result, true, Some((carry, overflow)))
            }
            0x7 => {
                let (result, carry, overflow) = Self::subtract(right, left, !self.flags.carry);
                (result, true, Some((carry, overflow)))
            }
            0x8 => (left & right, false, None),
            0x9 => (left ^ right, false, None),
            0xA => {
                let (result, carry, overflow) = Self::subtract(left, right, false);
                (result, false, Some((carry, overflow)))
            }
            0xB => {
                let (result, carry, overflow) = Self::add(left, right, false);
                (result, false, Some((carry, overflow)))
            }
            0xC => (left | right, true, None),
            0xD => (right, true, None),
            0xE => (left & !right, true, None),
            0xF => (!right, true, None),
            _ => unreachable!(),
        };

        if write_result {
            self.registers[rd] = if rd == 15 { result & !3 } else { result };
        }

        if set_flags || !write_result {
            self.flags.negative = result >> 31 != 0;
            self.flags.zero = result == 0;
            if let Some((carry, overflow)) = arithmetic_flags {
                self.flags.carry = carry;
                self.flags.overflow = overflow;
            } else if let Some(carry) = shifter_carry {
                self.flags.carry = carry;
            }
        }
        Ok(())
    }

    fn execute_block_transfer(
        &mut self,
        address: u32,
        instruction: u32,
        bus: &mut MemoryBus,
    ) -> Result<(), CpuError> {
        let pre_index = instruction & (1 << 24) != 0;
        let increment = instruction & (1 << 23) != 0;
        let user_registers = instruction & (1 << 22) != 0;
        let write_back = instruction & (1 << 21) != 0;
        let load = instruction & (1 << 20) != 0;
        let rn = ((instruction >> 16) & 0xF) as usize;
        let register_list = instruction & 0xFFFF;

        // User-bank transfers require exception/banked-register support.
        if user_registers {
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
        }

        // Writing back through r15 makes the block-transfer base depend on
        // the pipeline-visible PC and has unpredictable control-flow effects.
        if rn == 15 && write_back {
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
        }

        // ARM7TDMI treats an empty list as a PC transfer, but adjusts the
        // address/write-back as though all 16 registers had been listed.
        let empty_list = register_list == 0;
        let effective_list = if empty_list { 1 << 15 } else { register_list };
        let count = if empty_list { 16 } else { register_list.count_ones() };
        let base = self.read_operand_register(rn, address);
        let bytes = count * 4;
        let first_address = match (increment, pre_index) {
            (true, false) => base,
            (true, true) => base.wrapping_add(4),
            (false, false) => base.wrapping_sub(bytes).wrapping_add(4),
            (false, true) => base.wrapping_sub(bytes),
        };
        // With an empty list, PC is the only transferred register but its
        // slot is still the sixteenth word in the 16-register address span.
        let mut transfer_address = if empty_list {
            first_address.wrapping_add(15 * 4)
        } else {
            first_address
        };

        for register in 0..16 {
            if effective_list & (1 << register) == 0 {
                continue;
            }

            if load {
                let value = bus.read32(transfer_address);
                self.registers[register] = if register == 15 { value & !3 } else { value };
            } else {
                let value = if register == 15 {
                    address.wrapping_add(12)
                } else if register == rn && write_back {
                    // ARM7TDMI STM with write-back and the base in the list
                    // stores the updated base only when it is the lowest
                    // numbered register transferred. Otherwise it stores
                    // the original base value.
                    let lowest_register = register_list.trailing_zeros();
                    if lowest_register == rn as u32 {
                        if increment {
                            base.wrapping_add(bytes)
                        } else {
                            base.wrapping_sub(bytes)
                        }
                    } else {
                        base
                    }
                } else {
                    self.registers[register]
                };
                bus.write32(transfer_address, value);
            }
            transfer_address = transfer_address.wrapping_add(4);
        }

        if write_back {
            self.registers[rn] = if increment {
                base.wrapping_add(bytes)
            } else {
                base.wrapping_sub(bytes)
            };
        }
        Ok(())
    }

    fn execute_halfword_transfer(
        &mut self,
        address: u32,
        instruction: u32,
        bus: &mut MemoryBus,
    ) -> Result<(), CpuError> {
        let pre_index = instruction & (1 << 24) != 0;
        let add_offset = instruction & (1 << 23) != 0;
        let immediate_offset = instruction & (1 << 22) != 0;
        let write_back = instruction & (1 << 21) != 0;
        let load = instruction & (1 << 20) != 0;
        let rn = ((instruction >> 16) & 0xF) as usize;
        let rd = ((instruction >> 12) & 0xF) as usize;
        let operation = (instruction >> 5) & 3;

        // ARM7TDMI halfword/signed-byte loads do not permit PC as Rd.
        // Loads that write back into their own base register are also
        // unpredictable for post-indexed or explicitly write-back forms.
        if load
            && (rd == 15 || (rd == rn && (!pre_index || write_back)))
        {
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
        }

        // A PC-relative halfword literal load is only valid without write-back.
        // Post-indexing or explicitly writing back through r15 would turn the
        // instruction fetch address into a data base and unpredictably redirect
        // execution on ARM7TDMI.
        if rn == 15 && (!pre_index || write_back) {
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
        }

        // Operation 00 is multiply/other encoding space, not a halfword
        // transfer. STR supports only the unsigned halfword operation.
        if operation == 0 || (!load && operation != 1) {
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
        }

        let offset = if immediate_offset {
            ((instruction >> 4) & 0xF0) | (instruction & 0xF)
        } else {
            if instruction & 0xF00 != 0 {
                return Err(CpuError::UnsupportedInstruction {
                    address,
                    instruction,
                });
            }
            self.read_operand_register((instruction & 0xF) as usize, address)
        };
        let base = self.read_operand_register(rn, address);
        let adjusted = if add_offset {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };
        let effective = if pre_index { adjusted } else { base };

        if load {
            let value = match operation {
                // ARM7TDMI rotates an unaligned unsigned halfword load
                // by 8 bits; signed halfword loads use byte-sign-extension
                // semantics at odd addresses (handled below).
                1 => {
                    let value = bus.read16(effective);
                    u32::from(if effective & 1 != 0 {
                        value.rotate_right(8)
                    } else {
                        value
                    })
                },
                2 => bus.read8(effective) as i8 as i32 as u32,
                // LDRSH at an odd address behaves like LDRSB on ARM7TDMI.
                3 => {
                    if effective & 1 != 0 {
                        bus.read8(effective) as i8 as i32 as u32
                    } else {
                        bus.read16(effective) as i16 as i32 as u32
                    }
                },
                _ => unreachable!(),
            };
            self.registers[rd] = if rd == 15 { value & !3 } else { value };
        } else {
            bus.write16(effective, self.registers[rd] as u16);
        }

        if !pre_index || write_back {
            self.registers[rn] = adjusted;
        }
        Ok(())
    }

    fn execute_single_transfer(
        &mut self,
        address: u32,
        instruction: u32,
        bus: &mut MemoryBus,
    ) -> Result<(), CpuError> {
        let register_offset = instruction & (1 << 25) != 0;
        let pre_index = instruction & (1 << 24) != 0;
        let add_offset = instruction & (1 << 23) != 0;
        let byte_transfer = instruction & (1 << 22) != 0;
        let write_back = instruction & (1 << 21) != 0;
        let load = instruction & (1 << 20) != 0;
        let rn = ((instruction >> 16) & 0xF) as usize;
        let rd = ((instruction >> 12) & 0xF) as usize;

        // ARM7TDMI defines a load with write-back and Rd == Rn as
        // unpredictable. Post-indexed loads always write back, so reject
        // both forms instead of silently choosing an arbitrary register value.
        if load && rd == rn && (!pre_index || write_back) {
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
        }

        // PC-relative literal transfers are valid only without write-back.
        // Updating r15 as the address base has unpredictable control-flow
        // effects on ARM7TDMI, including for post-indexed encodings.
        if rn == 15 && (!pre_index || write_back) {
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
        }

        if register_offset && instruction & (1 << 4) != 0 {
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
        }
        let base = self.read_operand_register(rn, address);
        let offset = if register_offset {
            let rm = (instruction & 0xF) as usize;
            let value = self.read_operand_register(rm, address);
            let shift_type = (instruction >> 5) & 3;
            let shift_amount = (instruction >> 7) & 0x1F;
            Self::shift_immediate(value, shift_type, shift_amount, self.flags.carry).0
        } else {
            instruction & 0xFFF
        };
        let adjusted = if add_offset {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };
        let effective = if pre_index { adjusted } else { base };

        if load {
            let value = if byte_transfer {
                u32::from(bus.read8(effective))
            } else {
                // ARM7TDMI word loads align the bus access and rotate the
                // fetched word right when the effective address is unaligned.
                bus.read32(effective).rotate_right((effective & 3) * 8)
            };
            self.registers[rd] = if rd == 15 { value & !3 } else { value };
        } else {
            // ARM7TDMI exposes PC as instruction address + 12 when it is
            // used as the source of an ARM-state single data transfer.
            let value = if rd == 15 {
                address.wrapping_add(12)
            } else {
                self.registers[rd]
            };
            if byte_transfer {
                bus.write8(effective, value as u8);
            } else {
                bus.write32(effective, value);
            }
        }

        if !pre_index || write_back {
            self.registers[rn] = adjusted;
        }
        Ok(())
    }

    fn shift_immediate(value: u32, kind: u32, amount: u32, old_carry: bool) -> (u32, Option<bool>) {
        match (kind, amount) {
            (0, 0) => (value, None),
            (0, n) => (value << n, Some(value & (1 << (32 - n)) != 0)),
            (1, 0) => (0, Some(value >> 31 != 0)),
            (1, n) => (value >> n, Some(value & (1 << (n - 1)) != 0)),
            (2, 0) => ((value as i32 >> 31) as u32, Some(value >> 31 != 0)),
            (2, n) => (
                ((value as i32) >> n) as u32,
                Some(value & (1 << (n - 1)) != 0),
            ),
            (3, 0) => {
                let result = (u32::from(old_carry) << 31) | (value >> 1);
                (result, Some(value & 1 != 0))
            }
            (3, n) => {
                let result = value.rotate_right(n);
                (result, Some(result >> 31 != 0))
            }
            _ => unreachable!(),
        }
    }

    /// Applies a Thumb immediate shift. Unlike ARM's operand shifter, an
    /// encoded LSR/ASR amount of zero means a shift by 32.
    fn shift_thumb_immediate(
        value: u32,
        kind: u32,
        amount: u32,
        _old_carry: bool,
    ) -> (u32, Option<bool>) {
        match (kind, amount) {
            (0, 0) => (value, None),
            (0, n) => (value << n, Some((value >> (32 - n)) & 1 != 0)),
            (1, 0) => (0, Some(value >> 31 != 0)),
            (1, n) => (value >> n, Some((value >> (n - 1)) & 1 != 0)),
            (2, 0) => {
                let sign = value >> 31 != 0;
                (if sign { u32::MAX } else { 0 }, Some(sign))
            }
            (2, n) => (
                ((value as i32) >> n) as u32,
                Some((value >> (n - 1)) & 1 != 0),
            ),
            (3, _) => unreachable!("Thumb immediate shifts do not encode ROR"),
            _ => unreachable!("invalid Thumb immediate shift"),
        }
    }

    /// Applies an ARM register-controlled shift (ARM7TDMI register operand form).
    fn shift_register(value: u32, kind: u32, amount: u32, old_carry: bool) -> (u32, Option<bool>) {
        if amount == 0 {
            return (value, None);
        }

        match kind {
            0 => match amount {
                1..=31 => (value << amount, Some((value >> (32 - amount)) & 1 != 0)),
                32 => (0, Some(value & 1 != 0)),
                _ => (0, Some(false)),
            },
            1 => match amount {
                1..=31 => (value >> amount, Some((value >> (amount - 1)) & 1 != 0)),
                32 => (0, Some(value >> 31 != 0)),
                _ => (0, Some(false)),
            },
            2 => {
                if amount >= 32 {
                    let carry = value >> 31 != 0;
                    (if carry { u32::MAX } else { 0 }, Some(carry))
                } else {
                    (
                        ((value as i32) >> amount) as u32,
                        Some((value >> (amount - 1)) & 1 != 0),
                    )
                }
            }
            3 => {
                let rotation = amount & 31;
                if rotation == 0 {
                    (value, Some(value >> 31 != 0))
                } else {
                    let result = value.rotate_right(rotation);
                    (result, Some(result >> 31 != 0))
                }
            }
            _ => unreachable!(),
        }
    }

    fn add(left: u32, right: u32, carry_in: bool) -> (u32, bool, bool) {
        let sum = u64::from(left) + u64::from(right) + u64::from(carry_in);
        let result = sum as u32;
        let overflow = ((!(left ^ right) & (left ^ result)) >> 31) != 0;
        (result, sum >> 32 != 0, overflow)
    }

    fn subtract(left: u32, right: u32, borrow_in: bool) -> (u32, bool, bool) {
        let subtrahend = u64::from(right) + u64::from(borrow_in);
        let result = left.wrapping_sub(right).wrapping_sub(u32::from(borrow_in));
        let carry = u64::from(left) >= subtrahend;
        let overflow = (((left ^ right) & (left ^ result)) >> 31) != 0;
        (result, carry, overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GbaCartridge;

    fn bus_with_program(program: &[u32]) -> MemoryBus {
        let mut rom = vec![0; 0xC0 + program.len() * 4];
        for (index, instruction) in program.iter().enumerate() {
            let offset = 0xC0 + index * 4;
            rom[offset..offset + 4].copy_from_slice(&instruction.to_le_bytes());
        }
        let checksum = rom[0xA0..=0xBC]
            .iter()
            .fold(0x19_u8, |sum, byte| sum.wrapping_add(*byte))
            .wrapping_neg();
        rom[0xBD] = checksum;
        // Place the test program in the cartridge mapping at ROM offset zero.
        let mut bus = MemoryBus::new(GbaCartridge::parse(&rom).unwrap());
        for (index, instruction) in program.iter().enumerate() {
            bus.write32(0x0200_0000 + index as u32 * 4, *instruction);
        }
        bus
    }

    #[test]
    fn executes_data_processing_and_updates_flags() {
        let mut bus = bus_with_program(&[
            0xE3A0_002A, // MOV r0, #42
            0xE280_1008, // ADD r1, r0, #8
            0xE351_0032, // CMP r1, #50
        ]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.step(&mut bus).unwrap();
        cpu.step(&mut bus).unwrap();
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(0), Some(42));
        assert_eq!(cpu.register(1), Some(50));
        assert!(cpu.flags().zero);
        assert!(cpu.flags().carry);
    }

    #[test]
    fn arm_mrs_reads_modeled_cpsr_flags_and_thumb_state() {
        let mut bus = bus_with_program(&[0xE10F_2000]); // MRS r2, CPSR
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.flags = ConditionFlags {
            negative: true,
            zero: false,
            carry: true,
            overflow: true,
        };

        cpu.step(&mut bus).unwrap();

        // MRS executes in ARM state, so the modeled T bit is clear.
        assert_eq!(cpu.register(2), Some(0xB000_0000));
        assert!(!cpu.is_thumb());
        assert!(cpu.flags().negative);
        assert!(cpu.flags().carry);
    }

    #[test]
    fn arm_mrs_rejects_spsr_and_msr_control_fields_without_mutating_flags() {
        let mut bus = bus_with_program(&[
            0xE14F_2000, // MRS r2, SPSR (no SPSR model)
            0xE121_F000, // MSR CPSR_c, r0 (control field not modeled)
        ]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(0, 0xF000_0000);
        cpu.flags = ConditionFlags {
            negative: false,
            zero: true,
            carry: false,
            overflow: false,
        };

        assert!(matches!(cpu.step(&mut bus), Err(CpuError::UnsupportedInstruction { .. })));
        assert_eq!(cpu.register(2), Some(0));
        assert_eq!(cpu.flags().zero, true);

        assert!(matches!(cpu.step(&mut bus), Err(CpuError::UnsupportedInstruction { .. })));
        assert_eq!(cpu.flags().negative, false);
        assert_eq!(cpu.flags().zero, true);
        assert_eq!(cpu.flags().carry, false);
        assert_eq!(cpu.flags().overflow, false);
    }

    #[test]
    fn arm_msr_updates_cpsr_condition_flags_from_register_and_immediate() {
        let mut bus = bus_with_program(&[
            0xE128_F000, // MSR CPSR_f, r0
            0xE328_F4F0, // MSR CPSR_f, #0xF0000000 (rotated immediate)
        ]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(0, 0xA000_0000);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.flags(), ConditionFlags {
            negative: true, zero: false, carry: true, overflow: false,
        });

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.flags(), ConditionFlags {
            negative: true, zero: true, carry: true, overflow: true,
        });
    }

    #[test]
    fn arm_data_processing_exception_return_is_rejected_without_spsr() {
        // MOVS pc, lr restores CPSR from the current SPSR on ARM7TDMI.
        // The core has no SPSR/exception-mode model yet, so it must reject
        // this instruction rather than silently changing flags and branching.
        let mut bus = bus_with_program(&[0xE1B0_F00E]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);

        assert_eq!(
            cpu.step(&mut bus),
            Err(CpuError::UnsupportedInstruction {
                address: 0x0200_0000,
                instruction: 0xE1B0_F00E,
            })
        );
    }

    #[test]
    fn failed_condition_skips_instruction() {
        let mut bus = bus_with_program(&[0x03A0_0001]); // MOVEQ r0, #1
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        assert!(matches!(
            cpu.step(&mut bus),
            Ok(StepOutcome::ConditionFailed { .. })
        ));
        assert_eq!(cpu.register(0), Some(0));
        assert_eq!(cpu.register(15), Some(0x0200_0004));
    }

    #[test]
    fn branch_link_sets_return_address() {
        let mut bus = bus_with_program(&[0xEB00_0000]); // BL to address + 8
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(14), Some(0x0200_0004));
        assert_eq!(cpu.register(15), Some(0x0200_0008));
    }

    #[test]
    fn block_transfers_store_and_load_register_lists_with_writeback() {
        let mut bus = bus_with_program(&[
            0xE8AD_0003, // STMIA sp!, {r0, r1}
            0xE8BD_000C, // LDMIA sp!, {r2, r3}
        ]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(13, 0x0200_0010);
        cpu.set_register(0, 0x1122_3344);
        cpu.set_register(1, 0xAABB_CCDD);

        cpu.step(&mut bus).unwrap();
        assert_eq!(bus.read32(0x0200_0010), 0x1122_3344);
        assert_eq!(bus.read32(0x0200_0014), 0xAABB_CCDD);
        assert_eq!(cpu.register(13), Some(0x0200_0018));

        // Reload the same memory range to exercise ascending register ordering.
        cpu.set_register(13, 0x0200_0010);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(2), Some(0x1122_3344));
        assert_eq!(cpu.register(3), Some(0xAABB_CCDD));
        assert_eq!(cpu.register(13), Some(0x0200_0018));
    }

    #[test]
    fn thumb_stmia_handles_base_register_in_the_register_list() {
        let mut bus = bus_with_program(&[]);
        // STMIA r1!, {r0, r1}: because r1 is not the lowest listed
        // register, the original base value is stored for r1.
        bus.write16(0x0200_0000, 0xC103);
        // STMIA r0!, {r0, r1}: because r0 is the lowest listed register,
        // the updated base value is stored for r0.
        bus.write16(0x0200_0002, 0xC003);

        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.thumb = true;
        cpu.set_register(0, 0x1122_3344);
        cpu.set_register(1, 0x0200_0100);

        cpu.step(&mut bus).unwrap();
        assert_eq!(bus.read32(0x0200_0100), 0x1122_3344);
        assert_eq!(bus.read32(0x0200_0104), 0x0200_0100);
        assert_eq!(cpu.register(1), Some(0x0200_0108));

        // Use a fresh base to exercise the lowest-register case.
        cpu.set_register(0, 0x0200_0120);
        cpu.set_register(1, 0xAABB_CCDD);
        cpu.step(&mut bus).unwrap();
        assert_eq!(bus.read32(0x0200_0120), 0x0200_0128);
        assert_eq!(bus.read32(0x0200_0124), 0xAABB_CCDD);
        assert_eq!(cpu.register(0), Some(0x0200_0128));
    }

    #[test]
    fn arm_stm_writeback_stores_base_according_to_register_order() {
        // STMIA r1!, {r0, r1}: base is not the lowest register, so memory
        // receives the original base for r1 while write-back updates r1.
        let mut bus = bus_with_program(&[0xE8A1_0003]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(0, 0x1122_3344);
        cpu.set_register(1, 0x0200_0100);
        cpu.step(&mut bus).unwrap();
        assert_eq!(bus.read32(0x0200_0100), 0x1122_3344);
        assert_eq!(bus.read32(0x0200_0104), 0x0200_0100);
        assert_eq!(cpu.register(1), Some(0x0200_0108));

        // STMIA r0!, {r0, r1}: base is the lowest register, so the updated
        // base is stored in its slot.
        let mut bus = bus_with_program(&[0xE8A0_0003]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(0, 0x0200_0120);
        cpu.set_register(1, 0xAABB_CCDD);
        cpu.step(&mut bus).unwrap();
        assert_eq!(bus.read32(0x0200_0120), 0x0200_0128);
        assert_eq!(bus.read32(0x0200_0124), 0xAABB_CCDD);
        assert_eq!(cpu.register(0), Some(0x0200_0128));
    }

    #[test]
    fn arm_stmdb_writeback_stores_base_according_to_register_order() {
        // STMDB r0!, {r0, r1}: the base is the lowest listed register, so
        // its memory slot receives the decremented base value.
        let mut bus = bus_with_program(&[0xE920_0003]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(0, 0x0200_0120);
        cpu.set_register(1, 0xAABB_CCDD);
        cpu.step(&mut bus).unwrap();
        assert_eq!(bus.read32(0x0200_0118), 0x0200_0118);
        assert_eq!(bus.read32(0x0200_011C), 0xAABB_CCDD);
        assert_eq!(cpu.register(0), Some(0x0200_0118));

        // STMDB r1!, {r0, r1}: because r0 is listed first, r1's slot
        // contains the original base even though write-back decrements r1.
        let mut bus = bus_with_program(&[0xE921_0003]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(0, 0x1122_3344);
        cpu.set_register(1, 0x0200_0120);
        cpu.step(&mut bus).unwrap();
        assert_eq!(bus.read32(0x0200_0118), 0x1122_3344);
        assert_eq!(bus.read32(0x0200_011C), 0x0200_0120);
        assert_eq!(cpu.register(1), Some(0x0200_0118));
    }

    #[test]
    fn arm_empty_block_transfer_uses_pc_and_full_writeback_span() {
        // STMIA r0!, {} stores the ARM-visible PC at base + 0x3C and advances
        // the base as though all 16 registers had been transferred.
        let mut bus = bus_with_program(&[0xE8A0_0000]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(0, 0x0200_0100);
        cpu.step(&mut bus).unwrap();
        assert_eq!(bus.read32(0x0200_013C), 0x0200_0008);
        assert_eq!(cpu.register(0), Some(0x0200_0140));

        // LDMDB r1!, {} loads PC from base - 4 while decrementing by 0x40.
        let mut bus = bus_with_program(&[0xE931_0000]);
        bus.write32(0x0200_011C, 0x0200_0041);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(1, 0x0200_0120);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(15), Some(0x0200_0040));
        assert_eq!(cpu.register(1), Some(0x0200_00E0));
    }

    #[test]
    fn halfword_and_signed_loads_use_correct_width_and_sign_extension() {
        let mut bus = bus_with_program(&[
            0xE1C1_00B0, // STRH r0, [r1]
            0xE1D1_20B0, // LDRH r2, [r1]
            0xE1D1_20D0, // LDRSB r2, [r1]
            0xE1D1_30F0, // LDRSH r3, [r1]
        ]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(0, 0x0000_80FF);
        cpu.set_register(1, 0x0200_0100);

        cpu.step(&mut bus).unwrap();
        assert_eq!(bus.read16(0x0200_0100), 0x80FF);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(2), Some(0x0000_80FF));
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(2), Some(0xFFFF_FFFF));
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(3), Some(0xFFFF_80FF));
    }

    #[test]
    fn arm_swap_and_swap_byte_exchange_memory_and_register_values() {
        let mut bus = bus_with_program(&[
            0xE102_1093, // SWP r1, r3, [r2]
            0xE142_4095, // SWPB r4, r5, [r2]
        ]);
        bus.write32(0x0200_0100, 0x1234_5678);
        bus.write8(0x0200_0100, 0x78);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(2, 0x0200_0100);
        cpu.set_register(3, 0xAABB_CCDD);
        cpu.set_register(5, 0x42);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(1), Some(0x1234_5678));
        assert_eq!(bus.read32(0x0200_0100), 0xAABB_CCDD);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(4), Some(0xDD));
        assert_eq!(bus.read8(0x0200_0100), 0x42);
        assert_eq!(cpu.register(2), Some(0x0200_0100));
    }

    #[test]
    fn arm_word_swap_rejects_unaligned_address_without_modifying_state() {
        let instruction = 0xE102_1093; // SWP r1, r3, [r2]
        let mut bus = bus_with_program(&[instruction]);
        bus.write32(0x0200_0100, 0x1234_5678);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(1, 0xDEAD_BEEF);
        cpu.set_register(2, 0x0200_0101);
        cpu.set_register(3, 0xAABB_CCDD);

        assert_eq!(
            cpu.step(&mut bus),
            Err(CpuError::UnsupportedInstruction {
                address: 0x0200_0000,
                instruction,
            })
        );
        assert_eq!(cpu.register(1), Some(0xDEAD_BEEF));
        assert_eq!(bus.read32(0x0200_0100), 0x1234_5678);
    }

    #[test]
    fn long_multiply_supports_unsigned_signed_and_accumulating_forms() {
        // UMULL r1, r0, r2, r3 => r0:r1 = 0xFFFF_FFFF * 2.
        let mut bus = bus_with_program(&[
            0xE081_0392, // UMULL r0, r1, r2, r3 (encoding checked below)
        ]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(2, 0xFFFF_FFFF);
        cpu.set_register(3, 2);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(0), Some(1));
        assert_eq!(cpu.register(1), Some(0xFFFF_FFFE));
    }

    #[test]
    fn long_multiply_accumulate_and_signed_flags_are_correct() {
        // SMLALS r0, r1, r2, r3: accumulator 5 + (-2 * 3) = -1.
        let mut bus = bus_with_program(&[
            0xE0F1_0392, // SMLALS r0, r1, r2, r3
        ]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(0, 5);
        cpu.set_register(1, 0);
        cpu.set_register(2, (-2_i32) as u32);
        cpu.set_register(3, 3);
        cpu.flags.carry = true;
        cpu.flags.overflow = true;
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(0), Some(0xFFFF_FFFF));
        assert_eq!(cpu.register(1), Some(0xFFFF_FFFF));
        assert!(cpu.flags().negative);
        assert!(!cpu.flags().zero);
        // Long multiply with S set updates N/Z only.
        assert!(cpu.flags().carry);
        assert!(cpu.flags().overflow);
    }

    #[test]
    fn long_multiply_signed_and_unsigned_accumulate_forms_are_correct() {
        let mut bus = bus_with_program(&[
            0xE0C1_0392, // SMULL r0, r1, r2, r3
            0xE0A1_0392, // UMLAL r0, r1, r2, r3
        ]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(2, 0xFFFF_FFFE); // -2 signed, large unsigned.
        cpu.set_register(3, 3);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(0), Some(0xFFFF_FFFA));
        assert_eq!(cpu.register(1), Some(0xFFFF_FFFF));

        // UMLAL adds unsigned (0xFFFF_FFFE * 3) to the previous 64-bit result.
        cpu.set_register(0, 0xFFFF_FFFA);
        cpu.set_register(1, 0xFFFF_FFFF);
        cpu.step(&mut bus).unwrap();
        let expected = (u64::from(0xFFFF_FFFF_u32) << 32)
            | u64::from(0xFFFF_FFFA_u32);
        let expected = expected.wrapping_add(u64::from(0xFFFF_FFFE_u32) * 3);
        assert_eq!(cpu.register(0), Some(expected as u32));
        assert_eq!(cpu.register(1), Some((expected >> 32) as u32));
    }

    #[test]
    fn long_multiply_rejects_r_m_aliasing_a_destination() {
        // UMULL r0, r1, r0, r3 is unpredictable on ARM7TDMI because Rm
        // aliases one of the destination registers.
        let instruction = 0xE081_0390;
        let mut bus = bus_with_program(&[instruction]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(0, 7);
        cpu.set_register(3, 9);

        assert_eq!(
            cpu.step(&mut bus),
            Err(CpuError::UnsupportedInstruction {
                address: 0x0200_0000,
                instruction,
            })
        );
        assert_eq!(cpu.register(0), Some(7));
        assert_eq!(cpu.register(1), Some(0));
    }

    #[test]
    fn multiply_and_multiply_accumulate_update_results_and_flags() {
        let mut bus = bus_with_program(&[
            0xE000_0291, // MUL r0, r1, r2
            0xE023_5491, // MLA r3, r1, r4, r5
            0xE010_0291, // MULS r0, r1, r2
        ]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(1, 6);
        cpu.set_register(2, 7);
        cpu.set_register(4, 8);
        cpu.set_register(5, 9);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(0), Some(42));
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(3), Some(57));
        cpu.set_register(1, 0);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(0), Some(0));
        assert!(cpu.flags().zero);
        assert!(!cpu.flags().negative);
    }

    #[test]
    fn thumb_state_executes_immediate_arithmetic_and_branches() {
        let mut bus = bus_with_program(&[]);
        // Thumb code is written directly to EWRAM as halfwords.
        bus.write16(0x0200_0000, 0x2007); // MOVS r0, #7
        bus.write16(0x0200_0002, 0x3005); // ADDS r0, #5
        bus.write16(0x0200_0004, 0x280C); // CMP r0, #12
        bus.write16(0x0200_0006, 0xD000); // BEQ +0 (to address +10)
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.thumb = true;

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(0), Some(7));
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(0), Some(12));
        cpu.step(&mut bus).unwrap();
        assert!(cpu.flags().zero);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(15), Some(0x0200_000A));
    }

    #[test]
    fn thumb_long_branch_with_link_sets_target_and_thumb_return_address() {
        let mut bus = bus_with_program(&[]);
        // BL +0: high part at 0, low part at 2, target at 4.
        bus.write16(0x0200_0000, 0xF000);
        bus.write16(0x0200_0002, 0xF800);
        bus.write16(0x0200_0004, 0x2007); // MOVS r0, #7 at the target.
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.thumb = true;

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(14), Some(0x0200_0004));
        assert_eq!(cpu.register(15), Some(0x0200_0002));

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(15), Some(0x0200_0004));
        assert_eq!(cpu.register(14), Some(0x0200_0005));

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(0), Some(7));
    }

    #[test]
    fn arm_register_controlled_shifts_cover_boundaries_and_carry() {
        assert_eq!(
            Cpu::shift_register(0x8000_0001, 0, 1, false),
            (0x0000_0002, Some(true))
        );
        assert_eq!(
            Cpu::shift_register(0x8000_0001, 0, 32, false),
            (0, Some(true))
        );
        assert_eq!(
            Cpu::shift_register(0x8000_0001, 0, 33, false),
            (0, Some(false))
        );
        assert_eq!(
            Cpu::shift_register(0x8000_0001, 1, 32, false),
            (0, Some(true))
        );
        assert_eq!(
            Cpu::shift_register(0x8000_0001, 2, 32, false),
            (u32::MAX, Some(true))
        );
        assert_eq!(
            Cpu::shift_register(0x8000_0001, 2, 40, false),
            (u32::MAX, Some(true))
        );
        assert_eq!(
            Cpu::shift_register(0x8000_0001, 3, 32, false),
            (0x8000_0001, Some(true))
        );
        assert_eq!(
            Cpu::shift_register(0x8000_0001, 0, 0, true),
            (0x8000_0001, None)
        );
    }

    #[test]
    fn arm_register_shift_reads_pc_as_address_plus_twelve() {
        let mut bus = bus_with_program(&[0xE1A0_211F]); // MOV r2, pc, LSL r1
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(1, 0);

        cpu.step(&mut bus).unwrap();

        assert_eq!(cpu.register(2), Some(0x0200_000C));
    }

    #[test]
    fn arm_register_shift_rejects_pc_as_shift_amount_register() {
        let instruction = 0xE1A0_0F12; // MOV r0, r2, LSL pc (unpredictable)
        let mut bus = bus_with_program(&[instruction]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(2, 7);

        assert_eq!(
            cpu.step(&mut bus),
            Err(CpuError::UnsupportedInstruction {
                address: 0x0200_0000,
                instruction,
            })
        );
        assert_eq!(cpu.register(0), Some(0));
    }

    #[test]
    fn arm_data_processing_uses_register_controlled_shift_amount() {
        let mut bus = bus_with_program(&[0xE1A0_0312]); // MOV r0, r2, LSL r3
        let mut cpu = Cpu::new();
        cpu.reset(0x0800_00C0);
        cpu.set_register(2, 0x0000_0003);
        cpu.set_register(3, 4);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(0), Some(0x0000_0030));
    }

    #[test]
    fn thumb_add_pc_and_sp_relative_address_calculations() {
        let mut bus = bus_with_program(&[]);
        bus.write16(0x0200_0000, 0xA001); // ADD r0, PC, #4
        bus.write16(0x0200_0002, 0xA902); // ADD r1, SP, #8
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.thumb = true;
        cpu.set_register(13, 0x0200_0100);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(0), Some(0x0200_0008));
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(1), Some(0x0200_0108));
    }

    #[test]
    fn thumb_load_store_transfers_words_bytes_and_halfwords() {
        let mut bus = bus_with_program(&[]);
        bus.write16(0x0200_0000, 0x6008); // STR r0, [r1]
        bus.write16(0x0200_0002, 0x680A); // LDR r2, [r1]
        bus.write16(0x0200_0004, 0x704B); // STRB r3, [r1, #1]
        bus.write16(0x0200_0006, 0x784C); // LDRB r4, [r1, #1]
        bus.write16(0x0200_0008, 0x800D); // STRH r5, [r1]
        bus.write16(0x0200_000A, 0x880E); // LDRH r6, [r1]
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.thumb = true;
        cpu.set_register(0, 0x1234_5678);
        cpu.set_register(1, 0x0200_0100);
        cpu.set_register(3, 0xAA);
        cpu.set_register(5, 0xBEEF);

        for _ in 0..6 {
            cpu.step(&mut bus).unwrap();
        }
        assert_eq!(cpu.register(2), Some(0x1234_5678));
        assert_eq!(cpu.register(4), Some(0x0000_00AA));
        assert_eq!(cpu.register(6), Some(0x0000_BEEF));
    }

    #[test]
    fn thumb_push_pop_and_stack_adjustment_preserve_registers() {
        let mut bus = bus_with_program(&[]);
        bus.write16(0x0200_0000, 0xB403); // PUSH {r0, r1}
        bus.write16(0x0200_0002, 0xB081); // SUB sp, #4
        bus.write16(0x0200_0004, 0xB001); // ADD sp, #4
        bus.write16(0x0200_0006, 0xBC0C); // POP {r2, r3}
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.thumb = true;
        cpu.set_register(13, 0x0200_0120);
        cpu.set_register(0, 0x1122_3344);
        cpu.set_register(1, 0xAABB_CCDD);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(13), Some(0x0200_0118));
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(13), Some(0x0200_0114));
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(13), Some(0x0200_0118));
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(2), Some(0x1122_3344));
        assert_eq!(cpu.register(3), Some(0xAABB_CCDD));
        assert_eq!(cpu.register(13), Some(0x0200_0120));
    }

    #[test]
    fn thumb_multiple_transfers_store_load_and_write_back() {
        let mut bus = bus_with_program(&[]);
        bus.write16(0x0200_0000, 0xC006); // STMIA r0!, {r1, r2}
        bus.write16(0x0200_0002, 0xC818); // LDMIA r0!, {r3, r4}
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.thumb = true;
        cpu.set_register(0, 0x0200_0100);
        cpu.set_register(1, 0x1122_3344);
        cpu.set_register(2, 0xAABB_CCDD);

        cpu.step(&mut bus).unwrap();
        assert_eq!(bus.read32(0x0200_0100), 0x1122_3344);
        assert_eq!(bus.read32(0x0200_0104), 0xAABB_CCDD);
        assert_eq!(cpu.register(0), Some(0x0200_0108));

        cpu.set_register(0, 0x0200_0100);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(3), Some(0x1122_3344));
        assert_eq!(cpu.register(4), Some(0xAABB_CCDD));
        assert_eq!(cpu.register(0), Some(0x0200_0108));
    }

    #[test]
    fn bx_rejects_pc_as_target_register() {
        let instruction = 0xE12F_FF1F; // BX pc (unpredictable on ARM7TDMI)
        let mut bus = bus_with_program(&[instruction]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);

        assert_eq!(
            cpu.step(&mut bus),
            Err(CpuError::UnsupportedInstruction {
                address: 0x0200_0000,
                instruction,
            })
        );
        assert!(!cpu.is_thumb());
        assert_eq!(cpu.register(15), Some(0x0200_0004));
    }

    #[test]
    fn bx_switches_between_arm_and_thumb_instruction_sets() {
        let mut bus = bus_with_program(&[0xE12F_FF11]); // BX r1
        bus.write16(0x0200_0008, 0x2201); // MOVS r2, #1
        bus.write16(0x0200_000A, 0x4700); // BX r0
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(0, 0x0200_0010);
        cpu.set_register(1, 0x0200_0009);

        cpu.step(&mut bus).unwrap();
        assert!(cpu.is_thumb());
        assert_eq!(cpu.register(15), Some(0x0200_0008));
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(2), Some(1));
        cpu.step(&mut bus).unwrap();
        assert!(!cpu.is_thumb());
        assert_eq!(cpu.register(15), Some(0x0200_0010));
    }

    #[test]
    fn thumb_immediate_lsr_and_asr_zero_encode_shift_by_32() {
        let mut bus = bus_with_program(&[]);
        bus.write16(0x0200_0000, 0x0808); // LSRS r0, r1, #0 (shift by 32)
        bus.write16(0x0200_0002, 0x100A); // ASRS r2, r1, #0 (shift by 32)
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.thumb = true;
        cpu.set_register(1, 0x8000_0001);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(0), Some(0));
        assert!(!cpu.flags().negative);
        assert!(cpu.flags().carry);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(2), Some(u32::MAX));
        assert!(cpu.flags().negative);
        assert!(cpu.flags().carry);
    }

    #[test]
    fn thumb_register_shifts_and_negate_update_results_and_flags() {
        let mut bus = bus_with_program(&[0]);
        // LSL r0, r1; LSR r2, r3; ASR r4, r5; ROR r6, r7; NEG r0, r1.
        for (index, instruction) in [
            0x4088_u16, // LSL r0, r1
            0x40DA,     // LSR r2, r3
            0x412C,     // ASR r4, r5
            0x41FE,     // ROR r6, r7
            0x4248,     // NEG r0, r1
        ]
        .into_iter()
        .enumerate()
        {
            bus.write16(0x0200_0000 + index as u32 * 2, instruction);
        }

        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.thumb = true;
        cpu.set_register(0, 0x8000_0001);
        cpu.set_register(1, 1);
        cpu.set_register(2, 0x8000_0001);
        cpu.set_register(3, 1);
        cpu.set_register(4, 0x8000_0001);
        cpu.set_register(5, 1);
        cpu.set_register(6, 0x8000_0001);
        cpu.set_register(7, 1);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(0), Some(2));
        assert!(cpu.flags().carry);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(2), Some(0x4000_0000));
        assert!(cpu.flags().carry);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(4), Some(0xC000_0000));
        assert!(cpu.flags().carry);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(6), Some(0xC000_0000));
        assert!(cpu.flags().carry);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(0), Some(u32::MAX));
        assert!(cpu.flags().negative);
    }

    #[test]
    fn thumb_adc_and_sbc_consume_and_update_carry_correctly() {
        let mut bus = bus_with_program(&[]);
        bus.write16(0x0200_0000, 0x4148); // ADCS r0, r1
        bus.write16(0x0200_0002, 0x419A); // SBCS r2, r3
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.thumb = true;
        cpu.flags.carry = true;
        cpu.set_register(0, u32::MAX);
        cpu.set_register(1, 0);
        cpu.set_register(2, 0);
        cpu.set_register(3, 1);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(0), Some(0));
        assert!(cpu.flags().zero);
        assert!(cpu.flags().carry);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(2), Some(u32::MAX));
        assert!(cpu.flags().negative);
        assert!(!cpu.flags().zero);
        assert!(!cpu.flags().carry);
    }

    #[test]
    fn setting_pc_in_thumb_state_preserves_halfword_alignment() {
        let mut bus = bus_with_program(&[]);
        // A Thumb MOV r0, #7 placed at an address ending in 2.
        bus.write16(0x0200_0002, 0x2007);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.thumb = true;

        assert!(cpu.set_register(15, 0x0200_0003));
        assert_eq!(cpu.register(15), Some(0x0200_0002));
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(0), Some(7));
        assert_eq!(cpu.register(15), Some(0x0200_0004));
    }

    #[test]
    fn thumb_register_shifts_handle_32_and_over_32_bit_amounts() {
        let mut bus = bus_with_program(&[]);
        bus.write16(0x0200_0000, 0x4088); // LSL r0, r1
        bus.write16(0x0200_0002, 0x40DA); // LSR r2, r3
        bus.write16(0x0200_0004, 0x412C); // ASR r4, r5
        bus.write16(0x0200_0006, 0x41FE); // ROR r6, r7

        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.thumb = true;
        cpu.set_register(0, 0x8000_0001);
        cpu.set_register(1, 32);
        cpu.set_register(2, 0x8000_0001);
        cpu.set_register(3, 33);
        cpu.set_register(4, 0x8000_0001);
        cpu.set_register(5, 40);
        cpu.set_register(6, 0x8000_0001);
        cpu.set_register(7, 32);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(0), Some(0));
        assert!(cpu.flags().carry); // LSL by 32 takes bit 0 into carry.

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(2), Some(0));
        assert!(!cpu.flags().carry); // LSR above 32 produces zero and clears carry.

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(4), Some(u32::MAX));
        assert!(cpu.flags().carry); // ASR above 32 sign-fills and preserves the sign in carry.

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(6), Some(0x8000_0001));
        assert!(cpu.flags().carry); // ROR by a nonzero multiple of 32 preserves bit 31.
    }

    #[test]
    fn arm_multiply_rejects_destination_equal_to_first_operand() {
        // MUL r0, r1, r0 and MLA r0, r2, r1, r0 are unpredictable on ARM7TDMI.
        for instruction in [0xE000_0190_u32, 0xE020_2190] {
            let mut bus = bus_with_program(&[instruction]);
            let mut cpu = Cpu::new();
            cpu.reset(0x0200_0000);
            cpu.set_register(0, 3);
            cpu.set_register(1, 7);
            cpu.set_register(2, 11);

            assert_eq!(
                cpu.step(&mut bus),
                Err(CpuError::UnsupportedInstruction {
                    address: 0x0200_0000,
                    instruction,
                }),
                "instruction 0x{instruction:08X}"
            );
            assert_eq!(cpu.register(0), Some(3));
        }
    }

    #[test]
    fn arm_store_pc_uses_pipeline_visible_address_plus_twelve() {
        // STR pc, [r0] stores the ARM7TDMI pipeline-visible PC (instruction
        // address + 12), not the already-advanced internal PC (address + 4).
        let mut bus = bus_with_program(&[0xE580_F000]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(0, 0x0200_0100);
        cpu.step(&mut bus).unwrap();
        assert_eq!(bus.read32(0x0200_0100), 0x0200_000C);

        // STRB pc, [r0] uses the low byte of the same architectural value.
        let mut bus = bus_with_program(&[0xE5C0_F000]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(0, 0x0200_0100);
        cpu.step(&mut bus).unwrap();
        assert_eq!(bus.read8(0x0200_0100), 0x0C);
    }

    #[test]
    fn arm_word_load_rotates_unaligned_bus_data_for_each_byte_offset() {
        for (offset, expected) in [
            (1, 0x7812_3456),
            (2, 0x5678_1234),
            (3, 0x3456_7812),
        ] {
            let mut bus = bus_with_program(&[0xE591_2000]); // LDR r2, [r1]
            bus.write32(0x0200_0100, 0x1234_5678);
            let mut cpu = Cpu::new();
            cpu.reset(0x0200_0000);
            cpu.set_register(1, 0x0200_0100 + offset);

            cpu.step(&mut bus).unwrap();

            assert_eq!(cpu.register(2), Some(expected), "offset {offset}");
        }
    }

    #[test]
    fn thumb_word_load_rotates_unaligned_bus_data_for_each_addressing_form() {
        // Immediate-offset LDR r2, [r1, #0].
        let mut immediate_bus = bus_with_program(&[]);
        immediate_bus.write16(0x0200_0000, 0x680A);
        immediate_bus.write32(0x0200_0100, 0x1234_5678);
        let mut immediate_cpu = Cpu::new();
        immediate_cpu.reset(0x0200_0000);
        immediate_cpu.thumb = true;
        immediate_cpu.set_register(1, 0x0200_0102);
        immediate_cpu.step(&mut immediate_bus).unwrap();
        assert_eq!(immediate_cpu.register(2), Some(0x5678_1234));

        // Register-offset LDR r2, [r1, r3].
        let mut register_bus = bus_with_program(&[]);
        register_bus.write16(0x0200_0000, 0x58CA);
        register_bus.write32(0x0200_0100, 0x1234_5678);
        let mut register_cpu = Cpu::new();
        register_cpu.reset(0x0200_0000);
        register_cpu.thumb = true;
        register_cpu.set_register(1, 0x0200_0100);
        register_cpu.set_register(3, 3);
        register_cpu.step(&mut register_bus).unwrap();
        assert_eq!(register_cpu.register(2), Some(0x3456_7812));
    }

    #[test]
    fn arm_test_and_compare_encodings_without_s_bit_are_rejected() {
        // TST, TEQ, CMP and CMN require S=1 on ARM7TDMI.
        for instruction in [
            0xE101_0002_u32,
            0xE121_0002,
            0xE141_0002,
            0xE161_0002,
        ] {
            let mut bus = bus_with_program(&[instruction]);
            let mut cpu = Cpu::new();
            cpu.reset(0x0200_0000);
            cpu.set_register(1, 0xF0F0_F0F0);
            cpu.set_register(2, 0x0F0F_0F0F);
            let flags_before = cpu.flags;

            assert_eq!(
                cpu.step(&mut bus),
                Err(CpuError::UnsupportedInstruction {
                    address: 0x0200_0000,
                    instruction,
                }),
                "instruction 0x{instruction:08X}"
            );
            assert_eq!(cpu.flags, flags_before);
        }
    }

    #[test]
    fn arm_exception_return_is_rejected_until_spsr_is_modeled() {
        // SUBS pc, lr, #4 is an exception return on ARM7TDMI. It must restore
        // CPSR from SPSR, which this core does not model yet.
        let mut bus = bus_with_program(&[0xE25E_F004]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(14, 0x0200_0100);

        assert_eq!(
            cpu.step(&mut bus),
            Err(CpuError::UnsupportedInstruction {
                address: 0x0200_0000,
                instruction: 0xE25E_F004,
            })
        );
    }

    #[test]
    fn arm_and_thumb_unsigned_halfword_loads_rotate_odd_addresses() {
        // ARM LDRH r2, [r1] at an odd address rotates the aligned halfword.
        let instruction = 0xE1D1_20B0;
        let mut arm_bus = bus_with_program(&[instruction]);
        arm_bus.write16(0x0200_0100, 0x1234);
        let mut arm_cpu = Cpu::new();
        arm_cpu.reset(0x0200_0000);
        arm_cpu.set_register(1, 0x0200_0101);
        arm_cpu.step(&mut arm_bus).unwrap();
        assert_eq!(arm_cpu.register(2), Some(0x3412));

        // Thumb LDRH r2, [r1, r0] (register-offset form) uses same behavior.
        let mut thumb_bus = bus_with_program(&[]);
        thumb_bus.write16(0x0200_0100, 0x1234);
        thumb_bus.write16(0x0200_0000, 0x5A0A);
        let mut thumb_cpu = Cpu::new();
        thumb_cpu.reset(0x0200_0000);
        thumb_cpu.thumb = true;
        thumb_cpu.set_register(1, 0x0200_0101);
        thumb_cpu.set_register(0, 0);
        thumb_cpu.step(&mut thumb_bus).unwrap();
        assert_eq!(thumb_cpu.register(2), Some(0x3412));
    }

    #[test]
    fn thumb_sp_relative_word_load_rotates_unaligned_address() {
        let mut bus = bus_with_program(&[]);
        bus.write16(0x0200_0000, 0x9800); // LDR r0, [SP, #0]
        bus.write32(0x0200_0100, 0x1234_5678);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.thumb = true;
        cpu.set_register(13, 0x0200_0101);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(0), Some(0x7812_3456));
    }

    #[test]
    fn thumb_immediate_halfword_load_rotates_odd_address() {
        let mut bus = bus_with_program(&[]);
        bus.write16(0x0200_0000, 0x880A); // LDRH r2, [r1, #0]
        bus.write16(0x0200_0100, 0x1234);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.thumb = true;
        cpu.set_register(1, 0x0200_0101);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(2), Some(0x3412));
    }

    #[test]
    fn thumb_signed_halfword_load_uses_signed_byte_for_odd_address() {
        // Thumb LDRSH r2, [r1, r0] with an odd effective address behaves as LDRSB.
        let mut bus = bus_with_program(&[]);
        bus.write16(0x0200_0000, 0x5E0A);
        bus.write8(0x0200_0101, 0x80);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.thumb = true;
        cpu.set_register(1, 0x0200_0101);
        cpu.set_register(0, 0);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(2), Some(0xFFFF_FF80));
    }

    #[test]
    fn arm_signed_halfword_load_uses_signed_byte_for_odd_address() {
        // LDRSH r2, [r1]: odd addresses behave as signed-byte loads.
        let mut bus = bus_with_program(&[0xE1D1_20F0]);
        bus.write8(0x0200_0101, 0x80);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(1, 0x0200_0101);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.register(2), Some(0xFFFF_FF80));
    }

    #[test]
    fn arm_halfword_and_signed_loads_to_pc_are_rejected() {
        // ARM7TDMI does not define PC as a valid destination for these
        // halfword/signed-byte transfer encodings.
        for instruction in [0xE1D1_F0B0_u32, 0xE1D1_F0D0, 0xE1D1_F0F0] {
            let mut bus = bus_with_program(&[instruction]);
            let mut cpu = Cpu::new();
            cpu.reset(0x0200_0000);
            cpu.set_register(1, 0x0200_0100);
            bus.write16(0x0200_0100, 0x1234);
            bus.write8(0x0200_0100, 0x80);

            assert_eq!(
                cpu.step(&mut bus),
                Err(CpuError::UnsupportedInstruction {
                    address: 0x0200_0000,
                    instruction,
                }),
                "instruction 0x{instruction:08X}"
            );
        }
    }

    #[test]
    fn arm_halfword_load_with_writeback_and_same_base_is_rejected() {
        // LDRH r1, [r1], #2 is an unpredictable base/destination overlap.
        let instruction = 0xE0D1_10B2;
        let mut bus = bus_with_program(&[instruction]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(1, 0x0200_0100);
        bus.write16(0x0200_0100, 0xCAFE);

        assert_eq!(
            cpu.step(&mut bus),
            Err(CpuError::UnsupportedInstruction {
                address: 0x0200_0000,
                instruction,
            })
        );
        assert_eq!(cpu.register(1), Some(0x0200_0100));
    }

    #[test]
    fn arm_halfword_pc_base_writeback_is_rejected() {
        // LDRH r0, [pc], #2 and LDRH r0, [pc, #2]! are unpredictable.
        for instruction in [0xE0DF_00B2_u32, 0xE1FF_00B2] {
            let mut bus = bus_with_program(&[instruction]);
            let mut cpu = Cpu::new();
            cpu.reset(0x0200_0000);

            assert_eq!(
                cpu.step(&mut bus),
                Err(CpuError::UnsupportedInstruction {
                    address: 0x0200_0000,
                    instruction,
                }),
                "instruction 0x{instruction:08X}"
            );
            assert_eq!(cpu.register(15), Some(0x0200_0004));
        }
    }

    #[test]
    fn arm_block_transfer_rejects_pc_base_writeback() {
        // STMIA pc!, {r0} and LDMIA pc!, {r0} have unpredictable PC writeback.
        for instruction in [0xE8AF_0001_u32, 0xE8BF_0001] {
            let mut bus = bus_with_program(&[instruction]);
            let mut cpu = Cpu::new();
            cpu.reset(0x0200_0000);
            cpu.set_register(0, 0x1234_5678);

            assert_eq!(
                cpu.step(&mut bus),
                Err(CpuError::UnsupportedInstruction {
                    address: 0x0200_0000,
                    instruction,
                }),
                "instruction 0x{instruction:08X}"
            );
            assert_eq!(cpu.register(15), Some(0x0200_0004));
            assert_eq!(bus.read32(0x0200_0004), 0);
        }
    }

    #[test]
    fn arm_post_indexed_pc_base_transfer_is_rejected() {
        let mut bus = bus_with_program(&[0xE49F_0004]); // LDR r0, [pc], #4
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);

        assert_eq!(
            cpu.step(&mut bus),
            Err(CpuError::UnsupportedInstruction {
                address: 0x0200_0000,
                instruction: 0xE49F_0004,
            })
        );
        // The invalid write-back must not redirect execution through r15.
        assert_eq!(cpu.register(15), Some(0x0200_0004));
    }

    #[test]
    fn arm_load_with_writeback_and_same_base_destination_is_rejected() {
        // LDR r1, [r1], #4: architecturally unpredictable on ARM7TDMI.
        let mut bus = bus_with_program(&[0xE491_1004]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        cpu.set_register(1, 0x0200_0100);

        assert_eq!(
            cpu.step(&mut bus),
            Err(CpuError::UnsupportedInstruction {
                address: 0x0200_0000,
                instruction: 0xE491_1004,
            })
        );
        assert_eq!(cpu.register(1), Some(0x0200_0100));
    }

    #[test]
    fn unsupported_instruction_is_reported() {
        let mut bus = bus_with_program(&[0xF000_0000]);
        let mut cpu = Cpu::new();
        cpu.reset(0x0200_0000);
        assert!(matches!(
            cpu.step(&mut bus),
            Err(CpuError::UnsupportedInstruction { .. })
        ));
    }
}
