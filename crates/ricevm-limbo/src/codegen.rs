//! Code generator: translates Limbo AST to Dis bytecode.
//!
//! Produces a `ricevm_core::Module` from a parsed AST.

use ricevm_core::{
    AddressMode, DataItem, ExportEntry, Header, ImportEntry, ImportModule, Instruction, MiddleMode,
    MiddleOperand, Module, Opcode, Operand, PointerMap, RuntimeFlags, TypeDescriptor, XMAGIC,
};

use crate::ast::*;

/// Value type tracking for selecting correct Dis opcodes.
#[derive(Clone, Copy, PartialEq)]
enum ValType {
    Word,
    Ptr,   // string, list, ref, module, channel
    Array, // array types (use Lena instead of Lenc)
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
fn sys_return_kind(name: &str) -> NumKind {
    match name {
        // big-returning sys builtins (per sys.m signatures)
        "seek" => NumKind::Big,
        _ => NumKind::Word,
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
    /// Frame sizes for each compiled function, in order.
    func_frames: Vec<i32>,
    /// Exception handlers for the module.
    handlers: Vec<ricevm_core::Handler>,
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
            handlers: Vec::new(),
        }
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
        self.imports.push(ImportModule { functions: vec![] });

        // Pre-scan to count functions and allocate type indices
        let funcs: Vec<&FuncDecl> = file
            .decls
            .iter()
            .filter_map(|d| if let Decl::Func(f) = d { Some(f) } else { None })
            .collect();

        // Generate code for each function
        for func in &funcs {
            self.gen_func(func)?;
        }

        self.build_types();
        let entry_type = (self.types.len() as i32 - 1).max(0);
        let entry_pc = self.exports.first().map(|e| e.pc).unwrap_or(0);

        Ok(Module {
            header: Header {
                magic: XMAGIC,
                signature: vec![],
                runtime_flags: RuntimeFlags(if self.handlers.is_empty() { 0x40 } else { 0x60 }),
                stack_extent: (self.frame_size + 256).max(480),
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

        // Register parameter names at fixed offsets
        let mut param_off = 32;
        for param in &func.sig.params {
            for name in &param.names {
                let ty = self.infer_param_type(param);
                let kind = type_num_kind(&param.ty);
                if name != "nil" {
                    self.locals.push((name.clone(), param_off, ty, kind));
                }
                // Frame param slots are 4-byte aligned in the reference ABI;
                // big/real params occupy two adjacent slots. This keeps the
                // offsets consistent with how the caller packs arguments.
                param_off += kind.byte_size();
            }
        }
        self.next_local = param_off.max(40);

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

        // Record function info
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
        self.func_table
            .push((full_name, entry_pc as i32, self.frame_size, ret_kind));
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
            _ => ValType::Ptr,
        }
    }

    // ── Statements ────────────────────────────────────────────

    fn gen_stmt(&mut self, stmt: &Stmt) -> Result<(), String> {
        match stmt {
            Stmt::Expr(e) => self.gen_expr_discard(e),
            Stmt::VarDecl(v) => {
                let ty = self.infer_decl_type(v);
                let kind = self.decl_num_kind(v);
                for name in &v.names {
                    self.alloc_local(name, ty, kind);
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
                    // Write return value through the return pointer at 16(fp).
                    // Numeric returns use the kind-matched opcode (Movw/Movl/
                    // Movf); pointer-typed returns use Movp.
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
            Stmt::Label(_, inner) => self.gen_stmt(inner),
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
                        self.emit(Opcode::Spawn, op_fp(frame_tmp), mid_unused(), op_imm(pc));
                        return Ok(());
                    }
                }
                // Fallback: just evaluate
                self.gen_expr_discard(e)?;
                Ok(())
            }
            _ => Ok(()),
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
            Expr::Ident(name, _) => self.local_num_kind(name),
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
            Expr::Ident(name, _) => self
                .get_local(name)
                .map(|(_, t)| t)
                .unwrap_or(ValType::Word),
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
        let loop_start = self.code.len() as i32;
        let cond_tmp = self.alloc_temp();
        self.gen_cond_to(&s.cond, cond_tmp)?;
        let jump_idx = self.code.len();
        self.emit(Opcode::Beqw, op_fp(cond_tmp), mid_imm(0), op_imm(0));
        self.gen_stmt(&s.body)?;
        self.emit(Opcode::Jmp, op_unused(), mid_unused(), op_imm(loop_start));
        self.code[jump_idx].destination = op_imm(self.code.len() as i32);
        Ok(())
    }

    fn gen_for(&mut self, s: &ForStmt) -> Result<(), String> {
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
        if let Some(post) = &s.post {
            self.gen_stmt(post)?;
        }
        self.emit(Opcode::Jmp, op_unused(), mid_unused(), op_imm(loop_start));
        if let Some(idx) = jump_idx {
            self.code[idx].destination = op_imm(self.code.len() as i32);
        }
        Ok(())
    }

    fn gen_do(&mut self, s: &DoStmt) -> Result<(), String> {
        let loop_start = self.code.len() as i32;
        self.gen_stmt(&s.body)?;
        let cond_tmp = self.alloc_temp();
        self.gen_cond_to(&s.cond, cond_tmp)?;
        // Branch back to start if condition is true (nonzero)
        self.emit(
            Opcode::Bnew,
            op_fp(cond_tmp),
            mid_imm(0),
            op_imm(loop_start),
        );
        Ok(())
    }

    fn gen_case(&mut self, s: &CaseStmt) -> Result<(), String> {
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
                        // val >= lo && val <= hi
                        let lo_tmp = self.alloc_temp();
                        let hi_tmp = self.alloc_temp();
                        self.gen_expr_to(lo, lo_tmp)?;
                        self.gen_expr_to(hi, hi_tmp)?;
                        let idx = self.code.len();
                        // Use Bgew val, lo and Blew val, hi
                        self.emit(Opcode::Bltw, op_fp(val_tmp), mid_fp(lo_tmp), op_imm(0)); // skip if val < lo
                        let skip1 = self.code.len() - 1;
                        let idx2 = self.code.len();
                        self.emit(Opcode::Blew, op_fp(val_tmp), mid_fp(hi_tmp), op_imm(0)); // match if val <= hi
                        arm_jumps.push(idx2);
                        // Patch skip1 to skip this arm
                        let _ = (idx, skip1); // skip1 will be patched after arm body
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
                    let ty = self.infer_expr_type(rhs);
                    let off = self
                        .get_local(name)
                        .map(|(o, _)| o)
                        .unwrap_or_else(|| self.alloc_local(name, ty, NumKind::Word));
                    self.gen_expr_to(rhs, off)
                } else if let Expr::Index(arr_expr, idx_expr, _) = lhs.as_ref() {
                    // s[i] = val → Insc val, idx, str
                    let val_tmp = self.alloc_temp();
                    let idx_tmp = self.alloc_temp();
                    let arr_tmp = self.alloc_temp();
                    self.gen_expr_to(rhs, val_tmp)?;
                    self.gen_expr_to(idx_expr, idx_tmp)?;
                    self.gen_expr_to(arr_expr, arr_tmp)?;
                    // Insc src, mid, dst: dst[mid] = src
                    self.emit(
                        Opcode::Insc,
                        op_fp(val_tmp),
                        mid_fp(idx_tmp),
                        op_fp(arr_tmp),
                    );
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
                    // p.field = val → write through ref pointer
                    let ref_tmp = self.alloc_temp();
                    let val_tmp = self.alloc_temp();
                    self.gen_expr_to(inner_expr, ref_tmp)?;
                    self.gen_expr_to(rhs, val_tmp)?;
                    let field_off = self.estimate_field_offset(inner_expr, field);
                    let ty = self.infer_expr_type(rhs);
                    let op = if ty != ValType::Word {
                        Opcode::Movp
                    } else {
                        Opcode::Movw
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
                    let off = self
                        .get_local(name)
                        .map(|(o, _)| o)
                        .unwrap_or_else(|| self.alloc_local(name, ValType::Word, NumKind::Word));
                    let (_, vt) = self.get_local(name).unwrap_or((off, ValType::Word));
                    // String +=: use Addc with the same operand layout.
                    if vt == ValType::Ptr && *op == BinOp::Add {
                        let rhs_tmp = self.alloc_temp();
                        self.gen_expr_to(rhs, rhs_tmp)?;
                        self.emit(Opcode::Addc, op_fp(rhs_tmp), mid_unused(), op_fp(off));
                        return Ok(());
                    }
                    // Numeric compound assign: dispatch by the lvalue's kind
                    // so big/real `x op= y` uses the wide opcode family.
                    let kind = self.local_num_kind(name);
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
                    self.emit(opcode, op_fp(rhs_tmp), mid_unused(), op_fp(off));
                    Ok(())
                } else {
                    Ok(())
                }
            }
            Expr::DeclAssign(names, rhs, _) => {
                let ty = self.infer_expr_type(rhs);
                let name = names.first().map(|s| s.as_str()).unwrap_or("_");
                let off = self.alloc_local(name, ty, NumKind::Word);
                self.gen_expr_to(rhs, off)
            }
            Expr::TupleDeclAssign(names, rhs, _) => {
                // Generate call, then extract tuple fields
                let ret_tmp = self.alloc_temp();
                self.gen_expr_to(rhs, ret_tmp)?;
                for (i, name) in names.iter().enumerate() {
                    if name != "nil" {
                        let off = self.alloc_local(name, ValType::Word, NumKind::Word);
                        // Tuple fields at ret_tmp + i*4
                        let field_off = ret_tmp + (i as i32) * 4;
                        if field_off != off {
                            self.emit(Opcode::Movw, op_fp(field_off), mid_unused(), op_fp(off));
                        }
                    }
                }
                Ok(())
            }
            Expr::PostInc(inner, _) => {
                if let Expr::Ident(name, _) = inner.as_ref()
                    && let Some((off, _)) = self.get_local(name)
                {
                    self.emit(Opcode::Addw, op_imm(1), mid_unused(), op_fp(off));
                }
                Ok(())
            }
            Expr::PostDec(inner, _) => {
                if let Expr::Ident(name, _) = inner.as_ref()
                    && let Some((off, _)) = self.get_local(name)
                {
                    self.emit(Opcode::Subw, op_imm(1), mid_unused(), op_fp(off));
                }
                Ok(())
            }
            Expr::Call(_, _, _) => self.gen_call_expr(expr),
            Expr::Send(chan_expr, val_expr, _) => {
                let chan_tmp = self.alloc_temp();
                let val_tmp = self.alloc_temp();
                self.gen_expr_to(chan_expr, chan_tmp)?;
                self.gen_expr_to(val_expr, val_tmp)?;
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
                    self.emit(Opcode::Movw, op_imm(*v as i32), mid_unused(), op_fp(dst));
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
                if let Some((off, ty)) = self.get_local(name) {
                    if off != dst {
                        let kind = self.local_num_kind(name);
                        let op = match (ty, kind) {
                            // Big/real locals carry an 8-byte payload: use the
                            // matching wide move regardless of the surrounding
                            // ValType (which is Word for both).
                            (_, NumKind::Big) => Opcode::Movl,
                            (_, NumKind::Real) => Opcode::Movf,
                            (ValType::Word, NumKind::Word) => Opcode::Movw,
                            // Strings, lists, refs, channels, modules: 4-byte
                            // pointer move with ref-counting in the VM.
                            _ => Opcode::Movp,
                        };
                        self.emit(op, op_fp(off), mid_unused(), op_fp(dst));
                    }
                } else {
                    self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(dst));
                }
                Ok(())
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
                let arr_tmp = self.alloc_temp();
                let idx_tmp = self.alloc_temp();
                self.gen_expr_to(arr, arr_tmp)?;
                self.gen_expr_to(idx, idx_tmp)?;
                let arr_ty = self.infer_expr_type(arr);
                if arr_ty == ValType::Ptr {
                    // String indexing: Indc src, mid, dst → dst = src[mid]
                    self.emit(Opcode::Indc, op_fp(arr_tmp), mid_fp(idx_tmp), op_fp(dst));
                } else {
                    // Array indexing: Indw
                    self.emit(Opcode::Indw, op_fp(arr_tmp), mid_unused(), op_fp(dst));
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
                self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(dst));
                Ok(())
            }
            Expr::Call(callee, args, _) => self.gen_call_with_result(callee, args, dst),
            Expr::DeclAssign(names, rhs, _) => {
                let ty = self.infer_expr_type(rhs);
                let name = names.first().map(|s| s.as_str()).unwrap_or("_");
                let off = self.alloc_local(name, ty, NumKind::Word);
                self.gen_expr_to(rhs, off)?;
                if off != dst {
                    let op = if ty != ValType::Word {
                        Opcode::Movp
                    } else {
                        Opcode::Movw
                    };
                    self.emit(op, op_fp(off), mid_unused(), op_fp(dst));
                }
                Ok(())
            }
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
            Expr::RefAlloc(_, args, _) => {
                // ref Adt(field1, field2, ...) → New $type, dst; then fill fields
                // Allocate a record with size = args.len() * 4
                let record_size = (args.len() as i32) * 4;
                // We need a type descriptor for this record. Use a simple one.
                // For now, use New with an immediate size (simplified)
                self.emit(Opcode::New, op_imm(1), mid_unused(), op_fp(dst));
                // Fill fields at offsets 0, 4, 8, ...
                for (i, arg) in args.iter().enumerate() {
                    let field_off = (i as i32) * 4;
                    let arg_tmp = self.alloc_temp();
                    self.gen_expr_to(arg, arg_tmp)?;
                    let ty = self.infer_expr_type(arg);
                    let op = if ty != ValType::Word {
                        Opcode::Movp
                    } else {
                        Opcode::Movw
                    };
                    self.emit(op, op_fp(arg_tmp), mid_unused(), op_fp_ind(dst, field_off));
                }
                let _ = record_size;
                Ok(())
            }
            Expr::Dot(inner, field, _) => {
                // expr.field → read field at offset from the ref pointer
                // Simplified: field offset = field_index * 4
                let ref_tmp = self.alloc_temp();
                self.gen_expr_to(inner, ref_tmp)?;
                let field_off = self.estimate_field_offset(inner, field);
                // Double-indirect: read offset(ref(fp))
                self.emit(
                    Opcode::Movw,
                    op_fp_ind(ref_tmp, field_off),
                    mid_unused(),
                    op_fp(dst),
                );
                Ok(())
            }
            Expr::ChanAlloc(_, _) => {
                // chan of type → Newcw $0, dst
                self.emit(Opcode::Newcw, op_unused(), mid_imm(0), op_fp(dst));
                Ok(())
            }
            Expr::Recv(chan_expr, _) => {
                // <-chan → Recv chan, dst
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
                self.gen_expr_to(rhs, dst)?;
                if let Expr::Ident(name, _) = lhs.as_ref() {
                    let ty = self.infer_expr_type(rhs);
                    let off = self
                        .get_local(name)
                        .map(|(o, _)| o)
                        .unwrap_or_else(|| self.alloc_local(name, ty, NumKind::Word));
                    if off != dst {
                        let op = if ty != ValType::Word {
                            Opcode::Movp
                        } else {
                            Opcode::Movw
                        };
                        self.emit(op, op_fp(dst), mid_unused(), op_fp(off));
                    }
                }
                Ok(())
            }
            _ => {
                self.emit(Opcode::Movw, op_imm(0), mid_unused(), op_fp(dst));
                Ok(())
            }
        }
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
                self.gen_expr_to(inner, dst)?;
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

            self.emit(
                Opcode::Lea,
                op_fp(ret_tmp),
                mid_unused(),
                op_fp_ind(frame_tmp, 16),
            );
            self.emit(
                Opcode::Call,
                op_fp(frame_tmp),
                mid_unused(),
                op_imm(func_pc),
            );

            if let Some(dst) = result_dst {
                let op = match ret_kind {
                    NumKind::Word => Opcode::Movw,
                    NumKind::Big => Opcode::Movl,
                    NumKind::Real => Opcode::Movf,
                };
                self.emit(op, op_fp(ret_tmp), mid_unused(), op_fp(dst));
            }
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

        self.emit(
            Opcode::Lea,
            op_fp(ret_tmp),
            mid_unused(),
            op_fp_ind(frame_tmp, 16),
        );
        self.emit(
            Opcode::Mcall,
            op_fp(frame_tmp),
            mid_imm(func_idx as i32),
            op_mp(self.sys_mp_ref),
        );

        if let Some(dst) = result_dst {
            let ret_is_ptr = matches!(
                func_name,
                "fildes" | "open" | "create" | "fstat" | "stat" | "sprint" | "aprint"
            );
            let op = if ret_is_ptr {
                Opcode::Movp
            } else {
                match sys_return_kind(func_name) {
                    NumKind::Big => Opcode::Movl,
                    NumKind::Real => Opcode::Movf,
                    NumKind::Word => Opcode::Movw,
                }
            };
            self.emit(op, op_fp(ret_tmp), mid_unused(), op_fp(dst));
        }
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
