//! VM execution state and main loop.

use ricevm_core::{Big, Byte, ExecError, Instruction, Module, Pc, Real, Word};

use crate::address::{self, AddrTarget};
use crate::builtin::ModuleRegistry;
use crate::data;
use crate::frame::FrameStack;
use crate::heap::{self, Heap, HeapId};
use crate::memory;
use crate::ops;
use crate::sys;

pub(crate) struct VmState<'m> {
    pub module: &'m Module,
    pub mp: Vec<u8>,
    pub frames: FrameStack,
    pub heap: Heap,
    pub modules: ModuleRegistry,
    pub pc: usize,
    pub next_pc: usize,
    pub halted: bool,

    // Resolved operand targets for the current instruction.
    pub src: AddrTarget,
    pub mid: AddrTarget,
    pub dst: AddrTarget,

    // Scratch storage for immediate values.
    pub imm_src: Word,
    pub imm_mid: Word,
    pub imm_dst: Word,
}

impl<'m> VmState<'m> {
    pub fn new(module: &'m Module) -> Result<Self, ExecError> {
        let mut heap = Heap::new();
        let mp = data::init_mp(module.header.data_size as usize, &module.data, &mut heap);
        let mut frames = FrameStack::new();

        // Push the initial entry frame.
        let entry_type_idx = module.header.entry_type;
        let frame_size = if entry_type_idx >= 0 && (entry_type_idx as usize) < module.types.len() {
            module.types[entry_type_idx as usize].size as usize
        } else {
            // No type descriptor: use a default frame size.
            64
        };
        frames.push_entry(frame_size, -1); // -1 sentinel for "no caller"

        let mut modules = ModuleRegistry::new();
        modules.register(sys::create_sys_module());

        Ok(Self {
            module,
            mp,
            frames,
            heap,
            modules,
            pc: module.header.entry_pc as usize,
            next_pc: 0,
            halted: false,
            src: AddrTarget::None,
            mid: AddrTarget::None,
            dst: AddrTarget::None,
            imm_src: 0,
            imm_mid: 0,
            imm_dst: 0,
        })
    }

    pub fn run(&mut self) -> Result<(), ExecError> {
        while !self.halted {
            if self.pc >= self.module.code.len() {
                return Err(ExecError::InvalidPc(self.pc as Pc));
            }
            let inst = self.module.code[self.pc].clone();
            self.resolve_operands(&inst)?;
            self.next_pc = self.pc + 1;
            ops::dispatch(self, &inst)?;
            self.pc = self.next_pc;
        }
        Ok(())
    }

    fn resolve_operands(&mut self, inst: &Instruction) -> Result<(), ExecError> {
        let fp_base = self.frames.current_data_offset();

        self.imm_src = inst.source.register1;
        self.src = address::resolve_operand(&inst.source, fp_base, &self.frames.data, &self.mp)?;

        self.imm_mid = inst.middle.register1;
        self.mid = address::resolve_middle(&inst.middle, fp_base)?;

        self.imm_dst = inst.destination.register1;
        self.dst =
            address::resolve_operand(&inst.destination, fp_base, &self.frames.data, &self.mp)?;

        Ok(())
    }

    // --- Value read helpers ---

    pub(crate) fn read_word_at(&self, target: AddrTarget, imm: Word) -> Result<Word, ExecError> {
        match target {
            AddrTarget::Frame(off) => Ok(memory::read_word(&self.frames.data, off)),
            AddrTarget::Mp(off) => Ok(memory::read_word(&self.mp, off)),
            AddrTarget::Immediate => Ok(imm),
            AddrTarget::None => Ok(0),
        }
    }

    pub(crate) fn write_word_at(&mut self, target: AddrTarget, val: Word) -> Result<(), ExecError> {
        match target {
            AddrTarget::Frame(off) => {
                memory::write_word(&mut self.frames.data, off, val);
                Ok(())
            }
            AddrTarget::Mp(off) => {
                memory::write_word(&mut self.mp, off, val);
                Ok(())
            }
            AddrTarget::Immediate => Err(ExecError::Other("cannot write to immediate".to_string())),
            AddrTarget::None => Ok(()),
        }
    }

    pub(crate) fn read_big_at(&self, target: AddrTarget, imm: Word) -> Result<Big, ExecError> {
        match target {
            AddrTarget::Frame(off) => Ok(memory::read_big(&self.frames.data, off)),
            AddrTarget::Mp(off) => Ok(memory::read_big(&self.mp, off)),
            AddrTarget::Immediate => Ok(imm as Big),
            AddrTarget::None => Ok(0),
        }
    }

    pub(crate) fn write_big_at(&mut self, target: AddrTarget, val: Big) -> Result<(), ExecError> {
        match target {
            AddrTarget::Frame(off) => {
                memory::write_big(&mut self.frames.data, off, val);
                Ok(())
            }
            AddrTarget::Mp(off) => {
                memory::write_big(&mut self.mp, off, val);
                Ok(())
            }
            AddrTarget::Immediate => Err(ExecError::Other("cannot write to immediate".to_string())),
            AddrTarget::None => Ok(()),
        }
    }

    pub(crate) fn read_real_at(&self, target: AddrTarget, _imm: Word) -> Result<Real, ExecError> {
        match target {
            AddrTarget::Frame(off) => Ok(memory::read_real(&self.frames.data, off)),
            AddrTarget::Mp(off) => Ok(memory::read_real(&self.mp, off)),
            AddrTarget::Immediate => Ok(0.0),
            AddrTarget::None => Ok(0.0),
        }
    }

    pub(crate) fn write_real_at(&mut self, target: AddrTarget, val: Real) -> Result<(), ExecError> {
        match target {
            AddrTarget::Frame(off) => {
                memory::write_real(&mut self.frames.data, off, val);
                Ok(())
            }
            AddrTarget::Mp(off) => {
                memory::write_real(&mut self.mp, off, val);
                Ok(())
            }
            AddrTarget::Immediate => Err(ExecError::Other("cannot write to immediate".to_string())),
            AddrTarget::None => Ok(()),
        }
    }

    pub(crate) fn read_byte_at(&self, target: AddrTarget, imm: Word) -> Result<Byte, ExecError> {
        match target {
            AddrTarget::Frame(off) => Ok(memory::read_byte(&self.frames.data, off)),
            AddrTarget::Mp(off) => Ok(memory::read_byte(&self.mp, off)),
            AddrTarget::Immediate => Ok(imm as Byte),
            AddrTarget::None => Ok(0),
        }
    }

    pub(crate) fn write_byte_at(&mut self, target: AddrTarget, val: Byte) -> Result<(), ExecError> {
        match target {
            AddrTarget::Frame(off) => {
                memory::write_byte(&mut self.frames.data, off, val);
                Ok(())
            }
            AddrTarget::Mp(off) => {
                memory::write_byte(&mut self.mp, off, val);
                Ok(())
            }
            AddrTarget::Immediate => Err(ExecError::Other("cannot write to immediate".to_string())),
            AddrTarget::None => Ok(()),
        }
    }

    // --- Convenience accessors for src/mid/dst ---

    pub(crate) fn src_word(&self) -> Result<Word, ExecError> {
        self.read_word_at(self.src, self.imm_src)
    }
    pub(crate) fn mid_word(&self) -> Result<Word, ExecError> {
        self.read_word_at(self.mid, self.imm_mid)
    }
    pub(crate) fn dst_word(&self) -> Result<Word, ExecError> {
        self.read_word_at(self.dst, self.imm_dst)
    }
    pub(crate) fn set_dst_word(&mut self, val: Word) -> Result<(), ExecError> {
        self.write_word_at(self.dst, val)
    }

    pub(crate) fn src_big(&self) -> Result<Big, ExecError> {
        self.read_big_at(self.src, self.imm_src)
    }
    pub(crate) fn mid_big(&self) -> Result<Big, ExecError> {
        self.read_big_at(self.mid, self.imm_mid)
    }
    pub(crate) fn set_dst_big(&mut self, val: Big) -> Result<(), ExecError> {
        self.write_big_at(self.dst, val)
    }

    pub(crate) fn src_real(&self) -> Result<Real, ExecError> {
        self.read_real_at(self.src, self.imm_src)
    }
    pub(crate) fn mid_real(&self) -> Result<Real, ExecError> {
        self.read_real_at(self.mid, self.imm_mid)
    }
    pub(crate) fn dst_real(&self) -> Result<Real, ExecError> {
        self.read_real_at(self.dst, self.imm_dst)
    }
    pub(crate) fn set_dst_real(&mut self, val: Real) -> Result<(), ExecError> {
        self.write_real_at(self.dst, val)
    }

    pub(crate) fn src_byte(&self) -> Result<Byte, ExecError> {
        self.read_byte_at(self.src, self.imm_src)
    }
    pub(crate) fn mid_byte(&self) -> Result<Byte, ExecError> {
        self.read_byte_at(self.mid, self.imm_mid)
    }
    pub(crate) fn set_dst_byte(&mut self, val: Byte) -> Result<(), ExecError> {
        self.write_byte_at(self.dst, val)
    }

    // --- Pointer (HeapId) accessors ---

    pub(crate) fn src_ptr(&self) -> Result<HeapId, ExecError> {
        Ok(self.read_word_at(self.src, self.imm_src)? as HeapId)
    }
    pub(crate) fn mid_ptr(&self) -> Result<HeapId, ExecError> {
        Ok(self.read_word_at(self.mid, self.imm_mid)? as HeapId)
    }
    pub(crate) fn dst_ptr(&self) -> Result<HeapId, ExecError> {
        Ok(self.read_word_at(self.dst, self.imm_dst)? as HeapId)
    }
    pub(crate) fn set_dst_ptr(&mut self, id: HeapId) -> Result<(), ExecError> {
        self.write_word_at(self.dst, id as Word)
    }

    /// Move a pointer from one location to another with reference counting.
    /// Increments the new value's ref count and decrements the old value's ref count.
    pub(crate) fn move_ptr_to_dst(&mut self, new_id: HeapId) -> Result<(), ExecError> {
        let old_id = self.dst_ptr()?;
        if new_id != heap::NIL {
            self.heap.inc_ref(new_id);
        }
        self.set_dst_ptr(new_id)?;
        if old_id != heap::NIL {
            self.heap.dec_ref(old_id);
        }
        Ok(())
    }

    /// Read a pointer (HeapId) from an arbitrary absolute offset in the frame stack.
    #[allow(dead_code)]
    pub(crate) fn read_ptr_stack(&self, abs_offset: usize) -> HeapId {
        memory::read_word(&self.frames.data, abs_offset) as HeapId
    }

    /// Write a pointer (HeapId) to an arbitrary absolute offset in the frame stack.
    #[allow(dead_code)]
    pub(crate) fn write_ptr_stack(&mut self, abs_offset: usize, id: HeapId) {
        memory::write_word(&mut self.frames.data, abs_offset, id as Word);
    }
}
