//! Limbo language tokens.

/// Source location for error reporting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub line: u32,
    pub col: u32,
}

/// A token with its source location.
#[derive(Clone, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// All Limbo token types.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    // Literals
    IntLit(i64),
    RealLit(f64),
    StringLit(String),
    CharLit(i32),

    // Identifier
    Ident(String),

    // Keywords
    Adt,
    Alt,
    Array,
    Big,
    Break,
    Byte,
    Case,
    Chan,
    Con,
    Continue,
    Cyclic,
    Do,
    Else,
    Exception,
    Exit,
    Fn,
    For,
    Hd,
    If,
    Implement,
    Import,
    Include,
    Int,
    Len,
    List,
    Load,
    Module,
    Nil,
    Of,
    Or,
    Pick,
    Real,
    Ref,
    Return,
    Self_,
    Spawn,
    String_,
    Tagof,
    Tl,
    To,
    Type,
    While,
    Raise,
    Iota,

    // Operators
    Plus,       // +
    Minus,      // -
    Star,       // *
    Slash,      // /
    Percent,    // %
    Power,      // **
    Amp,        // &
    Pipe,       // |
    Caret,      // ^
    Tilde,      // ~
    Bang,       // !
    Lshift,     // <<
    Rshift,     // >>
    Eq,         // ==
    Neq,        // !=
    Lt,         // <
    Gt,         // >
    Leq,        // <=
    Geq,        // >=
    AndAnd,     // &&
    OrOr,       // ||
    ColonColon, // ::
    Arrow,      // ->
    ChanRecv,   // <-
    ChanSend,   // <-=
    Assign,     // =
    ColonEq,    // :=
    PlusEq,     // +=
    MinusEq,    // -=
    StarEq,     // *=
    SlashEq,    // /=
    PercentEq,  // %=
    AmpEq,     // &=
    PipeEq,    // |=
    CaretEq,   // ^=
    LshiftEq,  // <<=
    RshiftEq,  // >>=
    Inc,        // ++
    Dec,        // --

    // Delimiters
    LParen,    // (
    RParen,    // )
    LBracket,  // [
    RBracket,  // ]
    LBrace,    // {
    RBrace,    // }

    // Punctuation
    Comma,     // ,
    Dot,       // .
    Semicolon, // ;
    Colon,     // :
    FatArrow,  // =>

    // Special
    Eof,
}

impl TokenKind {
    /// Look up a keyword from an identifier string.
    pub fn keyword(s: &str) -> Option<TokenKind> {
        match s {
            "adt" => Some(TokenKind::Adt),
            "alt" => Some(TokenKind::Alt),
            "array" => Some(TokenKind::Array),
            "big" => Some(TokenKind::Big),
            "break" => Some(TokenKind::Break),
            "byte" => Some(TokenKind::Byte),
            "case" => Some(TokenKind::Case),
            "chan" => Some(TokenKind::Chan),
            "con" => Some(TokenKind::Con),
            "continue" => Some(TokenKind::Continue),
            "cyclic" => Some(TokenKind::Cyclic),
            "do" => Some(TokenKind::Do),
            "else" => Some(TokenKind::Else),
            "exception" => Some(TokenKind::Exception),
            "exit" => Some(TokenKind::Exit),
            "fn" => Some(TokenKind::Fn),
            "for" => Some(TokenKind::For),
            "hd" => Some(TokenKind::Hd),
            "if" => Some(TokenKind::If),
            "implement" => Some(TokenKind::Implement),
            "import" => Some(TokenKind::Import),
            "include" => Some(TokenKind::Include),
            "int" => Some(TokenKind::Int),
            "iota" => Some(TokenKind::Iota),
            "len" => Some(TokenKind::Len),
            "list" => Some(TokenKind::List),
            "load" => Some(TokenKind::Load),
            "module" => Some(TokenKind::Module),
            "nil" => Some(TokenKind::Nil),
            "of" => Some(TokenKind::Of),
            "or" => Some(TokenKind::Or),
            "pick" => Some(TokenKind::Pick),
            "raise" => Some(TokenKind::Raise),
            "real" => Some(TokenKind::Real),
            "ref" => Some(TokenKind::Ref),
            "return" => Some(TokenKind::Return),
            "self" => Some(TokenKind::Self_),
            "spawn" => Some(TokenKind::Spawn),
            "string" => Some(TokenKind::String_),
            "tagof" => Some(TokenKind::Tagof),
            "tl" => Some(TokenKind::Tl),
            "to" => Some(TokenKind::To),
            "type" => Some(TokenKind::Type),
            "while" => Some(TokenKind::While),
            _ => None,
        }
    }
}
