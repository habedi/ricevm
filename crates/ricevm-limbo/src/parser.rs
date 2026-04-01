//! Recursive descent parser for Limbo.
//!
//! Produces an AST from a token stream. Uses Pratt parsing (operator
//! precedence climbing) for expressions.

use crate::ast::*;
use crate::token::{Span, Token, TokenKind};

/// Parser error with source location.
#[derive(Clone, Debug, thiserror::Error)]
#[error("{file}:{}:{}: {message}", span.line, span.col)]
pub struct ParseError {
    pub file: String,
    pub span: Span,
    pub message: String,
}

/// Parser state.
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    file: String,
}

impl Parser {
    pub fn new(tokens: Vec<Token>, file: &str) -> Self {
        Self {
            tokens,
            pos: 0,
            file: file.to_string(),
        }
    }

    // ── Helpers ─────────────────────────────────────────────────

    fn span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .map(|t| t.span)
            .unwrap_or_default()
    }

    fn peek(&self) -> &TokenKind {
        self.tokens
            .get(self.pos)
            .map(|t| &t.kind)
            .unwrap_or(&TokenKind::Eof)
    }

    fn at(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(self.peek()) == std::mem::discriminant(kind)
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos];
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, kind: &TokenKind) -> Result<&Token, ParseError> {
        if self.at(kind) {
            Ok(self.advance())
        } else {
            Err(self.err(format!("expected {kind:?}, got {:?}", self.peek())))
        }
    }

    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.peek().clone() {
            TokenKind::Ident(name) => {
                self.advance();
                Ok(name)
            }
            _ => Err(self.err(format!("expected identifier, got {:?}", self.peek()))),
        }
    }

    fn expect_semi(&mut self) -> Result<(), ParseError> {
        self.expect(&TokenKind::Semicolon)?;
        Ok(())
    }

    fn err(&self, msg: impl Into<String>) -> ParseError {
        ParseError {
            file: self.file.clone(),
            span: self.span(),
            message: msg.into(),
        }
    }

    // ── Top Level ──────────────────────────────────────────────

    /// Parse a complete Limbo source file.
    pub fn parse_file(&mut self) -> Result<SourceFile, ParseError> {
        let mut implement = Vec::new();
        let mut includes = Vec::new();
        let mut decls = Vec::new();

        // Optional: implement Name, Name2, ...;
        if self.at(&TokenKind::Implement) {
            self.advance();
            loop {
                implement.push(self.expect_ident()?);
                if !self.at(&TokenKind::Comma) {
                    break;
                }
                self.advance();
            }
            self.expect_semi()?;
        }

        // Parse top-level items
        while !self.at(&TokenKind::Eof) {
            if self.at(&TokenKind::Include) {
                let span = self.span();
                self.advance();
                let path = match self.peek().clone() {
                    TokenKind::StringLit(s) => {
                        self.advance();
                        s
                    }
                    _ => return Err(self.err("expected string after include")),
                };
                self.expect_semi()?;
                includes.push(Include { path, span });
            } else {
                match self.parse_top_decl() {
                    Ok(d) => decls.push(d),
                    Err(e) => {
                        // Try to recover by skipping to next semicolon
                        if self.at(&TokenKind::Eof) {
                            return Err(e);
                        }
                        return Err(e);
                    }
                }
            }
        }

        Ok(SourceFile {
            implement,
            includes,
            decls,
        })
    }

    /// Parse a top-level declaration: variable, constant, type, module, adt, function, or import.
    fn parse_top_decl(&mut self) -> Result<Decl, ParseError> {
        let span = self.span();

        // Function definition: name(args) or Qualifier.name(args)
        // We need to look ahead to distinguish name: type from name(args)
        if let TokenKind::Ident(_) = self.peek() {
            // Look ahead: could be name(, name., name:, name,
            let la = self.look_ahead_after_ident();
            match la {
                LookAhead::FuncDef => return self.parse_func_def(),
                LookAhead::ColonDecl => return self.parse_colon_decl(span),
                LookAhead::Assign => return self.parse_top_assign(span),
                LookAhead::DeclAssign => return self.parse_top_decl_assign(span),
            }
        }

        Err(self.err(format!("unexpected token at top level: {:?}", self.peek())))
    }

    /// Determine what follows an identifier at the top level.
    fn look_ahead_after_ident(&self) -> LookAhead {
        let mut i = self.pos + 1;
        // Skip past qualified names: A.B.C
        while i < self.tokens.len() {
            match &self.tokens[i].kind {
                TokenKind::Dot => {
                    i += 1; // skip dot
                    if i < self.tokens.len() && matches!(self.tokens[i].kind, TokenKind::Ident(_)) {
                        i += 1; // skip ident after dot
                    }
                }
                TokenKind::LParen => return LookAhead::FuncDef,
                TokenKind::LBracket => {
                    // Skip polymorphic params: name[T1, T2]
                    i += 1;
                    while i < self.tokens.len() && self.tokens[i].kind != TokenKind::RBracket {
                        i += 1;
                    }
                    if i < self.tokens.len() {
                        i += 1;
                    } // skip ]
                }
                TokenKind::Colon => return LookAhead::ColonDecl,
                TokenKind::Comma => return LookAhead::ColonDecl,
                TokenKind::Assign => return LookAhead::Assign,
                TokenKind::ColonEq => return LookAhead::DeclAssign,
                _ => break,
            }
        }
        LookAhead::ColonDecl
    }

    // ── Declarations ───────────────────────────────────────────

    /// Parse `names : <type|con|module|adt|import|exception> ...;`
    fn parse_colon_decl(&mut self, span: Span) -> Result<Decl, ParseError> {
        // Parse one or more names
        let mut names = vec![self.expect_ident()?];
        while self.at(&TokenKind::Comma) {
            self.advance();
            names.push(self.expect_ident()?);
        }
        self.expect(&TokenKind::Colon)?;

        match self.peek() {
            TokenKind::Con => {
                self.advance();
                let value = self.parse_expr()?;
                self.expect_semi()?;
                Ok(Decl::Const(ConstDecl {
                    name: names.into_iter().next().unwrap_or_default(),
                    ty: None,
                    value,
                    span,
                }))
            }
            TokenKind::Type => {
                self.advance();
                let ty = self.parse_type()?;
                self.expect_semi()?;
                Ok(Decl::TypeAlias(TypeAliasDecl {
                    name: names.into_iter().next().unwrap_or_default(),
                    ty,
                    span,
                }))
            }
            TokenKind::Module => {
                self.advance();
                self.expect(&TokenKind::LBrace)?;
                let members = self.parse_module_members()?;
                self.expect(&TokenKind::RBrace)?;
                self.expect_semi()?;
                Ok(Decl::Module(ModuleDecl {
                    name: names.into_iter().next().unwrap_or_default(),
                    members,
                    span,
                }))
            }
            TokenKind::Adt => {
                self.advance();
                // Skip optional polymorphic type parameters: [T1, T2]
                if self.at(&TokenKind::LBracket) {
                    self.advance();
                    while !self.at(&TokenKind::RBracket) && !self.at(&TokenKind::Eof) {
                        self.advance();
                    }
                    if self.at(&TokenKind::RBracket) {
                        self.advance();
                    }
                }
                // Skip optional 'for { ... }' clause
                if self.at(&TokenKind::For) {
                    self.advance();
                    self.expect(&TokenKind::LBrace)?;
                    while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
                        self.advance();
                    }
                    if self.at(&TokenKind::RBrace) {
                        self.advance();
                    }
                }
                self.expect(&TokenKind::LBrace)?;
                let (members, pick) = self.parse_adt_members()?;
                self.expect(&TokenKind::RBrace)?;
                self.expect_semi()?;
                Ok(Decl::Adt(AdtDecl {
                    name: names.into_iter().next().unwrap_or_default(),
                    members,
                    pick,
                    span,
                }))
            }
            TokenKind::Import => {
                self.advance();
                let module = self.expect_ident()?;
                self.expect_semi()?;
                Ok(Decl::Import(ImportDecl {
                    names,
                    module,
                    span,
                }))
            }
            TokenKind::Exception => {
                self.advance();
                let ty = if self.at(&TokenKind::LParen) {
                    self.advance();
                    let t = self.parse_type()?;
                    self.expect(&TokenKind::RParen)?;
                    Some(t)
                } else {
                    None
                };
                self.expect_semi()?;
                Ok(Decl::Exception(ExceptionDecl {
                    name: names.into_iter().next().unwrap_or_default(),
                    ty,
                    span,
                }))
            }
            _ => {
                // Variable declaration: names : type [= expr];
                let ty = self.parse_type()?;
                let init = if self.at(&TokenKind::Assign) {
                    self.advance();
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                self.expect_semi()?;
                Ok(Decl::Var(VarDecl {
                    names,
                    ty: Some(ty),
                    init,
                    span,
                }))
            }
        }
    }

    /// Parse `name = expr;` at top level.
    fn parse_top_assign(&mut self, span: Span) -> Result<Decl, ParseError> {
        let name = self.expect_ident()?;
        self.expect(&TokenKind::Assign)?;
        let init = self.parse_expr()?;
        self.expect_semi()?;
        Ok(Decl::Var(VarDecl {
            names: vec![name],
            ty: None,
            init: Some(init),
            span,
        }))
    }

    /// Parse `name := expr;` at top level.
    fn parse_top_decl_assign(&mut self, span: Span) -> Result<Decl, ParseError> {
        let name = self.expect_ident()?;
        self.expect(&TokenKind::ColonEq)?;
        let init = self.parse_expr()?;
        self.expect_semi()?;
        Ok(Decl::Var(VarDecl {
            names: vec![name],
            ty: None,
            init: Some(init),
            span,
        }))
    }

    /// Parse module members inside { ... }.
    fn parse_module_members(&mut self) -> Result<Vec<ModuleMember>, ParseError> {
        let mut members = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            let span = self.span();
            let name = self.expect_ident()?;
            // Check for multiple names: a, b, c : type;
            let mut names = vec![name];
            while self.at(&TokenKind::Comma) {
                self.advance();
                names.push(self.expect_ident()?);
            }
            self.expect(&TokenKind::Colon)?;

            match self.peek() {
                TokenKind::Con => {
                    self.advance();
                    let value = self.parse_expr()?;
                    self.expect_semi()?;
                    members.push(ModuleMember::Const(ConstDecl {
                        name: names.into_iter().next().unwrap_or_default(),
                        ty: None,
                        value,
                        span,
                    }));
                }
                TokenKind::Type => {
                    self.advance();
                    let ty = self.parse_type()?;
                    self.expect_semi()?;
                    members.push(ModuleMember::TypeAlias(TypeAliasDecl {
                        name: names.into_iter().next().unwrap_or_default(),
                        ty,
                        span,
                    }));
                }
                TokenKind::Fn => {
                    let sig = self.parse_func_sig(names.into_iter().next().unwrap_or_default())?;
                    self.expect_semi()?;
                    members.push(ModuleMember::Func(sig));
                }
                TokenKind::Adt => {
                    self.advance();
                    self.expect(&TokenKind::LBrace)?;
                    let (adt_members, pick) = self.parse_adt_members()?;
                    self.expect(&TokenKind::RBrace)?;
                    self.expect_semi()?;
                    members.push(ModuleMember::Adt(AdtDecl {
                        name: names.into_iter().next().unwrap_or_default(),
                        members: adt_members,
                        pick,
                        span,
                    }));
                }
                _ => {
                    // Variable: type ;
                    let ty = self.parse_type()?;
                    self.expect_semi()?;
                    members.push(ModuleMember::Var(VarDecl {
                        names,
                        ty: Some(ty),
                        init: None,
                        span,
                    }));
                }
            }
        }
        Ok(members)
    }

    /// Parse ADT members and optional pick clause.
    fn parse_adt_members(&mut self) -> Result<(Vec<AdtMember>, Option<Vec<PickCase>>), ParseError> {
        let mut members = Vec::new();
        let mut pick = None;

        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            if self.at(&TokenKind::Pick) {
                self.advance();
                self.expect(&TokenKind::LBrace)?;
                pick = Some(self.parse_pick_cases()?);
                self.expect(&TokenKind::RBrace)?;
                continue;
            }

            let span = self.span();
            let name = self.expect_ident()?;
            let mut names = vec![name];
            while self.at(&TokenKind::Comma) {
                self.advance();
                names.push(self.expect_ident()?);
            }
            self.expect(&TokenKind::Colon)?;

            match self.peek() {
                TokenKind::Con => {
                    self.advance();
                    let value = self.parse_expr()?;
                    self.expect_semi()?;
                    members.push(AdtMember::Const(ConstDecl {
                        name: names.into_iter().next().unwrap_or_default(),
                        ty: None,
                        value,
                        span,
                    }));
                }
                TokenKind::Fn => {
                    let sig = self.parse_func_sig(names.into_iter().next().unwrap_or_default())?;
                    self.expect_semi()?;
                    members.push(AdtMember::Func(sig));
                }
                _ => {
                    let mut is_cyclic = false;
                    if self.at(&TokenKind::Cyclic) {
                        self.advance();
                        is_cyclic = true;
                    }
                    let _ = is_cyclic; // TODO: track cyclic in AST
                    let ty = self.parse_type()?;
                    self.expect_semi()?;
                    members.push(AdtMember::Field(VarDecl {
                        names,
                        ty: Some(ty),
                        init: None,
                        span,
                    }));
                }
            }
        }
        Ok((members, pick))
    }

    fn parse_pick_cases(&mut self) -> Result<Vec<PickCase>, ParseError> {
        let mut cases = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            let mut tags = vec![self.expect_ident()?];
            while self.at(&TokenKind::Or) {
                self.advance();
                tags.push(self.expect_ident()?);
            }
            self.expect(&TokenKind::FatArrow)?;
            let mut fields = Vec::new();
            while !self.at(&TokenKind::RBrace)
                && !self.at(&TokenKind::Eof)
                && !matches!(self.peek(), TokenKind::Ident(_))
                || self.is_field_start()
            {
                let span = self.span();
                let name = self.expect_ident()?;
                let mut names = vec![name];
                while self.at(&TokenKind::Comma) {
                    self.advance();
                    names.push(self.expect_ident()?);
                }
                self.expect(&TokenKind::Colon)?;
                let ty = self.parse_type()?;
                self.expect_semi()?;
                fields.push(VarDecl {
                    names,
                    ty: Some(ty),
                    init: None,
                    span,
                });
                // Check if next token starts a new tag (identifier followed by => or 'or')
                if self.is_pick_tag_start() {
                    break;
                }
            }
            cases.push(PickCase { tags, fields });
        }
        Ok(cases)
    }

    fn is_field_start(&self) -> bool {
        // A field starts with an identifier followed by ',' or ':'
        if let TokenKind::Ident(_) = self.peek() {
            let next = self.pos + 1;
            if next < self.tokens.len() {
                matches!(self.tokens[next].kind, TokenKind::Comma | TokenKind::Colon)
            } else {
                false
            }
        } else {
            false
        }
    }

    fn is_pick_tag_start(&self) -> bool {
        if let TokenKind::Ident(_) = self.peek() {
            let mut i = self.pos + 1;
            while i < self.tokens.len() {
                match &self.tokens[i].kind {
                    TokenKind::Or => {
                        i += 1;
                        if i < self.tokens.len()
                            && matches!(self.tokens[i].kind, TokenKind::Ident(_))
                        {
                            i += 1;
                        }
                    }
                    TokenKind::FatArrow => return true,
                    _ => return false,
                }
            }
        }
        false
    }

    // ── Functions ──────────────────────────────────────────────

    /// Parse a function definition: name.name(args): ret { body }
    fn parse_func_def(&mut self) -> Result<Decl, ParseError> {
        let span = self.span();
        let mut qualifier = None;
        let mut name = self.expect_ident()?;

        // Skip optional polymorphic params: func[T1, T2]
        if self.at(&TokenKind::LBracket) {
            self.advance();
            while !self.at(&TokenKind::RBracket) && !self.at(&TokenKind::Eof) {
                self.advance();
            }
            if self.at(&TokenKind::RBracket) {
                self.advance();
            }
        }

        // Qualified name: A.B(
        while self.at(&TokenKind::Dot) {
            self.advance();
            qualifier = Some(name);
            name = self.expect_ident()?;
            // Skip polymorphic params after qualifier
            if self.at(&TokenKind::LBracket) {
                self.advance();
                while !self.at(&TokenKind::RBracket) && !self.at(&TokenKind::Eof) {
                    self.advance();
                }
                if self.at(&TokenKind::RBracket) {
                    self.advance();
                }
            }
        }

        let sig = self.parse_func_sig(name.clone())?;
        let body = self.parse_block()?;

        Ok(Decl::Func(FuncDecl {
            name: QualName { qualifier, name },
            sig,
            body,
            span,
        }))
    }

    /// Parse function signature: fn(params): rettype
    fn parse_func_sig(&mut self, name: String) -> Result<FuncSig, ParseError> {
        let span = self.span();

        // Could start with 'fn' keyword or directly with '('
        if self.at(&TokenKind::Fn) {
            self.advance();
            // Skip optional polymorphic params after fn keyword
            if self.at(&TokenKind::LBracket) {
                self.advance();
                while !self.at(&TokenKind::RBracket) && !self.at(&TokenKind::Eof) {
                    self.advance();
                }
                if self.at(&TokenKind::RBracket) {
                    self.advance();
                }
            }
        }

        self.expect(&TokenKind::LParen)?;
        let params = if self.at(&TokenKind::Star) {
            self.advance(); // varargs: fn(*)
            Vec::new()
        } else {
            let p = self.parse_params()?;
            // Skip trailing varargs: , *
            if self.at(&TokenKind::Comma) {
                let saved = self.pos;
                self.advance();
                if self.at(&TokenKind::Star) {
                    self.advance(); // consume *
                } else {
                    self.pos = saved; // restore
                }
            }
            p
        };
        self.expect(&TokenKind::RParen)?;

        let ret = if self.at(&TokenKind::Colon) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        // Skip optional 'raises (exceptions)' or 'raises ExcName' clause
        if self.at(&TokenKind::Raise) || matches!(self.peek(), TokenKind::Ident(n) if n == "raises")
        {
            self.advance();
            if self.at(&TokenKind::LParen) {
                self.advance();
                while !self.at(&TokenKind::RParen) && !self.at(&TokenKind::Eof) {
                    self.advance();
                }
                if self.at(&TokenKind::RParen) {
                    self.advance();
                }
            } else if let TokenKind::Ident(_) = self.peek() {
                self.advance(); // consume exception name
            }
        }

        // Skip optional 'for { ... }' polymorphic clause
        if self.at(&TokenKind::For) {
            self.advance();
            if self.at(&TokenKind::LBrace) {
                self.advance();
                while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
                    self.advance();
                }
                if self.at(&TokenKind::RBrace) {
                    self.advance();
                }
            }
        }

        Ok(FuncSig {
            name,
            params,
            ret,
            span,
        })
    }

    fn parse_params(&mut self) -> Result<Vec<Param>, ParseError> {
        let mut params = Vec::new();
        if self.at(&TokenKind::RParen) {
            return Ok(params);
        }

        loop {
            let is_self = if self.at(&TokenKind::Self_) {
                self.advance();
                true
            } else {
                false
            };

            // Check for varargs: *, in param list
            if self.at(&TokenKind::Star) {
                self.advance();
                break; // varargs ends the param list
            }

            // Parse parameter names
            let mut names = Vec::new();
            let is_nil = self.at(&TokenKind::Nil);
            if is_nil {
                self.advance();
                names.push("nil".to_string());
            } else {
                names.push(self.expect_ident()?);
            }
            while self.at(&TokenKind::Comma) && !self.is_param_type_next() {
                self.advance();
                if self.at(&TokenKind::Nil) {
                    self.advance();
                    names.push("nil".to_string());
                } else {
                    names.push(self.expect_ident()?);
                }
            }

            // Expect : type
            self.expect(&TokenKind::Colon)?;
            let ty = self.parse_type()?;

            params.push(Param {
                names,
                ty,
                is_self,
                is_nil,
            });

            if !self.at(&TokenKind::Comma) {
                break;
            }
            self.advance();
            if self.at(&TokenKind::RParen) {
                break;
            }
        }
        Ok(params)
    }

    /// Check if the next comma starts a new parameter group.
    /// In Limbo, `a, b: int` groups a and b with int, while `a: int, b: string` are separate.
    /// We distinguish by looking ahead: if ident is followed by ':' and then a type keyword,
    /// but there's another comma+ident before the colon, it's still the same group.
    fn is_param_type_next(&self) -> bool {
        // After a comma, look ahead for the pattern: ident ':' type
        // But also check: could there be more names before the colon?
        // e.g., (a, b, c: int) — all three share the type
        // vs (a: int, b: string) — separate groups
        let mut i = self.pos + 1; // past the comma
        // Skip identifiers and commas to find the colon
        while i < self.tokens.len() {
            match &self.tokens[i].kind {
                TokenKind::Ident(_) | TokenKind::Nil => {
                    i += 1;
                    if i < self.tokens.len() && self.tokens[i].kind == TokenKind::Comma {
                        i += 1; // more names follow
                        continue;
                    }
                    if i < self.tokens.len() && self.tokens[i].kind == TokenKind::Colon {
                        // Found the colon — this is the end of the name list
                        return false; // names share the type after colon
                    }
                    return true; // no colon found, must be a new group
                }
                TokenKind::Self_ => return true, // self always starts new group
                _ => return true,
            }
        }
        true
    }

    // ── Types ──────────────────────────────────────────────────

    fn parse_type(&mut self) -> Result<Type, ParseError> {
        let base = match self.peek().clone() {
            TokenKind::Int => {
                self.advance();
                Type::Basic(BasicType::Int)
            }
            TokenKind::Byte => {
                self.advance();
                Type::Basic(BasicType::Byte)
            }
            TokenKind::Big => {
                self.advance();
                Type::Basic(BasicType::Big)
            }
            TokenKind::Real => {
                self.advance();
                Type::Basic(BasicType::Real)
            }
            TokenKind::String_ => {
                self.advance();
                Type::Basic(BasicType::String)
            }
            TokenKind::Array => {
                self.advance();
                self.expect(&TokenKind::Of)?;
                let elem = self.parse_type()?;
                Type::Array(Box::new(elem))
            }
            TokenKind::List => {
                self.advance();
                self.expect(&TokenKind::Of)?;
                let elem = self.parse_type()?;
                Type::List(Box::new(elem))
            }
            TokenKind::Chan => {
                self.advance();
                self.expect(&TokenKind::Of)?;
                let elem = self.parse_type()?;
                Type::Chan(Box::new(elem))
            }
            TokenKind::Ref => {
                self.advance();
                let inner = self.parse_type()?;
                Type::Ref(Box::new(inner))
            }
            TokenKind::Fn => {
                self.advance();
                let sig = self.parse_func_sig(String::new())?;
                Type::Func(Box::new(sig))
            }
            TokenKind::LParen => {
                self.advance();
                let first = self.parse_type()?;
                if self.at(&TokenKind::Comma) {
                    let mut types = vec![first];
                    while self.at(&TokenKind::Comma) {
                        self.advance();
                        types.push(self.parse_type()?);
                    }
                    self.expect(&TokenKind::RParen)?;
                    Type::Tuple(types)
                } else {
                    self.expect(&TokenKind::RParen)?;
                    first
                }
            }
            TokenKind::Ident(name) => {
                self.advance();
                // Skip optional polymorphic params: Type[T1, T2]
                if self.at(&TokenKind::LBracket) {
                    self.advance();
                    while !self.at(&TokenKind::RBracket) && !self.at(&TokenKind::Eof) {
                        self.advance();
                    }
                    if self.at(&TokenKind::RBracket) {
                        self.advance();
                    }
                }
                // Check for Module->Type or Type.SubType
                if self.at(&TokenKind::Arrow) || self.at(&TokenKind::Dot) {
                    self.advance();
                    let member = self.expect_ident()?;
                    Type::Named(QualName {
                        qualifier: Some(name),
                        name: member,
                    })
                } else {
                    Type::Named(QualName {
                        qualifier: None,
                        name,
                    })
                }
            }
            TokenKind::Self_ => {
                self.advance();
                let inner = self.parse_type()?;
                Type::Ref(Box::new(inner)) // self Type is sugar for ref Type
            }
            _ => {
                return Err(self.err(format!("expected type, got {:?}", self.peek())));
            }
        };
        Ok(base)
    }

    // ── Statements ─────────────────────────────────────────────

    fn parse_block(&mut self) -> Result<Block, ParseError> {
        let span = self.span();
        self.expect(&TokenKind::LBrace)?;
        let mut stmts = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            stmts.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(Block { stmts, span })
    }

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        let span = self.span();

        match self.peek() {
            TokenKind::LBrace => {
                let block = self.parse_block()?;
                // Check for exception handler: { ... } exception [id] { ... }
                if self.at(&TokenKind::Exception) {
                    self.advance();
                    // Optional exception variable name
                    if let TokenKind::Ident(_) = self.peek() {
                        self.advance();
                    }
                    // Parse exception handler body: { pattern => stmts; ... }
                    self.expect(&TokenKind::LBrace)?;
                    self.parse_exception_body()?;
                    self.expect(&TokenKind::RBrace)?;
                }
                Ok(Stmt::Block(block))
            }
            TokenKind::If => self.parse_if(),
            TokenKind::For => self.parse_for(span),
            TokenKind::While => self.parse_while(span),
            TokenKind::Do => self.parse_do(span),
            TokenKind::Return => {
                self.advance();
                let expr = if self.at(&TokenKind::Semicolon) {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                self.expect_semi()?;
                Ok(Stmt::Return(expr, span))
            }
            TokenKind::Break => {
                self.advance();
                let label = if let TokenKind::Ident(name) = self.peek().clone() {
                    self.advance();
                    Some(name)
                } else {
                    None
                };
                self.expect_semi()?;
                Ok(Stmt::Break(label, span))
            }
            TokenKind::Continue => {
                self.advance();
                let label = if let TokenKind::Ident(name) = self.peek().clone() {
                    self.advance();
                    Some(name)
                } else {
                    None
                };
                self.expect_semi()?;
                Ok(Stmt::Continue(label, span))
            }
            TokenKind::Exit => {
                self.advance();
                self.expect_semi()?;
                Ok(Stmt::Exit(span))
            }
            TokenKind::Spawn => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect_semi()?;
                Ok(Stmt::Spawn(expr, span))
            }
            TokenKind::Raise => {
                self.advance();
                let expr = if self.at(&TokenKind::Semicolon) {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                self.expect_semi()?;
                Ok(Stmt::Raise(expr, span))
            }
            TokenKind::Case => self.parse_case(span),
            TokenKind::Pick => {
                self.advance();
                let name = self.expect_ident()?;
                self.expect(&TokenKind::ColonEq)?;
                let expr = self.parse_expr()?;
                self.expect(&TokenKind::LBrace)?;
                let mut arms = Vec::new();
                while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
                    let mut tags = Vec::new();
                    if self.at(&TokenKind::Star) {
                        self.advance();
                        tags.push("*".to_string());
                    } else {
                        tags.push(self.expect_ident()?);
                        while self.at(&TokenKind::Or) {
                            self.advance();
                            tags.push(self.expect_ident()?);
                        }
                    }
                    self.expect(&TokenKind::FatArrow)?;
                    let mut body = Vec::new();
                    while !self.at(&TokenKind::RBrace)
                        && !self.at(&TokenKind::Eof)
                        && !self.is_pick_tag_start()
                        && !self.at(&TokenKind::Star)
                    {
                        body.push(self.parse_stmt()?);
                    }
                    arms.push(PickArm { tags, body });
                }
                self.expect(&TokenKind::RBrace)?;
                Ok(Stmt::Pick(PickStmt {
                    name,
                    expr,
                    arms,
                    span,
                }))
            }
            TokenKind::Alt => self.parse_alt(span),
            TokenKind::Semicolon => {
                self.advance();
                Ok(Stmt::Empty)
            }
            TokenKind::Exception => {
                self.advance();
                if let TokenKind::Ident(_) = self.peek() {
                    self.advance();
                }
                self.expect(&TokenKind::LBrace)?;
                self.parse_exception_body()?;
                self.expect(&TokenKind::RBrace)?;
                Ok(Stmt::Empty)
            }
            _ => {
                // Check for label: name: stmt (where stmt is for/while/case/alt/etc.)
                if self.is_label() {
                    let label = self.expect_ident()?;
                    self.expect(&TokenKind::Colon)?;
                    let stmt = self.parse_stmt()?;
                    return Ok(Stmt::Label(label, Box::new(stmt)));
                }
                // Check for local variable declaration: name: type [= expr];
                // or local import: Name: import module;
                if self.is_local_var_decl() {
                    return self.parse_local_var_decl();
                }
                // Check for local include
                if self.at(&TokenKind::Include) {
                    self.advance();
                    if let TokenKind::StringLit(_) = self.peek() {
                        self.advance();
                    }
                    self.expect_semi()?;
                    return Ok(Stmt::Empty);
                }
                // Expression statement
                let expr = self.parse_expr()?;
                self.expect_semi()?;
                Ok(Stmt::Expr(expr))
            }
        }
    }

    /// Parse the body of an exception handler block: `pattern => stmts; ...`
    fn parse_exception_body(&mut self) -> Result<(), ParseError> {
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            // Parse pattern: string literal, "*", or identifier, possibly with "or"
            let mut depth = 0;
            loop {
                match self.peek() {
                    TokenKind::FatArrow if depth == 0 => {
                        self.advance();
                        break;
                    }
                    TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                        depth += 1;
                        self.advance();
                    }
                    TokenKind::RParen | TokenKind::RBracket => {
                        depth -= 1;
                        self.advance();
                    }
                    TokenKind::RBrace => {
                        if depth > 0 {
                            depth -= 1;
                            self.advance();
                        } else {
                            return Ok(());
                        }
                    }
                    TokenKind::Eof => return Ok(()),
                    _ => {
                        self.advance();
                    }
                }
            }
            // Parse handler body statements until next pattern or closing brace
            while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
                // Check if next tokens form a new pattern (string/ident/star followed by =>)
                if self.is_exception_pattern_start() {
                    break;
                }
                self.parse_stmt()?;
            }
        }
        Ok(())
    }

    /// Check if the current position starts an exception pattern.
    fn is_exception_pattern_start(&self) -> bool {
        // Patterns: "string", *, identifier — all followed eventually by =>
        match self.peek() {
            TokenKind::Star => return true,
            TokenKind::StringLit(_) | TokenKind::Ident(_) => {}
            _ => return false,
        }
        // Look ahead for => (possibly through "or" separators)
        let mut i = self.pos + 1;
        while i < self.tokens.len() {
            match &self.tokens[i].kind {
                TokenKind::FatArrow => return true,
                TokenKind::Or => {
                    i += 1;
                } // skip 'or' separator
                TokenKind::StringLit(_) | TokenKind::Ident(_) | TokenKind::Star => {
                    i += 1;
                }
                _ => return false,
            }
        }
        false
    }

    /// Check if the current position is a label: `name: stmt`
    fn is_label(&self) -> bool {
        if !matches!(self.peek(), TokenKind::Ident(_)) {
            return false;
        }
        let i = self.pos + 1;
        if i >= self.tokens.len() || self.tokens[i].kind != TokenKind::Colon {
            return false;
        }
        if i + 1 >= self.tokens.len() {
            return false;
        }
        matches!(
            self.tokens[i + 1].kind,
            TokenKind::For
                | TokenKind::While
                | TokenKind::Do
                | TokenKind::Case
                | TokenKind::Alt
                | TokenKind::Pick
                | TokenKind::LBrace
        )
    }

    /// Check if the current position starts a local variable declaration:
    /// `name [, name]* : type [= expr] ;`
    /// Must distinguish from labels (`name: stmt`) and expressions (`name(args)`).
    fn is_local_var_decl(&self) -> bool {
        if !matches!(self.peek(), TokenKind::Ident(_)) {
            return false;
        }
        let mut i = self.pos + 1;
        // Single ident + colon: check for label (name: followed by statement keyword)
        if i < self.tokens.len()
            && self.tokens[i].kind == TokenKind::Colon
            && i + 1 < self.tokens.len()
        {
            let after = &self.tokens[i + 1].kind;
            if matches!(
                after,
                TokenKind::For
                    | TokenKind::While
                    | TokenKind::Do
                    | TokenKind::Case
                    | TokenKind::Alt
                    | TokenKind::Pick
                    | TokenKind::LBrace
                    | TokenKind::Semicolon
            ) {
                return false; // it's a label
            }
        }
        // Skip comma-separated names
        while i < self.tokens.len() {
            match &self.tokens[i].kind {
                TokenKind::Comma => {
                    i += 1;
                    if i < self.tokens.len() && matches!(self.tokens[i].kind, TokenKind::Ident(_)) {
                        i += 1;
                    }
                }
                TokenKind::Colon => {
                    // Check what follows the colon — must be a type keyword or ident (not =, etc.)
                    if i + 1 < self.tokens.len() {
                        return matches!(
                            self.tokens[i + 1].kind,
                            TokenKind::Int
                                | TokenKind::Byte
                                | TokenKind::Big
                                | TokenKind::Real
                                | TokenKind::String_
                                | TokenKind::Array
                                | TokenKind::List
                                | TokenKind::Chan
                                | TokenKind::Ref
                                | TokenKind::Fn
                                | TokenKind::LParen
                                | TokenKind::Cyclic
                                | TokenKind::Import
                                | TokenKind::Con
                                | TokenKind::Type
                                | TokenKind::Self_
                                | TokenKind::Ident(_)
                        );
                    }
                    return false;
                }
                _ => return false,
            }
        }
        false
    }

    /// Parse a local variable declaration: `name [, name]* : type [= expr] ;`
    fn parse_local_var_decl(&mut self) -> Result<Stmt, ParseError> {
        let span = self.span();
        let mut names = vec![self.expect_ident()?];
        while self.at(&TokenKind::Comma) {
            self.advance();
            names.push(self.expect_ident()?);
        }
        self.expect(&TokenKind::Colon)?;

        // Handle special forms: import, con, type
        if self.at(&TokenKind::Import) {
            self.advance();
            let _module = self.expect_ident()?;
            self.expect_semi()?;
            return Ok(Stmt::Empty); // import handled as side effect
        }
        if self.at(&TokenKind::Con) {
            self.advance();
            let _value = self.parse_expr()?;
            self.expect_semi()?;
            return Ok(Stmt::Empty); // local const
        }
        if self.at(&TokenKind::Type) {
            self.advance();
            let _ty = self.parse_type()?;
            self.expect_semi()?;
            return Ok(Stmt::Empty); // local type alias
        }

        let ty = self.parse_type()?;
        let init = if self.at(&TokenKind::Assign) {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect_semi()?;
        Ok(Stmt::VarDecl(VarDecl {
            names,
            ty: Some(ty),
            init,
            span,
        }))
    }

    fn parse_if(&mut self) -> Result<Stmt, ParseError> {
        let span = self.span();
        self.expect(&TokenKind::If)?;
        self.expect(&TokenKind::LParen)?;
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::RParen)?;
        let then = Box::new(self.parse_stmt()?);
        let else_ = if self.at(&TokenKind::Else) {
            self.advance();
            Some(Box::new(self.parse_stmt()?))
        } else {
            None
        };
        Ok(Stmt::If(IfStmt {
            cond,
            then,
            else_,
            span,
        }))
    }

    fn parse_for(&mut self, span: Span) -> Result<Stmt, ParseError> {
        self.expect(&TokenKind::For)?;
        self.expect(&TokenKind::LParen)?;
        let init = if self.at(&TokenKind::Semicolon) {
            None
        } else {
            let expr = self.parse_expr()?;
            Some(Box::new(Stmt::Expr(expr)))
        };
        self.expect_semi()?;
        let cond = if self.at(&TokenKind::Semicolon) {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.expect_semi()?;
        let post = if self.at(&TokenKind::RParen) {
            None
        } else {
            let expr = self.parse_expr()?;
            Some(Box::new(Stmt::Expr(expr)))
        };
        self.expect(&TokenKind::RParen)?;
        let body = Box::new(self.parse_stmt()?);
        Ok(Stmt::For(ForStmt {
            init,
            cond,
            post,
            body,
            span,
        }))
    }

    fn parse_while(&mut self, span: Span) -> Result<Stmt, ParseError> {
        self.expect(&TokenKind::While)?;
        self.expect(&TokenKind::LParen)?;
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::RParen)?;
        let body = Box::new(self.parse_stmt()?);
        Ok(Stmt::While(WhileStmt { cond, body, span }))
    }

    fn parse_do(&mut self, span: Span) -> Result<Stmt, ParseError> {
        self.expect(&TokenKind::Do)?;
        let body = Box::new(self.parse_stmt()?);
        self.expect(&TokenKind::While)?;
        self.expect(&TokenKind::LParen)?;
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::RParen)?;
        self.expect_semi()?;
        Ok(Stmt::Do(DoStmt { body, cond, span }))
    }

    fn parse_case(&mut self, span: Span) -> Result<Stmt, ParseError> {
        self.expect(&TokenKind::Case)?;
        let expr = self.parse_expr()?;
        self.expect(&TokenKind::LBrace)?;
        let mut arms = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            let patterns = self.parse_case_patterns()?;
            self.expect(&TokenKind::FatArrow)?;
            let mut body = Vec::new();
            loop {
                if self.at(&TokenKind::RBrace) || self.at(&TokenKind::Eof) {
                    break;
                }
                if self.is_case_pattern_start() {
                    break;
                }
                match self.parse_stmt() {
                    Ok(s) => body.push(s),
                    Err(_) => {
                        while !self.at(&TokenKind::RBrace)
                            && !self.at(&TokenKind::Eof)
                            && !self.is_case_pattern_start()
                        {
                            self.advance();
                        }
                        break;
                    }
                }
            }
            arms.push(CaseArm { patterns, body });
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(Stmt::Case(CaseStmt { expr, arms, span }))
    }

    fn parse_case_patterns(&mut self) -> Result<Vec<CasePattern>, ParseError> {
        let mut patterns = Vec::new();
        loop {
            if self.at(&TokenKind::Star) {
                self.advance();
                patterns.push(CasePattern::Wildcard);
            } else {
                let expr = self.parse_expr()?;
                if self.at(&TokenKind::To) {
                    self.advance();
                    let end = self.parse_expr()?;
                    patterns.push(CasePattern::Range(expr, end));
                } else {
                    patterns.push(CasePattern::Expr(expr));
                }
            }
            if !self.at(&TokenKind::Or) {
                break;
            }
            self.advance();
        }
        Ok(patterns)
    }

    fn is_case_pattern_start(&self) -> bool {
        if self.at(&TokenKind::Star) {
            return true;
        }
        // Patterns must start with an expression token, not a statement/block token
        if matches!(
            self.peek(),
            TokenKind::LBrace
                | TokenKind::If
                | TokenKind::For
                | TokenKind::While
                | TokenKind::Do
                | TokenKind::Return
                | TokenKind::Break
                | TokenKind::Continue
                | TokenKind::Exit
                | TokenKind::Spawn
                | TokenKind::Raise
                | TokenKind::Alt
                | TokenKind::Pick
                | TokenKind::Case
                | TokenKind::Semicolon
        ) {
            return false;
        }
        // Fast path: single-token pattern followed by =>
        if self.pos + 1 < self.tokens.len()
            && matches!(self.tokens[self.pos + 1].kind, TokenKind::FatArrow)
        {
            return true;
        }
        let mut depth = 0;
        let mut i = self.pos;
        let limit = (self.pos + 60).min(self.tokens.len()); // limit lookahead
        while i < limit {
            match &self.tokens[i].kind {
                TokenKind::FatArrow if depth == 0 => return true,
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                TokenKind::RParen | TokenKind::RBracket => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                TokenKind::RBrace => {
                    if depth < 1 {
                        return false;
                    } // at case level, stop
                    depth -= 1;
                }
                TokenKind::Semicolon if depth == 0 => return false,
                // Statement keywords at depth 0 mean this isn't a pattern
                TokenKind::If
                | TokenKind::For
                | TokenKind::While
                | TokenKind::Do
                | TokenKind::Case
                | TokenKind::Return
                | TokenKind::Break
                | TokenKind::Continue
                | TokenKind::Exit
                | TokenKind::Spawn
                | TokenKind::Raise
                | TokenKind::Alt
                | TokenKind::Pick
                    if depth == 0 =>
                {
                    return false;
                }
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn parse_alt(&mut self, span: Span) -> Result<Stmt, ParseError> {
        self.expect(&TokenKind::Alt)?;
        self.expect(&TokenKind::LBrace)?;
        let mut arms = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            let guard = if self.at(&TokenKind::Star) {
                self.advance();
                AltGuard::Wildcard
            } else {
                // Parse guard: expr [or expr]* =>
                // Consume everything until => at depth 0
                let expr = self.parse_expr()?;
                while self.at(&TokenKind::Or) {
                    self.advance();
                    let _ = self.parse_expr()?; // consume alternative guard
                }
                AltGuard::Recv(None, expr)
            };
            self.expect(&TokenKind::FatArrow)?;
            let mut body = Vec::new();
            while !self.at(&TokenKind::RBrace)
                && !self.at(&TokenKind::Eof)
                && !self.is_alt_guard_start()
            {
                body.push(self.parse_stmt()?);
            }
            arms.push(AltArm { guard, body });
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(Stmt::Alt(AltStmt { arms, span }))
    }

    fn is_alt_guard_start(&self) -> bool {
        if self.at(&TokenKind::Star) {
            return true;
        }
        // Look for => within a limited range, not crossing { or ;
        let mut i = self.pos;
        let limit = (self.pos + 30).min(self.tokens.len());
        while i < limit {
            match &self.tokens[i].kind {
                TokenKind::FatArrow => return true,
                TokenKind::Semicolon | TokenKind::LBrace | TokenKind::RBrace => return false,
                _ => i += 1,
            }
        }
        false
    }

    // ── Expressions (Pratt parser) ─────────────────────────────

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_expr_bp(0)
    }

    /// Pratt parser: parse expression with minimum binding power.
    fn parse_expr_bp(&mut self, min_bp: u8) -> Result<Expr, ParseError> {
        let span = self.span();
        let mut lhs = self.parse_prefix()?;

        loop {
            // Postfix operators
            match self.peek() {
                TokenKind::LParen => {
                    self.advance();
                    let args = self.parse_expr_list()?;
                    self.expect(&TokenKind::RParen)?;
                    lhs = Expr::Call(Box::new(lhs), args, span);
                    continue;
                }
                TokenKind::Dot => {
                    self.advance();
                    let member = self.expect_ident()?;
                    lhs = Expr::Dot(Box::new(lhs), member, span);
                    continue;
                }
                TokenKind::Arrow => {
                    self.advance();
                    let member = self.expect_ident()?;
                    lhs = Expr::ModQual(Box::new(lhs), member, span);
                    continue;
                }
                TokenKind::LBracket => {
                    self.advance();
                    if self.at(&TokenKind::Colon) {
                        // [: hi]
                        self.advance();
                        let hi = if self.at(&TokenKind::RBracket) {
                            None
                        } else {
                            Some(Box::new(self.parse_expr()?))
                        };
                        self.expect(&TokenKind::RBracket)?;
                        lhs = Expr::Slice(Box::new(lhs), None, hi, span);
                    } else {
                        let idx = self.parse_expr()?;
                        if self.at(&TokenKind::Colon) {
                            // [lo : hi]
                            self.advance();
                            let hi = if self.at(&TokenKind::RBracket) {
                                None
                            } else {
                                Some(Box::new(self.parse_expr()?))
                            };
                            self.expect(&TokenKind::RBracket)?;
                            lhs = Expr::Slice(Box::new(lhs), Some(Box::new(idx)), hi, span);
                        } else {
                            // [idx]
                            self.expect(&TokenKind::RBracket)?;
                            lhs = Expr::Index(Box::new(lhs), Box::new(idx), span);
                        }
                    }
                    continue;
                }
                TokenKind::Inc => {
                    self.advance();
                    lhs = Expr::PostInc(Box::new(lhs), span);
                    continue;
                }
                TokenKind::Dec => {
                    self.advance();
                    lhs = Expr::PostDec(Box::new(lhs), span);
                    continue;
                }
                _ => {}
            }

            // Infix operators
            if let Some((l_bp, r_bp, op)) = self.infix_binding_power() {
                if l_bp < min_bp {
                    break;
                }
                self.advance();
                let rhs = self.parse_expr_bp(r_bp)?;
                lhs = Expr::Binary(Box::new(lhs), op, Box::new(rhs), span);
                continue;
            }

            // Assignment operators
            if let Some((r_bp, kind)) = self.assign_binding_power() {
                if 1 < min_bp {
                    break;
                }
                self.advance();
                let rhs = self.parse_expr_bp(r_bp)?;
                match kind {
                    AssignKind::Simple => {
                        lhs = Expr::Assign(Box::new(lhs), Box::new(rhs), span);
                    }
                    AssignKind::Compound(op) => {
                        lhs = Expr::CompoundAssign(Box::new(lhs), op, Box::new(rhs), span);
                    }
                    AssignKind::Decl => {
                        // a := expr  or  (a, b) := expr
                        if let Expr::Ident(name, _) = lhs {
                            lhs = Expr::DeclAssign(vec![name], Box::new(rhs), span);
                        } else if let Expr::Tuple(exprs, _) = &lhs {
                            let mut names = Vec::new();
                            for e in exprs {
                                match e {
                                    Expr::Ident(n, _) => names.push(n.clone()),
                                    Expr::Nil(_) => names.push("nil".to_string()),
                                    _ => {
                                        return Err(
                                            self.err("tuple := elements must be identifiers")
                                        );
                                    }
                                }
                            }
                            lhs = Expr::TupleDeclAssign(names, Box::new(rhs), span);
                        } else {
                            return Err(self.err("left side of := must be identifier or tuple"));
                        }
                    }
                }
                continue;
            }

            // Channel send: expr <-= expr
            if self.at(&TokenKind::ChanSend) {
                if 1 < min_bp {
                    break;
                }
                self.advance();
                let rhs = self.parse_expr_bp(1)?;
                lhs = Expr::Send(Box::new(lhs), Box::new(rhs), span);
                continue;
            }

            // Cons operator (right-associative)
            if self.at(&TokenKind::ColonColon) {
                if 5 < min_bp {
                    break;
                }
                self.advance();
                let rhs = self.parse_expr_bp(5)?;
                lhs = Expr::Cons(Box::new(lhs), Box::new(rhs), span);
                continue;
            }

            break;
        }

        Ok(lhs)
    }

    /// Parse prefix expressions (unary ops, atoms, array/chan/list constructors).
    fn parse_prefix(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();

        match self.peek().clone() {
            TokenKind::IntLit(v) => {
                self.advance();
                Ok(Expr::IntLit(v, span))
            }
            TokenKind::RealLit(v) => {
                self.advance();
                Ok(Expr::RealLit(v, span))
            }
            TokenKind::StringLit(s) => {
                self.advance();
                Ok(Expr::StringLit(s, span))
            }
            TokenKind::CharLit(v) => {
                self.advance();
                Ok(Expr::CharLit(v, span))
            }
            TokenKind::Nil => {
                self.advance();
                Ok(Expr::Nil(span))
            }
            TokenKind::Ident(name) => {
                self.advance();
                Ok(Expr::Ident(name, span))
            }
            TokenKind::Iota => {
                self.advance();
                Ok(Expr::Ident("iota".to_string(), span))
            }
            TokenKind::Inc => {
                // Pre-increment: ++x (semantically same as x++ for Limbo)
                self.advance();
                let expr = self.parse_expr_bp(14)?;
                Ok(Expr::PostInc(Box::new(expr), span))
            }
            TokenKind::Dec => {
                // Pre-decrement: --x
                self.advance();
                let expr = self.parse_expr_bp(14)?;
                Ok(Expr::PostDec(Box::new(expr), span))
            }
            TokenKind::Plus => {
                // Unary plus
                self.advance();
                self.parse_expr_bp(14)
            }
            TokenKind::Star => {
                // Dereference: *expr
                self.advance();
                // Check if this is a wildcard in initializer context (followed by =>)
                if self.at(&TokenKind::FatArrow) {
                    return Ok(Expr::Ident("*".to_string(), span));
                }
                let expr = self.parse_expr_bp(14)?;
                Ok(Expr::Unary(UnaryOp::Ref, Box::new(expr), span)) // deref uses Ref variant for now
            }
            TokenKind::Minus => {
                self.advance();
                let expr = self.parse_expr_bp(14)?;
                Ok(Expr::Unary(UnaryOp::Neg, Box::new(expr), span))
            }
            TokenKind::Bang => {
                self.advance();
                let expr = self.parse_expr_bp(14)?;
                Ok(Expr::Unary(UnaryOp::Not, Box::new(expr), span))
            }
            TokenKind::Tilde => {
                self.advance();
                let expr = self.parse_expr_bp(14)?;
                Ok(Expr::Unary(UnaryOp::BitNot, Box::new(expr), span))
            }
            TokenKind::Hd => {
                self.advance();
                let expr = self.parse_expr_bp(14)?;
                Ok(Expr::Hd(Box::new(expr), span))
            }
            TokenKind::Tl => {
                self.advance();
                let expr = self.parse_expr_bp(14)?;
                Ok(Expr::Tl(Box::new(expr), span))
            }
            TokenKind::Len => {
                self.advance();
                let expr = self.parse_expr_bp(14)?;
                Ok(Expr::Len(Box::new(expr), span))
            }
            TokenKind::Tagof => {
                self.advance();
                let expr = self.parse_expr_bp(14)?;
                Ok(Expr::Tagof(Box::new(expr), span))
            }
            TokenKind::Ref => {
                self.advance();
                let expr = self.parse_expr_bp(14)?;
                Ok(Expr::Unary(UnaryOp::Ref, Box::new(expr), span))
            }
            TokenKind::ChanRecv => {
                // <-chan (receive)
                self.advance();
                let expr = self.parse_expr_bp(14)?;
                Ok(Expr::Recv(Box::new(expr), span))
            }
            TokenKind::Array => {
                self.advance();
                if self.at(&TokenKind::LBracket) {
                    self.advance();
                    if self.at(&TokenKind::RBracket) {
                        // array[] of { ... }
                        self.advance();
                        self.expect(&TokenKind::Of)?;
                        if self.at(&TokenKind::LBrace) {
                            self.advance();
                            let elems = self.parse_expr_list()?;
                            self.expect(&TokenKind::RBrace)?;
                            Ok(Expr::ArrayLit(elems, None, span))
                        } else {
                            let ty = self.parse_type()?;
                            Ok(Expr::ArrayAlloc(
                                Box::new(Expr::IntLit(0, span)),
                                Box::new(ty),
                                span,
                            ))
                        }
                    } else {
                        let size = self.parse_expr()?;
                        self.expect(&TokenKind::RBracket)?;
                        self.expect(&TokenKind::Of)?;
                        if self.at(&TokenKind::LBrace) {
                            self.advance();
                            let elems = self.parse_expr_list()?;
                            self.expect(&TokenKind::RBrace)?;
                            Ok(Expr::ArrayLit(elems, None, span))
                        } else {
                            let ty = self.parse_type()?;
                            Ok(Expr::ArrayAlloc(Box::new(size), Box::new(ty), span))
                        }
                    }
                } else {
                    self.expect(&TokenKind::Of)?;
                    let ty = self.parse_type()?;
                    // array of type monexp (cast)
                    let expr = self.parse_expr_bp(14)?;
                    Ok(Expr::Cast(
                        Box::new(Type::Array(Box::new(ty))),
                        Box::new(expr),
                        span,
                    ))
                }
            }
            TokenKind::Chan => {
                self.advance();
                if self.at(&TokenKind::LBracket) {
                    self.advance();
                    let _size = self.parse_expr()?;
                    self.expect(&TokenKind::RBracket)?;
                    self.expect(&TokenKind::Of)?;
                    let ty = self.parse_type()?;
                    Ok(Expr::ChanAlloc(Box::new(ty), span))
                } else {
                    self.expect(&TokenKind::Of)?;
                    let ty = self.parse_type()?;
                    Ok(Expr::ChanAlloc(Box::new(ty), span))
                }
            }
            TokenKind::List => {
                self.advance();
                self.expect(&TokenKind::Of)?;
                self.expect(&TokenKind::LBrace)?;
                let elems = self.parse_expr_list()?;
                self.expect(&TokenKind::RBrace)?;
                Ok(Expr::ListLit(elems, span))
            }
            TokenKind::Load => {
                self.advance();
                let module_name = self.expect_ident()?;
                let ty = Type::Named(QualName {
                    qualifier: None,
                    name: module_name,
                });
                let path = self.parse_expr_bp(2)?;
                Ok(Expr::Load(Box::new(ty), Box::new(path), span))
            }
            TokenKind::LParen => {
                self.advance();
                let first = self.parse_expr()?;
                if self.at(&TokenKind::Comma) {
                    // Tuple: (e1, e2, ...)
                    let mut exprs = vec![first];
                    while self.at(&TokenKind::Comma) {
                        self.advance();
                        if self.at(&TokenKind::RParen) {
                            break;
                        }
                        exprs.push(self.parse_expr()?);
                    }
                    self.expect(&TokenKind::RParen)?;
                    Ok(Expr::Tuple(exprs, span))
                } else {
                    // Parenthesized expression
                    self.expect(&TokenKind::RParen)?;
                    Ok(first)
                }
            }
            // Type cast: int expr, string expr, etc.
            TokenKind::Int
            | TokenKind::Byte
            | TokenKind::Big
            | TokenKind::Real
            | TokenKind::String_ => {
                let ty = self.parse_type()?;
                // If next token is ] or , or ) or => — this is a type expression, not a cast
                if matches!(
                    self.peek(),
                    TokenKind::RBracket
                        | TokenKind::Comma
                        | TokenKind::RParen
                        | TokenKind::FatArrow
                        | TokenKind::Semicolon
                        | TokenKind::RBrace
                ) {
                    // Treat as type name in expression context (e.g., array index with type)
                    return Ok(Expr::Ident(format!("{ty:?}"), span));
                }
                let expr = self.parse_expr_bp(14)?;
                Ok(Expr::Cast(Box::new(ty), Box::new(expr), span))
            }
            TokenKind::LBrace => {
                // Recovery: skip balanced braces in expression context
                self.advance();
                let mut depth = 1;
                while depth > 0 && !self.at(&TokenKind::Eof) {
                    if self.at(&TokenKind::LBrace) {
                        depth += 1;
                    } else if self.at(&TokenKind::RBrace) {
                        depth -= 1;
                    }
                    self.advance();
                }
                Ok(Expr::Nil(span))
            }
            _ => Err(self.err(format!("unexpected token in expression: {:?}", self.peek()))),
        }
    }

    /// Parse an expression that may be a qualified initializer:
    /// `qual => expr`, `qual to qual => expr`, `qual or qual => expr`
    fn parse_init_expr(&mut self) -> Result<Expr, ParseError> {
        let expr = self.parse_expr()?;
        // Check for qualifier patterns
        if self.at(&TokenKind::FatArrow) || self.at(&TokenKind::To) || self.at(&TokenKind::Or) {
            // Skip all qualifier parts until we hit =>
            loop {
                if self.at(&TokenKind::FatArrow) {
                    self.advance();
                    return self.parse_expr();
                } else if self.at(&TokenKind::Or) {
                    self.advance();
                    let _ = self.parse_expr()?; // consume next qual
                } else if self.at(&TokenKind::To) {
                    self.advance();
                    let _ = self.parse_expr()?; // consume range end
                } else {
                    break;
                }
            }
        }
        Ok(expr)
    }

    fn parse_expr_list(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut exprs = Vec::new();
        if self.at(&TokenKind::RParen)
            || self.at(&TokenKind::RBracket)
            || self.at(&TokenKind::RBrace)
        {
            return Ok(exprs);
        }
        exprs.push(self.parse_init_expr()?);
        while self.at(&TokenKind::Comma) {
            self.advance();
            if self.at(&TokenKind::RParen)
                || self.at(&TokenKind::RBracket)
                || self.at(&TokenKind::RBrace)
            {
                break;
            }
            exprs.push(self.parse_init_expr()?);
        }
        Ok(exprs)
    }

    /// Return (left_bp, right_bp, op) for infix binary operators.
    fn infix_binding_power(&self) -> Option<(u8, u8, BinOp)> {
        match self.peek() {
            TokenKind::OrOr => Some((3, 4, BinOp::LogOr)),
            TokenKind::AndAnd => Some((5, 6, BinOp::LogAnd)),
            TokenKind::Pipe => Some((7, 8, BinOp::Or)),
            TokenKind::Caret => Some((9, 10, BinOp::Xor)),
            TokenKind::Amp => Some((11, 12, BinOp::And)),
            TokenKind::Eq => Some((13, 14, BinOp::Eq)),
            TokenKind::Neq => Some((13, 14, BinOp::Neq)),
            TokenKind::Lt => Some((15, 16, BinOp::Lt)),
            TokenKind::Gt => Some((15, 16, BinOp::Gt)),
            TokenKind::Leq => Some((15, 16, BinOp::Leq)),
            TokenKind::Geq => Some((15, 16, BinOp::Geq)),
            TokenKind::Lshift => Some((17, 18, BinOp::Lshift)),
            TokenKind::Rshift => Some((17, 18, BinOp::Rshift)),
            TokenKind::Plus => Some((19, 20, BinOp::Add)),
            TokenKind::Minus => Some((19, 20, BinOp::Sub)),
            TokenKind::Star => Some((21, 22, BinOp::Mul)),
            TokenKind::Slash => Some((21, 22, BinOp::Div)),
            TokenKind::Percent => Some((21, 22, BinOp::Mod)),
            TokenKind::Power => Some((24, 23, BinOp::Power)), // right-assoc
            _ => None,
        }
    }

    /// Return (right_bp, kind) for assignment operators.
    fn assign_binding_power(&self) -> Option<(u8, AssignKind)> {
        match self.peek() {
            TokenKind::Assign => Some((1, AssignKind::Simple)),
            TokenKind::ColonEq => Some((1, AssignKind::Decl)),
            TokenKind::PlusEq => Some((1, AssignKind::Compound(BinOp::Add))),
            TokenKind::MinusEq => Some((1, AssignKind::Compound(BinOp::Sub))),
            TokenKind::StarEq => Some((1, AssignKind::Compound(BinOp::Mul))),
            TokenKind::SlashEq => Some((1, AssignKind::Compound(BinOp::Div))),
            TokenKind::PercentEq => Some((1, AssignKind::Compound(BinOp::Mod))),
            TokenKind::AmpEq => Some((1, AssignKind::Compound(BinOp::And))),
            TokenKind::PipeEq => Some((1, AssignKind::Compound(BinOp::Or))),
            TokenKind::CaretEq => Some((1, AssignKind::Compound(BinOp::Xor))),
            TokenKind::LshiftEq => Some((1, AssignKind::Compound(BinOp::Lshift))),
            TokenKind::RshiftEq => Some((1, AssignKind::Compound(BinOp::Rshift))),
            _ => None,
        }
    }
}

enum LookAhead {
    FuncDef,
    ColonDecl,
    Assign,
    DeclAssign,
}

enum AssignKind {
    Simple,
    Compound(BinOp),
    Decl,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse(src: &str) -> SourceFile {
        let tokens = Lexer::new(src, "<test>")
            .tokenize()
            .expect("lex should succeed");
        Parser::new(tokens, "<test>")
            .parse_file()
            .expect("parse should succeed")
    }

    #[test]
    fn parse_implement_and_include() {
        let file = parse(
            r#"implement Echo;
include "sys.m";
"#,
        );
        assert_eq!(file.implement, vec!["Echo"]);
        assert_eq!(file.includes.len(), 1);
        assert_eq!(file.includes[0].path, "sys.m");
    }

    #[test]
    fn parse_module_decl() {
        let file = parse(
            r#"implement Test;
Test: module {
    init: fn(nil: ref Draw->Context, argv: list of string);
    PATH: con "/dis/test.dis";
};
"#,
        );
        assert_eq!(file.decls.len(), 1);
        assert!(matches!(file.decls[0], Decl::Module(_)));
    }

    #[test]
    fn parse_variable_decl() {
        let file = parse(
            r#"implement T;
sys: Sys;
n: int;
"#,
        );
        assert_eq!(file.decls.len(), 2);
        assert!(matches!(file.decls[0], Decl::Var(_)));
    }

    #[test]
    fn parse_simple_function() {
        let file = parse(
            r#"implement T;
init(nil: ref Draw->Context, args: list of string)
{
    sys = load Sys Sys->PATH;
    sys->print("hello\n");
}
"#,
        );
        assert_eq!(file.decls.len(), 1);
        assert!(matches!(file.decls[0], Decl::Func(_)));
    }

    #[test]
    fn parse_if_else() {
        let file = parse(
            r#"implement T;
test()
{
    if (x > 0)
        y = 1;
    else
        y = 2;
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        assert!(matches!(f.body.stmts[0], Stmt::If(_)));
    }

    #[test]
    fn parse_for_loop() {
        let file = parse(
            r#"implement T;
test()
{
    for (i := 0; i < n; i++)
        x = x + 1;
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        assert!(matches!(f.body.stmts[0], Stmt::For(_)));
    }

    #[test]
    fn parse_echo_program() {
        let src = r#"implement Echo;

include "sys.m";
    sys: Sys;

include "draw.m";

Echo: module
{
    init: fn(ctxt: ref Draw->Context, argv: list of string);
};

init(ctxt: ref Draw->Context, argv: list of string)
{
    sys = load Sys Sys->PATH;
    argv = tl argv;
    s := "";
    while(argv != nil) {
        s = s + hd argv;
        argv = tl argv;
        if(argv != nil)
            s = s + " ";
    }
    sys->print("%s\n", s);
}
"#;
        let file = parse(src);
        assert_eq!(file.implement, vec!["Echo"]);
        assert_eq!(file.includes.len(), 2);
        // sys: Sys; (var), Echo: module{...}; (module), init(...){...} (func)
        assert!(file.decls.len() >= 3);
    }

    #[test]
    fn parse_array_and_list_constructors() {
        let file = parse(
            r#"implement T;
test()
{
    a := array[10] of int;
    b := array[] of {"hello", "world"};
    c := list of {1, 2, 3};
    d := chan of int;
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        assert_eq!(f.body.stmts.len(), 4);
    }

    #[test]
    fn parse_case_statement() {
        let file = parse(
            r#"implement T;
test(x: int)
{
    case x {
    0 =>
        y = 1;
    1 to 10 =>
        y = 2;
    * =>
        y = 3;
    }
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        assert!(matches!(f.body.stmts[0], Stmt::Case(_)));
    }

    #[test]
    fn parse_operator_precedence() {
        let file = parse(
            r#"implement T;
test()
{
    x = a + b * c;
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        // a + (b * c), not (a + b) * c
        if let Stmt::Expr(Expr::Assign(_, rhs, _)) = &f.body.stmts[0] {
            assert!(matches!(rhs.as_ref(), Expr::Binary(_, BinOp::Add, _, _)));
        }
    }
}
