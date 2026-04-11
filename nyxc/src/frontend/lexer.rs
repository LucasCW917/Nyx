// Stage 1 — Lexer
// Turns raw .nyx source text into a flat stream of tokens.
// Reads character by character, handles whitespace-sensitive operator
// disambiguation, recognizes %, #, ! prefixes, and tracks source locations.

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Literals
    Ident(String),        // regular identifier or keyword
    Directive(String),    // %something — % stripped, name kept
    Int(i64),             // 42
    Float(f64),           // 3.14
    Str(String),          // "hello"
    DocComment(String),   // /// doc comment

    // Special literals
    True,                 // %true
    False,                // %false
    Void,                 // %void

    // Top-level marker
    Hash,                 // # prefix

    // Punctuation
    Semi,                 // ;
    Colon,                // :
    ColonColon,           // ::
    Comma,                // ,
    Dot,                  // .
    DotDot,               // ..
    Arrow,                // ->
    Question,             // ?

    // Brackets
    LBrace,               // {
    RBrace,               // }
    LParen,               // (
    RParen,               // )
    LBracket,             // [
    RBracket,             // ]

    // Operators (only valid with spaces on both sides)
    Plus,                 // +
    Minus,                // -
    Star,                 // *
    Slash,                // /
    Percent,              // %  (as operator, not directive prefix)

    // Comparison
    Eq,                   // =
    EqEq,                 // ==
    BangEq,               // !=
    Lt,                   // <
    Gt,                   // >
    LtEq,                 // <=
    GtEq,                 // >=

    // Logical
    AmpAmp,               // &&
    PipePipe,             // ||
    Bang,                 // !

    // Borrow
    Amp,                  // &
    AmpMut,               // &mut
}

/// A token with its source location attached.
#[derive(Debug, Clone)]
pub struct SpannedToken {
    pub token: Token,
    pub line: usize,
    pub col: usize,
}

/// A hard lexer error — compilation halts immediately.
#[derive(Debug)]
pub struct LexError {
    pub message: String,
    pub line: usize,
    pub col: usize,
}

impl std::fmt::Display for LexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[Lexer Error] {}:{} — {}", self.line, self.col, self.message)
    }
}

/// The lexer state machine.
pub struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Lexer {
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    // ── Cursor helpers ────────────────────────────────────────────────

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn peek_prev(&self) -> Option<char> {
        if self.pos == 0 { None } else { self.chars.get(self.pos - 1).copied() }
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.chars.get(self.pos).copied();
        if let Some(c) = ch {
            self.pos += 1;
            if c == '\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
        }
        ch
    }

    fn err(&self, msg: impl Into<String>) -> LexError {
        LexError { message: msg.into(), line: self.line, col: self.col }
    }

    fn spanned(&self, token: Token, line: usize, col: usize) -> SpannedToken {
        SpannedToken { token, line, col }
    }

    // ── Whitespace & comments ─────────────────────────────────────────

    /// Skip whitespace (but NOT newlines — they matter for disambiguation).
    /// Returns true if at least one space/tab was consumed.
    fn skip_whitespace(&mut self) -> bool {
        let mut skipped = false;
        while matches!(self.peek(), Some(' ') | Some('\t') | Some('\r') | Some('\n')) {
            self.advance();
            skipped = true;
        }
        skipped
    }

    /// Skip a `// ...` line comment. Returns the content if it was a `///` doc comment.
    fn skip_line_comment(&mut self) -> Option<String> {
        // consume both slashes
        self.advance(); // /
        self.advance(); // /
        let is_doc = self.peek() == Some('/');
        if is_doc { self.advance(); } // third /

        let mut content = String::new();
        while !matches!(self.peek(), Some('\n') | None) {
            content.push(self.advance().unwrap());
        }
        if is_doc { Some(content.trim().to_string()) } else { None }
    }

    /// Skip a `/* ... */` block comment. Hard error if unterminated.
    fn skip_block_comment(&mut self) -> Result<(), LexError> {
        let (start_line, start_col) = (self.line, self.col);
        self.advance(); // /
        self.advance(); // *
        loop {
            match self.peek() {
                None => return Err(LexError {
                    message: "Unterminated block comment /* ... */".to_string(),
                    line: start_line,
                    col: start_col,
                }),
                Some('*') if self.peek_next() == Some('/') => {
                    self.advance(); // *
                    self.advance(); // /
                    return Ok(());
                }
                _ => { self.advance(); }
            }
        }
    }

    // ── Literals ──────────────────────────────────────────────────────

    /// Lex a string literal `"..."`. Hard error if unterminated.
    fn lex_string(&mut self) -> Result<String, LexError> {
        let (start_line, start_col) = (self.line, self.col);
        self.advance(); // opening "
        let mut s = String::new();
        loop {
            match self.peek() {
                None | Some('\n') => return Err(LexError {
                    message: "Unterminated string literal".to_string(),
                    line: start_line,
                    col: start_col,
                }),
                Some('"') => { self.advance(); return Ok(s); }
                Some('\\') => {
                    self.advance();
                    match self.advance() {
                        Some('n')  => s.push('\n'),
                        Some('t')  => s.push('\t'),
                        Some('r')  => s.push('\r'),
                        Some('"')  => s.push('"'),
                        Some('\\') => s.push('\\'),
                        Some(c) => return Err(self.err(format!("Unknown escape sequence \\{}", c))),
                        None => return Err(self.err("Unterminated string escape")),
                    }
                }
                Some(c) => { s.push(c); self.advance(); }
            }
        }
    }

    /// Lex a numeric literal (integer or float).
    fn lex_number(&mut self) -> Token {
        let mut s = String::new();
        let mut is_float = false;

        while matches!(self.peek(), Some('0'..='9')) {
            s.push(self.advance().unwrap());
        }

        // Check for decimal point followed by a digit (not `..`)
        if self.peek() == Some('.') && matches!(self.peek_next(), Some('0'..='9')) {
            is_float = true;
            s.push(self.advance().unwrap()); // .
            while matches!(self.peek(), Some('0'..='9')) {
                s.push(self.advance().unwrap());
            }
        }

        if is_float {
            Token::Float(s.parse().unwrap())
        } else {
            Token::Int(s.parse().unwrap())
        }
    }

    // ── Identifier / directive / keyword ─────────────────────────────

    /// Lex a run of identifier characters.
    /// Nyx identifiers are very permissive — most non-whitespace,
    /// non-delimiter characters are valid in an identifier name as long as
    /// they are not surrounded by spaces (which would make them operators).
    fn lex_ident_chars(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if Self::is_ident_char(c) {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        s
    }

    /// Characters that can appear in a Nyx identifier (after disambiguation).
    /// Excludes whitespace, delimiters, and quote characters.
    fn is_ident_char(c: char) -> bool {
        !matches!(c,
            ' ' | '\t' | '\n' | '\r' |
            '{' | '}' | '(' | ')' | '[' | ']' |
            '"' | '\'' |
            ';' | ','
        )
    }

    // ── Operator disambiguation ───────────────────────────────────────

    /// Determines whether the character at the current position should be
    /// treated as an operator or as part of an identifier, based on the
    /// whitespace context around it.
    ///
    /// Rules:
    ///   - preceded by space AND followed by space → operator
    ///   - preceded by non-space AND followed by non-space → part of identifier
    ///   - one side has space, other does not → hard error (ambiguous)
    ///
    /// "Preceded by space" means the character immediately before pos is a space/tab.
    /// "Followed by space" means the character immediately after pos is a space/tab/newline/EOF.
    fn operator_context(&self) -> OperatorContext {
        let before = self.peek_prev();
        let after = self.peek_next();

        let space_before = matches!(before, Some(' ') | Some('\t') | Some('\n') | Some('\r') | None);
        let space_after = matches!(after, Some(' ') | Some('\t') | Some('\n') | Some('\r') | None);

        match (space_before, space_after) {
            (true, true)   => OperatorContext::Operator,
            (false, false) => OperatorContext::Identifier,
            _              => OperatorContext::Ambiguous,
        }
    }

    // ── Main tokenization ─────────────────────────────────────────────

    /// Tokenize the entire source string into a Vec of SpannedTokens.
    pub fn tokenize(&mut self) -> Result<Vec<SpannedToken>, LexError> {
        let mut tokens: Vec<SpannedToken> = Vec::new();

        loop {
            self.skip_whitespace();

            let (line, col) = (self.line, self.col);

            let ch = match self.peek() {
                None => break,
                Some(c) => c,
            };

            // ── Comments ──────────────────────────────────────────────
            if ch == '/' {
                match self.peek_next() {
                    Some('/') => {
                        if let Some(doc) = self.skip_line_comment() {
                            tokens.push(self.spanned(Token::DocComment(doc), line, col));
                        }
                        continue;
                    }
                    Some('*') => {
                        self.skip_block_comment()?;
                        continue;
                    }
                    _ => {}
                }
            }

            // ── String literals ───────────────────────────────────────
            if ch == '"' {
                let s = self.lex_string()?;
                tokens.push(self.spanned(Token::Str(s), line, col));
                continue;
            }

            // ── Numeric literals ──────────────────────────────────────
            if ch.is_ascii_digit() {
                let tok = self.lex_number();
                tokens.push(self.spanned(tok, line, col));
                continue;
            }

            // ── # top-level marker ────────────────────────────────────
            if ch == '#' {
                self.advance();
                tokens.push(self.spanned(Token::Hash, line, col));
                continue;
            }

            // ── % directive or % operator ─────────────────────────────
            if ch == '%' {
                // Peek ahead: if the next char is an ident char, this is a directive
                if matches!(self.peek_next(), Some(c) if Self::is_ident_char(c) && c != ' ') {
                    self.advance(); // consume %
                    let name = self.lex_ident_chars();
                    let tok = match name.as_str() {
                        "true"  => Token::True,
                        "false" => Token::False,
                        "void"  => Token::Void,
                        _       => Token::Directive(name),
                    };
                    tokens.push(self.spanned(tok, line, col));
                } else {
                    // % as arithmetic operator — requires spaces on both sides
                    match self.operator_context() {
                        OperatorContext::Operator => {
                            self.advance();
                            tokens.push(self.spanned(Token::Percent, line, col));
                        }
                        OperatorContext::Identifier => {
                            // bare % with no following ident and no spaces — malformed
                            return Err(self.err("Bare '%' with no directive name and no operator spacing"));
                        }
                        OperatorContext::Ambiguous => {
                            return Err(self.err(
                                "Ambiguous '%' — use spaces on both sides for operator, or no spaces for directive"
                            ));
                        }
                    }
                }
                continue;
            }

            // ── & borrow / && logical ─────────────────────────────────
            if ch == '&' {
                self.advance();
                if self.peek() == Some('&') {
                    self.advance();
                    tokens.push(self.spanned(Token::AmpAmp, line, col));
                } else if self.peek() == Some('m') {
                    // peek ahead for "mut"
                    let rest: String = self.chars[self.pos..].iter()
                        .take(3).collect();
                    if rest == "mut" && !matches!(self.chars.get(self.pos + 3), Some(c) if Self::is_ident_char(*c)) {
                        self.advance(); self.advance(); self.advance(); // m u t
                        tokens.push(self.spanned(Token::AmpMut, line, col));
                    } else {
                        tokens.push(self.spanned(Token::Amp, line, col));
                    }
                } else {
                    tokens.push(self.spanned(Token::Amp, line, col));
                }
                continue;
            }

            // ── | → || ───────────────────────────────────────────────
            if ch == '|' {
                self.advance();
                if self.peek() == Some('|') {
                    self.advance();
                    tokens.push(self.spanned(Token::PipePipe, line, col));
                } else {
                    return Err(self.err("Unexpected '|' — did you mean '||'?"));
                }
                continue;
            }

            // ── ! or != ───────────────────────────────────────────────
            if ch == '!' {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    tokens.push(self.spanned(Token::BangEq, line, col));
                } else {
                    tokens.push(self.spanned(Token::Bang, line, col));
                }
                continue;
            }

            // ── = or == ───────────────────────────────────────────────
            if ch == '=' {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    tokens.push(self.spanned(Token::EqEq, line, col));
                } else {
                    tokens.push(self.spanned(Token::Eq, line, col));
                }
                continue;
            }

            // ── < or <= ───────────────────────────────────────────────
            if ch == '<' {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    tokens.push(self.spanned(Token::LtEq, line, col));
                } else {
                    tokens.push(self.spanned(Token::Lt, line, col));
                }
                continue;
            }

            // ── > or >= ───────────────────────────────────────────────
            if ch == '>' {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    tokens.push(self.spanned(Token::GtEq, line, col));
                } else {
                    tokens.push(self.spanned(Token::Gt, line, col));
                }
                continue;
            }

            // ── - or -> ───────────────────────────────────────────────
            if ch == '-' {
                match self.operator_context() {
                    OperatorContext::Operator => {
                        self.advance();
                        if self.peek() == Some('>') {
                            self.advance();
                            tokens.push(self.spanned(Token::Arrow, line, col));
                        } else {
                            tokens.push(self.spanned(Token::Minus, line, col));
                        }
                    }
                    OperatorContext::Identifier => {
                        // part of an identifier — fall through to ident lexing below
                        let name = self.lex_ident_chars();
                        tokens.push(self.spanned(Token::Ident(name), line, col));
                    }
                    OperatorContext::Ambiguous => {
                        return Err(self.err(
                            "Ambiguous '-' — use spaces on both sides for operator (a - b), or no spaces for identifier (a-b)"
                        ));
                    }
                }
                continue;
            }

            // ── + * / (operator only — spaces required) ──────────────
            if matches!(ch, '+' | '*' | '/') {
                match self.operator_context() {
                    OperatorContext::Operator => {
                        self.advance();
                        let tok = match ch {
                            '+' => Token::Plus,
                            '*' => Token::Star,
                            '/' => Token::Slash,
                            _   => unreachable!(),
                        };
                        tokens.push(self.spanned(tok, line, col));
                    }
                    OperatorContext::Identifier => {
                        // Treat as part of an identifier (Nyx allows loose names)
                        let name = self.lex_ident_chars();
                        tokens.push(self.spanned(Token::Ident(name), line, col));
                    }
                    OperatorContext::Ambiguous => {
                        return Err(self.err(format!(
                            "Ambiguous '{}' — use spaces on both sides for operator, or no spaces for identifier", ch
                        )));
                    }
                }
                continue;
            }

            // ── : or :: ───────────────────────────────────────────────
            if ch == ':' {
                self.advance();
                if self.peek() == Some(':') {
                    self.advance();
                    tokens.push(self.spanned(Token::ColonColon, line, col));
                } else {
                    tokens.push(self.spanned(Token::Colon, line, col));
                }
                continue;
            }

            // ── . or .. ───────────────────────────────────────────────
            if ch == '.' {
                self.advance();
                if self.peek() == Some('.') {
                    self.advance();
                    tokens.push(self.spanned(Token::DotDot, line, col));
                } else {
                    tokens.push(self.spanned(Token::Dot, line, col));
                }
                continue;
            }

            // ── Single-character punctuation ──────────────────────────
            match ch {
                ';' => { self.advance(); tokens.push(self.spanned(Token::Semi,     line, col)); continue; }
                ',' => { self.advance(); tokens.push(self.spanned(Token::Comma,    line, col)); continue; }
                '{' => { self.advance(); tokens.push(self.spanned(Token::LBrace,   line, col)); continue; }
                '}' => { self.advance(); tokens.push(self.spanned(Token::RBrace,   line, col)); continue; }
                '(' => { self.advance(); tokens.push(self.spanned(Token::LParen,   line, col)); continue; }
                ')' => { self.advance(); tokens.push(self.spanned(Token::RParen,   line, col)); continue; }
                '[' => { self.advance(); tokens.push(self.spanned(Token::LBracket, line, col)); continue; }
                ']' => { self.advance(); tokens.push(self.spanned(Token::RBracket, line, col)); continue; }
                '?' => { self.advance(); tokens.push(self.spanned(Token::Question, line, col)); continue; }
                _   => {}
            }

            // ── Identifier (catch-all for permissive Nyx names) ───────
            if Self::is_ident_char(ch) {
                let name = self.lex_ident_chars();
                tokens.push(self.spanned(Token::Ident(name), line, col));
                continue;
            }

            // ── Unrecognized character ────────────────────────────────
            return Err(self.err(format!("Unrecognized character '{}'", ch)));
        }

        Ok(tokens)
    }
}

/// The whitespace context around a potential operator character.
#[derive(Debug, PartialEq)]
enum OperatorContext {
    Operator,    // spaces on both sides → treat as operator
    Identifier,  // no spaces → treat as part of identifier
    Ambiguous,   // space on one side only → hard error
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Lex a Nyx source string into a flat token stream.
/// Returns a hard error on the first lexer fault encountered.
pub fn lex(source: &str) -> Result<Vec<SpannedToken>, LexError> {
    Lexer::new(source).tokenize()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(src: &str) -> Vec<Token> {
        lex(src).unwrap().into_iter().map(|s| s.token).collect()
    }

    fn lex_err(src: &str) -> String {
        lex(src).unwrap_err().message
    }

    #[test]
    fn test_identifier() {
        assert_eq!(tokens("hello"), vec![Token::Ident("hello".into())]);
    }

    #[test]
    fn test_hyphenated_identifier() {
        assert_eq!(tokens("my-var"), vec![Token::Ident("my-var".into())]);
    }

    #[test]
    fn test_minus_operator() {
        assert_eq!(tokens("a - b"), vec![
            Token::Ident("a".into()),
            Token::Minus,
            Token::Ident("b".into()),
        ]);
    }

    #[test]
    fn test_ambiguous_minus_is_error() {
        assert!(lex_err("a -b").contains("Ambiguous"));
    }

    #[test]
    fn test_directive() {
        assert_eq!(tokens("%mut"), vec![Token::Directive("mut".into())]);
        assert_eq!(tokens("%pub"), vec![Token::Directive("pub".into())]);
    }

    #[test]
    fn test_true_false_void() {
        assert_eq!(tokens("%true"),  vec![Token::True]);
        assert_eq!(tokens("%false"), vec![Token::False]);
        assert_eq!(tokens("%void"),  vec![Token::Void]);
    }

    #[test]
    fn test_hash() {
        assert_eq!(tokens("#"), vec![Token::Hash]);
    }

    #[test]
    fn test_string_literal() {
        assert_eq!(tokens(r#""hello world""#), vec![Token::Str("hello world".into())]);
    }

    #[test]
    fn test_unterminated_string_is_error() {
        assert!(lex_err(r#""unterminated"#).contains("Unterminated string"));
    }

    #[test]
    fn test_integer() {
        assert_eq!(tokens("42"), vec![Token::Int(42)]);
    }

    #[test]
    fn test_float() {
        assert_eq!(tokens("3.14"), vec![Token::Float(3.14)]);
    }

    #[test]
    fn test_colon_colon() {
        assert_eq!(tokens("math::add"), vec![
            Token::Ident("math".into()),
            Token::ColonColon,
            Token::Ident("add".into()),
        ]);
    }

    #[test]
    fn test_dot_dot_range() {
        assert_eq!(tokens("1..10"), vec![
            Token::Int(1),
            Token::DotDot,
            Token::Int(10),
        ]);
    }

    #[test]
    fn test_arrow() {
        assert_eq!(tokens("-> %i32"), vec![
            Token::Arrow,
            Token::Directive("i32".into()),
        ]);
    }

    #[test]
    fn test_amp_mut() {
        assert_eq!(tokens("&mut x"), vec![
            Token::AmpMut,
            Token::Ident("x".into()),
        ]);
    }

    #[test]
    fn test_line_comment_skipped() {
        assert_eq!(tokens("x // this is a comment\ny"), vec![
            Token::Ident("x".into()),
            Token::Ident("y".into()),
        ]);
    }

    #[test]
    fn test_doc_comment_preserved() {
        let toks = tokens("/// hello doc");
        assert_eq!(toks, vec![Token::DocComment("hello doc".into())]);
    }

    #[test]
    fn test_block_comment_skipped() {
        assert_eq!(tokens("x /* ignored */ y"), vec![
            Token::Ident("x".into()),
            Token::Ident("y".into()),
        ]);
    }

    #[test]
    fn test_unterminated_block_comment_is_error() {
        assert!(lex_err("/* unterminated").contains("Unterminated block comment"));
    }

    #[test]
    fn test_semicolon() {
        assert_eq!(tokens("x;"), vec![
            Token::Ident("x".into()),
            Token::Semi,
        ]);
    }

    #[test]
    fn test_eq_eq() {
        assert_eq!(tokens("x == y"), vec![
            Token::Ident("x".into()),
            Token::EqEq,
            Token::Ident("y".into()),
        ]);
    }

    #[test]
    fn test_source_location() {
        let spanned = lex("x\ny").unwrap();
        assert_eq!(spanned[0].line, 1);
        assert_eq!(spanned[1].line, 2);
    }

    #[test]
    fn test_full_let_statement() {
        let toks = tokens("let %mut %i32 x = 42;");
        assert_eq!(toks, vec![
            Token::Ident("let".into()),
            Token::Directive("mut".into()),
            Token::Directive("i32".into()),
            Token::Ident("x".into()),
            Token::Eq,
            Token::Int(42),
            Token::Semi,
        ]);
    }

    #[test]
    fn test_fn_signature() {
        let toks = tokens("fn add(x: %i32, y: %i32) -> %i32");
        assert_eq!(toks, vec![
            Token::Ident("fn".into()),
            Token::Ident("add".into()),
            Token::LParen,
            Token::Ident("x".into()),
            Token::Colon,
            Token::Directive("i32".into()),
            Token::Comma,
            Token::Ident("y".into()),
            Token::Colon,
            Token::Directive("i32".into()),
            Token::RParen,
            Token::Arrow,
            Token::Directive("i32".into()),
        ]);
    }
}