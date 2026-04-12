// Stage 3 — Parser
// Turns the flat token stream from the lexer into a fully structured AST.
// Recursive descent parser. Tracks scope depth, enforces the single-return-
// value rule, validates # prefix usage, and handles all Nyx syntax constructs.

use crate::lexer::{SpannedToken, Token};
use crate::make_pass::CompileConfig;

// ─────────────────────────────────────────────────────────────────────────────
// AST node types
// ─────────────────────────────────────────────────────────────────────────────

/// Source location carried on every AST node.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub line: usize,
    pub col: usize,
}

impl Span {
    fn new(line: usize, col: usize) -> Self { Span { line, col } }
}

/// A unique ID stamped on every Expr node during parsing.
/// Used by the TypeTable to associate types with expressions.
pub type NodeId = usize;

/// The root of the AST — the entire file.
#[derive(Debug, Clone)]
pub struct Program {
    pub items: Vec<TopLevelItem>,
    pub warnings: Vec<ParseWarning>,
}

/// A top-level item — something that lives at depth 0.
#[derive(Debug, Clone)]
pub struct TopLevelItem {
    pub kind: TopLevelKind,
    pub doc: Option<String>,     // preceding /// doc comment
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TopLevelKind {
    Function(FnDef),
    Class(ClassDef),
    Namespace(NamespaceDef),
}

// ── Function ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FnDef {
    pub name: String,
    pub modifiers: FnModifiers,
    pub params: Vec<Param>,
    pub return_type: Option<TypeExpr>,
    pub body: Block,
}

/// The set of % modifier directives that can appear before `fn`.
#[derive(Debug, Clone, Default)]
pub struct FnModifiers {
    pub is_pub: bool,
    pub is_async: bool,
    pub is_spawn: bool,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: TypeExpr,
    pub is_self: bool,      // true for %self / "this" / whatever it's renamed to
    pub is_mut_self: bool,  // true for &mut %self
    pub borrow: BorrowKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BorrowKind {
    Owned,
    Ref,
    MutRef,
}

// ── Class ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ClassDef {
    pub name: String,
    pub generics: Vec<String>,
    pub create_block: CreateBlock,
    pub methods: Vec<FnDef>,
}

/// The `create { ... }` block — field declarations with optional defaults.
#[derive(Debug, Clone)]
pub struct CreateBlock {
    pub fields: Vec<FieldDecl>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FieldDecl {
    pub name: String,
    pub ty: TypeExpr,
    pub default: Option<Expr>,
    pub span: Span,
}

// ── Namespace ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct NamespaceDef {
    pub name: String,
    pub items: Vec<TopLevelItem>,
}

// ── Type expressions ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    /// A plain % directive type: %i32, %str, %bool, etc.
    Primitive(String),
    /// A generic type: %Result<%i32>, [%str], etc.
    Generic(String, Vec<TypeExpr>),
    /// An array type: [T]
    Array(Box<TypeExpr>),
    /// A %nyx(args) -> T code block type.
    NyxBlock(Vec<TypeExpr>, Box<TypeExpr>),
    /// A %rust(args) -> T code block type.
    RustBlock(Vec<TypeExpr>, Box<TypeExpr>),
    /// Inferred — no annotation given.
    Inferred,
    /// %void
    Void,
}

// ── Statements ────────────────────────────────────────────────────────────────

/// A block: a `{ ... }` scope with zero or more statements and at most one
/// bare return expression.
#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    /// The single bare expression in this scope, if present (the return value).
    pub tail: Option<Box<Expr>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    /// `let [%mut] [%type] name = expr;`
    Let(LetBinding),
    /// `let = { name = expr, ... };`  — mass let
    MassLet(Vec<LetBinding>),
    /// `let %solve name = expr;`
    Solve(SolveBinding),
    /// `let %solve { eq; eq; } -> (a, b);`  — system of equations
    SolveSystem(SolveSystem),
    /// An expression used as a statement (must end with `;`)
    ExprStmt(Expr),
    /// `return expr;`
    Return(Option<Expr>),
}

#[derive(Debug, Clone)]
pub struct LetBinding {
    pub name: String,
    pub is_mut: bool,
    pub ty: TypeExpr,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct SolveBinding {
    pub unknown: String,
    pub expr: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct SolveSystem {
    pub equations: Vec<Expr>,
    pub unknowns: Vec<String>,
    pub span: Span,
}

// ── Expressions ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    pub id: NodeId,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    // Literals
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Void,

    // Identifier / path
    Ident(String),
    Path(Vec<String>),          // math::add

    // Unary
    Unary(UnaryOp, Box<Expr>),

    // Binary
    Binary(BinaryOp, Box<Expr>, Box<Expr>),

    // Assignment
    Assign(Box<Expr>, Box<Expr>),

    // Borrow
    Borrow(Box<Expr>),          // &x
    BorrowMut(Box<Expr>),       // &mut x

    // Propagate (?)
    Propagate(Box<Expr>),

    // Function call
    Call(Box<Expr>, Vec<Expr>),

    // Method call: expr.method(args)
    MethodCall(Box<Expr>, String, Vec<Expr>),

    // Field access: expr.field
    Field(Box<Expr>, String),

    // Index: expr[idx]
    Index(Box<Expr>, Box<Expr>),

    // Array literal: [a, b, c]
    Array(Vec<Expr>),

    // Range: 1..10
    Range(Box<Expr>, Box<Expr>),

    // Block expression: { stmts... tail? }
    Block(Block),

    // if / else if / else
    If(Box<Expr>, Block, Vec<(Expr, Block)>, Option<Block>),

    // while
    While(Box<Expr>, Block),

    // loop
    Loop(Block),

    // for i in expr { }
    For(String, Box<Expr>, Block),

    // match
    Match(Box<Expr>, Vec<MatchArm>),

    // break [value]
    Break(Option<Box<Expr>>),

    // return [value]  (as expression, e.g. in closures)
    Return(Option<Box<Expr>>),

    // panic(msg)
    Panic(Box<Expr>),

    // ok(value) / err(msg)
    Ok(Box<Expr>),
    Err(Box<Expr>),

    // %nyx(params) -> RetTy { body }
    NyxBlock(Vec<Param>, Option<TypeExpr>, Block),

    // %rust(params) -> RetTy { raw_src }
    RustBlock(Vec<Param>, Option<TypeExpr>, String),

    // await expr
    Await(Box<Expr>),

    // Class construction: ClassName.create { field = val, ... }
    Create(Box<Expr>, Vec<(String, Expr)>),

    // String interpolation segments (after partial parsing)
    // Each segment is either a literal string or an embedded expression.
    Interpolated(Vec<InterpolSegment>),
}

#[derive(Debug, Clone)]
pub enum InterpolSegment {
    Lit(String),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Wildcard,                   // _
    Literal(Expr),              // 0, "hello", %true
    Range(Expr, Expr),          // 1..10
    Ident(String),              // x  (binding)
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Neg,    // -
    Not,    // !
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinaryOp {
    Add, Sub, Mul, Div, Mod,
    Eq, Ne, Lt, Gt, Le, Ge,
    And, Or,
}

// ─────────────────────────────────────────────────────────────────────────────
// Parse errors and warnings
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

impl ParseError {
    fn new(msg: impl Into<String>, span: Span) -> Self {
        ParseError { message: msg.into(), span }
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[Parse Error] {}:{} — {}", self.span.line, self.span.col, self.message)
    }
}

#[derive(Debug, Clone)]
pub struct ParseWarning {
    pub message: String,
    pub span: Span,
}

// ─────────────────────────────────────────────────────────────────────────────
// Parser state
// ─────────────────────────────────────────────────────────────────────────────

struct Parser<'a> {
    tokens: &'a [SpannedToken],
    pos: usize,
    depth: usize,                   // current scope depth (0 = Main)
    warnings: Vec<ParseWarning>,
    config: &'a CompileConfig,
    /// The self keyword name for this file (from %self rename)
    self_name: String,
    /// Monotonically incrementing counter — stamps a unique NodeId on every Expr
    next_id: NodeId,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [SpannedToken], config: &'a CompileConfig) -> Self {
        let self_name = match &config.self_rename {
            crate::make_pass::SelfRename::Global(s) => s.clone(),
            crate::make_pass::SelfRename::PerClass { default, .. } => default.clone(),
        };
        Parser { tokens, pos: 0, depth: 0, warnings: Vec::new(), config, self_name, next_id: 0 }
    }

    /// Allocate the next NodeId. Called once per Expr construction.
    fn alloc_id(&mut self) -> NodeId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    // ── Cursor helpers ────────────────────────────────────────────────

    fn peek(&self) -> Option<&SpannedToken> {
        self.tokens.get(self.pos)
    }

    fn peek_tok(&self) -> Option<&Token> {
        self.peek().map(|s| &s.token)
    }

    fn peek_next(&self) -> Option<&Token> {
        self.tokens.get(self.pos + 1).map(|s| &s.token)
    }

    fn advance(&mut self) -> Option<&SpannedToken> {
        let t = self.tokens.get(self.pos);
        self.pos += 1;
        t
    }

    fn span(&self) -> Span {
        self.peek()
            .map(|s| Span::new(s.line, s.col))
            .unwrap_or(Span::new(0, 0))
    }

    fn err(&self, msg: impl Into<String>) -> ParseError {
        ParseError::new(msg, self.span())
    }

    fn warn(&mut self, msg: impl Into<String>) {
        if !self.config.suppress_warnings {
            let span = self.span();
            self.warnings.push(ParseWarning { message: msg.into(), span });
        }
    }

    fn is_exhausted(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn expect_tok(&mut self, expected: &Token, ctx: &str) -> Result<Span, ParseError> {
        let span = self.span();
        match self.advance() {
            Some(st) if &st.token == expected => Ok(span),
            Some(st) => Err(ParseError::new(
                format!("Expected {:?} in {}, got {:?}", expected, ctx, st.token),
                Span::new(st.line, st.col),
            )),
            None => Err(ParseError::new(
                format!("Expected {:?} in {}, got end of input", expected, ctx),
                span,
            )),
        }
    }

    fn expect_ident(&mut self, ctx: &str) -> Result<(String, Span), ParseError> {
        let span = self.span();
        match self.advance() {
            Some(st) => match &st.token {
                Token::Ident(s) => Ok((s.clone(), Span::new(st.line, st.col))),
                other => Err(ParseError::new(
                    format!("Expected identifier in {}, got {:?}", ctx, other),
                    Span::new(st.line, st.col),
                )),
            },
            None => Err(ParseError::new(
                format!("Expected identifier in {}, got end of input", ctx), span,
            )),
        }
    }

    /// Consume a `;` — required after every statement and closing `}` in Nyx.
    fn expect_semi(&mut self, ctx: &str) -> Result<(), ParseError> {
        self.expect_tok(&Token::Semi, ctx).map(|_| ())
    }

    /// Check if the current token is a `%` directive with the given name.
    fn peek_directive(&self, name: &str) -> bool {
        matches!(self.peek_tok(), Some(Token::Directive(d)) if d == name)
    }

    /// Check if the current token is an identifier with the given value.
    fn peek_kw(&self, kw: &str) -> bool {
        matches!(self.peek_tok(), Some(Token::Ident(s)) if s == kw)
    }

    // ── Generic list `< T, U >` ───────────────────────────────────────
    fn parse_generic_params(&mut self) -> Result<Vec<String>, ParseError> {
        if self.peek_tok() != Some(&Token::Lt) { return Ok(vec![]); }
        self.advance(); // <
        let mut params = Vec::new();
        loop {
            match self.peek_tok() {
                Some(Token::Gt) => { self.advance(); break; }
                Some(Token::Comma) => { self.advance(); }
                Some(Token::Ident(_)) => {
                    let (name, _) = self.expect_ident("generic params")?;
                    params.push(name);
                }
                _ => return Err(self.err("Malformed generic parameter list")),
            }
        }
        Ok(params)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Public entry point
// ─────────────────────────────────────────────────────────────────────────────

/// Parse a full Nyx source file into a Program AST.
pub fn parse(
    tokens: &[SpannedToken],
    config: &CompileConfig,
) -> Result<Program, ParseError> {
    let mut parser = Parser::new(tokens, config);
    let program = parser.parse_program()?;
    Ok(program)
}

// ─────────────────────────────────────────────────────────────────────────────
// Top-level parsing
// ─────────────────────────────────────────────────────────────────────────────

impl<'a> Parser<'a> {
    fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut items = Vec::new();

        while !self.is_exhausted() {
            // Skip stray semicolons at the top level
            if self.peek_tok() == Some(&Token::Semi) {
                self.advance();
                continue;
            }
            // Skip doc comments that don't precede a declaration — just consume
            if matches!(self.peek_tok(), Some(Token::DocComment(_))) {
                self.advance();
                continue;
            }
            let item = self.parse_top_level_item()?;
            items.push(item);
        }

        let warnings = std::mem::take(&mut self.warnings);
        Ok(Program { items, warnings })
    }

    fn parse_top_level_item(&mut self) -> Result<TopLevelItem, ParseError> {
        let span = self.span();

        // Consume leading /// doc comment if present
        let doc = if let Some(Token::DocComment(_)) = self.peek_tok() {
            if let Some(st) = self.advance() {
                if let Token::DocComment(s) = &st.token { Some(s.clone()) } else { None }
            } else { None }
        } else { None };

        // At depth 0, declarations MUST be preceded by #
        let has_hash = self.peek_tok() == Some(&Token::Hash);
        if has_hash {
            self.advance(); // consume #
        }

        // Collect function modifiers: %pub %async %spawn
        let modifiers = self.parse_fn_modifiers();

        // Peek at what keyword follows
        let kw = match self.peek_tok() {
            Some(Token::Ident(s)) => s.clone(),
            Some(other) => return Err(ParseError::new(
                format!("Expected 'fn', 'class', or 'namespace' at top level, got {:?}", other),
                span.clone(),
            )),
            None => return Err(ParseError::new("Unexpected end of input at top level", span.clone())),
        };

        // Warn if top-level keyword missing #
        if !has_hash && matches!(kw.as_str(), "fn" | "class" | "namespace") {
            self.warn(format!(
                "Top-level '{}' declaration should be prefixed with '#' (e.g. '#{}' ...)",
                kw, kw
            ));
        }

        let kind = match kw.as_str() {
            "fn" => {
                self.advance(); // consume "fn"
                // Skip %make function — already handled by Stage 2
                if self.peek_directive("make") {
                    self.skip_to_after_semi_brace();
                    // Return a dummy that the rest of the pipeline ignores
                    return self.parse_top_level_item();
                }
                let func = self.parse_fn_def(modifiers)?;
                TopLevelKind::Function(func)
            }
            "class" => {
                self.advance(); // consume "class"
                if !modifiers.is_default_modifiers() {
                    return Err(ParseError::new(
                        "Modifiers like %pub, %async, %spawn are not valid on class declarations",
                        span.clone(),
                    ));
                }
                let class = self.parse_class_def()?;
                TopLevelKind::Class(class)
            }
            "namespace" => {
                self.advance(); // consume "namespace"
                let ns = self.parse_namespace_def()?;
                TopLevelKind::Namespace(ns)
            }
            other => {
                return Err(ParseError::new(
                    format!("Unexpected identifier '{}' at top level — expected fn, class, or namespace", other),
                    span.clone(),
                ));
            }
        };

        // Top-level declarations end with `};`
        self.expect_semi("top-level declaration")?;

        Ok(TopLevelItem { kind, doc, span })
    }

    /// Skip over a `{ ... }` block when we want to ignore its contents.
    fn skip_to_after_semi_brace(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.peek_tok() {
                None => break,
                Some(Token::LBrace) => { depth += 1; self.advance(); }
                Some(Token::RBrace) => {
                    self.advance();
                    if depth == 0 || { depth -= 1; depth == 0 } {
                        // consume trailing ;
                        if self.peek_tok() == Some(&Token::Semi) { self.advance(); }
                        break;
                    }
                }
                _ => { self.advance(); }
            }
        }
    }

    /// Parse zero or more `%pub`, `%async`, `%spawn` modifiers before `fn`.
    fn parse_fn_modifiers(&mut self) -> FnModifiers {
        let mut m = FnModifiers::default();
        loop {
            match self.peek_tok() {
                Some(Token::Directive(d)) => match d.as_str() {
                    "pub"   => { m.is_pub   = true; self.advance(); }
                    "async" => { m.is_async = true; self.advance(); }
                    "spawn" => { m.is_spawn = true; self.advance(); }
                    _ => break,
                },
                _ => break,
            }
        }
        m
    }
}

impl FnModifiers {
    fn is_default_modifiers(&self) -> bool {
        !self.is_pub && !self.is_async && !self.is_spawn
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Function definitions
// ─────────────────────────────────────────────────────────────────────────────

impl<'a> Parser<'a> {
    fn parse_fn_def(&mut self, modifiers: FnModifiers) -> Result<FnDef, ParseError> {
        let (name, _) = self.expect_ident("function name")?;
        let _generics = self.parse_generic_params()?;
        let params = self.parse_param_list()?;
        let return_type = self.parse_optional_return_type()?;
        let body = self.parse_block()?;
        Ok(FnDef { name, modifiers, params, return_type, body })
    }

    fn parse_param_list(&mut self) -> Result<Vec<Param>, ParseError> {
        self.expect_tok(&Token::LParen, "parameter list")?;
        let mut params = Vec::new();

        loop {
            match self.peek_tok() {
                Some(Token::RParen) => { self.advance(); break; }
                Some(Token::Comma)  => { self.advance(); continue; }
                _ => params.push(self.parse_param()?),
            }
        }
        Ok(params)
    }

    fn parse_param(&mut self) -> Result<Param, ParseError> {
        let span = self.span();

        // Handle borrow prefixes: & or &mut
        let borrow = match self.peek_tok() {
            Some(Token::Amp)    => { self.advance(); BorrowKind::Ref }
            Some(Token::AmpMut) => { self.advance(); BorrowKind::MutRef }
            _                   => BorrowKind::Owned,
        };

        // Parameter name — could be a directive (%self) or ident
        let (name, is_self) = match self.peek_tok() {
            Some(Token::Directive(d)) if d == "self" || d == &self.self_name.clone() => {
                let d = d.clone();
                self.advance();
                (d, true)
            }
            Some(Token::Ident(s)) if s == &self.self_name => {
                let s = s.clone();
                self.advance();
                (s, true)
            }
            _ => {
                let (n, _) = self.expect_ident("parameter name")?;
                (n, false)
            }
        };

        let is_mut_self = is_self && borrow == BorrowKind::MutRef;

        // If it's a self param, type annotation is optional
        let ty = if !is_self && self.peek_tok() == Some(&Token::Colon) {
            self.advance(); // :
            self.parse_type_expr()?
        } else if !is_self {
            return Err(ParseError::new(
                format!("Parameter '{}' is missing a type annotation", name),
                span.clone(),
            ));
        } else {
            TypeExpr::Inferred
        };

        Ok(Param { name, ty, is_self, is_mut_self, borrow, span })
    }

    fn parse_optional_return_type(&mut self) -> Result<Option<TypeExpr>, ParseError> {
        if self.peek_tok() == Some(&Token::Arrow) {
            self.advance(); // ->
            Ok(Some(self.parse_type_expr()?))
        } else {
            Ok(None)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Class definitions
// ─────────────────────────────────────────────────────────────────────────────

impl<'a> Parser<'a> {
    fn parse_class_def(&mut self) -> Result<ClassDef, ParseError> {
        let (name, _) = self.expect_ident("class name")?;
        let generics = self.parse_generic_params()?;

        self.expect_tok(&Token::LBrace, "class body")?;
        self.depth += 1;

        let mut create_block: Option<CreateBlock> = None;
        let mut methods: Vec<FnDef> = Vec::new();

        loop {
            match self.peek_tok() {
                Some(Token::RBrace) => { self.advance(); break; }
                Some(Token::Semi)   => { self.advance(); continue; }
                Some(Token::Hash)   => {
                    return Err(ParseError::new(
                        "'#' prefix is not valid inside a class body",
                        self.span(),
                    ));
                }
                Some(Token::Ident(s)) if s == "create" => {
                    if create_block.is_some() {
                        return Err(ParseError::new(
                            "A class may only have one 'create' block",
                            self.span(),
                        ));
                    }
                    create_block = Some(self.parse_create_block()?);
                    self.expect_semi("create block")?;
                }
                Some(Token::Ident(s)) if s == "fn" => {
                    self.advance(); // consume "fn"
                    let modifiers = FnModifiers::default();
                    let method = self.parse_fn_def(modifiers)?;
                    methods.push(method);
                    self.expect_semi("method definition")?;
                }
                _ => {
                    // Could be %pub fn, %async fn, etc.
                    let modifiers = self.parse_fn_modifiers();
                    if self.peek_kw("fn") {
                        self.advance(); // consume "fn"
                        let method = self.parse_fn_def(modifiers)?;
                        methods.push(method);
                        self.expect_semi("method definition")?;
                    } else {
                        return Err(ParseError::new(
                            format!("Unexpected token in class body: {:?}", self.peek_tok()),
                            self.span(),
                        ));
                    }
                }
            }
        }

        self.depth -= 1;

        let create_block = create_block.unwrap_or(CreateBlock {
            fields: Vec::new(),
            span: self.span(),
        });

        Ok(ClassDef { name, generics, create_block, methods })
    }

    fn parse_create_block(&mut self) -> Result<CreateBlock, ParseError> {
        let span = self.span();
        self.advance(); // consume "create"
        self.expect_tok(&Token::LBrace, "create block")?;
        let mut fields = Vec::new();

        loop {
            match self.peek_tok() {
                Some(Token::RBrace) => { self.advance(); break; }
                Some(Token::Comma)  => { self.advance(); continue; }
                Some(Token::Semi)   => { self.advance(); continue; }
                Some(Token::Ident(s)) if s == "let" => {
                    self.advance(); // consume "let"
                    fields.push(self.parse_field_decl()?);
                }
                _ => return Err(ParseError::new(
                    format!("Expected 'let' in create block, got {:?}", self.peek_tok()),
                    self.span(),
                )),
            }
        }

        Ok(CreateBlock { fields, span })
    }

    fn parse_field_decl(&mut self) -> Result<FieldDecl, ParseError> {
        let span = self.span();

        // Optional type directive before name: let %str name
        let ty = if matches!(self.peek_tok(), Some(Token::Directive(_))) {
            self.parse_type_expr()?
        } else {
            TypeExpr::Inferred
        };

        let (name, _) = self.expect_ident("field name")?;

        // Optional `: type` annotation
        let ty = if ty == TypeExpr::Inferred && self.peek_tok() == Some(&Token::Colon) {
            self.advance(); // :
            self.parse_type_expr()?
        } else {
            ty
        };

        // Optional `= default`
        let default = if self.peek_tok() == Some(&Token::Eq) {
            self.advance(); // =
            Some(self.parse_expr()?)
        } else {
            None
        };

        Ok(FieldDecl { name, ty, default, span })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Namespace definitions
// ─────────────────────────────────────────────────────────────────────────────

impl<'a> Parser<'a> {
    fn parse_namespace_def(&mut self) -> Result<NamespaceDef, ParseError> {
        let (name, _) = self.expect_ident("namespace name")?;
        self.expect_tok(&Token::LBrace, "namespace body")?;
        self.depth += 1;

        let mut items = Vec::new();
        loop {
            match self.peek_tok() {
                Some(Token::RBrace) => { self.advance(); break; }
                Some(Token::Semi)   => { self.advance(); continue; }
                Some(Token::Hash)   => {
                    return Err(ParseError::new(
                        "'#' prefix is not valid inside a namespace body",
                        self.span(),
                    ));
                }
                None => return Err(ParseError::new("Unterminated namespace body", self.span())),
                _ => {
                    let item = self.parse_namespace_item()?;
                    items.push(item);
                }
            }
        }
        self.depth -= 1;
        Ok(NamespaceDef { name, items })
    }

    /// Inside a namespace, `fn` and nested `namespace` are allowed without `#`.
    fn parse_namespace_item(&mut self) -> Result<TopLevelItem, ParseError> {
        let span = self.span();
        let doc = None;
        let modifiers = self.parse_fn_modifiers();

        let kw = match self.peek_tok() {
            Some(Token::Ident(s)) => s.clone(),
            other => return Err(ParseError::new(
                format!("Expected fn or namespace inside namespace, got {:?}", other),
                span.clone(),
            )),
        };

        let kind = match kw.as_str() {
            "fn" => {
                self.advance();
                let func = self.parse_fn_def(modifiers)?;
                TopLevelKind::Function(func)
            }
            "namespace" => {
                self.advance();
                let ns = self.parse_namespace_def()?;
                TopLevelKind::Namespace(ns)
            }
            other => return Err(ParseError::new(
                format!("Unexpected '{}' inside namespace", other), span.clone(),
            )),
        };

        self.expect_semi("namespace item")?;
        Ok(TopLevelItem { kind, doc, span })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Type expressions
// ─────────────────────────────────────────────────────────────────────────────

impl<'a> Parser<'a> {
    fn parse_type_expr(&mut self) -> Result<TypeExpr, ParseError> {
        // Array type: [T]
        if self.peek_tok() == Some(&Token::LBracket) {
            self.advance(); // [
            let inner = self.parse_type_expr()?;
            self.expect_tok(&Token::RBracket, "array type")?;
            return Ok(TypeExpr::Array(Box::new(inner)));
        }

        // Must be a % directive type
        let span = self.span();
        let name = match self.advance() {
            Some(st) => match &st.token {
                Token::Directive(d) => d.clone(),
                Token::Void         => return Ok(TypeExpr::Void),
                other => return Err(ParseError::new(
                    format!("Expected a %%type directive, got {:?}", other),
                    Span::new(st.line, st.col),
                )),
            },
            None => return Err(ParseError::new("Expected type, got end of input", span)),
        };

        // Handle %nyx(args) -> ret
        if name == "nyx" {
            return self.parse_nyx_type();
        }

        // Handle %rust(args) -> ret
        if name == "rust" {
            return self.parse_rust_type();
        }

        // Handle %Result<T>
        if name == "Result" {
            if self.peek_tok() == Some(&Token::Lt) {
                self.advance(); // <
                let inner = self.parse_type_expr()?;
                self.expect_tok(&Token::Gt, "%Result<T>")?;
                return Ok(TypeExpr::Generic("Result".to_string(), vec![inner]));
            }
            return Ok(TypeExpr::Primitive("Result".to_string()));
        }

        // Plain primitive type
        Ok(TypeExpr::Primitive(name))
    }

    fn parse_nyx_type(&mut self) -> Result<TypeExpr, ParseError> {
        // %nyx(ParamType, ...) -> RetType
        self.expect_tok(&Token::LParen, "%nyx type")?;
        let mut arg_types = Vec::new();
        loop {
            match self.peek_tok() {
                Some(Token::RParen) => { self.advance(); break; }
                Some(Token::Comma)  => { self.advance(); continue; }
                _ => arg_types.push(self.parse_type_expr()?),
            }
        }
        let ret = if self.peek_tok() == Some(&Token::Arrow) {
            self.advance();
            self.parse_type_expr()?
        } else {
            TypeExpr::Void
        };
        Ok(TypeExpr::NyxBlock(arg_types, Box::new(ret)))
    }

    fn parse_rust_type(&mut self) -> Result<TypeExpr, ParseError> {
        // %rust(ParamType, ...) -> RetType
        self.expect_tok(&Token::LParen, "%rust type")?;
        let mut arg_types = Vec::new();
        loop {
            match self.peek_tok() {
                Some(Token::RParen) => { self.advance(); break; }
                Some(Token::Comma)  => { self.advance(); continue; }
                _ => arg_types.push(self.parse_type_expr()?),
            }
        }
        let ret = if self.peek_tok() == Some(&Token::Arrow) {
            self.advance();
            self.parse_type_expr()?
        } else {
            TypeExpr::Void
        };
        Ok(TypeExpr::RustBlock(arg_types, Box::new(ret)))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Blocks and statements
// ─────────────────────────────────────────────────────────────────────────────

impl<'a> Parser<'a> {
    /// Parse a `{ stmts... [tail] }` block.
    /// Enforces the single-bare-expression (tail) rule.
    fn parse_block(&mut self) -> Result<Block, ParseError> {
        let span = self.span();
        self.expect_tok(&Token::LBrace, "block")?;
        self.depth += 1;

        // Hard error: # inside any inner block
        if self.depth > 1 && self.peek_tok() == Some(&Token::Hash) {
            return Err(ParseError::new(
                "'#' prefix is not valid inside a block — '#' is only allowed at the top level (depth 0)",
                self.span(),
            ));
        }

        let mut stmts: Vec<Stmt> = Vec::new();
        let mut tail: Option<Box<Expr>> = None;
        let mut seen_tail = false;

        loop {
            match self.peek_tok() {
                Some(Token::RBrace) => { self.advance(); break; }
                Some(Token::Semi)   => { self.advance(); continue; }
                None => return Err(ParseError::new("Unterminated block — missing '}'", span.clone())),
                Some(Token::Hash) => {
                    return Err(ParseError::new(
                        "'#' prefix is only valid at the top level",
                        self.span(),
                    ));
                }
                _ => {
                    let (stmt_or_tail, is_tail) = self.parse_stmt_or_tail()?;

                    if is_tail {
                        // Two bare expressions in same scope → hard error
                        if seen_tail {
                            return Err(ParseError::new(
                                "A scope may have at most one bare expression (return value). \
                                 Add ';' to turn a value into a statement, or remove one expression.",
                                self.span(),
                            ));
                        }
                        seen_tail = true;
                        if let StmtKind::ExprStmt(e) = stmt_or_tail.kind {
                            tail = Some(Box::new(e));
                        }
                    } else {
                        stmts.push(stmt_or_tail);
                    }
                }
            }
        }

        self.depth -= 1;
        Ok(Block { stmts, tail, span })
    }

    /// Parse one statement or a bare expression (tail).
    /// Returns `(stmt, is_tail)` where `is_tail` is true if no `;` followed.
    fn parse_stmt_or_tail(&mut self) -> Result<(Stmt, bool), ParseError> {
        let span = self.span();

        // `let` statement (including mass let and solve)
        if self.peek_kw("let") {
            let stmt = self.parse_let_stmt()?;
            return Ok((stmt, false));
        }

        // `return` statement
        if self.peek_kw("return") {
            self.advance(); // consume "return"
            let val = if self.peek_tok() != Some(&Token::Semi) {
                Some(self.parse_expr()?)
            } else {
                None
            };
            self.expect_semi("return")?;
            return Ok((Stmt { kind: StmtKind::Return(val), span }, false));
        }

        // Otherwise parse an expression — then check for `;`
        let expr = self.parse_expr()?;

        if self.peek_tok() == Some(&Token::Semi) {
            self.advance(); // consume ;
            Ok((Stmt { kind: StmtKind::ExprStmt(expr), span }, false))
        } else {
            // No semicolon → this is the tail (return value)
            Ok((Stmt { kind: StmtKind::ExprStmt(expr), span }, true))
        }
    }

    fn parse_let_stmt(&mut self) -> Result<Stmt, ParseError> {
        let span = self.span();
        self.advance(); // consume "let"

        // Mass let: `let = { ... };`
        if self.peek_tok() == Some(&Token::Eq) {
            self.advance(); // =
            return self.parse_mass_let(span);
        }

        // %solve let
        if self.peek_directive("solve") {
            self.advance(); // consume %solve
            return self.parse_solve_stmt(span);
        }

        // Collect optional modifiers: %mut, %type
        let mut is_mut = false;
        let mut ty = TypeExpr::Inferred;

        loop {
            match self.peek_tok() {
                Some(Token::Directive(d)) if d == "mut" => {
                    is_mut = true;
                    self.advance();
                }
                Some(Token::Directive(_)) => {
                    // Type annotation directive
                    ty = self.parse_type_expr()?;
                }
                _ => break,
            }
        }

        let (name, _) = self.expect_ident("let binding")?;

        // Optional `: type` after name
        if self.peek_tok() == Some(&Token::Colon) && ty == TypeExpr::Inferred {
            self.advance(); // :
            ty = self.parse_type_expr()?;
        }

        self.expect_tok(&Token::Eq, "let binding")?;
        let value = self.parse_expr()?;
        self.expect_semi("let binding")?;

        Ok(Stmt {
            kind: StmtKind::Let(LetBinding { name, is_mut, ty, value, span: span.clone() }),
            span,
        })
    }

    /// Parse `let = { name = expr, name = expr, ... };`
    fn parse_mass_let(&mut self, span: Span) -> Result<Stmt, ParseError> {
        self.expect_tok(&Token::LBrace, "mass let")?;
        let mut bindings = Vec::new();

        loop {
            match self.peek_tok() {
                Some(Token::RBrace) => { self.advance(); break; }
                Some(Token::Comma)  => { self.advance(); continue; }
                Some(Token::Semi)   => { self.advance(); continue; }
                _ => {
                    let bind_span = self.span();
                    // Optional %mut / %type before name
                    let mut is_mut = false;
                    let mut ty = TypeExpr::Inferred;

                    loop {
                        match self.peek_tok() {
                            Some(Token::Directive(d)) if d == "mut" => {
                                is_mut = true; self.advance();
                            }
                            Some(Token::Directive(_)) => {
                                ty = self.parse_type_expr()?;
                            }
                            _ => break,
                        }
                    }

                    let (name, _) = self.expect_ident("mass let field")?;
                    self.expect_tok(&Token::Eq, "mass let field")?;
                    let value = self.parse_expr()?;
                    bindings.push(LetBinding { name, is_mut, ty, value, span: bind_span });
                }
            }
        }
        self.expect_semi("mass let")?;
        Ok(Stmt { kind: StmtKind::MassLet(bindings), span })
    }

    /// Parse `let %solve name = expr;` or `let %solve { eq; eq; } -> (x, y);`
    fn parse_solve_stmt(&mut self, span: Span) -> Result<Stmt, ParseError> {
        // System of equations: let %solve { ... } -> (x, y);
        if self.peek_tok() == Some(&Token::LBrace) {
            return self.parse_solve_system(span);
        }

        // Single unknown: let %solve x = expr;
        let (unknown, _) = self.expect_ident("%solve unknown")?;
        self.expect_tok(&Token::Eq, "%solve")?;
        let expr = self.parse_expr()?;
        self.expect_semi("%solve")?;

        Ok(Stmt { kind: StmtKind::Solve(SolveBinding { unknown, expr, span: span.clone() }), span })
    }

    /// Parse `let %solve { eq; eq; } -> (x, y);`
    fn parse_solve_system(&mut self, span: Span) -> Result<Stmt, ParseError> {
        self.expect_tok(&Token::LBrace, "%solve system")?;
        let mut equations = Vec::new();

        loop {
            match self.peek_tok() {
                Some(Token::RBrace) => { self.advance(); break; }
                Some(Token::Semi)   => { self.advance(); continue; }
                None => return Err(ParseError::new("Unterminated %solve system block", span.clone())),
                _ => {
                    let eq = self.parse_expr()?;
                    equations.push(eq);
                    // Equations end with ;
                    if self.peek_tok() == Some(&Token::Semi) { self.advance(); }
                }
            }
        }

        // -> (x, y)
        self.expect_tok(&Token::Arrow, "%solve system")?;
        self.expect_tok(&Token::LParen, "%solve unknowns")?;
        let mut unknowns = Vec::new();
        loop {
            match self.peek_tok() {
                Some(Token::RParen) => { self.advance(); break; }
                Some(Token::Comma)  => { self.advance(); continue; }
                Some(Token::Ident(_)) => {
                    let (name, _) = self.expect_ident("%solve unknown")?;
                    unknowns.push(name);
                }
                _ => return Err(ParseError::new("Expected identifier in %solve unknowns", self.span())),
            }
        }
        self.expect_semi("%solve system")?;

        Ok(Stmt { kind: StmtKind::SolveSystem(SolveSystem { equations, unknowns, span: span.clone() }), span })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Expression parsing (recursive descent with precedence climbing)
// ─────────────────────────────────────────────────────────────────────────────

impl<'a> Parser<'a> {
    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_assignment()
    }

    // ── Precedence levels (low → high) ───────────────────────────────
    // assignment  =
    // logical     || &&
    // equality    == !=
    // comparison  < > <= >=
    // additive    + -
    // multiplicative  * / %
    // unary       ! -
    // postfix     . () [] ? await
    // primary

    fn parse_assignment(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        let lhs = self.parse_logical()?;

        if self.peek_tok() == Some(&Token::Eq) {
            // Make sure next token is NOT = (which would be ==, already handled)
            self.advance(); // consume =
            let rhs = self.parse_assignment()?;
            return Ok(Expr {
                kind: ExprKind::Assign(Box::new(lhs), Box::new(rhs)),
                span,
            });
        }
        Ok(lhs)
    }

    fn parse_logical(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        let mut lhs = self.parse_equality()?;
        loop {
            let op = match self.peek_tok() {
                Some(Token::AmpAmp)  => BinaryOp::And,
                Some(Token::PipePipe) => BinaryOp::Or,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_equality()?;
            lhs = Expr { kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), span: span.clone(), id: self.alloc_id() };
        }
        Ok(lhs)
    }

    fn parse_equality(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        let mut lhs = self.parse_comparison()?;
        loop {
            let op = match self.peek_tok() {
                Some(Token::EqEq)   => BinaryOp::Eq,
                Some(Token::BangEq) => BinaryOp::Ne,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_comparison()?;
            lhs = Expr { kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), span: span.clone(), id: self.alloc_id() };
        }
        Ok(lhs)
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        let mut lhs = self.parse_additive()?;
        loop {
            let op = match self.peek_tok() {
                Some(Token::Lt)   => BinaryOp::Lt,
                Some(Token::Gt)   => BinaryOp::Gt,
                Some(Token::LtEq) => BinaryOp::Le,
                Some(Token::GtEq) => BinaryOp::Ge,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_additive()?;
            lhs = Expr { kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), span: span.clone(), id: self.alloc_id() };
        }
        Ok(lhs)
    }

    fn parse_additive(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        let mut lhs = self.parse_multiplicative()?;
        loop {
            let op = match self.peek_tok() {
                Some(Token::Plus)  => BinaryOp::Add,
                Some(Token::Minus) => BinaryOp::Sub,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_multiplicative()?;
            lhs = Expr { kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), span: span.clone(), id: self.alloc_id() };
        }
        Ok(lhs)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek_tok() {
                Some(Token::Star)    => BinaryOp::Mul,
                Some(Token::Slash)   => BinaryOp::Div,
                Some(Token::Percent) => BinaryOp::Mod,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_unary()?;
            lhs = Expr { kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), span: span.clone(), id: self.alloc_id() };
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        match self.peek_tok() {
            Some(Token::Bang) => {
                self.advance();
                let operand = self.parse_unary()?;
                return Ok(Expr { kind: ExprKind::Unary(UnaryOp::Not, Box::new(operand)), span, id: self.alloc_id() });
            }
            Some(Token::Minus) => {
                self.advance();
                let operand = self.parse_unary()?;
                return Ok(Expr { kind: ExprKind::Unary(UnaryOp::Neg, Box::new(operand)), span, id: self.alloc_id() });
            }
            Some(Token::Amp) => {
                self.advance();
                let operand = self.parse_unary()?;
                return Ok(Expr { kind: ExprKind::Borrow(Box::new(operand)), span, id: self.alloc_id() });
            }
            Some(Token::AmpMut) => {
                self.advance();
                let operand = self.parse_unary()?;
                return Ok(Expr { kind: ExprKind::BorrowMut(Box::new(operand)), span, id: self.alloc_id() });
            }
            _ => {}
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary()?;

        loop {
            let span = self.span();
            match self.peek_tok() {
                // Function call: expr(args)
                Some(Token::LParen) => {
                    let args = self.parse_call_args()?;
                    expr = Expr { kind: ExprKind::Call(Box::new(expr), args), span, id: self.alloc_id() };
                }

                // Index: expr[idx]
                Some(Token::LBracket) => {
                    self.advance(); // [
                    let idx = self.parse_expr()?;
                    self.expect_tok(&Token::RBracket, "index expression")?;
                    expr = Expr { kind: ExprKind::Index(Box::new(expr), Box::new(idx)), span, id: self.alloc_id() };
                }

                // Field access or method call: expr.field or expr.method(args)
                Some(Token::Dot) => {
                    self.advance(); // .
                    let (field, _) = self.expect_ident("field or method name")?;
                    if self.peek_tok() == Some(&Token::LParen) {
                        let args = self.parse_call_args()?;
                        expr = Expr { kind: ExprKind::MethodCall(Box::new(expr), field, args), span, id: self.alloc_id() };
                    } else {
                        expr = Expr { kind: ExprKind::Field(Box::new(expr), field), span, id: self.alloc_id() };
                    }
                }

                // ? propagation
                Some(Token::Question) => {
                    self.advance();
                    expr = Expr { kind: ExprKind::Propagate(Box::new(expr)), span, id: self.alloc_id() };
                }

                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_call_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        self.expect_tok(&Token::LParen, "call arguments")?;
        let mut args = Vec::new();
        loop {
            match self.peek_tok() {
                Some(Token::RParen) => { self.advance(); break; }
                Some(Token::Comma)  => { self.advance(); continue; }
                _ => args.push(self.parse_expr()?),
            }
        }
        Ok(args)
    }

    // ── Primary expressions ───────────────────────────────────────────

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();

        match self.peek_tok().cloned() {
            // Literals
            Some(Token::Int(n))   => { self.advance(); Ok(Expr { kind: ExprKind::Int(n),   span, id: self.alloc_id() }) }
            Some(Token::Float(f)) => { self.advance(); Ok(Expr { kind: ExprKind::Float(f), span, id: self.alloc_id() }) }
            Some(Token::Str(s))   => { self.advance(); Ok(self.parse_interpolated_string(s, span)) }
            Some(Token::True)     => { self.advance(); Ok(Expr { kind: ExprKind::Bool(true),  span, id: self.alloc_id() }) }
            Some(Token::False)    => { self.advance(); Ok(Expr { kind: ExprKind::Bool(false), span, id: self.alloc_id() }) }
            Some(Token::Void)     => { self.advance(); Ok(Expr { kind: ExprKind::Void, span, id: self.alloc_id() }) }

            // Grouped expression: (expr)
            Some(Token::LParen) => {
                self.advance(); // (
                let inner = self.parse_expr()?;
                self.expect_tok(&Token::RParen, "grouped expression")?;
                Ok(inner)
            }

            // Array literal: [a, b, c]
            Some(Token::LBracket) => self.parse_array_literal(span),

            // Block expression: { ... }
            Some(Token::LBrace) => {
                let block = self.parse_block()?;
                Ok(Expr { kind: ExprKind::Block(block), span, id: self.alloc_id() })
            }

            // if expression
            Some(Token::Ident(ref s)) if s == "if" => self.parse_if_expr(),

            // while expression
            Some(Token::Ident(ref s)) if s == "while" => self.parse_while_expr(),

            // loop expression
            Some(Token::Ident(ref s)) if s == "loop" => self.parse_loop_expr(),

            // for expression
            Some(Token::Ident(ref s)) if s == "for" => self.parse_for_expr(),

            // match expression
            Some(Token::Ident(ref s)) if s == "match" => self.parse_match_expr(),

            // break [value]
            Some(Token::Ident(ref s)) if s == "break" => {
                self.advance();
                let val = if !matches!(self.peek_tok(), Some(Token::Semi) | Some(Token::RBrace) | None) {
                    Some(Box::new(self.parse_expr()?))
                } else { None };
                Ok(Expr { kind: ExprKind::Break(val), span, id: self.alloc_id() })
            }

            // await expr
            Some(Token::Ident(ref s)) if s == "await" => {
                self.advance();
                let inner = self.parse_expr()?;
                Ok(Expr { kind: ExprKind::Await(Box::new(inner)), span, id: self.alloc_id() })
            }

            // panic(msg)
            Some(Token::Ident(ref s)) if s == "panic" => {
                self.advance();
                let args = self.parse_call_args()?;
                let msg = args.into_iter().next()
                    .ok_or_else(|| self.err("panic() requires a message argument"))?;
                Ok(Expr { kind: ExprKind::Panic(Box::new(msg)), span, id: self.alloc_id() })
            }

            // ok(value) / err(msg)
            Some(Token::Ident(ref s)) if s == "ok" => {
                self.advance();
                let args = self.parse_call_args()?;
                let val = args.into_iter().next()
                    .ok_or_else(|| self.err("ok() requires a value argument"))?;
                Ok(Expr { kind: ExprKind::Ok(Box::new(val)), span, id: self.alloc_id() })
            }
            Some(Token::Ident(ref s)) if s == "err" => {
                self.advance();
                let args = self.parse_call_args()?;
                let val = args.into_iter().next()
                    .ok_or_else(|| self.err("err() requires a message argument"))?;
                Ok(Expr { kind: ExprKind::Err(Box::new(val)), span, id: self.alloc_id() })
            }

            // Directive-led expressions: %nyx, %rust, %spawn
            Some(Token::Directive(ref d)) => {
                let d = d.clone();
                match d.as_str() {
                    "nyx"   => self.parse_nyx_block_expr(span),
                    "rust"  => self.parse_rust_block_expr(span),
                    "spawn" => {
                        return Err(ParseError::new(
                            "%spawn cannot be used on a code block — only on #fn %spawn functions",
                            span,
                        ));
                    }
                    other => {
                        // Could be a type used in a cast-like context — leave to type checker
                        self.advance();
                        Ok(Expr { kind: ExprKind::Ident(format!("%{}", other)), span, id: self.alloc_id() })
                    }
                }
            }

            // Identifier or path: foo, foo::bar, foo.create { }
            Some(Token::Ident(_)) => self.parse_ident_or_path_expr(span),

            Some(other) => Err(ParseError::new(
                format!("Unexpected token in expression: {:?}", other),
                span,
            )),
            None => Err(ParseError::new("Unexpected end of input in expression", span)),
        }
    }

    // ── Identifier / path / create ────────────────────────────────────

    fn parse_ident_or_path_expr(&mut self, span: Span) -> Result<Expr, ParseError> {
        let (first, _) = self.expect_ident("identifier")?;
        let mut path = vec![first];

        // Build :: path
        while self.peek_tok() == Some(&Token::ColonColon) {
            self.advance(); // ::
            let (segment, _) = self.expect_ident("path segment")?;
            path.push(segment);
        }

        let base = if path.len() == 1 {
            Expr { kind: ExprKind::Ident(path.into_iter().next().unwrap()), span: span.clone(), id: self.alloc_id() }
        } else {
            Expr { kind: ExprKind::Path(path), span: span.clone(), id: self.alloc_id() }
        };

        // Class.create { field = val, ... }
        if self.peek_tok() == Some(&Token::Dot) && self.peek_next() == Some(&Token::Ident("create".to_string())) {
            // Actually check it's `.create`
            self.advance(); // .
            self.advance(); // create
            return self.parse_create_expr(base, span);
        }

        Ok(base)
    }

    fn parse_create_expr(&mut self, class: Expr, span: Span) -> Result<Expr, ParseError> {
        self.expect_tok(&Token::LBrace, "create expression")?;
        let mut fields = Vec::new();

        loop {
            match self.peek_tok() {
                Some(Token::RBrace) => { self.advance(); break; }
                Some(Token::Comma)  => { self.advance(); continue; }
                Some(Token::Ident(_)) => {
                    let (name, _) = self.expect_ident("create field")?;
                    self.expect_tok(&Token::Eq, "create field")?;
                    let value = self.parse_expr()?;
                    fields.push((name, value));
                }
                _ => return Err(ParseError::new(
                    format!("Expected field name in create expression, got {:?}", self.peek_tok()),
                    self.span(),
                )),
            }
        }

        Ok(Expr { kind: ExprKind::Create(Box::new(class), fields), span, id: self.alloc_id() })
    }

    // ── Array literal ─────────────────────────────────────────────────

    fn parse_array_literal(&mut self, span: Span) -> Result<Expr, ParseError> {
        self.advance(); // [
        let mut elems = Vec::new();
        loop {
            match self.peek_tok() {
                Some(Token::RBracket) => { self.advance(); break; }
                Some(Token::Comma)    => { self.advance(); continue; }
                _ => {
                    let elem = self.parse_expr()?;
                    // Check for range: 1..10
                    if self.peek_tok() == Some(&Token::DotDot) {
                        self.advance(); // ..
                        let end = self.parse_expr()?;
                        self.expect_tok(&Token::RBracket, "range expression")?;
                        return Ok(Expr {
                            kind: ExprKind::Range(Box::new(elem), Box::new(end)),
                            span,
                        });
                    }
                    elems.push(elem);
                }
            }
        }
        Ok(Expr { kind: ExprKind::Array(elems), span, id: self.alloc_id() })
    }

    // ── Control flow expressions ──────────────────────────────────────

    fn parse_if_expr(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        self.advance(); // consume "if"
        let cond = self.parse_expr()?;
        let then_block = self.parse_block()?;
        let mut else_ifs = Vec::new();
        let mut else_block = None;

        loop {
            if self.peek_kw("else") {
                self.advance(); // consume "else"
                if self.peek_kw("if") {
                    self.advance(); // consume "if"
                    let ei_cond = self.parse_expr()?;
                    let ei_block = self.parse_block()?;
                    else_ifs.push((ei_cond, ei_block));
                } else {
                    else_block = Some(self.parse_block()?);
                    break;
                }
            } else {
                break;
            }
        }

        Ok(Expr { kind: ExprKind::If(Box::new(cond), then_block, else_ifs, else_block), span, id: self.alloc_id() })
    }

    fn parse_while_expr(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        self.advance(); // consume "while"
        let cond = self.parse_expr()?;
        let body = self.parse_block()?;
        Ok(Expr { kind: ExprKind::While(Box::new(cond), body), span, id: self.alloc_id() })
    }

    fn parse_loop_expr(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        self.advance(); // consume "loop"
        let body = self.parse_block()?;
        Ok(Expr { kind: ExprKind::Loop(body), span, id: self.alloc_id() })
    }

    fn parse_for_expr(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        self.advance(); // consume "for"
        let (binding, _) = self.expect_ident("for binding")?;
        // expect "in"
        match self.advance() {
            Some(st) if matches!(&st.token, Token::Ident(s) if s == "in") => {}
            Some(st) => return Err(ParseError::new(
                format!("Expected 'in' in for loop, got {:?}", st.token),
                Span::new(st.line, st.col),
            )),
            None => return Err(ParseError::new("Expected 'in' in for loop", span.clone())),
        }
        let iter = self.parse_expr()?;
        let body = self.parse_block()?;
        Ok(Expr { kind: ExprKind::For(binding, Box::new(iter), body), span, id: self.alloc_id() })
    }

    fn parse_match_expr(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        self.advance(); // consume "match"
        let subject = self.parse_expr()?;
        self.expect_tok(&Token::LBrace, "match body")?;
        self.depth += 1;

        let mut arms = Vec::new();
        loop {
            match self.peek_tok() {
                Some(Token::RBrace) => { self.advance(); break; }
                Some(Token::Comma)  => { self.advance(); continue; }
                None => return Err(ParseError::new("Unterminated match expression", span.clone())),
                _ => arms.push(self.parse_match_arm()?),
            }
        }

        self.depth -= 1;
        Ok(Expr { kind: ExprKind::Match(Box::new(subject), arms), span, id: self.alloc_id() })
    }

    fn parse_match_arm(&mut self) -> Result<MatchArm, ParseError> {
        let span = self.span();
        let pattern = self.parse_pattern()?;
        // expect =>
        match self.advance() {
            Some(st) if matches!(&st.token, Token::Eq) => {
                // consume the > of =>
                match self.advance() {
                    Some(st) if matches!(&st.token, Token::Gt) => {}
                    Some(st) => return Err(ParseError::new(
                        format!("Expected '=>' in match arm, got '={:?}'", st.token),
                        Span::new(st.line, st.col),
                    )),
                    None => return Err(ParseError::new("Expected '>' after '=' in match arm", span.clone())),
                }
            }
            Some(st) => return Err(ParseError::new(
                format!("Expected '=>' in match arm, got {:?}", st.token),
                Span::new(st.line, st.col),
            )),
            None => return Err(ParseError::new("Expected '=>' in match arm", span.clone())),
        }
        let body = self.parse_expr()?;
        Ok(MatchArm { pattern, body, span })
    }

    fn parse_pattern(&mut self) -> Result<Pattern, ParseError> {
        match self.peek_tok().cloned() {
            Some(Token::Ident(ref s)) if s == "_" => {
                self.advance();
                Ok(Pattern::Wildcard)
            }
            Some(Token::Ident(s)) => {
                self.advance();
                Ok(Pattern::Ident(s))
            }
            Some(Token::Int(_)) | Some(Token::Float(_)) | Some(Token::Str(_))
            | Some(Token::True) | Some(Token::False) => {
                let lit = self.parse_primary()?;
                // Range pattern: 1..10
                if self.peek_tok() == Some(&Token::DotDot) {
                    self.advance(); // ..
                    let end = self.parse_primary()?;
                    return Ok(Pattern::Range(lit, end));
                }
                Ok(Pattern::Literal(lit))
            }
            _ => Err(ParseError::new(
                format!("Expected pattern in match arm, got {:?}", self.peek_tok()),
                self.span(),
            )),
        }
    }

    // ── Language block expressions ────────────────────────────────────

    fn parse_nyx_block_expr(&mut self, span: Span) -> Result<Expr, ParseError> {
        self.advance(); // consume %nyx directive token
        let params = self.parse_param_list()?;
        let ret_ty = if self.peek_tok() == Some(&Token::Arrow) {
            self.advance();
            Some(self.parse_type_expr()?)
        } else { None };
        let body = self.parse_block()?;
        Ok(Expr { kind: ExprKind::NyxBlock(params, ret_ty, body), span, id: self.alloc_id() })
    }

    fn parse_rust_block_expr(&mut self, span: Span) -> Result<Expr, ParseError> {
        self.advance(); // consume %rust directive token
        let params = self.parse_param_list()?;
        let ret_ty = if self.peek_tok() == Some(&Token::Arrow) {
            self.advance();
            Some(self.parse_type_expr()?)
        } else { None };

        // Capture raw Rust source between { }
        let rust_src = self.capture_raw_block()?;
        let _ = ret_ty; // passed to ExprKind
        Ok(Expr {
            kind: ExprKind::RustBlock(params, None, rust_src),
            span,
        })
    }

    /// Capture everything between `{` and the matching `}` as a raw string.
    fn capture_raw_block(&mut self) -> Result<String, ParseError> {
        let span = self.span();
        self.expect_tok(&Token::LBrace, "raw block")?;
        let mut depth = 1usize;
        let mut parts: Vec<String> = Vec::new();

        loop {
            match self.advance() {
                None => return Err(ParseError::new("Unterminated raw block", span)),
                Some(st) => match &st.token {
                    Token::LBrace => { depth += 1; parts.push("{".to_string()); }
                    Token::RBrace => {
                        depth -= 1;
                        if depth == 0 { break; }
                        parts.push("}".to_string());
                    }
                    tok => parts.push(crate::make_pass::token_to_rust_repr_pub(tok)),
                }
            }
        }
        Ok(parts.join(" "))
    }

    // ── String interpolation ──────────────────────────────────────────

    /// Walk a string literal and split it into literal and interpolated segments.
    /// Interpolated segments look like `{expr}` inside the string.
    fn parse_interpolated_string(&mut self, raw: String, span: Span) -> Expr {
        // A quick scan: if there is no `{`, return a plain Str.
        if !raw.contains('{') {
            return Expr { kind: ExprKind::Str(raw), span, id: self.alloc_id() };
        }

        let mut segments: Vec<InterpolSegment> = Vec::new();
        let mut lit = String::new();
        let mut chars = raw.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '{' {
                if !lit.is_empty() {
                    segments.push(InterpolSegment::Lit(std::mem::take(&mut lit)));
                }
                // Collect the inner expression source until matching }
                let mut inner = String::new();
                let mut depth = 1usize;
                for ic in chars.by_ref() {
                    if ic == '{' { depth += 1; inner.push(ic); }
                    else if ic == '}' {
                        depth -= 1;
                        if depth == 0 { break; }
                        inner.push(ic);
                    } else {
                        inner.push(ic);
                    }
                }
                // We store the raw inner string; the type checker will resolve it.
                segments.push(InterpolSegment::Lit(format!("{{{}}}", inner)));
            } else {
                lit.push(c);
            }
        }
        if !lit.is_empty() {
            segments.push(InterpolSegment::Lit(lit));
        }

        Expr { kind: ExprKind::Interpolated(segments), span, id: self.alloc_id() }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::make_pass::{CompileConfig, run_make_pass};

    fn parse_src(src: &str) -> Program {
        let tokens = lex(src).expect("lex failed");
        let config = run_make_pass(&tokens).expect("make failed");
        parse(&tokens, &config).expect("parse failed")
    }

    fn parse_err(src: &str) -> String {
        let tokens = lex(src).expect("lex failed");
        let config = run_make_pass(&tokens).expect("make failed");
        parse(&tokens, &config).unwrap_err().message
    }

    fn first_item(src: &str) -> TopLevelKind {
        parse_src(src).items.into_iter().next().unwrap().kind
    }

    // ── Top-level function ────────────────────────────────────────────

    #[test]
    fn test_simple_fn() {
        let prog = parse_src("#fn add(x: %i32, y: %i32) -> %i32 { x + y };");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0].kind {
            TopLevelKind::Function(f) => {
                assert_eq!(f.name, "add");
                assert_eq!(f.params.len(), 2);
                assert!(matches!(f.return_type, Some(TypeExpr::Primitive(ref s)) if s == "i32"));
            }
            _ => panic!("expected function"),
        }
    }

    #[test]
    fn test_fn_with_pub_modifier() {
        let prog = parse_src("#fn %pub greet() -> %void { };");
        match &prog.items[0].kind {
            TopLevelKind::Function(f) => assert!(f.modifiers.is_pub),
            _ => panic!(),
        }
    }

    #[test]
    fn test_fn_async_spawn() {
        let prog = parse_src("#fn %spawn %async run() -> %void { };");
        match &prog.items[0].kind {
            TopLevelKind::Function(f) => {
                assert!(f.modifiers.is_spawn);
                assert!(f.modifiers.is_async);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_missing_hash_produces_warning() {
        let tokens = lex("fn hello() -> %void { };").expect("lex failed");
        let config = run_make_pass(&tokens).expect("make failed");
        let prog = parse(&tokens, &config).expect("parse failed");
        assert!(!prog.warnings.is_empty());
        assert!(prog.warnings[0].message.contains("should be prefixed with '#'"));
    }

    #[test]
    fn test_hash_inside_block_is_error() {
        assert!(parse_err("#fn foo() -> %void { #fn bar() -> %void { }; };")
            .contains("only valid at the top level"));
    }

    // ── Let bindings ──────────────────────────────────────────────────

    #[test]
    fn test_let_binding() {
        let prog = parse_src("#fn main() -> %void { let x = 42; };");
        match &prog.items[0].kind {
            TopLevelKind::Function(f) => {
                assert_eq!(f.body.stmts.len(), 1);
                assert!(matches!(f.body.stmts[0].kind, StmtKind::Let(_)));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_let_mut_with_type() {
        let prog = parse_src("#fn main() -> %void { let %mut %i64 x = 0; };");
        match &prog.items[0].kind {
            TopLevelKind::Function(f) => {
                if let StmtKind::Let(ref b) = f.body.stmts[0].kind {
                    assert!(b.is_mut);
                    assert!(matches!(b.ty, TypeExpr::Primitive(ref s) if s == "i64"));
                } else { panic!(); }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_mass_let() {
        let prog = parse_src("#fn main() -> %void { let = { x = 1, y = 2, }; };");
        match &prog.items[0].kind {
            TopLevelKind::Function(f) => {
                assert!(matches!(f.body.stmts[0].kind, StmtKind::MassLet(_)));
            }
            _ => panic!(),
        }
    }

    // ── Return value / tail ───────────────────────────────────────────

    #[test]
    fn test_tail_expression() {
        let prog = parse_src("#fn add(x: %i32, y: %i32) -> %i32 { x + y };");
        match &prog.items[0].kind {
            TopLevelKind::Function(f) => {
                assert!(f.body.tail.is_some());
                assert!(f.body.stmts.is_empty());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_two_bare_expressions_is_error() {
        assert!(parse_err("#fn bad() -> %i32 { x\n y };")
            .contains("at most one bare expression"));
    }

    // ── Class ─────────────────────────────────────────────────────────

    #[test]
    fn test_class_with_create_and_method() {
        let prog = parse_src(
            "#class Circle { \
               create { let radius: %f64, }; \
               fn area(%self) -> %f64 { 3 }; \
             };"
        );
        match &prog.items[0].kind {
            TopLevelKind::Class(c) => {
                assert_eq!(c.name, "Circle");
                assert_eq!(c.create_block.fields.len(), 1);
                assert_eq!(c.methods.len(), 1);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_create_outside_class_is_error() {
        // create { } as a standalone statement has no meaning — parser won't
        // recognise it as a valid expression outside a class body.
        // This should produce an unexpected token error.
        assert!(parse_err("#fn bad() -> %void { create { }; };").is_empty() == false);
    }

    // ── Control flow ──────────────────────────────────────────────────

    #[test]
    fn test_if_else() {
        let prog = parse_src(
            "#fn classify(x: %i32) -> %str { \
               if x > 0 { \"pos\" } else { \"neg\" } \
             };"
        );
        match &prog.items[0].kind {
            TopLevelKind::Function(f) => {
                assert!(matches!(f.body.tail.as_ref().map(|e| &e.kind),
                    Some(ExprKind::If(_, _, _, _))));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_for_loop() {
        let prog = parse_src(
            "#fn main() -> %void { for i in 1..10 { print(i); }; };"
        );
        match &prog.items[0].kind {
            TopLevelKind::Function(f) => {
                assert!(matches!(f.body.stmts[0].kind,
                    StmtKind::ExprStmt(Expr { kind: ExprKind::For(_, _, _), .. })));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_match_expr() {
        let prog = parse_src(
            "#fn main() -> %void { \
               let r = match x { 0 => \"zero\", _ => \"other\", }; \
             };"
        );
        match &prog.items[0].kind {
            TopLevelKind::Function(f) => {
                if let StmtKind::Let(ref b) = f.body.stmts[0].kind {
                    assert!(matches!(b.value.kind, ExprKind::Match(_, _)));
                } else { panic!(); }
            }
            _ => panic!(),
        }
    }

    // ── %solve ────────────────────────────────────────────────────────

    #[test]
    fn test_solve_single() {
        let prog = parse_src(
            "#fn main() -> %void { let %solve x = 2 * x + 4; };"
        );
        match &prog.items[0].kind {
            TopLevelKind::Function(f) => {
                assert!(matches!(f.body.stmts[0].kind, StmtKind::Solve(_)));
            }
            _ => panic!(),
        }
    }

    // ── Namespace ─────────────────────────────────────────────────────

    #[test]
    fn test_namespace() {
        let prog = parse_src(
            "#namespace math { fn add(x: %i32, y: %i32) -> %i32 { x + y }; };"
        );
        match &prog.items[0].kind {
            TopLevelKind::Namespace(ns) => {
                assert_eq!(ns.name, "math");
                assert_eq!(ns.items.len(), 1);
            }
            _ => panic!(),
        }
    }

    // ── Expressions ───────────────────────────────────────────────────

    #[test]
    fn test_binary_arithmetic() {
        let prog = parse_src("#fn f() -> %i32 { 1 + 2 * 3 };");
        match &prog.items[0].kind {
            TopLevelKind::Function(f) => {
                assert!(matches!(f.body.tail.as_ref().map(|e| &e.kind),
                    Some(ExprKind::Binary(BinaryOp::Add, _, _))));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_method_call() {
        let prog = parse_src("#fn f() -> %void { x.length(); };");
        match &prog.items[0].kind {
            TopLevelKind::Function(f) => {
                assert!(matches!(f.body.stmts[0].kind,
                    StmtKind::ExprStmt(Expr { kind: ExprKind::MethodCall(_, _, _), .. })));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_index_expression() {
        let prog = parse_src("#fn f() -> %i32 { arr[1] };");
        match &prog.items[0].kind {
            TopLevelKind::Function(f) => {
                assert!(matches!(f.body.tail.as_ref().map(|e| &e.kind),
                    Some(ExprKind::Index(_, _))));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_result_propagation() {
        let prog = parse_src("#fn f() -> %Result<%i32> { some_fn()? };");
        match &prog.items[0].kind {
            TopLevelKind::Function(f) => {
                assert!(matches!(f.body.tail.as_ref().map(|e| &e.kind),
                    Some(ExprKind::Propagate(_))));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_nyx_block_expr() {
        let prog = parse_src(
            "#fn f() -> %void { let double = %nyx(x: %i32) -> %i32 { x * 2 }; };"
        );
        match &prog.items[0].kind {
            TopLevelKind::Function(f) => {
                if let StmtKind::Let(ref b) = f.body.stmts[0].kind {
                    assert!(matches!(b.value.kind, ExprKind::NyxBlock(_, _, _)));
                } else { panic!(); }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_doc_comment_attached() {
        let prog = parse_src("/// Adds two numbers\n#fn add(x: %i32, y: %i32) -> %i32 { x + y };");
        assert!(prog.items[0].doc.is_some());
        assert_eq!(prog.items[0].doc.as_deref(), Some("Adds two numbers"));
    }
}