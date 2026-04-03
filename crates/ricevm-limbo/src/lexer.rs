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
        LexError {
            file: self.file.clone(),
            span: self.span(),
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

        // Identifiers and keywords
        if ch.is_ascii_alphabetic() || ch == b'_' {
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
            && (self.peek().is_ascii_alphanumeric() || self.peek() == b'_')
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
        let mut is_float = false;
        if self.peek() == b'.' && !self.peek2().is_ascii_alphabetic() && self.peek2() != b'.' {
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
            let ch = self.advance();
            if ch == b'"' {
                break;
            }
            if ch == b'\\' {
                let esc = self.advance();
                match esc {
                    b'n' => s.push('\n'),
                    b't' => s.push('\t'),
                    b'r' => s.push('\r'),
                    b'\\' => s.push('\\'),
                    b'"' => s.push('"'),
                    b'\'' => s.push('\''),
                    b'0' => s.push('\0'),
                    b'a' => s.push('\x07'),
                    b'b' => s.push('\x08'),
                    b'f' => s.push('\x0C'),
                    b'v' => s.push('\x0B'),
                    b'u' => {
                        let val = self.lex_hex_escape(4)?;
                        if let Some(c) = char::from_u32(val) {
                            s.push(c);
                        }
                    }
                    _ => {
                        s.push(esc as char);
                    }
                }
            } else {
                s.push(ch as char);
            }
        }
        Ok(Token {
            kind: TokenKind::StringLit(s),
            span,
        })
    }

    fn lex_char(&mut self, span: Span) -> Result<Token, LexError> {
        self.advance(); // skip opening '
        let val = if self.peek() == b'\\' {
            self.advance();
            let esc = self.advance();
            match esc {
                b'n' => b'\n' as i32,
                b't' => b'\t' as i32,
                b'r' => b'\r' as i32,
                b'\\' => b'\\' as i32,
                b'\'' => b'\'' as i32,
                b'0' => 0,
                b'u' => self.lex_hex_escape(4)? as i32,
                _ => esc as i32,
            }
        } else {
            // Handle UTF-8 character
            let start = self.pos;
            self.advance();
            // Check for multi-byte UTF-8
            let slice = &self.src[start..self.pos.min(self.src.len())];
            if let Ok(s) = std::str::from_utf8(slice) {
                s.chars().next().map(|c| c as i32).unwrap_or(0)
            } else {
                // Try reading more bytes for multi-byte chars
                let end = (start + 4).min(self.src.len());
                if let Ok(s) = std::str::from_utf8(&self.src[start..end]) {
                    if let Some(c) = s.chars().next() {
                        // Advance past the remaining bytes
                        let extra = c.len_utf8() - 1;
                        for _ in 0..extra {
                            self.advance();
                        }
                        c as i32
                    } else {
                        0
                    }
                } else {
                    self.src[start] as i32
                }
            }
        };
        if self.peek() == b'\'' {
            self.advance();
        }
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
            _ => return Err(self.err(format!("unexpected character: {:?}", ch as char))),
        };
        Ok(Token { kind, span })
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
        let tokens = lex("3.14 1e10 2.5e-3");
        assert_eq!(
            tokens,
            vec![
                TokenKind::RealLit(3.14),
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
}
