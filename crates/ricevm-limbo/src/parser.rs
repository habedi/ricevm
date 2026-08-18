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

    /// Is the token `n` places ahead of the cursor of this kind?
    fn at_offset(&self, n: usize, kind: &TokenKind) -> bool {
        let ahead = self
            .tokens
            .get(self.pos + n)
            .map(|t| &t.kind)
            .unwrap_or(&TokenKind::Eof);
        std::mem::discriminant(ahead) == std::mem::discriminant(kind)
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

    /// Is the cursor on the wildcard arm of a `case`, `alt`, or `pick`? The
    /// wildcard is `'*'` followed by the `=>` that opens the arm, or by the
    /// `or` that joins it to another pattern, as in `* or "disc" =>`
    /// (appl/ebook/reader.b:1371). The token after the star is what decides,
    /// because a statement may start with one too (`qual: '*'` in limbo.y
    /// against the `'*' monexp` dereference at limbo.y:1250). Taking every star
    /// for a wildcard cut the arm short at a statement such as `*in = *b;`
    /// (appl/cmd/limbo/gen.b:563).
    fn at_wildcard_arm(&self) -> bool {
        self.at(&TokenKind::Star)
            && (self.at_offset(1, &TokenKind::FatArrow) || self.at_offset(1, &TokenKind::Or))
    }

    /// Step over a `[T1, T2]` type parameter list, which the grammar allows on
    /// a declaration (`polydec`) and on a type name (`Lid '[' types ']'`).
    /// Polymorphic types are not represented yet, so the list is discarded
    /// rather than recorded.
    fn skip_type_params(&mut self) {
        if !self.at(&TokenKind::LBracket) {
            return;
        }
        self.advance();
        while !self.at(&TokenKind::RBracket) && !self.at(&TokenKind::Eof) {
            self.advance();
        }
        if self.at(&TokenKind::RBracket) {
            self.advance();
        }
    }

    /// Step over the header of an ADT declaration: its `polydec` type
    /// parameters and an optional `for { ... }` clause
    /// (`adtdecl: ids ':' Ladt polydec '{' fields '}' forpoly` and
    /// `ids ':' Ladt polydec Lfor '{' tpolys '}' '{' fields '}'`,
    /// limbo.y:250-263). A module interface declares its ADTs with the same
    /// rule, so both places need this: module/tables.m:3 writes
    /// `Table: adt[T] {`, and module/alphabet.m:116 writes
    /// `Context: adt[V, M, Ectxt] for { ... } {`.
    fn skip_adt_header(&mut self) -> Result<(), ParseError> {
        self.skip_type_params();
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
        Ok(())
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
                    Ok(d) => decls.extend(d),
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
    ///
    /// A single `name, name2: ...` declaration yields one `Decl` per name.
    fn parse_top_decl(&mut self) -> Result<Vec<Decl>, ParseError> {
        let span = self.span();

        // Function definition: name(args) or Qualifier.name(args)
        // We need to look ahead to distinguish name: type from name(args)
        if let TokenKind::Ident(_) = self.peek() {
            // Look ahead: could be name(, name., name:, name,
            let la = self.look_ahead_after_ident();
            match la {
                LookAhead::FuncDef => return Ok(vec![self.parse_func_def()?]),
                LookAhead::ColonDecl => return self.parse_colon_decl(span),
                LookAhead::Assign => return Ok(vec![self.parse_top_assign(span)?]),
                LookAhead::DeclAssign => return Ok(vec![self.parse_top_decl_assign(span)?]),
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
    fn parse_colon_decl(&mut self, span: Span) -> Result<Vec<Decl>, ParseError> {
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
                Ok(names
                    .into_iter()
                    .map(|name| {
                        Decl::Const(ConstDecl {
                            name,
                            ty: None,
                            value: value.clone(),
                            span,
                        })
                    })
                    .collect())
            }
            TokenKind::Type => {
                self.advance();
                let ty = self.parse_type()?;
                self.expect_semi()?;
                Ok(names
                    .into_iter()
                    .map(|name| {
                        Decl::TypeAlias(TypeAliasDecl {
                            name,
                            ty: ty.clone(),
                            span,
                        })
                    })
                    .collect())
            }
            TokenKind::Module => {
                self.advance();
                self.expect(&TokenKind::LBrace)?;
                let members = self.parse_module_members()?;
                self.expect(&TokenKind::RBrace)?;
                self.expect_semi()?;
                Ok(names
                    .into_iter()
                    .map(|name| {
                        Decl::Module(ModuleDecl {
                            name,
                            members: members.clone(),
                            span,
                        })
                    })
                    .collect())
            }
            TokenKind::Adt => {
                self.advance();
                self.skip_adt_header()?;
                self.expect(&TokenKind::LBrace)?;
                let (members, pick) = self.parse_adt_members()?;
                self.expect(&TokenKind::RBrace)?;
                self.expect_semi()?;
                Ok(names
                    .into_iter()
                    .map(|name| {
                        Decl::Adt(AdtDecl {
                            name,
                            members: members.clone(),
                            pick: pick.clone(),
                            span,
                        })
                    })
                    .collect())
            }
            TokenKind::Import => {
                self.advance();
                let module = self.expect_ident()?;
                self.expect_semi()?;
                Ok(vec![Decl::Import(ImportDecl {
                    names,
                    module,
                    span,
                })])
            }
            TokenKind::Exception => {
                self.advance();
                // The parenthesized part of an exception declaration is a list
                // of types, not one type: `ids ':' Lexcept '(' tuplist ')' ';'`
                // (limbo.y:172-177). Reading only the first type rejected
                // `FIB: exception(int, int);` (appl/math/fibonacci.b:22) and
                // `Syntax: exception(string, big);` (appl/lib/sexprs.b:24).
                let ty = if self.at(&TokenKind::LParen) {
                    self.advance();
                    let mut types = vec![self.parse_type()?];
                    while self.at(&TokenKind::Comma) {
                        self.advance();
                        types.push(self.parse_type()?);
                    }
                    self.expect(&TokenKind::RParen)?;
                    if types.len() == 1 {
                        types.into_iter().next()
                    } else {
                        Some(Type::Tuple(types))
                    }
                } else {
                    None
                };
                self.expect_semi()?;
                Ok(names
                    .into_iter()
                    .map(|name| {
                        Decl::Exception(ExceptionDecl {
                            name,
                            ty: ty.clone(),
                            span,
                        })
                    })
                    .collect())
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
                Ok(vec![Decl::Var(VarDecl {
                    names,
                    ty: Some(ty),
                    init,
                    span,
                })])
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
                    members.extend(names.into_iter().map(|name| {
                        ModuleMember::Const(ConstDecl {
                            name,
                            ty: None,
                            value: value.clone(),
                            span,
                        })
                    }));
                }
                TokenKind::Type => {
                    self.advance();
                    let ty = self.parse_type()?;
                    self.expect_semi()?;
                    members.extend(names.into_iter().map(|name| {
                        ModuleMember::TypeAlias(TypeAliasDecl {
                            name,
                            ty: ty.clone(),
                            span,
                        })
                    }));
                }
                TokenKind::Fn => {
                    let sig = self.parse_func_sig(first_name(&names))?;
                    self.expect_semi()?;
                    members.extend(names.into_iter().map(|name| {
                        ModuleMember::Func(FuncSig {
                            name,
                            ..sig.clone()
                        })
                    }));
                }
                TokenKind::Adt => {
                    self.advance();
                    self.skip_adt_header()?;
                    self.expect(&TokenKind::LBrace)?;
                    let (adt_members, pick) = self.parse_adt_members()?;
                    self.expect(&TokenKind::RBrace)?;
                    self.expect_semi()?;
                    members.extend(names.into_iter().map(|name| {
                        ModuleMember::Adt(AdtDecl {
                            name,
                            members: adt_members.clone(),
                            pick: pick.clone(),
                            span,
                        })
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
                    members.extend(names.into_iter().map(|name| {
                        AdtMember::Const(ConstDecl {
                            name,
                            ty: None,
                            value: value.clone(),
                            span,
                        })
                    }));
                }
                TokenKind::Fn => {
                    let sig = self.parse_func_sig(first_name(&names))?;
                    self.expect_semi()?;
                    members.extend(names.into_iter().map(|name| {
                        AdtMember::Func(FuncSig {
                            name,
                            ..sig.clone()
                        })
                    }));
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
                // The fields of a pick case are `dfields`, so each one may be
                // marked `cyclic` (`dfield: ids ':' Lcyclic type ';'`,
                // limbo.y:292, reached through `pfields: pfbody dfields`,
                // limbo.y:312). module/json.m:8 declares
                // `mem: cyclic list of (string, ref JValue);`.
                if self.at(&TokenKind::Cyclic) {
                    self.advance();
                }
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
        self.skip_type_params();

        // Qualified name: A.B(
        while self.at(&TokenKind::Dot) {
            self.advance();
            qualifier = Some(name);
            name = self.expect_ident()?;
            // Skip polymorphic params after qualifier
            self.skip_type_params();
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
            self.skip_type_params();
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

            // Expect : type. The receiver of an ADT function member is
            // spelled `b: self ref Iobuf` — `self` sits between the colon and
            // the type, marking the parameter without changing it.
            self.expect(&TokenKind::Colon)?;
            let is_self = self.at(&TokenKind::Self_);
            if is_self {
                self.advance();
            }
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
            // `fn` in a type position keeps the polymorphic parameter list that
            // the grammar allows there: `type: Lfn polydec fnargretp raises`
            // (limbo.y:1368-1374). Consuming the keyword here instead of
            // leaving it to `parse_func_sig` skipped that list, so a type such
            // as `fn[T](x: T)` was rejected at the bracket.
            TokenKind::Fn => {
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
                self.skip_type_params();
                // Check for Module->Type or Type.SubType
                if self.at(&TokenKind::Arrow) || self.at(&TokenKind::Dot) {
                    self.advance();
                    let member = self.expect_ident()?;
                    // A qualified name carries its own type arguments:
                    // `type Lmdot Lid '[' types ']'` (limbo.y:38-42 of the type
                    // rules). module/alphabet.m:8 writes
                    // `chan of ref Proxy->Typescmd[ref Value]`.
                    self.skip_type_params();
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
            // `self` only marks a parameter as the receiver; the type that
            // follows is the parameter's own. Wrapping it in another `ref`
            // made `b: self ref Iobuf` a `ref ref Iobuf`, which named no ADT.
            TokenKind::Self_ => {
                self.advance();
                self.parse_type()?
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
                    if self.at_wildcard_arm() {
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
                        && !self.at_wildcard_arm()
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
            TokenKind::Star => return self.at_wildcard_arm(),
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
            let module = self.expect_ident()?;
            self.expect_semi()?;
            return Ok(Stmt::Import(ImportDecl {
                names,
                module,
                span,
            }));
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
            if self.at_wildcard_arm() {
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
        if self.at_wildcard_arm() {
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
            let guards = if self.at_wildcard_arm() {
                self.advance();
                vec![AltGuard::Wildcard]
            } else {
                // Guard list: `expr [or expr]* =>`. Every guard gets its own
                // entry in the alt table and they all share this arm's body,
                // so all of them are kept.
                let first = self.parse_expr()?;
                let mut guards = vec![self.classify_alt_guard(first)?];
                while self.at(&TokenKind::Or) {
                    self.advance();
                    let expr = self.parse_expr()?;
                    guards.push(self.classify_alt_guard(expr)?);
                }
                guards
            };
            self.expect(&TokenKind::FatArrow)?;
            let mut body = Vec::new();
            while !self.at(&TokenKind::RBrace)
                && !self.at(&TokenKind::Eof)
                && !self.is_alt_guard_start()
            {
                body.push(self.parse_stmt()?);
            }
            arms.push(AltArm { guards, body });
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(Stmt::Alt(AltStmt { arms, span }))
    }

    /// Turn a parsed `alt` guard expression into the send/receive form
    /// codegen needs. A guard that is neither is rejected here rather than
    /// carried along as a receive that isn't one.
    fn classify_alt_guard(&self, expr: Expr) -> Result<AltGuard, ParseError> {
        // Peel the binding off a receive guard: the destination is whatever
        // the `<-c` was assigned or declared into.
        let (dest, comm) = match expr {
            Expr::DeclAssign(names, rhs, _) => (Some(AltDest::Decl(names)), *rhs),
            Expr::TupleDeclAssign(names, rhs, _) => (Some(AltDest::TupleDecl(names)), *rhs),
            Expr::Assign(lhs, rhs, _) => (Some(AltDest::Assign(*lhs)), *rhs),
            other => (None, other),
        };
        match (dest, comm) {
            (None, Expr::Send(chan, val, _)) => Ok(AltGuard::Send(*chan, *val)),
            (dest, Expr::Recv(chan, _)) => Ok(AltGuard::Recv(dest, *chan)),
            _ => Err(self
                .err("an `alt` guard must be a channel send (`c <-= v`) or receive (`x := <-c`)")),
        }
    }

    fn is_alt_guard_start(&self) -> bool {
        if self.at_wildcard_arm() {
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

            // Channel send: `expr <-= expr`. The reference lexes `<-` as a
            // single token (`Lcomm`) and its grammar spells the send as
            // `exp Lcomm '=' exp` (lex.c:1041-1047, limbo.y:1127-1134), so
            // whitespace between the arrow and the `=` is legal and the
            // spaced form has to be accepted here too.
            if self.at(&TokenKind::ChanSend)
                || (self.at(&TokenKind::ChanRecv) && self.at_offset(1, &TokenKind::Assign))
            {
                if 1 < min_bp {
                    break;
                }
                if self.at(&TokenKind::ChanRecv) {
                    self.advance();
                }
                self.advance();
                let rhs = self.parse_expr_bp(1)?;
                lhs = Expr::Send(Box::new(lhs), Box::new(rhs), span);
                continue;
            }

            // Cons operator: binds tighter than `&&`, looser than `|`,
            // and is right-associative.
            if self.at(&TokenKind::ColonColon) {
                if CONS_BP < min_bp {
                    break;
                }
                self.advance();
                let rhs = self.parse_expr_bp(CONS_BP)?;
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
                let expr = self.parse_expr_bp(UNARY_BP)?;
                Ok(Expr::PostInc(Box::new(expr), span))
            }
            TokenKind::Dec => {
                // Pre-decrement: --x
                self.advance();
                let expr = self.parse_expr_bp(UNARY_BP)?;
                Ok(Expr::PostDec(Box::new(expr), span))
            }
            TokenKind::Plus => {
                // Unary plus
                self.advance();
                self.parse_expr_bp(UNARY_BP)
            }
            TokenKind::Star => {
                // Dereference: *expr
                self.advance();
                // Check if this is a wildcard in initializer context (followed by =>)
                if self.at(&TokenKind::FatArrow) {
                    return Ok(Expr::Ident("*".to_string(), span));
                }
                let expr = self.parse_expr_bp(UNARY_BP)?;
                Ok(Expr::Unary(UnaryOp::Ref, Box::new(expr), span)) // deref uses Ref variant for now
            }
            TokenKind::Minus => {
                self.advance();
                let expr = self.parse_expr_bp(UNARY_BP)?;
                Ok(Expr::Unary(UnaryOp::Neg, Box::new(expr), span))
            }
            TokenKind::Bang => {
                self.advance();
                let expr = self.parse_expr_bp(UNARY_BP)?;
                Ok(Expr::Unary(UnaryOp::Not, Box::new(expr), span))
            }
            TokenKind::Tilde => {
                self.advance();
                let expr = self.parse_expr_bp(UNARY_BP)?;
                Ok(Expr::Unary(UnaryOp::BitNot, Box::new(expr), span))
            }
            TokenKind::Hd => {
                self.advance();
                let expr = self.parse_expr_bp(UNARY_BP)?;
                Ok(Expr::Hd(Box::new(expr), span))
            }
            TokenKind::Tl => {
                self.advance();
                let expr = self.parse_expr_bp(UNARY_BP)?;
                Ok(Expr::Tl(Box::new(expr), span))
            }
            TokenKind::Len => {
                self.advance();
                let expr = self.parse_expr_bp(UNARY_BP)?;
                Ok(Expr::Len(Box::new(expr), span))
            }
            TokenKind::Tagof => {
                self.advance();
                let expr = self.parse_expr_bp(UNARY_BP)?;
                Ok(Expr::Tagof(Box::new(expr), span))
            }
            TokenKind::Ref => {
                self.advance();
                let expr = self.parse_expr_bp(UNARY_BP)?;
                Ok(Expr::Unary(UnaryOp::Ref, Box::new(expr), span))
            }
            TokenKind::ChanRecv => {
                // <-chan (receive)
                self.advance();
                let expr = self.parse_expr_bp(UNARY_BP)?;
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
                            let elems = self.parse_array_elem_list()?;
                            self.expect(&TokenKind::RBrace)?;
                            Ok(Expr::ArrayLit(None, elems, None, span))
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
                            let elems = self.parse_array_elem_list()?;
                            self.expect(&TokenKind::RBrace)?;
                            // The declared size is the array's length; the
                            // elements only say what goes where inside it.
                            Ok(Expr::ArrayLit(Some(Box::new(size)), elems, None, span))
                        } else {
                            let ty = self.parse_type()?;
                            Ok(Expr::ArrayAlloc(Box::new(size), Box::new(ty), span))
                        }
                    }
                } else {
                    self.expect(&TokenKind::Of)?;
                    let ty = self.parse_type()?;
                    // array of type monexp (cast)
                    let expr = self.parse_expr_bp(UNARY_BP)?;
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
                let expr = self.parse_expr_bp(UNARY_BP)?;
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

    /// Parse the element list of an array literal.
    fn parse_array_elem_list(&mut self) -> Result<Vec<ArrayElem>, ParseError> {
        let mut elems = Vec::new();
        if self.at(&TokenKind::RBrace) {
            return Ok(elems);
        }
        elems.push(self.parse_array_elem()?);
        while self.at(&TokenKind::Comma) {
            self.advance();
            if self.at(&TokenKind::RBrace) {
                break;
            }
            elems.push(self.parse_array_elem()?);
        }
        Ok(elems)
    }

    /// Parse one array-literal element: `expr`, `k => expr`, `k1 or k2 =>
    /// expr`, `lo to hi => expr`, or `* => expr`. The index selector is part of
    /// the element's meaning, so it is kept rather than discarded.
    fn parse_array_elem(&mut self) -> Result<ArrayElem, ParseError> {
        if self.at(&TokenKind::Star) {
            self.advance();
            self.expect(&TokenKind::FatArrow)?;
            return Ok(ArrayElem {
                index: Some(ArrayIndex::Wildcard),
                value: self.parse_expr()?,
            });
        }
        let first = self.parse_expr()?;
        if !self.at(&TokenKind::To) && !self.at(&TokenKind::Or) && !self.at(&TokenKind::FatArrow) {
            return Ok(ArrayElem {
                index: None,
                value: first,
            });
        }
        // A selector list: single indices and `lo to hi` ranges joined by `or`.
        let mut selectors = Vec::new();
        let mut lo = first;
        loop {
            let hi = if self.at(&TokenKind::To) {
                self.advance();
                Some(self.parse_expr()?)
            } else {
                None
            };
            selectors.push((lo, hi));
            if !self.at(&TokenKind::Or) {
                break;
            }
            self.advance();
            lo = self.parse_expr()?;
        }
        self.expect(&TokenKind::FatArrow)?;
        Ok(ArrayElem {
            index: Some(ArrayIndex::Selectors(selectors)),
            value: self.parse_expr()?,
        })
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
    ///
    /// Limbo precedence, loosest first: `||`, `&&`, `::`, `|`, `^`, `&`,
    /// equality, relational, shifts, additive, multiplicative, `**`.
    fn infix_binding_power(&self) -> Option<(u8, u8, BinOp)> {
        match self.peek() {
            TokenKind::OrOr => Some((3, 4, BinOp::LogOr)),
            TokenKind::AndAnd => Some((5, 6, BinOp::LogAnd)),
            // 7 is CONS_BP
            TokenKind::Pipe => Some((9, 10, BinOp::Or)),
            TokenKind::Caret => Some((11, 12, BinOp::Xor)),
            TokenKind::Amp => Some((13, 14, BinOp::And)),
            TokenKind::Eq => Some((15, 16, BinOp::Eq)),
            TokenKind::Neq => Some((15, 16, BinOp::Neq)),
            TokenKind::Lt => Some((17, 18, BinOp::Lt)),
            TokenKind::Gt => Some((17, 18, BinOp::Gt)),
            TokenKind::Leq => Some((17, 18, BinOp::Leq)),
            TokenKind::Geq => Some((17, 18, BinOp::Geq)),
            TokenKind::Lshift => Some((19, 20, BinOp::Lshift)),
            TokenKind::Rshift => Some((19, 20, BinOp::Rshift)),
            TokenKind::Plus => Some((21, 22, BinOp::Add)),
            TokenKind::Minus => Some((21, 22, BinOp::Sub)),
            TokenKind::Star => Some((23, 24, BinOp::Mul)),
            TokenKind::Slash => Some((23, 24, BinOp::Div)),
            TokenKind::Percent => Some((23, 24, BinOp::Mod)),
            TokenKind::Power => Some((26, 25, BinOp::Power)), // right-assoc
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

/// Binding power of `::`: below `|` (9) and above `&&` (5). Used as both the
/// left and the right binding power, which makes the operator right-associative.
const CONS_BP: u8 = 7;

/// Binding power of the operand of a prefix operator, and of the operand of a
/// cast. The reference grammar gives every prefix form its own `monexp`
/// operand (limbo.y:1228-1288 for the unary operators, limbo.y:1334-1355 for
/// the casts), and `monexp` cannot derive a binary expression. That puts every
/// prefix operator above `**` (`exp Lexp exp`, limbo.y:1147), so `-a ** b` is
/// `(-a) ** b`. A binding power of 25 would have let `**` (left power 26) pull
/// the exponentiation inside the operand instead.
const UNARY_BP: u8 = 27;

/// First name of a `a, b, c: ...` declaration group.
fn first_name(names: &[String]) -> String {
    names.first().cloned().unwrap_or_default()
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
        if let Stmt::Expr(Expr::Assign(_, rhs, _)) = &f.body.stmts[0] {
            assert!(matches!(rhs.as_ref(), Expr::Binary(_, BinOp::Add, _, _)));
        }
    }

    #[test]
    fn parse_do_while() {
        let file = parse(
            r#"implement T;
test()
{
    do { x++; } while(x < 10);
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        assert!(matches!(f.body.stmts[0], Stmt::Do(_)));
    }

    #[test]
    fn parse_channel_send() {
        let file = parse(
            r#"implement T;
test()
{
    c <-= 42;
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        assert!(matches!(f.body.stmts[0], Stmt::Expr(Expr::Send(_, _, _))));
    }

    #[test]
    fn parse_spawn() {
        let file = parse(
            r#"implement T;
test()
{
    spawn worker(c);
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        assert!(matches!(f.body.stmts[0], Stmt::Spawn(_, _)));
    }

    #[test]
    fn parse_raise() {
        let file = parse(
            r#"implement T;
test()
{
    raise "fail:error";
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        assert!(matches!(f.body.stmts[0], Stmt::Raise(Some(_), _)));
    }

    #[test]
    fn parse_tuple_decl_assign() {
        let file = parse(
            r#"implement T;
test()
{
    (a, b) := func();
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        assert!(matches!(
            f.body.stmts[0],
            Stmt::Expr(Expr::TupleDeclAssign(_, _, _))
        ));
    }

    #[test]
    fn parse_local_var_decl() {
        let file = parse(
            r#"implement T;
test()
{
    x: int;
    y: string = "hello";
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        assert!(matches!(f.body.stmts[0], Stmt::VarDecl(_)));
        assert!(matches!(f.body.stmts[1], Stmt::VarDecl(_)));
    }

    #[test]
    fn parse_label_statement() {
        let file = parse(
            r#"implement T;
test()
{
    loop: for(;;) break;
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        assert!(matches!(f.body.stmts[0], Stmt::Label(_, _)));
    }

    #[test]
    fn parse_alt_statement() {
        let file = parse(
            r#"implement T;
test()
{
    alt {
        v := <-c1 =>
            x = v;
        * =>
            x = 0;
    }
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        assert!(matches!(f.body.stmts[0], Stmt::Alt(_)));
    }

    #[test]
    fn parse_pick_statement() {
        let file = parse(
            r#"implement T;
test()
{
    pick x := val {
        A =>
            y = 1;
        B =>
            y = 2;
    }
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        assert!(matches!(f.body.stmts[0], Stmt::Pick(_)));
    }

    #[test]
    fn parse_import_decl() {
        let file = parse(
            r#"implement T;
    Iobuf: import bufio;
"#,
        );
        assert!(matches!(file.decls[0], Decl::Import(_)));
    }

    #[test]
    fn parse_exception_decl() {
        let file = parse(
            r#"implement T;
    BadVal: exception;
"#,
        );
        assert!(matches!(file.decls[0], Decl::Exception(_)));
    }

    #[test]
    fn parse_adt_decl() {
        let file = parse(
            r#"implement T;
Point: adt {
    x: int;
    y: int;
};
"#,
        );
        assert!(matches!(file.decls[0], Decl::Adt(_)));
    }

    #[test]
    fn parse_const_decl() {
        let file = parse(
            r#"implement T;
    MAX: con 100;
"#,
        );
        assert!(matches!(file.decls[0], Decl::Const(_)));
    }

    #[test]
    fn parse_qualified_func_def() {
        let file = parse(
            r#"implement T;
Point.distance(p: ref Point): int
{
    return 0;
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        assert_eq!(f.name.qualifier, Some("Point".to_string()));
        assert_eq!(f.name.name, "distance");
    }

    #[test]
    fn parse_multiple_functions() {
        let file = parse(
            r#"implement T;
include "sys.m"; sys: Sys;
include "draw.m";
T: module { init: fn(nil: ref Draw->Context, nil: list of string); };
helper(): int { return 42; }
init(nil: ref Draw->Context, nil: list of string) { x := helper(); }
"#,
        );
        let func_count = file
            .decls
            .iter()
            .filter(|d| matches!(d, Decl::Func(_)))
            .count();
        assert_eq!(func_count, 2);
    }

    #[test]
    fn parse_deref_expression() {
        let file = parse(
            r#"implement T;
test()
{
    x = *p;
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        if let Stmt::Expr(Expr::Assign(_, rhs, _)) = &f.body.stmts[0] {
            assert!(matches!(rhs.as_ref(), Expr::Unary(_, _, _)));
        }
    }

    #[test]
    fn parse_list_cons() {
        let file = parse(
            r#"implement T;
test()
{
    l := 1 :: 2 :: nil;
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        assert!(matches!(
            f.body.stmts[0],
            Stmt::Expr(Expr::DeclAssign(_, _, _))
        ));
    }

    #[test]
    fn parse_chan_alloc() {
        let file = parse(
            r#"implement T;
test()
{
    c := chan of int;
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        if let Stmt::Expr(Expr::DeclAssign(_, rhs, _)) = &f.body.stmts[0] {
            assert!(matches!(rhs.as_ref(), Expr::ChanAlloc(_, _)));
        }
    }

    #[test]
    fn parse_array_alloc() {
        let file = parse(
            r#"implement T;
test()
{
    a := array[10] of int;
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        if let Stmt::Expr(Expr::DeclAssign(_, rhs, _)) = &f.body.stmts[0] {
            assert!(matches!(rhs.as_ref(), Expr::ArrayAlloc(_, _, _)));
        }
    }

    #[test]
    fn parse_hd_tl_len() {
        let file = parse(
            r#"implement T;
test()
{
    x := hd l;
    y := tl l;
    n := len s;
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        assert_eq!(f.body.stmts.len(), 3);
    }

    #[test]
    fn parse_multi_name_const_and_type_decls() {
        let file = parse(
            r#"implement T;
A, B: con 7;
X, Y: type int;
"#,
        );
        let names: Vec<&str> = file
            .decls
            .iter()
            .map(|d| match d {
                Decl::Const(c) => c.name.as_str(),
                Decl::TypeAlias(t) => t.name.as_str(),
                other => panic!("unexpected decl: {other:?}"),
            })
            .collect();
        assert_eq!(names, vec!["A", "B", "X", "Y"]);
    }

    #[test]
    fn parse_multi_name_module_members() {
        let file = parse(
            r#"implement T;
T: module {
    A, B: con 1;
    X, Y: type int;
};
"#,
        );
        let Decl::Module(m) = &file.decls[0] else {
            panic!("expected module");
        };
        let names: Vec<&str> = m
            .members
            .iter()
            .map(|mem| match mem {
                ModuleMember::Const(c) => c.name.as_str(),
                ModuleMember::TypeAlias(t) => t.name.as_str(),
                other => panic!("unexpected member: {other:?}"),
            })
            .collect();
        assert_eq!(names, vec!["A", "B", "X", "Y"]);
    }

    #[test]
    fn parse_multi_name_adt_members() {
        let file = parse(
            r#"implement T;
T: adt {
    A, B: con 1;
};
"#,
        );
        let Decl::Adt(a) = &file.decls[0] else {
            panic!("expected adt");
        };
        let names: Vec<&str> = a
            .members
            .iter()
            .map(|mem| match mem {
                AdtMember::Const(c) => c.name.as_str(),
                other => panic!("unexpected member: {other:?}"),
            })
            .collect();
        assert_eq!(names, vec!["A", "B"]);
    }

    #[test]
    fn cons_binds_tighter_than_logical_and() {
        let file = parse(
            r#"implement T;
test()
{
    x = a && b :: c;
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        let Stmt::Expr(Expr::Assign(_, rhs, _)) = &f.body.stmts[0] else {
            panic!("expected assignment");
        };
        // Must parse as `a && (b :: c)`, not `(a && b) :: c`.
        let Expr::Binary(lhs, BinOp::LogAnd, and_rhs, _) = rhs.as_ref() else {
            panic!("expected && at the root, got {rhs:?}");
        };
        assert!(matches!(lhs.as_ref(), Expr::Ident(n, _) if n == "a"));
        assert!(matches!(and_rhs.as_ref(), Expr::Cons(_, _, _)));
    }

    #[test]
    fn cons_binds_looser_than_bitwise_or() {
        let file = parse(
            r#"implement T;
test()
{
    x = a | b :: c;
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        let Stmt::Expr(Expr::Assign(_, rhs, _)) = &f.body.stmts[0] else {
            panic!("expected assignment");
        };
        // Must parse as `(a | b) :: c`.
        let Expr::Cons(head, tail, _) = rhs.as_ref() else {
            panic!("expected :: at the root, got {rhs:?}");
        };
        assert!(matches!(head.as_ref(), Expr::Binary(_, BinOp::Or, _, _)));
        assert!(matches!(tail.as_ref(), Expr::Ident(n, _) if n == "c"));
    }

    #[test]
    fn cons_is_right_associative() {
        let file = parse(
            r#"implement T;
test()
{
    x = a :: b :: c;
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        let Stmt::Expr(Expr::Assign(_, rhs, _)) = &f.body.stmts[0] else {
            panic!("expected assignment");
        };
        let Expr::Cons(head, tail, _) = rhs.as_ref() else {
            panic!("expected :: at the root, got {rhs:?}");
        };
        assert!(matches!(head.as_ref(), Expr::Ident(n, _) if n == "a"));
        assert!(matches!(tail.as_ref(), Expr::Cons(_, _, _)));
    }

    #[test]
    fn parse_load_expression() {
        let file = parse(
            r#"implement T;
test()
{
    sys = load Sys Sys->PATH;
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        if let Stmt::Expr(Expr::Assign(_, rhs, _)) = &f.body.stmts[0] {
            assert!(matches!(rhs.as_ref(), Expr::Load(_, _, _)));
        }
    }

    /// Pull the single `alt` statement out of a one-function file.
    fn alt_of(src: &str) -> AltStmt {
        let file = parse(src);
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        match &f.body.stmts[0] {
            Stmt::Alt(a) => a.clone(),
            other => panic!("expected alt, got {other:?}"),
        }
    }

    /// `x := <-c1 or x = <-c2 or x = <-c3 =>` names three channels, and an
    /// `alt` that listens on one of them is a different program. The guards
    /// after the first used to be parsed and dropped on the floor.
    #[test]
    fn alt_arm_retains_every_or_joined_guard() {
        let alt = alt_of(
            r#"implement T;
test(c1: chan of int, c2: chan of int, c3: chan of int)
{
    alt {
    x := <-c1 or
    x = <-c2 or
    x = <-c3 =>
        y = x;
    }
}
"#,
        );
        assert_eq!(alt.arms.len(), 1, "one arm");
        let text = format!("{:?}", alt.arms[0].guards);
        for chan in ["c1", "c2", "c3"] {
            assert!(
                text.contains(chan),
                "guard for {chan} was discarded: {text}"
            );
        }
        assert_eq!(alt.arms[0].guards.len(), 3, "three guards share one body");
    }

    /// Guards are classified at parse time, so codegen never has to guess
    /// whether `Recv` really holds a send.
    #[test]
    fn alt_classifies_send_and_recv_and_wildcard_guards() {
        let alt = alt_of(
            r#"implement T;
test(c: chan of int, d: chan of int)
{
    alt {
    c <-= 1 =>
        x = 1;
    y := <-d =>
        x = y;
    * =>
        x = 3;
    }
}
"#,
        );
        assert_eq!(alt.arms.len(), 3);
        assert!(
            matches!(alt.arms[0].guards[0], AltGuard::Send(_, _)),
            "`c <-= 1` is a send guard, got {:?}",
            alt.arms[0].guards[0]
        );
        assert!(
            matches!(alt.arms[1].guards[0], AltGuard::Recv(_, _)),
            "`y := <-d` is a recv guard, got {:?}",
            alt.arms[1].guards[0]
        );
        assert!(matches!(alt.arms[2].guards[0], AltGuard::Wildcard));
    }

    /// `c <- = v` — a space between `<-` and `=` — is a channel send. The
    /// reference lexes `<-` as `Lcomm` and the grammar accepts
    /// `exp Lcomm '=' exp` (lex.c:1041-1047, limbo.y:1127-1134).
    #[test]
    fn chan_send_accepts_space_between_arrow_and_equals() {
        let file = parse(
            r#"implement T;
test(c: chan of int)
{
    c <- = 1;
}
"#,
        );
        let Decl::Func(f) = &file.decls[0] else {
            panic!("expected func");
        };
        assert!(
            matches!(&f.body.stmts[0], Stmt::Expr(Expr::Send(_, _, _))),
            "expected a send, got {:?}",
            f.body.stmts[0]
        );
    }

    /// The spaced form is a send in guard position too — that is where the
    /// corpus actually uses it.
    #[test]
    fn chan_send_with_space_parses_in_an_alt_guard() {
        let alt = alt_of(
            r#"implement T;
test(c: chan of int)
{
    alt {
    c <- = 1 =>
        x = 1;
    }
}
"#,
        );
        assert!(matches!(alt.arms[0].guards[0], AltGuard::Send(_, _)));
    }

    // ── Shape helpers ──────────────────────────────────────────

    /// Message of the diagnostic a malformed source produces.
    fn parse_err(src: &str) -> String {
        let tokens = Lexer::new(src, "<test>")
            .tokenize()
            .expect("lex should succeed");
        Parser::new(tokens, "<test>")
            .parse_file()
            .expect_err("parse should fail")
            .message
    }

    /// The one function declared by a single-function source.
    fn func_of(src: &str) -> FuncDecl {
        let file = parse(src);
        for decl in &file.decls {
            if let Decl::Func(f) = decl {
                return f.clone();
            }
        }
        panic!("no function declared by {src}");
    }

    /// Statements of the body of the one function in `src`.
    fn stmts_of(src: &str) -> Vec<Stmt> {
        func_of(src).body.stmts
    }

    /// Parse one expression by putting it in statement position.
    fn expr_of(src: &str) -> Expr {
        let text = format!("implement T;\ntest()\n{{\n\t{src};\n}}\n");
        let stmts = stmts_of(&text);
        assert_eq!(stmts.len(), 1, "expected one statement from `{src}`");
        match &stmts[0] {
            Stmt::Expr(e) => e.clone(),
            other => panic!("expected an expression statement from `{src}`, got {other:?}"),
        }
    }

    fn binop_name(op: BinOp) -> &'static str {
        match op {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::Power => "**",
            BinOp::And => "&",
            BinOp::Or => "|",
            BinOp::Xor => "^",
            BinOp::Lshift => "<<",
            BinOp::Rshift => ">>",
            BinOp::Eq => "==",
            BinOp::Neq => "!=",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::Leq => "<=",
            BinOp::Geq => ">=",
            BinOp::LogAnd => "&&",
            BinOp::LogOr => "||",
        }
    }

    fn unop_name(op: UnaryOp) -> &'static str {
        match op {
            UnaryOp::Neg => "-",
            UnaryOp::Not => "!",
            UnaryOp::BitNot => "~",
            UnaryOp::Ref => "ref",
        }
    }

    /// Render a type as a one-line form for shape assertions.
    fn ty_shape(ty: &Type) -> String {
        match ty {
            Type::Basic(BasicType::Int) => "int".to_string(),
            Type::Basic(BasicType::Byte) => "byte".to_string(),
            Type::Basic(BasicType::Big) => "big".to_string(),
            Type::Basic(BasicType::Real) => "real".to_string(),
            Type::Basic(BasicType::String) => "string".to_string(),
            Type::Array(t) => format!("(array {})", ty_shape(t)),
            Type::List(t) => format!("(list {})", ty_shape(t)),
            Type::Chan(t) => format!("(chan {})", ty_shape(t)),
            Type::BufChan(n, t) => format!("(bufchan {} {})", sexp(n), ty_shape(t)),
            Type::Ref(t) => format!("(ref {})", ty_shape(t)),
            Type::Tuple(ts) => {
                let parts: Vec<String> = ts.iter().map(ty_shape).collect();
                format!("(tuple {})", parts.join(" "))
            }
            Type::Func(sig) => {
                let params: Vec<String> = sig.params.iter().map(|p| ty_shape(&p.ty)).collect();
                let ret = sig.ret.as_ref().map(ty_shape).unwrap_or_default();
                format!("(fn [{}] {ret})", params.join(" "))
            }
            Type::Named(q) => match &q.qualifier {
                Some(m) => format!("{m}->{}", q.name),
                None => q.name.clone(),
            },
            Type::Module(_) => "module".to_string(),
        }
    }

    /// Render an expression as a fully parenthesized form. A precedence or
    /// associativity change anywhere in the table then shows up as one
    /// specific mismatch rather than as a test that still passes.
    fn sexp(e: &Expr) -> String {
        let joined = |items: &[Expr]| -> String {
            items
                .iter()
                .map(sexp)
                .map(|s| format!(" {s}"))
                .collect::<String>()
        };
        match e {
            Expr::IntLit(v, _) => v.to_string(),
            Expr::RealLit(v, _) => format!("{v:?}"),
            Expr::StringLit(s, _) => format!("{s:?}"),
            Expr::CharLit(v, _) => format!("char:{v}"),
            Expr::Nil(_) => "nil".to_string(),
            Expr::Ident(n, _) => n.clone(),
            Expr::Binary(l, op, r, _) => {
                format!("({} {} {})", binop_name(*op), sexp(l), sexp(r))
            }
            Expr::Unary(op, x, _) => format!("({} {})", unop_name(*op), sexp(x)),
            Expr::Call(f, args, _) => format!("(call {}{})", sexp(f), joined(args)),
            Expr::Dot(x, m, _) => format!("(dot {} {m})", sexp(x)),
            Expr::ModQual(x, m, _) => format!("(mdot {} {m})", sexp(x)),
            Expr::Index(x, i, _) => format!("(index {} {})", sexp(x), sexp(i)),
            Expr::Slice(x, lo, hi, _) => {
                let part = |p: &Option<Box<Expr>>| match p {
                    Some(e) => sexp(e),
                    None => "_".to_string(),
                };
                format!("(slice {} {} {})", sexp(x), part(lo), part(hi))
            }
            Expr::Tuple(xs, _) => format!("(tuple{})", joined(xs)),
            Expr::Cons(h, t, _) => format!("(:: {} {})", sexp(h), sexp(t)),
            Expr::Recv(c, _) => format!("(<- {})", sexp(c)),
            Expr::Send(c, v, _) => format!("(<-= {} {})", sexp(c), sexp(v)),
            Expr::Load(ty, p, _) => format!("(load {} {})", ty_shape(ty), sexp(p)),
            Expr::ArrayAlloc(n, ty, _) => format!("(arrayalloc {} {})", sexp(n), ty_shape(ty)),
            Expr::ArrayLit(n, elems, _, _) => {
                let size = match n {
                    Some(e) => sexp(e),
                    None => "_".to_string(),
                };
                let parts: Vec<String> = elems.iter().map(elem_shape).collect();
                format!("(arraylit {size} {})", parts.join(" "))
            }
            Expr::ChanAlloc(ty, _) => format!("(chanalloc {})", ty_shape(ty)),
            Expr::ListLit(xs, _) => format!("(listlit{})", joined(xs)),
            Expr::RefAlloc(ty, args, _) => {
                format!("(refalloc {}{})", ty_shape(ty), joined(args))
            }
            Expr::Cast(ty, x, _) => format!("(cast {} {})", ty_shape(ty), sexp(x)),
            Expr::DeclAssign(names, x, _) => {
                format!("(:= [{}] {})", names.join(","), sexp(x))
            }
            Expr::TupleDeclAssign(names, x, _) => {
                format!("(tuple:= [{}] {})", names.join(","), sexp(x))
            }
            Expr::Assign(l, r, _) => format!("(= {} {})", sexp(l), sexp(r)),
            Expr::CompoundAssign(l, op, r, _) => {
                format!("({}= {} {})", binop_name(*op), sexp(l), sexp(r))
            }
            Expr::Hd(x, _) => format!("(hd {})", sexp(x)),
            Expr::Tl(x, _) => format!("(tl {})", sexp(x)),
            Expr::Len(x, _) => format!("(len {})", sexp(x)),
            Expr::Tagof(x, _) => format!("(tagof {})", sexp(x)),
            Expr::PostInc(x, _) => format!("(++ {})", sexp(x)),
            Expr::PostDec(x, _) => format!("(-- {})", sexp(x)),
        }
    }

    /// Render one array-literal element, index selector included.
    fn elem_shape(elem: &ArrayElem) -> String {
        match &elem.index {
            None => sexp(&elem.value),
            Some(ArrayIndex::Wildcard) => format!("(* => {})", sexp(&elem.value)),
            Some(ArrayIndex::Selectors(sels)) => {
                let parts: Vec<String> = sels
                    .iter()
                    .map(|(lo, hi)| match hi {
                        Some(h) => format!("({} to {})", sexp(lo), sexp(h)),
                        None => sexp(lo),
                    })
                    .collect();
                format!("([{}] => {})", parts.join(" "), sexp(&elem.value))
            }
        }
    }

    /// Every rung of the precedence ladder, in both directions, plus the
    /// associativity of each rung. The reference declares the ladder in
    /// limbo.y:33-53: assignment, `load`, `||`, `&&`, `::`, `|`, `^`, `&`,
    /// equality, relational, shifts, additive, multiplicative, `**`, and then
    /// the postfix forms.
    const PRECEDENCE_TABLE: &[(&str, &str)] = &[
        // Logical or and and.
        ("a || b && c", "(|| a (&& b c))"),
        ("a && b || c", "(|| (&& a b) c)"),
        ("a || b || c", "(|| (|| a b) c)"),
        ("a && b && c", "(&& (&& a b) c)"),
        // Cons sits between `&&` and `|`, and is right-associative.
        ("a && b :: c", "(&& a (:: b c))"),
        ("a :: b && c", "(&& (:: a b) c)"),
        ("a :: b || c", "(|| (:: a b) c)"),
        ("a | b :: c", "(:: (| a b) c)"),
        ("a :: b | c", "(:: a (| b c))"),
        ("a :: b :: c", "(:: a (:: b c))"),
        ("a || b :: c && d", "(|| a (&& (:: b c) d))"),
        // Bitwise or, xor, and and.
        ("a | b ^ c", "(| a (^ b c))"),
        ("a ^ b | c", "(| (^ a b) c)"),
        ("a ^ b & c", "(^ a (& b c))"),
        ("a & b ^ c", "(^ (& a b) c)"),
        ("a | b | c", "(| (| a b) c)"),
        // Equality and relational.
        ("a & b == c", "(& a (== b c))"),
        ("a == b != c", "(!= (== a b) c)"),
        ("a == b < c", "(== a (< b c))"),
        ("a < b > c", "(> (< a b) c)"),
        ("a <= b >= c", "(>= (<= a b) c)"),
        // Shifts, additive, and multiplicative.
        ("a < b << c", "(< a (<< b c))"),
        ("a << b >> c", "(>> (<< a b) c)"),
        ("a << b + c", "(<< a (+ b c))"),
        ("a + b - c", "(- (+ a b) c)"),
        ("a + b * c", "(+ a (* b c))"),
        ("a * b + c", "(+ (* a b) c)"),
        ("a * b / c % d", "(% (/ (* a b) c) d)"),
        ("1 + 2 * 3 ** 4 - 5", "(- (+ 1 (* 2 (** 3 4))) 5)"),
        // Exponentiation binds tighter than multiplication and is
        // right-associative (limbo.y:47).
        ("a * b ** c", "(* a (** b c))"),
        ("a ** b ** c", "(** a (** b c))"),
        ("a ** b * c", "(* (** a b) c)"),
        // Prefix operators take a `monexp`, so they bind tighter than `**`.
        ("-a ** b", "(** (- a) b)"),
        ("a ** -b", "(** a (- b))"),
        ("len a ** b", "(** (len a) b)"),
        ("<-c ** 2", "(** (<- c) 2)"),
        ("!a && b", "(&& (! a) b)"),
        ("-a * b", "(* (- a) b)"),
        ("~a | b", "(| (~ a) b)"),
        ("- -a", "(- (- a))"),
        ("!!a", "(! (! a))"),
        ("+a + b", "(+ a b)"),
        ("hd a :: b", "(:: (hd a) b)"),
        ("hd tl a", "(hd (tl a))"),
        ("len a + 1", "(+ (len a) 1)"),
        ("tagof x == tagof y", "(== (tagof x) (tagof y))"),
        ("-f(x)", "(- (call f x))"),
        ("-a[i]", "(- (index a i))"),
        ("ref X(1)", "(ref (call X 1))"),
        ("*p = 1", "(= (ref p) 1)"),
        // Assignment is right-associative and looser than every operator.
        ("a = b = c", "(= a (= b c))"),
        ("a = b || c", "(= a (|| b c))"),
        ("a = b :: c", "(= a (:: b c))"),
        ("a += b + c", "(+= a (+ b c))"),
        ("a -= b", "(-= a b)"),
        ("a *= b", "(*= a b)"),
        ("a /= b", "(/= a b)"),
        ("a %= b", "(%= a b)"),
        ("a &= b", "(&= a b)"),
        ("a |= b", "(|= a b)"),
        ("a ^= b", "(^= a b)"),
        ("a <<= b", "(<<= a b)"),
        ("a >>= b", "(>>= a b)"),
        ("x := a || b", "(:= [x] (|| a b))"),
        ("(x, y) := f()", "(tuple:= [x,y] (call f))"),
        ("(x, nil) := f()", "(tuple:= [x,nil] (call f))"),
        // Channel communication.
        ("c <-= a + b", "(<-= c (+ a b))"),
        ("c <- = 1", "(<-= c 1)"),
        ("<-c + 1", "(+ (<- c) 1)"),
        ("x = <-c", "(= x (<- c))"),
        ("c[i] <-= 1", "(<-= (index c i) 1)"),
        // Postfix forms bind tightest of all (limbo.y:1358-1405).
        ("a.b.c", "(dot (dot a b) c)"),
        ("m->f(x)", "(call (mdot m f) x)"),
        ("a[0][1]", "(index (index a 0) 1)"),
        ("a.b[0].c", "(dot (index (dot a b) 0) c)"),
        ("a[1:2]", "(slice a 1 2)"),
        ("a[:2]", "(slice a _ 2)"),
        ("a[1:]", "(slice a 1 _)"),
        ("a[:]", "(slice a _ _)"),
        ("a[i+1:j-1]", "(slice a (+ i 1) (- j 1))"),
        ("a++ + b", "(+ (++ a) b)"),
        ("a-- - b", "(- (-- a) b)"),
        ("++a.b", "(++ (dot a b))"),
        ("--a", "(-- a)"),
        ("f(a, b)(c)", "(call (call f a b) c)"),
        ("f()", "(call f)"),
        ("(a + b) * c", "(* (+ a b) c)"),
        ("(a)", "a"),
        ("(a, b)", "(tuple a b)"),
        ("(a, b, )", "(tuple a b)"),
        // Casts take a `monexp` too (limbo.y:1334-1355).
        ("int x + 1", "(+ (cast int x) 1)"),
        ("big 1 ** 2", "(** (cast big 1) 2)"),
        ("real n / 2.0", "(/ (cast real n) 2.0)"),
        ("byte 65", "(cast byte 65)"),
        ("string x", "(cast string x)"),
        ("array of byte s", "(cast (array byte) s)"),
        // Literals and constructors.
        ("nil :: nil", "(:: nil nil)"),
        ("'a' + 1", "(+ char:97 1)"),
        ("1.5 * 2.", "(* 1.5 2.0)"),
        ("s + \"x\"", "(+ s \"x\")"),
        ("iota", "iota"),
        ("array[10] of int", "(arrayalloc 10 int)"),
        ("array[] of int", "(arrayalloc 0 int)"),
        ("array[] of {1, 2}", "(arraylit _ 1 2)"),
        ("array[4] of {1, 2}", "(arraylit 4 1 2)"),
        (
            "array[4] of {2 => 1, * => 0}",
            "(arraylit 4 ([2] => 1) (* => 0))",
        ),
        (
            "array[26] of {'a' to 'z' or '_' => 1}",
            "(arraylit 26 ([(char:97 to char:122) char:95] => 1))",
        ),
        ("chan of int", "(chanalloc int)"),
        ("list of {1, 2, 3}", "(listlit 1 2 3)"),
        ("list of {}", "(listlit)"),
        ("load Sys Sys->PATH", "(load Sys (mdot Sys PATH))"),
        ("load Sys path + x", "(load Sys (+ path x))"),
    ];

    #[test]
    fn expression_shapes_match_the_reference_precedence_table() {
        for (src, want) in PRECEDENCE_TABLE {
            assert_eq!(sexp(&expr_of(src)), *want, "shape of `{src}`");
        }
    }

    /// `chan[n] of T` carries a buffer size in the reference, which keeps it as
    /// the size child of its `Ochan` node (limbo.y:1327-1332). `Expr::ChanAlloc`
    /// has nowhere to put it, so the size is parsed and dropped and the channel
    /// comes out unbuffered. Recorded as a known gap: closing it needs a field
    /// on the AST node and a codegen change.
    #[test]
    fn buffered_chan_alloc_drops_its_size() {
        assert_eq!(sexp(&expr_of("chan[10] of int")), "(chanalloc int)");
    }

    /// The reference lexes `**=` as one token and reduces `exp Lexpeq exp`
    /// (lex.c:113, limbo.y:1123). This front end has no such token, so the
    /// statement is rejected in the expression that follows `**`.
    #[test]
    fn power_assign_is_rejected() {
        let msg = parse_err("implement T;\ntest()\n{\n\tx **= 2;\n}\n");
        assert!(
            msg.contains("unexpected token in expression: Assign"),
            "unexpected message: {msg}"
        );
    }

    // ── Types ──────────────────────────────────────────────────

    /// Parse a type by putting it in a top-level variable declaration.
    fn ty_of(src: &str) -> Type {
        let file = parse(&format!("implement T;\nx: {src};\n"));
        match &file.decls[0] {
            Decl::Var(v) => v.ty.clone().expect("declaration should carry a type"),
            other => panic!("expected a variable declaration, got {other:?}"),
        }
    }

    #[test]
    fn type_shapes() {
        // `Foo.Bar` and `Foo->Bar` land in the same qualified node, which is
        // why both render with an arrow.
        let table: &[(&str, &str)] = &[
            ("int", "int"),
            ("byte", "byte"),
            ("big", "big"),
            ("real", "real"),
            ("string", "string"),
            ("array of int", "(array int)"),
            ("array of array of byte", "(array (array byte))"),
            ("list of ref Foo", "(list (ref Foo))"),
            ("chan of list of string", "(chan (list string))"),
            ("ref Draw->Context", "(ref Draw->Context)"),
            ("(int, string)", "(tuple int string)"),
            ("(int, (byte, real))", "(tuple int (tuple byte real))"),
            ("(int)", "int"),
            ("fn(x: int): int", "(fn [int] int)"),
            ("fn(): string", "(fn [] string)"),
            ("fn(x: int, y: string)", "(fn [int string] )"),
            ("Sys", "Sys"),
            ("Sys->FD", "Sys->FD"),
            ("Foo.Bar", "Foo->Bar"),
            ("Set[int]", "Set"),
        ];
        for (src, want) in table {
            assert_eq!(ty_shape(&ty_of(src)), *want, "type `{src}`");
        }
    }

    // ── Declarations ───────────────────────────────────────────

    #[test]
    fn multi_name_variable_declaration_keeps_every_name() {
        let file = parse("implement T;\na, b, c: int;\n");
        assert_eq!(file.decls.len(), 1);
        let Decl::Var(v) = &file.decls[0] else {
            panic!("expected a variable declaration");
        };
        assert_eq!(v.names, vec!["a", "b", "c"]);
        assert!(v.init.is_none());
    }

    #[test]
    fn variable_declaration_with_initializer() {
        let file = parse("implement T;\nn: int = 1 + 2;\n");
        let Decl::Var(v) = &file.decls[0] else {
            panic!("expected a variable declaration");
        };
        assert_eq!(v.names, vec!["n"]);
        assert_eq!(sexp(v.init.as_ref().expect("initializer")), "(+ 1 2)");
    }

    /// `A, B: con iota;` declares both constants. Every name after the first
    /// used to be dropped, which silently renumbered the enumeration.
    #[test]
    fn multi_name_constant_declaration_with_iota() {
        let file = parse("implement T;\nA, B, C: con iota;\n");
        let names: Vec<&str> = file
            .decls
            .iter()
            .map(|d| match d {
                Decl::Const(c) => {
                    assert_eq!(sexp(&c.value), "iota");
                    c.name.as_str()
                }
                other => panic!("unexpected declaration: {other:?}"),
            })
            .collect();
        assert_eq!(names, vec!["A", "B", "C"]);
    }

    /// `E: exception (T1, T2)` names a list of types (limbo.y:176). Reading
    /// only the first one rejected appl/math/fibonacci.b:22.
    #[test]
    fn exception_declarations_with_and_without_a_type() {
        let file = parse(
            "implement T;\nE: exception;\nA, B: exception (string, int);\nS: exception (string);\n",
        );
        let Decl::Exception(bare) = &file.decls[0] else {
            panic!("expected an exception declaration");
        };
        assert_eq!(bare.name, "E");
        assert!(bare.ty.is_none());
        let typed: Vec<(&str, String)> = file.decls[1..]
            .iter()
            .map(|d| match d {
                Decl::Exception(e) => (
                    e.name.as_str(),
                    ty_shape(e.ty.as_ref().expect("exception type")),
                ),
                other => panic!("unexpected declaration: {other:?}"),
            })
            .collect();
        // A one-type list stays that type; it does not become a one-tuple.
        assert_eq!(
            typed,
            vec![
                ("A", "(tuple string int)".to_string()),
                ("B", "(tuple string int)".to_string()),
                ("S", "string".to_string()),
            ]
        );
    }

    #[test]
    fn import_declaration_keeps_every_imported_name() {
        let file = parse("implement T;\nIobuf, Iobufio: import bufio;\n");
        assert_eq!(file.decls.len(), 1);
        let Decl::Import(i) = &file.decls[0] else {
            panic!("expected an import declaration");
        };
        assert_eq!(i.names, vec!["Iobuf", "Iobufio"]);
        assert_eq!(i.module, "bufio");
    }

    #[test]
    fn type_alias_declaration() {
        let file = parse("implement T;\nP: type ref Point;\n");
        let Decl::TypeAlias(t) = &file.decls[0] else {
            panic!("expected a type alias");
        };
        assert_eq!(t.name, "P");
        assert_eq!(ty_shape(&t.ty), "(ref Point)");
    }

    #[test]
    fn top_level_assignment_forms() {
        let file = parse("implement T;\nx = 1;\ny := 2;\n");
        let names: Vec<(&str, String)> = file
            .decls
            .iter()
            .map(|d| match d {
                Decl::Var(v) => (
                    v.names[0].as_str(),
                    sexp(v.init.as_ref().expect("initializer")),
                ),
                other => panic!("unexpected declaration: {other:?}"),
            })
            .collect();
        assert_eq!(names, vec![("x", "1".to_string()), ("y", "2".to_string())]);
    }

    #[test]
    fn module_declaration_holds_every_member_kind() {
        let file = parse(
            r#"implement T;
T: module {
    PATH: con "/dis/t.dis";
    Alias: type ref Point;
    state: int;
    f, g: fn(x: int): int;
    Point: adt {
        x, y: int;
    };
};
"#,
        );
        let Decl::Module(m) = &file.decls[0] else {
            panic!("expected a module declaration");
        };
        let kinds: Vec<String> = m
            .members
            .iter()
            .map(|mem| match mem {
                ModuleMember::Const(c) => format!("con {}", c.name),
                ModuleMember::TypeAlias(t) => format!("type {}", t.name),
                ModuleMember::Var(v) => format!("var {}", v.names.join(",")),
                ModuleMember::Func(f) => format!("fn {}", f.name),
                ModuleMember::Adt(a) => format!("adt {}", a.name),
                ModuleMember::Exception(e) => format!("exception {}", e.name),
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "con PATH",
                "type Alias",
                "var state",
                "fn f",
                "fn g",
                "adt Point",
            ]
        );
        // Both names of `f, g: fn(x: int): int;` keep the whole signature.
        for mem in &m.members {
            if let ModuleMember::Func(sig) = mem {
                assert_eq!(sig.params.len(), 1, "{} lost its parameter", sig.name);
                assert_eq!(ty_shape(sig.ret.as_ref().expect("return type")), "int");
            }
        }
    }

    #[test]
    fn adt_declaration_holds_fields_constants_and_functions() {
        let file = parse(
            r#"implement T;
Point: adt {
    x, y: int;
    next: cyclic ref Point;
    ORIGIN: con 0;
    add: fn(p: self ref Point, q: ref Point): ref Point;
};
"#,
        );
        let Decl::Adt(a) = &file.decls[0] else {
            panic!("expected an ADT declaration");
        };
        let kinds: Vec<String> = a
            .members
            .iter()
            .map(|mem| match mem {
                AdtMember::Field(v) => format!(
                    "field {}: {}",
                    v.names.join(","),
                    ty_shape(v.ty.as_ref().expect("field type"))
                ),
                AdtMember::Const(c) => format!("con {}", c.name),
                AdtMember::Func(f) => format!("fn {}", f.name),
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "field x,y: int",
                "field next: (ref Point)",
                "con ORIGIN",
                "fn add",
            ]
        );
        assert!(a.pick.is_none());
    }

    /// The receiver of an ADT function member is `p: self ref Point`. The
    /// `self` marks the parameter; it does not add a level of `ref`.
    #[test]
    fn self_parameter_keeps_its_own_type() {
        let file = parse(
            r#"implement T;
Point: adt {
    add: fn(p: self ref Point): int;
};
"#,
        );
        let Decl::Adt(a) = &file.decls[0] else {
            panic!("expected an ADT declaration");
        };
        let AdtMember::Func(sig) = &a.members[0] else {
            panic!("expected a function member");
        };
        assert!(sig.params[0].is_self);
        assert_eq!(ty_shape(&sig.params[0].ty), "(ref Point)");
    }

    /// A module interface declares ADTs with the same rule as the top level,
    /// so the type parameters and the `for { ... }` clause belong there too
    /// (limbo.y:250-263). module/tables.m:3 and module/alphabet.m:116 use both.
    #[test]
    fn module_member_adt_with_type_parameters_and_a_for_clause() {
        let file = parse(
            r#"implement T;
Tables: module {
    Table: adt[T] {
        items: array of T;
    };
    Context: adt[V, M] for {
    V =>
        dup: fn(t: self V): V;
    M =>
        mks: fn(s: string): V;
    }
    {
        eval: fn(v: V): int;
    };
};
"#,
        );
        let Decl::Module(m) = &file.decls[0] else {
            panic!("expected a module declaration");
        };
        let names: Vec<&str> = m
            .members
            .iter()
            .map(|mem| match mem {
                ModuleMember::Adt(a) => a.name.as_str(),
                other => panic!("unexpected member: {other:?}"),
            })
            .collect();
        assert_eq!(names, vec!["Table", "Context"]);
    }

    /// A pick case's fields are `dfields`, so each may be `cyclic`
    /// (limbo.y:292 and limbo.y:312). module/json.m:8 declares
    /// `mem: cyclic list of (string, ref JValue);`.
    #[test]
    fn pick_case_field_may_be_cyclic() {
        let file = parse(
            r#"implement T;
JValue: adt {
    pick {
    Object =>
        mem: cyclic list of (string, ref JValue);
    Array =>
        a: cyclic array of ref JValue;
    }
};
"#,
        );
        let Decl::Adt(a) = &file.decls[0] else {
            panic!("expected an ADT declaration");
        };
        let cases = a.pick.as_ref().expect("pick clause");
        let shapes: Vec<String> = cases
            .iter()
            .flat_map(|c| {
                c.fields.iter().map(|f| {
                    format!(
                        "{}: {}",
                        f.names.join(","),
                        ty_shape(f.ty.as_ref().expect("field type"))
                    )
                })
            })
            .collect();
        assert_eq!(
            shapes,
            vec![
                "mem: (list (tuple string (ref JValue)))",
                "a: (array (ref JValue))",
            ]
        );
    }

    /// A qualified type name carries its own type arguments, as in
    /// `ref Extvalues->Values[ref Abc->Value]` (module/alphabet/abctypes.m:5).
    #[test]
    fn qualified_type_name_with_type_arguments() {
        assert_eq!(
            ty_shape(&ty_of("ref Extvalues->Values[ref Abc->Value]")),
            "(ref Extvalues->Values)"
        );
        assert_eq!(
            ty_shape(&ty_of("chan of ref Proxy->Typescmd[ref Value]")),
            "(chan (ref Proxy->Typescmd))"
        );
    }

    #[test]
    fn adt_with_a_pick_clause() {
        let file = parse(
            r#"implement T;
Node: adt {
    line: int;
    pick {
    Nil or Empty =>
    Cons =>
        head: int;
        rest: ref Node;
    }
};
"#,
        );
        let Decl::Adt(a) = &file.decls[0] else {
            panic!("expected an ADT declaration");
        };
        let cases = a.pick.as_ref().expect("pick clause");
        assert_eq!(cases.len(), 2);
        assert_eq!(cases[0].tags, vec!["Nil", "Empty"]);
        assert!(cases[0].fields.is_empty(), "the first case has no fields");
        assert_eq!(cases[1].tags, vec!["Cons"]);
        let fields: Vec<String> = cases[1]
            .fields
            .iter()
            .map(|f| {
                format!(
                    "{}: {}",
                    f.names.join(","),
                    ty_shape(f.ty.as_ref().expect("field type"))
                )
            })
            .collect();
        assert_eq!(fields, vec!["head: int", "rest: (ref Node)"]);
    }

    #[test]
    fn polymorphic_declarations_parse_their_type_parameters_away() {
        let file = parse(
            r#"implement T;
Set: adt[T] {
    items: array of T;
};
Pair: adt for { A } {
    a: int;
};
lookup[T](s: T): int
{
    return 0;
}
"#,
        );
        let names: Vec<&str> = file
            .decls
            .iter()
            .map(|d| match d {
                Decl::Adt(a) => a.name.as_str(),
                Decl::Func(f) => f.name.name.as_str(),
                other => panic!("unexpected declaration: {other:?}"),
            })
            .collect();
        assert_eq!(names, vec!["Set", "Pair", "lookup"]);
    }

    #[test]
    fn function_signature_clauses_are_accepted() {
        // `raises`, a `raises` list, and a polymorphic `for` clause all sit
        // between the return type and the body.
        let sources = [
            "implement T;\nf(): int raises (E, F)\n{\n\treturn 0;\n}\n",
            "implement T;\nf(): int raises E\n{\n\treturn 0;\n}\n",
            "implement T;\nf(): int raise (E)\n{\n\treturn 0;\n}\n",
            "implement T;\nf[T](x: T): int for { T => }\n{\n\treturn 0;\n}\n",
        ];
        for src in sources {
            let f = func_of(src);
            assert_eq!(f.name.name, "f");
            assert_eq!(ty_shape(f.sig.ret.as_ref().expect("return type")), "int");
        }
    }

    #[test]
    fn parameter_lists() {
        let f = func_of(
            r#"implement T;
f(a, b: int, s: string, nil: ref Draw->Context, c: chan of int): (int, string)
{
    return (0, "");
}
"#,
        );
        let shapes: Vec<String> = f
            .sig
            .params
            .iter()
            .map(|p| format!("{}: {}", p.names.join(","), ty_shape(&p.ty)))
            .collect();
        assert_eq!(
            shapes,
            vec![
                "a,b: int",
                "s: string",
                "nil: (ref Draw->Context)",
                "c: (chan int)",
            ]
        );
        assert!(f.sig.params[2].is_nil, "the `nil` parameter is marked");
        assert_eq!(
            ty_shape(f.sig.ret.as_ref().expect("return type")),
            "(tuple int string)"
        );
    }

    #[test]
    fn function_type_with_polymorphic_parameters() {
        // `fn[T](...)` appears both as a member signature and as a type.
        let file =
            parse("implement T;\nT: module {\n\tf: fn[T](x: T): int;\n};\ng: fn[T](x: T);\n");
        let Decl::Module(m) = &file.decls[0] else {
            panic!("expected a module declaration");
        };
        let ModuleMember::Func(sig) = &m.members[0] else {
            panic!("expected a function member");
        };
        assert_eq!(sig.params.len(), 1);
        assert_eq!(ty_shape(&ty_of("fn[T](x: T): int")), "(fn [T] int)");
        assert!(matches!(&file.decls[1], Decl::Var(_)));
    }

    #[test]
    fn parameter_name_groups_with_nil_and_a_trailing_comma() {
        let f = func_of("implement T;\nf(nil, nil: int, a: string,)\n{\n\tx = 1;\n}\n");
        let shapes: Vec<String> = f
            .sig
            .params
            .iter()
            .map(|p| format!("{}: {}", p.names.join(","), ty_shape(&p.ty)))
            .collect();
        assert_eq!(shapes, vec!["nil,nil: int", "a: string"]);
    }

    #[test]
    fn variadic_parameter_lists() {
        // `fn(*)` and a trailing `, *` both end the list (limbo.y fnarg).
        for (src, want) in [
            ("implement T;\nf(*)\n{\n\tx = 1;\n}\n", 0),
            ("implement T;\nf(a: int, *)\n{\n\tx = 1;\n}\n", 1),
        ] {
            let f = func_of(src);
            assert_eq!(f.sig.params.len(), want, "parameter count of `{src}`");
        }
    }

    #[test]
    fn qualified_function_definition_with_polymorphic_parameters() {
        let f = func_of("implement T;\nSet[T].add[U](x: int): int\n{\n\treturn x;\n}\n");
        assert_eq!(f.name.qualifier, Some("Set".to_string()));
        assert_eq!(f.name.name, "add");
    }

    #[test]
    fn a_declaration_group_yields_one_declaration_per_name() {
        // Only `import` keeps its names together, because one import binds all
        // of them to the same module.
        let file = parse("implement T;\nA, B: con 1;\nC, D: type int;\nE, F: import m;\n");
        assert_eq!(file.decls.len(), 5);
        assert!(matches!(file.decls[4], Decl::Import(_)));
    }

    // ── Statements ─────────────────────────────────────────────

    #[test]
    fn if_else_chain() {
        let stmts = stmts_of(
            r#"implement T;
test()
{
    if (a) x = 1;
    else if (b) x = 2;
    else x = 3;
}
"#,
        );
        let Stmt::If(outer) = &stmts[0] else {
            panic!("expected an if statement");
        };
        assert_eq!(sexp(&outer.cond), "a");
        let Some(else_) = &outer.else_ else {
            panic!("expected an else branch");
        };
        let Stmt::If(inner) = else_.as_ref() else {
            panic!("expected a nested if statement");
        };
        assert_eq!(sexp(&inner.cond), "b");
        assert!(inner.else_.is_some());
    }

    #[test]
    fn if_without_else() {
        let stmts = stmts_of("implement T;\ntest()\n{\n\tif (a) x = 1;\n}\n");
        let Stmt::If(s) = &stmts[0] else {
            panic!("expected an if statement");
        };
        assert!(s.else_.is_none());
    }

    #[test]
    fn loop_statements() {
        let stmts = stmts_of(
            r#"implement T;
test()
{
    while (i < n) i++;
    do i++; while (i < n);
    for (i := 0; i < n; i++) x = i;
    for (;;) break;
}
"#,
        );
        let Stmt::While(w) = &stmts[0] else {
            panic!("expected a while statement");
        };
        assert_eq!(sexp(&w.cond), "(< i n)");
        let Stmt::Do(d) = &stmts[1] else {
            panic!("expected a do statement");
        };
        assert_eq!(sexp(&d.cond), "(< i n)");
        let Stmt::For(f) = &stmts[2] else {
            panic!("expected a for statement");
        };
        assert!(matches!(f.init.as_deref(), Some(Stmt::Expr(_))));
        assert_eq!(sexp(f.cond.as_ref().expect("condition")), "(< i n)");
        assert!(matches!(f.post.as_deref(), Some(Stmt::Expr(_))));
        let Stmt::For(bare) = &stmts[3] else {
            panic!("expected a for statement");
        };
        assert!(bare.init.is_none() && bare.cond.is_none() && bare.post.is_none());
    }

    #[test]
    fn jump_statements() {
        let stmts = stmts_of(
            r#"implement T;
test()
{
    return;
    return 1 + 2;
    break;
    break outer;
    continue;
    continue outer;
    exit;
    spawn worker(c);
    raise "fail:oops";
    raise;
    ;
}
"#,
        );
        assert!(matches!(&stmts[0], Stmt::Return(None, _)));
        let Stmt::Return(Some(e), _) = &stmts[1] else {
            panic!("expected a return with a value");
        };
        assert_eq!(sexp(e), "(+ 1 2)");
        assert!(matches!(&stmts[2], Stmt::Break(None, _)));
        assert!(matches!(&stmts[3], Stmt::Break(Some(l), _) if l == "outer"));
        assert!(matches!(&stmts[4], Stmt::Continue(None, _)));
        assert!(matches!(&stmts[5], Stmt::Continue(Some(l), _) if l == "outer"));
        assert!(matches!(&stmts[6], Stmt::Exit(_)));
        let Stmt::Spawn(e, _) = &stmts[7] else {
            panic!("expected a spawn statement");
        };
        assert_eq!(sexp(e), "(call worker c)");
        assert!(matches!(&stmts[8], Stmt::Raise(Some(_), _)));
        assert!(matches!(&stmts[9], Stmt::Raise(None, _)));
        assert!(matches!(&stmts[10], Stmt::Empty));
    }

    #[test]
    fn labelled_statements() {
        // A label may only precede the statements the reference lets it name
        // (limbo.y stmt: Lid ':' ...).
        for (src, want) in [
            ("out: for (;;) break;", "for"),
            ("out: while (a) break;", "while"),
            ("out: do break; while (a);", "do"),
            ("out: case x { * => break; }", "case"),
            ("out: alt { * => break; }", "alt"),
            ("out: { x = 1; }", "block"),
        ] {
            let stmts = stmts_of(&format!("implement T;\ntest()\n{{\n\t{src}\n}}\n"));
            let Stmt::Label(name, inner) = &stmts[0] else {
                panic!("expected a label for `{src}`, got {:?}", stmts[0]);
            };
            assert_eq!(name, "out");
            let kind = match inner.as_ref() {
                Stmt::For(_) => "for",
                Stmt::While(_) => "while",
                Stmt::Do(_) => "do",
                Stmt::Case(_) => "case",
                Stmt::Alt(_) => "alt",
                Stmt::Pick(_) => "pick",
                Stmt::Block(_) => "block",
                other => panic!("unexpected labelled statement: {other:?}"),
            };
            assert_eq!(kind, want, "labelled statement of `{src}`");
        }
    }

    #[test]
    fn case_statement_patterns() {
        let stmts = stmts_of(
            r#"implement T;
test(x: int)
{
    case x {
    0 or 1 =>
        y = 1;
    2 to 10 =>
        y = 2;
    "s" =>
        y = 3;
    * =>
        y = 4;
    }
}
"#,
        );
        let Stmt::Case(c) = &stmts[0] else {
            panic!("expected a case statement");
        };
        assert_eq!(sexp(&c.expr), "x");
        let shapes: Vec<String> = c
            .arms
            .iter()
            .map(|arm| {
                let parts: Vec<String> = arm
                    .patterns
                    .iter()
                    .map(|p| match p {
                        CasePattern::Expr(e) => sexp(e),
                        CasePattern::Range(lo, hi) => format!("({} to {})", sexp(lo), sexp(hi)),
                        CasePattern::Wildcard => "*".to_string(),
                    })
                    .collect();
                format!("{} [{}]", parts.join(" or "), arm.body.len())
            })
            .collect();
        assert_eq!(
            shapes,
            vec!["0 or 1 [1]", "(2 to 10) [1]", "\"s\" [1]", "* [1]"]
        );
    }

    /// A statement that starts with `*` is a dereference, not the wildcard arm.
    /// Treating every `*` as an arm opener ended the arm early and then asked
    /// for a `=>`, which is what broke appl/cmd/limbo/gen.b:563.
    #[test]
    fn a_dereference_statement_does_not_open_a_new_arm() {
        let stmts = stmts_of(
            r#"implement T;
test(x: int)
{
    case x {
    0 =>
        next := in.next;
        *in = *b;
        in.next = next;
    * =>
        y = 1;
    }
}
"#,
        );
        let Stmt::Case(c) = &stmts[0] else {
            panic!("expected a case statement");
        };
        assert_eq!(c.arms.len(), 2);
        assert_eq!(c.arms[0].body.len(), 3, "the whole arm body is one arm");
        assert!(matches!(c.arms[1].patterns[0], CasePattern::Wildcard));
    }

    /// The wildcard may be joined to another pattern with `or`, as in
    /// `* or 4 =>` (appl/lib/sets32.b:120) and `* or "disc" =>`
    /// (appl/ebook/reader.b:1371).
    #[test]
    fn wildcard_joined_to_another_pattern_by_or() {
        let stmts = stmts_of(
            r#"implement T;
test(x: int)
{
    case x {
    3 =>
        y = 1;
    * or
    4 =>
        y = 2;
    }
}
"#,
        );
        let Stmt::Case(c) = &stmts[0] else {
            panic!("expected a case statement");
        };
        assert_eq!(c.arms.len(), 2);
        let shapes: Vec<String> = c.arms[1]
            .patterns
            .iter()
            .map(|p| match p {
                CasePattern::Wildcard => "*".to_string(),
                CasePattern::Expr(e) => sexp(e),
                CasePattern::Range(lo, hi) => format!("({} to {})", sexp(lo), sexp(hi)),
            })
            .collect();
        assert_eq!(shapes, vec!["*", "4"]);
    }

    #[test]
    fn case_arm_with_an_empty_body() {
        let stmts = stmts_of(
            r#"implement T;
test(x: int)
{
    case x {
    0 =>
    * =>
        y = 1;
    }
}
"#,
        );
        let Stmt::Case(c) = &stmts[0] else {
            panic!("expected a case statement");
        };
        assert_eq!(c.arms.len(), 2);
        assert!(c.arms[0].body.is_empty());
        assert_eq!(c.arms[1].body.len(), 1);
    }

    #[test]
    fn alt_guard_destinations() {
        let alt = alt_of(
            r#"implement T;
test(c: chan of int, d: chan of (int, int))
{
    alt {
    x := <-c =>
        y = x;
    (a, b) := <-d =>
        y = a;
    z = <-c =>
        y = z;
    arr[i] = <-c =>
        y = 1;
    <-c =>
        y = 2;
    c <-= 3 =>
        y = 3;
    * =>
        y = 4;
    }
}
"#,
        );
        let shapes: Vec<String> = alt
            .arms
            .iter()
            .map(|arm| match &arm.guards[0] {
                AltGuard::Send(chan, val) => format!("send {} {}", sexp(chan), sexp(val)),
                AltGuard::Recv(None, chan) => format!("recv _ {}", sexp(chan)),
                AltGuard::Recv(Some(AltDest::Decl(names)), chan) => {
                    format!("recv decl[{}] {}", names.join(","), sexp(chan))
                }
                AltGuard::Recv(Some(AltDest::TupleDecl(names)), chan) => {
                    format!("recv tupledecl[{}] {}", names.join(","), sexp(chan))
                }
                AltGuard::Recv(Some(AltDest::Assign(lhs)), chan) => {
                    format!("recv assign {} {}", sexp(lhs), sexp(chan))
                }
                AltGuard::Wildcard => "wildcard".to_string(),
            })
            .collect();
        assert_eq!(
            shapes,
            vec![
                "recv decl[x] c",
                "recv tupledecl[a,b] d",
                "recv assign z c",
                "recv assign (index arr i) c",
                "recv _ c",
                "send c 3",
                "wildcard",
            ]
        );
    }

    #[test]
    fn alt_guard_that_is_not_a_communication_is_rejected() {
        let msg = parse_err(
            r#"implement T;
test()
{
    alt {
    x + 1 =>
        y = 1;
    }
}
"#,
        );
        assert!(
            msg.contains("`alt` guard must be"),
            "unexpected message: {msg}"
        );
    }

    #[test]
    fn pick_statement_arms() {
        let stmts = stmts_of(
            r#"implement T;
test(val: ref Node)
{
    pick n := val {
    Cons or Nil =>
        y = 1;
    Leaf =>
        y = 2;
        z = 3;
    * =>
        y = 4;
    }
}
"#,
        );
        let Stmt::Pick(p) = &stmts[0] else {
            panic!("expected a pick statement");
        };
        assert_eq!(p.name, "n");
        assert_eq!(sexp(&p.expr), "val");
        let shapes: Vec<String> = p
            .arms
            .iter()
            .map(|arm| format!("{} [{}]", arm.tags.join(" or "), arm.body.len()))
            .collect();
        assert_eq!(shapes, vec!["Cons or Nil [1]", "Leaf [2]", "* [1]"]);
    }

    #[test]
    fn local_declaration_forms() {
        let stmts = stmts_of(
            r#"implement T;
test()
{
    x: int;
    y, z: string = "s";
    N: con 5;
    A: type ref Point;
    Iobuf: import bufio;
    include "sys.m";
    { w := 1; }
    ;
}
"#,
        );
        let Stmt::VarDecl(bare) = &stmts[0] else {
            panic!("expected a variable declaration");
        };
        assert_eq!(bare.names, vec!["x"]);
        assert!(bare.init.is_none());
        let Stmt::VarDecl(init) = &stmts[1] else {
            panic!("expected a variable declaration");
        };
        assert_eq!(init.names, vec!["y", "z"]);
        assert_eq!(sexp(init.init.as_ref().expect("initializer")), "\"s\"");
        // A local constant and a local type alias bind no storage, so they
        // reach codegen as nothing at all.
        assert!(matches!(&stmts[2], Stmt::Empty));
        assert!(matches!(&stmts[3], Stmt::Empty));
        let Stmt::Import(i) = &stmts[4] else {
            panic!("expected an import statement");
        };
        assert_eq!(i.names, vec!["Iobuf"]);
        assert_eq!(i.module, "bufio");
        assert!(
            matches!(&stmts[5], Stmt::Empty),
            "a local include binds nothing"
        );
        assert!(matches!(&stmts[6], Stmt::Block(_)));
        assert!(matches!(&stmts[7], Stmt::Empty));
    }

    /// A name followed by a colon is a declaration, a label, or neither, and
    /// the three have to stay apart.
    #[test]
    fn colon_after_a_name_is_classified_by_what_follows() {
        let stmts = stmts_of(
            r#"implement T;
test()
{
    a: int;
    b: for (;;) break;
    c(1);
}
"#,
        );
        assert!(matches!(&stmts[0], Stmt::VarDecl(_)));
        assert!(matches!(&stmts[1], Stmt::Label(_, _)));
        assert!(matches!(&stmts[2], Stmt::Expr(Expr::Call(_, _, _))));
    }

    #[test]
    fn block_with_an_exception_handler() {
        // The handler is skipped rather than kept, so the statement is the
        // block itself.
        let stmts = stmts_of(
            r#"implement T;
test()
{
    {
        x = 1;
    } exception e {
    "fail:*" =>
        x = 2;
    * =>
        x = 3;
    }
    exception {
    "other" =>
        x = 4;
    }
}
"#,
        );
        let Stmt::Block(b) = &stmts[0] else {
            panic!("expected a block statement");
        };
        assert_eq!(b.stmts.len(), 1);
        assert!(matches!(&stmts[1], Stmt::Empty));
    }

    /// A guard's communication may sit anywhere inside the guard expression.
    /// The reference finds it with `hascomm` (typecheck.c:3381-3421) and
    /// rewrites the rest of the guard around it (com.c:1208-1242), which makes
    /// `reqpool = <-reqdone :: reqpool =>` legal (appl/cmd/wmexport.b:180).
    /// This front end only classifies a communication at the top of the guard,
    /// so such a guard is reported rather than miscompiled. Known gap: closing
    /// it needs the guard expression kept in the AST and codegen support for
    /// the rewrite.
    #[test]
    fn a_nested_communication_in_an_alt_guard_is_reported() {
        let msg = parse_err(
            r#"implement T;
test(reqdone: chan of int)
{
    alt {
    reqpool = <-reqdone :: reqpool =>
        x = 1;
    }
}
"#,
        );
        assert!(
            msg.contains("`alt` guard must be"),
            "unexpected message: {msg}"
        );
    }

    /// An exception handler pattern may hold brackets of its own, and the
    /// scanner that looks for the `=>` has to count them rather than stop at
    /// the first one.
    #[test]
    fn exception_handler_pattern_with_nested_brackets() {
        let stmts = stmts_of(
            r#"implement T;
test()
{
    {
        x = 1;
    } exception e {
    Sys->E(1) or "a[1]" =>
        x = 2;
    * =>
        x = 3;
    }
}
"#,
        );
        assert!(matches!(&stmts[0], Stmt::Block(_)));
    }

    /// A selector in an expression list is parsed and dropped. The reference
    /// only allows selectors in an array initializer (limbo.y initlist), so
    /// this path is reached by malformed input; it must not loop or panic.
    #[test]
    fn selector_in_an_expression_list_is_skipped() {
        assert_eq!(sexp(&expr_of("list of {1 => 2}")), "(listlit 2)");
        assert_eq!(sexp(&expr_of("list of {1 to 3 => 4}")), "(listlit 4)");
        assert_eq!(sexp(&expr_of("list of {1 or 2 => 3}")), "(listlit 3)");
    }

    /// A basic type in a position where no operand can follow becomes a name
    /// rather than a cast.
    #[test]
    fn basic_type_with_no_operand_is_a_name() {
        assert_eq!(
            sexp(&expr_of("f(int, string)")),
            "(call f Basic(Int) Basic(String))"
        );
    }

    // ── Errors and recovery ────────────────────────────────────

    #[test]
    fn malformed_sources_report_a_specific_diagnostic() {
        let table: &[(&str, &str)] = &[
            // Header.
            ("implement 3;\n", "expected identifier"),
            ("implement T, ;\n", "expected identifier"),
            ("implement T\n", "expected Semicolon"),
            (
                "implement T;\ninclude 3;\n",
                "expected string after include",
            ),
            ("implement T;\ninclude \"sys.m\"\n", "expected Semicolon"),
            // Top level.
            (
                "implement T;\nif (x) { }\n",
                "unexpected token at top level",
            ),
            ("implement T;\n}\n", "unexpected token at top level"),
            ("implement T;\n42;\n", "unexpected token at top level"),
            // Truncated declarations.
            ("implement T;\nx: int\n", "expected Semicolon"),
            ("implement T;\nx:\n", "expected type"),
            ("implement T;\nx: int =\n", "unexpected token in expression"),
            (
                "implement T;\nA, B: con\n",
                "unexpected token in expression",
            ),
            ("implement T;\nA, : con 1;\n", "expected identifier"),
            ("implement T;\nx: import\n", "expected identifier"),
            // Unterminated constructs.
            ("implement T;\nT: module { f: fn();\n", "expected RBrace"),
            ("implement T;\nP: adt { x: int;\n", "expected RBrace"),
            ("implement T;\nf()\n{\n\tx = 1;\n", "expected RBrace"),
            ("implement T;\nf(a: int\n", "expected RParen"),
            ("implement T;\nf(a\n", "expected Colon"),
            // A name group that runs into something other than a name.
            ("implement T;\nf(a, *)\n{\n}\n", "expected Colon"),
            (
                "implement T;\nf()\n{\n\tx = (1 + 2;\n}\n",
                "expected RParen",
            ),
            ("implement T;\nf()\n{\n\tx = a[1;\n}\n", "expected RBracket"),
            ("implement T;\nf()\n{\n\tcase x\n}\n", "expected LBrace"),
            ("implement T;\nf()\n{\n\talt x\n}\n", "expected LBrace"),
            (
                "implement T;\nf()\n{\n\tpick x = v { }\n}\n",
                "expected ColonEq",
            ),
            ("implement T;\nf()\n{\n\tdo x = 1;\n}\n", "expected While"),
            // Stray delimiters and malformed expressions.
            (
                "implement T;\nf()\n{\n\t) ;\n}\n",
                "unexpected token in expression",
            ),
            (
                "implement T;\nf()\n{\n\tx = a.;\n}\n",
                "expected identifier",
            ),
            (
                "implement T;\nf()\n{\n\treturn\n}\n",
                "unexpected token in expression",
            ),
            (
                "implement T;\nf()\n{\n\tbreak 1;\n}\n",
                "expected Semicolon",
            ),
            (
                "implement T;\nf()\n{\n\t(a + b) := c;\n}\n",
                "left side of := must be identifier or tuple",
            ),
            (
                "implement T;\nf()\n{\n\t(1, 2) := c;\n}\n",
                "tuple := elements must be identifiers",
            ),
        ];
        for (src, want) in table {
            let msg = parse_err(src);
            assert!(
                msg.contains(want),
                "for {src:?} expected a message containing {want:?}, got {msg:?}"
            );
        }
    }

    #[test]
    fn error_carries_the_file_name_and_place() {
        let tokens = Lexer::new("implement T;\nx: int\n", "prog.b")
            .tokenize()
            .expect("lex should succeed");
        let err = Parser::new(tokens, "prog.b")
            .parse_file()
            .expect_err("parse should fail");
        assert_eq!(err.file, "prog.b");
        assert_eq!(err.span.line, 3);
        assert!(err.to_string().starts_with("prog.b:3:"));
    }

    /// A malformed statement inside a case arm is skipped up to the next arm,
    /// and the surrounding case still parses.
    #[test]
    fn case_arm_recovers_from_a_malformed_statement() {
        let stmts = stmts_of(
            r#"implement T;
test(x: int)
{
    case x {
    0 =>
        y = ) ;
    * =>
        y = 2;
    }
}
"#,
        );
        let Stmt::Case(c) = &stmts[0] else {
            panic!("expected a case statement");
        };
        assert_eq!(c.arms.len(), 2);
        assert!(
            c.arms[0].body.is_empty(),
            "the malformed statement is dropped"
        );
        assert_eq!(c.arms[1].body.len(), 1);
    }

    /// A brace where an expression belongs is skipped as a balanced group so
    /// that one syntax error does not cascade.
    #[test]
    fn brace_in_expression_position_is_skipped_as_a_group() {
        let stmts = stmts_of("implement T;\ntest()\n{\n\tx = { a; { b; } };\n}\n");
        assert_eq!(stmts.len(), 1);
        assert_eq!(sexp_of_stmt(&stmts[0]), "(= x nil)");
    }

    fn sexp_of_stmt(stmt: &Stmt) -> String {
        match stmt {
            Stmt::Expr(e) => sexp(e),
            other => panic!("expected an expression statement, got {other:?}"),
        }
    }

    /// Every prefix of a rich program has to end in a parse or a diagnostic.
    /// The recovery paths (case arms, exception handler bodies, the pick and
    /// alt arm scanners, and the balanced-brace skip) are loops over the token
    /// stream, so a truncated stream is what would spin them. A panic fails
    /// this test and a loop hangs it, which is the signal either way.
    #[test]
    fn every_truncated_prefix_terminates() {
        let src = r#"implement T;
include "sys.m";
    sys: Sys;
A, B: con iota;
E: exception (string);
T: module {
    PATH: con "/dis/t.dis";
    Node: adt {
        v: int;
        pick {
        Nil =>
        Cons =>
            hd: int;
        }
    };
    init: fn(nil: ref Draw->Context, argv: list of string);
};
init(nil: ref Draw->Context, argv: list of string)
{
    sys = load Sys Sys->PATH;
    a := array[4] of {2 => 1, * => 0};
    l := 1 :: 2 :: nil;
    c := chan of int;
    spawn worker(c);
    for (i := 0; i < len a; i++) {
        case a[i] {
        0 or 1 =>
            continue;
        2 to 3 =>
            break;
        * =>
            x = -i ** 2;
        }
    }
    alt {
    v := <-c =>
        x = v;
    c <-= 1 =>
        x = 2;
    * =>
        x = 3;
    }
    pick n := node {
    Cons =>
        x = n.hd;
    * =>
        x = 0;
    }
    {
        x = 1;
    } exception e {
    "fail:*" =>
        x = 2;
    }
    while (x != 0) do_something(x);
}
"#;
        let tokens = Lexer::new(src, "<test>")
            .tokenize()
            .expect("lex should succeed");
        for n in 0..tokens.len() {
            let mut prefix: Vec<Token> = tokens[..n].to_vec();
            prefix.push(Token {
                kind: TokenKind::Eof,
                span: Span::default(),
            });
            let _ = Parser::new(prefix, "<test>").parse_file();
        }
    }

    /// The whole program parses, not just its prefixes.
    #[test]
    fn stray_delimiters_do_not_derail_the_parser() {
        for src in [
            "implement T;\nf()\n{\n\t]\n}\n",
            "implement T;\nf()\n{\n\t{ ] }\n}\n",
            "implement T;\nf()\n{\n\tx = a[)];\n}\n",
            "implement T;\nf()\n{\n\tcase x { ) => y = 1; }\n}\n",
            "implement T;\nf()\n{\n\talt { ) => y = 1; }\n}\n",
            "implement T;\nf()\n{\n\tpick p := v { ) => y = 1; }\n}\n",
            "implement T;\nf()\n{\n\t{ } exception { ) }\n}\n",
            "implement T;\nT: module { ) };\n",
            "implement T;\nP: adt { ) };\n",
            "implement T;\nP: adt { pick { ) } };\n",
        ] {
            let tokens = Lexer::new(src, "<test>")
                .tokenize()
                .expect("lex should succeed");
            // Either outcome is fine; the point is that it returns.
            let _ = Parser::new(tokens, "<test>").parse_file();
        }
    }
}
