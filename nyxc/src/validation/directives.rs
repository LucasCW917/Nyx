// Stage 4 — Directive Validator
// Walks the AST and verifies every % prefixed name is either a known directive
// used in the correct context, or emits a warning if it looks like a directive
// but isn't recognised. Hard errors on directives used in the wrong context.

use crate::frontend::make_pass::CompileConfig;
use crate::frontend::parser::{
    Block, ClassDef, Expr, ExprKind, FnDef, FieldDecl, LetBinding,
    NamespaceDef, Program, Span, Stmt, StmtKind, TopLevelItem, TopLevelKind,
    TypeExpr,
};
use super::{ValidationError, ValidationWarning};

const MAKE_ONLY: &[&str] = &[
    "make", "logic-%make", "suppress-warnings", "target", "entry",
    "strict", "hard", "when-run", "when-compile", "import", "use",
    "def", "repl", "async", "self", "look-for-path",
];

const TYPE_DIRECTIVES: &[&str] = &[
    "i8", "i16", "i32", "i64",
    "u8", "u16", "u32", "u64",
    "f32", "f64",
    "str", "bool", "char", "void",
    "Result",
    "nyx", "rust", "python", "js", "c", "cpp", "lua", "asm",
];

const FN_MODIFIER_DIRECTIVES: &[&str] = &["pub", "async", "spawn"];
const VAR_MODIFIER_DIRECTIVES: &[&str] = &["mut", "pub", "solve"];

const ALL_VALID: &[&str] = &[
    "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64",
    "str", "bool", "char", "void", "Result",
    "nyx", "rust", "python", "js", "c", "cpp", "lua", "asm",
    "pub", "async", "spawn", "mut", "solve", "true", "false", "self",
];

pub fn check(
    program: Program,
    config: &CompileConfig,
) -> Result<(Program, Vec<ValidationWarning>), ValidationError> {
    let mut ctx = DirectiveCtx { warnings: Vec::new(), config };
    for item in &program.items {
        ctx.check_top_level_item(item)?;
    }
    Ok((program, ctx.warnings))
}

struct DirectiveCtx<'a> {
    warnings: Vec<ValidationWarning>,
    config: &'a CompileConfig,
}

impl<'a> DirectiveCtx<'a> {
    fn warn(&mut self, msg: impl Into<String>, span: &Span) {
        if !self.config.suppress_warnings {
            self.warnings.push(ValidationWarning {
                message: msg.into(),
                stage: "Directive Validator",
                line: span.line,
                col: span.col,
            });
        }
    }

    fn check_top_level_item(&mut self, item: &TopLevelItem) -> Result<(), ValidationError> {
        match &item.kind {
            TopLevelKind::Function(f)  => self.check_fn(f, &item.span),
            TopLevelKind::Class(c)     => self.check_class(c, &item.span),
            TopLevelKind::Namespace(n) => self.check_namespace(n, &item.span),
        }
    }

    fn check_fn(&mut self, f: &FnDef, _span: &Span) -> Result<(), ValidationError> {
        if let Some(ty) = &f.return_type {
            self.check_type_expr(ty, &f.body.span)?;
        }
        for param in &f.params {
            if !param.is_self {
                self.check_type_expr(&param.ty, &param.span)?;
            }
        }
        self.check_block(&f.body)
    }

    fn check_class(&mut self, c: &ClassDef, span: &Span) -> Result<(), ValidationError> {
        for field in &c.create_block.fields {
            self.check_type_expr(&field.ty, &field.span)?;
            if let Some(default) = &field.default {
                self.check_expr(default)?;
            }
        }
        for method in &c.methods {
            self.check_fn(method, span)?;
        }
        Ok(())
    }

    fn check_namespace(&mut self, n: &NamespaceDef, span: &Span) -> Result<(), ValidationError> {
        for item in &n.items {
            self.check_top_level_item(item)?;
        }
        Ok(())
    }

    fn check_type_expr(&mut self, ty: &TypeExpr, span: &Span) -> Result<(), ValidationError> {
        match ty {
            TypeExpr::Primitive(name) => {
                self.validate_directive_name(name, DirectiveContext::Type, span)?;
            }
            TypeExpr::Generic(name, args) => {
                self.validate_directive_name(name, DirectiveContext::Type, span)?;
                for arg in args { self.check_type_expr(arg, span)?; }
            }
            TypeExpr::Array(inner) => self.check_type_expr(inner, span)?,
            TypeExpr::NyxBlock(args, ret) | TypeExpr::RustBlock(args, ret) => {
                for arg in args { self.check_type_expr(arg, span)?; }
                self.check_type_expr(ret, span)?;
            }
            TypeExpr::Inferred | TypeExpr::Void => {}
        }
        Ok(())
    }

    fn check_block(&mut self, block: &Block) -> Result<(), ValidationError> {
        for stmt in &block.stmts { self.check_stmt(stmt)?; }
        if let Some(tail) = &block.tail { self.check_expr(tail)?; }
        Ok(())
    }

    fn check_stmt(&mut self, stmt: &Stmt) -> Result<(), ValidationError> {
        match &stmt.kind {
            StmtKind::Let(b) => {
                self.check_type_expr(&b.ty, &b.span)?;
                self.check_expr(&b.value)?;
            }
            StmtKind::MassLet(bindings) => {
                for b in bindings {
                    self.check_type_expr(&b.ty, &b.span)?;
                    self.check_expr(&b.value)?;
                }
            }
            StmtKind::Solve(s)       => self.check_expr(&s.expr)?,
            StmtKind::SolveSystem(s) => {
                for eq in &s.equations { self.check_expr(eq)?; }
            }
            StmtKind::ExprStmt(e)        => self.check_expr(e)?,
            StmtKind::Return(Some(e))    => self.check_expr(e)?,
            StmtKind::Return(None)       => {}
        }
        Ok(())
    }

    fn check_expr(&mut self, expr: &Expr) -> Result<(), ValidationError> {
        match &expr.kind {
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_)
            | ExprKind::Bool(_) | ExprKind::Void | ExprKind::Ident(_)
            | ExprKind::Path(_) | ExprKind::Interpolated(_) => {}

            ExprKind::Unary(_, e) | ExprKind::Borrow(e) | ExprKind::BorrowMut(e)
            | ExprKind::Propagate(e) | ExprKind::Await(e) | ExprKind::Panic(e)
            | ExprKind::Ok(e) | ExprKind::Err(e) => self.check_expr(e)?,

            ExprKind::Return(Some(e)) | ExprKind::Break(Some(e)) => self.check_expr(e)?,
            ExprKind::Return(None) | ExprKind::Break(None) => {}

            ExprKind::Binary(_, l, r) | ExprKind::Assign(l, r)
            | ExprKind::Range(l, r) => {
                self.check_expr(l)?;
                self.check_expr(r)?;
            }
            ExprKind::Call(callee, args) => {
                self.check_expr(callee)?;
                for a in args { self.check_expr(a)?; }
            }
            ExprKind::MethodCall(obj, _, args) => {
                self.check_expr(obj)?;
                for a in args { self.check_expr(a)?; }
            }
            ExprKind::Field(obj, _) => self.check_expr(obj)?,
            ExprKind::Index(obj, idx) => {
                self.check_expr(obj)?;
                self.check_expr(idx)?;
            }
            ExprKind::Array(elems) => {
                for e in elems { self.check_expr(e)?; }
            }
            ExprKind::Block(b) => self.check_block(b)?,
            ExprKind::Loop(body) => self.check_block(body)?,
            ExprKind::While(cond, body) => {
                self.check_expr(cond)?;
                self.check_block(body)?;
            }
            ExprKind::For(_, iter, body) => {
                self.check_expr(iter)?;
                self.check_block(body)?;
            }
            ExprKind::If(cond, then_b, else_ifs, else_b) => {
                self.check_expr(cond)?;
                self.check_block(then_b)?;
                for (ei_cond, ei_b) in else_ifs {
                    self.check_expr(ei_cond)?;
                    self.check_block(ei_b)?;
                }
                if let Some(eb) = else_b { self.check_block(eb)?; }
            }
            ExprKind::Match(subject, arms) => {
                self.check_expr(subject)?;
                for arm in arms { self.check_expr(&arm.body)?; }
            }
            ExprKind::NyxBlock(params, ret_ty, body) => {
                for p in params { self.check_type_expr(&p.ty, &p.span)?; }
                if let Some(ty) = ret_ty { self.check_type_expr(ty, &expr.span)?; }
                self.check_block(body)?;
            }
            ExprKind::RustBlock(params, ret_ty, _) => {
                for p in params { self.check_type_expr(&p.ty, &p.span)?; }
                if let Some(ty) = ret_ty { self.check_type_expr(ty, &expr.span)?; }
            }
            ExprKind::Create(class, fields) => {
                self.check_expr(class)?;
                for (_, val) in fields { self.check_expr(val)?; }
            }
        }
        Ok(())
    }

    fn validate_directive_name(
        &mut self,
        name: &str,
        ctx: DirectiveContext,
        span: &Span,
    ) -> Result<(), ValidationError> {
        if MAKE_ONLY.contains(&name) {
            return Err(ValidationError::directive(
                format!(
                    "'%{}' is a compile-time config directive and is only valid inside \
                     #fn %make(). Remove it or move it into your %make block.",
                    name
                ),
                span,
            ));
        }

        let valid_in_context = match ctx {
            DirectiveContext::Type   => TYPE_DIRECTIVES.contains(&name),
            DirectiveContext::FnMod  => FN_MODIFIER_DIRECTIVES.contains(&name),
            DirectiveContext::VarMod => VAR_MODIFIER_DIRECTIVES.contains(&name),
            DirectiveContext::General => ALL_VALID.contains(&name),
        };

        if !valid_in_context {
            if ALL_VALID.contains(&name) {
                return Err(ValidationError::directive(
                    format!("'%{}' is a valid directive but cannot be used here (wrong context)", name),
                    span,
                ));
            }
            let suggestion = closest_directive(name);
            let msg = match suggestion {
                Some(s) => format!(
                    "'%{}' is not a known directive. Did you mean '%{}'? Treating as an identifier.",
                    name, s
                ),
                None => format!(
                    "'%{}' is not a known directive. Treating as an identifier.", name
                ),
            };
            self.warn(msg, span);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
enum DirectiveContext {
    Type, FnMod, VarMod, General,
}

fn closest_directive(name: &str) -> Option<&'static str> {
    let all: &[&str] = &[
        "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64",
        "str", "bool", "char", "void", "Result", "nyx", "rust",
        "pub", "async", "spawn", "mut", "solve", "true", "false", "self",
        "suppress-warnings", "target", "entry", "strict", "hard",
        "when-run", "when-compile", "import", "use", "def", "repl",
        "look-for-path", "logic-%make",
    ];
    all.iter()
        .map(|&d| (d, edit_distance(name, d)))
        .filter(|(_, dist)| *dist <= 3)
        .min_by_key(|(_, dist)| *dist)
        .map(|(d, _)| d)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in 0..=m { dp[i][0] = i; }
    for j in 0..=n { dp[0][j] = j; }
    for i in 1..=m {
        for j in 1..=n {
            dp[i][j] = if a[i-1] == b[j-1] { dp[i-1][j-1] }
            else { 1 + dp[i-1][j].min(dp[i][j-1]).min(dp[i-1][j-1]) };
        }
    }
    dp[m][n]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edit_distance() {
        assert_eq!(edit_distance("mut", "mut"), 0);
        assert_eq!(edit_distance("mute", "mut"), 1);
        assert_eq!(edit_distance("pub", "sub"), 1);
    }

    #[test]
    fn test_closest_directive() {
        assert_eq!(closest_directive("mute"), Some("mut"));
        assert_eq!(closest_directive("asynch"), Some("async"));
    }
}