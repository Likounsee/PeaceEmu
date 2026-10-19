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

/// Errors for ARM instructions that are not implemented yet.
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
                "unsupported ARM instruction 0x{instruction:08X} at 0x{address:08X}"
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
        }
    }

    /// Resets all registers and flags and sets the initial program counter.
    pub fn reset(&mut self, entry_point: u32) {
        self.registers = [0; 16];
        self.registers[15] = entry_point & !3;
        self.flags = ConditionFlags::default();
    }

    /// Returns a general-purpose register, including r15 as the current PC.
    pub fn register(&self, index: usize) -> Option<u32> {
        self.registers.get(index).copied()
    }

    /// Sets a general-purpose register. Writes to r15 are word-aligned.
    pub fn set_register(&mut self, index: usize, value: u32) -> bool {
        if index >= self.registers.len() {
            return false;
        }
        self.registers[index] = if index == 15 { value & !3 } else { value };
        true
    }

    /// Returns the modeled CPSR condition flags.
    pub fn flags(&self) -> ConditionFlags {
        self.flags
    }

    /// Executes one ARM-state instruction from the current program counter.
    ///
    /// The CPU advances the PC by four bytes before executing the operation.
    /// This first implementation is instruction-functional, not cycle-accurate,
    /// and does not implement Thumb state, exceptions, banked registers, or SPSR.
    pub fn step(&mut self, bus: &mut MemoryBus) -> Result<StepOutcome, CpuError> {
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

        if instruction & 0x0E00_0000 == 0x0A00_0000 {
            self.execute_branch(address, instruction);
        } else if instruction & 0x0C00_0000 == 0x0400_0000 {
            self.execute_single_transfer(address, instruction, bus)?;
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

    fn execute_data_processing(&mut self, address: u32, instruction: u32) -> Result<(), CpuError> {
        // Multiply and miscellaneous encodings also occupy this major opcode.
        if instruction & 0x0FC0_00F0 == 0x0000_0090 {
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
        }

        let immediate = instruction & (1 << 25) != 0;
        let opcode = (instruction >> 21) & 0xF;
        let set_flags = instruction & (1 << 20) != 0;
        let rn = ((instruction >> 16) & 0xF) as usize;
        let rd = ((instruction >> 12) & 0xF) as usize;
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
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
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
                bus.read32(effective)
            };
            self.registers[rd] = if rd == 15 { value & !3 } else { value };
        } else if byte_transfer {
            bus.write8(effective, self.registers[rd] as u8);
        } else {
            bus.write32(effective, self.registers[rd]);
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
