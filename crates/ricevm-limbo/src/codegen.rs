//! Code generator: translates Limbo AST to Dis bytecode.
//!
//! Produces a `ricevm_core::Module` from a parsed AST.

use ricevm_core::{
    AddressMode, DataItem, ExportEntry, Header, ImportEntry, ImportModule, Instruction, MiddleMode,
    MiddleOperand, Module, Opcode, Operand, PointerMap, RuntimeFlags, TypeDescriptor, XMAGIC,
};

use crate::ast::*;
use crate::symtab::{ConstValue, Symbol, SymbolTable};
use crate::token::Span;

/// Value type tracking for selecting correct Dis opcodes.
#[derive(Clone, Copy, PartialEq)]
enum ValType {
    Word,
    Ptr,   // string, list, ref, module, channel
    Array, // array types (use Lena instead of Lenc)
}

/// Where a named variable lives. Function-locals are frame-relative; module
/// level variables live in the module's MP data area so every function in the
/// module (and every thread) sees the same storage.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Local(i32),
    Global(i32),
}

impl Slot {
    /// The Dis operand that addresses this slot.
    fn operand(self) -> Operand {
        match self {
            Slot::Local(off) => op_fp(off),
            Slot::Global(off) => op_mp(off),
        }
    }
}

/// A folded compile-time constant value.
#[derive(Clone, Debug, PartialEq)]
enum ConstVal {
    Int(i64),
    Real(f64),
    Str(String),
}

/// A breakable (and usually continuable) construct being generated. `break`
/// and `continue` emit an unpatched `Jmp` and record its index here; the
/// construct patches every recorded site once its exit and continue PCs are
/// known — the same patch-after-the-fact pattern used for `if`/`while`.
struct LoopFrame {
    /// Label attached to the construct, for `break label` / `continue label`.
    label: Option<String>,
    /// Code indices of jumps that must land on the construct's exit.
    breaks: Vec<usize>,
    /// Code indices of jumps that must land on the construct's continue point.
    continues: Vec<usize>,
    /// `case` statements are breakable but not continuable: a `continue`
    /// inside a case arm belongs to the enclosing loop.
    continuable: bool,
}

/// Numeric kind: selects 4-byte word, 8-byte big, or 8-byte real slots and
/// the corresponding Dis opcode family (Addw vs Addl vs Addf etc.). Ordering
/// is used for width promotion in mixed expressions via `.max()`: word < big
/// < real, so e.g. `word + real` promotes the temp widths to Real.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum NumKind {
    Word,
    Big,
    Real,
}

impl NumKind {
    fn byte_size(self) -> i32 {
        match self {
            NumKind::Word => 4,
            NumKind::Big | NumKind::Real => 8,
        }
    }
}

/// Map a Limbo type to a NumKind. Non-numeric types collapse to Word because
/// this enum is only consulted for slot sizing inside numeric paths; pointer
/// sites use `ValType::Ptr` separately.
fn type_num_kind(ty: &Type) -> NumKind {
    match ty {
        Type::Basic(BasicType::Big) => NumKind::Big,
        Type::Basic(BasicType::Real) => NumKind::Real,
        _ => NumKind::Word,
    }
}

/// Return the numeric kind of the value produced by a known `$Sys` builtin.
/// Default Word covers everything not in the lookup; the kind is used to
/// pick the right Mov/Cvt opcode when copying the return value out.
/// Extract the array element Type from a VarDecl, if any. Looks at the
/// explicit type annotation first, then the init expression's array forms
/// (`array[N] of T`, `array[] of {...}`, `array of byte stringExpr`).
fn decl_array_elem_type(v: &VarDecl) -> Option<Type> {
    if let Some(Type::Array(elem)) = &v.ty {
        return Some((**elem).clone());
    }
    match v.init.as_ref()? {
        Expr::ArrayAlloc(_, ty, _) | Expr::ArrayLit(_, Some(ty), _) => Some((**ty).clone()),
        Expr::Cast(ty, _, _) => match ty.as_ref() {
            Type::Array(elem) => Some((**elem).clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Reduce a Type to a BasicType when possible. Used by sites that pick a
/// kind-aware opcode pair from an element type.
fn type_basic(ty: &Type) -> Option<BasicType> {
    match ty {
        Type::Basic(b) => Some(*b),
        _ => None,
    }
}

/// Extract the channel element BasicType from a VarDecl, if any.
fn decl_chan_elem_basic(v: &VarDecl) -> Option<BasicType> {
    if let Some(Type::Chan(elem)) = &v.ty
        && let Type::Basic(b) = elem.as_ref()
    {
        return Some(*b);
    }
    if let Some(Expr::ChanAlloc(ty, _)) = v.init.as_ref()
        && let Type::Basic(b) = ty.as_ref()
    {
        return Some(*b);
    }
    None
}

/// Field width and alignment for ADT layout. Matches the reference Limbo
/// ABI: word/byte/ptr fields are 4-byte sized and 4-byte aligned, and
/// big/real are 8-byte sized with 8-byte alignment.
fn type_size_align(ty: &Type) -> (i32, i32) {
    match ty {
        Type::Basic(BasicType::Big) | Type::Basic(BasicType::Real) => (8, 8),
        // byte stored in a 4-byte slot in records (matches reference).
        _ => (4, 4),
    }
}

/// Compute an ADT's field layout: list of `(name, type, byte_offset)`.
fn compute_adt_layout(adt: &AdtDecl) -> Vec<(String, Type, i32)> {
    let mut fields = Vec::new();
    let mut off = 0i32;
    for member in &adt.members {
        if let AdtMember::Field(v) = member {
            let ty = match &v.ty {
                Some(t) => t.clone(),
                None => continue,
            };
            let (size, align) = type_size_align(&ty);
            // Round `off` up to alignment.
            off = (off + align - 1) & !(align - 1);
            for name in &v.names {
                fields.push((name.clone(), ty.clone(), off));
                off += size;
            }
        }
    }
    fields
}

/// Pick the Newc* opcode for a channel of the given element BasicType.
fn newc_opcode(basic: Option<BasicType>) -> Opcode {
    match basic {
        Some(BasicType::Byte) => Opcode::Newcb,
        Some(BasicType::Big) => Opcode::Newcl,
        Some(BasicType::Real) => Opcode::Newcf,
        Some(BasicType::String) => Opcode::Newcp,
        // int and unknown default to a 4-byte word channel.
        _ => Opcode::Newcw,
    }
}

fn sys_return_kind(name: &str) -> NumKind {
    match name {
        // big-returning sys builtins (per sys.m signatures)
        "seek" => NumKind::Big,
        _ => NumKind::Word,
    }
}

/// Pick the Mov opcode that moves a whole value of the given shape: 8-byte
/// big/real payloads use Movl/Movf, pointers use the ref-counting Movp, and
/// everything else is a plain 4-byte Movw.
fn mov_opcode(ty: ValType, kind: NumKind) -> Opcode {
    match (ty, kind) {
        (_, NumKind::Big) => Opcode::Movl,
        (_, NumKind::Real) => Opcode::Movf,
        (ValType::Word, NumKind::Word) => Opcode::Movw,
        _ => Opcode::Movp,
    }
}

impl From<&ConstValue> for ConstVal {
    fn from(value: &ConstValue) -> Self {
        match value {
            ConstValue::Int(v) => ConstVal::Int(*v),
            ConstValue::Real(v) => ConstVal::Real(*v),
            ConstValue::String(s) => ConstVal::Str(s.clone()),
        }
    }
}

impl ConstVal {
    /// The ValType a use of this constant produces.
    fn val_type(&self) -> ValType {
        match self {
            ConstVal::Str(_) => ValType::Ptr,
            _ => ValType::Word,
        }
    }

    /// The NumKind (slot width) a use of this constant produces.
    fn num_kind(&self) -> NumKind {
        match self {
            ConstVal::Real(_) => NumKind::Real,
            ConstVal::Int(v) if *v > i32::MAX as i64 || *v < i32::MIN as i64 => NumKind::Big,
            _ => NumKind::Word,
        }
    }
}

/// Supplies the value of `iota` while folding a sequence of `con`
/// declarations: it counts up within one declaration (whose names all carry
/// the same span) and restarts at the next declaration.
#[derive(Default)]
struct IotaCounter {
    group: Option<Span>,
    value: i64,
}

impl IotaCounter {
    fn next(&mut self, span: Span) -> i64 {
        if self.group != Some(span) {
            self.group = Some(span);
            self.value = 0;
        }
        let v = self.value;
        self.value += 1;
        v
    }
}

/// Fold a binary operation over two constant values.
fn fold_const_binary(lhs: &ConstVal, op: BinOp, rhs: &ConstVal) -> Result<ConstVal, String> {
    match (lhs, rhs) {
        (ConstVal::Str(a), ConstVal::Str(b)) if op == BinOp::Add => {
            Ok(ConstVal::Str(format!("{a}{b}")))
        }
        (ConstVal::Int(a), ConstVal::Int(b)) => {
            let (a, b) = (*a, *b);
            let v = match op {
                BinOp::Add => a.wrapping_add(b),
                BinOp::Sub => a.wrapping_sub(b),
                BinOp::Mul => a.wrapping_mul(b),
                BinOp::Div if b == 0 => return Err("constant division by zero".to_string()),
                BinOp::Div => a.wrapping_div(b),
                BinOp::Mod if b == 0 => return Err("constant division by zero".to_string()),
                BinOp::Mod => a.wrapping_rem(b),
                BinOp::Lshift => a.wrapping_shl(b as u32),
                BinOp::Rshift => a.wrapping_shr(b as u32),
                BinOp::And => a & b,
                BinOp::Or => a | b,
                BinOp::Xor => a ^ b,
                BinOp::Eq => (a == b) as i64,
                BinOp::Neq => (a != b) as i64,
                BinOp::Lt => (a < b) as i64,
                BinOp::Gt => (a > b) as i64,
                BinOp::Leq => (a <= b) as i64,
                BinOp::Geq => (a >= b) as i64,
                _ => return Err(format!("unsupported constant operator {op:?}")),
            };
            Ok(ConstVal::Int(v))
        }
        (ConstVal::Real(a), ConstVal::Real(b)) => {
            let (a, b) = (*a, *b);
            let v = match op {
                BinOp::Add => a + b,
                BinOp::Sub => a - b,
                BinOp::Mul => a * b,
                BinOp::Div => a / b,
                _ => return Err(format!("unsupported constant operator {op:?} on real")),
            };
            Ok(ConstVal::Real(v))
        }
        // Mixed int/real: promote the int operand.
        (ConstVal::Int(a), ConstVal::Real(_)) => {
            fold_const_binary(&ConstVal::Real(*a as f64), op, rhs)
        }
        (ConstVal::Real(_), ConstVal::Int(b)) => {
            fold_const_binary(lhs, op, &ConstVal::Real(*b as f64))
        }
        _ => Err(format!("unsupported constant operator {op:?}")),
    }
}

/// Code generation context.
pub struct CodeGen {
    code: Vec<Instruction>,
    types: Vec<TypeDescriptor>,
    data: Vec<DataItem>,
    mp_size: i32,
    string_pool: Vec<(String, i32)>,
    module_name: String,
    exports: Vec<ExportEntry>,
    imports: Vec<ImportModule>,
    sys_path_mp: i32,
    sys_mp_ref: i32,
    /// Local variable table: name -> (fp offset, ValType, NumKind).
    /// NumKind is Word for non-numeric locals (strings, arrays, refs); only
    /// big/real locals carry a widened kind that sizes the slot and picks
    /// the correct Mov/arith opcode family.
    locals: Vec<(String, i32, ValType, NumKind)>,
    next_local: i32,
    frame_size: i32,
    sys_funcs: Vec<(String, usize)>,
    /// Local function table: name -> (pc, frame_size, return NumKind).
    /// The return kind picks the right Mov/Cvt opcode when copying the
    /// callee's return value into the caller's slot.
    func_table: Vec<(String, i32, i32, NumKind)>,
    /// Sidecar map for array locals: name -> element Type.
    /// Storing the full Type (not just BasicType) lets us express nested
    /// types like `array of array of big` — `aa[i]` returns a slot whose
    /// element type is `array of big`, which the recursive Index handler
    /// then peels to compute the inner indexing.
    local_array_elem: std::collections::HashMap<String, Type>,
    /// Sidecar map for channel locals: name -> element BasicType. Lets
    /// Send/Recv size the data temp by the channel's element width and
    /// pick the right Newc* opcode at allocation time.
    local_chan_elem: std::collections::HashMap<String, BasicType>,
    /// Sidecar map for function return tuple shapes: name -> field Types.
    /// Lets `(a, b, c) := func()` allocate per-field locals at correct
    /// offsets and use kind-matched Mov for each field.
    func_tuple_ret: std::collections::HashMap<String, Vec<Type>>,
    /// Pending PC fixups for forward-referenced local calls and spawns.
    /// `(code_idx, callee_name)` — the destination operand of code[code_idx]
    /// will be patched to the callee's actual entry PC after all functions
    /// have been generated.
    pending_call_fixups: Vec<(usize, String)>,
    /// ADT layouts: ADT name -> ordered (field_name, field_type, byte_off).
    /// Built once from the AST so Dot/Arrow accesses can pick a kind-aware
    /// Mov opcode and the correct field offset instead of a heuristic.
    adt_layouts: std::collections::HashMap<String, Vec<(String, Type, i32)>>,
    /// Sidecar map for ADT-typed locals: local name -> ADT name. Lets Dot
    /// access resolve `local.field` to the right ADT layout.
    local_adt_type: std::collections::HashMap<String, String>,
    /// Frame sizes for each compiled function, in order.
    func_frames: Vec<i32>,
    /// Exception handlers for the module.
    handlers: Vec<ricevm_core::Handler>,
    /// Module-level variables: name -> (MP offset, ValType, NumKind). These
    /// are shared by every function in the module, unlike frame locals.
    globals: Vec<(String, i32, ValType, NumKind)>,
    /// Folded module-level constants: name -> value, or the reason the value
    /// could not be folded. Unfoldable constants are only an error if some
    /// expression actually uses them.
    module_consts: std::collections::HashMap<String, Result<ConstVal, String>>,
    /// Constants declared inside a `Mod: module { ... }` block, keyed
    /// `Mod->NAME`, so `Mod->NAME` use sites resolve.
    qualified_consts: std::collections::HashMap<String, Result<ConstVal, String>>,
    /// Symbols gathered from included `.m` files, when the driver supplies
    /// them. Consulted for constants that this file does not declare itself.
    symtab: Option<SymbolTable>,
    /// Module-level initialisers that are not compile-time constants. They
    /// are emitted at the top of the entry function, which runs before any
    /// other code in the module.
    pending_global_inits: Vec<(String, Expr)>,
    /// Enclosing breakable constructs, innermost last.
    loop_stack: Vec<LoopFrame>,
    /// Label of the statement currently being generated, consumed by the next
    /// loop/case construct so `break label` can find it.
    pending_label: Option<String>,
}

impl Default for CodeGen {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGen {
    pub fn new() -> Self {
        Self {
            code: Vec::new(),
            types: Vec::new(),
            data: Vec::new(),
            mp_size: 0,
            string_pool: Vec::new(),
            module_name: String::new(),
            exports: Vec::new(),
            imports: Vec::new(),
            sys_path_mp: -1,
            sys_mp_ref: -1,
            locals: Vec::new(),
            next_local: 40,
            frame_size: 80,
            sys_funcs: Vec::new(),
            func_table: Vec::new(),
            func_frames: Vec::new(),
            local_array_elem: std::collections::HashMap::new(),
            local_chan_elem: std::collections::HashMap::new(),
            pending_call_fixups: Vec::new(),
            adt_layouts: std::collections::HashMap::new(),
            local_adt_type: std::collections::HashMap::new(),
            func_tuple_ret: std::collections::HashMap::new(),
            handlers: Vec::new(),
            globals: Vec::new(),
            module_consts: std::collections::HashMap::new(),
            qualified_consts: std::collections::HashMap::new(),
            symtab: None,
            pending_global_inits: Vec::new(),
            loop_stack: Vec::new(),
            pending_label: None,
        }
    }

    /// Attach the symbol table built from the file's `include` directives so
    /// constants declared in `.m` interfaces resolve at their use sites.
    pub fn with_symtab(mut self, symtab: SymbolTable) -> Self {
        self.symtab = Some(symtab);
        self
    }

    pub fn compile(mut self, file: &SourceFile) -> Result<Module, String> {
        self.module_name = file
            .implement
            .first()
            .cloned()
            .unwrap_or_else(|| "Unknown".to_string());

        self.sys_path_mp = self.intern_string("$Sys");
        self.sys_mp_ref = self.alloc_mp(4);
        self.collect_strings(file);
        self.collect_adts(file);
        // Module-level `con` values are folded once, up front, so use sites
        // can resolve them to literals; module-level variables get real MP
        // storage so every function sees the same slot.
        self.collect_consts(file);
        self.collect_globals(file)?;
        self.imports.push(ImportModule { functions: vec![] });

        // Pre-scan to count functions and allocate type indices
        let funcs: Vec<&FuncDecl> = file
            .decls
            .iter()
            .filter_map(|d| if let Decl::Func(f) = d { Some(f) } else { None })
            .collect();

        // Pre-register every function name in func_table with a placeholder
        // PC. This lets `spawn func()` and `func()` calls resolve their
        // callee even when the callee is defined later in the source. The
        // actual PC is filled in by gen_func when it lays out the body.
        for func in &funcs {
            let full_name = if let Some(q) = &func.name.qualifier {
                format!("{q}.{}", func.name.name)
            } else {
                func.name.name.clone()
            };
            let ret_kind = func
                .sig
                .ret
                .as_ref()
                .map(type_num_kind)
                .unwrap_or(NumKind::Word);
            if let Some(Type::Tuple(fields)) = &func.sig.ret {
                self.func_tuple_ret
                    .insert(full_name.clone(), fields.clone());
            }
            // PC = -1 placeholder, frame_size = 0 placeholder.
            self.func_table.push((full_name, -1, 0, ret_kind));
        }

        // Generate code for each function. gen_func patches the matching
        // func_table entry's PC and frame_size.
        for func in &funcs {
            self.gen_func(func)?;
        }

        // Patch any forward-referenced Call/Spawn destinations now that
        // every function's entry PC is known.
        let fixups = std::mem::take(&mut self.pending_call_fixups);
        for (code_idx, name) in fixups {
            let pc = self
                .func_table
                .iter()
                .find(|(n, _, _, _)| n == &name)
                .map(|(_, pc, _, _)| *pc)
                .unwrap_or(-1);
            if pc < 0 {
                return Err(format!("unresolved forward reference to function `{name}`"));
            }
            self.code[code_idx].destination = op_imm(pc);
        }

        self.build_types();
        // The entry frame must be described by the *entry function's* type
        // descriptor. Using the last generated function's descriptor only
        // happened to work while every frame was sized to the cumulative
        // maximum; with per-function frame sizes it would under- or
        // over-allocate the entry frame.
        let entry_pc = self.exports.first().map(|e| e.pc).unwrap_or(0);
        let entry_type = match self.exports.first() {
            Some(e) => e.frame_type,
            // No `init`: entry_pc falls back to 0, which is the first
            // generated function, so use that function's descriptor.
            None if !self.func_frames.is_empty() => 2,
            None => 0,
        };
        let max_frame = self
            .func_frames
            .iter()
            .copied()
            .max()
            .unwrap_or(self.frame_size);

        Ok(Module {
            header: Header {
                magic: XMAGIC,
                signature: vec![],
                runtime_flags: RuntimeFlags(if self.handlers.is_empty() { 0x40 } else { 0x60 }),
                stack_extent: (max_frame + 256).max(480),
                code_size: self.code.len() as i32,
                data_size: self.mp_size,
                type_size: self.types.len() as i32,
                export_size: self.exports.len() as i32,
                entry_pc,
                entry_type,
            },
            code: self.code,
            types: self.types,
            data: self.data,
            name: self.module_name,
            exports: self.exports,
            imports: self.imports,
            handlers: self.handlers,
        })
    }

    fn alloc_mp(&mut self, size: i32) -> i32 {
        // Align to size boundary (4 for words, 8 for big/real)
        if size >= 8 {
            self.mp_size = (self.mp_size + 7) & !7;
        }
        let off = self.mp_size;
        self.mp_size += size;
        self.mp_size = (self.mp_size + 3) & !3;
        off
    }

    fn intern_string(&mut self, s: &str) -> i32 {
        if let Some((_, off)) = self.string_pool.iter().find(|(st, _)| st == s) {
            return *off;
        }
        let off = self.alloc_mp(4);
        self.data.push(DataItem::String {
            offset: off,
            value: s.to_string(),
        });
        self.string_pool.push((s.to_string(), off));
        off
    }

    fn alloc_local(&mut self, name: &str, ty: ValType, kind: NumKind) -> i32 {
        if let Some((_, off, _, _)) = self.locals.iter().find(|(n, _, _, _)| n == name) {
            return *off;
        }
        let off = self.next_local;
        self.next_local += kind.byte_size();
        self.grow_frame();
        self.locals.push((name.to_string(), off, ty, kind));
        off
    }

    fn get_local(&self, name: &str) -> Option<(i32, ValType)> {
        self.locals
            .iter()
            .find(|(n, _, _, _)| n == name)
            .map(|(_, o, t, _)| (*o, *t))
    }

    fn local_num_kind(&self, name: &str) -> NumKind {
        self.locals
            .iter()
            .find(|(n, _, _, _)| n == name)
            .map(|(_, _, _, k)| *k)
            .unwrap_or(NumKind::Word)
    }

    fn alloc_temp(&mut self) -> i32 {
        let off = self.next_local;
        self.next_local += 4;
        self.grow_frame();
        off
    }

    fn alloc_temp_for(&mut self, kind: NumKind) -> i32 {
        let off = self.next_local;
        self.next_local += kind.byte_size();
        self.grow_frame();
        off
    }

    fn grow_frame(&mut self) {
        if self.next_local > self.frame_size - 8 {
            self.frame_size = ((self.next_local + 24) + 7) & !7;
        }
    }

    fn ensure_sys_func(&mut self, name: &str) -> usize {
        if let Some((_, idx)) = self.sys_funcs.iter().find(|(n, _)| n == name) {
            return *idx;
        }
        let idx = self.imports[0].functions.len();
        self.imports[0].functions.push(ImportEntry {
            signature: 0,
            name: name.to_string(),
        });
        self.sys_funcs.push((name.to_string(), idx));
        idx
    }

    fn collect_strings(&mut self, file: &SourceFile) {
        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                for stmt in &func.body.stmts {
                    self.scan_stmt_strings(stmt);
                }
            }
        }
    }

    /// Walk top-level declarations and module-member declarations to record
    /// every ADT's field layout. Subsequent Dot/Arrow access can then
    /// resolve `obj.field` to the right offset and type.
    fn collect_adts(&mut self, file: &SourceFile) {
        for decl in &file.decls {
            match decl {
                Decl::Adt(adt) => {
                    self.adt_layouts
                        .insert(adt.name.clone(), compute_adt_layout(adt));
                }
                Decl::Module(m) => {
                    for member in &m.members {
                        if let ModuleMember::Adt(adt) = member {
                            self.adt_layouts
                                .insert(adt.name.clone(), compute_adt_layout(adt));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Fold every module-level `con` declaration into a literal value.
    ///
    /// Constants declared in the interface block of the module this file
    /// implements are in scope unqualified; constants from any module block
    /// are also recorded under `Mod->NAME` for qualified use sites.
    ///
    /// `iota` takes the value 0 for the first name of a `con` declaration, 1
    /// for the second, and so on, restarting at every declaration. The parser
    /// expands `Red, Green, Blue: con iota;` into one declaration per name,
    /// all sharing the declaration's span, so the span identifies the group.
    fn collect_consts(&mut self, file: &SourceFile) {
        for decl in &file.decls {
            let Decl::Module(m) = decl else { continue };
            let implemented = file.implement.first() == Some(&m.name);
            let mut iota = IotaCounter::default();
            for member in &m.members {
                let ModuleMember::Const(c) = member else {
                    continue;
                };
                let value = self.fold_const(&c.value, iota.next(c.span));
                self.qualified_consts
                    .insert(format!("{}->{}", m.name, c.name), value.clone());
                if implemented {
                    self.module_consts.insert(c.name.clone(), value);
                }
            }
        }

        let mut iota = IotaCounter::default();
        for decl in &file.decls {
            if let Decl::Const(c) = decl {
                let value = self.fold_const(&c.value, iota.next(c.span));
                self.module_consts.insert(c.name.clone(), value);
            }
        }
    }

    /// Evaluate a constant expression. `iota` supplies the value of the
    /// `iota` keyword for the declaration being folded.
    fn fold_const(&self, expr: &Expr, iota: i64) -> Result<ConstVal, String> {
        match expr {
            Expr::IntLit(v, _) => Ok(ConstVal::Int(*v)),
            Expr::CharLit(v, _) => Ok(ConstVal::Int(*v as i64)),
            Expr::RealLit(v, _) => Ok(ConstVal::Real(*v)),
            Expr::StringLit(s, _) => Ok(ConstVal::Str(s.clone())),
            Expr::Ident(name, _) if name == "iota" => Ok(ConstVal::Int(iota)),
            Expr::Ident(name, _) => self
                .const_value(name)
                .unwrap_or_else(|| Err(format!("`{name}` is not a constant"))),
            Expr::ModQual(module, member, _) => {
                let Expr::Ident(mod_name, _) = module.as_ref() else {
                    return Err("unsupported qualified constant".to_string());
                };
                self.qualified_const_value(mod_name, member)
                    .unwrap_or_else(|| Err(format!("`{mod_name}->{member}` is not a constant")))
            }
            Expr::Unary(op, inner, _) => {
                let v = self.fold_const(inner, iota)?;
                match (op, v) {
                    (UnaryOp::Neg, ConstVal::Int(n)) => Ok(ConstVal::Int(-n)),
                    (UnaryOp::Neg, ConstVal::Real(n)) => Ok(ConstVal::Real(-n)),
                    (UnaryOp::BitNot, ConstVal::Int(n)) => Ok(ConstVal::Int(!n)),
                    (UnaryOp::Not, ConstVal::Int(n)) => {
                        Ok(ConstVal::Int(if n == 0 { 1 } else { 0 }))
                    }
                    (op, _) => Err(format!("unsupported constant operator {op:?}")),
                }
            }
            Expr::Cast(ty, inner, _) => {
                let v = self.fold_const(inner, iota)?;
                match (ty.as_ref(), v) {
                    (Type::Basic(BasicType::Real), ConstVal::Int(n)) => {
                        Ok(ConstVal::Real(n as f64))
                    }
                    (
                        Type::Basic(BasicType::Int | BasicType::Big | BasicType::Byte),
                        ConstVal::Real(n),
                    ) => Ok(ConstVal::Int(n as i64)),
                    (Type::Basic(BasicType::String), ConstVal::Int(n)) => {
                        Ok(ConstVal::Str(n.to_string()))
                    }
                    (_, v) => Ok(v),
                }
            }
            Expr::Binary(lhs, op, rhs, _) => {
                let l = self.fold_const(lhs, iota)?;
                let r = self.fold_const(rhs, iota)?;
                fold_const_binary(&l, *op, &r)
            }
            _ => Err("not a constant expression".to_string()),
        }
    }

    /// Look up a folded constant by unqualified name, falling back to the
    /// include-derived symbol table.
    fn const_value(&self, name: &str) -> Option<Result<ConstVal, String>> {
        if let Some(v) = self.module_consts.get(name) {
            return Some(v.clone());
        }
        match self.symtab.as_ref()?.lookup(name) {
            Some(Symbol::Const { value, .. }) => Some(Ok(ConstVal::from(value))),
            _ => None,
        }
    }

    /// Look up a folded constant by `Module->NAME`.
    fn qualified_const_value(
        &self,
        module: &str,
        member: &str,
    ) -> Option<Result<ConstVal, String>> {
        if let Some(v) = self.qualified_consts.get(&format!("{module}->{member}")) {
            return Some(v.clone());
        }
        match self.symtab.as_ref()?.lookup_qualified(module, member) {
            Some(Symbol::Const { value, .. }) => Some(Ok(ConstVal::from(value))),
            _ => None,
        }
    }

    /// Give every module-level variable MP-resident storage. Declarations in
    /// the interface block of the implemented module count too — they are the
    /// module's own globals.
    fn collect_globals(&mut self, file: &SourceFile) -> Result<(), String> {
        for decl in &file.decls {
            match decl {
                Decl::Var(v) => self.declare_global(v)?,
                Decl::Module(m) if file.implement.first() == Some(&m.name) => {
                    for member in &m.members {
                        if let ModuleMember::Var(v) = member {
                            self.declare_global(v)?;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn declare_global(&mut self, v: &VarDecl) -> Result<(), String> {
        let ty = self.infer_decl_type(v);
        let kind = self.decl_num_kind(v);
        let elem_type = decl_array_elem_type(v);
        let chan_elem_basic = decl_chan_elem_basic(v);
        let adt_name = v.ty.as_ref().and_then(Self::adt_name_for_type);
        // A constant initialiser goes straight into the data section;
        // anything else is deferred to the top of the entry function.
        let init = match &v.init {
            Some(e) => self.fold_const(e, 0).ok(),
            None => None,
        };
        for name in &v.names {
            if name == "nil" || self.globals.iter().any(|(n, _, _, _)| n == name) {
                continue;
            }
            let off = self.alloc_mp(kind.byte_size());
            self.globals.push((name.clone(), off, ty, kind));
            if let Some(t) = &elem_type {
                self.local_array_elem.insert(name.clone(), t.clone());
            }
            if let Some(b) = chan_elem_basic {
                self.local_chan_elem.insert(name.clone(), b);
            }
            if let Some(a) = &adt_name {
                self.local_adt_type.insert(name.clone(), a.clone());
            }
            if init.is_none()
                && let Some(expr) = &v.init
            {
                self.pending_global_inits.push((name.clone(), expr.clone()));
            }
            if let Some(value) = &init {
                let item = match (value, kind) {
                    (ConstVal::Int(n), NumKind::Big) => DataItem::Bigs {
                        offset: off,
                        values: vec![*n],
                    },
                    (ConstVal::Int(n), _) => DataItem::Words {
                        offset: off,
                        values: vec![*n as i32],
                    },
                    (ConstVal::Real(n), _) => DataItem::Reals {
                        offset: off,
                        values: vec![*n],
                    },
                    (ConstVal::Str(s), _) => DataItem::String {
                        offset: off,
                        value: s.clone(),
                    },
                };
                self.data.push(item);
            }
        }
        Ok(())
    }

    /// Resolve a name to its storage slot, preferring function locals over
    /// module-level variables (locals shadow globals).
    fn lookup_var(&self, name: &str) -> Option<(Slot, ValType, NumKind)> {
        if let Some((_, off, ty, kind)) = self.locals.iter().find(|(n, _, _, _)| n == name) {
            return Some((Slot::Local(*off), *ty, *kind));
        }
        self.globals
            .iter()
            .find(|(n, _, _, _)| n == name)
            .map(|(_, off, ty, kind)| (Slot::Global(*off), *ty, *kind))
    }

    /// Load a 32-bit constant into `dst`. Dis operands are encoded in at most
    /// 30 signed bits, so a wider value has to travel through the data
    /// section rather than as an immediate.
    fn gen_word_const_to(&mut self, v: i32, dst: i32) {
        const OPERAND_MIN: i32 = -(1 << 29);
        const OPERAND_MAX: i32 = (1 << 29) - 1;
        if (OPERAND_MIN..=OPERAND_MAX).contains(&v) {
            self.emit(Opcode::Movw, op_imm(v), mid_unused(), op_fp(dst));
        } else {
            let mp_off = self.alloc_mp(4);
            self.data.push(DataItem::Words {
                offset: mp_off,
                values: vec![v],
            });
            self.emit(Opcode::Movw, op_mp(mp_off), mid_unused(), op_fp(dst));
        }
    }

    /// Materialize a folded constant into `dst`.
    fn gen_const_to(&mut self, value: &ConstVal, dst: i32) -> Result<(), String> {
        match value {
            ConstVal::Int(v) if *v > i32::MAX as i64 || *v < i32::MIN as i64 => {
                let mp_off = self.alloc_mp(8);
                self.data.push(DataItem::Bigs {
                    offset: mp_off,
                    values: vec![*v],
                });
                self.emit(Opcode::Movl, op_mp(mp_off), mid_unused(), op_fp(dst));
            }
            ConstVal::Int(v) => self.gen_word_const_to(*v as i32, dst),
            ConstVal::Real(v) => {
                let mp_off = self.alloc_mp(8);
                self.data.push(DataItem::Reals {
                    offset: mp_off,
                    values: vec![*v],
                });
                self.emit(Opcode::Movf, op_mp(mp_off), mid_unused(), op_fp(dst));
            }
            ConstVal::Str(s) => {
                let mp = self.intern_string(s);
                self.emit(Opcode::Movp, op_mp(mp), mid_unused(), op_fp(dst));
            }
        }
        Ok(())
    }

    /// Look up `(field_offset, field_type)` for `adt_name.field_name`.
    fn adt_field_info(&self, adt_name: &str, field_name: &str) -> Option<(i32, Type)> {
        let layout = self.adt_layouts.get(adt_name)?;
        layout
            .iter()
            .find(|(n, _, _)| n == field_name)
            .map(|(_, t, off)| (*off, t.clone()))
    }

    /// Extract the ADT name from a Limbo Type, peeling `ref` wrappers.
    fn adt_name_for_type(ty: &Type) -> Option<String> {
        match ty {
            Type::Named(q) => Some(q.name.clone()),
            Type::Ref(inner) => Self::adt_name_for_type(inner),
            _ => None,
        }
    }

    /// Resolve an expression to the ADT name of the value it produces, when
    /// derivable. Handles Ident lookup and one level of Dot navigation
    /// (`outer.inner.field` works iff the outer type's `inner` field is
    /// itself an ADT-typed field).
    fn adt_name_for_expr(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident(name, _) => self.local_adt_type.get(name).cloned(),
            Expr::Dot(inner, field, _) => {
                let outer = self.adt_name_for_expr(inner)?;
                let (_, ty) = self.adt_field_info(&outer, field)?;
                Self::adt_name_for_type(&ty)
            }
            _ => None,
        }
    }

    fn scan_stmt_strings(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Expr(e) => self.scan_expr_strings(e),
            Stmt::If(s) => {
                self.scan_expr_strings(&s.cond);
                self.scan_stmt_strings(&s.then);
                if let Some(e) = &s.else_ {
                    self.scan_stmt_strings(e);
                }
            }
            Stmt::While(s) => {
                self.scan_expr_strings(&s.cond);
                self.scan_stmt_strings(&s.body);
            }
            Stmt::For(s) => {
                if let Some(i) = &s.init {
                    self.scan_stmt_strings(i);
                }
                if let Some(c) = &s.cond {
                    self.scan_expr_strings(c);
                }
                if let Some(p) = &s.post {
                    self.scan_stmt_strings(p);
                }
                self.scan_stmt_strings(&s.body);
            }
            Stmt::Block(b) => {
                for s in &b.stmts {
                    self.scan_stmt_strings(s);
                }
            }
            Stmt::Return(Some(e), _) | Stmt::Raise(Some(e), _) | Stmt::Spawn(e, _) => {
                self.scan_expr_strings(e)
            }
            Stmt::VarDecl(v) => {
                if let Some(init) = &v.init {
                    self.scan_expr_strings(init);
                }
            }
            _ => {}
        }
    }

    fn scan_expr_strings(&mut self, expr: &Expr) {
        match expr {
            Expr::StringLit(s, _) => {
                self.intern_string(s);
            }
            Expr::Call(c, args, _) => {
                self.scan_expr_strings(c);
                for a in args {
                    self.scan_expr_strings(a);
                }
            }
            Expr::Binary(l, _, r, _)
            | Expr::Assign(l, r, _)
            | Expr::CompoundAssign(l, _, r, _)
            | Expr::Cons(l, r, _)
            | Expr::Send(l, r, _) => {
                self.scan_expr_strings(l);
                self.scan_expr_strings(r);
            }
            Expr::ModQual(l, _, _)
            | Expr::Dot(l, _, _)
            | Expr::Hd(l, _)
            | Expr::Tl(l, _)
            | Expr::Len(l, _)
            | Expr::Unary(_, l, _)
            | Expr::Recv(l, _)
            | Expr::PostInc(l, _)
            | Expr::PostDec(l, _)
            | Expr::Tagof(l, _) => self.scan_expr_strings(l),
            Expr::Load(_, p, _)
            | Expr::DeclAssign(_, p, _)
            | Expr::TupleDeclAssign(_, p, _)
            | Expr::Cast(_, p, _)
            | Expr::ArrayAlloc(p, _, _) => self.scan_expr_strings(p),
            Expr::Index(a, i, _) => {
                self.scan_expr_strings(a);
                self.scan_expr_strings(i);
            }
            Expr::Slice(a, lo, hi, _) => {
                self.scan_expr_strings(a);
                for l in lo.iter() {
                    self.scan_expr_strings(l);
                }
                for h in hi.iter() {
                    self.scan_expr_strings(h);
                }
            }
            Expr::Tuple(es, _) | Expr::ArrayLit(es, _, _) | Expr::ListLit(es, _) => {
                for e in es {
                    self.scan_expr_strings(e);
                }
            }
            _ => {}
        }
    }

    fn build_types(&mut self) {
        // Type 0: small/generic frame
        self.types.push(TypeDescriptor {
            id: 0,
            size: 16,
            pointer_map: PointerMap { bytes: vec![0x80] },
            pointer_count: 1,
        });
        // Type 1: sys call frame (48 bytes, ptrs at 32,36)
        self.types.push(TypeDescriptor {
            id: 1,
            size: 48,
            pointer_map: PointerMap {
                bytes: vec![0x00, 0x30],
            },
            pointer_count: 2,
        });
        // Types 2+: one per compiled function with its actual frame size
        for (i, &fsize) in self.func_frames.iter().enumerate() {
            let map_bytes = (fsize as usize).div_ceil(32).max(1);
            let mut pmap = vec![0u8; map_bytes];
            if pmap.len() > 4 {
                pmap[4] = 0x03;
            } // ptrs at 32, 36
            self.types.push(TypeDescriptor {
                id: (2 + i) as u32,
                size: fsize,
                pointer_map: PointerMap { bytes: pmap },
                pointer_count: 2,
            });
        }
    }

    fn gen_func(&mut self, func: &FuncDecl) -> Result<(), String> {
        let entry_pc = self.code.len();
        self.locals.clear();
        self.next_local = 40;
        // Each function gets its own frame layout; without this reset every
        // function's descriptor would carry the running maximum frame size.
        self.frame_size = 80;

        // Register parameter names at fixed offsets
        let mut param_off = 32;
        for param in &func.sig.params {
            for name in &param.names {
                let ty = self.infer_param_type(param);
                let kind = type_num_kind(&param.ty);
                if name != "nil" {
                    self.locals.push((name.clone(), param_off, ty, kind));
                    // Track array params' element Type so indexing into
                    // them later picks the right Ind/Mov opcode pair, and
                    // recursive nested types (`array of array of T`) work
                    // by peeling one layer per Index.
                    if let Type::Array(elem) = &param.ty {
                        self.local_array_elem.insert(name.clone(), (**elem).clone());
                    }
                    // Same for chan params so Send/Recv on them know the
                    // element width.
                    if let Type::Chan(elem) = &param.ty
                        && let Type::Basic(b) = elem.as_ref()
                    {
                        self.local_chan_elem.insert(name.clone(), *b);
                    }
                    // ADT-typed params (`p: ref Foo` or `p: Foo`): record the
                    // ADT name so field access through `p` finds the layout.
                    if let Some(adt) = Self::adt_name_for_type(&param.ty) {
                        self.local_adt_type.insert(name.clone(), adt);
                    }
                }
                // Frame param slots are 4-byte aligned in the reference ABI;
                // big/real params occupy two adjacent slots. This keeps the
                // offsets consistent with how the caller packs arguments.
                param_off += kind.byte_size();
            }
        }
        self.next_local = param_off.max(40);

        // Module-level initialisers that aren't compile-time constants run
        // first, inside the entry function: it is the module's first code.
        if func.name.name == "init" && !self.pending_global_inits.is_empty() {
            let inits = std::mem::take(&mut self.pending_global_inits);
            for (name, expr) in &inits {
                self.gen_assign_to_ident(name, expr, None)?;
            }
        }

        for stmt in &func.body.stmts {
            self.gen_stmt(stmt)?;
        }

        if self.code.is_empty()
            || !matches!(
                self.code.last().map(|i| i.opcode),
                Some(Opcode::Ret) | Some(Opcode::Exit)
            )
        {
            self.emit(Opcode::Ret, op_unused(), mid_unused(), op_unused());
        }

        // Patch the pre-registered func_table entry with the actual PC
        // and frame size. The pre-registration pass in `compile()` filled
        // name and ret_kind so spawn/call could look up forward-defined
        // functions while emitting code; here we finalize the layout.
        let full_name = if let Some(q) = &func.name.qualifier {
            format!("{q}.{}", func.name.name)
        } else {
            func.name.name.clone()
        };
        if let Some(entry) = self
            .func_table
            .iter_mut()
            .find(|(n, pc, _, _)| n == &full_name && *pc < 0)
        {
            entry.1 = entry_pc as i32;
            entry.2 = self.frame_size;
        }
        self.func_frames.push(self.frame_size);

        let func_idx = self.func_frames.len() as i32 - 1;
        let type_idx = 2 + func_idx; // types 0,1 are reserved, func types start at 2
        if func.name.name == "init" {
            self.exports.push(ExportEntry {
                pc: entry_pc as i32,
                frame_type: type_idx,
                signature: 0,
                name: "init".to_string(),
            });
        }
        Ok(())
    }

    fn infer_param_type(&self, param: &Param) -> ValType {
        match &param.ty {
            Type::Basic(BasicType::Int)
            | Type::Basic(BasicType::Byte)
            | Type::Basic(BasicType::Big)
            | Type::Basic(BasicType::Real) => ValType::Word,
            // Arrays carry their own ValType so call sites and Index handlers
            // can pick array-specific opcodes (Indw/Indl/Movw/Movl/...).
            Type::Array(_) => ValType::Array,
            _ => ValType::Ptr,
        }
    }

    /// Look up an array local's element Type, or None if not an array.
    fn array_elem_type(&self, name: &str) -> Option<Type> {
        self.local_array_elem.get(name).cloned()
    }

    /// Resolve the array element Type for an arr expression. Handles
    /// `Ident` (look up sidecar) and `Index(inner, _)` (recursively peel
    /// one layer of nesting); other forms fall back to None.
    fn array_elem_type_for_expr(&self, expr: &Expr) -> Option<Type> {
        match expr {
            Expr::Ident(name, _) => self.array_elem_type(name),
            Expr::Index(inner_arr, _, _) => {
                // Outer Index returns an element of inner's elem type. If
                // that elem type is itself `array of X`, we want X.
                let inner_elem = self.array_elem_type_for_expr(inner_arr)?;
                if let Type::Array(elem) = inner_elem {
                    Some(*elem)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Convenience for sites that need a BasicType (to pick Indw vs Indl
    /// etc.). Returns None when the element type isn't a single basic type
    /// — e.g., nested arrays or ADT-typed elements collapse to Word/Indw
    /// at the call site.
    fn array_elem_basic_for_expr(&self, expr: &Expr) -> Option<BasicType> {
        type_basic(&self.array_elem_type_for_expr(expr)?)
    }

    /// Pick the (Ind*, Mov*) opcode pair for an array element of the given
    /// BasicType. Defaults (None or non-numeric BasicType::String) treat
    /// the element as a 4-byte word (the common case for ADT/pointer arrays).
    fn array_elem_opcodes(basic: Option<BasicType>) -> (Opcode, Opcode) {
        match basic {
            Some(BasicType::Byte) => (Opcode::Indb, Opcode::Movb),
            Some(BasicType::Big) => (Opcode::Indl, Opcode::Movl),
            Some(BasicType::Real) => (Opcode::Indf, Opcode::Movf),
            _ => (Opcode::Indw, Opcode::Movw),
        }
    }

    // ── Statements ────────────────────────────────────────────

    fn gen_stmt(&mut self, stmt: &Stmt) -> Result<(), String> {
        match stmt {
            Stmt::Expr(e) => self.gen_expr_discard(e),
            Stmt::VarDecl(v) => {
                let ty = self.infer_decl_type(v);
                let kind = self.decl_num_kind(v);
                // If the declared (or inferred) Limbo type is an array,
                // record its full element Type so nested `array of array
                // of T` works through recursive peeling.
                let elem_type = decl_array_elem_type(v);
                let chan_elem_basic = decl_chan_elem_basic(v);
                let adt_name = v.ty.as_ref().and_then(Self::adt_name_for_type);
                for name in &v.names {
                    self.alloc_local(name, ty, kind);
                    if let Some(t) = &elem_type {
                        self.local_array_elem.insert(name.clone(), t.clone());
                    }
                    if let Some(b) = chan_elem_basic {
                        self.local_chan_elem.insert(name.clone(), b);
                    }
                    if let Some(a) = &adt_name {
                        self.local_adt_type.insert(name.clone(), a.clone());
                    }
                }
                if let Some(init) = &v.init {
                    let name = v.names.first().map(|s| s.as_str()).unwrap_or("");
                    let off = self.get_local(name).map(|(o, _)| o).unwrap_or(0);
                    self.gen_expr_to(init, off)?;
                }
                Ok(())
            }
            Stmt::Return(expr, _) => {
                if let Some(e) = expr {
                    // Tuple return: write each field through the return
                    // pointer at the appropriate cumulative offset, sized
                    // by each field's NumKind.
                    if let Expr::Tuple(fields, _) = e {
                        let mut field_off = 0i32;
                        for field in fields {
                            let ty = self.infer_expr_type(field);
                            let kind = self.infer_num_kind(field);
                            let tmp = self.alloc_temp_for(kind);
                            self.gen_expr_to(field, tmp)?;
                            let op = match (ty, kind) {
                                (_, NumKind::Big) => Opcode::Movl,
                                (_, NumKind::Real) => Opcode::Movf,
                                (ValType::Word, NumKind::Word) => Opcode::Movw,
                                _ => Opcode::Movp,
                            };
                            self.emit(op, op_fp(tmp), mid_unused(), op_fp_ind(16, field_off));
                            field_off += kind.byte_size();
                        }
                    } else {
                        // Single value: kind-matched opcode through return ptr.
                        let ty = self.infer_expr_type(e);
                        let kind = self.infer_num_kind(e);
                        let val_tmp = self.alloc_temp_for(kind);
                        self.gen_expr_to(e, val_tmp)?;
                        let op = match (ty, kind) {
                            (_, NumKind::Big) => Opcode::Movl,
                            (_, NumKind::Real) => Opcode::Movf,
                            (ValType::Word, NumKind::Word) => Opcode::Movw,
                            _ => Opcode::Movp,
                        };
                        self.emit(op, op_fp(val_tmp), mid_unused(), op_fp_ind(16, 0));
                    }
                }
                self.emit(Opcode::Ret, op_unused(), mid_unused(), op_unused());
                Ok(())
            }
            Stmt::Exit(_) => {
                self.emit(Opcode::Exit, op_unused(), mid_unused(), op_unused());
                Ok(())
            }
            Stmt::Block(b) => {
                for s in &b.stmts {
                    self.gen_stmt(s)?;
                }
                Ok(())
            }
            Stmt::If(s) => self.gen_if(s),
            Stmt::While(s) => self.gen_while(s),
            Stmt::For(s) => self.gen_for(s),
            Stmt::Raise(Some(e), _) => {
                let tmp = self.alloc_temp();
                self.gen_expr_to(e, tmp)?;
                self.emit(Opcode::Raise, op_fp(tmp), mid_unused(), op_unused());
                Ok(())
            }
            Stmt::Case(s) => self.gen_case(s),
            Stmt::Do(s) => self.gen_do(s),
            Stmt::Label(name, inner) => {
                // The label belongs to the construct it prefixes; the next
                // loop/case takes it. Clear it afterwards so it can't leak
                // onto an unrelated later construct.
                self.pending_label = Some(name.clone());
                let result = self.gen_stmt(inner);
                self.pending_label = None;
                result
            }
            Stmt::Break(label, _) => self.gen_break(label.as_deref()),
            Stmt::Continue(label, _) => self.gen_continue(label.as_deref()),
            Stmt::Spawn(e, _) => {
                // spawn func(args) → Frame + Spawn
                if let Expr::Call(callee, args, _) = e
                    && let Expr::Ident(func_name, _) = callee.as_ref()
                {
                    // Look up function PC
                    let func_pc = self
                        .func_table
                        .iter()
                        .find(|(n, _, _, _)| n == func_name)
                        .map(|(_, pc, _, _)| *pc);
                    if let Some(pc) = func_pc {
                        // Find the type index for this function
                        let func_type = self
                            .func_table
                            .iter()
                            .enumerate()
                            .find(|(_, (n, _, _, _))| n == func_name)
                            .map(|(i, _)| 2 + i as i32)
                            .unwrap_or(1);
                        let frame_tmp = self.alloc_temp();
                        self.emit(
                            Opcode::Frame,
                            op_imm(func_type),
                            mid_unused(),
                            op_fp(frame_tmp),
                        );
                        // Pack arguments at cumulative offsets matching the
                        // spawnee's kind-sized param layout.
                        let mut arg_off = 32i32;
                        for arg in args.iter() {
                            let kind = self.infer_num_kind(arg);
                            let arg_tmp = self.alloc_temp_for(kind);
                            self.gen_expr_to(arg, arg_tmp)?;
                            let ty = self.infer_expr_type(arg);
                            let op = match (ty, kind) {
                                (_, NumKind::Big) => Opcode::Movl,
                                (_, NumKind::Real) => Opcode::Movf,
                                (ValType::Word, NumKind::Word) => Opcode::Movw,
                                _ => Opcode::Movp,
                            };
                            self.emit(
                                op,
                                op_fp(arg_tmp),
                                mid_unused(),
                                op_fp_ind(frame_tmp, arg_off),
                            );
                            arg_off += kind.byte_size();
                        }
                        let spawn_idx = self.code.len();
                        self.emit(Opcode::Spawn, op_fp(frame_tmp), mid_unused(), op_imm(pc));
                        if pc < 0 {
                            // Forward reference: patch the destination's
                            // immediate after the callee is generated.
                            self.pending_call_fixups
                                .push((spawn_idx, func_name.clone()));
                        }
                        return Ok(());
                    }
                }
                // Fallback: just evaluate
                self.gen_expr_discard(e)?;
                Ok(())
            }
            Stmt::Empty => Ok(()),
            // Unsupported constructs are hard errors: emitting nothing at all
            // would silently drop the statement's behavior.
            Stmt::Alt(_) => Err("`alt` statements are not supported yet".to_string()),
            Stmt::Pick(_) => Err("`pick` statements are not supported yet".to_string()),
            Stmt::Raise(None, _) => Err("bare `raise` (re-raise) is not supported yet".to_string()),
        }
    }

    /// Enter a breakable construct, taking any label that prefixed it.
    fn push_loop(&mut self, continuable: bool) {
        let label = self.pending_label.take();
        self.loop_stack.push(LoopFrame {
            label,
            breaks: Vec::new(),
            continues: Vec::new(),
            continuable,
        });
    }

    /// Find the innermost enclosing construct a `break`/`continue` refers to.
    fn find_loop(&self, label: Option<&str>, need_continue: bool) -> Option<usize> {
        self.loop_stack.iter().rposition(|frame| {
            (!need_continue || frame.continuable)
                && match label {
                    Some(l) => frame.label.as_deref() == Some(l),
                    None => true,
                }
        })
    }

    fn gen_break(&mut self, label: Option<&str>) -> Result<(), String> {
        let Some(idx) = self.find_loop(label, false) else {
            return Err(match label {
                Some(l) => format!("`break {l}`: no enclosing statement labelled `{l}`"),
                None => "`break` outside of a loop or case statement".to_string(),
            });
        };
        let jump = self.code.len();
        self.emit(Opcode::Jmp, op_unused(), mid_unused(), op_imm(0));
        self.loop_stack[idx].breaks.push(jump);
        Ok(())
    }

    fn gen_continue(&mut self, label: Option<&str>) -> Result<(), String> {
        let Some(idx) = self.find_loop(label, true) else {
            return Err(match label {
                Some(l) => format!("`continue {l}`: no enclosing loop labelled `{l}`"),
                None => "`continue` outside of a loop".to_string(),
            });
        };
        let jump = self.code.len();
        self.emit(Opcode::Jmp, op_unused(), mid_unused(), op_imm(0));
        self.loop_stack[idx].continues.push(jump);
        Ok(())
    }

    /// Leave a breakable construct, patching its recorded `break` jumps to
    /// `exit_pc` and its `continue` jumps to `continue_pc`.
    fn pop_loop(&mut self, exit_pc: i32, continue_pc: i32) {
        let Some(frame) = self.loop_stack.pop() else {
            return;
        };
        for idx in frame.breaks {
            self.code[idx].destination = op_imm(exit_pc);
        }
        for idx in frame.continues {
            self.code[idx].destination = op_imm(continue_pc);
        }
    }

    fn infer_decl_type(&self, v: &VarDecl) -> ValType {
        if let Some(ty) = &v.ty {
            return match ty {
                Type::Basic(BasicType::Int)
                | Type::Basic(BasicType::Byte)
                | Type::Basic(BasicType::Big)
                | Type::Basic(BasicType::Real) => ValType::Word,
                _ => ValType::Ptr,
            };
        }
        // Infer from init expression
        if let Some(init) = &v.init {
            return self.infer_expr_type(init);
        }
        ValType::Word
    }

    /// Resolve the NumKind for a VarDecl. An explicit `: big` / `: real`
    /// annotation wins; otherwise we look at the init expression.
    fn decl_num_kind(&self, v: &VarDecl) -> NumKind {
        if let Some(ty) = &v.ty {
            return type_num_kind(ty);
        }
        if let Some(init) = &v.init {
            return self.infer_num_kind(init);
        }
        NumKind::Word
    }

    /// Infer the numeric kind of an expression. Used only by numeric paths
    /// (binary arith, casts, var-decl sizing) — non-numeric expressions
    /// collapse to Word, which is a valid default for those callers.
    fn infer_num_kind(&self, expr: &Expr) -> NumKind {
        match expr {
            Expr::IntLit(v, _) => {
                if *v > i32::MAX as i64 || *v < i32::MIN as i64 {
                    NumKind::Big
                } else {
                    NumKind::Word
                }
            }
            Expr::RealLit(_, _) => NumKind::Real,
            Expr::CharLit(_, _) => NumKind::Word,
            Expr::Ident(name, _) => match self.lookup_var(name) {
                Some((_, _, kind)) => kind,
                None => match self.const_value(name) {
                    Some(Ok(v)) => v.num_kind(),
                    _ => NumKind::Word,
                },
            },
            // An assignment used as an expression has the kind of the value
            // it stores; without this the surrounding slot is sized as Word.
            Expr::Assign(_, rhs, _)
            | Expr::DeclAssign(_, rhs, _)
            | Expr::CompoundAssign(_, _, rhs, _) => self.infer_num_kind(rhs),
            Expr::PostInc(inner, _) | Expr::PostDec(inner, _) => self.infer_num_kind(inner),
            Expr::Cast(ty, _, _) => type_num_kind(ty),
            // Unary ops preserve the inner kind (negation of big stays big).
            Expr::Unary(_, inner, _) => self.infer_num_kind(inner),
            Expr::Binary(lhs, op, rhs, _) => match op {
                // Relational and logical ops always yield a word-sized bool.
                BinOp::Eq
                | BinOp::Neq
                | BinOp::Lt
                | BinOp::Gt
                | BinOp::Leq
                | BinOp::Geq
                | BinOp::LogAnd
                | BinOp::LogOr => NumKind::Word,
                // Arithmetic: promote to the widest operand kind.
                _ => self.infer_num_kind(lhs).max(self.infer_num_kind(rhs)),
            },
            Expr::Len(_, _) => NumKind::Word,
            // Local function calls: look up return kind in func_table.
            Expr::Call(callee, _, _) => {
                if let Expr::Ident(name, _) = callee.as_ref() {
                    self.func_table
                        .iter()
                        .find(|(n, _, _, _)| n == name)
                        .map(|(_, _, _, k)| *k)
                        .unwrap_or(NumKind::Word)
                } else if let Expr::ModQual(_, name, _) = callee.as_ref() {
                    sys_return_kind(name)
                } else {
                    NumKind::Word
                }
            }
            // Array element access inherits the element's NumKind so callers
            // (Cast, gen_expr_to_kind, sys-print arg packing) treat `a[i]`
            // as Big/Real when the array's element type is.
            Expr::Index(arr, _, _) => {
                if let Some(b) = self.array_elem_basic_for_expr(arr) {
                    type_num_kind(&Type::Basic(b))
                } else {
                    NumKind::Word
                }
            }
            // `<-chan` returns the channel's element kind.
            Expr::Recv(chan_expr, _) => {
                if let Expr::Ident(name, _) = chan_expr.as_ref()
                    && let Some(b) = self.local_chan_elem.get(name)
                {
                    type_num_kind(&Type::Basic(*b))
                } else {
                    NumKind::Word
                }
            }
            // ADT field access propagates the field's NumKind so big/real
            // fields read as Big/Real (and arg packing/comparisons size
            // their slots correctly).
            Expr::Dot(inner, field, _) => self
                .adt_name_for_expr(inner)
                .and_then(|a| self.adt_field_info(&a, field))
                .map(|(_, t)| type_num_kind(&t))
                .unwrap_or(NumKind::Word),
            _ => NumKind::Word,
        }
    }

    /// Produce a value of `target` kind at `dst`, emitting a Cvt* instruction
    /// when the expression's natural kind differs. This is the kind-aware
    /// replacement for raw `gen_expr_to(expr, dst)` at sites where the slot
    /// width and operand width must match (binary arith, returns, call args).
    fn gen_expr_to_kind(&mut self, expr: &Expr, dst: i32, target: NumKind) -> Result<(), String> {
        let inner = self.infer_num_kind(expr);
        if inner == target {
            return self.gen_expr_to(expr, dst);
        }
        // Narrow into a temp of the inner kind, then convert to target.
        let tmp = self.alloc_temp_for(inner);
        self.gen_expr_to(expr, tmp)?;
        let cvt = match (inner, target) {
            (NumKind::Word, NumKind::Big) => Opcode::Cvtwl,
            (NumKind::Word, NumKind::Real) => Opcode::Cvtwf,
            (NumKind::Big, NumKind::Word) => Opcode::Cvtlw,
            (NumKind::Big, NumKind::Real) => Opcode::Cvtlf,
            (NumKind::Real, NumKind::Word) => Opcode::Cvtfw,
            (NumKind::Real, NumKind::Big) => Opcode::Cvtfl,
            // Same-kind paths are handled by the early return above.
            _ => return self.gen_expr_to(expr, dst),
        };
        self.emit(cvt, op_fp(tmp), mid_unused(), op_fp(dst));
        Ok(())
    }

    fn infer_expr_type(&self, expr: &Expr) -> ValType {
        match expr {
            Expr::IntLit(_, _) | Expr::CharLit(_, _) | Expr::RealLit(_, _) => ValType::Word,
            Expr::StringLit(_, _) | Expr::Nil(_) => ValType::Ptr,
            Expr::Ident(name, _) => match self.lookup_var(name) {
                Some((_, ty, _)) => ty,
                None => match self.const_value(name) {
                    Some(Ok(v)) => v.val_type(),
                    _ => ValType::Word,
                },
            },
            Expr::Assign(_, rhs, _)
            | Expr::DeclAssign(_, rhs, _)
            | Expr::CompoundAssign(_, _, rhs, _) => self.infer_expr_type(rhs),
            Expr::PostInc(inner, _) | Expr::PostDec(inner, _) => self.infer_expr_type(inner),
            Expr::Binary(lhs, op, _, _) => match op {
                BinOp::Eq
                | BinOp::Neq
                | BinOp::Lt
                | BinOp::Gt
                | BinOp::Leq
                | BinOp::Geq
                | BinOp::LogAnd
                | BinOp::LogOr => ValType::Word,
                BinOp::Add => {
                    // String concatenation returns Ptr
                    if self.infer_expr_type(lhs) == ValType::Ptr {
                        ValType::Ptr
                    } else {
                        ValType::Word
                    }
                }
                _ => ValType::Word,
            },
            Expr::Hd(_, _) => ValType::Ptr,
            Expr::Tl(_, _) => ValType::Ptr,
            Expr::Len(_, _) => ValType::Word,
            // ADT field access mirrors the field's declared type.
            Expr::Dot(inner, field, _) => {
                match self
                    .adt_name_for_expr(inner)
                    .and_then(|a| self.adt_field_info(&a, field))
                {
                    Some((_, Type::Basic(_))) => ValType::Word,
                    Some((_, Type::Array(_))) => ValType::Array,
                    Some((_, _)) => ValType::Ptr,
                    None => ValType::Word,
                }
            }
            Expr::Index(arr, _, _) => {
                // Element type drives the resulting ValType: word/byte/big/
                // real → Word; nested arrays → Array; everything else (string,
                // adt, ref, list, chan) → Ptr.
                match self.array_elem_type_for_expr(arr) {
                    Some(Type::Basic(_)) => ValType::Word,
                    Some(Type::Array(_)) => ValType::Array,
                    Some(_) => ValType::Ptr,
                    // Unknown element type: default to Word (covers
                    // string-char indexing and anonymous arrays alike).
                    None => ValType::Word,
                }
            }
            Expr::Load(_, _, _) => ValType::Ptr,
            Expr::Call(callee, _, _) => {
                // Infer return type from callee name
                if let Expr::ModQual(_, name, _) = callee.as_ref() {
                    match name.as_str() {
                        "fildes" | "open" | "create" | "fstat" | "stat" | "dirread" | "dial"
                        | "announce" | "listen" => ValType::Ptr,
                        _ => ValType::Word,
                    }
                } else {
                    ValType::Word
                }
            }
            Expr::Cons(_, _, _) => ValType::Ptr,
            Expr::ArrayAlloc(_, _, _) | Expr::ArrayLit(_, _, _) => ValType::Array,
            Expr::ChanAlloc(_, _) | Expr::ListLit(_, _) => ValType::Ptr,
            Expr::Cast(ty, _, _) => match ty.as_ref() {
                // Numeric casts produce a numeric value; the slot is sized
                // by NumKind (looked up separately), but the ValType is Word
                // so binary arith doesn't misroute through the string-concat
                // path that triggers on `lhs ValType == Ptr`.
                Type::Basic(BasicType::Int)
                | Type::Basic(BasicType::Byte)
                | Type::Basic(BasicType::Big)
                | Type::Basic(BasicType::Real) => ValType::Word,
                Type::Array(_) => ValType::Array,
                _ => ValType::Ptr,
            },
            _ => ValType::Word,
        }
    }

    fn gen_if(&mut self, s: &IfStmt) -> Result<(), String> {
        // Optimize: nil comparison for pointer types
        let cond_tmp = self.alloc_temp();
        self.gen_cond_to(&s.cond, cond_tmp)?;
        let jump_idx = self.code.len();
        self.emit(Opcode::Beqw, op_fp(cond_tmp), mid_imm(0), op_imm(0));
        self.gen_stmt(&s.then)?;
        if let Some(else_stmt) = &s.else_ {
            let skip_idx = self.code.len();
            self.emit(Opcode::Jmp, op_unused(), mid_unused(), op_imm(0));
            self.code[jump_idx].destination = op_imm(self.code.len() as i32);
            self.gen_stmt(else_stmt)?;
            self.code[skip_idx].destination = op_imm(self.code.len() as i32);
        } else {
            self.code[jump_idx].destination = op_imm(self.code.len() as i32);
        }
        Ok(())
    }

    fn gen_while(&mut self, s: &WhileStmt) -> Result<(), String> {
        self.push_loop(true);
        let loop_start = self.code.len() as i32;
        let cond_tmp = self.alloc_temp();
        self.gen_cond_to(&s.cond, cond_tmp)?;
        let jump_idx = self.code.len();
        self.emit(Opcode::Beqw, op_fp(cond_tmp), mid_imm(0), op_imm(0));
        self.gen_stmt(&s.body)?;
        self.emit(Opcode::Jmp, op_unused(), mid_unused(), op_imm(loop_start));
        let exit_pc = self.code.len() as i32;
        self.code[jump_idx].destination = op_imm(exit_pc);
        // `continue` re-tests the condition.
        self.pop_loop(exit_pc, loop_start);
        Ok(())
    }

    fn gen_for(&mut self, s: &ForStmt) -> Result<(), String> {
        self.push_loop(true);
        if let Some(init) = &s.init {
            self.gen_stmt(init)?;
        }
        let loop_start = self.code.len() as i32;
        let jump_idx = if let Some(cond) = &s.cond {
            let cond_tmp = self.alloc_temp();
            self.gen_cond_to(cond, cond_tmp)?;
            let idx = self.code.len();
            self.emit(Opcode::Beqw, op_fp(cond_tmp), mid_imm(0), op_imm(0));
            Some(idx)
        } else {
            None
        };
        self.gen_stmt(&s.body)?;
        // `continue` runs the post statement, then re-tests the condition.
        let continue_pc = self.code.len() as i32;
        if let Some(post) = &s.post {
            self.gen_stmt(post)?;
        }
        self.emit(Opcode::Jmp, op_unused(), mid_unused(), op_imm(loop_start));
        let exit_pc = self.code.len() as i32;
        if let Some(idx) = jump_idx {
            self.code[idx].destination = op_imm(exit_pc);
        }
        self.pop_loop(exit_pc, continue_pc);
        Ok(())
    }

    fn gen_do(&mut self, s: &DoStmt) -> Result<(), String> {
        self.push_loop(true);
        let loop_start = self.code.len() as i32;
        self.gen_stmt(&s.body)?;
        // `continue` skips the rest of the body but still tests the condition.
        let continue_pc = self.code.len() as i32;
        let cond_tmp = self.alloc_temp();
        self.gen_cond_to(&s.cond, cond_tmp)?;
        // Branch back to start if condition is true (nonzero)
        self.emit(
            Opcode::Bnew,
            op_fp(cond_tmp),
            mid_imm(0),
            op_imm(loop_start),
        );
        let exit_pc = self.code.len() as i32;
        self.pop_loop(exit_pc, continue_pc);
        Ok(())
    }

    fn gen_case(&mut self, s: &CaseStmt) -> Result<(), String> {
        // A case is breakable (a `break` in an arm leaves the case) but not
        // continuable — `continue` belongs to the enclosing loop.
        self.push_loop(false);
        let val_tmp = self.alloc_temp();
        self.gen_expr_to(&s.expr, val_tmp)?;
        let val_ty = self.infer_expr_type(&s.expr);

        let mut end_jumps = Vec::new();

        for arm in &s.arms {
            let mut arm_jumps = Vec::new();

            // Generate condition checks for each pattern
            for pattern in &arm.patterns {
                match pattern {
                    CasePattern::Expr(e) => {
                        let pat_tmp = self.alloc_temp();
                        self.gen_expr_to(e, pat_tmp)?;
                        let branch = if val_ty != ValType::Word {
                            Opcode::Beqc
                        } else {
                            Opcode::Beqw
                        };
                        let idx = self.code.len();
                        self.emit(branch, op_fp(val_tmp), mid_fp(pat_tmp), op_imm(0));
                        arm_jumps.push(idx);
                    }
                    CasePattern::Range(lo, hi) => {
                        // Matches when `lo <= val && val <= hi`.
                        let lo_tmp = self.alloc_temp();
                        let hi_tmp = self.alloc_temp();
                        self.gen_expr_to(lo, lo_tmp)?;
                        self.gen_expr_to(hi, hi_tmp)?;
                        // val < lo: this pattern cannot match, so skip past
                        // the upper-bound test to the next pattern check.
                        let below_idx = self.code.len();
                        self.emit(Opcode::Bltw, op_fp(val_tmp), mid_fp(lo_tmp), op_imm(0));
                        // val <= hi: matched, jump to the arm body.
                        let match_idx = self.code.len();
                        self.emit(Opcode::Blew, op_fp(val_tmp), mid_fp(hi_tmp), op_imm(0));
                        arm_jumps.push(match_idx);
                        self.code[below_idx].destination = op_imm(self.code.len() as i32);
                    }
                    CasePattern::Wildcard => {
                        // Always matches — jump to body
                        let idx = self.code.len();
                        self.emit(Opcode::Jmp, op_unused(), mid_unused(), op_imm(0));
                        arm_jumps.push(idx);
                    }
                }
            }

            // Skip to next arm if no pattern matched
            let skip_idx = self.code.len();
            self.emit(Opcode::Jmp, op_unused(), mid_unused(), op_imm(0));

            // Patch arm jumps to here (body start)
            let body_pc = self.code.len() as i32;
            for idx in arm_jumps {
                self.code[idx].destination = op_imm(body_pc);
            }

            // Generate arm body
            for stmt in &arm.body {
                self.gen_stmt(stmt)?;
            }

            // Jump to end of case
            let end_idx = self.code.len();
            self.emit(Opcode::Jmp, op_unused(), mid_unused(), op_imm(0));
            end_jumps.push(end_idx);

            // Patch skip to here (next arm)
            self.code[skip_idx].destination = op_imm(self.code.len() as i32);
        }

        // Patch all end jumps
        let end_pc = self.code.len() as i32;
        for idx in end_jumps {
            self.code[idx].destination = op_imm(end_pc);
        }
        // `break` in an arm lands here too; `continue` never targets a case,
        // so its continue PC is unused.
        self.pop_loop(end_pc, end_pc);

        Ok(())
    }

    /// Generate condition code. For pointer nil comparisons, use Bnew/Beqw with $0.
    fn gen_cond_to(&mut self, expr: &Expr, dst: i32) -> Result<(), String> {
        match expr {
            // Optimize: x != nil or x == nil for pointer types
            Expr::Binary(lhs, BinOp::Neq, rhs, _) if self.is_nil(rhs) => {
                self.gen_expr_to(lhs, dst)?;
                // dst already holds the pointer; nonzero = true
                Ok(())
            }
            Expr::Binary(lhs, BinOp::Eq, rhs, _) if self.is_nil(rhs) => {
                let tmp = self.alloc_temp();
                self.gen_expr_to(lhs, tmp)?;
                self.emit(Opcode::Movw, op_imm(1), mid_unused(), op_fp(dst));
                let skip = self.code.len();
                self.emit(Opcode::Beqw, op_fp(tmp), mid_imm(0), op_imm(0));
                self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(dst));
                self.code[skip].destination = op_imm(self.code.len() as i32);
                Ok(())
            }
            Expr::Binary(lhs, BinOp::Neq, rhs, _) if self.is_nil(lhs) => {
                self.gen_expr_to(rhs, dst)?;
                Ok(())
            }
            _ => self.gen_expr_to(expr, dst),
        }
    }

    fn is_nil(&self, expr: &Expr) -> bool {
        matches!(expr, Expr::Nil(_))
    }

    // ── Expression generation ──────────────────────────────────

    fn gen_expr_discard(&mut self, expr: &Expr) -> Result<(), String> {
        match expr {
            Expr::Assign(lhs, rhs, _) => {
                if let Expr::Ident(name, _) = lhs.as_ref() {
                    self.gen_assign_to_ident(name, rhs, None)
                } else if let Expr::Index(arr_expr, idx_expr, _) = lhs.as_ref() {
                    // Distinguish array-element write from string-character
                    // insert by the lvalue's ValType. Strings are Ptr; arrays
                    // are Array (set by infer_param_type / infer_decl_type).
                    let arr_ty = self.infer_expr_type(arr_expr);
                    let idx_tmp = self.alloc_temp();
                    let arr_tmp = self.alloc_temp();
                    if arr_ty == ValType::Array {
                        // Array element write:
                        //   Ind* arr, ref, idx — install heap_ref at ref slot
                        //   Mov* val, *(ref)  — write through ref
                        let elem = self.array_elem_basic_for_expr(arr_expr);
                        let elem_kind = elem
                            .map(|b| type_num_kind(&Type::Basic(b)))
                            .unwrap_or(NumKind::Word);
                        let val_tmp = self.alloc_temp_for(elem_kind);
                        self.gen_expr_to(rhs, val_tmp)?;
                        self.gen_expr_to(idx_expr, idx_tmp)?;
                        self.gen_expr_to(arr_expr, arr_tmp)?;
                        let ref_tmp = self.alloc_temp();
                        let (ind_op, mov_op) = Self::array_elem_opcodes(elem);
                        self.emit(ind_op, op_fp(arr_tmp), mid_fp(ref_tmp), op_fp(idx_tmp));
                        self.emit(mov_op, op_fp(val_tmp), mid_unused(), op_fp_ind(ref_tmp, 0));
                    } else {
                        // String char insert: s[i] = c.
                        let val_tmp = self.alloc_temp();
                        self.gen_expr_to(rhs, val_tmp)?;
                        self.gen_expr_to(idx_expr, idx_tmp)?;
                        self.gen_expr_to(arr_expr, arr_tmp)?;
                        self.emit(
                            Opcode::Insc,
                            op_fp(val_tmp),
                            mid_fp(idx_tmp),
                            op_fp(arr_tmp),
                        );
                    }
                    Ok(())
                } else if let Expr::Slice(arr_expr, lo, _, _) = lhs.as_ref() {
                    // a[lo:] = rhs → Slicela rhs, lo, a
                    let rhs_tmp = self.alloc_temp();
                    self.gen_expr_to(rhs, rhs_tmp)?;
                    let arr_tmp = self.alloc_temp();
                    self.gen_expr_to(arr_expr, arr_tmp)?;
                    let lo_tmp = if let Some(lo_expr) = lo {
                        let t = self.alloc_temp();
                        self.gen_expr_to(lo_expr, t)?;
                        t
                    } else {
                        let t = self.alloc_temp();
                        self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(t));
                        t
                    };
                    self.emit(
                        Opcode::Slicela,
                        op_fp(rhs_tmp),
                        mid_fp(lo_tmp),
                        op_fp(arr_tmp),
                    );
                    Ok(())
                } else if let Expr::Dot(inner_expr, field, _) = lhs.as_ref() {
                    // p.field = val → write through the ref pointer. Use the
                    // ADT layout when available so big/real fields use the
                    // wide opcode and the right offset.
                    let ref_tmp = self.alloc_temp();
                    let (field_off, write_kind) = match self
                        .adt_name_for_expr(inner_expr)
                        .and_then(|a| self.adt_field_info(&a, field))
                    {
                        Some((off, ty)) => {
                            let kind = type_num_kind(&ty);
                            (off, Some((ty, kind)))
                        }
                        None => (self.estimate_field_offset(inner_expr, field), None),
                    };
                    let val_kind = write_kind
                        .as_ref()
                        .map(|(_, k)| *k)
                        .unwrap_or(NumKind::Word);
                    let val_tmp = self.alloc_temp_for(val_kind);
                    self.gen_expr_to_kind(rhs, val_tmp, val_kind)?;
                    self.gen_expr_to(inner_expr, ref_tmp)?;
                    let op = match &write_kind {
                        Some((Type::Basic(BasicType::Big), _)) => Opcode::Movl,
                        Some((Type::Basic(BasicType::Real), _)) => Opcode::Movf,
                        Some((Type::Basic(_), _)) => Opcode::Movw,
                        Some((_, _)) => Opcode::Movp,
                        None => {
                            // Heuristic fallback: use rhs's ValType.
                            if self.infer_expr_type(rhs) != ValType::Word {
                                Opcode::Movp
                            } else {
                                Opcode::Movw
                            }
                        }
                    };
                    self.emit(
                        op,
                        op_fp(val_tmp),
                        mid_unused(),
                        op_fp_ind(ref_tmp, field_off),
                    );
                    Ok(())
                } else {
                    let tmp = self.alloc_temp();
                    self.gen_expr_to(rhs, tmp)
                }
            }
            Expr::CompoundAssign(lhs, op, rhs, _) => {
                if let Expr::Ident(name, _) = lhs.as_ref() {
                    // The lvalue may be a frame local or a module-level
                    // (MP-resident) variable; both are legal Dis destinations.
                    let (slot, vt, kind) = match self.lookup_var(name) {
                        Some(v) => v,
                        None => {
                            let off = self.alloc_local(name, ValType::Word, NumKind::Word);
                            (Slot::Local(off), ValType::Word, NumKind::Word)
                        }
                    };
                    // String +=: use Addc with the same operand layout.
                    if vt == ValType::Ptr && *op == BinOp::Add {
                        let rhs_tmp = self.alloc_temp();
                        self.gen_expr_to(rhs, rhs_tmp)?;
                        self.emit(Opcode::Addc, op_fp(rhs_tmp), mid_unused(), slot.operand());
                        return Ok(());
                    }
                    // Numeric compound assign: dispatch by the lvalue's kind
                    // so big/real `x op= y` uses the wide opcode family.
                    let rhs_tmp = self.alloc_temp_for(kind);
                    self.gen_expr_to_kind(rhs, rhs_tmp, kind)?;
                    let opcode = match (op, kind) {
                        (BinOp::Add, NumKind::Word) => Opcode::Addw,
                        (BinOp::Add, NumKind::Big) => Opcode::Addl,
                        (BinOp::Add, NumKind::Real) => Opcode::Addf,
                        (BinOp::Sub, NumKind::Word) => Opcode::Subw,
                        (BinOp::Sub, NumKind::Big) => Opcode::Subl,
                        (BinOp::Sub, NumKind::Real) => Opcode::Subf,
                        (BinOp::Mul, NumKind::Word) => Opcode::Mulw,
                        (BinOp::Mul, NumKind::Big) => Opcode::Mull,
                        (BinOp::Mul, NumKind::Real) => Opcode::Mulf,
                        (BinOp::Div, NumKind::Word) => Opcode::Divw,
                        (BinOp::Div, NumKind::Big) => Opcode::Divl,
                        (BinOp::Div, NumKind::Real) => Opcode::Divf,
                        _ => Opcode::Addw,
                    };
                    self.emit(opcode, op_fp(rhs_tmp), mid_unused(), slot.operand());
                    Ok(())
                } else if let Expr::Index(arr_expr, idx_expr, _) = lhs.as_ref() {
                    // Array element compound assign: arr[i] op= val.
                    //   Ind* arr, ref, idx       — install heap ref
                    //   Op*  val, op_fp_ind(ref) — 2-op form: dst = dst OP src
                    let elem = self.array_elem_basic_for_expr(arr_expr);
                    let elem_kind = elem
                        .map(|b| type_num_kind(&Type::Basic(b)))
                        .unwrap_or(NumKind::Word);
                    let val_tmp = self.alloc_temp_for(elem_kind);
                    self.gen_expr_to_kind(rhs, val_tmp, elem_kind)?;
                    let idx_tmp = self.alloc_temp();
                    let arr_tmp = self.alloc_temp();
                    self.gen_expr_to(idx_expr, idx_tmp)?;
                    self.gen_expr_to(arr_expr, arr_tmp)?;
                    let ref_tmp = self.alloc_temp();
                    let (ind_op, _) = Self::array_elem_opcodes(elem);
                    self.emit(ind_op, op_fp(arr_tmp), mid_fp(ref_tmp), op_fp(idx_tmp));
                    let opcode = match (op, elem_kind) {
                        (BinOp::Add, NumKind::Word) => Opcode::Addw,
                        (BinOp::Add, NumKind::Big) => Opcode::Addl,
                        (BinOp::Add, NumKind::Real) => Opcode::Addf,
                        (BinOp::Sub, NumKind::Word) => Opcode::Subw,
                        (BinOp::Sub, NumKind::Big) => Opcode::Subl,
                        (BinOp::Sub, NumKind::Real) => Opcode::Subf,
                        (BinOp::Mul, NumKind::Word) => Opcode::Mulw,
                        (BinOp::Mul, NumKind::Big) => Opcode::Mull,
                        (BinOp::Mul, NumKind::Real) => Opcode::Mulf,
                        (BinOp::Div, NumKind::Word) => Opcode::Divw,
                        (BinOp::Div, NumKind::Big) => Opcode::Divl,
                        (BinOp::Div, NumKind::Real) => Opcode::Divf,
                        _ => Opcode::Addw,
                    };
                    self.emit(opcode, op_fp(val_tmp), mid_unused(), op_fp_ind(ref_tmp, 0));
                    Ok(())
                } else {
                    Ok(())
                }
            }
            Expr::DeclAssign(names, rhs, _) => {
                let ty = self.infer_expr_type(rhs);
                let kind = self.infer_num_kind(rhs);
                let name = names.first().map(|s| s.as_str()).unwrap_or("_");
                let off = self.alloc_local(name, ty, kind);
                // Capture array element type when the rhs is `array[N] of T`
                // so subsequent indexing picks the right opcode pair.
                if let Expr::ArrayAlloc(_, ty, _) | Expr::ArrayLit(_, Some(ty), _) = rhs.as_ref() {
                    self.local_array_elem
                        .insert(name.to_string(), (**ty).clone());
                }
                // Same for `chan of T` so Send/Recv can pick the right width.
                if let Expr::ChanAlloc(ty, _) = rhs.as_ref()
                    && let Type::Basic(b) = ty.as_ref()
                {
                    self.local_chan_elem.insert(name.to_string(), *b);
                }
                // ADT inference: the parser emits `ref Foo(...)` as
                // `Unary(Ref, Call(Ident(Foo), args))`, not RefAlloc. Detect
                // both shapes so DeclAssign records the ADT name.
                let adt_from_rhs = match rhs.as_ref() {
                    Expr::RefAlloc(ty, _, _) => Self::adt_name_for_type(ty),
                    Expr::Unary(UnaryOp::Ref, inner, _) => {
                        if let Expr::Call(callee, _, _) = inner.as_ref()
                            && let Expr::Ident(n, _) = callee.as_ref()
                            && self.adt_layouts.contains_key(n)
                        {
                            Some(n.clone())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(a) = adt_from_rhs {
                    self.local_adt_type.insert(name.to_string(), a);
                }
                self.gen_expr_to(rhs, off)
            }
            Expr::TupleDeclAssign(names, rhs, _) => {
                // For function-call rhs with a known tuple return shape, lay
                // out per-field offsets using each field's actual width.
                // Otherwise fall back to a 4-byte stride (the historical
                // behavior, accurate for word-only tuples).
                let field_types: Option<Vec<Type>> = if let Expr::Call(callee, _, _) = rhs.as_ref()
                {
                    let cname = match callee.as_ref() {
                        Expr::Ident(n, _) => Some(n.clone()),
                        _ => None,
                    };
                    cname.and_then(|n| self.func_tuple_ret.get(&n).cloned())
                } else {
                    None
                };
                let kinds: Vec<NumKind> = if let Some(types) = &field_types {
                    types.iter().map(type_num_kind).collect()
                } else {
                    names.iter().map(|_| NumKind::Word).collect()
                };
                // Allocate ret_tmp wide enough to hold the whole tuple by
                // summing field widths. Without this, big/real fields would
                // overflow into adjacent temps.
                let total: i32 = kinds.iter().map(|k| k.byte_size()).sum();
                let ret_tmp = self.next_local;
                self.next_local += total.max(4);
                self.grow_frame();
                self.gen_expr_to(rhs, ret_tmp)?;
                let mut field_off = ret_tmp;
                for (i, name) in names.iter().enumerate() {
                    let kind = kinds.get(i).copied().unwrap_or(NumKind::Word);
                    let ty = field_types
                        .as_ref()
                        .and_then(|t| t.get(i))
                        .map(|t| match t {
                            Type::Basic(BasicType::Int)
                            | Type::Basic(BasicType::Byte)
                            | Type::Basic(BasicType::Big)
                            | Type::Basic(BasicType::Real) => ValType::Word,
                            _ => ValType::Ptr,
                        })
                        .unwrap_or(ValType::Word);
                    if name != "nil" {
                        let off = self.alloc_local(name, ty, kind);
                        if field_off != off {
                            let op = match kind {
                                NumKind::Big => Opcode::Movl,
                                NumKind::Real => Opcode::Movf,
                                NumKind::Word if ty == ValType::Ptr => Opcode::Movp,
                                NumKind::Word => Opcode::Movw,
                            };
                            self.emit(op, op_fp(field_off), mid_unused(), op_fp(off));
                        }
                    }
                    field_off += kind.byte_size();
                }
                Ok(())
            }
            Expr::PostInc(inner, _) => self.gen_inc_dec(inner, true, None),
            Expr::PostDec(inner, _) => self.gen_inc_dec(inner, false, None),
            Expr::Call(_, _, _) => self.gen_call_expr(expr),
            Expr::Send(chan_expr, val_expr, _) => {
                // Size the value temp by the channel's element kind so the
                // VM's op_send (which reads `elem_size` bytes from the val
                // slot) doesn't truncate big/real messages.
                let chan_elem = if let Expr::Ident(name, _) = chan_expr.as_ref() {
                    self.local_chan_elem.get(name).copied()
                } else {
                    None
                };
                let val_kind = chan_elem
                    .map(|b| type_num_kind(&Type::Basic(b)))
                    .unwrap_or_else(|| self.infer_num_kind(val_expr));
                let chan_tmp = self.alloc_temp();
                let val_tmp = self.alloc_temp_for(val_kind);
                self.gen_expr_to(chan_expr, chan_tmp)?;
                self.gen_expr_to_kind(val_expr, val_tmp, val_kind)?;
                self.emit(Opcode::Send, op_fp(val_tmp), mid_unused(), op_fp(chan_tmp));
                Ok(())
            }
            _ => {
                if let Expr::ModQual(_, _, _) = expr {
                    return self.gen_call_expr(expr);
                }
                let tmp = self.alloc_temp();
                self.gen_expr_to(expr, tmp)
            }
        }
    }

    fn gen_expr_to(&mut self, expr: &Expr, dst: i32) -> Result<(), String> {
        match expr {
            Expr::IntLit(v, _) => {
                if *v > i32::MAX as i64 || *v < i32::MIN as i64 {
                    // Big literal (64-bit): store in data section
                    let mp_off = self.alloc_mp(8);
                    self.data.push(DataItem::Bigs {
                        offset: mp_off,
                        values: vec![*v],
                    });
                    self.emit(Opcode::Movl, op_mp(mp_off), mid_unused(), op_fp(dst));
                } else {
                    self.gen_word_const_to(*v as i32, dst);
                }
                Ok(())
            }
            Expr::CharLit(v, _) => {
                self.emit(Opcode::Movw, op_imm(*v), mid_unused(), op_fp(dst));
                Ok(())
            }
            Expr::RealLit(v, _) => {
                // Store real constant in data section and load from MP
                let mp_off = self.alloc_mp(8); // 8 bytes for f64
                self.data.push(DataItem::Reals {
                    offset: mp_off,
                    values: vec![*v],
                });
                self.emit(Opcode::Movf, op_mp(mp_off), mid_unused(), op_fp(dst));
                Ok(())
            }
            Expr::StringLit(s, _) => {
                let mp = self.intern_string(s);
                self.emit(Opcode::Movp, op_mp(mp), mid_unused(), op_fp(dst));
                Ok(())
            }
            Expr::Nil(_) => {
                // nil pointer = 0
                self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(dst));
                Ok(())
            }
            Expr::Ident(name, _) => {
                // Frame local or module-level variable. Big/real values carry
                // an 8-byte payload and use the matching wide move regardless
                // of the surrounding ValType (Word for both); strings, lists,
                // refs, channels and modules use the ref-counting Movp.
                if let Some((slot, ty, kind)) = self.lookup_var(name) {
                    if slot != Slot::Local(dst) {
                        self.emit(
                            mov_opcode(ty, kind),
                            slot.operand(),
                            mid_unused(),
                            op_fp(dst),
                        );
                    }
                    return Ok(());
                }
                // Module-level constant: materialize its folded value.
                if let Some(value) = self.const_value(name) {
                    let value = value
                        .map_err(|why| format!("constant `{name}` cannot be folded: {why}"))?;
                    return self.gen_const_to(&value, dst);
                }
                // Anything else used to compile to `Movw $0` — a silent zero.
                Err(format!("undefined identifier `{name}`"))
            }
            Expr::Binary(lhs, op, rhs, _) => {
                // String concatenation: string + string → Addc with 3 operands
                if *op == BinOp::Add && self.infer_expr_type(lhs) == ValType::Ptr {
                    let l = self.alloc_temp();
                    let r = self.alloc_temp();
                    self.gen_expr_to(lhs, l)?;
                    self.gen_expr_to(rhs, r)?;
                    // Addc src, mid, dst: dst = mid + src (reverse order!)
                    self.emit(Opcode::Addc, op_fp(r), mid_fp(l), op_fp(dst));
                    return Ok(());
                }
                self.gen_binary(lhs, *op, rhs, dst)
            }
            Expr::Index(arr, idx, _) => {
                let arr_ty = self.infer_expr_type(arr);
                if arr_ty == ValType::Ptr {
                    // String character read: Indc arr, idx, dst.
                    let arr_tmp = self.alloc_temp();
                    let idx_tmp = self.alloc_temp();
                    self.gen_expr_to(arr, arr_tmp)?;
                    self.gen_expr_to(idx, idx_tmp)?;
                    self.emit(Opcode::Indc, op_fp(arr_tmp), mid_fp(idx_tmp), op_fp(dst));
                } else {
                    // Array element read:
                    //   Ind* arr, ref, idx — install heap_ref at ref slot
                    //   Mov* *(ref), dst   — read through ref
                    let elem = self.array_elem_basic_for_expr(arr);
                    let arr_tmp = self.alloc_temp();
                    let idx_tmp = self.alloc_temp();
                    self.gen_expr_to(arr, arr_tmp)?;
                    self.gen_expr_to(idx, idx_tmp)?;
                    let ref_tmp = self.alloc_temp();
                    let (ind_op, mov_op) = Self::array_elem_opcodes(elem);
                    self.emit(ind_op, op_fp(arr_tmp), mid_fp(ref_tmp), op_fp(idx_tmp));
                    self.emit(mov_op, op_fp_ind(ref_tmp, 0), mid_unused(), op_fp(dst));
                }
                Ok(())
            }
            Expr::Slice(arr, lo, hi, _) => {
                let arr_tmp = self.alloc_temp();
                self.gen_expr_to(arr, arr_tmp)?;
                let lo_tmp = self.alloc_temp();
                let hi_tmp = self.alloc_temp();
                if let Some(lo_expr) = lo {
                    self.gen_expr_to(lo_expr, lo_tmp)?;
                } else {
                    self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(lo_tmp));
                }
                if let Some(hi_expr) = hi {
                    self.gen_expr_to(hi_expr, hi_tmp)?;
                } else {
                    // hi = len(arr)
                    let arr_ty = self.infer_expr_type(arr);
                    let len_op = if arr_ty == ValType::Array {
                        Opcode::Lena
                    } else {
                        Opcode::Lenc
                    };
                    self.emit(len_op, op_fp(arr_tmp), mid_unused(), op_fp(hi_tmp));
                }
                // Slicea lo, hi, arr → creates slice [lo:hi] of arr
                self.emit(
                    Opcode::Slicea,
                    op_fp(lo_tmp),
                    mid_fp(hi_tmp),
                    op_fp(arr_tmp),
                );
                if arr_tmp != dst {
                    self.emit(Opcode::Movp, op_fp(arr_tmp), mid_unused(), op_fp(dst));
                }
                Ok(())
            }
            Expr::Unary(op, inner, _) => self.gen_unary(*op, inner, dst),
            Expr::Hd(inner, _) => {
                let tmp = self.alloc_temp();
                self.gen_expr_to(inner, tmp)?;
                self.emit(Opcode::Headp, op_fp(tmp), mid_unused(), op_fp(dst));
                Ok(())
            }
            Expr::Tl(inner, _) => {
                let tmp = if let Expr::Ident(name, _) = inner.as_ref() {
                    self.get_local(name).map(|(o, _)| o).unwrap_or_else(|| {
                        let t = self.alloc_temp();
                        self.gen_expr_to(inner, t).ok();
                        t
                    })
                } else {
                    let t = self.alloc_temp();
                    self.gen_expr_to(inner, t)?;
                    t
                };
                self.emit(Opcode::Tail, op_fp(tmp), mid_unused(), op_fp(dst));
                Ok(())
            }
            Expr::Len(inner, _) => {
                let tmp = self.alloc_temp();
                self.gen_expr_to(inner, tmp)?;
                let ty = self.infer_expr_type(inner);
                let opcode = match ty {
                    ValType::Array => Opcode::Lena,
                    _ => Opcode::Lenc,
                };
                self.emit(opcode, op_fp(tmp), mid_unused(), op_fp(dst));
                Ok(())
            }
            Expr::Cons(head, tail, _) => {
                let h = self.alloc_temp();
                let t = self.alloc_temp();
                self.gen_expr_to(head, h)?;
                self.gen_expr_to(tail, t)?;
                self.emit(Opcode::Consp, op_fp(h), mid_unused(), op_fp(t));
                if t != dst {
                    self.emit(Opcode::Movp, op_fp(t), mid_unused(), op_fp(dst));
                }
                Ok(())
            }
            Expr::Load(ty, path, _) => {
                // load ModuleName path_expr
                // Determine module path from the expression
                let path_tmp = self.alloc_temp();
                self.gen_expr_to(path, path_tmp)?;

                // For $Sys, reuse existing module ref
                let module_name = if let Type::Named(qn) = ty.as_ref() {
                    qn.name.clone()
                } else {
                    "Unknown".to_string()
                };

                if module_name == "Sys" {
                    self.emit(
                        Opcode::Load,
                        op_mp(self.sys_path_mp),
                        mid_imm(0),
                        op_mp(self.sys_mp_ref),
                    );
                    self.emit(
                        Opcode::Movp,
                        op_mp(self.sys_mp_ref),
                        mid_unused(),
                        op_fp(dst),
                    );
                } else {
                    // Generic module: intern path, allocate MP ref, emit Load
                    let import_idx = self.imports.len() as i32;
                    self.imports.push(ImportModule { functions: vec![] });
                    let path_mp = self.alloc_mp(4);
                    // The path string should come from the expression (e.g., Bufio->PATH)
                    // For now, use the path_tmp which was already evaluated
                    let ref_mp = self.alloc_mp(4);
                    self.emit(Opcode::Movp, op_fp(path_tmp), mid_unused(), op_mp(path_mp));
                    self.emit(
                        Opcode::Load,
                        op_mp(path_mp),
                        mid_imm(import_idx),
                        op_mp(ref_mp),
                    );
                    self.emit(Opcode::Movp, op_mp(ref_mp), mid_unused(), op_fp(dst));
                }
                Ok(())
            }
            Expr::ModQual(module, member, _) => {
                // Module->member: for Sys->PATH, return the string "$Sys"
                if let Expr::Ident(mod_name, _) = module.as_ref()
                    && member == "PATH"
                {
                    let path = format!("${mod_name}");
                    let mp = self.intern_string(&path);
                    self.emit(Opcode::Movp, op_mp(mp), mid_unused(), op_fp(dst));
                    return Ok(());
                }
                // Constants declared in a module interface resolve to their
                // folded value.
                if let Expr::Ident(mod_name, _) = module.as_ref()
                    && let Some(Ok(value)) = self.qualified_const_value(mod_name, member)
                {
                    return self.gen_const_to(&value, dst);
                }
                self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(dst));
                Ok(())
            }
            Expr::Call(callee, args, _) => self.gen_call_with_result(callee, args, dst),
            Expr::DeclAssign(names, _, _) => {
                // Reuse the statement lowering so the local is sized by the
                // value's kind (an 8-byte real must not land in a 4-byte
                // slot) and the array/chan/ADT sidecars are recorded, then
                // copy the value out to `dst`.
                self.gen_expr_discard(expr)?;
                let name = names.first().map(|s| s.as_str()).unwrap_or("_");
                if let Some((off, ty)) = self.get_local(name) {
                    let kind = self.local_num_kind(name);
                    if off != dst {
                        self.emit(mov_opcode(ty, kind), op_fp(off), mid_unused(), op_fp(dst));
                    }
                }
                Ok(())
            }
            Expr::PostInc(inner, _) => self.gen_inc_dec(inner, true, Some(dst)),
            Expr::PostDec(inner, _) => self.gen_inc_dec(inner, false, Some(dst)),
            Expr::Cast(ty, inner, _) => {
                // Type cast: for 'array of byte string_expr' → Cvtca
                if let Type::Array(elem) = ty.as_ref()
                    && let Type::Basic(BasicType::Byte) = elem.as_ref()
                {
                    self.gen_expr_to(inner, dst)?;
                    self.emit(Opcode::Cvtca, op_fp(dst), mid_unused(), op_fp(dst));
                    return Ok(());
                }
                match ty.as_ref() {
                    Type::Basic(BasicType::Int) => {
                        // int(x) — narrow to word, picking the converter by
                        // the inner expression's actual kind.
                        let inner_kind = self.infer_num_kind(inner);
                        let inner_ty = self.infer_expr_type(inner);
                        match inner_kind {
                            NumKind::Word => {
                                self.gen_expr_to(inner, dst)?;
                                if inner_ty == ValType::Ptr {
                                    // string to int: Cvtcw
                                    self.emit(Opcode::Cvtcw, op_fp(dst), mid_unused(), op_fp(dst));
                                }
                            }
                            NumKind::Big => {
                                let tmp = self.alloc_temp_for(NumKind::Big);
                                self.gen_expr_to(inner, tmp)?;
                                self.emit(Opcode::Cvtlw, op_fp(tmp), mid_unused(), op_fp(dst));
                            }
                            NumKind::Real => {
                                let tmp = self.alloc_temp_for(NumKind::Real);
                                self.gen_expr_to(inner, tmp)?;
                                self.emit(Opcode::Cvtfw, op_fp(tmp), mid_unused(), op_fp(dst));
                            }
                        }
                    }
                    Type::Basic(BasicType::Big) => {
                        // big(x) — widen to big, dispatching on inner kind.
                        // Skip the converter when the inner is already big.
                        match self.infer_num_kind(inner) {
                            NumKind::Word => {
                                let tmp = self.alloc_temp_for(NumKind::Word);
                                self.gen_expr_to(inner, tmp)?;
                                self.emit(Opcode::Cvtwl, op_fp(tmp), mid_unused(), op_fp(dst));
                            }
                            NumKind::Big => {
                                self.gen_expr_to(inner, dst)?;
                            }
                            NumKind::Real => {
                                let tmp = self.alloc_temp_for(NumKind::Real);
                                self.gen_expr_to(inner, tmp)?;
                                self.emit(Opcode::Cvtfl, op_fp(tmp), mid_unused(), op_fp(dst));
                            }
                        }
                    }
                    Type::Basic(BasicType::Real) => match self.infer_num_kind(inner) {
                        NumKind::Word => {
                            let tmp = self.alloc_temp_for(NumKind::Word);
                            self.gen_expr_to(inner, tmp)?;
                            self.emit(Opcode::Cvtwf, op_fp(tmp), mid_unused(), op_fp(dst));
                        }
                        NumKind::Big => {
                            let tmp = self.alloc_temp_for(NumKind::Big);
                            self.gen_expr_to(inner, tmp)?;
                            self.emit(Opcode::Cvtlf, op_fp(tmp), mid_unused(), op_fp(dst));
                        }
                        NumKind::Real => {
                            self.gen_expr_to(inner, dst)?;
                        }
                    },
                    Type::Basic(BasicType::String) => {
                        // string x — various conversions
                        let inner_ty = self.infer_expr_type(inner);
                        if inner_ty == ValType::Array {
                            // string array_of_byte → Cvtac
                            self.gen_expr_to(inner, dst)?;
                            self.emit(Opcode::Cvtac, op_fp(dst), mid_unused(), op_fp(dst));
                        } else {
                            // int to string: Cvtwc
                            self.gen_expr_to(inner, dst)?;
                            self.emit(Opcode::Cvtwc, op_fp(dst), mid_unused(), op_fp(dst));
                        }
                    }
                    _ => {
                        self.gen_expr_to(inner, dst)?;
                    }
                }
                Ok(())
            }
            Expr::ArrayAlloc(size, _, _) => {
                let sz_tmp = self.alloc_temp();
                self.gen_expr_to(size, sz_tmp)?;
                self.emit(Opcode::Newa, op_fp(sz_tmp), mid_imm(0), op_fp(dst));
                Ok(())
            }
            Expr::RefAlloc(ty, args, _) => {
                // ref Adt(arg1, arg2, ...) → New $type, dst; then fill fields
                // at the ADT's actual layout offsets with kind-aware Mov so
                // big/real fields land in 8-byte slots. When the ADT layout
                // is unknown (or the type is non-Named), fall back to the
                // historical i*4 / Movw layout.
                self.emit(Opcode::New, op_imm(1), mid_unused(), op_fp(dst));
                let layout =
                    Self::adt_name_for_type(ty).and_then(|a| self.adt_layouts.get(&a).cloned());
                for (i, arg) in args.iter().enumerate() {
                    let (field_off, field_ty) = match layout.as_ref().and_then(|l| l.get(i)) {
                        Some((_, t, off)) => (*off, Some(t.clone())),
                        None => ((i as i32) * 4, None),
                    };
                    let kind = field_ty
                        .as_ref()
                        .map(type_num_kind)
                        .unwrap_or(NumKind::Word);
                    let arg_tmp = self.alloc_temp_for(kind);
                    self.gen_expr_to_kind(arg, arg_tmp, kind)?;
                    let op = match field_ty.as_ref() {
                        Some(Type::Basic(BasicType::Big)) => Opcode::Movl,
                        Some(Type::Basic(BasicType::Real)) => Opcode::Movf,
                        Some(Type::Basic(_)) => Opcode::Movw,
                        Some(_) => Opcode::Movp,
                        None => {
                            // Heuristic fallback when the ADT layout is unknown.
                            if self.infer_expr_type(arg) != ValType::Word {
                                Opcode::Movp
                            } else {
                                Opcode::Movw
                            }
                        }
                    };
                    self.emit(op, op_fp(arg_tmp), mid_unused(), op_fp_ind(dst, field_off));
                }
                Ok(())
            }
            Expr::Dot(inner, field, _) => {
                // expr.field → read through the ref pointer using the ADT
                // layout when known. Falls back to the historical heuristic
                // (`estimate_field_offset` + Movw) for unknown types so the
                // existing 155 Inferno programs keep compiling.
                let ref_tmp = self.alloc_temp();
                self.gen_expr_to(inner, ref_tmp)?;
                let (field_off, mov_op) = match self
                    .adt_name_for_expr(inner)
                    .and_then(|a| self.adt_field_info(&a, field))
                {
                    Some((off, ty)) => {
                        let op = match &ty {
                            Type::Basic(BasicType::Big) => Opcode::Movl,
                            Type::Basic(BasicType::Real) => Opcode::Movf,
                            Type::Basic(_) => Opcode::Movw,
                            _ => Opcode::Movp,
                        };
                        (off, op)
                    }
                    None => (self.estimate_field_offset(inner, field), Opcode::Movw),
                };
                self.emit(
                    mov_op,
                    op_fp_ind(ref_tmp, field_off),
                    mid_unused(),
                    op_fp(dst),
                );
                Ok(())
            }
            Expr::ChanAlloc(ty, _) => {
                // chan of T → Newc{w/b/l/f/p} $0, dst, picking the opcode
                // by element width so Send/Recv copy the right number of
                // bytes per message.
                let elem = match ty.as_ref() {
                    Type::Basic(b) => Some(*b),
                    _ => None,
                };
                self.emit(newc_opcode(elem), op_unused(), mid_imm(0), op_fp(dst));
                Ok(())
            }
            Expr::Recv(chan_expr, _) => {
                // <-chan → Recv chan, dst. The dst slot is the receiver's
                // local; its size is set by the surrounding kind-aware
                // alloc, so the channel's elem_size and dst's slot size
                // align as long as the Limbo program is type-clean.
                let chan_tmp = self.alloc_temp();
                self.gen_expr_to(chan_expr, chan_tmp)?;
                self.emit(Opcode::Recv, op_fp(chan_tmp), mid_unused(), op_fp(dst));
                Ok(())
            }
            Expr::Send(chan_expr, val_expr, _) => {
                // chan <-= val → Send val, chan
                let chan_tmp = self.alloc_temp();
                let val_tmp = self.alloc_temp();
                self.gen_expr_to(chan_expr, chan_tmp)?;
                self.gen_expr_to(val_expr, val_tmp)?;
                self.emit(Opcode::Send, op_fp(val_tmp), mid_unused(), op_fp(chan_tmp));
                self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(dst));
                Ok(())
            }
            Expr::ListLit(elems, _) => {
                // list of { e1, e2, e3 } → cons chain
                // Build in reverse: nil, cons(e3, nil), cons(e2, ...), cons(e1, ...)
                self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(dst)); // nil
                for elem in elems.iter().rev() {
                    let elem_tmp = self.alloc_temp();
                    self.gen_expr_to(elem, elem_tmp)?;
                    self.emit(Opcode::Consp, op_fp(elem_tmp), mid_unused(), op_fp(dst));
                }
                Ok(())
            }
            Expr::Assign(lhs, rhs, _) => {
                if let Expr::Ident(name, _) = lhs.as_ref() {
                    return self.gen_assign_to_ident(name, rhs, Some(dst));
                }
                self.gen_expr_to(rhs, dst)
            }
            _ => {
                self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(dst));
                Ok(())
            }
        }
    }

    /// Store `rhs` into the variable `name`, which may be a frame local, a
    /// module-level variable, or (for the historical `x = expr;`-without-a-
    /// declaration form) a local created on the spot. The new local is sized
    /// by the value's kind so an 8-byte value never lands in a 4-byte slot.
    /// When `dst` is `Some`, the assigned value is also left there so the
    /// assignment can be used as an expression.
    fn gen_assign_to_ident(
        &mut self,
        name: &str,
        rhs: &Expr,
        dst: Option<i32>,
    ) -> Result<(), String> {
        let (slot, ty, kind) = match self.lookup_var(name) {
            Some(v) => v,
            None => {
                let ty = self.infer_expr_type(rhs);
                let kind = self.infer_num_kind(rhs);
                let off = self.alloc_local(name, ty, kind);
                (Slot::Local(off), ty, kind)
            }
        };
        let mov = mov_opcode(ty, kind);
        match slot {
            Slot::Local(off) => {
                self.gen_expr_to_kind(rhs, off, kind)?;
                if let Some(d) = dst
                    && d != off
                {
                    self.emit(mov, op_fp(off), mid_unused(), op_fp(d));
                }
            }
            Slot::Global(mp_off) => {
                // MP destinations can't be written by every expression form,
                // so stage the value in a frame temp and move it across.
                let tmp = self.alloc_temp_for(kind);
                self.gen_expr_to_kind(rhs, tmp, kind)?;
                self.emit(mov, op_fp(tmp), mid_unused(), op_mp(mp_off));
                if let Some(d) = dst {
                    self.emit(mov, op_fp(tmp), mid_unused(), op_fp(d));
                }
            }
        }
        Ok(())
    }

    /// Generate `x++` / `x--`. When `dst` is `Some`, the *old* value is copied
    /// there first, which is the value `y := x++` must produce.
    ///
    /// NOTE: the parser lowers prefix `++x` to the same `PostInc` node, so a
    /// prefix form in value context yields the pre-increment value too;
    /// distinguishing the two needs an AST change in the parser.
    fn gen_inc_dec(&mut self, inner: &Expr, inc: bool, dst: Option<i32>) -> Result<(), String> {
        // Resolve the lvalue to (target operand, read opcode, kind).
        let (target, mov, kind) = match inner {
            Expr::Ident(name, _) => {
                let Some((slot, ty, kind)) = self.lookup_var(name) else {
                    return Err(format!("undefined variable `{name}` in `++`/`--`"));
                };
                if let Some(d) = dst
                    && slot == Slot::Local(d)
                {
                    // Value and storage share a slot; only the update is left.
                    self.emit_inc_dec(slot.operand(), kind, inc);
                    return Ok(());
                }
                (slot.operand(), mov_opcode(ty, kind), kind)
            }
            Expr::Index(arr, idx, _) => {
                if self.infer_expr_type(arr) != ValType::Array {
                    // String character: read with Indc, bump, write back with
                    // Insc. The write-back targets the string variable itself
                    // when there is one, so a fresh string id is not lost.
                    let arr_slot = match arr.as_ref() {
                        Expr::Ident(name, _) => self.lookup_var(name).map(|(slot, _, _)| slot),
                        _ => None,
                    };
                    let arr_tmp = self.alloc_temp();
                    let idx_tmp = self.alloc_temp();
                    self.gen_expr_to(arr, arr_tmp)?;
                    self.gen_expr_to(idx, idx_tmp)?;
                    let val_tmp = self.alloc_temp();
                    self.emit(
                        Opcode::Indc,
                        op_fp(arr_tmp),
                        mid_fp(idx_tmp),
                        op_fp(val_tmp),
                    );
                    if let Some(d) = dst {
                        self.emit(Opcode::Movw, op_fp(val_tmp), mid_unused(), op_fp(d));
                    }
                    let opc = if inc { Opcode::Addw } else { Opcode::Subw };
                    self.emit(opc, op_imm(1), mid_unused(), op_fp(val_tmp));
                    let target = arr_slot.map(|s| s.operand()).unwrap_or(op_fp(arr_tmp));
                    self.emit(Opcode::Insc, op_fp(val_tmp), mid_fp(idx_tmp), target);
                    return Ok(());
                }
                // Install a heap ref for the element, then update in place.
                let elem = self.array_elem_basic_for_expr(arr);
                let kind = elem
                    .map(|b| type_num_kind(&Type::Basic(b)))
                    .unwrap_or(NumKind::Word);
                let (ind_op, mov_op) = Self::array_elem_opcodes(elem);
                let arr_tmp = self.alloc_temp();
                let idx_tmp = self.alloc_temp();
                self.gen_expr_to(arr, arr_tmp)?;
                self.gen_expr_to(idx, idx_tmp)?;
                let ref_tmp = self.alloc_temp();
                self.emit(ind_op, op_fp(arr_tmp), mid_fp(ref_tmp), op_fp(idx_tmp));
                (op_fp_ind(ref_tmp, 0), mov_op, kind)
            }
            Expr::Dot(obj, field, _) => {
                let ref_tmp = self.alloc_temp();
                self.gen_expr_to(obj, ref_tmp)?;
                match self
                    .adt_name_for_expr(obj)
                    .and_then(|a| self.adt_field_info(&a, field))
                {
                    Some((off, ty)) => {
                        let kind = type_num_kind(&ty);
                        let mov = match &ty {
                            Type::Basic(BasicType::Big) => Opcode::Movl,
                            Type::Basic(BasicType::Real) => Opcode::Movf,
                            _ => Opcode::Movw,
                        };
                        (op_fp_ind(ref_tmp, off), mov, kind)
                    }
                    None => (
                        op_fp_ind(ref_tmp, self.estimate_field_offset(obj, field)),
                        Opcode::Movw,
                        NumKind::Word,
                    ),
                }
            }
            _ => return Err("`++`/`--` needs a variable, element or field".to_string()),
        };
        // Post-increment yields the value *before* the update.
        if let Some(d) = dst {
            self.emit(mov, target, mid_unused(), op_fp(d));
        }
        self.emit_inc_dec(target, kind, inc);
        Ok(())
    }

    fn gen_binary(&mut self, lhs: &Expr, op: BinOp, rhs: &Expr, dst: i32) -> Result<(), String> {
        // Comparison operators
        match op {
            BinOp::Eq | BinOp::Neq | BinOp::Lt | BinOp::Gt | BinOp::Leq | BinOp::Geq => {
                let lt = self.infer_expr_type(lhs);
                // Pointer comparisons (string ==/!=) compare 4-byte ids.
                if lt == ValType::Ptr && matches!(op, BinOp::Eq | BinOp::Neq) {
                    let l = self.alloc_temp();
                    let r = self.alloc_temp();
                    self.gen_expr_to(lhs, l)?;
                    self.gen_expr_to(rhs, r)?;
                    self.emit(Opcode::Movw, op_imm(1), mid_unused(), op_fp(dst));
                    let branch = if op == BinOp::Eq {
                        Opcode::Beqc
                    } else {
                        Opcode::Bnec
                    };
                    let skip = self.code.len();
                    self.emit(branch, op_fp(l), mid_fp(r), op_imm(0));
                    self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(dst));
                    self.code[skip].destination = op_imm(self.code.len() as i32);
                    return Ok(());
                }
                // Numeric comparisons promote both operands to the wider kind
                // and pick the matching branch opcode (Beqw/Beql/Beqf, etc.).
                let kind = self.infer_num_kind(lhs).max(self.infer_num_kind(rhs));
                let l = self.alloc_temp_for(kind);
                let r = self.alloc_temp_for(kind);
                self.gen_expr_to_kind(lhs, l, kind)?;
                self.gen_expr_to_kind(rhs, r, kind)?;
                self.emit(Opcode::Movw, op_imm(1), mid_unused(), op_fp(dst));
                let branch = match (op, kind) {
                    (BinOp::Eq, NumKind::Word) => Opcode::Beqw,
                    (BinOp::Eq, NumKind::Big) => Opcode::Beql,
                    (BinOp::Eq, NumKind::Real) => Opcode::Beqf,
                    (BinOp::Neq, NumKind::Word) => Opcode::Bnew,
                    (BinOp::Neq, NumKind::Big) => Opcode::Bnel,
                    (BinOp::Neq, NumKind::Real) => Opcode::Bnef,
                    (BinOp::Lt, NumKind::Word) => Opcode::Bltw,
                    (BinOp::Lt, NumKind::Big) => Opcode::Bltl,
                    (BinOp::Lt, NumKind::Real) => Opcode::Bltf,
                    (BinOp::Gt, NumKind::Word) => Opcode::Bgtw,
                    (BinOp::Gt, NumKind::Big) => Opcode::Bgtl,
                    (BinOp::Gt, NumKind::Real) => Opcode::Bgtf,
                    (BinOp::Leq, NumKind::Word) => Opcode::Blew,
                    (BinOp::Leq, NumKind::Big) => Opcode::Blel,
                    (BinOp::Leq, NumKind::Real) => Opcode::Blef,
                    (BinOp::Geq, NumKind::Word) => Opcode::Bgew,
                    (BinOp::Geq, NumKind::Big) => Opcode::Bgel,
                    (BinOp::Geq, NumKind::Real) => Opcode::Bgef,
                    _ => Opcode::Beqw,
                };
                let skip = self.code.len();
                self.emit(branch, op_fp(l), mid_fp(r), op_imm(0));
                self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(dst));
                self.code[skip].destination = op_imm(self.code.len() as i32);
                return Ok(());
            }
            BinOp::LogAnd => return self.gen_logical_and(lhs, rhs, dst),
            BinOp::LogOr => return self.gen_logical_or(lhs, rhs, dst),
            _ => {}
        }

        // Shift and exponent ops have an asymmetric kind contract: the base
        // (lhs) carries the operand kind, but the shift count / exponent
        // (rhs) is always a Word. Handle them separately so we don't widen
        // the count's temp slot.
        if matches!(op, BinOp::Lshift | BinOp::Rshift | BinOp::Power) {
            let lhs_kind = self.infer_num_kind(lhs);
            let l = self.alloc_temp_for(lhs_kind);
            let r = self.alloc_temp_for(NumKind::Word);
            self.gen_expr_to_kind(lhs, l, lhs_kind)?;
            self.gen_expr_to_kind(rhs, r, NumKind::Word)?;
            let opcode = match (op, lhs_kind) {
                (BinOp::Lshift, NumKind::Word) => Opcode::Shlw,
                (BinOp::Lshift, NumKind::Big) => Opcode::Shll,
                (BinOp::Rshift, NumKind::Word) => Opcode::Shrw,
                (BinOp::Rshift, NumKind::Big) => Opcode::Shrl,
                (BinOp::Power, NumKind::Word) => Opcode::Expw,
                (BinOp::Power, NumKind::Big) => Opcode::Expl,
                (BinOp::Power, NumKind::Real) => Opcode::Expf,
                _ => return Err(format!("unsupported {op:?} on real-typed operand")),
            };
            // 3-op: src = exponent/count (word), mid = base (kind), dst = result.
            self.emit(opcode, op_fp(r), mid_fp(l), op_fp(dst));
            return Ok(());
        }

        let kind = self.infer_num_kind(lhs).max(self.infer_num_kind(rhs));
        let l = self.alloc_temp_for(kind);
        let r = self.alloc_temp_for(kind);
        // Use kind-aware gen_expr_to so a narrower operand (e.g. an int
        // literal in `big_var + 1`) is widened via Cvtwl/Cvtwf rather than
        // leaving the high bytes of the wide temp uninitialized.
        self.gen_expr_to_kind(lhs, l, kind)?;
        self.gen_expr_to_kind(rhs, r, kind)?;
        let opcode = match (op, kind) {
            (BinOp::Add, NumKind::Word) => Opcode::Addw,
            (BinOp::Add, NumKind::Big) => Opcode::Addl,
            (BinOp::Add, NumKind::Real) => Opcode::Addf,
            (BinOp::Sub, NumKind::Word) => Opcode::Subw,
            (BinOp::Sub, NumKind::Big) => Opcode::Subl,
            (BinOp::Sub, NumKind::Real) => Opcode::Subf,
            (BinOp::Mul, NumKind::Word) => Opcode::Mulw,
            (BinOp::Mul, NumKind::Big) => Opcode::Mull,
            (BinOp::Mul, NumKind::Real) => Opcode::Mulf,
            (BinOp::Div, NumKind::Word) => Opcode::Divw,
            (BinOp::Div, NumKind::Big) => Opcode::Divl,
            (BinOp::Div, NumKind::Real) => Opcode::Divf,
            (BinOp::Mod, NumKind::Word) => Opcode::Modw,
            (BinOp::Mod, NumKind::Big) => Opcode::Modl,
            // No Modf in Dis: Limbo programs use math->fmod for real %.
            (BinOp::Mod, NumKind::Real) => {
                return Err("real % real has no Dis opcode; use math->fmod".to_string());
            }
            (BinOp::And, NumKind::Word) => Opcode::Andw,
            (BinOp::And, NumKind::Big) => Opcode::Andl,
            (BinOp::Or, NumKind::Word) => Opcode::Orw,
            (BinOp::Or, NumKind::Big) => Opcode::Orl,
            (BinOp::Xor, NumKind::Word) => Opcode::Xorw,
            (BinOp::Xor, NumKind::Big) => Opcode::Xorl,
            (BinOp::And | BinOp::Or | BinOp::Xor, NumKind::Real) => {
                return Err(format!("bitwise {op:?} on real operand is not valid"));
            }
            _ => Opcode::Movw,
        };
        // The 3-operand form computes `dst = mid OP src` for Sub/Div/Mod in
        // the reference Dis VM (xec.c), so place lhs in mid and rhs in src.
        // Commutative ops (Add/Mul/And/Or/Xor) are unaffected by the order.
        self.emit(opcode, op_fp(r), mid_fp(l), op_fp(dst));
        Ok(())
    }

    fn gen_logical_and(&mut self, lhs: &Expr, rhs: &Expr, dst: i32) -> Result<(), String> {
        let l = self.alloc_temp();
        self.gen_cond_to(lhs, l)?;
        let short = self.code.len();
        self.emit(Opcode::Beqw, op_fp(l), mid_imm(0), op_imm(0));
        self.gen_cond_to(rhs, dst)?;
        let end = self.code.len();
        self.emit(Opcode::Jmp, op_unused(), mid_unused(), op_imm(0));
        self.code[short].destination = op_imm(self.code.len() as i32);
        self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(dst));
        self.code[end].destination = op_imm(self.code.len() as i32);
        Ok(())
    }

    fn gen_logical_or(&mut self, lhs: &Expr, rhs: &Expr, dst: i32) -> Result<(), String> {
        let l = self.alloc_temp();
        self.gen_cond_to(lhs, l)?;
        let short = self.code.len();
        self.emit(Opcode::Bnew, op_fp(l), mid_imm(0), op_imm(0));
        self.gen_cond_to(rhs, dst)?;
        let end = self.code.len();
        self.emit(Opcode::Jmp, op_unused(), mid_unused(), op_imm(0));
        self.code[short].destination = op_imm(self.code.len() as i32);
        self.emit(Opcode::Movw, op_imm(1), mid_unused(), op_fp(dst));
        self.code[end].destination = op_imm(self.code.len() as i32);
        Ok(())
    }

    fn gen_unary(&mut self, op: UnaryOp, inner: &Expr, dst: i32) -> Result<(), String> {
        let kind = self.infer_num_kind(inner);
        match op {
            UnaryOp::Neg => {
                // Compute `0 - inner` using the matching opcode family. Dis
                // sub semantics: `dst = mid - src`, so emit src=inner_tmp,
                // mid=zero_tmp.
                let inner_tmp = self.alloc_temp_for(kind);
                let zero_tmp = self.alloc_temp_for(kind);
                self.gen_expr_to(inner, inner_tmp)?;
                let (mov_zero, sub) = match kind {
                    NumKind::Word => (Opcode::Movw, Opcode::Subw),
                    NumKind::Big => (Opcode::Movl, Opcode::Subl),
                    NumKind::Real => (Opcode::Movf, Opcode::Subf),
                };
                if kind == NumKind::Real {
                    // Real has no immediate move; use a cast from word 0.
                    let z_word = self.alloc_temp_for(NumKind::Word);
                    self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(z_word));
                    self.emit(Opcode::Cvtwf, op_fp(z_word), mid_unused(), op_fp(zero_tmp));
                } else if kind == NumKind::Big {
                    let z_word = self.alloc_temp_for(NumKind::Word);
                    self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(z_word));
                    self.emit(Opcode::Cvtwl, op_fp(z_word), mid_unused(), op_fp(zero_tmp));
                } else {
                    self.emit(mov_zero, op_imm(0), mid_unused(), op_fp(zero_tmp));
                }
                self.emit(sub, op_fp(inner_tmp), mid_fp(zero_tmp), op_fp(dst));
            }
            UnaryOp::Not => {
                // `!x` is a boolean negation: x == 0 ? 1 : 0. Always emits a
                // word boolean regardless of the inner kind.
                self.gen_expr_to(inner, dst)?;
                let skip = self.code.len();
                self.emit(Opcode::Beqw, op_fp(dst), mid_imm(0), op_imm(0));
                self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(dst));
                let end = self.code.len();
                self.emit(Opcode::Jmp, op_unused(), mid_unused(), op_imm(0));
                self.code[skip].destination = op_imm(self.code.len() as i32);
                self.emit(Opcode::Movw, op_imm(1), mid_unused(), op_fp(dst));
                self.code[end].destination = op_imm(self.code.len() as i32);
            }
            UnaryOp::BitNot => {
                // `~x = x ^ -1`. Big variant uses Xorl with a sign-extended
                // -1 in a wide temp.
                self.gen_expr_to(inner, dst)?;
                match kind {
                    NumKind::Word => {
                        let t = self.alloc_temp();
                        self.emit(Opcode::Movw, op_imm(-1), mid_unused(), op_fp(t));
                        self.emit(Opcode::Xorw, op_fp(t), mid_unused(), op_fp(dst));
                    }
                    NumKind::Big => {
                        let z_word = self.alloc_temp_for(NumKind::Word);
                        let t = self.alloc_temp_for(NumKind::Big);
                        self.emit(Opcode::Movw, op_imm(-1), mid_unused(), op_fp(z_word));
                        self.emit(Opcode::Cvtwl, op_fp(z_word), mid_unused(), op_fp(t));
                        self.emit(Opcode::Xorl, op_fp(t), mid_unused(), op_fp(dst));
                    }
                    NumKind::Real => {
                        return Err("bitwise NOT on real operand is not valid".to_string());
                    }
                }
            }
            UnaryOp::Ref => {
                // The parser emits `ref TypeName(args)` as
                // `Unary(Ref, Call(Ident(TypeName), args))`. If the callee
                // resolves to a known ADT (not a function), treat it as
                // record allocation: New + per-field init at the ADT's
                // actual layout offsets with kind-aware Mov.
                if let Expr::Call(callee, args, _) = inner
                    && let Expr::Ident(name, _) = callee.as_ref()
                    && self.adt_layouts.contains_key(name)
                {
                    self.emit(Opcode::New, op_imm(1), mid_unused(), op_fp(dst));
                    let layout = self.adt_layouts.get(name).cloned();
                    for (i, arg) in args.iter().enumerate() {
                        let (field_off, field_ty) = match layout.as_ref().and_then(|l| l.get(i)) {
                            Some((_, t, off)) => (*off, Some(t.clone())),
                            None => ((i as i32) * 4, None),
                        };
                        let kind = field_ty
                            .as_ref()
                            .map(type_num_kind)
                            .unwrap_or(NumKind::Word);
                        let arg_tmp = self.alloc_temp_for(kind);
                        self.gen_expr_to_kind(arg, arg_tmp, kind)?;
                        let op = match field_ty.as_ref() {
                            Some(Type::Basic(BasicType::Big)) => Opcode::Movl,
                            Some(Type::Basic(BasicType::Real)) => Opcode::Movf,
                            Some(Type::Basic(_)) => Opcode::Movw,
                            Some(_) => Opcode::Movp,
                            None => {
                                if self.infer_expr_type(arg) != ValType::Word {
                                    Opcode::Movp
                                } else {
                                    Opcode::Movw
                                }
                            }
                        };
                        self.emit(op, op_fp(arg_tmp), mid_unused(), op_fp_ind(dst, field_off));
                    }
                } else if let Expr::Ident(name, _) = inner
                    && self.adt_layouts.contains_key(name)
                {
                    // `ref Adt` with no initialiser list: allocate the record
                    // and leave its fields zeroed.
                    self.emit(Opcode::New, op_imm(1), mid_unused(), op_fp(dst));
                } else {
                    self.gen_expr_to(inner, dst)?;
                }
            }
        }
        Ok(())
    }

    // ── Call generation ────────────────────────────────────────

    fn gen_call_expr(&mut self, expr: &Expr) -> Result<(), String> {
        if let Expr::Call(callee, args, _) = expr {
            // sys->func(args)
            if let Expr::ModQual(module, func_name, _) = callee.as_ref()
                && let Expr::Ident(mod_name, _) = module.as_ref()
                && mod_name == "sys"
            {
                return self.gen_sys_call(func_name, args, None);
            }
            // Local function call: func(args)
            if let Expr::Ident(func_name, _) = callee.as_ref() {
                return self.gen_local_call(func_name, args, None);
            }
        }
        Ok(())
    }

    fn gen_call_with_result(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        dst: i32,
    ) -> Result<(), String> {
        // sys->func(args)
        if let Expr::ModQual(module, func_name, _) = callee
            && let Expr::Ident(mod_name, _) = module.as_ref()
            && mod_name == "sys"
        {
            return self.gen_sys_call(func_name, args, Some(dst));
        }
        // Nested calls like sys->fildes(1)
        if let Expr::Call(inner_callee, inner_args, _) = callee {
            return self.gen_call_with_result(inner_callee, inner_args, dst);
        }
        // Local function call
        if let Expr::Ident(func_name, _) = callee {
            return self.gen_local_call(func_name, args, Some(dst));
        }
        self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(dst));
        Ok(())
    }

    fn gen_local_call(
        &mut self,
        func_name: &str,
        args: &[Expr],
        result_dst: Option<i32>,
    ) -> Result<(), String> {
        // Look up function in func_table
        let func_info = self
            .func_table
            .iter()
            .enumerate()
            .find(|(_, (n, _, _, _))| n == func_name)
            .map(|(i, (_, pc, _, ret))| (i, *pc, *ret));

        if let Some((func_idx, func_pc, ret_kind)) = func_info {
            let func_type = 2 + func_idx as i32;

            // Evaluate args first into temps sized by each arg's kind so
            // big/real values aren't truncated.
            let mut arg_temps: Vec<(i32, ValType, NumKind)> = Vec::new();
            for arg in args {
                let kind = self.infer_num_kind(arg);
                let tmp = self.alloc_temp_for(kind);
                self.gen_expr_to(arg, tmp)?;
                let ty = self.infer_expr_type(arg);
                arg_temps.push((tmp, ty, kind));
            }

            let frame_tmp = self.alloc_temp();
            // ret_tmp receives the callee's return value via the standard
            // pointer-installed-at-frame[16] convention. Its slot must be
            // wide enough for the return kind so 8-byte big/real values
            // don't overflow into adjacent temps.
            let ret_tmp = self.alloc_temp_for(ret_kind);

            self.emit(
                Opcode::Frame,
                op_imm(func_type),
                mid_unused(),
                op_fp(frame_tmp),
            );

            // Pack args into the callee frame at cumulative offsets matching
            // the callee's param layout (4 bytes per Word/Ptr, 8 bytes per
            // Big/Real). The callee's `param_off += kind.byte_size()` loop
            // produces matching offsets.
            let mut arg_off = 32i32;
            for (tmp, ty, kind) in &arg_temps {
                let op = match (ty, kind) {
                    (_, NumKind::Big) => Opcode::Movl,
                    (_, NumKind::Real) => Opcode::Movf,
                    (ValType::Word, NumKind::Word) => Opcode::Movw,
                    _ => Opcode::Movp,
                };
                self.emit(op, op_fp(*tmp), mid_unused(), op_fp_ind(frame_tmp, arg_off));
                arg_off += kind.byte_size();
            }

            // When the caller wants the result, install dst directly as the
            // return target. This skips an intermediate ret_tmp + Mov pair,
            // and matters for tuple returns whose total width can exceed 8
            // bytes — a single Mov{w/l/f} couldn't copy the whole tuple.
            // The caller (e.g., TupleDeclAssign) is responsible for sizing
            // `dst` to fit the full return shape.
            let return_target = result_dst.unwrap_or(ret_tmp);
            self.emit(
                Opcode::Lea,
                op_fp(return_target),
                mid_unused(),
                op_fp_ind(frame_tmp, 16),
            );
            let call_idx = self.code.len();
            self.emit(
                Opcode::Call,
                op_fp(frame_tmp),
                mid_unused(),
                op_imm(func_pc),
            );
            if func_pc < 0 {
                // Forward reference: pre-registered placeholder PC. Patch
                // after the callee is generated.
                self.pending_call_fixups
                    .push((call_idx, func_name.to_string()));
            }
            // Suppress unused-variable warning when result_dst is None.
            let _ = ret_kind;
        }
        Ok(())
    }

    fn gen_sys_call(
        &mut self,
        func_name: &str,
        args: &[Expr],
        result_dst: Option<i32>,
    ) -> Result<(), String> {
        let func_idx = self.ensure_sys_func(func_name);

        // Phase 1: Evaluate all arguments into kind-sized temps BEFORE
        // allocating the call frame. Nested calls (like sys->fildes(1))
        // complete first; each temp's width matches its arg's kind so big
        // and real values aren't truncated when packed.
        let mut arg_temps: Vec<(i32, ValType, NumKind)> = Vec::new();
        for arg in args {
            let kind = self.infer_num_kind(arg);
            let tmp = self.alloc_temp_for(kind);
            self.gen_expr_to(arg, tmp)?;
            let ty = self.infer_expr_type(arg);
            arg_temps.push((tmp, ty, kind));
        }

        // Phase 2: Allocate call frame and fill it
        let frame_tmp = self.alloc_temp();
        // Wide enough for big/real returns; pointer returns share the 4-byte
        // size of Word so ValType::Ptr sys functions are unaffected.
        let ret_tmp = self.alloc_temp_for(sys_return_kind(func_name));

        self.emit(
            Opcode::Mframe,
            op_mp(self.sys_mp_ref),
            mid_imm(func_idx as i32),
            op_fp(frame_tmp),
        );

        // Pack args at cumulative offsets so each occupies the right width
        // (4 bytes per Word/Ptr, 8 bytes per Big/Real). For varargs like
        // `sys->print`, this puts the format-string-driven $Sys formatter's
        // expected layout in place: %d reads 4 bytes, %bd 8, %f 8, etc.
        let mut arg_off = 32i32;
        for (tmp, ty, kind) in &arg_temps {
            let op = match (ty, kind) {
                (_, NumKind::Big) => Opcode::Movl,
                (_, NumKind::Real) => Opcode::Movf,
                (ValType::Word, NumKind::Word) => Opcode::Movw,
                _ => Opcode::Movp,
            };
            self.emit(op, op_fp(*tmp), mid_unused(), op_fp_ind(frame_tmp, arg_off));
            arg_off += kind.byte_size();
        }

        // Same direct-return-target trick as gen_local_call: skip the
        // intermediate ret_tmp when the caller provides a destination.
        let return_target = result_dst.unwrap_or(ret_tmp);
        self.emit(
            Opcode::Lea,
            op_fp(return_target),
            mid_unused(),
            op_fp_ind(frame_tmp, 16),
        );
        self.emit(
            Opcode::Mcall,
            op_fp(frame_tmp),
            mid_imm(func_idx as i32),
            op_mp(self.sys_mp_ref),
        );
        Ok(())
    }

    /// Estimate field offset for ADT field access.
    /// In a proper compiler, this would use the type checker. For now, use simple heuristics.
    fn estimate_field_offset(&self, _expr: &Expr, field: &str) -> i32 {
        // Common Inferno ADT field offsets
        // For Sys->FD: fd field at offset 0
        // For generic ADTs: fields are at 0, 4, 8, 12, ... (4 bytes each)
        match field {
            "x" | "min" | "fd" | "path" | "name" => 0,
            "y" | "max" | "vers" | "uid" | "offset" => 4,
            "z" | "qtype" | "gid" | "label" => 8,
            "w" | "muid" | "mode" => 12,
            "length" | "size" => 16,
            "atime" => 20,
            "mtime" => 24,
            _ => {
                // Try to parse numeric tuple field: t0, t1, t2, ...
                if let Some(idx) = field.strip_prefix('t')
                    && let Ok(n) = idx.parse::<i32>()
                {
                    return n * 4;
                }
                0
            }
        }
    }

    fn emit(&mut self, opcode: Opcode, src: Operand, mid: MiddleOperand, dst: Operand) {
        self.code.push(Instruction {
            opcode,
            source: src,
            middle: mid,
            destination: dst,
        });
    }

    /// Emit an in-place increment or decrement of the variable addressed by
    /// `target` (a frame slot or an MP slot). Picks the opcode family by
    /// `kind`: Word uses Addw/Subw with an immediate; Big and Real
    /// materialize a kind-sized `1` in a wide temp via Cvt and use Addl/Subl
    /// or Addf/Subf so the carry/precision of the high bytes is preserved.
    fn emit_inc_dec(&mut self, target: Operand, kind: NumKind, inc: bool) {
        match kind {
            NumKind::Word => {
                let opc = if inc { Opcode::Addw } else { Opcode::Subw };
                self.emit(opc, op_imm(1), mid_unused(), target);
            }
            NumKind::Big => {
                let z = self.alloc_temp_for(NumKind::Word);
                let one = self.alloc_temp_for(NumKind::Big);
                self.emit(Opcode::Movw, op_imm(1), mid_unused(), op_fp(z));
                self.emit(Opcode::Cvtwl, op_fp(z), mid_unused(), op_fp(one));
                let opc = if inc { Opcode::Addl } else { Opcode::Subl };
                // 2-op form: dst = dst OP src, so target += one (or -= one).
                self.emit(opc, op_fp(one), mid_unused(), target);
            }
            NumKind::Real => {
                let z = self.alloc_temp_for(NumKind::Word);
                let one = self.alloc_temp_for(NumKind::Real);
                self.emit(Opcode::Movw, op_imm(1), mid_unused(), op_fp(z));
                self.emit(Opcode::Cvtwf, op_fp(z), mid_unused(), op_fp(one));
                let opc = if inc { Opcode::Addf } else { Opcode::Subf };
                self.emit(opc, op_fp(one), mid_unused(), target);
            }
        }
    }
}

fn op_unused() -> Operand {
    Operand::UNUSED
}
fn op_fp(offset: i32) -> Operand {
    Operand {
        mode: AddressMode::OffsetIndirectFp,
        register1: offset,
        register2: 0,
    }
}
fn op_mp(offset: i32) -> Operand {
    Operand {
        mode: AddressMode::OffsetIndirectMp,
        register1: offset,
        register2: 0,
    }
}
fn op_imm(val: i32) -> Operand {
    Operand {
        mode: AddressMode::Immediate,
        register1: val,
        register2: 0,
    }
}
fn op_fp_ind(fp_off: i32, field_off: i32) -> Operand {
    Operand {
        mode: AddressMode::OffsetDoubleIndirectFp,
        register1: fp_off,
        register2: field_off,
    }
}
fn mid_unused() -> MiddleOperand {
    MiddleOperand::UNUSED
}
fn mid_imm(val: i32) -> MiddleOperand {
    MiddleOperand {
        mode: MiddleMode::SmallImmediate,
        register1: val,
    }
}
fn mid_fp(offset: i32) -> MiddleOperand {
    MiddleOperand {
        mode: MiddleMode::SmallOffsetFp,
        register1: offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::Span;

    fn s() -> Span {
        Span::default()
    }

    /// Helper: compile a minimal Limbo source string through lexer+parser+codegen.
    fn compile_src(src: &str) -> Result<Module, String> {
        let tokens = crate::lexer::Lexer::new(src, "test.b")
            .tokenize()
            .map_err(|e| format!("{e}"))?;
        let ast = crate::parser::Parser::new(tokens, "test.b")
            .parse_file()
            .map_err(|e| format!("{e}"))?;
        CodeGen::new().compile(&ast)
    }

    // ── Hello world ─────────────────────────────────────────────

    #[test]
    fn hello_world_produces_exports_and_imports() {
        let src = r#"
implement Test;
include "sys.m";
sys: Sys;
init(nil: ref Draw->Context, nil: list of string)
{
    sys = load Sys Sys->PATH;
    sys->print("hello world\n");
}
"#;
        let module = compile_src(src).expect("hello world should compile");
        // Should have one export: "init"
        assert_eq!(module.exports.len(), 1);
        assert_eq!(module.exports[0].name, "init");
        assert_eq!(module.exports[0].pc, 0);
        // Should have imports (sys module functions)
        assert!(!module.imports.is_empty());
        // Code should have more than just a Ret
        assert!(module.code.len() > 1);
    }

    // ── If/else produces branch instructions ────────────────────

    #[test]
    fn if_else_produces_branch_instructions() {
        let src = r#"
implement Test;
init(nil: ref Draw->Context, nil: list of string)
{
    x: int;
    x = 1;
    if(x == 0)
        x = 2;
    else
        x = 3;
}
"#;
        let module = compile_src(src).expect("if/else should compile");
        // Should contain a Beqw (branch-equal-word) instruction
        let has_beqw = module.code.iter().any(|i| i.opcode == Opcode::Beqw);
        assert!(has_beqw, "if/else should produce a Beqw instruction");
        // Should also contain a Jmp for the else skip
        let has_jmp = module.code.iter().any(|i| i.opcode == Opcode::Jmp);
        assert!(
            has_jmp,
            "if/else should produce a Jmp instruction for the else branch"
        );
    }

    // ── While loop produces jump-back ───────────────────────────

    #[test]
    fn while_loop_produces_jump_back() {
        let src = r#"
implement Test;
init(nil: ref Draw->Context, nil: list of string)
{
    x := 0;
    while(x < 10)
        x++;
}
"#;
        let module = compile_src(src).expect("while loop should compile");
        // Should have a Jmp instruction that jumps back (to an earlier PC)
        let jmp_instrs: Vec<_> = module
            .code
            .iter()
            .enumerate()
            .filter(|(_, i)| i.opcode == Opcode::Jmp)
            .collect();
        assert!(!jmp_instrs.is_empty(), "while loop should produce a Jmp");
        // The Jmp target should be before the Jmp itself (jump back to loop start)
        for (idx, inst) in &jmp_instrs {
            if inst.destination.mode == AddressMode::Immediate {
                let target = inst.destination.register1 as usize;
                if target < *idx {
                    return; // Found a backwards jump — test passes
                }
            }
        }
        panic!("while loop should produce a backwards Jmp (jump-back)");
    }

    // ── String concatenation produces Addc ──────────────────────

    #[test]
    fn string_concat_produces_addc() {
        let src = r#"
implement Test;
init(nil: ref Draw->Context, nil: list of string)
{
    s := "hello";
    s = s + " world";
}
"#;
        let module = compile_src(src).expect("string concat should compile");
        let has_addc = module.code.iter().any(|i| i.opcode == Opcode::Addc);
        assert!(has_addc, "string concatenation should produce Addc opcode");
    }

    // ── Channel creation produces Newcw ─────────────────────────

    #[test]
    fn channel_creation_produces_newcw() {
        let src = r#"
implement Test;
init(nil: ref Draw->Context, nil: list of string)
{
    c := chan of int;
}
"#;
        let module = compile_src(src).expect("channel creation should compile");
        let has_newcw = module.code.iter().any(|i| i.opcode == Opcode::Newcw);
        assert!(has_newcw, "channel creation should produce Newcw opcode");
    }

    // ── Local function calls produce Call ────────────────────────

    #[test]
    fn local_function_call_produces_call() {
        let src = r#"
implement Test;
helper(): int
{
    return 42;
}
init(nil: ref Draw->Context, nil: list of string)
{
    x := helper();
}
"#;
        let module = compile_src(src).expect("local function call should compile");
        let has_call = module.code.iter().any(|i| i.opcode == Opcode::Call);
        assert!(has_call, "local function call should produce Call opcode");
        // Should also have a Frame instruction to set up the call frame
        let has_frame = module.code.iter().any(|i| i.opcode == Opcode::Frame);
        assert!(has_frame, "local function call should produce Frame opcode");
    }

    // ── Return value writes through return pointer ──────────────

    #[test]
    fn return_value_writes_through_return_pointer() {
        let src = r#"
implement Test;
answer(): int
{
    return 42;
}
init(nil: ref Draw->Context, nil: list of string)
{
    x := answer();
}
"#;
        let module = compile_src(src).expect("return value should compile");
        // The return statement should produce Movw to 0(16(fp)) — double indirect through fp[16]
        let has_movw_double_ind = module.code.iter().any(|i| {
            i.opcode == Opcode::Movw
                && i.destination.mode == AddressMode::OffsetDoubleIndirectFp
                && i.destination.register1 == 16
                && i.destination.register2 == 0
        });
        assert!(
            has_movw_double_ind,
            "return value should write through 0(16(fp)) — the return pointer"
        );
    }

    // ── AST-level codegen ───────────────────────────────────────

    #[test]
    fn compile_empty_init() {
        let file = SourceFile {
            implement: vec!["Test".to_string()],
            includes: vec![],
            decls: vec![Decl::Func(FuncDecl {
                name: QualName {
                    qualifier: None,
                    name: "init".to_string(),
                },
                sig: FuncSig {
                    name: "init".to_string(),
                    params: vec![],
                    ret: None,
                    span: s(),
                },
                body: Block {
                    stmts: vec![],
                    span: s(),
                },
                span: s(),
            })],
        };
        let module = CodeGen::new().compile(&file).expect("empty init compiles");
        assert_eq!(module.name, "Test");
        assert_eq!(module.exports.len(), 1);
        assert_eq!(module.exports[0].name, "init");
        // Should have at least a Ret instruction
        assert!(!module.code.is_empty());
        assert_eq!(module.code.last().unwrap().opcode, Opcode::Ret);
    }

    #[test]
    fn compile_integer_assignment() {
        let file = SourceFile {
            implement: vec!["Test".to_string()],
            includes: vec![],
            decls: vec![Decl::Func(FuncDecl {
                name: QualName {
                    qualifier: None,
                    name: "init".to_string(),
                },
                sig: FuncSig {
                    name: "init".to_string(),
                    params: vec![],
                    ret: None,
                    span: s(),
                },
                body: Block {
                    stmts: vec![Stmt::Expr(Expr::DeclAssign(
                        vec!["x".to_string()],
                        Box::new(Expr::IntLit(42, s())),
                        s(),
                    ))],
                    span: s(),
                },
                span: s(),
            })],
        };
        let module = CodeGen::new().compile(&file).expect("int assign compiles");
        // Should contain Movw with immediate 42
        let has_movw_42 = module.code.iter().any(|i| {
            i.opcode == Opcode::Movw
                && i.source.mode == AddressMode::Immediate
                && i.source.register1 == 42
        });
        assert!(has_movw_42, "should have Movw $42");
    }

    // ── Unsupported constructs are diagnosed, not silently dropped ──

    #[test]
    fn alt_statement_is_a_compile_error() {
        let src = r#"
implement Test;
init(nil: ref Draw->Context, nil: list of string)
{
    c := chan of int;
    alt {
    v := <-c =>
        v = 0;
    }
}
"#;
        let err = compile_src(src).expect_err("alt must not compile to nothing");
        assert!(err.contains("alt"), "diagnostic should mention alt: {err}");
    }

    #[test]
    fn pick_statement_is_a_compile_error() {
        let src = r#"
implement Test;
Shape: adt {
    pick {
    Circle => r: int;
    Square => s: int;
    }
};
init(nil: ref Draw->Context, nil: list of string)
{
    sh: ref Shape;
    pick x := sh {
    Circle =>
        x.r = 1;
    }
}
"#;
        let err = compile_src(src).expect_err("pick must not compile to nothing");
        assert!(
            err.contains("pick"),
            "diagnostic should mention pick: {err}"
        );
    }

    #[test]
    fn break_outside_a_loop_is_a_compile_error() {
        let src = r#"
implement Test;
init(nil: ref Draw->Context, nil: list of string)
{
    break;
}
"#;
        let err = compile_src(src).expect_err("stray break must not compile to nothing");
        assert!(
            err.contains("break"),
            "diagnostic should mention break: {err}"
        );
    }

    #[test]
    fn break_to_an_unknown_label_is_a_compile_error() {
        let src = r#"
implement Test;
init(nil: ref Draw->Context, nil: list of string)
{
    for(;;)
        break nowhere;
}
"#;
        let err = compile_src(src).expect_err("unknown label must not compile to nothing");
        assert!(
            err.contains("nowhere"),
            "diagnostic should name the label: {err}"
        );
    }

    #[test]
    fn undefined_identifier_is_a_compile_error() {
        let src = r#"
implement Test;
init(nil: ref Draw->Context, nil: list of string)
{
    x := undefined_thing;
}
"#;
        let err = compile_src(src).expect_err("unknown name must not compile to Movw $0");
        assert!(
            err.contains("undefined_thing"),
            "diagnostic should name the identifier: {err}"
        );
    }

    // ── Loops and cases patch every jump they emit ──────────────

    #[test]
    fn break_emits_a_forward_jump_past_the_loop() {
        let src = r#"
implement Test;
init(nil: ref Draw->Context, nil: list of string)
{
    i := 0;
    for(;;) {
        i++;
        break;
    }
    i = 1;
}
"#;
        let module = compile_src(src).expect("break should compile");
        // The `break` jump must leave the loop: it targets a PC after the
        // loop's own backwards jump, and it is never left pointing at 0.
        let forward_jumps: Vec<usize> = module
            .code
            .iter()
            .enumerate()
            .filter(|(idx, i)| {
                i.opcode == Opcode::Jmp
                    && i.destination.mode == AddressMode::Immediate
                    && i.destination.register1 as usize > *idx
            })
            .map(|(idx, _)| idx)
            .collect();
        assert!(
            !forward_jumps.is_empty(),
            "break should emit a forward Jmp out of the loop"
        );
        assert!(
            module.code.iter().all(|i| i.opcode != Opcode::Jmp
                || i.destination.mode != AddressMode::Immediate
                || i.destination.register1 != 0),
            "no jump should be left pointing at PC 0"
        );
    }

    #[test]
    fn case_range_lower_bound_branch_is_patched() {
        let src = r#"
implement Test;
init(nil: ref Draw->Context, nil: list of string)
{
    x := 0;
    r := 0;
    case x {
    1 to 10 =>
        r = 1;
    * =>
        r = 2;
    }
}
"#;
        let module = compile_src(src).expect("case range should compile");
        let range_branches: Vec<_> = module
            .code
            .iter()
            .enumerate()
            .filter(|(_, i)| i.opcode == Opcode::Bltw)
            .collect();
        assert!(
            !range_branches.is_empty(),
            "a `lo to hi` pattern should emit a lower-bound branch"
        );
        for (idx, inst) in range_branches {
            assert_eq!(
                inst.destination.mode,
                AddressMode::Immediate,
                "range branch should have an immediate target"
            );
            assert!(
                inst.destination.register1 as usize > idx,
                "range lower-bound branch must be patched to the next pattern \
                 check, not left pointing at PC {}",
                inst.destination.register1
            );
        }
    }

    // ── Module-level constants ──────────────────────────────────

    #[test]
    fn module_constant_loads_its_value_not_zero() {
        let src = r#"
implement Test;
MAX: con 100;
init(nil: ref Draw->Context, nil: list of string)
{
    x := MAX;
}
"#;
        let module = compile_src(src).expect("module constant should compile");
        let has_100 = module.code.iter().any(|i| {
            i.opcode == Opcode::Movw
                && i.source.mode == AddressMode::Immediate
                && i.source.register1 == 100
        });
        assert!(has_100, "`x := MAX` should load 100, not 0");
    }

    #[test]
    fn module_variable_gets_mp_storage() {
        let src = r#"
implement Test;
counter: int;
init(nil: ref Draw->Context, nil: list of string)
{
    counter = 7;
}
"#;
        let module = compile_src(src).expect("module variable should compile");
        let writes_mp = module.code.iter().any(|i| {
            i.opcode == Opcode::Movw && i.destination.mode == AddressMode::OffsetIndirectMp
        });
        assert!(
            writes_mp,
            "assigning a module-level variable should store into MP"
        );
    }

    // ── Entry frame descriptor ──────────────────────────────────

    #[test]
    fn entry_type_describes_the_entry_function() {
        // `init` is generated first, so the last function's descriptor (the
        // old entry_type) is a different — and differently sized — type.
        let src = r#"
implement Test;
init(nil: ref Draw->Context, nil: list of string)
{
    x := 1;
}
helper(): int
{
    a := 1; b := a + 1; c := b + 1; d := c + 1; e := d + 1;
    f := e + 1; g := f + 1; h := g + 1; i := h + 1; j := i + 1;
    return j;
}
"#;
        let module = compile_src(src).expect("two functions should compile");
        assert_eq!(module.exports.len(), 1);
        assert_eq!(
            module.header.entry_type, module.exports[0].frame_type,
            "entry_type must describe the entry function's frame"
        );
        let entry_size = module.types[module.header.entry_type as usize].size;
        assert!(
            entry_size >= 40,
            "entry frame must still hold the standard frame header and locals"
        );
    }

    #[test]
    fn module_header_has_correct_sizes() {
        let src = r#"
implement Test;
init(nil: ref Draw->Context, nil: list of string)
{
    x := 1;
}
"#;
        let module = compile_src(src).expect("should compile");
        assert_eq!(module.header.magic, XMAGIC);
        assert_eq!(module.header.code_size, module.code.len() as i32);
        assert_eq!(module.header.type_size, module.types.len() as i32);
        assert_eq!(module.header.export_size, module.exports.len() as i32);
    }
}
