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
    AmpEq,      // &=
    PipeEq,     // |=
    CaretEq,    // ^=
    LshiftEq,   // <<=
    RshiftEq,   // >>=
    Inc,        // ++
    Dec,        // --

    // Delimiters
    LParen,   // (
    RParen,   // )
    LBracket, // [
    RBracket, // ]
    LBrace,   // {
    RBrace,   // }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_returns_correct_token_kinds() {
        assert_eq!(TokenKind::keyword("if"), Some(TokenKind::If));
        assert_eq!(TokenKind::keyword("else"), Some(TokenKind::Else));
        assert_eq!(TokenKind::keyword("while"), Some(TokenKind::While));
        assert_eq!(TokenKind::keyword("for"), Some(TokenKind::For));
        assert_eq!(TokenKind::keyword("fn"), Some(TokenKind::Fn));
        assert_eq!(TokenKind::keyword("return"), Some(TokenKind::Return));
        assert_eq!(TokenKind::keyword("int"), Some(TokenKind::Int));
        assert_eq!(TokenKind::keyword("string"), Some(TokenKind::String_));
        assert_eq!(TokenKind::keyword("nil"), Some(TokenKind::Nil));
        assert_eq!(TokenKind::keyword("module"), Some(TokenKind::Module));
        assert_eq!(TokenKind::keyword("implement"), Some(TokenKind::Implement));
        assert_eq!(TokenKind::keyword("include"), Some(TokenKind::Include));
        assert_eq!(TokenKind::keyword("con"), Some(TokenKind::Con));
        assert_eq!(TokenKind::keyword("adt"), Some(TokenKind::Adt));
        assert_eq!(TokenKind::keyword("ref"), Some(TokenKind::Ref));
        assert_eq!(TokenKind::keyword("chan"), Some(TokenKind::Chan));
        assert_eq!(TokenKind::keyword("array"), Some(TokenKind::Array));
        assert_eq!(TokenKind::keyword("list"), Some(TokenKind::List));
        assert_eq!(TokenKind::keyword("self"), Some(TokenKind::Self_));
        assert_eq!(TokenKind::keyword("spawn"), Some(TokenKind::Spawn));
        assert_eq!(TokenKind::keyword("raise"), Some(TokenKind::Raise));
        assert_eq!(TokenKind::keyword("iota"), Some(TokenKind::Iota));
        assert_eq!(TokenKind::keyword("exception"), Some(TokenKind::Exception));
    }

    #[test]
    fn keyword_returns_none_for_non_keywords() {
        assert_eq!(TokenKind::keyword("foo"), None);
        assert_eq!(TokenKind::keyword("bar"), None);
        assert_eq!(TokenKind::keyword("main"), None);
        assert_eq!(TokenKind::keyword("init"), None);
        assert_eq!(TokenKind::keyword(""), None);
        assert_eq!(TokenKind::keyword("IF"), None);
        assert_eq!(TokenKind::keyword("Int"), None);
        assert_eq!(TokenKind::keyword("123"), None);
    }

    #[test]
    fn span_default() {
        let s = Span::default();
        assert_eq!(s.line, 0);
        assert_eq!(s.col, 0);
    }
}
