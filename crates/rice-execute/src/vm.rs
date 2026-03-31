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

/// A Dis module loaded from a .dis file at runtime.
pub(crate) struct LoadedModule {
    pub module: ricevm_core::Module,
    pub mp: Vec<u8>,
}

pub(crate) struct VmState<'m> {
    pub module: &'m Module,
    pub mp: Vec<u8>,
    pub frames: FrameStack,
    pub heap: Heap,
    pub modules: ModuleRegistry,
    pub loaded_modules: Vec<LoadedModule>,
    pub files: crate::filetab::FileTable,
    pub pc: usize,
    pub next_pc: usize,
    pub halted: bool,
    pub trace: bool,
    pub gc_enabled: bool,
    pub(crate) gc_counter: usize,
    /// Index of the currently executing loaded module (None = main module).
    pub(crate) current_loaded_module: Option<usize>,
    /// Inferno root directory for path resolution (empty = use host paths directly).
    pub(crate) root_path: String,

    // Resolved operand targets for the current instruction.
    pub src: AddrTarget,
    pub mid: AddrTarget,
    pub dst: AddrTarget,

    // Scratch storage for immediate values.
    pub imm_src: Word,
    pub imm_mid: Word,
    pub imm_dst: Word,

    // Heap array reference table for indx results.
    pub heap_refs: Vec<(heap::HeapId, usize)>,
}

impl<'m> VmState<'m> {
    pub fn new(module: &'m Module) -> Result<Self, ExecError> {
        Self::with_args(module, Vec::new())
    }

    pub fn with_args(module: &'m Module, args: Vec<String>) -> Result<Self, ExecError> {
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

        // Set up entry frame args for the Limbo init() convention:
        // fp[32] = ref Draw->Context
        // fp[36] = list of string (program name)
        if frame_size >= 40 {
            // Create Draw->Context record:
            //   offset 0: ref Display (pointer)
            //   offset 4: ref Screen (pointer)
            //   offset 8: wm channel (pointer)
            let mut ctx_data = vec![0u8; 16];
            // Create a Display record (simplified)
            let display_id = heap.alloc(0, heap::HeapData::Record(vec![0u8; 32]));
            memory::write_word(&mut ctx_data, 0, display_id as i32);
            // Create a Screen record
            let screen_id = heap.alloc(0, heap::HeapData::Record(vec![0u8; 16]));
            memory::write_word(&mut ctx_data, 4, screen_id as i32);
            // Create a wm channel
            let wm_chan = heap.alloc(
                0,
                heap::HeapData::Channel {
                    elem_size: 4,
                    pending: None,
                },
            );
            memory::write_word(&mut ctx_data, 8, wm_chan as i32);
            let ctx_id = heap.alloc(0, heap::HeapData::Record(ctx_data));

            // Build args list: [module_name, arg1, arg2, ...] as a linked list.
            // Construct in reverse so the list is in forward order.
            let mut all_args = vec![module.name.clone()];
            all_args.extend(args.iter().cloned());

            let mut args_list = heap::NIL;
            for arg in all_args.iter().rev() {
                let str_id = heap.alloc(0, heap::HeapData::Str(arg.clone()));
                let mut head_buf = vec![0u8; 4];
                memory::write_word(&mut head_buf, 0, str_id as i32);
                if args_list != heap::NIL {
                    heap.inc_ref(args_list);
                }
                args_list = heap.alloc(
                    0,
                    heap::HeapData::List {
                        head: head_buf,
                        tail: args_list,
                    },
                );
            }

            let fp_base = frames.current_data_offset();
            // fp[32] = Draw->Context
            if fp_base + 36 <= frames.data.len() {
                memory::write_word(&mut frames.data, fp_base + 32, ctx_id as i32);
            }
            // fp[36] = args list
            if fp_base + 40 <= frames.data.len() {
                memory::write_word(&mut frames.data, fp_base + 36, args_list as i32);
            }
        }

        let mut modules = ModuleRegistry::new();
        modules.register(sys::create_sys_module());
        modules.register(crate::math::create_math_module());
        modules.register(crate::draw::create_draw_module());
        modules.register(crate::tk::create_tk_module());

        let trace = std::env::var("RICEVM_TRACE").is_ok();
        let gc_enabled = std::env::var("RICEVM_NO_GC").is_err();
        let root_path = std::env::var("RICEVM_ROOT").unwrap_or_default();

        Ok(Self {
            module,
            mp,
            frames,
            heap,
            modules,
            loaded_modules: Vec::new(),
            files: crate::filetab::FileTable::with_root(root_path.clone()),
            pc: module.header.entry_pc as usize,
            next_pc: 0,
            halted: false,
            trace,
            gc_enabled,
            gc_counter: 0,
            current_loaded_module: None,
            root_path,
            src: AddrTarget::None,
            mid: AddrTarget::None,
            dst: AddrTarget::None,
            imm_src: 0,
            imm_mid: 0,
            imm_dst: 0,
            heap_refs: Vec::new(),
        })
    }

    /// Get the type descriptor size for the currently executing module.
    pub(crate) fn current_type_size(&self, type_idx: usize) -> Option<usize> {
        if let Some(lm_idx) = self.current_loaded_module {
            self.loaded_modules
                .get(lm_idx)
                .and_then(|lm| lm.module.types.get(type_idx))
                .map(|td| td.size as usize)
        } else {
            self.module.types.get(type_idx).map(|td| td.size as usize)
        }
    }

    pub fn run(&mut self) -> Result<(), ExecError> {
        const GC_INTERVAL: usize = 10_000;

        while !self.halted {
            if self.pc >= self.module.code.len() {
                return Err(ExecError::InvalidPc(self.pc as Pc));
            }
            let inst = self.module.code[self.pc].clone();
            if self.trace {
                self.trace_instruction(&inst);
            }
            self.resolve_operands(&inst)?;
            self.next_pc = self.pc + 1;
            ops::dispatch(self, &inst)?;
            self.pc = self.next_pc;

            // Periodic GC
            if self.gc_enabled {
                self.gc_counter += 1;
                if self.gc_counter >= GC_INTERVAL {
                    self.gc_counter = 0;
                    crate::gc::collect(
                        &mut self.heap,
                        &self.frames,
                        &self.mp,
                        &self.loaded_modules,
                    );
                }
            }
        }
        Ok(())
    }

    pub(crate) fn trace_instruction(&self, inst: &Instruction) {
        use ricevm_core::{AddressMode, MiddleMode};
        let mut parts = Vec::new();
        parts.push(format!("{:4}: {:?}", self.pc, inst.opcode));

        if inst.source.mode != AddressMode::None {
            parts.push(format_operand("src", &inst.source));
        }
        if inst.middle.mode != MiddleMode::None {
            parts.push(format_mid("mid", &inst.middle));
        }
        if inst.destination.mode != AddressMode::None {
            parts.push(format_operand("dst", &inst.destination));
        }
        eprintln!("{}", parts.join(" "));
    }

    pub(crate) fn resolve_operands(&mut self, inst: &Instruction) -> Result<(), ExecError> {
        let fp_base = self.frames.current_data_offset();

        self.imm_src = inst.source.register1;
        self.src = address::resolve_operand(
            &inst.source,
            fp_base,
            &self.frames.data,
            &self.mp,
            &self.heap_refs,
        )?;

        self.imm_mid = inst.middle.register1;
        self.mid = address::resolve_middle(&inst.middle, fp_base)?;

        self.imm_dst = inst.destination.register1;
        self.dst = address::resolve_operand(
            &inst.destination,
            fp_base,
            &self.frames.data,
            &self.mp,
            &self.heap_refs,
        )?;

        Ok(())
    }

    // --- Value read helpers ---

    /// Get a read-only copy of bytes from a heap object at the given offset.
    pub(crate) fn heap_slice(
        &self,
        id: heap::HeapId,
        offset: usize,
        len: usize,
    ) -> Option<Vec<u8>> {
        let obj = self.heap.get(id)?;
        match &obj.data {
            heap::HeapData::Array { data, .. }
            | heap::HeapData::Record(data)
            | heap::HeapData::Adt { data, .. } => {
                if offset + len <= data.len() {
                    Some(data[offset..offset + len].to_vec())
                } else {
                    Some(vec![0u8; len])
                }
            }
            _ => Some(vec![0u8; len]),
        }
    }

    /// Write bytes to a heap array element.
    fn heap_write(&mut self, id: heap::HeapId, offset: usize, bytes: &[u8]) {
        if let Some(obj) = self.heap.get_mut(id) {
            match &mut obj.data {
                heap::HeapData::Array { data, .. }
                | heap::HeapData::Record(data)
                | heap::HeapData::Adt { data, .. } => {
                    if offset + bytes.len() <= data.len() {
                        data[offset..offset + bytes.len()].copy_from_slice(bytes);
                    }
                }
                _ => {}
            }
        }
    }

    pub(crate) fn read_word_at(&self, target: AddrTarget, imm: Word) -> Result<Word, ExecError> {
        match target {
            AddrTarget::Frame(off) => Ok(memory::read_word(&self.frames.data, off)),
            AddrTarget::Mp(off) => Ok(memory::read_word(&self.mp, off)),
            AddrTarget::Immediate => Ok(imm),
            AddrTarget::None => Ok(0),
            AddrTarget::HeapArray { id, offset } => Ok(self
                .heap_slice(id, offset, 4)
                .map(|b| memory::read_word(&b, 0))
                .unwrap_or(0)),
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
            AddrTarget::HeapArray { id, offset } => {
                let mut buf = [0u8; 4];
                memory::write_word(&mut buf, 0, val);
                self.heap_write(id, offset, &buf);
                Ok(())
            }
        }
    }

    pub(crate) fn read_big_at(&self, target: AddrTarget, imm: Word) -> Result<Big, ExecError> {
        match target {
            AddrTarget::Frame(off) => Ok(memory::read_big(&self.frames.data, off)),
            AddrTarget::Mp(off) => Ok(memory::read_big(&self.mp, off)),
            AddrTarget::Immediate => Ok(imm as Big),
            AddrTarget::None => Ok(0),
            AddrTarget::HeapArray { id, offset } => Ok(self
                .heap_slice(id, offset, 8)
                .map(|b| memory::read_big(&b, 0))
                .unwrap_or(0)),
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
            AddrTarget::HeapArray { id, offset } => {
                let mut buf = [0u8; 8];
                memory::write_big(&mut buf, 0, val);
                self.heap_write(id, offset, &buf);
                Ok(())
            }
        }
    }

    pub(crate) fn read_real_at(&self, target: AddrTarget, _imm: Word) -> Result<Real, ExecError> {
        match target {
            AddrTarget::Frame(off) => Ok(memory::read_real(&self.frames.data, off)),
            AddrTarget::Mp(off) => Ok(memory::read_real(&self.mp, off)),
            AddrTarget::Immediate => Ok(0.0),
            AddrTarget::None => Ok(0.0),
            AddrTarget::HeapArray { id, offset } => Ok(self
                .heap_slice(id, offset, 8)
                .map(|b| memory::read_real(&b, 0))
                .unwrap_or(0.0)),
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
            AddrTarget::HeapArray { id, offset } => {
                let mut buf = [0u8; 8];
                memory::write_real(&mut buf, 0, val);
                self.heap_write(id, offset, &buf);
                Ok(())
            }
        }
    }

    pub(crate) fn read_byte_at(&self, target: AddrTarget, imm: Word) -> Result<Byte, ExecError> {
        match target {
            AddrTarget::Frame(off) => Ok(memory::read_byte(&self.frames.data, off)),
            AddrTarget::Mp(off) => Ok(memory::read_byte(&self.mp, off)),
            AddrTarget::Immediate => Ok(imm as Byte),
            AddrTarget::None => Ok(0),
            AddrTarget::HeapArray { id, offset } => {
                Ok(self.heap_slice(id, offset, 1).map(|b| b[0]).unwrap_or(0))
            }
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
            AddrTarget::HeapArray { id, offset } => {
                self.heap_write(id, offset, &[val]);
                Ok(())
            }
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
    #[allow(dead_code)]
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

fn format_operand(label: &str, op: &ricevm_core::Operand) -> String {
    use ricevm_core::AddressMode;
    match op.mode {
        AddressMode::OffsetIndirectFp => format!("{label}={}(fp)", op.register1),
        AddressMode::OffsetIndirectMp => format!("{label}={}(mp)", op.register1),
        AddressMode::Immediate => format!("{label}=${}", op.register1),
        AddressMode::None => String::new(),
        AddressMode::OffsetDoubleIndirectFp => {
            format!("{label}={}({}(fp))", op.register2, op.register1)
        }
        AddressMode::OffsetDoubleIndirectMp => {
            format!("{label}={}({}(mp))", op.register2, op.register1)
        }
        AddressMode::Reserved1 | AddressMode::Reserved2 => format!("{label}=reserved"),
    }
}

fn format_mid(label: &str, op: &ricevm_core::MiddleOperand) -> String {
    use ricevm_core::MiddleMode;
    match op.mode {
        MiddleMode::None => String::new(),
        MiddleMode::SmallImmediate => format!("{label}=${}", op.register1),
        MiddleMode::SmallOffsetFp => format!("{label}={}(fp)", op.register1),
        MiddleMode::SmallOffsetMp => format!("{label}={}(mp)", op.register1),
    }
}
