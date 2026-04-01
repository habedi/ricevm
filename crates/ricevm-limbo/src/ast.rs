//! Abstract syntax tree for the Limbo language.

use crate::token::Span;

/// A complete Limbo source file.
#[derive(Debug, Clone)]
pub struct SourceFile {
    pub implement: Vec<String>,
    pub includes: Vec<Include>,
    pub decls: Vec<Decl>,
}

#[derive(Debug, Clone)]
pub struct Include {
    pub path: String,
    pub span: Span,
}

/// Top-level declaration.
#[derive(Debug, Clone)]
pub enum Decl {
    /// `name: Sys;` or `name: type = expr;`
    Var(VarDecl),
    /// `name: con value;`
    Const(ConstDecl),
    /// `name: type data-type;`
    TypeAlias(TypeAliasDecl),
    /// `name: module { ... };`
    Module(ModuleDecl),
    /// `name: adt { ... };`
    Adt(AdtDecl),
    /// `name(params): rettype { body }`
    Func(FuncDecl),
    /// `name: import modname;`
    Import(ImportDecl),
    /// `name: exception (types);`
    Exception(ExceptionDecl),
}

#[derive(Debug, Clone)]
pub struct VarDecl {
    pub names: Vec<String>,
    pub ty: Option<Type>,
    pub init: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ConstDecl {
    pub name: String,
    pub ty: Option<Type>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypeAliasDecl {
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ModuleDecl {
    pub name: String,
    pub members: Vec<ModuleMember>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ModuleMember {
    Const(ConstDecl),
    TypeAlias(TypeAliasDecl),
    Var(VarDecl),
    Func(FuncSig),
    Adt(AdtDecl),
    Exception(ExceptionDecl),
}

#[derive(Debug, Clone)]
pub struct AdtDecl {
    pub name: String,
    pub members: Vec<AdtMember>,
    pub pick: Option<Vec<PickCase>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum AdtMember {
    Field(VarDecl),
    Const(ConstDecl),
    Func(FuncSig),
}

#[derive(Debug, Clone)]
pub struct PickCase {
    pub tags: Vec<String>,
    pub fields: Vec<VarDecl>,
}

#[derive(Debug, Clone)]
pub struct FuncDecl {
    pub name: QualName,
    pub sig: FuncSig,
    pub body: Block,
    pub span: Span,
}

/// Qualified name: `Module.func` or just `func`.
#[derive(Debug, Clone)]
pub struct QualName {
    pub qualifier: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct FuncSig {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub names: Vec<String>,
    pub ty: Type,
    pub is_self: bool,
    pub is_nil: bool,
}

#[derive(Debug, Clone)]
pub struct ImportDecl {
    pub names: Vec<String>,
    pub module: String,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ExceptionDecl {
    pub name: String,
    pub ty: Option<Type>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

/// Type expressions.
#[derive(Debug, Clone)]
pub enum Type {
    /// `int`, `byte`, `big`, `real`, `string`
    Basic(BasicType),
    /// `array of T`
    Array(Box<Type>),
    /// `list of T`
    List(Box<Type>),
    /// `chan of T`
    Chan(Box<Type>),
    /// `chan[N] of T` (buffered)
    BufChan(Box<Expr>, Box<Type>),
    /// `ref T`
    Ref(Box<Type>),
    /// `(T1, T2, ...)`
    Tuple(Vec<Type>),
    /// `fn(params): ret`
    Func(Box<FuncSig>),
    /// Named type: `Sys`, `Draw->Context`, etc.
    Named(QualName),
    /// `module { ... }`
    Module(Vec<ModuleMember>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasicType {
    Int,
    Byte,
    Big,
    Real,
    String,
}

/// Statements.
#[derive(Debug, Clone)]
pub enum Stmt {
    Expr(Expr),
    VarDecl(VarDecl),
    Block(Block),
    If(IfStmt),
    For(ForStmt),
    While(WhileStmt),
    Do(DoStmt),
    Case(CaseStmt),
    Alt(AltStmt),
    Pick(PickStmt),
    Return(Option<Expr>, Span),
    Break(Option<String>, Span),
    Continue(Option<String>, Span),
    Exit(Span),
    Spawn(Expr, Span),
    Raise(Option<Expr>, Span),
    Label(String, Box<Stmt>),
    Empty,
}

#[derive(Debug, Clone)]
pub struct IfStmt {
    pub cond: Expr,
    pub then: Box<Stmt>,
    pub else_: Option<Box<Stmt>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ForStmt {
    pub init: Option<Box<Stmt>>,
    pub cond: Option<Expr>,
    pub post: Option<Box<Stmt>>,
    pub body: Box<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct WhileStmt {
    pub cond: Expr,
    pub body: Box<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct DoStmt {
    pub body: Box<Stmt>,
    pub cond: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct CaseStmt {
    pub expr: Expr,
    pub arms: Vec<CaseArm>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct CaseArm {
    pub patterns: Vec<CasePattern>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum CasePattern {
    Expr(Expr),
    Range(Expr, Expr),
    Wildcard,
}

#[derive(Debug, Clone)]
pub struct AltStmt {
    pub arms: Vec<AltArm>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct AltArm {
    pub guard: AltGuard,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum AltGuard {
    Recv(Option<Expr>, Expr),
    Send(Expr, Expr),
    Wildcard,
}

#[derive(Debug, Clone)]
pub struct PickStmt {
    pub name: String,
    pub expr: Expr,
    pub arms: Vec<PickArm>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct PickArm {
    pub tags: Vec<String>,
    pub body: Vec<Stmt>,
}

/// Expressions.
#[derive(Debug, Clone)]
pub enum Expr {
    /// Integer literal
    IntLit(i64, Span),
    /// Real literal
    RealLit(f64, Span),
    /// String literal
    StringLit(String, Span),
    /// Character literal
    CharLit(i32, Span),
    /// `nil`
    Nil(Span),
    /// Variable or named reference
    Ident(String, Span),
    /// Binary operation
    Binary(Box<Expr>, BinOp, Box<Expr>, Span),
    /// Unary operation
    Unary(UnaryOp, Box<Expr>, Span),
    /// Function call: `f(args)`
    Call(Box<Expr>, Vec<Expr>, Span),
    /// Member access: `expr.member`
    Dot(Box<Expr>, String, Span),
    /// Module qualification: `Mod->member`
    ModQual(Box<Expr>, String, Span),
    /// Index: `expr[index]`
    Index(Box<Expr>, Box<Expr>, Span),
    /// Slice: `expr[lo:hi]`
    Slice(Box<Expr>, Option<Box<Expr>>, Option<Box<Expr>>, Span),
    /// Tuple: `(e1, e2, ...)`
    Tuple(Vec<Expr>, Span),
    /// List cons: `expr :: expr`
    Cons(Box<Expr>, Box<Expr>, Span),
    /// Channel receive: `<-chan`
    Recv(Box<Expr>, Span),
    /// Channel send: `chan <-= value`
    Send(Box<Expr>, Box<Expr>, Span),
    /// `load ModType path`
    Load(Box<Type>, Box<Expr>, Span),
    /// `array[size] of type`
    ArrayAlloc(Box<Expr>, Box<Type>, Span),
    /// `array of { elements }`
    ArrayLit(Vec<Expr>, Option<Box<Type>>, Span),
    /// `chan of type`
    ChanAlloc(Box<Type>, Span),
    /// `list of { elements }`
    ListLit(Vec<Expr>, Span),
    /// `ref ADT(args)`
    RefAlloc(Box<Type>, Vec<Expr>, Span),
    /// Type cast: `type(expr)`
    Cast(Box<Type>, Box<Expr>, Span),
    /// Declaration expression: `name := expr`
    DeclAssign(Vec<String>, Box<Expr>, Span),
    /// Tuple declaration: `(a, b) := expr`
    TupleDeclAssign(Vec<String>, Box<Expr>, Span),
    /// Assignment: `lhs = rhs`
    Assign(Box<Expr>, Box<Expr>, Span),
    /// Compound assignment: `lhs += rhs`
    CompoundAssign(Box<Expr>, BinOp, Box<Expr>, Span),
    /// `hd expr`
    Hd(Box<Expr>, Span),
    /// `tl expr`
    Tl(Box<Expr>, Span),
    /// `len expr`
    Len(Box<Expr>, Span),
    /// `tagof expr`
    Tagof(Box<Expr>, Span),
    /// Postfix increment: `expr++`
    PostInc(Box<Expr>, Span),
    /// Postfix decrement: `expr--`
    PostDec(Box<Expr>, Span),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Power,
    And,
    Or,
    Xor,
    Lshift,
    Rshift,
    Eq,
    Neq,
    Lt,
    Gt,
    Leq,
    Geq,
    LogAnd,
    LogOr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
    Ref,
}
