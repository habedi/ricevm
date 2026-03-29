use crate::opcode::Opcode;
use crate::types::Word;

/// Addressing mode for the source or destination operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AddressMode {
    OffsetIndirectMp = 0,
    OffsetIndirectFp = 1,
    Immediate = 2,
    None = 3,
    OffsetDoubleIndirectMp = 4,
    OffsetDoubleIndirectFp = 5,
    Reserved1 = 6,
    Reserved2 = 7,
}

/// Addressing mode for the middle operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MiddleMode {
    None = 0,
    SmallImmediate = 1,
    SmallOffsetFp = 2,
    SmallOffsetMp = 3,
}

/// A decoded source or destination operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Operand {
    pub mode: AddressMode,
    pub register1: Word,
    /// Only used for double-indirect modes.
    pub register2: Word,
}

/// A decoded middle operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MiddleOperand {
    pub mode: MiddleMode,
    pub register1: Word,
}

/// A single decoded Dis instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    pub opcode: Opcode,
    pub source: Operand,
    pub middle: MiddleOperand,
    pub destination: Operand,
}

impl Operand {
    pub const UNUSED: Self = Self {
        mode: AddressMode::None,
        register1: 0,
        register2: 0,
    };
}

impl MiddleOperand {
    pub const UNUSED: Self = Self {
        mode: MiddleMode::None,
        register1: 0,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unused_operands() {
        assert_eq!(Operand::UNUSED.mode, AddressMode::None);
        assert_eq!(MiddleOperand::UNUSED.mode, MiddleMode::None);
    }

    #[test]
    fn instruction_construction() {
        let inst = Instruction {
            opcode: Opcode::Addw,
            source: Operand {
                mode: AddressMode::OffsetIndirectFp,
                register1: 4,
                register2: 0,
            },
            middle: MiddleOperand::UNUSED,
            destination: Operand {
                mode: AddressMode::OffsetIndirectFp,
                register1: 8,
                register2: 0,
            },
        };
        assert_eq!(inst.opcode, Opcode::Addw);
    }
}
