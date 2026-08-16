//! Code generator: translates Limbo AST to Dis bytecode.
//!
//! Produces a `ricevm_core::Module` from a parsed AST.

use ricevm_core::{
    AddressMode, DataItem, ExportEntry, Header, ImportEntry, ImportModule, Instruction, MiddleMode,
    MiddleOperand, Module, Opcode, Operand, PointerMap, RuntimeFlags, TypeDescriptor, XMAGIC,
};

use crate::ast::*;
use crate::symtab::{ConstValue, ResolvedType, Symbol, SymbolTable};
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
pub(crate) enum ConstVal {
    Int(i64),
    Real(f64),
    Str(String),
}

impl From<&ConstVal> for ConstValue {
    fn from(value: &ConstVal) -> Self {
        match value {
            ConstVal::Int(v) => ConstValue::Int(*v),
            ConstVal::Real(v) => ConstValue::Real(*v),
            ConstVal::Str(s) => ConstValue::String(s.clone()),
        }
    }
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
        Expr::ArrayAlloc(_, ty, _) | Expr::ArrayLit(_, _, Some(ty), _) => Some((**ty).clone()),
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

/// Extract the channel element Type from a VarDecl, if any.
fn decl_chan_elem_type(v: &VarDecl) -> Option<Type> {
    match &v.ty {
        Some(Type::Chan(elem)) | Some(Type::BufChan(_, elem)) => return Some((**elem).clone()),
        _ => {}
    }
    if let Some(Expr::ChanAlloc(ty, _)) = v.init.as_ref() {
        return Some((**ty).clone());
    }
    None
}

/// Field width and alignment for ADT layout. Matches the reference Limbo
/// ABI: word/byte/ptr fields are 4-byte sized and 4-byte aligned, and
/// big/real are 8-byte sized with 8-byte alignment.
fn type_size_align(ty: &Type) -> (i32, i32) {
    match ty {
        Type::Basic(BasicType::Big) | Type::Basic(BasicType::Real) => (8, 8),
        // A tuple is stored inline, so it occupies its whole width and
        // inherits its widest field's alignment. Treating it as one word
        // would have every field but the first overwrite its neighbours.
        Type::Tuple(fields) => {
            let align = fields
                .iter()
                .map(|f| type_size_align(f).1)
                .max()
                .unwrap_or(4);
            (compute_tuple_layout(fields).size, align)
        }
        // byte stored in a 4-byte slot in records (matches reference).
        _ => (4, 4),
    }
}

/// `t0`, `t1`, ... name a tuple's fields by position. Returns the position.
fn tuple_field_index(field: &str) -> Option<usize> {
    field.strip_prefix('t')?.parse::<usize>().ok()
}

/// The storage class of a value of this type.
fn val_type_of(ty: &Type) -> ValType {
    match ty {
        Type::Basic(BasicType::Int)
        | Type::Basic(BasicType::Byte)
        | Type::Basic(BasicType::Big)
        | Type::Basic(BasicType::Real) => ValType::Word,
        Type::Array(_) => ValType::Array,
        _ => ValType::Ptr,
    }
}

/// Where an ADT's own fields start. A tagged (`pick`) ADT keeps its tag in
/// word 0 and its fields follow it, which is what `tagof` and every `pick` arm
/// rely on (`limbo/types.c:2176`).
fn adt_field_base(adt: &AdtDecl) -> i32 {
    match adt.pick {
        Some(_) => 4,
        None => 0,
    }
}

/// The field declarations of an ADT, in declaration order.
fn adt_field_decls(adt: &AdtDecl) -> impl Iterator<Item = &VarDecl> {
    adt.members.iter().filter_map(|m| match m {
        AdtMember::Field(v) => Some(v),
        _ => None,
    })
}

/// Place `decls` from `base` onwards, returning the placed fields and the
/// offset just past the last one.
fn layout_fields<'a>(
    base: i32,
    decls: impl Iterator<Item = &'a VarDecl>,
) -> (Vec<(String, Type, i32)>, i32) {
    let mut fields = Vec::new();
    let mut off = base;
    for v in decls {
        let ty = match &v.ty {
            Some(t) => t.clone(),
            None => continue,
        };
        let (size, align) = type_size_align(&ty);
        // Round `off` up to alignment.
        off = align_up(off, align);
        for name in &v.names {
            fields.push((name.clone(), ty.clone(), off));
            off += size;
        }
    }
    (fields, off)
}

/// Compute an ADT's field layout: list of `(name, type, byte_offset)`.
fn compute_adt_layout(adt: &AdtDecl) -> Vec<(String, Type, i32)> {
    layout_fields(adt_field_base(adt), adt_field_decls(adt)).0
}

/// The layout one `pick` variant presents: the ADT's own fields followed by
/// that variant's, which start where the common fields end. Every variant
/// starts at the same offset, so they overlay one another
/// (`limbo/types.c:2183-2199`).
fn compute_variant_layout(adt: &AdtDecl, case: &PickCase) -> Vec<(String, Type, i32)> {
    let (mut fields, common_end) = layout_fields(adt_field_base(adt), adt_field_decls(adt));
    fields.extend(layout_fields(common_end, case.fields.iter()).0);
    fields
}

/// The storage shape of a record with these fields, whose first `base` bytes
/// are the tag (or nothing, for an untagged ADT).
fn shape_of(base: i32, fields: &[(String, Type, i32)]) -> RecordShape {
    let mut ptr_offsets = Vec::new();
    let mut end = base;
    for (_, ty, off) in fields {
        end = end.max(off + type_size_align(ty).0);
        if type_is_ptr(ty) {
            ptr_offsets.push(*off);
        }
    }
    RecordShape {
        size: align_up(end.max(4), 4),
        ptr_offsets,
    }
}

/// The storage shape of a record built from an ADT declaration.
///
/// `pick` variants are laid out after the common fields, and the record has to
/// be big enough for the largest of them — a record sized for the common
/// fields alone would have its variant fields written past its end. The
/// variants' pointer slots are all marked, which over-approximates for any one
/// variant and is the safe direction for tracing.
fn compute_adt_shape(adt: &AdtDecl) -> RecordShape {
    let base = adt_field_base(adt);
    let mut shape = shape_of(base, &compute_adt_layout(adt));
    for case in adt.pick.iter().flatten() {
        let variant = shape_of(base, &compute_variant_layout(adt, case));
        shape.size = shape.size.max(variant.size);
        for off in variant.ptr_offsets {
            if !shape.ptr_offsets.contains(&off) {
                shape.ptr_offsets.push(off);
            }
        }
    }
    shape
}

/// The byte layout of a tuple value.
///
/// A tuple is a contiguous block, not a pointer to one, so every consumer —
/// frame slot, call argument, channel message, return value — has to agree on
/// exactly these offsets and this size. Having one function produce all of
/// them is what keeps them agreeing.
#[derive(Clone, Debug, Default)]
struct TupleLayout {
    /// `(field type, byte offset)` in declaration order.
    fields: Vec<(Type, i32)>,
    /// Total size, which is what a channel of this tuple moves per message.
    size: i32,
    /// Offsets of the fields holding heap pointers, for the descriptor.
    ptr_offsets: Vec<i32>,
}

impl TupleLayout {
    /// Offset and type of field `n`, i.e. of `.tN`.
    fn field(&self, n: usize) -> Option<&(Type, i32)> {
        self.fields.get(n)
    }
}

/// Lay out a tuple's fields using the same width and alignment rules as an
/// ADT's, which is what the reference compiler does.
fn compute_tuple_layout(fields: &[Type]) -> TupleLayout {
    let mut out = Vec::new();
    let mut ptr_offsets = Vec::new();
    let mut off = 0i32;
    for ty in fields {
        let (size, align) = type_size_align(ty);
        off = align_up(off, align);
        if type_is_ptr(ty) {
            ptr_offsets.push(off);
        }
        out.push((ty.clone(), off));
        off += size;
    }
    TupleLayout {
        fields: out,
        size: align_up(off, 4).max(4),
        ptr_offsets,
    }
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

/// Pick the Mov opcode that copies a single value of this type.
fn mov_opcode_for_type(ty: &Type) -> Opcode {
    match ty {
        Type::Basic(BasicType::Big) => Opcode::Movl,
        Type::Basic(BasicType::Real) => Opcode::Movf,
        Type::Basic(BasicType::Int) | Type::Basic(BasicType::Byte) => Opcode::Movw,
        // Strings, arrays, refs, channels and lists are heap ids the
        // collector has to see move.
        _ => Opcode::Movp,
    }
}

fn sys_return_kind(name: &str) -> NumKind {
    match name {
        // big-returning sys builtins (per sys.m signatures)
        "seek" => NumKind::Big,
        _ => NumKind::Word,
    }
}

/// Fallback shape of a `$Sys` function's return value, used only when the
/// interface behind the handle could not be read.
fn sys_return_val_type(name: &str) -> ValType {
    match name {
        "fildes" | "open" | "create" | "fstat" | "stat" | "dirread" | "dial" | "announce"
        | "listen" => ValType::Ptr,
        _ => ValType::Word,
    }
}

/// Name the expression form that has no lowering, so the diagnostic points at
/// the missing feature rather than merely at the file.
fn describe_expr_kind(expr: &Expr) -> &'static str {
    match expr {
        Expr::ArrayLit(_, _, _, _) => "an array literal outside a declaration",
        Expr::Tuple(_, _) => "a tuple value",
        Expr::TupleDeclAssign(_, _, _) => "a tuple `:=` used as an expression",
        Expr::Tagof(_, _) => "`tagof`",
        Expr::Slice(_, _, _, _) => "slicing",
        _ => "this expression",
    }
}

/// Name what could not be lowered about a call, so the reader knows which
/// feature is missing rather than only that "something" is.
fn unsupported_call_target(callee: &Expr) -> String {
    match callee {
        Expr::Dot(_, method, _) => {
            format!("calling the ADT function member `{method}` is not supported yet")
        }
        Expr::Index(_, _, _) => {
            "calling a function held in an array element is not supported yet".to_string()
        }
        _ => "unsupported call target".to_string(),
    }
}

/// The ADT a resolved interface type names, peeling `ref`, `array of` and
/// `list of` wrappers the way `adt_name_for_type` peels their AST forms.
fn resolved_adt_name(ty: &ResolvedType) -> Option<String> {
    match ty {
        ResolvedType::Adt(name) => Some(name.rsplit('.').next().unwrap_or(name).to_string()),
        ResolvedType::Ref(inner) | ResolvedType::Array(inner) | ResolvedType::List(inner) => {
            resolved_adt_name(inner)
        }
        _ => None,
    }
}

/// Slot width of a resolved interface type.
fn resolved_num_kind(ty: &ResolvedType) -> NumKind {
    match ty {
        ResolvedType::Big => NumKind::Big,
        ResolvedType::Real => NumKind::Real,
        _ => NumKind::Word,
    }
}

/// Storage shape of a resolved interface type.
fn resolved_val_type(ty: &ResolvedType) -> ValType {
    match ty {
        ResolvedType::Array(_) => ValType::Array,
        _ if ty.is_ptr() => ValType::Ptr,
        _ => ValType::Word,
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
pub(crate) struct IotaCounter {
    group: Option<Span>,
    value: i64,
}

impl IotaCounter {
    pub(crate) fn next(&mut self, span: Span) -> i64 {
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
pub(crate) fn fold_const_binary(
    lhs: &ConstVal,
    op: BinOp,
    rhs: &ConstVal,
) -> Result<ConstVal, String> {
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

/// The storage class of an array element, which fixes both the element width
/// and whether the collector has to trace it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ElemKind {
    Byte,
    Word,
    Big,
    Real,
    Ptr,
}

impl ElemKind {
    fn of(ty: &Type) -> Self {
        match ty {
            Type::Basic(BasicType::Byte) => ElemKind::Byte,
            Type::Basic(BasicType::Int) => ElemKind::Word,
            Type::Basic(BasicType::Big) => ElemKind::Big,
            Type::Basic(BasicType::Real) => ElemKind::Real,
            Type::Basic(BasicType::String) => ElemKind::Ptr,
            _ => ElemKind::Ptr,
        }
    }

    /// Element kind implied by the literal's values when no type was written.
    fn of_values(values: &[ConstVal]) -> Self {
        if values.iter().any(|v| matches!(v, ConstVal::Str(_))) {
            ElemKind::Ptr
        } else if values.iter().any(|v| matches!(v, ConstVal::Real(_))) {
            ElemKind::Real
        } else {
            ElemKind::Word
        }
    }

    fn byte_size(self) -> i32 {
        match self {
            ElemKind::Byte => 1,
            ElemKind::Big | ElemKind::Real => 8,
            _ => 4,
        }
    }

    fn is_ptr(self) -> bool {
        self == ElemKind::Ptr
    }

    /// The Limbo basic type an element of this width reads back as.
    fn basic(self) -> BasicType {
        match self {
            ElemKind::Byte => BasicType::Byte,
            ElemKind::Word => BasicType::Int,
            ElemKind::Big => BasicType::Big,
            ElemKind::Real => BasicType::Real,
            ElemKind::Ptr => BasicType::String,
        }
    }
}

/// A type the module needs a descriptor for.
///
/// Descriptor indices are only settled once every function's frame descriptor
/// exists, which is after all code has been emitted — so instructions that
/// name a descriptor are emitted with a placeholder and patched from this key.
#[derive(Clone, PartialEq, Eq, Debug)]
enum TypeKey {
    /// A record laid out by an ADT declaration.
    Adt(String),
    /// One element of an array of primitives or pointers.
    Elem(ElemKind),
    /// A block of known size whose pointer slots are known: a tuple. Keyed by
    /// the layout itself rather than by the source type, so two structurally
    /// identical tuples share one descriptor.
    Block { size: i32, ptr_offsets: Vec<i32> },
    /// A record whose layout is not known. Traced conservatively.
    OpaqueRecord,
}

/// Which operand of an instruction carries a type index.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TypeOperand {
    Source,
    Middle,
}

/// An evaluated call argument, waiting to be stored into a callee frame.
struct ArgSlot {
    /// Frame offset holding the value.
    tmp: i32,
    /// Set when the argument is a tuple, which moves field by field.
    tuple: Option<TupleLayout>,
    /// Move opcode for a non-tuple argument.
    op: Opcode,
    /// Bytes a non-tuple argument occupies in the callee frame.
    width: i32,
}

/// One communication guard of an `alt`, resolved to the table entry it
/// occupies and the storage the transfer uses.
struct AltEntry<'a> {
    guard: &'a AltGuard,
    /// Position in the alt table: sends occupy `0..nsend`, receives the rest.
    index: usize,
    /// Index of the arm whose body this guard runs.
    arm: usize,
    /// Frame offset of the value the entry sends from or receives into.
    slot: i32,
    prologue: AltPrologue,
}

/// What an `alt` arm does with a received value before its body runs.
enum AltPrologue {
    /// The instruction already wrote the value where the source wants it.
    None,
    /// `(a, b) := <-c`: unpack the received tuple into the named locals.
    Tuple {
        names: Vec<String>,
        fields: Vec<Type>,
    },
    /// `a[i] = <-c`, `m = <-c` for a module-level `m`: store the value from
    /// the frame temp the instruction wrote it into.
    Store { target: Expr, ty: Type },
}

/// A record's storage shape: total size in bytes and the byte offsets of the
/// words that hold heap pointers.
#[derive(Clone, Debug, Default)]
struct RecordShape {
    size: i32,
    ptr_offsets: Vec<i32>,
}

/// Does a field of this type hold a heap pointer?
///
/// The classification has to match the moves the field writes use: a slot
/// written with `movp` holds an id the collector must follow, and a slot the
/// map claims is a pointer but which holds an `int` is merely retained (the
/// collector checks the id is live), so erring towards "pointer" is safe.
fn type_is_ptr(ty: &Type) -> bool {
    !matches!(
        ty,
        Type::Basic(BasicType::Int)
            | Type::Basic(BasicType::Byte)
            | Type::Basic(BasicType::Big)
            | Type::Basic(BasicType::Real)
    )
}

/// Round `off` up to a multiple of `align`.
fn align_up(off: i32, align: i32) -> i32 {
    (off + align - 1) & !(align - 1)
}

/// A pointer map over `size` bytes with the given pointer offsets, written
/// most-significant-bit first: word `n` is bit `1 << (7 - n % 8)` of byte
/// `n / 8`. This is the order `types.c` in the reference compiler emits and
/// the order the VM's `TraceMap` reads.
fn pointer_map_for(size: i32, offsets: &[i32]) -> PointerMap {
    let words = (size.max(0) as usize).div_ceil(4);
    let mut bytes = vec![0u8; words.div_ceil(8)];
    for &off in offsets {
        if off < 0 || off + 4 > size {
            continue;
        }
        let word = (off / 4) as usize;
        bytes[word / 8] |= 1 << (7 - word % 8);
    }
    PointerMap { bytes }
}

/// A module-level array initialiser folded to a data-section image.
struct ConstArray {
    elem: ElemKind,
    len: i32,
    values: Vec<ConstVal>,
}

fn is_byte_cast(e: &Expr) -> bool {
    matches!(e, Expr::Cast(ty, _, _) if matches!(ty.as_ref(), Type::Basic(BasicType::Byte)))
}

fn const_word(v: &ConstVal) -> i32 {
    match v {
        ConstVal::Int(n) => *n as i32,
        ConstVal::Real(n) => *n as i32,
        ConstVal::Str(_) => 0,
    }
}

fn const_byte(v: &ConstVal) -> u8 {
    const_word(v) as u8
}

fn const_big(v: &ConstVal) -> i64 {
    match v {
        ConstVal::Int(n) => *n,
        ConstVal::Real(n) => *n as i64,
        ConstVal::Str(_) => 0,
    }
}

fn const_real(v: &ConstVal) -> f64 {
    match v {
        ConstVal::Int(n) => *n as f64,
        ConstVal::Real(n) => *n,
        ConstVal::Str(_) => 0.0,
    }
}

/// A name an `import` declaration put into unqualified scope.
///
/// Limbo's `NAMES: import m;` takes either a module *variable* (`sys: Sys;
/// ... : import sys;`) or a module *type* name (`... : import Fs;`). The
/// distinction matters: a function can only be reached through a variable,
/// because the call needs a live module reference — the reference compiler
/// rejects calling a function imported from an interface name with "cannot
/// call X because M is a module interface" (typecheck.c:1459).
#[derive(Clone, Debug)]
struct Imported {
    /// The operand of `import`, spelled as in the source.
    module: String,
    /// The module's type name, when the interface is known. `None` when no
    /// declaration for the module could be found, which is what happens when
    /// the `.m` file was not on the include path.
    module_type: Option<String>,
    /// Whether `module` names a variable (rather than an interface name).
    from_variable: bool,
}

/// One cross-module import block: the functions this module calls through a
/// particular module *interface*.
///
/// The Dis import section has one block per imported module type; `load`
/// names the block in its middle operand and `mframe`/`mcall` index into it,
/// so both spellings have to agree on the same index. Keying by interface
/// name (rather than by the handle variable, of which there may be several)
/// is what makes them agree.
struct ModuleImport {
    /// Index of this block in `CodeGen::imports`.
    index: usize,
    /// Imported function name -> index within the block.
    funcs: Vec<(String, usize)>,
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
    /// Import blocks by module interface name, in allocation order.
    module_imports: Vec<(String, ModuleImport)>,
    /// Module-handle variables: variable name -> module interface name. A
    /// call `h->f()` is a cross-module call through `h`'s storage slot, so
    /// this is what tells `h->f()` apart from an undefined identifier.
    module_handle_type: std::collections::HashMap<String, String>,
    /// Names of `Mod: module { ... }` declarations in this file. Together
    /// with the symbol table this distinguishes "interface name used where a
    /// module variable is needed" from "undefined identifier".
    module_decls: std::collections::HashSet<String>,
    /// Local variable table: name -> (fp offset, ValType, NumKind).
    /// NumKind is Word for non-numeric locals (strings, arrays, refs); only
    /// big/real locals carry a widened kind that sizes the slot and picks
    /// the correct Mov/arith opcode family.
    locals: Vec<(String, i32, ValType, NumKind)>,
    next_local: i32,
    frame_size: i32,
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
    /// Sidecar map for channel locals: name -> element Type. Lets Send/Recv
    /// size the data temp by the channel's element width and pick the right
    /// Newc* opcode at allocation time. The full Type is kept rather than a
    /// BasicType because a channel of a tuple moves the tuple's whole width
    /// per message, and a channel that thinks its elements are 4 bytes wide
    /// silently delivers only the first word.
    local_chan_elem: std::collections::HashMap<String, Type>,
    /// Sidecar map for tuple-typed locals: name -> field types. A tuple lives
    /// inline in the frame as a contiguous block, so this is what says how
    /// wide the local's slot is and where each `.tN` sits inside it.
    local_tuple: std::collections::HashMap<String, Vec<Type>>,
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
    /// Storage shapes for the same ADTs: the size and pointer offsets each
    /// record's type descriptor has to state.
    adt_shapes: std::collections::HashMap<String, RecordShape>,
    /// Function members declared by each ADT: ADT name -> method name ->
    /// signature. The signature says whether the method takes a `self`
    /// receiver and what it returns.
    adt_methods: std::collections::HashMap<String, std::collections::HashMap<String, FuncSig>>,
    /// The interface each ADT belongs to, for ADTs that come from one. A
    /// method of a foreign ADT is implemented by that module, so calling it
    /// is a cross-module call through a handle for that interface.
    adt_owner: std::collections::HashMap<String, String>,
    /// The variants of every tagged ADT, keyed `"Shape.Circle"`: the tag its
    /// constructor writes into word 0, and the index of the `pick` group whose
    /// fields it carries. Tags of one group share a layout, so an arm naming
    /// several of them can still resolve that group's fields.
    adt_variants: std::collections::HashMap<String, (i32, usize)>,
    /// Declared return type of each function this file defines, so a local
    /// whose value comes from a call knows which ADT it holds.
    func_ret_types: std::collections::HashMap<String, Type>,
    /// Type descriptors some instruction or data item needs, in first-use
    /// order. Their indices are assigned by `build_types`.
    needed_types: Vec<TypeKey>,
    /// Instructions whose type-index operand is patched once the descriptor
    /// table is laid out: `(code index, which operand, type)`.
    pending_type_fixups: Vec<(usize, TypeOperand, TypeKey)>,
    /// Sidecar map for ADT-typed locals: local name -> ADT name. Lets Dot
    /// access resolve `local.field` to the right ADT layout.
    local_adt_type: std::collections::HashMap<String, String>,
    /// Frame shape for each compiled function, in order: `(size, byte
    /// offsets of the slots known to hold pointers)`.
    func_frames: Vec<(i32, Vec<i32>)>,
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
    /// Names an `import` declaration brought into unqualified scope, keyed by
    /// the imported name. Every use site resolves through the owning module,
    /// so an imported constant folds to the module's value and an imported
    /// function compiles to the same cross-module call its qualified spelling
    /// `mod->f(...)` would produce.
    imported: std::collections::HashMap<String, Imported>,
    /// Data items of kind `Array` whose element type descriptor is still to be
    /// allocated: `(index into self.data, element kind)`. The descriptor table
    /// is only laid out once every function's frame descriptor is known, so the
    /// index is patched in at that point.
    pending_array_elem_types: Vec<(usize, ElemKind)>,
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
            module_imports: Vec::new(),
            module_handle_type: std::collections::HashMap::new(),
            module_decls: std::collections::HashSet::new(),
            locals: Vec::new(),
            next_local: 40,
            frame_size: 80,
            func_table: Vec::new(),
            func_frames: Vec::new(),
            local_array_elem: std::collections::HashMap::new(),
            local_chan_elem: std::collections::HashMap::new(),
            local_tuple: std::collections::HashMap::new(),
            pending_call_fixups: Vec::new(),
            adt_layouts: std::collections::HashMap::new(),
            adt_shapes: std::collections::HashMap::new(),
            adt_methods: std::collections::HashMap::new(),
            adt_owner: std::collections::HashMap::new(),
            adt_variants: std::collections::HashMap::new(),
            func_ret_types: std::collections::HashMap::new(),
            needed_types: Vec::new(),
            pending_type_fixups: Vec::new(),
            local_adt_type: std::collections::HashMap::new(),
            func_tuple_ret: std::collections::HashMap::new(),
            handlers: Vec::new(),
            globals: Vec::new(),
            module_consts: std::collections::HashMap::new(),
            qualified_consts: std::collections::HashMap::new(),
            symtab: None,
            imported: std::collections::HashMap::new(),
            pending_array_elem_types: Vec::new(),
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

        self.collect_strings(file);
        self.collect_adts(file);
        // `import` runs before constant folding: an imported constant has to
        // be usable inside another module-level constant expression.
        self.collect_imports(file)?;
        // ... and `implement X` puts X's own interface in scope the same way,
        // after explicit imports so an explicit one wins.
        self.bind_implement_scope(file);
        // Module-level `con` values are folded once, up front, so use sites
        // can resolve them to literals; module-level variables get real MP
        // storage so every function sees the same slot.
        self.collect_consts(file);
        self.collect_globals(file)?;

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
            if let Some(ret) = &func.sig.ret {
                self.func_ret_types.insert(full_name.clone(), ret.clone());
            }
            // PC = -1 placeholder, frame_size = 0 placeholder.
            self.func_table.push((full_name, -1, 0, ret_kind));
        }

        // Generate code for each function. gen_func patches the matching
        // func_table entry's PC and frame_size.
        for func in &funcs {
            self.gen_func(func)?;
        }

        // `gen_func` emits the deferred module-level initialisers into the
        // entry function. A module with no `init` has no entry function —
        // and nothing else is guaranteed to run before its exported
        // functions — so there is nowhere to put them. Say so instead of
        // leaving the variables silently zero at run time.
        if !self.pending_global_inits.is_empty() {
            let names: Vec<&str> = self
                .pending_global_inits
                .iter()
                .map(|(n, _)| n.as_str())
                .collect();
            return Err(format!(
                "module `{}` has no `init` function, so the module-level initialiser(s) for `{}` \
                 would never run: add an `init` function, or use a constant initialiser",
                self.module_name,
                names.join("`, `")
            ));
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
        // The header always claims HAS_IMPORT, and the on-disk import section
        // is only well formed (it ends with a null byte) when it holds at
        // least one block. A module that calls out to nobody still gets an
        // empty one.
        if self.imports.is_empty() {
            self.imports.push(ImportModule { functions: vec![] });
        }
        // The entry frame must be described by the *entry function's* type
        // descriptor. Using the last generated function's descriptor only
        // happened to work while every frame was sized to the cumulative
        // maximum; with per-function frame sizes it would under- or
        // over-allocate the entry frame.
        // A module with no `init` has no entry point: the reference writes
        // `-1, -1` for it (dis.c:110-120) and the loader accepts that for
        // library modules. Pointing the entry at the first function instead
        // named a function that was never meant to be the entry, and for a
        // module that is nothing but data it named code that does not exist.
        let (entry_pc, entry_type) = match self.exports.first() {
            Some(e) => (e.pc, e.frame_type),
            None => (-1, -1),
        };
        let max_frame = self
            .func_frames
            .iter()
            .map(|(size, _)| *size)
            .max()
            .unwrap_or(self.frame_size);

        // Nothing may leave `compile` with initialisers still queued: every
        // one of them is either emitted into the entry function or reported
        // above. Silently dropping them is the failure mode this guards.
        debug_assert!(
            self.pending_global_inits.is_empty(),
            "module-level initialisers were dropped: {:?}",
            self.pending_global_inits
                .iter()
                .map(|(n, _)| n)
                .collect::<Vec<_>>()
        );

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

    /// Reserve a contiguous `size`-byte block of frame. A tuple lives inline,
    /// so its slot has to be the whole tuple wide — a 4-byte slot would have
    /// every field but the first overwrite whatever came next.
    fn alloc_temp_block(&mut self, size: i32) -> i32 {
        let off = self.next_local;
        self.next_local += align_up(size.max(4), 4);
        self.grow_frame();
        off
    }

    /// Like `alloc_local`, but reserving a whole `size`-byte block.
    fn alloc_local_block(&mut self, name: &str, size: i32) -> i32 {
        if let Some((_, off, _, _)) = self.locals.iter().find(|(n, _, _, _)| n == name) {
            return *off;
        }
        let off = self.alloc_temp_block(size);
        self.locals
            .push((name.to_string(), off, ValType::Word, NumKind::Word));
        off
    }

    fn grow_frame(&mut self) {
        if self.next_local > self.frame_size - 8 {
            self.frame_size = ((self.next_local + 24) + 7) & !7;
        }
    }

    /// The import block for a module interface, creating it on first use.
    ///
    /// Returns the block's index, which is what `load` puts in its middle
    /// operand so the runtime can map this module's function indices onto the
    /// loaded module's exports.
    fn ensure_module_import(&mut self, interface: &str) -> usize {
        if let Some((_, imp)) = self.module_imports.iter().find(|(n, _)| n == interface) {
            return imp.index;
        }
        let index = self.imports.len();
        self.imports.push(ImportModule { functions: vec![] });
        self.module_imports.push((
            interface.to_string(),
            ModuleImport {
                index,
                funcs: Vec::new(),
            },
        ));
        index
    }

    /// The index of `name` within `interface`'s import block, adding it on
    /// first use. `mframe`/`mcall` carry this index.
    fn ensure_module_func(&mut self, interface: &str, name: &str) -> usize {
        let block = self.ensure_module_import(interface);
        let entry = self
            .module_imports
            .iter_mut()
            .find(|(n, _)| n == interface)
            .map(|(_, imp)| imp)
            .expect("ensure_module_import just created the block");
        if let Some((_, idx)) = entry.funcs.iter().find(|(n, _)| n == name) {
            return *idx;
        }
        let idx = entry.funcs.len();
        entry.funcs.push((name.to_string(), idx));
        self.imports[block].functions.push(ImportEntry {
            signature: 0,
            name: name.to_string(),
        });
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
                Decl::Adt(adt) => self.record_adt(adt, None, true),
                Decl::Module(m) => {
                    self.module_decls.insert(m.name.clone());
                    for member in &m.members {
                        if let ModuleMember::Adt(adt) = member {
                            self.record_adt(adt, Some(&m.name.clone()), true);
                        }
                    }
                }
                _ => {}
            }
        }
        self.collect_interface_adts(file);
    }

    /// Remember one ADT's field layout and its storage shape. `overwrite`
    /// distinguishes a declaration in this file, which is authoritative, from
    /// an interface's, which only fills a name in if it is still free.
    fn record_adt(&mut self, adt: &AdtDecl, owner: Option<&str>, overwrite: bool) {
        if !overwrite && self.adt_layouts.contains_key(&adt.name) {
            return;
        }
        self.adt_layouts
            .insert(adt.name.clone(), compute_adt_layout(adt));
        self.adt_shapes
            .insert(adt.name.clone(), compute_adt_shape(adt));
        let methods: std::collections::HashMap<String, FuncSig> = adt
            .members
            .iter()
            .filter_map(|m| match m {
                AdtMember::Func(sig) => Some((sig.name.clone(), sig.clone())),
                _ => None,
            })
            .collect();
        self.adt_methods.insert(adt.name.clone(), methods.clone());
        match owner {
            Some(owner) => {
                self.adt_owner.insert(adt.name.clone(), owner.to_string());
            }
            None => {
                self.adt_owner.remove(&adt.name);
            }
        }
        // Each variant of a tagged ADT is a type of its own: `ref Shape.Circle`
        // allocates the Circle layout and a `pick` arm reads its fields.
        // Tags are dense from zero in declaration order, one per name, so
        // `Square or Rect` numbers both while they share one field layout
        // (`limbo/types.c:681-717`).
        let mut tag = 0i32;
        for (group, case) in adt.pick.iter().flatten().enumerate() {
            let layout = compute_variant_layout(adt, case);
            let shape = shape_of(adt_field_base(adt), &layout);
            for name in &case.tags {
                let key = format!("{}.{name}", adt.name);
                self.adt_layouts.insert(key.clone(), layout.clone());
                self.adt_shapes.insert(key.clone(), shape.clone());
                self.adt_methods.insert(key.clone(), methods.clone());
                if let Some(owner) = owner {
                    self.adt_owner.insert(key.clone(), owner.to_string());
                }
                self.adt_variants.insert(key, (tag, group));
                tag += 1;
            }
        }
    }

    /// Take the field layouts of every ADT declared in an interface this file
    /// can see.
    ///
    /// The module this file implements goes first, so its own names win: its
    /// ADTs are the ones in unqualified scope. Other interfaces then fill in
    /// the names still free, which is what makes `ref Bufio->Iobuf(...)` and
    /// field access through it use the declared offsets rather than a
    /// positional guess. A declaration in this file always wins over both.
    fn collect_interface_adts(&mut self, file: &SourceFile) {
        let Some(symtab) = self.symtab.as_ref() else {
            return;
        };
        let implemented = file.implement.first().cloned();
        let mut ordered: Vec<(&String, &std::collections::HashMap<String, AdtDecl>)> =
            symtab.adt_decls.iter().collect();
        // Deterministic order, with the implemented module ahead of the rest.
        ordered.sort_by_key(|(name, _)| (implemented.as_ref() != Some(*name), (*name).clone()));
        let decls: Vec<(String, AdtDecl)> = ordered
            .into_iter()
            .flat_map(|(module, adts)| {
                let mut named: Vec<(&String, &AdtDecl)> = adts.iter().collect();
                named.sort_by_key(|(n, _)| (*n).clone());
                named
                    .into_iter()
                    .map(|(_, adt)| (module.clone(), adt.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        for (module, adt) in &decls {
            // The implemented module's own ADTs are compiled here, so they
            // are not foreign: their methods are local functions.
            let owner = (implemented.as_ref() != Some(module)).then_some(module.as_str());
            self.record_adt(adt, owner, false);
        }
    }

    /// Bind the members of the interface this file implements into
    /// unqualified scope.
    ///
    /// Limbo makes a module's own interface visible without qualification
    /// inside its implementation, which is why `STATFIXLEN`, `Rawimage` and
    /// friends are legal bare names there. The binding reuses `import`'s
    /// machinery, so a member resolves at its use site exactly as an imported
    /// one does — including saying why an unrepresentable one cannot be used.
    ///
    /// Functions are deliberately left out: the implementation defines them,
    /// so they resolve as local functions, and one it fails to define should
    /// be reported as missing rather than as an uncallable import.
    /// An explicit `import` wins, since it was written on purpose.
    fn bind_implement_scope(&mut self, file: &SourceFile) {
        let Some(module) = file.implement.first().cloned() else {
            return;
        };
        let Some(symtab) = self.symtab.as_ref() else {
            return;
        };
        let Some(members) = symtab.modules.get(&module) else {
            return;
        };
        let names: Vec<String> = members
            .iter()
            .filter(|(_, sym)| !matches!(sym, Symbol::Func { .. }))
            .map(|(name, _)| name.clone())
            .collect();
        for name in names {
            self.imported.entry(name).or_insert_with(|| Imported {
                module: module.clone(),
                module_type: Some(module.clone()),
                from_variable: false,
            });
        }
    }

    /// Is `name` the name of a module interface (as opposed to an ADT, a type
    /// alias or nothing at all)?
    ///
    /// When the interface was never found — an `include` that is not on the
    /// search path — there is no evidence either way, and the permissive
    /// answer is the useful one: `sys: Sys;` still names a module handle even
    /// if `sys.m` could not be read.
    fn is_module_interface_name(&self, name: &str) -> bool {
        if self.adt_layouts.contains_key(name) {
            return false;
        }
        if self.module_decls.contains(name) {
            return true;
        }
        match self.symtab.as_ref() {
            Some(st) if st.modules.contains_key(name) => true,
            Some(st) => !matches!(st.lookup(name), Some(Symbol::Type { .. })),
            None => true,
        }
    }

    /// Record `name` as a module handle when its declaration says so, either
    /// through an explicit module type (`sys: Sys;`) or through the interface
    /// a `load` names (`sys := load Sys Sys->PATH;`).
    fn note_module_handle(&mut self, name: &str, ty: Option<&Type>, init: Option<&Expr>) {
        let interface = match (ty, init) {
            (Some(Type::Named(qn)), _) if qn.qualifier.is_none() => qn.name.clone(),
            (_, Some(Expr::Load(load_ty, _, _))) => match load_ty.as_ref() {
                Type::Named(qn) => qn.name.clone(),
                _ => return,
            },
            _ => return,
        };
        if self.is_module_interface_name(&interface) {
            self.module_handle_type.insert(name.to_string(), interface);
        }
    }

    /// The interface member `handle->member` names, when the handle's
    /// interface is known and was found.
    fn handle_member(&self, handle: &str, member: &str) -> Option<&Symbol> {
        let interface = self.module_handle_type.get(handle)?;
        self.module_member(interface, member)
    }

    /// Resolve every `import` declaration, binding the imported names in
    /// unqualified scope.
    ///
    /// `import` may appear at the top level or inside a function body; both
    /// forms bind the same way. The operand names either a module variable,
    /// whose declared type gives the interface, or an interface directly.
    fn collect_imports(&mut self, file: &SourceFile) -> Result<(), String> {
        for decl in &file.decls {
            match decl {
                Decl::Import(imp) => self.bind_import(imp)?,
                Decl::Func(f) => self.collect_stmt_imports(&f.body.stmts)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn collect_stmt_imports(&mut self, stmts: &[Stmt]) -> Result<(), String> {
        for stmt in stmts {
            match stmt {
                Stmt::Import(imp) => self.bind_import(imp)?,
                Stmt::Block(b) => self.collect_stmt_imports(&b.stmts)?,
                Stmt::If(s) => {
                    self.collect_stmt_imports(std::slice::from_ref(&s.then))?;
                    if let Some(e) = &s.else_ {
                        self.collect_stmt_imports(std::slice::from_ref(e))?;
                    }
                }
                Stmt::For(s) => self.collect_stmt_imports(std::slice::from_ref(&s.body))?,
                Stmt::While(s) => self.collect_stmt_imports(std::slice::from_ref(&s.body))?,
                Stmt::Do(s) => self.collect_stmt_imports(std::slice::from_ref(&s.body))?,
                Stmt::Label(_, s) => self.collect_stmt_imports(std::slice::from_ref(s))?,
                Stmt::Case(s) => {
                    for arm in &s.arms {
                        self.collect_stmt_imports(&arm.body)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Bind one `NAMES: import m;` declaration.
    ///
    /// A name the module does not declare is a hard error naming both, which
    /// is what the reference compiler reports ("X is not a member of m",
    /// typecheck.c:942). The one case that cannot be checked is a module whose
    /// interface was never found; the names are still recorded so their use
    /// sites can say why they are unresolvable.
    fn bind_import(&mut self, imp: &ImportDecl) -> Result<(), String> {
        let (module_type, from_variable) = self.resolve_import_operand(&imp.module);
        for name in &imp.names {
            if let Some(ty) = &module_type
                && self.module_member(ty, name).is_none()
            {
                return Err(self.not_a_member(name, &imp.module, ty));
            }
            self.imported.insert(
                name.clone(),
                Imported {
                    module: imp.module.clone(),
                    module_type: module_type.clone(),
                    from_variable,
                },
            );
        }
        Ok(())
    }

    /// Map the operand of `import` to `(interface name, is a variable)`.
    fn resolve_import_operand(&self, operand: &str) -> (Option<String>, bool) {
        let Some(symtab) = self.symtab.as_ref() else {
            return (None, false);
        };
        match symtab.lookup(operand) {
            // `sys: Sys;` — a variable whose type is a module interface.
            Some(Symbol::Var {
                ty: ResolvedType::Module(name),
            }) => (Some(name.clone()), true),
            // `import Fs;` — the interface named directly.
            Some(Symbol::Module { name, .. }) => (Some(name.clone()), false),
            _ if symtab.modules.contains_key(operand) => (Some(operand.to_string()), false),
            _ => (None, false),
        }
    }

    /// Look up a member of a module interface by interface name.
    fn module_member(&self, module_type: &str, member: &str) -> Option<&Symbol> {
        self.symtab.as_ref()?.lookup_qualified(module_type, member)
    }

    /// "X is not a member of m", naming the interface too when the operand was
    /// a variable, so the reader knows which declaration to look at.
    fn not_a_member(&self, member: &str, module: &str, module_type: &str) -> String {
        if module == module_type {
            format!("`{member}` is not a member of module `{module}`")
        } else {
            format!("`{member}` is not a member of module `{module}` (interface `{module_type}`)")
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
            // `len` of a constant string is a constant. Limbo counts
            // characters, not bytes.
            Expr::Len(inner, _) => match self.fold_const(inner, iota)? {
                ConstVal::Str(s) => Ok(ConstVal::Int(s.chars().count() as i64)),
                _ => Err("`len` of a non-constant".to_string()),
            },
            _ => Err("not a constant expression".to_string()),
        }
    }

    /// Look up a folded constant by unqualified name, falling back to the
    /// include-derived symbol table.
    fn const_value(&self, name: &str) -> Option<Result<ConstVal, String>> {
        if let Some(v) = self.module_consts.get(name) {
            return Some(v.clone());
        }
        // An imported constant folds exactly like a locally declared one, in
        // constant expressions as well as at ordinary use sites.
        if let Some(imp) = self.imported.get(name)
            && let Some(module_type) = &imp.module_type
            && let Some(v) = self.qualified_const_value(module_type, name)
        {
            return Some(v);
        }
        match self.symtab.as_ref()?.lookup(name) {
            Some(Symbol::Const { value, .. }) => Some(Ok(ConstVal::from(value))),
            _ => None,
        }
    }

    /// Explain why an imported name cannot stand for a value here.
    ///
    /// Reached only after `const_value` has declined it, so the name is bound
    /// to something that is not a constant.
    fn imported_value_error(&self, name: &str, imp: &Imported) -> String {
        let Some(module_type) = &imp.module_type else {
            return format!(
                "undefined identifier `{name}`: it is imported from `{}`, whose interface was \
                 not found (is the include path set?)",
                imp.module
            );
        };
        match self.module_member(module_type, name) {
            Some(Symbol::Opaque { reason }) => format!(
                "`{name}`, imported from `{}`, cannot be used here: {reason}",
                imp.module
            ),
            Some(Symbol::Type { .. }) => format!("`{name}` is a type, not a value"),
            Some(Symbol::Func { .. }) => {
                format!("`{name}` is a function; it can only be called, not used as a value")
            }
            Some(Symbol::Var { .. }) => format!(
                "`{name}` is a variable of module `{}`; qualified access to another module's \
                 variables is not supported yet",
                imp.module
            ),
            _ => self.not_a_member(name, &imp.module, module_type),
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
        let chan_elem_type = decl_chan_elem_type(v);
        let adt_name = v.ty.as_ref().and_then(Self::adt_name_for_type);
        // A constant initialiser goes straight into the data section;
        // anything else is deferred to the top of the entry function.
        let init = match &v.init {
            Some(e) => self.fold_const(e, 0).ok(),
            None => None,
        };
        for name in &v.names {
            if name == "nil" {
                continue;
            }
            // A module's interface may declare the variable and the body then
            // supply its value, so a repeated name keeps its existing slot
            // instead of being skipped outright — skipping dropped the
            // initialiser with it.
            let off = match self.lookup_global(name) {
                Some(off) => off,
                None => {
                    let off = self.alloc_mp(kind.byte_size());
                    self.globals.push((name.clone(), off, ty, kind));
                    off
                }
            };
            if let Some(t) = &elem_type {
                self.local_array_elem.insert(name.clone(), t.clone());
            }
            if let Some(t) = &chan_elem_type {
                self.local_chan_elem.insert(name.clone(), t.clone());
            }
            if let Some(a) = &adt_name {
                self.local_adt_type.insert(name.clone(), a.clone());
            }
            self.note_module_handle(name, v.ty.as_ref(), v.init.as_ref());
            if init.is_none()
                && let Some(expr) = &v.init
            {
                // An array of constants is itself a constant in Limbo, so it
                // belongs in the data section rather than in code — that is
                // what lets a module with no `init` have one at all.
                match self.const_array(expr, elem_type.as_ref())? {
                    Some(array) => {
                        // The image fixes the element width, so record it:
                        // reading a byte-packed array with word loads would
                        // return neighbouring elements' bytes.
                        if elem_type.is_none() {
                            self.local_array_elem
                                .insert(name.clone(), Type::Basic(array.elem.basic()));
                        }
                        self.emit_const_array(off, &array);
                    }
                    None => self.pending_global_inits.push((name.clone(), expr.clone())),
                }
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

    fn lookup_global(&self, name: &str) -> Option<i32> {
        self.globals
            .iter()
            .find(|(n, _, _, _)| n == name)
            .map(|(_, off, _, _)| *off)
    }

    /// Fold a module-level array initialiser into a data-section image.
    ///
    /// `array[n] of T` and `array[] of {constants}` are constant expressions in
    /// Limbo (`initable`, nodes.c:168-186); the reference writes them into the
    /// data section and the loader builds them when it creates the module's MP.
    /// Returns `None` for anything else, which the caller then defers to the
    /// entry function as before.
    fn const_array(
        &self,
        expr: &Expr,
        declared_elem: Option<&Type>,
    ) -> Result<Option<ConstArray>, String> {
        match expr {
            Expr::ArrayAlloc(size, ty, _) => {
                let Ok(ConstVal::Int(len)) = self.fold_const(size, 0) else {
                    return Ok(None);
                };
                let len =
                    i32::try_from(len).map_err(|_| "array size is out of range".to_string())?;
                if len < 0 {
                    return Err("array size is negative".to_string());
                }
                Ok(Some(ConstArray {
                    elem: ElemKind::of(declared_elem.unwrap_or(ty)),
                    len,
                    values: Vec::new(),
                }))
            }
            Expr::ArrayLit(size, elems, ty, _) => {
                // Place each element at the index it names. A positional
                // element takes the next free slot, a keyed or ranged one the
                // slots it names, and `* => v` every slot left over.
                let mut placed: Vec<(usize, ConstVal)> = Vec::new();
                let mut default: Option<ConstVal> = None;
                let mut next = 0usize;
                let mut high = 0usize;
                for e in elems {
                    let Ok(value) = self.fold_const(&e.value, 0) else {
                        // Not a constant image; leave it to the entry function
                        // rather than emitting a half-built array.
                        return Ok(None);
                    };
                    // The indices this element initialises.
                    let indices: Vec<i64> = match &e.index {
                        None => vec![next as i64],
                        Some(ArrayIndex::Selectors(sels)) => {
                            let mut out = Vec::with_capacity(sels.len());
                            for (lo, hi) in sels {
                                let Ok(ConstVal::Int(lo)) = self.fold_const(lo, 0) else {
                                    return Ok(None);
                                };
                                match hi {
                                    None => out.push(lo),
                                    Some(hi) => {
                                        let Ok(ConstVal::Int(hi)) = self.fold_const(hi, 0) else {
                                            return Ok(None);
                                        };
                                        out.extend(lo..=hi);
                                    }
                                }
                            }
                            out
                        }
                        Some(ArrayIndex::Wildcard) => {
                            default = Some(value);
                            continue;
                        }
                    };
                    for i in indices {
                        let Ok(i) = usize::try_from(i) else {
                            return Ok(None);
                        };
                        placed.push((i, value.clone()));
                        next = i + 1;
                        high = high.max(i + 1);
                    }
                }
                // `array[n] of {..}` has length n; `array[] of {..}` is exactly
                // as long as the indices its elements reach.
                let len = match size {
                    Some(e) => match self.fold_const(e, 0) {
                        Ok(ConstVal::Int(n)) if n >= 0 => n as usize,
                        _ => return Ok(None),
                    },
                    None => high,
                };
                if placed.iter().any(|(i, _)| *i >= len) {
                    return Err("array initialiser index is outside the array".to_string());
                }
                let filler = default.unwrap_or(ConstVal::Int(0));
                let mut values = vec![filler; len];
                for (i, v) in placed {
                    values[i] = v;
                }
                let elem = match declared_elem.or(ty.as_deref()) {
                    Some(t) => ElemKind::of(t),
                    // No declared element type: `array[] of {byte 0, byte 1}`
                    // is an array of byte, a list of strings an array of
                    // string, and everything else follows the folded values.
                    None if !elems.is_empty() && elems.iter().all(|e| is_byte_cast(&e.value)) => {
                        ElemKind::Byte
                    }
                    None => ElemKind::of_values(&values),
                };
                Ok(Some(ConstArray {
                    elem,
                    len: len as i32,
                    values,
                }))
            }
            _ => Ok(None),
        }
    }

    /// Write a folded array into the data section at MP offset `off`.
    ///
    /// The element type index is filled in later by `build_types`, which is the
    /// only point at which the descriptor table's length is settled.
    fn emit_const_array(&mut self, off: i32, array: &ConstArray) {
        let item = self.data.len();
        self.data.push(DataItem::Array {
            offset: off,
            element_type: 0,
            length: array.len,
        });
        self.pending_array_elem_types.push((item, array.elem));
        if array.values.is_empty() {
            return;
        }
        // Inside an array context the data offsets address the array's own
        // storage, so one item covers every element from index 0.
        self.data.push(DataItem::SetArray {
            offset: off,
            index: 0,
        });
        match array.elem {
            ElemKind::Byte => self.data.push(DataItem::Bytes {
                offset: 0,
                values: array.values.iter().map(const_byte).collect(),
            }),
            ElemKind::Word => self.data.push(DataItem::Words {
                offset: 0,
                values: array.values.iter().map(const_word).collect(),
            }),
            ElemKind::Big => self.data.push(DataItem::Bigs {
                offset: 0,
                values: array.values.iter().map(const_big).collect(),
            }),
            ElemKind::Real => self.data.push(DataItem::Reals {
                offset: 0,
                values: array.values.iter().map(const_real).collect(),
            }),
            // Pointer elements each hold a heap id, so they need one item per
            // element at that element's own offset.
            ElemKind::Ptr => {
                for (i, v) in array.values.iter().enumerate() {
                    if let ConstVal::Str(s) = v {
                        self.data.push(DataItem::String {
                            offset: (i as i32) * 4,
                            value: s.clone(),
                        });
                    }
                }
            }
        }
        self.data.push(DataItem::RestoreBase);
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
            // An element of an `array of ref Adt` is that ADT.
            Expr::Index(arr, _, _) => Self::adt_name_for_type(&self.array_elem_type_for_expr(arr)?),
            // A message off a `chan of ref Adt` is that ADT, which is what
            // lets `pick m := <-c` resolve its tags.
            Expr::Recv(chan, _) => Self::adt_name_for_type(&self.chan_elem_type(chan)?),
            Expr::Call(_, _, _) => self.adt_name_from_call(expr),
            _ => None,
        }
    }

    /// The ADT a call's declared return type names, when it has one.
    ///
    /// Without this, `b := bufio->open(...)` leaves `b` untyped, and every
    /// `b.field` and `b.method()` after it has nothing to resolve against.
    fn adt_name_from_call(&self, expr: &Expr) -> Option<String> {
        let Expr::Call(callee, _, _) = expr else {
            return None;
        };
        match callee.as_ref() {
            Expr::ModQual(module, name, _) => {
                let Expr::Ident(handle, _) = module.as_ref() else {
                    return None;
                };
                match self.handle_member(handle, name) {
                    Some(Symbol::Func { ty }) => resolved_adt_name(ty.ret.as_deref()?),
                    _ => None,
                }
            }
            Expr::Dot(obj, method, _) => {
                let (adt, _) = self.adt_method_target(obj, method)?;
                Self::adt_name_for_type(self.adt_method_sig(&adt, method)?.ret.as_ref()?)
            }
            Expr::Ident(name, _) => Self::adt_name_for_type(self.func_ret_types.get(name)?),
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
            Expr::Tuple(es, _) | Expr::ListLit(es, _) => {
                for e in es {
                    self.scan_expr_strings(e);
                }
            }
            _ => {}
        }
    }

    /// Note that the instruction at `at` names the descriptor for `key`, and
    /// that the module therefore needs one. The index is patched in by
    /// `build_types`, which is the first point at which the table's layout —
    /// and so every index in it — is settled.
    fn need_type(&mut self, at: usize, operand: TypeOperand, key: TypeKey) {
        if !self.needed_types.contains(&key) {
            self.needed_types.push(key.clone());
        }
        self.pending_type_fixups.push((at, operand, key));
    }

    /// The descriptor for one needed type.
    fn descriptor_for(&self, id: u32, key: &TypeKey) -> TypeDescriptor {
        let (size, offsets) = match key {
            TypeKey::Adt(name) => {
                let shape = self.adt_shapes.get(name).cloned().unwrap_or_default();
                (shape.size.max(4), shape.ptr_offsets)
            }
            TypeKey::Elem(kind) => (
                kind.byte_size(),
                if kind.is_ptr() { vec![0] } else { vec![] },
            ),
            TypeKey::Block { size, ptr_offsets } => (*size, ptr_offsets.clone()),
            // No layout to describe. Every slot is marked so the collector
            // follows all of them: a slot that turns out to hold an `int` is
            // merely retained, whereas an unmarked pointer slot would let a
            // live object be swept.
            TypeKey::OpaqueRecord => (48, (0..12).map(|w| w * 4).collect()),
        };
        let pointer_map = pointer_map_for(size, &offsets);
        let pointer_count = pointer_map
            .bytes
            .iter()
            .map(|b| b.count_ones())
            .sum::<u32>();
        TypeDescriptor {
            id,
            size,
            pointer_map,
            pointer_count,
        }
    }

    fn build_types(&mut self) {
        // Types 0 and 1 are reserved: function frame descriptors start at 2,
        // and that base is baked into `Frame`/`Spawn` operands and export
        // entries. Nothing allocates against them any more, but a stale
        // reference must still find a descriptor that traces conservatively
        // rather than one that claims the object holds no pointers at all.
        for id in 0..2u32 {
            self.types
                .push(self.descriptor_for(id, &TypeKey::OpaqueRecord));
        }
        // Types 2+: one per compiled function with its actual frame size and
        // the frame slots we know hold pointers. Frames are also scanned
        // conservatively by the collector, so an omission here costs nothing;
        // a false claim of "no pointers" would not be so harmless.
        for (i, (fsize, ptrs)) in self.func_frames.clone().iter().enumerate() {
            let pointer_map = pointer_map_for(*fsize, ptrs);
            let pointer_count = pointer_map
                .bytes
                .iter()
                .map(|b| b.count_ones())
                .sum::<u32>();
            self.types.push(TypeDescriptor {
                id: (2 + i) as u32,
                size: *fsize,
                pointer_map,
                pointer_count,
            });
        }
        // Element descriptors for arrays built in the data section join the
        // same table, so an image and a runtime allocation of the same element
        // kind share one descriptor.
        let pending = std::mem::take(&mut self.pending_array_elem_types);
        for (_, elem) in &pending {
            let key = TypeKey::Elem(*elem);
            if !self.needed_types.contains(&key) {
                self.needed_types.push(key);
            }
        }
        let base = self.types.len() as i32;
        for (i, key) in self.needed_types.clone().iter().enumerate() {
            let td = self.descriptor_for((base + i as i32) as u32, key);
            self.types.push(td);
        }
        let index_of = |key: &TypeKey| -> i32 {
            base + self
                .needed_types
                .iter()
                .position(|k| k == key)
                .expect("every needed type was allocated") as i32
        };
        for (item, elem) in &pending {
            let id = index_of(&TypeKey::Elem(*elem));
            if let DataItem::Array { element_type, .. } = &mut self.data[*item] {
                *element_type = id;
            }
        }
        for (at, operand, key) in std::mem::take(&mut self.pending_type_fixups) {
            let id = index_of(&key);
            match operand {
                TypeOperand::Source => self.code[at].source = op_imm(id),
                TypeOperand::Middle => self.code[at].middle = mid_imm(id),
            }
        }
    }

    fn gen_func(&mut self, func: &FuncDecl) -> Result<(), String> {
        let entry_pc = self.code.len();
        self.locals.clear();
        // Tuple shapes are keyed by name and decide how *wide* a slot is, so
        // one function's `t: (int, int)` must not make another function's
        // `t: int` be read eight bytes at a time. Frame locals are per
        // function; this map has to be too.
        self.local_tuple.clear();
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
                // A tuple parameter occupies its whole width in the frame,
                // and the caller packs it at exactly these offsets.
                let param_width = match &param.ty {
                    Type::Tuple(fields) => compute_tuple_layout(fields).size,
                    _ => kind.byte_size(),
                };
                if name != "nil" {
                    self.locals.push((name.clone(), param_off, ty, kind));
                    if let Type::Tuple(fields) = &param.ty {
                        self.local_tuple.insert(name.clone(), fields.clone());
                    }
                    // Track array params' element Type so indexing into
                    // them later picks the right Ind/Mov opcode pair, and
                    // recursive nested types (`array of array of T`) work
                    // by peeling one layer per Index.
                    if let Type::Array(elem) = &param.ty {
                        self.local_array_elem.insert(name.clone(), (**elem).clone());
                    }
                    // Same for chan params so Send/Recv on them know the
                    // element width.
                    if let Type::Chan(elem) | Type::BufChan(_, elem) = &param.ty {
                        self.local_chan_elem.insert(name.clone(), (**elem).clone());
                    }
                    // ADT-typed params (`p: ref Foo` or `p: Foo`): record the
                    // ADT name so field access through `p` finds the layout.
                    if let Some(adt) = Self::adt_name_for_type(&param.ty) {
                        self.local_adt_type.insert(name.clone(), adt);
                    }
                    // A module-typed param is a module handle: `f(b: Bufio)`
                    // can call `b->open(...)`.
                    self.note_module_handle(name, Some(&param.ty), None);
                }
                // Frame param slots are 4-byte aligned in the reference ABI;
                // big/real params occupy two adjacent slots. This keeps the
                // offsets consistent with how the caller packs arguments.
                param_off += param_width;
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
        // Parameters and named locals whose type is a pointer. Temps are not
        // tracked, so the map is a subset of the frame's real pointers —
        // which is the safe direction: frames are scanned conservatively by
        // the collector, and the reference VM's `freeptrs` would merely leak
        // rather than release something twice.
        let frame_ptrs: Vec<i32> = self
            .locals
            .iter()
            .filter(|(_, _, ty, kind)| *ty != ValType::Word && *kind == NumKind::Word)
            .map(|(_, off, _, _)| *off)
            .collect();
        self.func_frames.push((self.frame_size, frame_ptrs));

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
                let chan_elem_type = decl_chan_elem_type(v);
                let adt_name = v.ty.as_ref().and_then(Self::adt_name_for_type);
                for name in &v.names {
                    match &v.ty {
                        // A declared tuple type sizes the local by the whole
                        // tuple and records where each `.tN` sits in it.
                        Some(t @ Type::Tuple(_)) => {
                            self.declare_local_of_type(name, t);
                        }
                        _ => {
                            self.alloc_local(name, ty, kind);
                        }
                    }
                    if let Some(t) = &elem_type {
                        self.local_array_elem.insert(name.clone(), t.clone());
                    }
                    if let Some(t) = &chan_elem_type {
                        self.local_chan_elem.insert(name.clone(), t.clone());
                    }
                    if let Some(a) = &adt_name {
                        self.local_adt_type.insert(name.clone(), a.clone());
                    }
                    self.note_module_handle(name, v.ty.as_ref(), v.init.as_ref());
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
                    if let Some(layout) = self.tuple_layout_of(e) {
                        // The whole tuple goes through the return pointer at
                        // the very offsets the caller reads it back from.
                        let block = self.alloc_temp_block(layout.size);
                        self.gen_expr_to(e, block)?;
                        for (ty, off) in layout.fields.clone() {
                            self.emit(
                                mov_opcode_for_type(&ty),
                                op_fp(block + off),
                                mid_unused(),
                                op_fp_ind(16, off),
                            );
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
                            let slot = self.gen_arg_value(arg)?;
                            arg_off = self.store_arg(&slot, frame_tmp, arg_off);
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
            // Already bound by `collect_imports`; declares no storage and
            // emits no code.
            Stmt::Import(_) => Ok(()),
            Stmt::Alt(s) => self.gen_alt(s),
            Stmt::Pick(s) => self.gen_pick(s),
            // Unsupported constructs are hard errors: emitting nothing at all
            // would silently drop the statement's behavior.
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

    // ── Tuple values ──────────────────────────────────────────
    //
    // A tuple is a contiguous block of frame, not a pointer to one. Every
    // site that stores, passes, sends or returns one has to agree on the
    // layout `compute_tuple_layout` produces, and has to reserve the whole
    // block: a tuple written into a 4-byte slot loses every field but the
    // first, silently.

    /// The element type of the channel `chan` denotes, if it is known.
    fn chan_elem_type(&self, chan: &Expr) -> Option<Type> {
        match chan {
            Expr::Ident(name, _) => self.local_chan_elem.get(name).cloned(),
            Expr::ChanAlloc(ty, _) => Some((**ty).clone()),
            Expr::Dot(inner, field, _) => self
                .adt_name_for_expr(inner)
                .and_then(|a| self.adt_field_info(&a, field))
                .and_then(|(_, t)| match t {
                    Type::Chan(e) | Type::BufChan(_, e) => Some(*e),
                    _ => None,
                }),
            _ => None,
        }
    }

    /// The type of a tuple element written as an expression. Only two things
    /// about it matter here — how wide it is, and whether the collector has
    /// to follow it — so a pointer of any kind is reported as `string`.
    fn tuple_field_type(&self, e: &Expr) -> Type {
        // A field that is itself a tuple is as wide as that tuple; calling it
        // one word would let its later fields overwrite the outer tuple's.
        if let Some(fields) = self.tuple_fields_of(e) {
            return Type::Tuple(fields);
        }
        match self.infer_num_kind(e) {
            NumKind::Big => Type::Basic(BasicType::Big),
            NumKind::Real => Type::Basic(BasicType::Real),
            NumKind::Word => match self.infer_expr_type(e) {
                ValType::Word => Type::Basic(BasicType::Int),
                _ => Type::Basic(BasicType::String),
            },
        }
    }

    /// The field types of `expr`, if `expr` denotes a tuple.
    fn tuple_fields_of(&self, expr: &Expr) -> Option<Vec<Type>> {
        match expr {
            Expr::Tuple(elems, _) => Some(elems.iter().map(|e| self.tuple_field_type(e)).collect()),
            Expr::Ident(name, _) => self.local_tuple.get(name).cloned(),
            Expr::Call(callee, _, _) => match callee.as_ref() {
                Expr::Ident(n, _) => self.func_tuple_ret.get(n).cloned(),
                _ => None,
            },
            Expr::Recv(chan, _) => match self.chan_elem_type(chan) {
                Some(Type::Tuple(fields)) => Some(fields),
                _ => None,
            },
            Expr::Index(arr, _, _) => match self.array_elem_type_of(arr) {
                Some(Type::Tuple(fields)) => Some(fields),
                _ => None,
            },
            Expr::Dot(inner, field, _) => {
                // An ADT field of tuple type, or `.tN` of a tuple whose Nth
                // field is itself a tuple.
                if let Some(fields) = self.tuple_fields_of(inner)
                    && let Some(n) = tuple_field_index(field)
                {
                    return match compute_tuple_layout(&fields).field(n) {
                        Some((Type::Tuple(inner_fields), _)) => Some(inner_fields.clone()),
                        _ => None,
                    };
                }
                self.adt_name_for_expr(inner)
                    .and_then(|a| self.adt_field_info(&a, field))
                    .and_then(|(_, t)| match t {
                        Type::Tuple(fields) => Some(fields),
                        _ => None,
                    })
            }
            Expr::TupleDeclAssign(_, rhs, _)
            | Expr::DeclAssign(_, rhs, _)
            | Expr::Assign(_, rhs, _) => self.tuple_fields_of(rhs),
            _ => None,
        }
    }

    /// The element type of the array `arr` denotes, if it is known.
    fn array_elem_type_of(&self, arr: &Expr) -> Option<Type> {
        match arr {
            Expr::Ident(name, _) => self.local_array_elem.get(name).cloned(),
            _ => None,
        }
    }

    fn tuple_layout_of(&self, expr: &Expr) -> Option<TupleLayout> {
        self.tuple_fields_of(expr)
            .map(|fields| compute_tuple_layout(&fields))
    }

    /// Copy one value of `ty` between two frame offsets. A tuple is copied
    /// field by field, because no single Mov is wide enough for one.
    fn copy_value(&mut self, ty: &Type, src: i32, dst: i32) {
        if src == dst {
            return;
        }
        if let Type::Tuple(fields) = ty {
            for (fty, off) in compute_tuple_layout(fields).fields {
                self.copy_value(&fty, src + off, dst + off);
            }
            return;
        }
        self.emit(
            mov_opcode_for_type(ty),
            op_fp(src),
            mid_unused(),
            op_fp(dst),
        );
    }

    /// Declare a local of the given Limbo type, reserving a whole block for a
    /// tuple and registering its shape so `.tN` and destructuring resolve.
    fn declare_local_of_type(&mut self, name: &str, ty: &Type) -> i32 {
        if let Type::Tuple(fields) = ty {
            let size = compute_tuple_layout(fields).size;
            let off = self.alloc_local_block(name, size);
            self.local_tuple.insert(name.to_string(), fields.clone());
            return off;
        }
        self.alloc_local(name, val_type_of(ty), type_num_kind(ty))
    }

    /// Write a tuple literal's fields into the block at `dst`.
    fn gen_tuple_literal_to(&mut self, elems: &[Expr], dst: i32) -> Result<(), String> {
        let fields: Vec<Type> = elems.iter().map(|e| self.tuple_field_type(e)).collect();
        let layout = compute_tuple_layout(&fields);
        for (elem, (ty, off)) in elems.iter().zip(layout.fields.iter()) {
            self.gen_expr_to_kind(elem, dst + off, type_num_kind(ty))?;
        }
        Ok(())
    }

    /// Get `expr`'s tuple value into a frame block, returning where it is and
    /// how it is laid out. A tuple already living in a local is used where it
    /// lies; anything else is materialised into a fresh block.
    fn gen_tuple_block(&mut self, expr: &Expr) -> Result<(i32, TupleLayout), String> {
        if let Expr::Ident(name, _) = expr
            && let Some(fields) = self.local_tuple.get(name).cloned()
            && let Some((off, _)) = self.get_local(name)
        {
            return Ok((off, compute_tuple_layout(&fields)));
        }
        // Array indexing has a fixed element stride and hands back one word,
        // so an array of tuples would yield the first field and three
        // uninitialised ones. Say so instead of producing that quietly.
        if let Expr::Index(arr, _, _) = expr
            && matches!(self.array_elem_type_of(arr), Some(Type::Tuple(_)))
        {
            return Err(
                "an array of tuples is not supported yet: element indexing moves a \
                        single word, so every field but the first would be read from the \
                        wrong place"
                    .to_string(),
            );
        }
        let Some(layout) = self.tuple_layout_of(expr) else {
            return Err(format!(
                "{} has no known tuple type here",
                describe_expr_kind(expr)
            ));
        };
        let block = self.alloc_temp_block(layout.size);
        self.gen_expr_to(expr, block)?;
        Ok((block, layout))
    }

    /// `(a, b) = rhs` — assignment, not declaration. The right-hand side is
    /// built whole before any target is written, so `(a, b) = (b, a)` swaps
    /// rather than reading back what it has just stored.
    fn gen_tuple_assign(&mut self, targets: &[Expr], rhs: &Expr) -> Result<(), String> {
        let layout = match self.tuple_layout_of(rhs) {
            Some(l) if l.fields.len() == targets.len() => l,
            _ => compute_tuple_layout(&vec![Type::Basic(BasicType::Int); targets.len()]),
        };
        let block = self.alloc_temp_block(layout.size);
        self.gen_expr_to(rhs, block)?;
        for (target, (ty, off)) in targets.iter().zip(layout.fields.clone()) {
            match target {
                // `nil` names a field that is deliberately dropped.
                Expr::Nil(_) => {}
                Expr::Ident(name, _) => {
                    let slot = match self.lookup_var(name) {
                        Some((slot, _, _)) => slot,
                        // Matches `x = expr`'s leniency about undeclared
                        // names rather than rejecting what used to compile.
                        None => Slot::Local(self.declare_local_of_type(name, &ty)),
                    };
                    match (&ty, slot) {
                        (Type::Tuple(_), Slot::Local(dst)) => {
                            self.copy_value(&ty, block + off, dst)
                        }
                        _ => self.emit(
                            mov_opcode_for_type(&ty),
                            op_fp(block + off),
                            mid_unused(),
                            slot.operand(),
                        ),
                    }
                }
                // `(date, tm.mday) = datenum(date)`, `(m[i], adp) = ...`:
                // any lvalue an ordinary assignment accepts works here too.
                // The field is named to the assignment machinery by parking
                // a synthetic local on the block slot it already occupies —
                // no copy, and no second implementation of lvalue stores.
                other => {
                    let alias = format!("\u{0}tuple-field-{}", self.locals.len());
                    self.locals.push((
                        alias.clone(),
                        block + off,
                        val_type_of(&ty),
                        type_num_kind(&ty),
                    ));
                    let assign = Expr::Assign(
                        Box::new(other.clone()),
                        Box::new(Expr::Ident(alias, Span::default())),
                        Span::default(),
                    );
                    let result = self.gen_expr_discard(&assign);
                    self.locals.pop();
                    result?;
                }
            }
        }
        Ok(())
    }

    /// `(a, b) := rhs`. `dst`, when given, is a block the caller has already
    /// sized to the tuple, and also receives the value — that is what makes
    /// `((a, b) := <-c).t0` work.
    fn gen_tuple_decl_assign(
        &mut self,
        names: &[String],
        rhs: &Expr,
        dst: Option<i32>,
    ) -> Result<(), String> {
        // An rhs of unknown shape keeps the historical one-word-per-name
        // packing, which is what tuple-returning calls into other modules
        // still rely on.
        let layout = match self.tuple_layout_of(rhs) {
            Some(l) if l.fields.len() == names.len() => l,
            _ => compute_tuple_layout(&vec![Type::Basic(BasicType::Int); names.len()]),
        };
        let block = match dst {
            Some(d) => d,
            None => self.alloc_temp_block(layout.size),
        };
        self.gen_expr_to(rhs, block)?;
        for (name, (ty, off)) in names.iter().zip(layout.fields.iter()) {
            if name == "nil" {
                continue;
            }
            let local = self.declare_local_of_type(name, ty);
            self.copy_value(ty, block + off, local);
        }
        Ok(())
    }

    /// `chan <-= val`. The value slot is sized by the channel's element type,
    /// because `op_send` reads exactly `elem_size` bytes out of it: too
    /// narrow a slot sends whatever happens to follow it in the frame.
    fn gen_send(&mut self, chan_expr: &Expr, val_expr: &Expr) -> Result<(), String> {
        let elem = self.chan_elem_type(chan_expr);
        let chan_tmp = self.alloc_temp();
        self.gen_expr_to(chan_expr, chan_tmp)?;
        if let Some(Type::Tuple(fields)) = &elem {
            let layout = compute_tuple_layout(fields);
            let val_tmp = self.alloc_temp_block(layout.size);
            self.gen_expr_to(val_expr, val_tmp)?;
            self.emit(Opcode::Send, op_fp(val_tmp), mid_unused(), op_fp(chan_tmp));
            return Ok(());
        }
        let val_kind = elem
            .as_ref()
            .map(type_num_kind)
            .unwrap_or_else(|| self.infer_num_kind(val_expr));
        let val_tmp = self.alloc_temp_for(val_kind);
        self.gen_expr_to_kind(val_expr, val_tmp, val_kind)?;
        self.emit(Opcode::Send, op_fp(val_tmp), mid_unused(), op_fp(chan_tmp));
        Ok(())
    }

    /// Evaluate one call argument into a temp, recording what it takes to
    /// move it into a callee frame later.
    fn gen_arg_value(&mut self, arg: &Expr) -> Result<ArgSlot, String> {
        if let Some(layout) = self.tuple_layout_of(arg) {
            let tmp = self.alloc_temp_block(layout.size);
            self.gen_expr_to(arg, tmp)?;
            return Ok(ArgSlot {
                tmp,
                tuple: Some(layout),
                op: Opcode::Movw,
                width: 0,
            });
        }
        let kind = self.infer_num_kind(arg);
        let tmp = self.alloc_temp_for(kind);
        self.gen_expr_to(arg, tmp)?;
        let ty = self.infer_expr_type(arg);
        let op = match (ty, kind) {
            (_, NumKind::Big) => Opcode::Movl,
            (_, NumKind::Real) => Opcode::Movf,
            (ValType::Word, NumKind::Word) => Opcode::Movw,
            _ => Opcode::Movp,
        };
        Ok(ArgSlot {
            tmp,
            tuple: None,
            op,
            width: kind.byte_size(),
        })
    }

    /// Store an evaluated argument into the callee frame at `arg_off`,
    /// returning the offset the next argument goes to.
    ///
    /// A tuple argument occupies its whole width in the callee's frame and
    /// moves field by field; the callee's own parameter layout advances by
    /// the same amount, which is what keeps the two in step.
    fn store_arg(&mut self, slot: &ArgSlot, frame_tmp: i32, arg_off: i32) -> i32 {
        match &slot.tuple {
            Some(layout) => {
                for (ty, off) in layout.fields.clone() {
                    self.emit(
                        mov_opcode_for_type(&ty),
                        op_fp(slot.tmp + off),
                        mid_unused(),
                        op_fp_ind(frame_tmp, arg_off + off),
                    );
                }
                arg_off + layout.size
            }
            None => {
                self.emit(
                    slot.op,
                    op_fp(slot.tmp),
                    mid_unused(),
                    op_fp_ind(frame_tmp, arg_off),
                );
                arg_off + slot.width
            }
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
                } else if let Expr::ModQual(module, name, _) = callee.as_ref() {
                    match module.as_ref() {
                        Expr::Ident(handle, _) => self.module_call_num_kind(handle, name),
                        _ => NumKind::Word,
                    }
                } else if let Expr::Dot(obj, method, _) = callee.as_ref() {
                    match self.adt_method_target(obj, method) {
                        Some((adt, _)) => self.adt_method_num_kind(&adt, method),
                        None => NumKind::Word,
                    }
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
                    && let Some(t) = self.local_chan_elem.get(name)
                {
                    type_num_kind(t)
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
                // Infer the return shape from the callee's interface when it
                // is known, falling back to what we know about `$Sys`.
                if let Expr::ModQual(module, name, _) = callee.as_ref() {
                    match module.as_ref() {
                        Expr::Ident(handle, _) => self.module_call_val_type(handle, name),
                        _ => ValType::Word,
                    }
                } else if let Expr::Dot(obj, method, _) = callee.as_ref() {
                    match self
                        .adt_method_target(obj, method)
                        .and_then(|(adt, _)| self.adt_method_sig(&adt, method))
                        .and_then(|sig| sig.ret.as_ref())
                    {
                        Some(Type::Basic(
                            BasicType::Int | BasicType::Byte | BasicType::Big | BasicType::Real,
                        )) => ValType::Word,
                        Some(Type::Array(_)) => ValType::Array,
                        Some(_) => ValType::Ptr,
                        None => ValType::Word,
                    }
                } else {
                    ValType::Word
                }
            }
            Expr::Cons(_, _, _) => ValType::Ptr,
            Expr::ArrayAlloc(_, _, _) | Expr::ArrayLit(_, _, _, _) => ValType::Array,
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

    /// `alt { guard => body ... }`: wait until one of several channel
    /// operations can proceed, then run that guard's arm.
    ///
    /// The instruction reads a table of `{nsend, nrecv}` followed by one
    /// `{channel, value address}` pair per guard, sends first, and answers
    /// with the index of the entry it selected. Two properties of the
    /// instruction shape the lowering:
    ///
    /// - A `*` arm occupies no entry. Its index is `nsend + nrecv`, which is
    ///   what `nbalt` reports when nothing was ready, and a zeroed entry would
    ///   name a nil channel, which the VM raises on.
    /// - A blocking `alt` parks the thread and re-executes the same
    ///   instruction when it wakes, so everything the guards need has to be in
    ///   the table before the instruction runs, and the addresses in the table
    ///   have to survive the suspension. Frame slots do, which is why the
    ///   table and every value slot live in the frame.
    fn gen_alt(&mut self, s: &AltStmt) -> Result<(), String> {
        // An `alt` is breakable (a `break` in an arm leaves the alt) but not
        // continuable, exactly like a `case` (`limbo/com.c:579-605`).
        self.push_loop(false);
        let result = self.gen_alt_body(s);
        if result.is_err() {
            self.loop_stack.pop();
        }
        result
    }

    fn gen_alt_body(&mut self, s: &AltStmt) -> Result<(), String> {
        // Entry indices follow the reference (`limbo/com.c:969-988`): the
        // n-th send is entry n, the m-th receive is entry `nsend + m`, both in
        // source order, and a `*` guard is skipped.
        let guards = || s.arms.iter().flat_map(|a| &a.guards);
        let nsend = guards()
            .filter(|g| matches!(g, AltGuard::Send(_, _)))
            .count();
        let nrecv = guards()
            .filter(|g| matches!(g, AltGuard::Recv(_, _)))
            .count();
        let count = nsend + nrecv;

        let mut entries: Vec<AltEntry<'_>> = Vec::with_capacity(count);
        let mut wildcard_arm: Option<usize> = None;
        let mut next_send = 0usize;
        let mut next_recv = 0usize;
        for (arm, alt_arm) in s.arms.iter().enumerate() {
            for guard in &alt_arm.guards {
                let index = match guard {
                    AltGuard::Send(_, _) => {
                        next_send += 1;
                        next_send - 1
                    }
                    // Receives are numbered from `nsend`, not from zero: they
                    // share one table with the sends.
                    AltGuard::Recv(_, _) => {
                        next_recv += 1;
                        nsend + next_recv - 1
                    }
                    AltGuard::Wildcard => {
                        if wildcard_arm.is_some() {
                            return Err("an `alt` may have only one `*` arm".to_string());
                        }
                        wildcard_arm = Some(arm);
                        continue;
                    }
                };
                entries.push(AltEntry {
                    guard,
                    index,
                    arm,
                    slot: 0,
                    prologue: AltPrologue::None,
                });
            }
        }

        let table = self.alloc_temp_block(8 + 8 * count as i32);
        let selection = self.alloc_temp();

        // Setup: one channel word and one value address per entry, then the
        // header, then the instruction. Guards are evaluated in source order,
        // as the reference does, so a guard's side effects happen in the order
        // the program wrote them.
        for entry in &mut entries {
            let guard = entry.guard;
            let (chan_expr, value) = match guard {
                AltGuard::Send(chan, value) => (chan, Some(value)),
                AltGuard::Recv(_, chan) => (chan, None),
                // Wildcard guards never became entries.
                AltGuard::Wildcard => continue,
            };
            // `(i, v) := <-a` over an `array of chan of T` waits on every
            // element at once. The reference expands one table entry per
            // element (`libinterp/alt.c:19-40`); our table names one channel
            // per entry and the instruction reports an entry index, not an
            // array index, so there is nothing to lower this to.
            if let Some(Type::Chan(_) | Type::BufChan(_, _)) = self.array_elem_type_of(chan_expr) {
                return Err(
                    "an `alt` guard on an array of channels is not supported: each table \
                     entry names one channel, so the array's elements cannot be waited on \
                     together"
                        .to_string(),
                );
            }
            let elem = self.chan_elem_type(chan_expr);
            let chan_tmp = self.alloc_temp();
            self.gen_expr_to(chan_expr, chan_tmp)?;
            self.emit(
                Opcode::Movp,
                op_fp(chan_tmp),
                mid_unused(),
                op_fp(table + 8 + 8 * entry.index as i32),
            );
            match value {
                Some(value) => {
                    // The instruction sends out of this slot, so the value has
                    // to be there before it executes.
                    let slot = self.alloc_chan_slot(elem.as_ref());
                    match &elem {
                        Some(Type::Tuple(_)) | None => self.gen_expr_to(value, slot)?,
                        Some(t) => self.gen_expr_to_kind(value, slot, type_num_kind(t))?,
                    }
                    entry.slot = slot;
                }
                None => {
                    let AltGuard::Recv(dest, _) = guard else {
                        continue;
                    };
                    let (slot, prologue) = self.alt_recv_destination(dest, elem.as_ref())?;
                    entry.slot = slot;
                    entry.prologue = prologue;
                }
            }
            self.emit(
                Opcode::Lea,
                op_fp(entry.slot),
                mid_unused(),
                op_fp(table + 12 + 8 * entry.index as i32),
            );
        }
        self.emit(
            Opcode::Movw,
            op_imm(nsend as i32),
            mid_unused(),
            op_fp(table),
        );
        self.emit(
            Opcode::Movw,
            op_imm(nrecv as i32),
            mid_unused(),
            op_fp(table + 4),
        );
        // `nbalt` is chosen by the presence of a `*` arm, and by nothing else
        // (`limbo/com.c:1073-1075`).
        let op = match wildcard_arm {
            Some(_) => Opcode::Nbalt,
            None => Opcode::Alt,
        };
        self.emit(op, op_fp(table), mid_unused(), op_fp(selection));

        // Dispatch on the reported index: one landing pad per entry, plus the
        // `*` arm at index `count`.
        let mut pads: Vec<usize> = Vec::with_capacity(count);
        for index in 0..count {
            pads.push(self.code.len());
            self.emit(
                Opcode::Beqw,
                op_fp(selection),
                mid_imm(index as i32),
                op_imm(0),
            );
        }
        let wildcard_pad = wildcard_arm.map(|_| {
            let at = self.code.len();
            self.emit(
                Opcode::Beqw,
                op_fp(selection),
                mid_imm(count as i32),
                op_imm(0),
            );
            at
        });
        let no_match = self.code.len();
        self.emit(Opcode::Jmp, op_unused(), mid_unused(), op_imm(0));

        // Arms, in source order. Each guard of an arm has its own landing pad
        // and its own prologue, and they all fall through to the one body.
        let mut end_jumps = vec![no_match];
        for (arm, alt_arm) in s.arms.iter().enumerate() {
            let mine: Vec<usize> = entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.arm == arm)
                .map(|(i, _)| i)
                .collect();
            let is_wildcard = wildcard_arm == Some(arm);
            let mut to_body = Vec::new();
            for (n, &i) in mine.iter().enumerate() {
                let pad_pc = op_imm(self.code.len() as i32);
                self.code[pads[entries[i].index]].destination = pad_pc;
                self.gen_alt_prologue(&entries[i])?;
                if n + 1 < mine.len() || is_wildcard {
                    to_body.push(self.code.len());
                    self.emit(Opcode::Jmp, op_unused(), mid_unused(), op_imm(0));
                }
            }
            if is_wildcard && let Some(at) = wildcard_pad {
                let pad_pc = op_imm(self.code.len() as i32);
                self.code[at].destination = pad_pc;
            }
            let body_pc = self.code.len() as i32;
            for at in to_body {
                self.code[at].destination = op_imm(body_pc);
            }
            for stmt in &alt_arm.body {
                self.gen_stmt(stmt)?;
            }
            end_jumps.push(self.code.len());
            self.emit(Opcode::Jmp, op_unused(), mid_unused(), op_imm(0));
        }

        let end_pc = self.code.len() as i32;
        for at in end_jumps {
            self.code[at].destination = op_imm(end_pc);
        }
        // `break` in an arm lands here too; `continue` never targets an `alt`.
        self.pop_loop(end_pc, end_pc);
        Ok(())
    }

    /// `pick x := e { Tag => ... }`: run the arm naming the variant `e` was
    /// built with, with `x` bound to `e` at that variant's type.
    ///
    /// The tag lives in word 0 of the record, so the dispatch is an ordinary
    /// comparison chain over a word load. A `pick` that names neither every
    /// variant nor `*` falls through to the statement after it, which is what
    /// the single-arm downcast idiom relies on.
    fn gen_pick(&mut self, s: &PickStmt) -> Result<(), String> {
        // Breakable but not continuable, exactly like `case` and `alt`
        // (`limbo/com.c:579-605`).
        self.push_loop(false);
        let result = self.gen_pick_body(s);
        if result.is_err() {
            self.loop_stack.pop();
        }
        result
    }

    fn gen_pick_body(&mut self, s: &PickStmt) -> Result<(), String> {
        let Some(adt) = self.adt_name_for_expr(&s.expr) else {
            return Err(format!(
                "`pick {} := ...`: the ADT of the value being picked over is \
                 not known here, so its tags cannot be resolved",
                s.name
            ));
        };
        // The value may already be narrowed to a variant (`pick y := x` inside
        // another arm); tags are named against the ADT either way.
        let base = adt.split('.').next().unwrap_or(&adt).to_string();

        let bind = self.alloc_local(&s.name, ValType::Ptr, NumKind::Word);
        self.gen_expr_to(&s.expr, bind)?;
        let tag = self.alloc_temp();
        self.emit(Opcode::Movw, op_fp_ind(bind, 0), mid_unused(), op_fp(tag));

        let mut pads: Vec<(usize, usize)> = Vec::new();
        let mut wildcard: Option<usize> = None;
        for (arm, pick_arm) in s.arms.iter().enumerate() {
            for name in &pick_arm.tags {
                if name == "*" {
                    if wildcard.is_some() {
                        return Err("a `pick` may have only one `*` arm".to_string());
                    }
                    wildcard = Some(arm);
                    continue;
                }
                let Some((value, _)) = self.adt_variants.get(&format!("{base}.{name}")).copied()
                else {
                    // An ADT with no declaration in scope has no variants
                    // either, and that is the more useful thing to report.
                    return Err(if self.adt_layouts.contains_key(&base) {
                        format!("`{name}` is not a variant of `{base}`")
                    } else {
                        format!(
                            "the declaration of `{base}` was not found, so `{name}` cannot be \
                             resolved to a tag (is the include path set?)"
                        )
                    });
                };
                pads.push((self.code.len(), arm));
                self.emit(Opcode::Beqw, op_fp(tag), mid_imm(value), op_imm(0));
            }
        }
        let default = self.code.len();
        self.emit(Opcode::Jmp, op_unused(), mid_unused(), op_imm(0));

        // Inside an arm the bound name has that arm's variant type, so the
        // variant's own fields resolve; outside it goes back to whatever it
        // was, which is what makes a `pick` over a name that already exists
        // leave that name alone.
        let saved = self.local_adt_type.get(&s.name).cloned();
        let mut end_jumps = Vec::new();
        for (arm, pick_arm) in s.arms.iter().enumerate() {
            let body_pc = op_imm(self.code.len() as i32);
            for (at, owner) in &pads {
                if *owner == arm {
                    self.code[*at].destination = body_pc;
                }
            }
            if wildcard == Some(arm) {
                self.code[default].destination = body_pc;
            }
            let layout = self.pick_arm_type(&base, &pick_arm.tags);
            self.local_adt_type.insert(s.name.clone(), layout);
            for stmt in &pick_arm.body {
                self.gen_stmt(stmt)?;
            }
            end_jumps.push(self.code.len());
            self.emit(Opcode::Jmp, op_unused(), mid_unused(), op_imm(0));
        }
        match saved {
            Some(previous) => self.local_adt_type.insert(s.name.clone(), previous),
            None => self.local_adt_type.remove(&s.name),
        };

        let end_pc = self.code.len() as i32;
        if wildcard.is_none() {
            self.code[default].destination = op_imm(end_pc);
        }
        for at in end_jumps {
            self.code[at].destination = op_imm(end_pc);
        }
        self.pop_loop(end_pc, end_pc);
        Ok(())
    }

    /// The layout the bound name has inside one arm: the variant's own when
    /// the arm names a single variant or several from one `pick` group, and
    /// the ADT's common fields when the arm spans groups or is `*`.
    fn pick_arm_type(&self, base: &str, tags: &[String]) -> String {
        let mut group = None;
        for name in tags {
            match self.adt_variants.get(&format!("{base}.{name}")) {
                Some((_, g)) if group.is_none_or(|seen| seen == *g) => group = Some(*g),
                _ => return base.to_string(),
            }
        }
        match tags.first() {
            Some(name) if group.is_some() => format!("{base}.{name}"),
            _ => base.to_string(),
        }
    }

    /// Reserve an anonymous frame slot wide enough for one message of a
    /// channel with this element type. Too narrow a slot would have the
    /// transfer run over whatever follows it in the frame.
    fn alloc_chan_slot(&mut self, elem: Option<&Type>) -> i32 {
        match elem {
            Some(Type::Tuple(fields)) => {
                let size = compute_tuple_layout(fields).size;
                self.alloc_temp_block(size)
            }
            Some(t) => self.alloc_temp_for(type_num_kind(t)),
            None => self.alloc_temp(),
        }
    }

    /// Where an `alt` receive guard puts its value, and what the arm has to do
    /// with it afterwards.
    ///
    /// The address in the table is read as an offset into the frame, so the
    /// slot has to be a frame slot: a module-level variable or an array
    /// element is written by the arm's prologue instead, out of a frame temp.
    fn alt_recv_destination(
        &mut self,
        dest: &Option<AltDest>,
        elem: Option<&Type>,
    ) -> Result<(i32, AltPrologue), String> {
        match dest {
            // `<-c`: the value is received and dropped.
            None => Ok((self.alloc_chan_slot(elem), AltPrologue::None)),
            Some(AltDest::Decl(names)) => {
                let [name] = names.as_slice() else {
                    return Err(format!(
                        "an `alt` receive guard declares one name, not {}",
                        names.len()
                    ));
                };
                let slot = match elem {
                    Some(t) => self.declare_local_of_type(name, t),
                    None => self.alloc_local(name, ValType::Word, NumKind::Word),
                };
                // The same sidecars `x := <-c` records outside an `alt`, so
                // that the arm body can index, call through, or select fields
                // of the value it just received.
                if let Some(t) = elem {
                    match t {
                        Type::Array(e) => {
                            self.local_array_elem.insert(name.clone(), (**e).clone());
                        }
                        Type::Chan(e) | Type::BufChan(_, e) => {
                            self.local_chan_elem.insert(name.clone(), (**e).clone());
                        }
                        _ => {}
                    }
                    if let Some(adt) = Self::adt_name_for_type(t) {
                        self.local_adt_type.insert(name.clone(), adt);
                    }
                }
                Ok((slot, AltPrologue::None))
            }
            Some(AltDest::TupleDecl(names)) => {
                let Some(Type::Tuple(fields)) = elem else {
                    return Err(
                        "an `alt` guard receiving into a tuple needs a channel of a known \
                         tuple type"
                            .to_string(),
                    );
                };
                let layout = compute_tuple_layout(fields);
                let block = self.alloc_temp_block(layout.size);
                Ok((
                    block,
                    AltPrologue::Tuple {
                        names: names.clone(),
                        fields: fields.clone(),
                    },
                ))
            }
            // A plain frame local receives in place; anything else needs a
            // store from a temp once the arm is chosen.
            Some(AltDest::Assign(target)) => {
                if let Expr::Ident(name, _) = target
                    && let Some((Slot::Local(off), _, _)) = self.lookup_var(name)
                {
                    return Ok((off, AltPrologue::None));
                }
                let slot = self.alloc_chan_slot(elem);
                Ok((
                    slot,
                    AltPrologue::Store {
                        target: target.clone(),
                        ty: elem.cloned().unwrap_or(Type::Basic(BasicType::Int)),
                    },
                ))
            }
        }
    }

    /// Code that runs after an `alt` selects an entry and before the arm's
    /// body: it moves the received value from where the instruction had to put
    /// it to where the source says it goes.
    fn gen_alt_prologue(&mut self, entry: &AltEntry<'_>) -> Result<(), String> {
        match &entry.prologue {
            AltPrologue::None => Ok(()),
            AltPrologue::Tuple { names, fields } => {
                let layout = compute_tuple_layout(fields);
                for (name, (ty, off)) in names.iter().zip(layout.fields.clone()) {
                    if name == "nil" {
                        continue;
                    }
                    let local = self.declare_local_of_type(name, &ty);
                    self.copy_value(&ty, entry.slot + off, local);
                }
                Ok(())
            }
            // Name the temp to the ordinary assignment path, which already
            // knows how to write every kind of lvalue there is.
            AltPrologue::Store { target, ty } => {
                let alias = format!("\u{0}alt-recv-{}", self.locals.len());
                self.locals.push((
                    alias.clone(),
                    entry.slot,
                    val_type_of(ty),
                    type_num_kind(ty),
                ));
                let assign = Expr::Assign(
                    Box::new(target.clone()),
                    Box::new(Expr::Ident(alias.clone(), Span::default())),
                    Span::default(),
                );
                let result = self.gen_expr_discard(&assign);
                // Removed by name: the assignment may have declared a local of
                // its own, and popping would take that one instead.
                self.locals.retain(|(name, _, _, _)| name != &alias);
                result
            }
        }
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
                } else if let Expr::Tuple(targets, _) = lhs.as_ref() {
                    self.gen_tuple_assign(targets, rhs)
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
                    //
                    // A tuple's `.tN` is not reached through a pointer — the
                    // tuple is the value. Writing one as if it were a ref
                    // would dereference the first field as an address, so
                    // write into the block, or say why we can't.
                    if let Some(n) = tuple_field_index(field)
                        && let Some(fields) = self.tuple_fields_of(inner_expr)
                    {
                        let layout = compute_tuple_layout(&fields);
                        let Some((fty, foff)) = layout.field(n).cloned() else {
                            return Err(format!(
                                "`.{field}` is out of range for a {}-field tuple",
                                layout.fields.len()
                            ));
                        };
                        let Expr::Ident(base, _) = inner_expr.as_ref() else {
                            return Err(
                                "assigning to a field of a tuple that is not a variable is \
                                 not supported yet"
                                    .to_string(),
                            );
                        };
                        let Some((base_off, _)) = self.get_local(base) else {
                            return Err(format!("`{base}` is not a tuple this function declares"));
                        };
                        let val_tmp = self.alloc_temp_for(type_num_kind(&fty));
                        self.gen_expr_to_kind(rhs, val_tmp, type_num_kind(&fty))?;
                        self.copy_value(&fty, val_tmp, base_off + foff);
                        return Ok(());
                    }
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
                // A tuple-valued rhs needs a block-sized local: `t := (1, 2)`
                // into a 4-byte slot would write the second field over
                // whatever local came next.
                if let Some(fields) = self.tuple_fields_of(rhs) {
                    let off = self.declare_local_of_type(name, &Type::Tuple(fields));
                    return self.gen_expr_to(rhs, off);
                }
                let off = self.alloc_local(name, ty, kind);
                // Capture array element type when the rhs is `array[N] of T`
                // so subsequent indexing picks the right opcode pair.
                match rhs.as_ref() {
                    Expr::ArrayAlloc(_, ty, _) | Expr::ArrayLit(_, _, Some(ty), _) => {
                        self.local_array_elem
                            .insert(name.to_string(), (**ty).clone());
                    }
                    // An untyped literal still fixes an element width; reading
                    // a byte-packed array with word loads returns its
                    // neighbours' bytes.
                    Expr::ArrayLit(_, elems, None, _) => {
                        let kind = self.array_lit_elem_kind(elems, None);
                        self.local_array_elem
                            .insert(name.to_string(), Type::Basic(kind.basic()));
                    }
                    _ => {}
                }
                // Same for a channel, so Send/Recv and `alt` move a whole
                // message rather than a word. `c := chan of T` is the common
                // case, but a channel that arrives through another variable
                // (`r := dummy`, `r = f.read`) carries its element type just
                // as much.
                if let Some(elem) = self.chan_elem_type(rhs) {
                    self.local_chan_elem.insert(name.to_string(), elem);
                }
                // ADT inference: the parser emits `ref Foo(...)` as
                // `Unary(Ref, Call(Ident(Foo), args))`, not RefAlloc. Detect
                // both shapes so DeclAssign records the ADT name.
                let adt_from_rhs = match rhs.as_ref() {
                    Expr::RefAlloc(ty, _, _) => Self::adt_name_for_type(ty),
                    // `ref Adt(...)`, `ref Mod->Adt(...)` and the bare
                    // `ref Adt` form all name the ADT the local holds.
                    Expr::Unary(UnaryOp::Ref, inner, _) => match inner.as_ref() {
                        Expr::Call(callee, _, _) => self.adt_ctor_name(callee),
                        other => self.adt_ctor_name(other),
                    },
                    // A call's declared return type names the ADT too.
                    other => self.adt_name_from_call(other),
                };
                if let Some(a) = adt_from_rhs {
                    self.local_adt_type.insert(name.to_string(), a);
                }
                // `b := load Bufio Bufio->PATH;` declares a module handle.
                self.note_module_handle(name, None, Some(rhs));
                self.gen_expr_to(rhs, off)
            }
            Expr::TupleDeclAssign(names, rhs, _) => self.gen_tuple_decl_assign(names, rhs, None),
            Expr::PostInc(inner, _) => self.gen_inc_dec(inner, true, None),
            Expr::PostDec(inner, _) => self.gen_inc_dec(inner, false, None),
            Expr::Call(_, _, _) => self.gen_call_expr(expr),
            Expr::Send(chan_expr, val_expr, _) => self.gen_send(chan_expr, val_expr),
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
                // A tuple local is wider than any single move: copying it
                // with one Mov would take the first field and leave the rest
                // of the destination holding whatever was there before.
                if let Some(fields) = self.local_tuple.get(name).cloned()
                    && let Some((src, _)) = self.get_local(name)
                {
                    self.copy_value(&Type::Tuple(fields), src, dst);
                    return Ok(());
                }
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
                // Module-level or imported constant: materialize its folded
                // value.
                if let Some(value) = self.const_value(name) {
                    let value = value
                        .map_err(|why| format!("constant `{name}` cannot be folded: {why}"))?;
                    return self.gen_const_to(&value, dst);
                }
                // Imported, but not as something that has a value here.
                if let Some(imp) = self.imported.get(name) {
                    return Err(self.imported_value_error(name, imp));
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
                // `load Mod path_expr` — the middle operand names the import
                // block for `Mod`, which is the same block `Mod`'s calls index
                // into. The runtime uses it to map our function indices onto
                // the loaded module's exports.
                let Type::Named(qn) = ty.as_ref() else {
                    return Err("`load` needs a module name".to_string());
                };
                let import_idx = self.ensure_module_import(&qn.name) as i32;
                let path_tmp = self.alloc_temp();
                self.gen_expr_to(path, path_tmp)?;
                self.emit(
                    Opcode::Load,
                    op_fp(path_tmp),
                    mid_imm(import_idx),
                    op_fp(dst),
                );
                Ok(())
            }
            Expr::ModQual(module, member, _) => {
                let Expr::Ident(name, _) = module.as_ref() else {
                    return Err(format!(
                        "`->{member}` must be applied to a module name or module variable"
                    ));
                };
                self.gen_mod_qual_value(name, member, dst)
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
                            // Each conversion reads its operand at that
                            // operand's own width, so pick it from the operand:
                            // a big or a real is eight bytes and has to be
                            // staged in a slot that size, not in the string
                            // destination.
                            match self.infer_num_kind(inner) {
                                NumKind::Big => {
                                    let tmp = self.alloc_temp_for(NumKind::Big);
                                    self.gen_expr_to(inner, tmp)?;
                                    self.emit(Opcode::Cvtlc, op_fp(tmp), mid_unused(), op_fp(dst));
                                }
                                NumKind::Real => {
                                    let tmp = self.alloc_temp_for(NumKind::Real);
                                    self.gen_expr_to(inner, tmp)?;
                                    self.emit(Opcode::Cvtfc, op_fp(tmp), mid_unused(), op_fp(dst));
                                }
                                NumKind::Word => {
                                    self.gen_expr_to(inner, dst)?;
                                    self.emit(
                                        Opcode::Cvtwc,
                                        op_fp(dst),
                                        mid_unused(),
                                        op_fp(dst),
                                    );
                                }
                            }
                        }
                    }
                    _ => {
                        self.gen_expr_to(inner, dst)?;
                    }
                }
                Ok(())
            }
            Expr::ArrayAlloc(size, elem, _) => {
                let sz_tmp = self.alloc_temp();
                self.gen_expr_to(size, sz_tmp)?;
                let at = self.code.len();
                self.emit(Opcode::Newa, op_fp(sz_tmp), mid_imm(0), op_fp(dst));
                // The element descriptor fixes both the element width and
                // which of its words the collector follows. Type 0 — a
                // 16-byte cell with a pointer at offset 0 — described none
                // of them.
                self.need_type(at, TypeOperand::Middle, TypeKey::Elem(ElemKind::of(elem)));
                Ok(())
            }
            Expr::ArrayLit(size, elems, ty, _) => {
                self.gen_array_literal(size.as_deref(), elems, ty.as_deref(), dst)
            }
            Expr::RefAlloc(ty, args, _) => {
                let name = Self::adt_name_for_type(ty).unwrap_or_default();
                self.gen_record_alloc(&name, args, dst)
            }
            Expr::Dot(inner, field, _) => {
                // `t.tN` on a tuple is not a load through a pointer: the
                // tuple is inline in the frame, so the field is read straight
                // out of the block at its layout offset.
                if let Some(n) = tuple_field_index(field)
                    && self.tuple_fields_of(inner).is_some()
                {
                    let (block, layout) = self.gen_tuple_block(inner)?;
                    let Some((ty, off)) = layout.field(n).cloned() else {
                        return Err(format!(
                            "`.{field}` is out of range for a {}-field tuple",
                            layout.fields.len()
                        ));
                    };
                    self.copy_value(&ty, block + off, dst);
                    return Ok(());
                }
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
                //
                // A tuple element has no fixed-width opcode: its size comes
                // from a type descriptor, via Newcm/Newcmp. Allocating one of
                // those channels with Newcw instead fixed elem_size at 4 and
                // the channel delivered only the message's first word.
                if let Type::Tuple(fields) = ty.as_ref() {
                    let layout = compute_tuple_layout(fields);
                    let opcode = if layout.ptr_offsets.is_empty() {
                        Opcode::Newcm
                    } else {
                        Opcode::Newcmp
                    };
                    let at = self.code.len();
                    self.emit(opcode, op_imm(0), mid_unused(), op_fp(dst));
                    self.need_type(
                        at,
                        TypeOperand::Source,
                        TypeKey::Block {
                            size: layout.size,
                            ptr_offsets: layout.ptr_offsets.clone(),
                        },
                    );
                    return Ok(());
                }
                let elem = match ty.as_ref() {
                    Type::Basic(b) => Some(*b),
                    _ => None,
                };
                self.emit(newc_opcode(elem), op_unused(), mid_imm(0), op_fp(dst));
                Ok(())
            }
            Expr::Tuple(elems, _) => self.gen_tuple_literal_to(elems, dst),
            Expr::TupleDeclAssign(names, rhs, _) => {
                self.gen_tuple_decl_assign(names, rhs, Some(dst))
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
                self.gen_send(chan_expr, val_expr)?;
                self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(dst));
                Ok(())
            }
            // A tagged value carries its tag in word 0, so `tagof e` is a word
            // load through the reference. `tagof Adt.Variant` names a variant
            // rather than a value, and folds to that variant's tag
            // (`limbo/ecom.c:459-463`, `limbo/nodes.c:489-497`).
            Expr::Tagof(inner, _) => {
                if let Some(tag) = self.variant_tag(inner) {
                    self.gen_word_const_to(tag, dst);
                    return Ok(());
                }
                let tmp = self.alloc_temp();
                self.gen_expr_to(inner, tmp)?;
                self.emit(Opcode::Movw, op_fp_ind(tmp, 0), mid_unused(), op_fp(dst));
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
            // No lowering for this shape. Storing a zero would let the
            // program run on with a value that never came from the source.
            other => Err(format!(
                "{} is not supported yet",
                describe_expr_kind(other)
            )),
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
                    && let Some(name) = self.adt_ctor_name(callee)
                {
                    self.gen_record_alloc(&name, args, dst)?;
                } else if let Some(name) = self.adt_ctor_name(inner) {
                    // `ref Adt` with no initialiser list: allocate the record
                    // and leave its fields zeroed.
                    self.gen_record_alloc(&name, &[], dst)?;
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
            // handle->func(args) — a cross-module call through any handle.
            if let Expr::ModQual(module, func_name, _) = callee.as_ref() {
                let Expr::Ident(handle, _) = module.as_ref() else {
                    return Err(format!(
                        "`{func_name}` must be called through a module variable"
                    ));
                };
                return self.gen_module_call(handle, func_name, args, None);
            }
            // Local function call: func(args)
            if let Expr::Ident(func_name, _) = callee.as_ref() {
                if let Some(qualified) = self.imported_callee(func_name)? {
                    return self.gen_call_expr(&Expr::Call(
                        Box::new(qualified),
                        args.clone(),
                        Span::default(),
                    ));
                }
                return self.gen_local_call(func_name, args, None);
            }
            // obj.method(args) — an ADT function member.
            if let Expr::Dot(obj, method, _) = callee.as_ref() {
                return self.gen_adt_method_call(obj, method, args, None);
            }
            // Statement position is no excuse for dropping the call: the
            // callee still has effects. Say what could not be lowered.
            return Err(unsupported_call_target(callee));
        }
        Ok(())
    }

    /// Rewrite a bare call to an imported function into its qualified form.
    ///
    /// This is what the reference compiler does: every imported name is
    /// reconstructed as `Omdot(eimport, importid)` before code generation
    /// (ecom.c:184), so `open(...)` and `sys->open(...)` reach codegen as the
    /// same tree and emit the same `MFRAME`/`MCALL` pair. A locally defined
    /// function of the same name wins, matching Limbo's scoping.
    fn imported_callee(&self, name: &str) -> Result<Option<Expr>, String> {
        if self.func_table.iter().any(|(n, _, _, _)| n == name) {
            return Ok(None);
        }
        let Some(imp) = self.imported.get(name) else {
            return Ok(None);
        };
        // With the interface in hand, only rewrite calls to things that really
        // are functions, and refuse the ones that have no module reference to
        // call through. Without it (no include path) the name is still known to
        // come from `imp.module`, and a call site can only be a call, so it goes
        // to the qualified path regardless: `open(...)` must behave exactly as
        // `sys->open(...)` would, including where that form is itself imperfect.
        if let Some(module_type) = &imp.module_type {
            if !matches!(
                self.module_member(module_type, name),
                Some(Symbol::Func { .. })
            ) {
                return Ok(None);
            }
            if !imp.from_variable {
                return Err(format!(
                    "cannot call `{name}` because `{}` is a module interface, not a module \
                     variable",
                    imp.module
                ));
            }
        }
        Ok(Some(Expr::ModQual(
            Box::new(Expr::Ident(imp.module.clone(), Span::default())),
            name.to_string(),
            Span::default(),
        )))
    }

    fn gen_call_with_result(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        dst: i32,
    ) -> Result<(), String> {
        // handle->func(args) — a cross-module call through any handle.
        if let Expr::ModQual(module, func_name, _) = callee {
            let Expr::Ident(handle, _) = module.as_ref() else {
                return Err(format!(
                    "`{func_name}` must be called through a module variable"
                ));
            };
            return self.gen_module_call(handle, func_name, args, Some(dst));
        }
        // obj.method(args) — an ADT function member.
        if let Expr::Dot(obj, method, _) = callee {
            return self.gen_adt_method_call(obj, method, args, Some(dst));
        }
        // Nested calls like sys->fildes(1)
        if let Expr::Call(inner_callee, inner_args, _) = callee {
            return self.gen_call_with_result(inner_callee, inner_args, dst);
        }
        // Local function call
        if let Expr::Ident(func_name, _) = callee {
            if let Some(qualified) = self.imported_callee(func_name)? {
                return self.gen_call_with_result(&qualified, args, dst);
            }
            return self.gen_local_call(func_name, args, Some(dst));
        }
        // A callee shape with no lowering at all: say so rather than storing a
        // zero and pretending the call happened.
        Err(unsupported_call_target(callee))
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

        // A call to a name no declaration provides used to emit nothing at
        // all, so execution simply carried on as though it had happened.
        let Some((func_idx, func_pc, ret_kind)) = func_info else {
            return Err(format!("call to undefined function `{func_name}`"));
        };
        {
            let func_type = 2 + func_idx as i32;

            // Evaluate args first into temps sized by each arg's kind so
            // big/real values aren't truncated.
            let mut arg_temps: Vec<ArgSlot> = Vec::new();
            for arg in args {
                arg_temps.push(self.gen_arg_value(arg)?);
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
            for slot in std::mem::take(&mut arg_temps) {
                arg_off = self.store_arg(&slot, frame_tmp, arg_off);
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

    /// Emit a cross-module call `handle->func_name(args)`.
    ///
    /// The module reference lives in the handle variable's own storage slot —
    /// the one `load` filled in — so any handle works, not just one spelled
    /// `sys`. The reference compiler reaches this same shape for both
    /// spellings: an imported name is rewritten to `handle->name` before code
    /// generation (ecom.c:184), so `open(...)` and `sys->open(...)` produce
    /// identical `MFRAME`/`MCALL` pairs.
    fn gen_module_call(
        &mut self,
        handle: &str,
        func_name: &str,
        args: &[Expr],
        result_dst: Option<i32>,
    ) -> Result<(), String> {
        let (slot, ..) = self
            .lookup_var(handle)
            .ok_or_else(|| self.unresolved_handle_error(handle, func_name))?;
        let interface = self
            .module_handle_type
            .get(handle)
            .cloned()
            .ok_or_else(|| self.not_a_module_handle_error(handle, func_name))?;
        // With the interface in hand, a name it does not declare is an error
        // naming both — the same check `import` makes. Without it (the `.m`
        // was not on the include path) there is nothing to check against.
        if self.interface_is_known(&interface) {
            match self.module_member(&interface, func_name) {
                Some(Symbol::Func { .. }) => {}
                Some(_) => {
                    return Err(format!(
                        "`{func_name}` is not a function of module `{handle}` (interface \
                         `{interface}`)"
                    ));
                }
                None => return Err(self.not_a_member(func_name, handle, &interface)),
            }
        }
        let ret_kind = self.module_call_num_kind(handle, func_name);
        self.emit_module_call(slot, &interface, func_name, args, result_dst, ret_kind)
    }

    /// Emit the `mframe`/`mcall` pair for one cross-module call: the module
    /// reference comes from `slot`, and `entry` names the callee as the
    /// callee module's export table spells it.
    fn emit_module_call(
        &mut self,
        slot: Slot,
        interface: &str,
        entry: &str,
        args: &[Expr],
        result_dst: Option<i32>,
        ret_kind: NumKind,
    ) -> Result<(), String> {
        let func_idx = self.ensure_module_func(interface, entry);
        let module_operand = slot.operand();

        // Phase 1: Evaluate all arguments into kind-sized temps BEFORE
        // allocating the call frame. Nested calls (like sys->fildes(1))
        // complete first; each temp's width matches its arg's kind so big
        // and real values aren't truncated when packed.
        let mut arg_temps: Vec<ArgSlot> = Vec::new();
        for arg in args {
            arg_temps.push(self.gen_arg_value(arg)?);
        }

        // Phase 2: Allocate call frame and fill it
        let frame_tmp = self.alloc_temp();
        // Wide enough for big/real returns; pointer returns share the 4-byte
        // size of Word so ValType::Ptr module functions are unaffected.
        let ret_tmp = self.alloc_temp_for(ret_kind);

        self.emit(
            Opcode::Mframe,
            module_operand,
            mid_imm(func_idx as i32),
            op_fp(frame_tmp),
        );

        // Pack args at cumulative offsets so each occupies the right width
        // (4 bytes per Word/Ptr, 8 bytes per Big/Real). For varargs like
        // `sys->print`, this puts the format-string-driven $Sys formatter's
        // expected layout in place: %d reads 4 bytes, %bd 8, %f 8, etc.
        let mut arg_off = 32i32;
        for slot in std::mem::take(&mut arg_temps) {
            arg_off = self.store_arg(&slot, frame_tmp, arg_off);
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
            module_operand,
        );
        Ok(())
    }

    /// Materialise the value of `name->member`, where `name` is either a
    /// module interface (`Sys->UTFmax`) or a module handle (`sys->UTFmax`).
    ///
    /// Only constants have a value this compiler can produce. Anything else —
    /// another module's variable, a member the interface does not declare, an
    /// unresolvable interface — is reported. Emitting `Movw $0` here is what
    /// turned a missing `PATH` or a mistyped constant into a silent zero.
    fn gen_mod_qual_value(&mut self, name: &str, member: &str, dst: i32) -> Result<(), String> {
        // The interface to look the member up in: `name` itself when it names
        // an interface, or the interface of the handle it names.
        let interface = self
            .module_handle_type
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string());
        match self.qualified_const_value(&interface, member) {
            Some(Ok(value)) => return self.gen_const_to(&value, dst),
            Some(Err(why)) => return Err(format!("`{name}->{member}` is unusable: {why}")),
            None => {}
        }
        // A module's `PATH` names where to load it from. Every Inferno
        // interface declares one, so the fallback only fires when the `.m`
        // could not be read — and `$Name` is the convention for the built-in
        // modules, which are exactly the ones a bare `include` misses.
        if member == "PATH" && !self.interface_is_known(&interface) {
            let path = format!("${interface}");
            let mp = self.intern_string(&path);
            self.emit(Opcode::Movp, op_mp(mp), mid_unused(), op_fp(dst));
            return Ok(());
        }
        Err(match self.module_member(&interface, member) {
            Some(Symbol::Opaque { reason }) => {
                format!("`{name}->{member}` cannot be used here: {reason}")
            }
            Some(Symbol::Type { .. }) => format!("`{name}->{member}` is a type, not a value"),
            Some(Symbol::Func { .. }) => {
                format!("`{name}->{member}` is a function; it can only be called")
            }
            Some(Symbol::Var { .. }) => format!(
                "`{name}->{member}` is a variable of another module; qualified access to another \
                 module's variables is not supported yet"
            ),
            Some(_) => format!("`{name}->{member}` is not a value"),
            None if self.interface_is_known(&interface) => {
                self.not_a_member(member, name, &interface)
            }
            None => format!(
                "`{name}->{member}`: the interface of `{interface}` was not found (is the include \
                 path set?)"
            ),
        })
    }

    /// The ADT an `obj.method(...)` callee refers to, and whether `obj` is a
    /// value to pass as the receiver.
    ///
    /// `p.sum(...)` names the ADT of `p` and passes `p`; `Point.make(...)`
    /// names the ADT directly and passes nothing.
    fn adt_method_target(&self, obj: &Expr, method: &str) -> Option<(String, bool)> {
        if let Expr::Ident(name, _) = obj
            && self.adt_methods.contains_key(name)
            && self
                .adt_methods
                .get(name)
                .is_some_and(|m| m.contains_key(method))
            && self.lookup_var(name).is_none()
        {
            // `Adt.method(...)` — the ADT named directly, no receiver.
            return Some((name.clone(), false));
        }
        let adt = self.adt_name_for_expr(obj)?;
        let sig = self.adt_methods.get(&adt)?.get(method)?;
        let takes_self = sig.params.first().is_some_and(|p| p.is_self);
        Some((adt, takes_self))
    }

    /// Lower `obj.method(args)`.
    ///
    /// A method of an ADT this module declares is compiled here, under the
    /// name `Adt.method`, so the call is an ordinary local one. A method of an
    /// ADT that belongs to another interface is compiled *there*, and its
    /// export is named `Adt.method` too (see any Inferno `.dis`), so the call
    /// is a cross-module call through a handle for that interface.
    fn gen_adt_method_call(
        &mut self,
        obj: &Expr,
        method: &str,
        args: &[Expr],
        result_dst: Option<i32>,
    ) -> Result<(), String> {
        let Some((adt, takes_self)) = self.adt_method_target(obj, method) else {
            return Err(unsupported_call_target(&Expr::Dot(
                Box::new(obj.clone()),
                method.to_string(),
                Span::default(),
            )));
        };
        let entry = format!("{adt}.{method}");
        let mut full_args: Vec<Expr> = Vec::with_capacity(args.len() + 1);
        if takes_self {
            full_args.push(obj.clone());
        }
        full_args.extend_from_slice(args);

        if self.func_table.iter().any(|(n, _, _, _)| n == &entry) {
            return self.gen_local_call(&entry, &full_args, result_dst);
        }
        let Some(owner) = self.adt_owner.get(&adt).cloned() else {
            return Err(format!(
                "`{entry}` is declared but not defined, and `{adt}` belongs to no interface this \
                 file can see"
            ));
        };
        let Some(handle) = self.handle_for_interface(&owner) else {
            return Err(format!(
                "cannot call `{entry}`: no module variable of interface `{owner}` is in scope to \
                 call it through"
            ));
        };
        let Some((slot, ..)) = self.lookup_var(&handle) else {
            return Err(format!("cannot call `{entry}`: `{handle}` has no storage"));
        };
        let ret_kind = self.adt_method_num_kind(&adt, method);
        self.emit_module_call(slot, &owner, &entry, &full_args, result_dst, ret_kind)
    }

    /// A module variable whose interface is `interface`, if one is in scope.
    /// Locals win over globals, and among equals the first declared.
    fn handle_for_interface(&self, interface: &str) -> Option<String> {
        let matches = |name: &String| {
            self.module_handle_type
                .get(name)
                .is_some_and(|i| i == interface)
        };
        if let Some((name, ..)) = self.locals.iter().find(|(n, _, _, _)| matches(n)) {
            return Some(name.clone());
        }
        self.globals
            .iter()
            .find(|(n, _, _, _)| matches(n))
            .map(|(n, _, _, _)| n.clone())
    }

    /// The width of the value `Adt.method` returns.
    fn adt_method_num_kind(&self, adt: &str, method: &str) -> NumKind {
        self.adt_method_sig(adt, method)
            .and_then(|s| s.ret.as_ref().map(type_num_kind))
            .unwrap_or(NumKind::Word)
    }

    fn adt_method_sig(&self, adt: &str, method: &str) -> Option<&FuncSig> {
        self.adt_methods.get(adt)?.get(method)
    }

    /// The ADT a constructor expression names, spelled either bare (`Xfid`)
    /// or through the interface that declares it (`Bufio->Iobuf`).
    fn adt_ctor_name(&self, expr: &Expr) -> Option<String> {
        // `ref Shape.Circle(...)` names one variant of a tagged ADT, which has
        // a layout, a size, and a tag of its own.
        if let Expr::Dot(base, variant, _) = expr
            && let Some(adt) = self.adt_variant_owner(base)
        {
            let key = format!("{adt}.{variant}");
            return self.adt_variants.contains_key(&key).then_some(key);
        }
        let name = match expr {
            Expr::Ident(name, _) => name,
            Expr::ModQual(_, name, _) => name,
            _ => return None,
        };
        self.adt_layouts.contains_key(name).then(|| name.clone())
    }

    /// The tag of the variant `expr` names, for the constant-folded
    /// `tagof Adt.Variant` form.
    fn variant_tag(&self, expr: &Expr) -> Option<i32> {
        let Expr::Dot(base, variant, _) = expr else {
            return None;
        };
        let adt = self.adt_variant_owner(base)?;
        self.adt_variants
            .get(&format!("{adt}.{variant}"))
            .map(|(tag, _)| *tag)
    }

    /// The ADT `expr` names when it is used as the left half of a variant
    /// name. A local variable of the same name wins: `x.f` selects a field of
    /// `x`, whatever ADTs are in scope.
    fn adt_variant_owner(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident(name, _) if self.lookup_var(name).is_none() => Some(name.clone()),
            Expr::ModQual(_, name, _) => Some(name.clone()),
            _ => None,
        }
    }

    /// The element kind of an array literal: its declared element type when
    /// it has one, otherwise what its elements are. The kind fixes both the
    /// allocation's element width and which of its words the collector
    /// follows, so the value has to be the same one the reader of `a[i]` uses.
    fn array_lit_elem_kind(&self, elems: &[ArrayElem], elem_ty: Option<&Type>) -> ElemKind {
        match elem_ty {
            Some(t) => ElemKind::of(t),
            None if !elems.is_empty() && elems.iter().all(|e| is_byte_cast(&e.value)) => {
                ElemKind::Byte
            }
            None => match elems.first().map(|e| self.infer_expr_type(&e.value)) {
                Some(ValType::Word) | None => {
                    match elems.first().map(|e| self.infer_num_kind(&e.value)) {
                        Some(NumKind::Big) => ElemKind::Big,
                        Some(NumKind::Real) => ElemKind::Real,
                        _ => ElemKind::Word,
                    }
                }
                Some(_) => ElemKind::Ptr,
            },
        }
    }

    /// `array[n] of {..}` where the value is wanted at run time rather than
    /// folded into the data section.
    ///
    /// Each element is written at the index it names, so a keyed or ranged
    /// selector places its value where the source says rather than in
    /// declaration order.
    fn gen_array_literal(
        &mut self,
        size: Option<&Expr>,
        elems: &[ArrayElem],
        elem_ty: Option<&Type>,
        dst: i32,
    ) -> Result<(), String> {
        // Where each element goes. A positional element takes the next free
        // slot; a keyed or ranged one the slots it names.
        let mut placed: Vec<(i64, &Expr)> = Vec::new();
        let mut default: Option<&Expr> = None;
        let mut next = 0i64;
        for e in elems {
            match &e.index {
                None => {
                    placed.push((next, &e.value));
                    next += 1;
                }
                Some(ArrayIndex::Selectors(sels)) => {
                    for (lo, hi) in sels {
                        let Ok(ConstVal::Int(lo)) = self.fold_const(lo, 0) else {
                            return Err(
                                "an array-literal index must be a constant here".to_string()
                            );
                        };
                        let hi = match hi {
                            None => lo,
                            Some(hi) => match self.fold_const(hi, 0) {
                                Ok(ConstVal::Int(hi)) => hi,
                                _ => {
                                    return Err("an array-literal index must be a constant here"
                                        .to_string());
                                }
                            },
                        };
                        for i in lo..=hi {
                            placed.push((i, &e.value));
                        }
                        next = hi + 1;
                    }
                }
                Some(ArrayIndex::Wildcard) => default = Some(&e.value),
            }
        }
        let kind = self.array_lit_elem_kind(elems, elem_ty);
        let len_tmp = self.alloc_temp();
        match size {
            Some(e) => self.gen_expr_to(e, len_tmp)?,
            None => {
                let len = placed.iter().map(|(i, _)| *i + 1).max().unwrap_or(0);
                let len =
                    i32::try_from(len).map_err(|_| "array size is out of range".to_string())?;
                self.gen_word_const_to(len, len_tmp);
            }
        }
        let at = self.code.len();
        self.emit(Opcode::Newa, op_fp(len_tmp), mid_imm(0), op_fp(dst));
        self.need_type(at, TypeOperand::Middle, TypeKey::Elem(kind));

        let val_kind = match kind {
            ElemKind::Big => NumKind::Big,
            ElemKind::Real => NumKind::Real,
            _ => NumKind::Word,
        };
        let (ind_op, mut mov_op) = Self::array_elem_opcodes(Some(kind.basic()));
        if kind.is_ptr() {
            // Pointer elements need the ref-counting move, not a raw word copy.
            mov_op = Opcode::Movp;
        }
        // `* => v` fills every slot; the named elements are then written over
        // it. The length may only be known at run time, so this is a loop
        // rather than an unrolled sequence.
        if let Some(value) = default {
            let val_tmp = self.alloc_temp_for(val_kind);
            self.gen_expr_to_kind(value, val_tmp, val_kind)?;
            let i_tmp = self.alloc_temp();
            self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(i_tmp));
            let loop_start = self.code.len() as i32;
            let exit = self.code.len();
            self.emit(Opcode::Bgew, op_fp(i_tmp), mid_fp(len_tmp), op_imm(0));
            let ref_tmp = self.alloc_temp();
            self.emit(ind_op, op_fp(dst), mid_fp(ref_tmp), op_fp(i_tmp));
            self.emit(mov_op, op_fp(val_tmp), mid_unused(), op_fp_ind(ref_tmp, 0));
            self.emit(Opcode::Addw, op_imm(1), mid_unused(), op_fp(i_tmp));
            self.emit(Opcode::Jmp, op_unused(), mid_unused(), op_imm(loop_start));
            self.code[exit].destination = op_imm(self.code.len() as i32);
        }
        for (index, value) in placed {
            let index =
                i32::try_from(index).map_err(|_| "array index is out of range".to_string())?;
            let val_tmp = self.alloc_temp_for(val_kind);
            self.gen_expr_to_kind(value, val_tmp, val_kind)?;
            let idx_tmp = self.alloc_temp();
            self.gen_word_const_to(index, idx_tmp);
            let ref_tmp = self.alloc_temp();
            self.emit(ind_op, op_fp(dst), mid_fp(ref_tmp), op_fp(idx_tmp));
            self.emit(mov_op, op_fp(val_tmp), mid_unused(), op_fp_ind(ref_tmp, 0));
        }
        Ok(())
    }

    /// `ref Adt(a, b, ...)` — allocate the record and fill its fields at the
    /// ADT's own layout offsets with kind-aware moves, so big/real fields land
    /// in 8-byte slots and pointer fields get the ref-counting move.
    ///
    /// An unknown layout falls back to the historical `i*4` / `Movw` packing.
    fn gen_record_alloc(&mut self, adt: &str, args: &[Expr], dst: i32) -> Result<(), String> {
        let key = if self.adt_shapes.contains_key(adt) {
            TypeKey::Adt(adt.to_string())
        } else {
            TypeKey::OpaqueRecord
        };
        let at = self.code.len();
        self.emit(Opcode::New, op_imm(0), mid_unused(), op_fp(dst));
        self.need_type(at, TypeOperand::Source, key);
        // A variant of a tagged ADT carries its tag in word 0: that word is
        // what `tagof` reads and what every `pick` dispatches on.
        if let Some((tag, _)) = self.adt_variants.get(adt).copied() {
            self.emit(Opcode::Movw, op_imm(tag), mid_unused(), op_fp_ind(dst, 0));
        }
        let layout = self.adt_layouts.get(adt).cloned();
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

    /// Was the interface behind a handle actually found and parsed?
    fn interface_is_known(&self, interface: &str) -> bool {
        self.symtab
            .as_ref()
            .is_some_and(|st| st.modules.contains_key(interface))
    }

    /// `h->f()` where `h` names no variable at all.
    fn unresolved_handle_error(&self, handle: &str, member: &str) -> String {
        if self.is_known_interface(handle) {
            format!(
                "cannot call `{member}` because `{handle}` is a module interface, not a module \
                 variable"
            )
        } else {
            format!("undefined identifier `{handle}` in `{handle}->{member}`")
        }
    }

    /// `h->f()` where `h` is a variable, but not one of module type.
    fn not_a_module_handle_error(&self, handle: &str, member: &str) -> String {
        format!(
            "cannot call `{member}` through `{handle}`: `{handle}` is not declared with a module \
             type"
        )
    }

    /// Is `name` an interface this file can see by name?
    fn is_known_interface(&self, name: &str) -> bool {
        self.module_decls.contains(name) || self.interface_is_known(name)
    }

    /// The width of the value `handle->func` returns.
    fn module_call_num_kind(&self, handle: &str, func: &str) -> NumKind {
        match self.handle_member(handle, func) {
            Some(Symbol::Func { ty }) => match ty.ret.as_deref() {
                Some(r) => resolved_num_kind(r),
                None => NumKind::Word,
            },
            // No interface to consult: fall back to the built-in knowledge of
            // `$Sys`, which is the module every program reaches for.
            _ => sys_return_kind(func),
        }
    }

    /// The storage shape of the value `handle->func` returns.
    fn module_call_val_type(&self, handle: &str, func: &str) -> ValType {
        match self.handle_member(handle, func) {
            Some(Symbol::Func { ty }) => match ty.ret.as_deref() {
                Some(r) => resolved_val_type(r),
                None => ValType::Word,
            },
            _ => sys_return_val_type(func),
        }
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

    /// Compile through the full driver so `include`/`module` declarations
    /// populate the symbol table that `import` resolves against.
    fn compile_resolved(src: &str) -> Result<Module, String> {
        crate::compile(src, "test.b")
    }

    /// `alt` over an array of channels has no lowering, and the diagnostic has
    /// to say which construct is missing: the guard reads like an ordinary
    /// tuple receive, and calling it one would send the reader looking for a
    /// tuple problem that is not there.
    #[test]
    fn alt_over_an_array_of_channels_names_the_missing_construct() {
        let err = compile_src(
            r#"
implement Test;
init(nil: ref Draw->Context, nil: list of string)
{
    a := array[2] of chan of string;
    alt {
    (i, s) := <-a =>
        return;
    }
}
"#,
        )
        .expect_err("an alt over an array of channels must not compile to nothing");
        assert!(
            err.contains("array of channels"),
            "diagnostic should name the construct: {err}"
        );
    }

    /// A `pick` over an ADT whose declaration never arrived (its interface was
    /// not on the include path) has no tags to resolve against. Reporting that
    /// the tag is "not a variant" would send the reader looking at the wrong
    /// file.
    #[test]
    fn pick_over_an_undeclared_adt_reports_the_missing_declaration() {
        let err = compile_src(
            r#"
implement Test;
init(nil: ref Draw->Context, nil: list of string)
{
}
handle(e: ref Event)
{
    pick x := e {
    Emouse =>
        return;
    }
}
"#,
        )
        .expect_err("a pick over an unknown ADT must not compile to nothing");
        assert!(
            err.contains("declaration of `Event` was not found"),
            "diagnostic should name the missing declaration: {err}"
        );
    }

    /// The byte offsets of a tagged ADT are an ABI, not an internal choice: a
    /// record built here is read by reference-compiled modules and vice versa.
    /// A run-time test cannot see a layout that is merely shifted, because our
    /// own writer and reader would shift together, so the offsets are pinned
    /// here: tag at 0, the ADT's own fields from 4, and the variant's fields
    /// where the common ones end (`limbo/types.c:2176-2199`).
    #[test]
    fn tagged_adt_places_the_tag_then_common_then_variant_fields() {
        let module = compile_src(
            r#"
implement Test;
Shape: adt {
    name: string;
    pick {
    Circle =>
        r: int;
    Square =>
        s: int;
    }
};
init(nil: ref Draw->Context, nil: list of string)
{
    c := ref Shape.Circle("circ", 5);
}
"#,
        )
        .expect("a tagged ADT constructor should compile");
        // Every write through the new record, as (field offset, mode).
        let writes: Vec<i32> = module
            .code
            .iter()
            .filter(|i| i.destination.mode == AddressMode::OffsetDoubleIndirectFp)
            .map(|i| i.destination.register2)
            .collect();
        assert_eq!(
            writes,
            vec![0, 4, 8],
            "tag at 0, `name` at 4, `r` at 8; got {writes:?}"
        );
        // The descriptor has to cover the variant, and to name `name` as the
        // one word the collector follows.
        let circle = module
            .types
            .iter()
            .find(|t| t.size == 12)
            .expect("a 12-byte descriptor for Shape.Circle");
        assert_eq!(circle.pointer_count, 1, "only `name` is a pointer");
        assert_eq!(
            circle.pointer_map.bytes[0], 0b0100_0000,
            "the pointer is word 1, the `name` field"
        );
    }

    // ── import ──────────────────────────────────────────────────

    /// An unqualified call to an imported function must emit the *same*
    /// cross-module call as its qualified spelling. Comparing the whole
    /// instruction stream is the point: anything that fell back to a local
    /// `Call`, or to the silent `Movw $0`, would differ here.
    #[test]
    fn imported_function_call_emits_the_same_code_as_the_qualified_call() {
        let prelude = r#"implement Test;
Sys: module {
    PATH: con "$Sys";
    print: fn(s: string): int;
};
sys: Sys;
"#;
        let qualified = compile_resolved(&format!(
            r#"{prelude}init(nil: ref Draw->Context, nil: list of string)
{{
    sys = load Sys Sys->PATH;
    sys->print("hi\n");
}}
"#
        ))
        .expect("qualified call should compile");
        let imported = compile_resolved(&format!(
            r#"{prelude}print: import sys;
init(nil: ref Draw->Context, nil: list of string)
{{
    sys = load Sys Sys->PATH;
    print("hi\n");
}}
"#
        ))
        .expect("imported call should compile");

        let render = |m: &Module| {
            m.code
                .iter()
                .map(|i| {
                    format!(
                        "{:?} {:?} {:?} {:?}",
                        i.opcode, i.source, i.middle, i.destination
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(render(&imported), render(&qualified));
        assert!(
            imported.code.iter().any(|i| i.opcode == Opcode::Mcall),
            "the imported call must be a cross-module Mcall"
        );
    }

    /// Calling a function imported from a module *interface* name has no
    /// module reference to call through, so it is an error — the same one the
    /// reference compiler reports (typecheck.c:1459).
    #[test]
    fn calling_a_function_imported_from_an_interface_name_is_an_error() {
        let err = compile_resolved(
            r#"implement Test;
Sys: module {
    print: fn(s: string): int;
};
print: import Sys;
init(nil: ref Draw->Context, nil: list of string)
{
    print("hi\n");
}
"#,
        )
        .expect_err("calling through an interface name must fail");
        assert!(err.contains("module interface"), "unexpected error: {err}");
    }

    /// Using an imported ADT name where a value is expected must say so
    /// instead of reporting the name as undefined.
    #[test]
    fn imported_type_used_as_a_value_is_a_typed_error() {
        let err = compile_resolved(
            r#"implement Test;
Bufio: module {
    Iobuf: adt { x: int; };
};
b: Bufio;
Iobuf: import b;
init(nil: ref Draw->Context, nil: list of string)
{
    x := Iobuf + 1;
}
"#,
        )
        .expect_err("a type is not a value");
        assert!(err.contains("is a type, not a value"), "got: {err}");
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

    #[test]
    fn module_without_init_rejects_non_constant_global_initialiser() {
        // A library module: exported helpers, no `init`. The initialiser for
        // `table` cannot be folded, so it needs generated code to run — and
        // there is no entry function to put that code in. Silently leaving
        // `table` zero at run time is the one outcome that is not allowed.
        // (An `array[n] of T` initialiser *is* constant and goes in the data
        // section, so it deliberately is not the example here.)
        let src = r#"
implement Test;
table := helper();
helper(): int
{
    return 1;
}
"#;
        let err = compile_src(src)
            .expect_err("a global initialiser with nowhere to run must be a compile error");
        assert!(
            err.contains("table"),
            "the error must name the dropped initialiser, got: {err}"
        );
    }

    #[test]
    fn module_with_init_still_runs_non_constant_global_initialisers() {
        // The counterpart to the check above: when there *is* an entry
        // function, the deferred initialiser is emitted into it.
        let src = r#"
implement Test;
table := helper();
helper(): int
{
    return 1;
}
init(nil: ref Draw->Context, nil: list of string)
{
    x := 1;
}
"#;
        let module = compile_src(src).expect("module with init should compile");
        let stores_to_mp = module
            .code
            .iter()
            .any(|i| i.destination.mode == AddressMode::OffsetIndirectMp);
        assert!(
            stores_to_mp,
            "the deferred initialiser for `table` must be emitted into `init`"
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
