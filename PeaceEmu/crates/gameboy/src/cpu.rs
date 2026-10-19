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

        if instruction & 0x0FC0_00F0 == 0x0000_0090 {
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

    fn execute_multiply(&mut self, address: u32, instruction: u32) -> Result<(), CpuError> {
        let accumulate = instruction & (1 << 21) != 0;
        let set_flags = instruction & (1 << 20) != 0;
        let rd = ((instruction >> 16) & 0xF) as usize;
        let rn = ((instruction >> 12) & 0xF) as usize;
        let rs = ((instruction >> 8) & 0xF) as usize;
        let rm = (instruction & 0xF) as usize;

        // ARM7TDMI multiply operands and destination must not use r15.
        if rd == 15 || rs == 15 || rm == 15 || (accumulate && rn == 15) {
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

        // User-bank transfers require exception/banked-register support, and an
        // empty list has special architectural behavior not modeled here.
        if user_registers || register_list == 0 {
            return Err(CpuError::UnsupportedInstruction {
                address,
                instruction,
            });
        }

        let count = register_list.count_ones();
        let base = self.read_operand_register(rn, address);
        let bytes = count * 4;
        let first_address = match (increment, pre_index) {
            (true, false) => base,
            (true, true) => base.wrapping_add(4),
            (false, false) => base.wrapping_sub(bytes).wrapping_add(4),
            (false, true) => base.wrapping_sub(bytes),
        };

        let mut transfer_address = first_address;
        for register in 0..16 {
            if register_list & (1 << register) == 0 {
                continue;
            }

            if load {
                let value = bus.read32(transfer_address);
                self.registers[register] = if register == 15 { value & !3 } else { value };
            } else {
                let value = if register == 15 {
                    address.wrapping_add(12)
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
                1 => u32::from(bus.read16(effective)),
                2 => bus.read8(effective) as i8 as i32 as u32,
                3 => bus.read16(effective) as i16 as i32 as u32,
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
