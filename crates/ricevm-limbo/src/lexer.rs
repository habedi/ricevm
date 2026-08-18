//! Limbo lexer: converts source text into a stream of tokens.

use crate::token::{Span, Token, TokenKind};

/// Compilation error with source location.
#[derive(Clone, Debug, thiserror::Error)]
#[error("{file}:{}:{}: {message}", span.line, span.col)]
pub struct LexError {
    pub file: String,
    pub span: Span,
    pub message: String,
}

/// Lexer state.
pub struct Lexer<'src> {
    src: &'src [u8],
    pos: usize,
    line: u32,
    col: u32,
    file: String,
}

impl<'src> Lexer<'src> {
    pub fn new(src: &'src str, file: &str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
            file: file.to_string(),
        }
    }

    /// Tokenize the entire source into a vector of tokens.
    pub fn tokenize(&mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token()?;
            let is_eof = tok.kind == TokenKind::Eof;
            tokens.push(tok);
            if is_eof {
                break;
            }
        }
        Ok(tokens)
    }

    fn span(&self) -> Span {
        Span {
            line: self.line,
            col: self.col,
        }
    }

    fn peek(&self) -> u8 {
        if self.pos < self.src.len() {
            self.src[self.pos]
        } else {
            0
        }
    }

    fn peek2(&self) -> u8 {
        if self.pos + 1 < self.src.len() {
            self.src[self.pos + 1]
        } else {
            0
        }
    }

    fn advance(&mut self) -> u8 {
        if self.pos < self.src.len() {
            let ch = self.src[self.pos];
            self.pos += 1;
            if ch == b'\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
            ch
        } else {
            0
        }
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            // Skip whitespace
            while self.pos < self.src.len() && self.peek().is_ascii_whitespace() {
                self.advance();
            }
            // Skip line comments
            if self.peek() == b'#' {
                while self.pos < self.src.len() && self.peek() != b'\n' {
                    self.advance();
                }
                continue;
            }
            break;
        }
    }

    fn err(&self, msg: impl Into<String>) -> LexError {
        self.err_at(self.span(), msg)
    }

    /// Error reported at a place the lexer has already moved past, such as the
    /// first character of the token being read.
    fn err_at(&self, span: Span, msg: impl Into<String>) -> LexError {
        LexError {
            file: self.file.clone(),
            span,
            message: msg.into(),
        }
    }

    fn next_token(&mut self) -> Result<Token, LexError> {
        self.skip_whitespace_and_comments();

        if self.pos >= self.src.len() {
            return Ok(Token {
                kind: TokenKind::Eof,
                span: self.span(),
            });
        }

        let span = self.span();
        let ch = self.peek();

        // Float literal starting with '.' (e.g., .000001)
        if ch == b'.' && self.peek2().is_ascii_digit() {
            return self.lex_dot_number(span);
        }

        // Identifiers and keywords. A scalar outside ASCII is a letter as far
        // as the reference is concerned: its character map marks every byte
        // above 0xA0 as an identifier character and reports every rune from
        // 0x100 up as a lowercase letter (lex.c:141-147 and lex.c:188-195),
        // and `lexid` accumulates whatever those two rules accept
        // (lex.c:511-543, reached from lex.c:1082-1083). The corpus uses it:
        // appl/spree/lib/testsets.b:18 declares `∈: Set;` and appl/wm/c4.b:387
        // passes `∞`.
        if ch.is_ascii_alphabetic() || ch == b'_' || is_ident_byte(ch) {
            return self.lex_ident(span);
        }

        // Numeric literals
        if ch.is_ascii_digit() {
            return self.lex_number(span);
        }

        // String literals
        if ch == b'"' {
            return self.lex_string(span);
        }

        // Character literals
        if ch == b'\'' {
            return self.lex_char(span);
        }

        // Operators and punctuation
        self.lex_operator(span)
    }

    fn lex_ident(&mut self, span: Span) -> Result<Token, LexError> {
        let start = self.pos;
        while self.pos < self.src.len()
            && (self.peek().is_ascii_alphanumeric()
                || self.peek() == b'_'
                || is_ident_byte(self.peek()))
        {
            self.advance();
        }
        let word = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| self.err("invalid UTF-8 in identifier"))?;

        let kind = TokenKind::keyword(word).unwrap_or_else(|| TokenKind::Ident(word.to_string()));
        Ok(Token { kind, span })
    }

    fn lex_dot_number(&mut self, span: Span) -> Result<Token, LexError> {
        let start = self.pos;
        self.advance(); // skip '.'
        while self.pos < self.src.len() && self.peek().is_ascii_digit() {
            self.advance();
        }
        if self.peek() == b'e' || self.peek() == b'E' {
            self.advance();
            if self.peek() == b'+' || self.peek() == b'-' {
                self.advance();
            }
            while self.pos < self.src.len() && self.peek().is_ascii_digit() {
                self.advance();
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| self.err("invalid float"))?;
        let val: f64 = text.parse().map_err(|e| self.err(format!("{e}")))?;
        Ok(Token {
            kind: TokenKind::RealLit(val),
            span,
        })
    }

    fn lex_number(&mut self, span: Span) -> Result<Token, LexError> {
        let start = self.pos;

        // Handle hex: 16r... or 0x...
        if self.peek() == b'0' && (self.peek2() == b'x' || self.peek2() == b'X') {
            self.advance();
            self.advance();
            while self.pos < self.src.len() && self.peek().is_ascii_hexdigit() {
                self.advance();
            }
            let hex = std::str::from_utf8(&self.src[start + 2..self.pos])
                .map_err(|_| self.err("invalid hex literal"))?;
            let val = i64::from_str_radix(hex, 16).map_err(|e| self.err(format!("{e}")))?;
            return Ok(Token {
                kind: TokenKind::IntLit(val),
                span,
            });
        }

        // Consume digits
        while self.pos < self.src.len() && self.peek().is_ascii_digit() {
            self.advance();
        }

        // Check for radix notation: NNr... (e.g., 16rFF)
        if self.peek() == b'r' || self.peek() == b'R' {
            let radix_str = std::str::from_utf8(&self.src[start..self.pos])
                .map_err(|_| self.err("invalid radix"))?;
            let radix: u32 = radix_str.parse().map_err(|e| self.err(format!("{e}")))?;
            if !(2..=36).contains(&radix) {
                return Err(self.err(format!("radix must be between 2 and 36, found {radix}")));
            }
            self.advance(); // skip 'r'
            let digits_start = self.pos;
            while self.pos < self.src.len()
                && (self.peek().is_ascii_alphanumeric() || self.peek() == b'_')
            {
                self.advance();
            }
            let digits = std::str::from_utf8(&self.src[digits_start..self.pos])
                .map_err(|_| self.err("invalid radix digits"))?;
            let val = i64::from_str_radix(digits, radix).map_err(|e| self.err(format!("{e}")))?;
            return Ok(Token {
                kind: TokenKind::IntLit(val),
                span,
            });
        }

        // Check for float: digits.digits, digits., digits.digitsE..., digits E...
        // A dot always ends the integer part: the reference moves from its
        // `Int` state to `Frac` on any dot and never returns (lex.c:702-705),
        // so `1.e-30` is one real literal (appl/math/gr.b:215) and `1.` is a
        // real even when a letter follows it.
        let mut is_float = false;
        if self.peek() == b'.' {
            is_float = true;
            self.advance(); // skip '.'
            while self.pos < self.src.len() && self.peek().is_ascii_digit() {
                self.advance();
            }
        }
        if self.peek() == b'e' || self.peek() == b'E' {
            is_float = true;
            self.advance();
            if self.peek() == b'+' || self.peek() == b'-' {
                self.advance();
            }
            while self.pos < self.src.len() && self.peek().is_ascii_digit() {
                self.advance();
            }
        }

        let text = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| self.err("invalid number literal"))?;

        if is_float {
            let val: f64 = text.parse().map_err(|e| self.err(format!("{e}")))?;
            Ok(Token {
                kind: TokenKind::RealLit(val),
                span,
            })
        } else {
            let val: i64 = text.parse().map_err(|e| self.err(format!("{e}")))?;
            Ok(Token {
                kind: TokenKind::IntLit(val),
                span,
            })
        }
    }

    fn lex_string(&mut self, span: Span) -> Result<Token, LexError> {
        self.advance(); // skip opening "
        let mut s = String::new();
        loop {
            if self.pos >= self.src.len() {
                return Err(self.err("unterminated string literal"));
            }
            let ch = self.peek();
            if ch == b'"' {
                self.advance();
                break;
            }
            if ch == b'\\' {
                self.advance();
                let esc = self.advance();
                let val = self.escape_value(esc)?;
                if let Some(c) = char::from_u32(val) {
                    s.push(c);
                }
            } else {
                s.push(self.next_utf8_char());
            }
        }
        Ok(Token {
            kind: TokenKind::StringLit(s),
            span,
        })
    }

    /// Decode exactly one UTF-8 scalar at the current position and advance
    /// past its bytes. Bytes that do not start a valid scalar are consumed
    /// one at a time and returned as-is.
    fn next_utf8_char(&mut self) -> char {
        let start = self.pos;
        let lead = self.peek();
        let end = (start + utf8_len(lead)).min(self.src.len());
        let decoded = std::str::from_utf8(&self.src[start..end])
            .ok()
            .and_then(|s| s.chars().next());
        if let Some(c) = decoded {
            for _ in 0..c.len_utf8() {
                self.advance();
            }
            return c;
        }
        self.advance();
        lead as char
    }

    /// Value of the escape sequence that follows a backslash. The reference
    /// keeps one escape table for both string and character constants
    /// (`escmap` in lex.c:167-182, read by `escchar` in lex.c:789-812), so
    /// `'\v'` and `"\v"` have to agree on 0x0B. A sequence the table does not
    /// name keeps the escaped byte, which is what `escchar` returns after it
    /// reports the unknown escape.
    fn escape_value(&mut self, esc: u8) -> Result<u32, LexError> {
        let val = match esc {
            b'n' => 0x0A,
            b't' => 0x09,
            b'r' => 0x0D,
            b'a' => 0x07,
            b'b' => 0x08,
            b'f' => 0x0C,
            b'v' => 0x0B,
            b'0' => 0x00,
            b'u' => self.lex_hex_escape(4)?,
            other => other as u32,
        };
        Ok(val)
    }

    fn lex_char(&mut self, span: Span) -> Result<Token, LexError> {
        self.advance(); // skip opening '
        let val = if self.peek() == b'\\' {
            self.advance();
            let esc = self.advance();
            self.escape_value(esc)? as i32
        } else {
            // One UTF-8 scalar, decoded from its own bytes only
            self.next_utf8_char() as i32
        };
        // The closing quote is part of the literal; the reference reports a
        // missing one rather than accepting the truncated form (lex.c:930-934).
        if self.peek() != b'\'' {
            return Err(self.err("missing closing ' in character literal"));
        }
        self.advance();
        Ok(Token {
            kind: TokenKind::CharLit(val),
            span,
        })
    }

    fn lex_hex_escape(&mut self, digits: usize) -> Result<u32, LexError> {
        let mut val = 0u32;
        for _ in 0..digits {
            let d = self.advance();
            let n = match d {
                b'0'..=b'9' => d - b'0',
                b'a'..=b'f' => d - b'a' + 10,
                b'A'..=b'F' => d - b'A' + 10,
                _ => return Err(self.err("invalid hex escape")),
            };
            val = val * 16 + n as u32;
        }
        Ok(val)
    }

    fn lex_operator(&mut self, span: Span) -> Result<Token, LexError> {
        let ch = self.advance();
        let kind = match ch {
            b'+' => {
                if self.peek() == b'+' {
                    self.advance();
                    TokenKind::Inc
                } else if self.peek() == b'=' {
                    self.advance();
                    TokenKind::PlusEq
                } else {
                    TokenKind::Plus
                }
            }
            b'-' => {
                if self.peek() == b'-' {
                    self.advance();
                    TokenKind::Dec
                } else if self.peek() == b'=' {
                    self.advance();
                    TokenKind::MinusEq
                } else if self.peek() == b'>' {
                    self.advance();
                    TokenKind::Arrow
                } else {
                    TokenKind::Minus
                }
            }
            b'*' => {
                if self.peek() == b'*' {
                    self.advance();
                    TokenKind::Power
                } else if self.peek() == b'=' {
                    self.advance();
                    TokenKind::StarEq
                } else {
                    TokenKind::Star
                }
            }
            b'/' => {
                if self.peek() == b'=' {
                    self.advance();
                    TokenKind::SlashEq
                } else {
                    TokenKind::Slash
                }
            }
            b'%' => {
                if self.peek() == b'=' {
                    self.advance();
                    TokenKind::PercentEq
                } else {
                    TokenKind::Percent
                }
            }
            b'&' => {
                if self.peek() == b'&' {
                    self.advance();
                    TokenKind::AndAnd
                } else if self.peek() == b'=' {
                    self.advance();
                    TokenKind::AmpEq
                } else {
                    TokenKind::Amp
                }
            }
            b'|' => {
                if self.peek() == b'|' {
                    self.advance();
                    TokenKind::OrOr
                } else if self.peek() == b'=' {
                    self.advance();
                    TokenKind::PipeEq
                } else {
                    TokenKind::Pipe
                }
            }
            b'^' => {
                if self.peek() == b'=' {
                    self.advance();
                    TokenKind::CaretEq
                } else {
                    TokenKind::Caret
                }
            }
            b'~' => TokenKind::Tilde,
            b'!' => {
                if self.peek() == b'=' {
                    self.advance();
                    TokenKind::Neq
                } else {
                    TokenKind::Bang
                }
            }
            b'<' => {
                if self.peek() == b'<' {
                    self.advance();
                    if self.peek() == b'=' {
                        self.advance();
                        TokenKind::LshiftEq
                    } else {
                        TokenKind::Lshift
                    }
                } else if self.peek() == b'=' {
                    self.advance();
                    TokenKind::Leq
                } else if self.peek() == b'-' {
                    self.advance();
                    if self.peek() == b'=' {
                        self.advance();
                        TokenKind::ChanSend
                    } else {
                        TokenKind::ChanRecv
                    }
                } else {
                    TokenKind::Lt
                }
            }
            b'>' => {
                if self.peek() == b'>' {
                    self.advance();
                    if self.peek() == b'=' {
                        self.advance();
                        TokenKind::RshiftEq
                    } else {
                        TokenKind::Rshift
                    }
                } else if self.peek() == b'=' {
                    self.advance();
                    TokenKind::Geq
                } else {
                    TokenKind::Gt
                }
            }
            b'=' => {
                if self.peek() == b'=' {
                    self.advance();
                    TokenKind::Eq
                } else if self.peek() == b'>' {
                    self.advance();
                    TokenKind::FatArrow
                } else {
                    TokenKind::Assign
                }
            }
            b':' => {
                if self.peek() == b'=' {
                    self.advance();
                    TokenKind::ColonEq
                } else if self.peek() == b':' {
                    self.advance();
                    TokenKind::ColonColon
                } else {
                    TokenKind::Colon
                }
            }
            b'(' => TokenKind::LParen,
            b')' => TokenKind::RParen,
            b'[' => TokenKind::LBracket,
            b']' => TokenKind::RBracket,
            b'{' => TokenKind::LBrace,
            b'}' => TokenKind::RBrace,
            b',' => TokenKind::Comma,
            b'.' => TokenKind::Dot,
            b';' => TokenKind::Semicolon,
            // Reported at the character itself, which `advance` has already
            // stepped over.
            _ => {
                return Err(self.err_at(span, format!("unexpected character: {:?}", ch as char)));
            }
        };
        Ok(Token { kind, span })
    }
}

/// Is this byte part of a scalar outside ASCII? Such a byte belongs to an
/// identifier, since the source is valid UTF-8 and every non-ASCII scalar
/// counts as a letter (lex.c:141-147 and lex.c:188-195).
fn is_ident_byte(b: u8) -> bool {
    b >= 0x80
}

/// Number of bytes in the UTF-8 scalar introduced by `lead`.
fn utf8_len(lead: u8) -> usize {
    match lead {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(src: &str) -> Vec<TokenKind> {
        Lexer::new(src, "<test>")
            .tokenize()
            .expect("lex should succeed")
            .into_iter()
            .map(|t| t.kind)
            .filter(|k| *k != TokenKind::Eof)
            .collect()
    }

    fn lex_err(src: &str) -> String {
        Lexer::new(src, "<test>")
            .tokenize()
            .expect_err("lex should fail")
            .message
    }

    #[test]
    fn keywords() {
        let tokens = lex("if else while for fn return");
        assert_eq!(
            tokens,
            vec![
                TokenKind::If,
                TokenKind::Else,
                TokenKind::While,
                TokenKind::For,
                TokenKind::Fn,
                TokenKind::Return,
            ]
        );
    }

    #[test]
    fn identifiers_and_numbers() {
        let tokens = lex("x := 42;");
        assert_eq!(
            tokens,
            vec![
                TokenKind::Ident("x".to_string()),
                TokenKind::ColonEq,
                TokenKind::IntLit(42),
                TokenKind::Semicolon,
            ]
        );
    }

    #[test]
    fn hex_literal() {
        let tokens = lex("0xFF 16rAB");
        assert_eq!(
            tokens,
            vec![TokenKind::IntLit(255), TokenKind::IntLit(0xAB)]
        );
    }

    #[test]
    fn float_literal() {
        let tokens = lex("3.25 1e10 2.5e-3");
        assert_eq!(
            tokens,
            vec![
                TokenKind::RealLit(3.25),
                TokenKind::RealLit(1e10),
                TokenKind::RealLit(2.5e-3),
            ]
        );
    }

    #[test]
    fn string_literal() {
        let tokens = lex(r#""hello\nworld""#);
        assert_eq!(
            tokens,
            vec![TokenKind::StringLit("hello\nworld".to_string())]
        );
    }

    #[test]
    fn operators() {
        let tokens = lex("<- <-= :: -> ** =>");
        assert_eq!(
            tokens,
            vec![
                TokenKind::ChanRecv,
                TokenKind::ChanSend,
                TokenKind::ColonColon,
                TokenKind::Arrow,
                TokenKind::Power,
                TokenKind::FatArrow,
            ]
        );
    }

    #[test]
    fn comments() {
        let tokens = lex("x # this is a comment\ny");
        assert_eq!(
            tokens,
            vec![
                TokenKind::Ident("x".to_string()),
                TokenKind::Ident("y".to_string()),
            ]
        );
    }

    #[test]
    fn echo_program_tokens() {
        let src = r#"implement Echo;
include "sys.m";
    sys: Sys;
"#;
        let tokens = lex(src);
        assert_eq!(
            tokens,
            vec![
                TokenKind::Implement,
                TokenKind::Ident("Echo".to_string()),
                TokenKind::Semicolon,
                TokenKind::Include,
                TokenKind::StringLit("sys.m".to_string()),
                TokenKind::Semicolon,
                TokenKind::Ident("sys".to_string()),
                TokenKind::Colon,
                TokenKind::Ident("Sys".to_string()),
                TokenKind::Semicolon,
            ]
        );
    }

    #[test]
    fn char_literal() {
        let tokens = lex("'a' '\\n' '\\t'");
        assert_eq!(
            tokens,
            vec![
                TokenKind::CharLit(b'a' as i32),
                TokenKind::CharLit(b'\n' as i32),
                TokenKind::CharLit(b'\t' as i32),
            ]
        );
    }

    #[test]
    fn string_escapes() {
        let tokens = lex(r#""hello\tworld\n""#);
        assert_eq!(
            tokens,
            vec![TokenKind::StringLit("hello\tworld\n".to_string())]
        );
    }

    #[test]
    fn radix_literal() {
        let tokens = lex("8r77 2r1010");
        assert_eq!(
            tokens,
            vec![TokenKind::IntLit(0o77), TokenKind::IntLit(0b1010)]
        );
    }

    #[test]
    fn assignment_operators() {
        let tokens = lex("+= -= *= /= %= &= |= ^= <<= >>=");
        assert_eq!(
            tokens,
            vec![
                TokenKind::PlusEq,
                TokenKind::MinusEq,
                TokenKind::StarEq,
                TokenKind::SlashEq,
                TokenKind::PercentEq,
                TokenKind::AmpEq,
                TokenKind::PipeEq,
                TokenKind::CaretEq,
                TokenKind::LshiftEq,
                TokenKind::RshiftEq,
            ]
        );
    }

    #[test]
    fn comparison_operators() {
        let tokens = lex("== != < > <= >=");
        assert_eq!(
            tokens,
            vec![
                TokenKind::Eq,
                TokenKind::Neq,
                TokenKind::Lt,
                TokenKind::Gt,
                TokenKind::Leq,
                TokenKind::Geq,
            ]
        );
    }

    #[test]
    fn delimiters() {
        let tokens = lex("( ) [ ] { }");
        assert_eq!(
            tokens,
            vec![
                TokenKind::LParen,
                TokenKind::RParen,
                TokenKind::LBracket,
                TokenKind::RBracket,
                TokenKind::LBrace,
                TokenKind::RBrace,
            ]
        );
    }

    #[test]
    fn punctuation() {
        let tokens = lex(", . ; : :=");
        assert_eq!(
            tokens,
            vec![
                TokenKind::Comma,
                TokenKind::Dot,
                TokenKind::Semicolon,
                TokenKind::Colon,
                TokenKind::ColonEq,
            ]
        );
    }

    #[test]
    fn increment_decrement() {
        let tokens = lex("++ --");
        assert_eq!(tokens, vec![TokenKind::Inc, TokenKind::Dec]);
    }

    #[test]
    fn logical_operators() {
        let tokens = lex("&& || ! ~");
        assert_eq!(
            tokens,
            vec![
                TokenKind::AndAnd,
                TokenKind::OrOr,
                TokenKind::Bang,
                TokenKind::Tilde
            ]
        );
    }

    #[test]
    fn trailing_dot_float() {
        let tokens = lex("1000. 5.");
        assert_eq!(
            tokens,
            vec![TokenKind::RealLit(1000.0), TokenKind::RealLit(5.0)]
        );
    }

    #[test]
    fn leading_dot_float() {
        let tokens = lex(".5 .001");
        assert_eq!(
            tokens,
            vec![TokenKind::RealLit(0.5), TokenKind::RealLit(0.001)]
        );
    }

    #[test]
    fn empty_string() {
        let tokens = lex(r#""""#);
        assert_eq!(tokens, vec![TokenKind::StringLit(String::new())]);
    }

    #[test]
    fn radix_literal_bounds() {
        let tokens = lex("2r1010 36rzz");
        assert_eq!(
            tokens,
            vec![TokenKind::IntLit(0b1010), TokenKind::IntLit(35 * 36 + 35)]
        );
    }

    #[test]
    fn radix_out_of_range_is_error() {
        // Radices outside 2..=36 must be reported, not passed to
        // `i64::from_str_radix` (which panics on them). The reference draws
        // the same bounds (lex.c:757-760).
        for src in ["99r11", "1r0", "0r1", "37r1", "100r5"] {
            let msg = lex_err(src);
            assert!(msg.contains("radix"), "unexpected message for {src}: {msg}");
        }
    }

    #[test]
    fn string_literal_non_ascii_utf8() {
        let tokens = lex(r#""héllo ü""#);
        assert_eq!(tokens, vec![TokenKind::StringLit("héllo ü".to_string())]);
    }

    #[test]
    fn char_literal_non_ascii_utf8() {
        // Each literal must decode exactly one scalar starting at its own
        // first byte, independent of the bytes that follow it.
        let tokens = lex("'é''ü' '\u{20AC}' '\u{1F600}'");
        assert_eq!(
            tokens,
            vec![
                TokenKind::CharLit(0xE9),
                TokenKind::CharLit(0xFC),
                TokenKind::CharLit(0x20AC),
                TokenKind::CharLit(0x1F600),
            ]
        );
    }

    #[test]
    fn consecutive_keywords() {
        let tokens = lex("if else while for do case alt pick spawn");
        assert_eq!(
            tokens,
            vec![
                TokenKind::If,
                TokenKind::Else,
                TokenKind::While,
                TokenKind::For,
                TokenKind::Do,
                TokenKind::Case,
                TokenKind::Alt,
                TokenKind::Pick,
                TokenKind::Spawn,
            ]
        );
    }

    /// Every word the lexer treats as a keyword, checked one at a time so a
    /// missing entry names itself. The reference keyword table (lex.c:60-99)
    /// also holds `dynamic`, `fixed`, and `raises`; this front end leaves those
    /// as identifiers, and the parser recognizes `raises` by name.
    #[test]
    fn every_keyword_lexes_to_its_own_token() {
        let table: &[(&str, TokenKind)] = &[
            ("adt", TokenKind::Adt),
            ("alt", TokenKind::Alt),
            ("array", TokenKind::Array),
            ("big", TokenKind::Big),
            ("break", TokenKind::Break),
            ("byte", TokenKind::Byte),
            ("case", TokenKind::Case),
            ("chan", TokenKind::Chan),
            ("con", TokenKind::Con),
            ("continue", TokenKind::Continue),
            ("cyclic", TokenKind::Cyclic),
            ("do", TokenKind::Do),
            ("else", TokenKind::Else),
            ("exception", TokenKind::Exception),
            ("exit", TokenKind::Exit),
            ("fn", TokenKind::Fn),
            ("for", TokenKind::For),
            ("hd", TokenKind::Hd),
            ("if", TokenKind::If),
            ("implement", TokenKind::Implement),
            ("import", TokenKind::Import),
            ("include", TokenKind::Include),
            ("int", TokenKind::Int),
            ("iota", TokenKind::Iota),
            ("len", TokenKind::Len),
            ("list", TokenKind::List),
            ("load", TokenKind::Load),
            ("module", TokenKind::Module),
            ("nil", TokenKind::Nil),
            ("of", TokenKind::Of),
            ("or", TokenKind::Or),
            ("pick", TokenKind::Pick),
            ("raise", TokenKind::Raise),
            ("real", TokenKind::Real),
            ("ref", TokenKind::Ref),
            ("return", TokenKind::Return),
            ("self", TokenKind::Self_),
            ("spawn", TokenKind::Spawn),
            ("string", TokenKind::String_),
            ("tagof", TokenKind::Tagof),
            ("tl", TokenKind::Tl),
            ("to", TokenKind::To),
            ("type", TokenKind::Type),
            ("while", TokenKind::While),
        ];
        assert_eq!(table.len(), 44, "the keyword table changed size");
        for (word, kind) in table {
            assert_eq!(lex(word), vec![kind.clone()], "keyword {word}");
        }
    }

    #[test]
    fn keyword_lookalikes_are_identifiers() {
        for word in [
            "ifx",
            "elsewhere",
            "_if",
            "If",
            "INT",
            "int32",
            "raises",
            "dynamic",
            "fixed",
        ] {
            assert_eq!(
                lex(word),
                vec![TokenKind::Ident(word.to_string())],
                "{word} must stay an identifier"
            );
        }
    }

    #[test]
    fn identifier_shapes() {
        let tokens = lex("_ _x x_1 X9 aB_2");
        assert_eq!(
            tokens,
            vec![
                TokenKind::Ident("_".to_string()),
                TokenKind::Ident("_x".to_string()),
                TokenKind::Ident("x_1".to_string()),
                TokenKind::Ident("X9".to_string()),
                TokenKind::Ident("aB_2".to_string()),
            ]
        );
    }

    /// Radix literals across the whole legal range, in both letter cases. The
    /// reference accepts `[0-9]+[rR][0-9A-Za-z]+` (lex.c:661-662, lex.c:143-152).
    #[test]
    fn radix_literals_across_the_valid_range() {
        let table: &[(&str, i64)] = &[
            ("2r1010", 10),
            ("2R11", 3),
            ("3r222", 26),
            ("8r777", 511),
            ("10r99", 99),
            ("16rff", 255),
            ("16rFF", 255),
            ("16RfF", 255),
            ("35ryy", 1224),
            ("36rz", 35),
            ("36rzz", 1295),
            ("36RZZ", 1295),
        ];
        for (src, want) in table {
            assert_eq!(lex(src), vec![TokenKind::IntLit(*want)], "literal {src}");
        }
    }

    #[test]
    fn radix_digit_outside_the_radix_is_error() {
        // The reference rejects the digit rather than the literal's shape
        // (lex.c:566-571).
        for src in ["2r12", "8r99", "16rgg", "10rff"] {
            let msg = lex_err(src);
            assert!(msg.contains("digit"), "unexpected message for {src}: {msg}");
        }
    }

    #[test]
    fn radix_without_digits_is_error() {
        let msg = lex_err("16r");
        assert!(msg.contains("empty"), "unexpected message: {msg}");
    }

    #[test]
    fn hex_literal_forms() {
        let table: &[(&str, i64)] = &[
            ("0x0", 0),
            ("0xff", 255),
            ("0XFF", 255),
            ("0x7fffffff", 0x7fff_ffff),
            ("0xffffffff", 0xffff_ffff),
            ("0x7fffffffffffffff", i64::MAX),
        ];
        for (src, want) in table {
            assert_eq!(lex(src), vec![TokenKind::IntLit(*want)], "literal {src}");
        }
    }

    #[test]
    fn hex_literal_without_digits_is_error() {
        for src in ["0x", "0X"] {
            let msg = lex_err(src);
            assert!(msg.contains("empty"), "unexpected message for {src}: {msg}");
        }
    }

    #[test]
    fn integer_literal_that_overflows_is_error() {
        // Every literal path reports overflow instead of panicking.
        for src in [
            "99999999999999999999",
            "0xffffffffffffffffff",
            "36rzzzzzzzzzzzzzz",
        ] {
            let msg = lex_err(src);
            assert!(
                msg.contains("too large") || msg.contains("number"),
                "unexpected message for {src}: {msg}"
            );
        }
    }

    #[test]
    fn decimal_integer_bounds() {
        assert_eq!(lex("0"), vec![TokenKind::IntLit(0)]);
        assert_eq!(
            lex("9223372036854775807"),
            vec![TokenKind::IntLit(i64::MAX)]
        );
        // The lexer never signs a literal; `-5` is a unary minus and a literal.
        assert_eq!(lex("-5"), vec![TokenKind::Minus, TokenKind::IntLit(5)]);
    }

    /// Every real-literal shape the reference accepts:
    /// `([0-9]+(\.[0-9]*)?|\.[0-9]+)([eE][+-]?[0-9]+)?` (lex.c:662).
    #[test]
    fn real_literal_forms() {
        let table: &[(&str, f64)] = &[
            ("3.25", 3.25),
            ("0.0", 0.0),
            ("1000.", 1000.0),
            (".5", 0.5),
            (".000001", 0.000001),
            ("1e10", 1e10),
            ("1E10", 1e10),
            ("1e+10", 1e10),
            ("1e-10", 1e-10),
            ("2.5e-3", 2.5e-3),
            ("2.5E+3", 2.5e3),
            (".5e2", 50.0),
            (".5E-2", 0.005),
            ("1.e3", 1000.0),
        ];
        for (src, want) in table {
            assert_eq!(lex(src), vec![TokenKind::RealLit(*want)], "literal {src}");
        }
    }

    #[test]
    fn real_literal_with_empty_exponent_is_error() {
        for src in ["1e", "1e+", "1.5e-", ".5e", "1.e"] {
            let msg = lex_err(src);
            assert!(
                msg.contains("float") || msg.contains("number"),
                "unexpected message for {src}: {msg}"
            );
        }
    }

    /// `1.e-30` is one real literal: the reference leaves its integer state on
    /// any dot and reads the exponent from the fraction state (lex.c:702-724).
    /// It used to lex as `1`, `.`, `e`, `-`, `30`, which broke every source
    /// that writes a real that way, such as appl/math/gr.b:215.
    #[test]
    fn real_literal_with_a_dot_before_the_exponent() {
        assert_eq!(lex("1.e-30"), vec![TokenKind::RealLit(1e-30)]);
        assert_eq!(lex("1.E+3"), vec![TokenKind::RealLit(1000.0)]);
        assert_eq!(
            lex("r := 1.e-30;"),
            vec![
                TokenKind::Ident("r".to_string()),
                TokenKind::ColonEq,
                TokenKind::RealLit(1e-30),
                TokenKind::Semicolon,
            ]
        );
    }

    /// A trailing dot ends the literal even when a letter follows, which is
    /// what the reference's `Frac` state does (lex.c:715-723).
    #[test]
    fn trailing_dot_before_a_letter_is_still_a_real() {
        assert_eq!(
            lex("1000.foo"),
            vec![
                TokenKind::RealLit(1000.0),
                TokenKind::Ident("foo".to_string())
            ]
        );
    }

    /// A number followed by `r` and a real fraction (`16r1.8`) is a radix real
    /// in the reference (the `FracB` state, lex.c:766-785). This front end has
    /// no radix-real form, so it stops the literal at the dot; the digits after
    /// it lex as a separate real. Recorded here so the split is a decision and
    /// not a surprise.
    #[test]
    fn radix_real_literal_splits_into_two_tokens() {
        assert_eq!(
            lex("16r1.8"),
            vec![TokenKind::IntLit(1), TokenKind::RealLit(0.8)]
        );
    }

    /// The escape table is shared by string and character constants
    /// (`escmap`, lex.c:167-182), so a control escape has to produce the
    /// control code in both. `'\b'` used to lex as the letter `b` (0x62), which
    /// silently changed programs such as appl/cmd/sed.b:550.
    #[test]
    fn char_literal_control_escapes() {
        let table: &[(&str, i32)] = &[
            (r"'\n'", 0x0A),
            (r"'\t'", 0x09),
            (r"'\r'", 0x0D),
            (r"'\a'", 0x07),
            (r"'\b'", 0x08),
            (r"'\f'", 0x0C),
            (r"'\v'", 0x0B),
            (r"'\0'", 0x00),
            (r"'\\'", 0x5C),
            (r"'\''", 0x27),
            (r#"'\"'"#, 0x22),
        ];
        for (src, want) in table {
            assert_eq!(lex(src), vec![TokenKind::CharLit(*want)], "literal {src}");
        }
    }

    #[test]
    fn string_control_escapes() {
        let tokens = lex(r#""\n\t\r\a\b\f\v\0\\\"\'""#);
        assert_eq!(
            tokens,
            vec![TokenKind::StringLit(
                "\u{0A}\u{09}\u{0D}\u{07}\u{08}\u{0C}\u{0B}\u{00}\\\"'".to_string()
            )]
        );
    }

    /// An escape the table does not name keeps the escaped character. The
    /// reference reports it and then returns the same character (lex.c:810-812),
    /// so the value agrees even though this front end does not diagnose it.
    #[test]
    fn unknown_escape_keeps_the_escaped_character() {
        assert_eq!(lex(r#""\q""#), vec![TokenKind::StringLit("q".to_string())]);
        assert_eq!(lex(r"'\q'"), vec![TokenKind::CharLit(b'q' as i32)]);
    }

    #[test]
    fn unicode_escapes() {
        assert_eq!(
            lex(r#""Aé€""#),
            vec![TokenKind::StringLit("A\u{E9}\u{20AC}".to_string())]
        );
        assert_eq!(lex(r"'é'"), vec![TokenKind::CharLit(0xE9)]);
        assert_eq!(lex(r"'€'"), vec![TokenKind::CharLit(0x20AC)]);
        assert_eq!(lex(r"'￿'"), vec![TokenKind::CharLit(0xFFFF)]);
    }

    #[test]
    fn malformed_unicode_escape_is_error() {
        // A short or non-hex \u escape is reported, not silently truncated.
        for src in [r#""\uZZZZ""#, r#""\u12""#, r#""\u""#, r"'\u00'", r"'\u'"] {
            let msg = lex_err(src);
            assert!(
                msg.contains("hex escape"),
                "unexpected message for {src}: {msg}"
            );
        }
    }

    /// A `\u` escape that names half of a surrogate pair is not a Rust scalar,
    /// so it contributes nothing. The point of the test is that the lexer keeps
    /// going instead of panicking.
    #[test]
    fn lone_surrogate_escape_is_dropped() {
        assert_eq!(
            lex(r#""a\ud800b""#),
            vec![TokenKind::StringLit("ab".to_string())]
        );
    }

    #[test]
    fn multibyte_utf8_in_string_and_char_literals() {
        // Two-, three-, and four-byte scalars all keep their own code point.
        assert_eq!(
            lex("\"a\u{E9}\u{20AC}\u{1F600}b\""),
            vec![TokenKind::StringLit(
                "a\u{E9}\u{20AC}\u{1F600}b".to_string()
            )]
        );
        assert_eq!(
            lex("'\u{1F600}'"),
            vec![TokenKind::CharLit(0x1F600)],
            "a four-byte scalar is one character"
        );
    }

    #[test]
    fn unterminated_string_is_error() {
        for src in ["\"abc", "\""] {
            let msg = lex_err(src);
            assert!(
                msg.contains("unterminated string"),
                "unexpected message for {src}: {msg}"
            );
        }
    }

    #[test]
    fn unterminated_char_literal_is_error() {
        // The reference reports the missing quote (lex.c:930-934).
        for src in ["'a", "'", r"'\n"] {
            let msg = lex_err(src);
            assert!(
                msg.contains("missing closing"),
                "unexpected message for {src}: {msg}"
            );
        }
    }

    /// A newline inside a string is a reference error ("newline in string
    /// constant", lex.c:876-882). This front end carries it through instead,
    /// which accepts a superset; the value is what matters here.
    #[test]
    fn newline_inside_string_is_kept() {
        assert_eq!(
            lex("\"a\nb\""),
            vec![TokenKind::StringLit("a\nb".to_string())]
        );
    }

    /// A scalar outside ASCII is an identifier character, which is what lets
    /// appl/spree/lib/testsets.b:18 declare `∈: Set;` and appl/wm/c4.b:387 pass
    /// `∞` as an argument.
    #[test]
    fn non_ascii_identifiers() {
        assert_eq!(
            lex("∈: Set;"),
            vec![
                TokenKind::Ident("∈".to_string()),
                TokenKind::Colon,
                TokenKind::Ident("Set".to_string()),
                TokenKind::Semicolon,
            ]
        );
        assert_eq!(
            lex("minimax(me, ∞)"),
            vec![
                TokenKind::Ident("minimax".to_string()),
                TokenKind::LParen,
                TokenKind::Ident("me".to_string()),
                TokenKind::Comma,
                TokenKind::Ident("∞".to_string()),
                TokenKind::RParen,
            ]
        );
        // A non-ASCII scalar also continues an identifier that starts in ASCII.
        assert_eq!(
            lex("aé1"),
            vec![TokenKind::Ident("aé1".to_string())],
            "a non-ASCII scalar continues an identifier"
        );
    }

    #[test]
    fn unexpected_character_is_error() {
        for src in ["@", "$", "?", "`", "\\"] {
            let msg = lex_err(src);
            assert!(
                msg.contains("unexpected character"),
                "unexpected message for {src}: {msg}"
            );
        }
    }

    /// Every operator, checked against the longest match the reference takes
    /// (lex.c:100-133 and the per-character cases from lex.c:936 onward).
    #[test]
    fn operators_take_the_longest_match() {
        let table: &[(&str, &[TokenKind])] = &[
            ("+", &[TokenKind::Plus]),
            ("++", &[TokenKind::Inc]),
            ("+++", &[TokenKind::Inc, TokenKind::Plus]),
            ("+=", &[TokenKind::PlusEq]),
            ("-", &[TokenKind::Minus]),
            ("--", &[TokenKind::Dec]),
            ("-=", &[TokenKind::MinusEq]),
            ("->", &[TokenKind::Arrow]),
            ("*", &[TokenKind::Star]),
            ("**", &[TokenKind::Power]),
            ("*=", &[TokenKind::StarEq]),
            ("/", &[TokenKind::Slash]),
            ("/=", &[TokenKind::SlashEq]),
            ("%", &[TokenKind::Percent]),
            ("%=", &[TokenKind::PercentEq]),
            ("&", &[TokenKind::Amp]),
            ("&&", &[TokenKind::AndAnd]),
            ("&=", &[TokenKind::AmpEq]),
            ("|", &[TokenKind::Pipe]),
            ("||", &[TokenKind::OrOr]),
            ("|=", &[TokenKind::PipeEq]),
            ("^", &[TokenKind::Caret]),
            ("^=", &[TokenKind::CaretEq]),
            ("~", &[TokenKind::Tilde]),
            ("!", &[TokenKind::Bang]),
            ("!=", &[TokenKind::Neq]),
            ("<", &[TokenKind::Lt]),
            ("<=", &[TokenKind::Leq]),
            ("<<", &[TokenKind::Lshift]),
            ("<<=", &[TokenKind::LshiftEq]),
            ("<-", &[TokenKind::ChanRecv]),
            ("<-=", &[TokenKind::ChanSend]),
            (">", &[TokenKind::Gt]),
            (">=", &[TokenKind::Geq]),
            (">>", &[TokenKind::Rshift]),
            (">>=", &[TokenKind::RshiftEq]),
            ("=", &[TokenKind::Assign]),
            ("==", &[TokenKind::Eq]),
            ("=>", &[TokenKind::FatArrow]),
            (":", &[TokenKind::Colon]),
            (":=", &[TokenKind::ColonEq]),
            ("::", &[TokenKind::ColonColon]),
            (":::", &[TokenKind::ColonColon, TokenKind::Colon]),
            ("(", &[TokenKind::LParen]),
            (")", &[TokenKind::RParen]),
            ("[", &[TokenKind::LBracket]),
            ("]", &[TokenKind::RBracket]),
            ("{", &[TokenKind::LBrace]),
            ("}", &[TokenKind::RBrace]),
            (",", &[TokenKind::Comma]),
            (".", &[TokenKind::Dot]),
            (";", &[TokenKind::Semicolon]),
        ];
        for (src, want) in table {
            assert_eq!(lex(src), want.to_vec(), "operator {src}");
        }
    }

    /// The reference lexes `**=` as one token (`Lexpeq`, lex.c:113) and its
    /// grammar has `exp Lexpeq exp` (limbo.y:1123). This front end has no such
    /// token, so `x **= 2` splits and the parser rejects it. Recorded so the
    /// gap is visible rather than mistaken for support.
    #[test]
    fn power_assign_is_not_one_token() {
        assert_eq!(lex("**="), vec![TokenKind::Power, TokenKind::Assign]);
    }

    #[test]
    fn spans_track_lines_and_columns() {
        let tokens = Lexer::new("x\n  y # note\n\tz", "<test>")
            .tokenize()
            .expect("lex should succeed");
        let places: Vec<(u32, u32)> = tokens.iter().map(|t| (t.span.line, t.span.col)).collect();
        assert_eq!(places, vec![(1, 1), (2, 3), (3, 2), (3, 3)]);
        assert_eq!(tokens[3].kind, TokenKind::Eof);
    }

    #[test]
    fn multi_line_literal_updates_the_next_span() {
        // A newline inside a string still advances the line counter.
        let tokens = Lexer::new("\"a\nb\" c", "<test>")
            .tokenize()
            .expect("lex should succeed");
        assert_eq!((tokens[0].span.line, tokens[0].span.col), (1, 1));
        assert_eq!((tokens[1].span.line, tokens[1].span.col), (2, 4));
    }

    #[test]
    fn comments_and_whitespace_are_skipped() {
        assert_eq!(lex("# only a comment"), vec![]);
        assert_eq!(lex("x # trailing comment"), vec![lex_ident("x")]);
        assert_eq!(lex("# one\n# two\nx"), vec![lex_ident("x")]);
        assert_eq!(lex("\t \r\n x \n"), vec![lex_ident("x")]);
        assert_eq!(lex("x#c1\n#c2\ny"), vec![lex_ident("x"), lex_ident("y")]);
    }

    fn lex_ident(name: &str) -> TokenKind {
        TokenKind::Ident(name.to_string())
    }

    #[test]
    fn empty_input_is_a_single_eof() {
        let tokens = Lexer::new("", "<test>")
            .tokenize()
            .expect("lex should succeed");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenKind::Eof);
    }

    #[test]
    fn tokenize_appends_exactly_one_eof() {
        let tokens = Lexer::new("a b c", "<test>")
            .tokenize()
            .expect("lex should succeed");
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[3].kind, TokenKind::Eof);
        assert_eq!(
            tokens.iter().filter(|t| t.kind == TokenKind::Eof).count(),
            1
        );
    }

    #[test]
    fn error_carries_the_file_name_and_place() {
        let err = Lexer::new("x\n  @", "prog.b")
            .tokenize()
            .expect_err("lex should fail");
        assert_eq!(err.file, "prog.b");
        assert_eq!((err.span.line, err.span.col), (2, 3));
        assert_eq!(err.to_string(), "prog.b:2:3: unexpected character: '@'");
    }
}
