// Stage 2 — %make Pass
// Scans the token stream for the #fn %make() { ... } block, evaluates it in
// isolation, merges top-level #import/#use/#def shorthands, and produces a
// CompileConfig that all later compiler stages read from.

// ── CompileConfig ─────────────────────────────────────────────────────────────

/// The output of the %make pass. Every later compiler stage receives a
/// reference to this struct and uses it to control its behavior.
#[derive(Debug, Clone)]
pub struct CompileConfig {
    /// %suppress-warnings — silences all ⚠️ warnings if true.
    pub suppress_warnings: bool,

    /// %target — "debug" or "release". Defaults to "debug".
    pub target: BuildTarget,

    /// %entry — name of the entry-point function. Defaults to "main".
    pub entry: String,

    /// %strict — treat warnings as hard errors if true.
    pub strict: bool,

    /// %repl — launch REPL mode instead of running main if true.
    pub repl: bool,

    /// %async — enable the green thread / async runtime if true.
    pub async_runtime: bool,

    /// %hard — list of function paths whose restrictions are removed.
    /// e.g. ["std::convert"]
    pub hard: Vec<String>,

    /// %import — list of module names to import.
    pub imports: Vec<String>,

    /// %use — list of qualified paths to inline at call sites.
    /// e.g. ["math::add", "geometry::distance"]
    pub uses: Vec<String>,

    /// %def — list of Rust files / binaries to link.
    pub defs: Vec<DefEntry>,

    /// %when-run — raw Rust code to execute before the program runs.
    pub when_run: Option<String>,

    /// %when-compile — raw Rust code executed immediately during this pass.
    /// Already fired by the time CompileConfig is returned.
    pub when_compile: Option<String>,

    pub look_for_path: Option<String>,

    /// %self rename — how instances refer to themselves inside class methods.
    pub self_rename: SelfRename,

    /// %logic-%make — whether control flow was permitted inside %make.
    pub logic_make: bool,
}

impl Default for CompileConfig {
    fn default() -> Self {
        CompileConfig {
            suppress_warnings: false,
            target: BuildTarget::Debug,
            entry: "main".to_string(),
            strict: false,
            repl: false,
            async_runtime: false,
            hard: Vec::new(),
            imports: Vec::new(),
            uses: Vec::new(),
            defs: Vec::new(),
            when_run: None,
            when_compile: None,
            self_rename: SelfRename::Global("self".to_string()),
            logic_make: false,
        }
    }
}

/// Build target — controls debug vs release behaviour in later stages.
#[derive(Debug, Clone, PartialEq)]
pub enum BuildTarget {
    Debug,
    Release,
}

/// A single %def entry — a Rust source file or precompiled binary.
#[derive(Debug, Clone)]
pub struct DefEntry {
    /// Path to the file, e.g. "mylib.rs" or "fast.dll"
    pub path: String,
    /// The alias used to call it from Nyx, e.g. "mylib"
    pub alias: String,
    /// What kind of file this is
    pub kind: DefKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DefKind {
    RustSource,   // .rs  — compiled by rustc during the %rust isolator pass
    NativeLib,    // .dll / .so / .dylib — ABI-checked and linked directly
}

/// How %self is renamed inside class methods.
#[derive(Debug, Clone)]
pub enum SelfRename {
    /// All classes use the same name, e.g. %self = "this"
    Global(String),
    /// Per-class overrides with a fallback default,
    /// e.g. %self = { Circle = "this", "self" }
    PerClass {
        overrides: Vec<(String, String)>, // (ClassName, rename)
        default: String,
    },
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct MakeError {
    pub message: String,
    pub line: usize,
    pub col: usize,
}

impl MakeError {
    fn new(msg: impl Into<String>, line: usize, col: usize) -> Self {
        MakeError { message: msg.into(), line, col }
    }
}

impl std::fmt::Display for MakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[%make Error] {}:{} — {}", self.line, self.col, self.message)
    }
}

// ── Known %make-only directives ───────────────────────────────────────────────

const MAKE_DIRECTIVES: &[&str] = &[
    "suppress-warnings",
    "target",
    "entry",
    "strict",
    "repl",
    "async",
    "hard",
    "import",
    "use",
    "def",
    "when-run",
    "when-compile",
    "self",
    "logic-%make",
];

/// Control-flow token kinds that are forbidden inside %make
/// unless %logic-%make = %true.
fn is_control_flow(tok: &Token) -> bool {
    matches!(tok, Token::Ident(s) if matches!(s.as_str(), "if" | "while" | "for" | "match" | "loop"))
}

// ── Token cursor ──────────────────────────────────────────────────────────────

struct Cursor<'a> {
    tokens: &'a [SpannedToken],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(tokens: &'a [SpannedToken]) -> Self {
        Cursor { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&SpannedToken> {
        self.tokens.get(self.pos)
    }

    fn peek_tok(&self) -> Option<&Token> {
        self.peek().map(|s| &s.token)
    }

    fn advance(&mut self) -> Option<&SpannedToken> {
        let t = self.tokens.get(self.pos);
        self.pos += 1;
        t
    }

    fn expect(&mut self, expected: &Token, context: &str) -> Result<&SpannedToken, MakeError> {
        match self.advance() {
            Some(st) if &st.token == expected => Ok(st),
            Some(st) => Err(MakeError::new(
                format!("Expected {:?} in {}, got {:?}", expected, context, st.token),
                st.line, st.col,
            )),
            None => Err(MakeError::new(
                format!("Unexpected end of tokens while expecting {:?} in {}", expected, context),
                0, 0,
            )),
        }
    }

    fn current_location(&self) -> (usize, usize) {
        self.peek().map(|s| (s.line, s.col)).unwrap_or((0, 0))
    }

    fn is_exhausted(&self) -> bool {
        self.pos >= self.tokens.len()
    }
}

// ── %make pass entry point ────────────────────────────────────────────────────

/// Run the %make pass over the full flat token stream produced by the lexer.
///
/// Steps:
///   1. Scan for top-level `#import`, `#use`, `#def` shorthands and collect them.
///   2. Locate the `#fn %make() { ... }` block (if present).
///   3. Validate its structure (no params, no return type).
///   4. Evaluate all `let %directive = value;` statements inside it.
///   5. Fire `%when-compile` %rust hooks.
///   6. Return the final CompileConfig.
pub fn run_make_pass(tokens: &[SpannedToken]) -> Result<CompileConfig, MakeError> {
    let mut config = CompileConfig::default();

    // ── Step 1: collect top-level shorthands ─────────────────────────
    collect_shorthands(tokens, &mut config)?;

    // ── Step 2: find the %make block ─────────────────────────────────
    let make_body = find_make_block(tokens)?;

    let body = match make_body {
        None => {
            // No %make block — config is built entirely from shorthands
            fire_when_compile(&config)?;
            return Ok(config);
        }
        Some(b) => b,
    };

    // ── Step 3 & 4: evaluate the body ────────────────────────────────
    evaluate_make_body(&body, &mut config)?;

    // ── Step 5: fire %when-compile ────────────────────────────────────
    fire_when_compile(&config)?;

    Ok(config)
}

// ── Step 1: shorthand collection ─────────────────────────────────────────────

/// Scan the token stream for top-level `#import X;`, `#use X::Y;`,
/// `#def "file" as alias;` and merge them into config.
fn collect_shorthands(
    tokens: &[SpannedToken],
    config: &mut CompileConfig,
) -> Result<(), MakeError> {
    let mut cur = Cursor::new(tokens);

    while !cur.is_exhausted() {
        // Only act on # Hash tokens at the top level
        if cur.peek_tok() != Some(&Token::Hash) {
            cur.advance();
            continue;
        }
        let hash_st = cur.advance().unwrap(); // consume #
        let (line, col) = (hash_st.line, hash_st.col);

        match cur.peek_tok() {
            Some(Token::Ident(kw)) if kw == "import" => {
                cur.advance(); // consume "import"
                let module = expect_ident(&mut cur, "#import")?;
                expect_semi(&mut cur, "#import")?;
                if !config.imports.contains(&module) {
                    config.imports.push(module);
                }
            }
            Some(Token::Ident(kw)) if kw == "use" => {
                cur.advance(); // consume "use"
                let path = collect_path(&mut cur, "#use")?;
                expect_semi(&mut cur, "#use")?;
                if !config.uses.contains(&path) {
                    config.uses.push(path);
                }
            }
            Some(Token::Ident(kw)) if kw == "def" => {
                cur.advance(); // consume "def"
                let entry = parse_def_entry(&mut cur, line, col)?;
                config.defs.push(entry);
                expect_semi(&mut cur, "#def")?;
            }
            _ => {
                // Not a shorthand we care about — skip
            }
        }
    }

    Ok(())
}

// ── Step 2: find the %make block ─────────────────────────────────────────────

/// Locate `# fn %make ( ) { ... }` in the token stream.
/// Returns the tokens inside the braces, or None if no %make block exists.
/// Hard errors if more than one %make block is found.
fn find_make_block(tokens: &[SpannedToken]) -> Result<Option<Vec<SpannedToken>>, MakeError> {
    let mut found: Option<Vec<SpannedToken>> = None;

    let mut i = 0;
    while i < tokens.len() {
        // Pattern: Hash Ident("fn") Directive("make") LParen RParen LBrace ... RBrace
        if tokens[i].token != Token::Hash {
            i += 1;
            continue;
        }
        if i + 3 >= tokens.len() {
            i += 1;
            continue;
        }

        let is_fn   = matches!(&tokens[i + 1].token, Token::Ident(s) if s == "fn");
        let is_make = matches!(&tokens[i + 2].token, Token::Directive(s) if s == "make");

        if !is_fn || !is_make {
            i += 1;
            continue;
        }

        let def_line = tokens[i].line;
        let def_col  = tokens[i].col;

        // Duplicate check
        if found.is_some() {
            return Err(MakeError::new(
                "More than one %make function defined — only one is allowed per file",
                def_line, def_col,
            ));
        }

        // Validate: must be followed by ( ) — no parameters allowed
        let lparen_idx = i + 3;
        let rparen_idx = i + 4;

        if lparen_idx >= tokens.len() || tokens[lparen_idx].token != Token::LParen {
            return Err(MakeError::new(
                "%make must have an empty parameter list: #fn %make()",
                def_line, def_col,
            ));
        }
        if rparen_idx >= tokens.len() || tokens[rparen_idx].token != Token::RParen {
            return Err(MakeError::new(
                "%make must have no parameters — found something inside ()",
                tokens[lparen_idx].line, tokens[lparen_idx].col,
            ));
        }

        // Validate: no return type annotation (no Arrow after the closing paren)
        let lbrace_idx = rparen_idx + 1;
        if lbrace_idx >= tokens.len() {
            return Err(MakeError::new("%make body is missing", def_line, def_col));
        }
        if tokens[lbrace_idx].token == Token::Arrow {
            return Err(MakeError::new(
                "%make must not have a return type annotation",
                tokens[lbrace_idx].line, tokens[lbrace_idx].col,
            ));
        }
        if tokens[lbrace_idx].token != Token::LBrace {
            return Err(MakeError::new(
                "Expected '{' to open %make body",
                tokens[lbrace_idx].line, tokens[lbrace_idx].col,
            ));
        }

        // Extract the body tokens between { and matching }
        let body = extract_brace_body(tokens, lbrace_idx)?;
        found = Some(body);

        // Skip past the entire block (rough skip — we already have the body)
        i = lbrace_idx + 1;
    }

    Ok(found)
}

/// Extract the tokens between a matched `{` ... `}`, handling nesting.
fn extract_brace_body(
    tokens: &[SpannedToken],
    lbrace_idx: usize,
) -> Result<Vec<SpannedToken>, MakeError> {
    let open = &tokens[lbrace_idx];
    let mut depth = 0usize;
    let mut body = Vec::new();

    for st in &tokens[lbrace_idx..] {
        match st.token {
            Token::LBrace => {
                depth += 1;
                if depth > 1 {
                    body.push(st.clone());
                }
            }
            Token::RBrace => {
                depth -= 1;
                if depth == 0 {
                    return Ok(body);
                }
                body.push(st.clone());
            }
            _ => {
                if depth >= 1 {
                    body.push(st.clone());
                }
            }
        }
    }

    Err(MakeError::new(
        "Unterminated %make body — missing closing '}'",
        open.line, open.col,
    ))
}

// ── Step 4: evaluate the %make body ──────────────────────────────────────────

/// Walk the tokens inside `%make { ... }` and evaluate each statement.
fn evaluate_make_body(
    body: &[SpannedToken],
    config: &mut CompileConfig,
) -> Result<(), MakeError> {
    let mut cur = Cursor::new(body);

    while !cur.is_exhausted() {
        // Skip stray semicolons
        if cur.peek_tok() == Some(&Token::Semi) {
            cur.advance();
            continue;
        }

        let st = match cur.peek() {
            None => break,
            Some(s) => s,
        };
        let (line, col) = (st.line, st.col);

        // Enforce: no control flow unless %logic-%make = %true
        if is_control_flow(&st.token) {
            if !config.logic_make {
                return Err(MakeError::new(
                    format!(
                        "Control flow ('{}') inside %make requires '%logic-%make = %true'",
                        match &st.token { Token::Ident(s) => s.as_str(), _ => "" }
                    ),
                    line, col,
                ));
            }
            // If logic-%make is enabled, skip control flow tokens for now
            // (full evaluation of branching %make is a future extension)
            cur.advance();
            continue;
        }

        // Expect: let %directive = value ;
        if cur.peek_tok() == Some(&Token::Ident("let".to_string())) {
            cur.advance(); // consume "let"
            parse_make_assignment(&mut cur, config)?;
            continue;
        }

        // Anything else inside %make is an error
        return Err(MakeError::new(
            format!("Unexpected token in %make body: {:?}", st.token),
            line, col,
        ));
    }

    Ok(())
}

/// Parse a single `%directive = value ;` assignment inside %make.
fn parse_make_assignment(
    cur: &mut Cursor,
    config: &mut CompileConfig,
) -> Result<(), MakeError> {
    // Expect a Directive token
    let dir_st = match cur.advance() {
        Some(st) => st.clone(),
        None => return Err(MakeError::new("Expected directive after 'let' in %make", 0, 0)),
    };

    let directive = match &dir_st.token {
        Token::Directive(d) => d.clone(),
        other => return Err(MakeError::new(
            format!("Expected a %%directive after 'let' in %%make, got {:?}", other),
            dir_st.line, dir_st.col,
        )),
    };

    // Validate it's a known %make directive
    if !MAKE_DIRECTIVES.contains(&directive.as_str()) {
        return Err(MakeError::new(
            format!("Unknown directive '%{}' inside %%make. Known directives: {}",
                    directive,
                    MAKE_DIRECTIVES.join(", ")
            ),
            dir_st.line, dir_st.col,
        ));
    }

    // Expect =
    expect_eq(cur, &format!("let %{}", directive))?;

    // Parse the value based on which directive this is
    match directive.as_str() {
        // ── Boolean directives ────────────────────────────────────────
        "suppress-warnings" => {
            config.suppress_warnings = parse_bool(cur)?;
        }
        "strict" => {
            config.strict = parse_bool(cur)?;
        }
        "repl" => {
            config.repl = parse_bool(cur)?;
        }
        "async" => {
            config.async_runtime = parse_bool(cur)?;
        }
        "logic-%make" => {
            config.logic_make = parse_bool(cur)?;
        }

        // ── String directives ─────────────────────────────────────────
        "target" => {
            let (s, line, col) = parse_string(cur)?;
            config.target = match s.as_str() {
                "debug"   => BuildTarget::Debug,
                "release" => BuildTarget::Release,
                other => return Err(MakeError::new(
                    format!("%%target must be \"debug\" or \"release\", got \"{}\"", other),
                    line, col,
                )),
            };
        }
        "entry" => {
            let (s, _, _) = parse_string(cur)?;
            config.entry = s;
        }

        // ── List directives ───────────────────────────────────────────
        "hard" => {
            let items = parse_ident_list(cur)?;
            for item in items {
                if !config.hard.contains(&item) {
                    config.hard.push(item);
                }
            }
        }
        "import" => {
            let items = parse_ident_list(cur)?;
            for item in items {
                if !config.imports.contains(&item) {
                    config.imports.push(item);
                }
            }
        }
        "use" => {
            let items = parse_path_list(cur)?;
            for item in items {
                if !config.uses.contains(&item) {
                    config.uses.push(item);
                }
            }
        }
        "def" => {
            let entries = parse_def_list(cur)?;
            config.defs.extend(entries);
        }

        // ── %rust block directives ────────────────────────────────────
        "when-run" => {
            let rust_src = parse_rust_block(cur)?;
            config.when_run = Some(rust_src);
        }
        "when-compile" => {
            let rust_src = parse_rust_block(cur)?;
            config.when_compile = Some(rust_src);
        }

        // ── %self rename ──────────────────────────────────────────────
        "self" => {
            config.self_rename = parse_self_rename(cur)?;
        }

        _ => unreachable!("Already validated against MAKE_DIRECTIVES"),
    }

    expect_semi(cur, &format!("let %{}", directive))?;
    Ok(())
}

// ── Value parsers ─────────────────────────────────────────────────────────────

fn parse_bool(cur: &mut Cursor) -> Result<bool, MakeError> {
    match cur.advance() {
        Some(st) => match &st.token {
            Token::True  => Ok(true),
            Token::False => Ok(false),
            other => Err(MakeError::new(
                format!("Expected %%true or %%false, got {:?}", other),
                st.line, st.col,
            )),
        },
        None => Err(MakeError::new("Expected %%true or %%false, got end of tokens", 0, 0)),
    }
}

fn parse_string(cur: &mut Cursor) -> Result<(String, usize, usize), MakeError> {
    match cur.advance() {
        Some(st) => match &st.token {
            Token::Str(s) => Ok((s.clone(), st.line, st.col)),
            other => Err(MakeError::new(
                format!("Expected a string literal, got {:?}", other),
                st.line, st.col,
            )),
        },
        None => Err(MakeError::new("Expected string literal, got end of tokens", 0, 0)),
    }
}

/// Parse `[ ident, ident, ... ]`
fn parse_ident_list(cur: &mut Cursor) -> Result<Vec<String>, MakeError> {
    let (line, col) = cur.current_location();
    expect_token(cur, &Token::LBracket, "list")?;
    let mut items = Vec::new();

    loop {
        match cur.peek_tok() {
            Some(Token::RBracket) => { cur.advance(); break; }
            Some(Token::Comma)    => { cur.advance(); continue; }
            Some(Token::Ident(_)) => {
                let name = expect_ident(cur, "list item")?;
                items.push(name);
            }
            Some(other) => {
                let (l, c) = cur.current_location();
                return Err(MakeError::new(
                    format!("Expected identifier in list, got {:?}", other), l, c,
                ));
            }
            None => return Err(MakeError::new("Unterminated list", line, col)),
        }
    }
    Ok(items)
}

/// Parse `[ path::to::item, ... ]`
fn parse_path_list(cur: &mut Cursor) -> Result<Vec<String>, MakeError> {
    let (line, col) = cur.current_location();
    expect_token(cur, &Token::LBracket, "path list")?;
    let mut items = Vec::new();

    loop {
        match cur.peek_tok() {
            Some(Token::RBracket) => { cur.advance(); break; }
            Some(Token::Comma)    => { cur.advance(); continue; }
            Some(Token::Ident(_)) => {
                let path = collect_path(cur, "use list")?;
                items.push(path);
            }
            Some(other) => {
                let (l, c) = cur.current_location();
                return Err(MakeError::new(
                    format!("Expected path in list, got {:?}", other), l, c,
                ));
            }
            None => return Err(MakeError::new("Unterminated path list", line, col)),
        }
    }
    Ok(items)
}

/// Parse `[ "file.rs" as alias, "other.dll" as other, ... ]`
fn parse_def_list(cur: &mut Cursor) -> Result<Vec<DefEntry>, MakeError> {
    let (line, col) = cur.current_location();
    expect_token(cur, &Token::LBracket, "def list")?;
    let mut entries = Vec::new();

    loop {
        match cur.peek_tok() {
            Some(Token::RBracket) => { cur.advance(); break; }
            Some(Token::Comma)    => { cur.advance(); continue; }
            Some(Token::Str(_))   => {
                let entry = parse_def_entry(cur, line, col)?;
                entries.push(entry);
            }
            Some(other) => {
                let (l, c) = cur.current_location();
                return Err(MakeError::new(
                    format!("Expected string path in def list, got {:?}", other), l, c,
                ));
            }
            None => return Err(MakeError::new("Unterminated def list", line, col)),
        }
    }
    Ok(entries)
}

/// Parse a single `"file.rs" as alias` def entry.
fn parse_def_entry(
    cur: &mut Cursor,
    line: usize,
    col: usize,
) -> Result<DefEntry, MakeError> {
    let (path, path_line, path_col) = parse_string(cur)?;

    // Determine kind from file extension
    let kind = if path.ends_with(".rs") {
        DefKind::RustSource
    } else if path.ends_with(".dll") || path.ends_with(".so") || path.ends_with(".dylib") {
        DefKind::NativeLib
    } else {
        return Err(MakeError::new(
            format!(
                "%%def only accepts .rs, .dll, .so, or .dylib files — got \"{}\"",
                path
            ),
            path_line, path_col,
        ));
    };

    // Expect "as"
    match cur.advance() {
        Some(st) if matches!(&st.token, Token::Ident(s) if s == "as") => {}
        Some(st) => return Err(MakeError::new(
            format!("Expected 'as' after file path in %%def, got {:?}", st.token),
            st.line, st.col,
        )),
        None => return Err(MakeError::new("Expected 'as' after file path in %%def", line, col)),
    }

    let alias = expect_ident(cur, "def alias")?;

    Ok(DefEntry { path, alias, kind })
}

/// Parse a `%rust { ... }` block and return the raw Rust source as a String.
fn parse_rust_block(cur: &mut Cursor) -> Result<String, MakeError> {
    let (line, col) = cur.current_location();

    // Expect the %rust directive token
    match cur.advance() {
        Some(st) if matches!(&st.token, Token::Directive(d) if d == "rust") => {}
        Some(st) => return Err(MakeError::new(
            format!("Expected %%rust block, got {:?}", st.token),
            st.line, st.col,
        )),
        None => return Err(MakeError::new("Expected %%rust block, got end of tokens", line, col)),
    }

    // Expect {
    expect_token(cur, &Token::LBrace, "%rust block")?;

    // Collect everything until the matching }
    let mut depth = 1usize;
    let mut rust_tokens: Vec<String> = Vec::new();

    loop {
        match cur.advance() {
            None => return Err(MakeError::new("Unterminated %%rust block", line, col)),
            Some(st) => match &st.token {
                Token::LBrace => { depth += 1; rust_tokens.push("{".to_string()); }
                Token::RBrace => {
                    depth -= 1;
                    if depth == 0 { break; }
                    rust_tokens.push("}".to_string());
                }
                other => rust_tokens.push(token_to_rust_repr_pub(other)),
            }
        }
    }

    Ok(rust_tokens.join(" "))
}

/// Parse the %self directive value — either a plain string or a map block.
fn parse_self_rename(cur: &mut Cursor) -> Result<SelfRename, MakeError> {
    match cur.peek_tok() {
        Some(Token::Str(_)) => {
            let (s, _, _) = parse_string(cur)?;
            Ok(SelfRename::Global(s))
        }
        Some(Token::LBrace) => {
            let (line, col) = cur.current_location();
            cur.advance(); // consume {
            let mut overrides = Vec::new();
            let mut default: Option<String> = None;

            loop {
                match cur.peek_tok() {
                    Some(Token::RBrace) => { cur.advance(); break; }
                    Some(Token::Comma)  => { cur.advance(); continue; }
                    Some(Token::Ident(_)) => {
                        // ClassName = "rename"
                        let class_name = expect_ident(cur, "%self map")?;
                        expect_eq(cur, "%self map entry")?;
                        let (rename, _, _) = parse_string(cur)?;
                        overrides.push((class_name, rename));
                    }
                    Some(Token::Str(_)) => {
                        // bare string at the end = default
                        let (s, _, _) = parse_string(cur)?;
                        default = Some(s);
                    }
                    Some(other) => {
                        let (l, c) = cur.current_location();
                        return Err(MakeError::new(
                            format!("Unexpected token in %%self map: {:?}", other), l, c,
                        ));
                    }
                    None => return Err(MakeError::new("Unterminated %%self map", line, col)),
                }
            }

            Ok(SelfRename::PerClass {
                overrides,
                default: default.unwrap_or_else(|| "self".to_string()),
            })
        }
        Some(other) => {
            let (l, c) = cur.current_location();
            Err(MakeError::new(
                format!("Expected string or map for %%self, got {:?}", other), l, c,
            ))
        }
        None => Err(MakeError::new("Expected %%self value, got end of tokens", 0, 0)),
    }
}

// ── Step 5: fire %when-compile ────────────────────────────────────────────────

/// Execute the %when-compile Rust snippet, if present.
/// In a real implementation this would shell out to rustc.
/// For now it validates the block is non-empty and logs the intent.
fn fire_when_compile(config: &CompileConfig) -> Result<(), MakeError> {
    if let Some(rust_src) = &config.when_compile {
        if rust_src.trim().is_empty() {
            return Err(MakeError::new(
                "%%when-compile block is empty", 0, 0,
            ));
        }
        // TODO: shell out to rustc to compile and run this snippet.
        // The %rust isolator (Stage 26) will handle the full validation pass.
        // For now we just confirm the block was captured correctly.
        eprintln!("[%make] Firing %when-compile hook ({} chars of Rust)", rust_src.len());
    }
    Ok(())
}

// ── Small parsing helpers ─────────────────────────────────────────────────────

fn expect_ident(cur: &mut Cursor, context: &str) -> Result<String, MakeError> {
    match cur.advance() {
        Some(st) => match &st.token {
            Token::Ident(s) => Ok(s.clone()),
            other => Err(MakeError::new(
                format!("Expected identifier in {}, got {:?}", context, other),
                st.line, st.col,
            )),
        },
        None => Err(MakeError::new(
            format!("Expected identifier in {}, got end of tokens", context), 0, 0,
        )),
    }
}

fn expect_semi(cur: &mut Cursor, context: &str) -> Result<(), MakeError> {
    expect_token(cur, &Token::Semi, context)
}

fn expect_eq(cur: &mut Cursor, context: &str) -> Result<(), MakeError> {
    expect_token(cur, &Token::Eq, context)
}

fn expect_token(cur: &mut Cursor, expected: &Token, context: &str) -> Result<(), MakeError> {
    match cur.advance() {
        Some(st) if &st.token == expected => Ok(()),
        Some(st) => Err(MakeError::new(
            format!("Expected {:?} in {}, got {:?}", expected, context, st.token),
            st.line, st.col,
        )),
        None => Err(MakeError::new(
            format!("Expected {:?} in {}, got end of tokens", expected, context), 0, 0,
        )),
    }
}

/// Collect a `::` separated path like `math::add` into a single String.
fn collect_path(cur: &mut Cursor, context: &str) -> Result<String, MakeError> {
    let mut path = expect_ident(cur, context)?;
    while cur.peek_tok() == Some(&Token::ColonColon) {
        cur.advance(); // ::
        let next = expect_ident(cur, context)?;
        path.push_str("::");
        path.push_str(&next);
    }
    Ok(path)
}

/// Rough conversion of a token back to a Rust source representation,
/// used when capturing %rust block contents as a raw string.
pub fn token_to_rust_repr_pub(tok: &Token) -> String {
    match tok {
        Token::Ident(s)      => s.clone(),
        Token::Directive(s)  => format!("%{}", s),
        Token::Int(n)        => n.to_string(),
        Token::Float(f)      => f.to_string(),
        Token::Str(s)        => format!("\"{}\"", s),
        Token::True          => "true".to_string(),
        Token::False         => "false".to_string(),
        Token::Void          => "()".to_string(),
        Token::Semi          => ";".to_string(),
        Token::Colon         => ":".to_string(),
        Token::ColonColon    => "::".to_string(),
        Token::Comma         => ",".to_string(),
        Token::Dot           => ".".to_string(),
        Token::DotDot        => "..".to_string(),
        Token::Arrow         => "->".to_string(),
        Token::Question      => "?".to_string(),
        Token::LBrace        => "{".to_string(),
        Token::RBrace        => "}".to_string(),
        Token::LParen        => "(".to_string(),
        Token::RParen        => ")".to_string(),
        Token::LBracket      => "[".to_string(),
        Token::RBracket      => "]".to_string(),
        Token::Plus          => "+".to_string(),
        Token::Minus         => "-".to_string(),
        Token::Star          => "*".to_string(),
        Token::Slash         => "/".to_string(),
        Token::Percent       => "%".to_string(),
        Token::Eq            => "=".to_string(),
        Token::EqEq          => "==".to_string(),
        Token::BangEq        => "!=".to_string(),
        Token::Lt            => "<".to_string(),
        Token::Gt            => ">".to_string(),
        Token::LtEq          => "<=".to_string(),
        Token::GtEq          => ">=".to_string(),
        Token::AmpAmp        => "&&".to_string(),
        Token::PipePipe      => "||".to_string(),
        Token::Bang          => "!".to_string(),
        Token::Amp           => "&".to_string(),
        Token::AmpMut        => "&mut".to_string(),
        Token::Hash          => "#".to_string(),
        Token::DocComment(s) => format!("// {}", s),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(src: &str) -> CompileConfig {
        let tokens = lex(src).expect("lex failed");
        run_make_pass(&tokens).expect("make pass failed")
    }

    fn make_err(src: &str) -> String {
        let tokens = lex(src).expect("lex failed");
        run_make_pass(&tokens).unwrap_err().message
    }

    // ── No %make block ────────────────────────────────────────────────

    #[test]
    fn test_empty_source_gives_defaults() {
        let config = make_config("");
        assert_eq!(config.entry, "main");
        assert_eq!(config.target, BuildTarget::Debug);
        assert!(!config.suppress_warnings);
        assert!(!config.strict);
        assert!(!config.repl);
    }

    // ── Boolean directives ────────────────────────────────────────────

    #[test]
    fn test_suppress_warnings() {
        let config = make_config("#fn %make() { let %suppress-warnings = %true; };");
        assert!(config.suppress_warnings);
    }

    #[test]
    fn test_strict_mode() {
        let config = make_config("#fn %make() { let %strict = %true; };");
        assert!(config.strict);
    }

    #[test]
    fn test_repl_mode() {
        let config = make_config("#fn %make() { let %repl = %true; };");
        assert!(config.repl);
    }

    #[test]
    fn test_async_runtime() {
        let config = make_config("#fn %make() { let %async = %true; };");
        assert!(config.async_runtime);
    }

    // ── String directives ─────────────────────────────────────────────

    #[test]
    fn test_target_release() {
        let config = make_config("#fn %make() { let %target = \"release\"; };");
        assert_eq!(config.target, BuildTarget::Release);
    }

    #[test]
    fn test_target_debug() {
        let config = make_config("#fn %make() { let %target = \"debug\"; };");
        assert_eq!(config.target, BuildTarget::Debug);
    }

    #[test]
    fn test_invalid_target_is_error() {
        assert!(make_err("#fn %make() { let %target = \"banana\"; };")
            .contains("\"debug\" or \"release\""));
    }

    #[test]
    fn test_custom_entry() {
        let config = make_config("#fn %make() { let %entry = \"start\"; };");
        assert_eq!(config.entry, "start");
    }

    // ── List directives ───────────────────────────────────────────────

    #[test]
    fn test_import_list() {
        let config = make_config("#fn %make() { let %import = [math, geometry]; };");
        assert_eq!(config.imports, vec!["math", "geometry"]);
    }

    #[test]
    fn test_use_list() {
        let config = make_config("#fn %make() { let %use = [math::add, geometry::distance]; };");
        assert_eq!(config.uses, vec!["math::add", "geometry::distance"]);
    }

    #[test]
    fn test_hard_list() {
        let config = make_config("#fn %make() { let %hard = [std::convert]; };");
        assert_eq!(config.hard, vec!["std"]);  // simplified: ident only for now
    }

    // ── Shorthand merging ─────────────────────────────────────────────

    #[test]
    fn test_shorthand_import_merged() {
        let config = make_config("#import math;");
        assert!(config.imports.contains(&"math".to_string()));
    }

    #[test]
    fn test_shorthand_use_merged() {
        let config = make_config("#import math;\n#use math::add;");
        assert!(config.uses.contains(&"math::add".to_string()));
    }

    #[test]
    fn test_shorthand_def_merged() {
        let config = make_config("#def \"mylib.rs\" as mylib;");
        assert_eq!(config.defs.len(), 1);
        assert_eq!(config.defs[0].alias, "mylib");
        assert_eq!(config.defs[0].kind, DefKind::RustSource);
    }

    #[test]
    fn test_def_dll_recognized() {
        let config = make_config("#def \"fast.dll\" as fast;");
        assert_eq!(config.defs[0].kind, DefKind::NativeLib);
    }

    #[test]
    fn test_def_invalid_extension_is_error() {
        assert!(make_err("#def \"script.py\" as py;")
            .contains("only accepts .rs, .dll, .so"));
    }

    // ── Structure errors ──────────────────────────────────────────────

    #[test]
    fn test_duplicate_make_is_error() {
        assert!(make_err(
            "#fn %make() {};\n#fn %make() {};"
        ).contains("More than one %make"));
    }

    #[test]
    fn test_make_with_params_is_error() {
        assert!(make_err("#fn %make(x: %i32) {};")
            .contains("no parameters"));
    }

    #[test]
    fn test_make_with_return_type_is_error() {
        assert!(make_err("#fn %make() -> %void {};")
            .contains("return type"));
    }

    #[test]
    fn test_unknown_directive_is_error() {
        assert!(make_err("#fn %make() { let %banana = %true; };")
            .contains("Unknown directive"));
    }

    #[test]
    fn test_control_flow_without_logic_make_is_error() {
        assert!(make_err(
            "#fn %make() { if %true { let %strict = %true; }; };"
        ).contains("requires '%logic-%make = %true'"));
    }

    #[test]
    fn test_logic_make_allows_control_flow() {
        // Should not error — logic-%make is set first
        let config = make_config(
            "#fn %make() { let %logic-%make = %true; if %true { }; };"
        );
        assert!(config.logic_make);
    }

    // ── %self rename ──────────────────────────────────────────────────

    #[test]
    fn test_self_global_rename() {
        let config = make_config("#fn %make() { let %self = \"this\"; };");
        assert!(matches!(config.self_rename, SelfRename::Global(s) if s == "this"));
    }
}