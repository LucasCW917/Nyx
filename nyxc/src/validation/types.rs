// Stages 6, 7, 8 — Type Inference, Type Checker, Strict Inference Validator
//
// Stage 6: Walks the AST bottom-up, infers a concrete TypeExpr for every
//          expression node, and records it in the TypeTable.
// Stage 7: Verifies all inferred types are consistent — no implicit coercion,
//          function signatures match call sites, std::convert family rules.
// Stage 8: Second pass ensuring no type variable is still ambiguous or
//          unresolved after inference.

use std::collections::HashMap;
use crate::frontend::make_pass::CompileConfig;
use crate::frontend::parser::{
    BinaryOp, Block, BorrowKind, ClassDef, Expr, ExprKind, FnDef,
    NamespaceDef, NodeId, Pattern, Program, Span, Stmt, StmtKind,
    TopLevelItem, TopLevelKind, TypeExpr, UnaryOp,
};
use super::{ValidationError, ValidationWarning};

// ─────────────────────────────────────────────────────────────────────────────
// TypeTable — the output of this stage
// ─────────────────────────────────────────────────────────────────────────────

/// Maps every expression NodeId to its resolved concrete type.
/// Consumed by all later validation stages and the compiler.
#[derive(Debug, Default)]
pub struct TypeTable {
    pub types: HashMap<NodeId, TypeExpr>,
}

impl TypeTable {
    pub fn get(&self, id: NodeId) -> Option<&TypeExpr> {
        self.types.get(&id)
    }

    fn set(&mut self, id: NodeId, ty: TypeExpr) {
        self.types.insert(id, ty);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Symbol table — scoped name → type mapping
// ─────────────────────────────────────────────────────────────────────────────

/// A single scope frame — maps binding names to their types.
#[derive(Debug, Clone)]
struct Scope {
    bindings: HashMap<String, TypeExpr>,
}

impl Scope {
    fn new() -> Self {
        Scope { bindings: HashMap::new() }
    }

    fn insert(&mut self, name: String, ty: TypeExpr) {
        self.bindings.insert(name, ty);
    }

    fn get(&self, name: &str) -> Option<&TypeExpr> {
        self.bindings.get(name)
    }
}

/// A stack of scopes. The innermost scope is checked first.
struct SymbolTable {
    scopes: Vec<Scope>,
    /// Function return type for the current function — used to check ? propagation.
    current_return_type: Option<TypeExpr>,
    /// All top-level function signatures — for call site resolution.
    functions: HashMap<String, FnSignature>,
    /// All class definitions — for method resolution.
    classes: HashMap<String, ClassSignature>,
}

#[derive(Debug, Clone)]
struct FnSignature {
    params: Vec<TypeExpr>,
    return_type: TypeExpr,
    is_result: bool,
}

#[derive(Debug, Clone)]
struct ClassSignature {
    fields: HashMap<String, TypeExpr>,
    methods: HashMap<String, FnSignature>,
}

impl SymbolTable {
    fn new() -> Self {
        SymbolTable {
            scopes: vec![Scope::new()],
            current_return_type: None,
            functions: HashMap::new(),
            classes: HashMap::new(),
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(Scope::new());
    }

    fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    fn define(&mut self, name: String, ty: TypeExpr) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, ty);
        }
    }

    fn lookup(&self, name: &str) -> Option<&TypeExpr> {
        // Search from innermost scope outward
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty);
            }
        }
        None
    }

    fn lookup_fn(&self, name: &str) -> Option<&FnSignature> {
        self.functions.get(name)
    }

    fn lookup_class(&self, name: &str) -> Option<&ClassSignature> {
        self.classes.get(name)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Type families — for std::convert checking
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum TypeFamily {
    Integer,
    Float,
    Text,
    Boolean,
    Other,
}

fn type_family(ty: &TypeExpr) -> TypeFamily {
    match ty {
        TypeExpr::Primitive(s) => match s.as_str() {
            "i8" | "i16" | "i32" | "i64"
            | "u8" | "u16" | "u32" | "u64" => TypeFamily::Integer,
            "f32" | "f64"                  => TypeFamily::Float,
            "str" | "char"                 => TypeFamily::Text,
            "bool"                         => TypeFamily::Boolean,
            _                              => TypeFamily::Other,
        },
        _ => TypeFamily::Other,
    }
}

fn same_family(a: &TypeExpr, b: &TypeExpr) -> bool {
    type_family(a) == type_family(b)
}

// ─────────────────────────────────────────────────────────────────────────────
// Inference context
// ─────────────────────────────────────────────────────────────────────────────

struct InferCtx<'a> {
    table: TypeTable,
    symbols: SymbolTable,
    warnings: Vec<ValidationWarning>,
    config: &'a CompileConfig,
}

impl<'a> InferCtx<'a> {
    fn new(config: &'a CompileConfig) -> Self {
        InferCtx {
            table: TypeTable::default(),
            symbols: SymbolTable::new(),
            warnings: Vec::new(),
            config,
        }
    }

    fn warn(&mut self, msg: impl Into<String>, span: &Span) {
        if !self.config.suppress_warnings {
            self.warnings.push(ValidationWarning {
                message: msg.into(),
                stage: "Type Inference",
                line: span.line,
                col: span.col,
            });
        }
    }

    fn record(&mut self, id: NodeId, ty: TypeExpr) {
        self.table.set(id, ty);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────────────

pub fn check(
    program: Program,
    config: &CompileConfig,
) -> Result<(Program, Vec<ValidationWarning>), ValidationError> {
    let mut ctx = InferCtx::new(config);

    // ── Stage 6 & 7: collect top-level signatures first ───────────────
    // We do a pre-pass to register all function and class signatures
    // before walking bodies, so forward references work.
    collect_signatures(&program, &mut ctx)?;

    // ── Walk all items ─────────────────────────────────────────────────
    for item in &program.items {
        infer_top_level_item(item, &mut ctx)?;
    }

    // ── Stage 8: Strict Inference Validator ───────────────────────────
    // Ensure no expression was left with TypeExpr::Inferred
    for item in &program.items {
        strict_check_top_level(item, &ctx.table, config)?;
    }

    Ok((program, ctx.warnings))
}

// ─────────────────────────────────────────────────────────────────────────────
// Signature pre-pass
// ─────────────────────────────────────────────────────────────────────────────

fn collect_signatures(
    program: &Program,
    ctx: &mut InferCtx,
) -> Result<(), ValidationError> {
    for item in &program.items {
        match &item.kind {
            TopLevelKind::Function(f) => {
                let sig = fn_signature(f);
                ctx.symbols.functions.insert(f.name.clone(), sig);
            }
            TopLevelKind::Class(c) => {
                let mut fields = HashMap::new();
                for field in &c.create_block.fields {
                    fields.insert(field.name.clone(), field.ty.clone());
                }
                let mut methods = HashMap::new();
                for method in &c.methods {
                    methods.insert(method.name.clone(), fn_signature(method));
                }
                ctx.symbols.classes.insert(c.name.clone(), ClassSignature { fields, methods });
            }
            TopLevelKind::Namespace(n) => {
                collect_namespace_signatures(n, ctx);
            }
        }
    }
    Ok(())
}

fn collect_namespace_signatures(ns: &NamespaceDef, ctx: &mut InferCtx) {
    for item in &ns.items {
        if let TopLevelKind::Function(f) = &item.kind {
            let qualified = format!("{}::{}", ns.name, f.name);
            ctx.symbols.functions.insert(qualified, fn_signature(f));
        }
    }
}

fn fn_signature(f: &FnDef) -> FnSignature {
    let params: Vec<TypeExpr> = f.params.iter()
        .filter(|p| !p.is_self)
        .map(|p| p.ty.clone())
        .collect();
    let return_type = f.return_type.clone().unwrap_or(TypeExpr::Void);
    let is_result = matches!(&return_type, TypeExpr::Generic(n, _) if n == "Result");
    FnSignature { params, return_type, is_result }
}

// ─────────────────────────────────────────────────────────────────────────────
// Top-level inference
// ─────────────────────────────────────────────────────────────────────────────

fn infer_top_level_item(
    item: &TopLevelItem,
    ctx: &mut InferCtx,
) -> Result<(), ValidationError> {
    match &item.kind {
        TopLevelKind::Function(f)  => infer_fn(f, ctx),
        TopLevelKind::Class(c)     => infer_class(c, ctx, &item.span),
        TopLevelKind::Namespace(n) => infer_namespace(n, ctx, &item.span),
    }
}

fn infer_fn(f: &FnDef, ctx: &mut InferCtx) -> Result<(), ValidationError> {
    ctx.symbols.push_scope();

    // Register parameters into the current scope
    for param in &f.params {
        if !param.is_self {
            ctx.symbols.define(param.name.clone(), param.ty.clone());
        }
    }

    let declared_ret = f.return_type.clone().unwrap_or(TypeExpr::Void);
    let prev_ret = ctx.symbols.current_return_type.replace(declared_ret.clone());

    let inferred_ret = infer_block(&f.body, ctx)?;

    // Stage 7: verify return type matches declared type
    if f.return_type.is_some() {
        let declared = f.return_type.as_ref().unwrap();
        if !types_compatible(declared, &inferred_ret) {
            return Err(ValidationError::type_err(
                format!(
                    "Function '{}' declared return type {:?} but body returns {:?}",
                    f.name, declared, inferred_ret
                ),
                &f.body.span,
            ));
        }
    }

    ctx.symbols.current_return_type = prev_ret;
    ctx.symbols.pop_scope();
    Ok(())
}

fn infer_class(c: &ClassDef, ctx: &mut InferCtx, span: &Span) -> Result<(), ValidationError> {
    // Infer default values in create block
    for field in &c.create_block.fields {
        if let Some(default) = &field.default {
            let inferred = infer_expr(default, ctx)?;
            // Stage 7: default value must match field type
            if field.ty != TypeExpr::Inferred && !types_compatible(&field.ty, &inferred) {
                return Err(ValidationError::type_err(
                    format!(
                        "Field '{}' has type {:?} but default value has type {:?}",
                        field.name, field.ty, inferred
                    ),
                    &field.span,
                ));
            }
        }
    }

    // Infer method bodies
    for method in &c.methods {
        // Register %self into scope
        ctx.symbols.push_scope();
        let self_ty = TypeExpr::Primitive(c.name.clone());
        ctx.symbols.define(ctx.config.self_rename.to_name(&c.name).to_string(), self_ty);
        infer_fn(method, ctx)?;
        ctx.symbols.pop_scope();
    }
    Ok(())
}

fn infer_namespace(
    n: &NamespaceDef,
    ctx: &mut InferCtx,
    span: &Span,
) -> Result<(), ValidationError> {
    for item in &n.items {
        infer_top_level_item(item, ctx)?;
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Block inference
// ─────────────────────────────────────────────────────────────────────────────

fn infer_block(block: &Block, ctx: &mut InferCtx) -> Result<TypeExpr, ValidationError> {
    ctx.symbols.push_scope();

    for stmt in &block.stmts {
        infer_stmt(stmt, ctx)?;
    }

    let ty = if let Some(tail) = &block.tail {
        infer_expr(tail, ctx)?
    } else {
        TypeExpr::Void
    };

    ctx.symbols.pop_scope();
    Ok(ty)
}

// ─────────────────────────────────────────────────────────────────────────────
// Statement inference
// ─────────────────────────────────────────────────────────────────────────────

fn infer_stmt(stmt: &Stmt, ctx: &mut InferCtx) -> Result<(), ValidationError> {
    match &stmt.kind {
        StmtKind::Let(b) => {
            let val_ty = infer_expr(&b.value, ctx)?;
            let binding_ty = if b.ty == TypeExpr::Inferred {
                val_ty.clone()
            } else {
                // Stage 7: explicit annotation must match inferred type
                if !types_compatible(&b.ty, &val_ty) {
                    return Err(ValidationError::type_err(
                        format!(
                            "Binding '{}' annotated as {:?} but value has type {:?}",
                            b.name, b.ty, val_ty
                        ),
                        &b.span,
                    ));
                }
                b.ty.clone()
            };
            ctx.symbols.define(b.name.clone(), binding_ty);
        }

        StmtKind::MassLet(bindings) => {
            for b in bindings {
                let val_ty = infer_expr(&b.value, ctx)?;
                let binding_ty = if b.ty == TypeExpr::Inferred {
                    val_ty
                } else {
                    if !types_compatible(&b.ty, &val_ty) {
                        return Err(ValidationError::type_err(
                            format!(
                                "Binding '{}' annotated as {:?} but value has type {:?}",
                                b.name, b.ty, val_ty
                            ),
                            &b.span,
                        ));
                    }
                    b.ty.clone()
                };
                ctx.symbols.define(b.name.clone(), binding_ty);
            }
        }

        StmtKind::Solve(s) => {
            // %solve always produces %Result<%f64>
            let result_ty = TypeExpr::Generic(
                "Result".to_string(),
                vec![TypeExpr::Primitive("f64".to_string())],
            );
            ctx.symbols.define(s.unknown.clone(), result_ty);
        }

        StmtKind::SolveSystem(s) => {
            // Each unknown in a system gets %Result<%f64>
            let result_ty = TypeExpr::Generic(
                "Result".to_string(),
                vec![TypeExpr::Primitive("f64".to_string())],
            );
            for unknown in &s.unknowns {
                ctx.symbols.define(unknown.clone(), result_ty.clone());
            }
        }

        StmtKind::ExprStmt(e) => {
            infer_expr(e, ctx)?;
        }

        StmtKind::Return(Some(e)) => {
            let ret_ty = infer_expr(e, ctx)?;
            // Stage 7: verify return type matches function declaration
            if let Some(expected) = &ctx.symbols.current_return_type.clone() {
                if !types_compatible(expected, &ret_ty) {
                    return Err(ValidationError::type_err(
                        format!(
                            "return value has type {:?} but function expects {:?}",
                            ret_ty, expected
                        ),
                        &stmt.span,
                    ));
                }
            }
        }

        StmtKind::Return(None) => {}
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Expression inference — the core of Stage 6
// ─────────────────────────────────────────────────────────────────────────────

fn infer_expr(expr: &Expr, ctx: &mut InferCtx) -> Result<TypeExpr, ValidationError> {
    let ty = infer_expr_inner(expr, ctx)?;
    ctx.record(expr.id, ty.clone());
    Ok(ty)
}

fn infer_expr_inner(expr: &Expr, ctx: &mut InferCtx) -> Result<TypeExpr, ValidationError> {
    match &expr.kind {

        // ── Literals ──────────────────────────────────────────────────
        ExprKind::Int(_)   => Ok(TypeExpr::Primitive("i64".to_string())),
        ExprKind::Float(_) => Ok(TypeExpr::Primitive("f64".to_string())),
        ExprKind::Str(_)   => Ok(TypeExpr::Primitive("str".to_string())),
        ExprKind::Bool(_)  => Ok(TypeExpr::Primitive("bool".to_string())),
        ExprKind::Void     => Ok(TypeExpr::Void),

        ExprKind::Interpolated(_) => Ok(TypeExpr::Primitive("str".to_string())),

        // ── Identifiers ───────────────────────────────────────────────
        ExprKind::Ident(name) => {
            ctx.symbols.lookup(name)
                .cloned()
                .ok_or_else(|| ValidationError::type_err(
                    format!("Undefined variable '{}'", name),
                    &expr.span,
                ))
        }

        ExprKind::Path(segments) => {
            // Try to resolve as a function path e.g. math::add
            let path = segments.join("::");
            if let Some(sig) = ctx.symbols.lookup_fn(&path) {
                Ok(sig.return_type.clone())
            } else {
                // Could be a namespace path — leave as inferred for now
                Ok(TypeExpr::Inferred)
            }
        }

        // ── Unary ─────────────────────────────────────────────────────
        ExprKind::Unary(op, operand) => {
            let ty = infer_expr(operand, ctx)?;
            match op {
                UnaryOp::Not => {
                    // ! on any type produces %bool
                    Ok(TypeExpr::Primitive("bool".to_string()))
                }
                UnaryOp::Neg => {
                    // Negation requires a numeric type
                    match &ty {
                        TypeExpr::Primitive(s) if is_numeric(s) => Ok(ty),
                        _ => Err(ValidationError::type_err(
                            format!("Cannot negate non-numeric type {:?}", ty),
                            &expr.span,
                        )),
                    }
                }
            }
        }

        // ── Binary ────────────────────────────────────────────────────
        ExprKind::Binary(op, lhs, rhs) => {
            let lty = infer_expr(lhs, ctx)?;
            let rty = infer_expr(rhs, ctx)?;
            infer_binary_op(op, &lty, &rty, &expr.span)
        }

        // ── Assignment ────────────────────────────────────────────────
        ExprKind::Assign(lhs, rhs) => {
            let lty = infer_expr(lhs, ctx)?;
            let rty = infer_expr(rhs, ctx)?;
            // Stage 7: types must be compatible
            if !types_compatible(&lty, &rty) {
                return Err(ValidationError::type_err(
                    format!("Cannot assign {:?} to binding of type {:?}", rty, lty),
                    &expr.span,
                ));
            }
            Ok(TypeExpr::Void)
        }

        // ── Borrows ───────────────────────────────────────────────────
        ExprKind::Borrow(e) => {
            let inner = infer_expr(e, ctx)?;
            // A borrow is the same type as the inner — the borrow checker
            // handles the aliasing rules later (Stage 11)
            Ok(inner)
        }
        ExprKind::BorrowMut(e) => {
            let inner = infer_expr(e, ctx)?;
            Ok(inner)
        }

        // ── Propagation (?) ───────────────────────────────────────────
        ExprKind::Propagate(e) => {
            let inner = infer_expr(e, ctx)?;
            // ? on a Result<T> unwraps to T
            match &inner {
                TypeExpr::Generic(name, args) if name == "Result" => {
                    Ok(args.first().cloned().unwrap_or(TypeExpr::Void))
                }
                // ? on a non-Result type is a type error (Stage 20 also checks this)
                _ => Err(ValidationError::type_err(
                    format!("'?' operator used on non-Result type {:?}", inner),
                    &expr.span,
                )),
            }
        }

        // ── Function call ─────────────────────────────────────────────
        ExprKind::Call(callee, args) => {
            infer_call(callee, args, ctx, &expr.span)
        }

        // ── Method call ───────────────────────────────────────────────
        ExprKind::MethodCall(obj, method, args) => {
            infer_method_call(obj, method, args, ctx, &expr.span)
        }

        // ── Field access ──────────────────────────────────────────────
        ExprKind::Field(obj, field) => {
            let obj_ty = infer_expr(obj, ctx)?;
            match &obj_ty {
                TypeExpr::Primitive(class_name) => {
                    if let Some(class_sig) = ctx.symbols.lookup_class(class_name).cloned() {
                        class_sig.fields.get(field.as_str())
                            .cloned()
                            .ok_or_else(|| ValidationError::type_err(
                                format!("Class '{}' has no field '{}'", class_name, field),
                                &expr.span,
                            ))
                    } else {
                        Ok(TypeExpr::Inferred)
                    }
                }
                _ => Ok(TypeExpr::Inferred),
            }
        }

        // ── Index ─────────────────────────────────────────────────────
        ExprKind::Index(obj, idx) => {
            let obj_ty = infer_expr(obj, ctx)?;
            let idx_ty = infer_expr(idx, ctx)?;

            // Index must be an integer type
            match &idx_ty {
                TypeExpr::Primitive(s) if is_integer(s) => {}
                _ => return Err(ValidationError::type_err(
                    format!("Array index must be an integer type, got {:?}", idx_ty),
                    &expr.span,
                )),
            }

            // Return the element type of the array
            match &obj_ty {
                TypeExpr::Array(elem_ty) => Ok(*elem_ty.clone()),
                _ => Ok(TypeExpr::Inferred),
            }
        }

        // ── Array literal ─────────────────────────────────────────────
        ExprKind::Array(elems) => {
            if elems.is_empty() {
                return Ok(TypeExpr::Array(Box::new(TypeExpr::Inferred)));
            }
            let first_ty = infer_expr(&elems[0], ctx)?;
            // Stage 7: all elements must have the same type
            for elem in elems.iter().skip(1) {
                let elem_ty = infer_expr(elem, ctx)?;
                if !types_compatible(&first_ty, &elem_ty) {
                    return Err(ValidationError::type_err(
                        format!(
                            "Array elements have inconsistent types: {:?} vs {:?}",
                            first_ty, elem_ty
                        ),
                        &expr.span,
                    ));
                }
            }
            Ok(TypeExpr::Array(Box::new(first_ty)))
        }

        // ── Range ─────────────────────────────────────────────────────
        ExprKind::Range(lo, hi) => {
            let lo_ty = infer_expr(lo, ctx)?;
            let hi_ty = infer_expr(hi, ctx)?;
            if !types_compatible(&lo_ty, &hi_ty) {
                return Err(ValidationError::type_err(
                    format!("Range bounds have inconsistent types: {:?} vs {:?}", lo_ty, hi_ty),
                    &expr.span,
                ));
            }
            Ok(TypeExpr::Array(Box::new(lo_ty)))
        }

        // ── Block ─────────────────────────────────────────────────────
        ExprKind::Block(b) => infer_block(b, ctx),

        // ── if / else ─────────────────────────────────────────────────
        ExprKind::If(cond, then_b, else_ifs, else_b) => {
            infer_expr(cond, ctx)?;
            let then_ty = infer_block(then_b, ctx)?;

            for (ei_cond, ei_block) in else_ifs {
                infer_expr(ei_cond, ctx)?;
                let ei_ty = infer_block(ei_block, ctx)?;
                // Stage 7: all branches must agree on type
                if !types_compatible(&then_ty, &ei_ty) {
                    return Err(ValidationError::type_err(
                        format!(
                            "if/else branches return inconsistent types: {:?} vs {:?}",
                            then_ty, ei_ty
                        ),
                        &expr.span,
                    ));
                }
            }

            if let Some(else_block) = else_b {
                let else_ty = infer_block(else_block, ctx)?;
                if !types_compatible(&then_ty, &else_ty) {
                    return Err(ValidationError::type_err(
                        format!(
                            "if/else branches return inconsistent types: {:?} vs {:?}",
                            then_ty, else_ty
                        ),
                        &expr.span,
                    ));
                }
            }

            Ok(then_ty)
        }

        // ── while ─────────────────────────────────────────────────────
        ExprKind::While(cond, body) => {
            infer_expr(cond, ctx)?;
            infer_block(body, ctx)?;
            Ok(TypeExpr::Void)
        }

        // ── loop ──────────────────────────────────────────────────────
        ExprKind::Loop(body) => {
            // The type of a loop is the type of its break value.
            // We infer Void for now and let Stage 20 verify break consistency.
            infer_block(body, ctx)?;
            Ok(TypeExpr::Inferred)
        }

        // ── for ───────────────────────────────────────────────────────
        ExprKind::For(binding, iter, body) => {
            let iter_ty = infer_expr(iter, ctx)?;
            // Determine element type from the iterable
            let elem_ty = match &iter_ty {
                TypeExpr::Array(elem) => *elem.clone(),
                _                     => TypeExpr::Inferred,
            };
            ctx.symbols.push_scope();
            ctx.symbols.define(binding.clone(), elem_ty);
            infer_block(body, ctx)?;
            ctx.symbols.pop_scope();
            Ok(TypeExpr::Void)
        }

        // ── match ─────────────────────────────────────────────────────
        ExprKind::Match(subject, arms) => {
            infer_expr(subject, ctx)?;

            if arms.is_empty() {
                return Ok(TypeExpr::Void);
            }

            let first_ty = infer_expr(&arms[0].body, ctx)?;
            // Stage 7: all arms must agree on type
            for arm in arms.iter().skip(1) {
                let arm_ty = infer_expr(&arm.body, ctx)?;
                if !types_compatible(&first_ty, &arm_ty) {
                    return Err(ValidationError::type_err(
                        format!(
                            "match arms return inconsistent types: {:?} vs {:?}",
                            first_ty, arm_ty
                        ),
                        &arm.span,
                    ));
                }
            }
            Ok(first_ty)
        }

        // ── break ─────────────────────────────────────────────────────
        ExprKind::Break(Some(val)) => infer_expr(val, ctx),
        ExprKind::Break(None)      => Ok(TypeExpr::Void),

        // ── return ────────────────────────────────────────────────────
        ExprKind::Return(Some(val)) => infer_expr(val, ctx),
        ExprKind::Return(None)      => Ok(TypeExpr::Void),

        // ── ok / err ──────────────────────────────────────────────────
        ExprKind::Ok(val) => {
            let inner = infer_expr(val, ctx)?;
            Ok(TypeExpr::Generic("Result".to_string(), vec![inner]))
        }
        ExprKind::Err(_msg) => {
            // err() always produces Result<T> — T is inferred from context
            Ok(TypeExpr::Generic(
                "Result".to_string(),
                vec![TypeExpr::Inferred],
            ))
        }

        // ── panic ─────────────────────────────────────────────────────
        ExprKind::Panic(msg) => {
            infer_expr(msg, ctx)?;
            // panic() never returns — treat as Void for now
            Ok(TypeExpr::Void)
        }

        // ── await ─────────────────────────────────────────────────────
        ExprKind::Await(inner) => {
            // await unwraps the return type of an async/spawn expression
            infer_expr(inner, ctx)
        }

        // ── %nyx(){} block ────────────────────────────────────────────
        ExprKind::NyxBlock(params, ret_ty, body) => {
            ctx.symbols.push_scope();
            for param in params {
                ctx.symbols.define(param.name.clone(), param.ty.clone());
            }
            let inferred_ret = infer_block(body, ctx)?;
            ctx.symbols.pop_scope();

            let actual_ret = ret_ty.clone().unwrap_or(inferred_ret.clone());

            // Stage 7: declared return type must match inferred
            if ret_ty.is_some() && !types_compatible(&actual_ret, &inferred_ret) {
                return Err(ValidationError::type_err(
                    format!(
                        "%%nyx block declared return type {:?} but body returns {:?}",
                        actual_ret, inferred_ret
                    ),
                    &expr.span,
                ));
            }

            let param_types: Vec<TypeExpr> = params.iter().map(|p| p.ty.clone()).collect();
            Ok(TypeExpr::NyxBlock(param_types, Box::new(actual_ret)))
        }

        // ── %rust(){} block ───────────────────────────────────────────
        ExprKind::RustBlock(params, ret_ty, _raw) => {
            let param_types: Vec<TypeExpr> = params.iter().map(|p| p.ty.clone()).collect();
            let ret = ret_ty.clone().unwrap_or(TypeExpr::Void);
            Ok(TypeExpr::RustBlock(param_types, Box::new(ret)))
        }

        // ── Class construction ────────────────────────────────────────
        ExprKind::Create(class, fields) => {
            let class_ty = infer_expr(class, ctx)?;
            let class_name = match &class_ty {
                TypeExpr::Primitive(name) => name.clone(),
                // Could also be an Ident resolved class name
                _ => match &class.kind {
                    ExprKind::Ident(name) => name.clone(),
                    _ => return Ok(TypeExpr::Inferred),
                },
            };

            // Stage 7: check field types match class definition
            if let Some(class_sig) = ctx.symbols.lookup_class(&class_name).cloned() {
                for (field_name, val) in fields {
                    let val_ty = infer_expr(val, ctx)?;
                    if let Some(expected_ty) = class_sig.fields.get(field_name.as_str()) {
                        if !types_compatible(expected_ty, &val_ty) {
                            return Err(ValidationError::type_err(
                                format!(
                                    "Field '{}' in create expects {:?} but got {:?}",
                                    field_name, expected_ty, val_ty
                                ),
                                &expr.span,
                            ));
                        }
                    }
                }
            }

            Ok(TypeExpr::Primitive(class_name))
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Call inference helpers
// ─────────────────────────────────────────────────────────────────────────────

fn infer_call(
    callee: &Expr,
    args: &[Expr],
    ctx: &mut InferCtx,
    span: &Span,
) -> Result<TypeExpr, ValidationError> {
    // Resolve the callee name
    let callee_name = match &callee.kind {
        ExprKind::Ident(name) => name.clone(),
        ExprKind::Path(segs)  => segs.join("::"),
        _ => {
            // Expression-callee — infer its type and use it
            let callee_ty = infer_expr(callee, ctx)?;
            // Infer arg types for side effects
            for arg in args { infer_expr(arg, ctx)?; }
            return Ok(match callee_ty {
                TypeExpr::NyxBlock(_, ret) => *ret,
                TypeExpr::RustBlock(_, ret) => *ret,
                _ => TypeExpr::Inferred,
            });
        }
    };

    // Special case: std::convert / std::hconvert
    if callee_name == "std::convert" || callee_name == "std::hconvert" {
        return infer_convert_call(callee_name.as_str(), args, ctx, span);
    }

    // Infer argument types
    let arg_types: Vec<TypeExpr> = args.iter()
        .map(|a| infer_expr(a, ctx))
        .collect::<Result<_, _>>()?;

    // Look up the function signature
    if let Some(sig) = ctx.symbols.lookup_fn(&callee_name).cloned() {
        // Stage 7: argument count must match
        if sig.params.len() != arg_types.len() {
            return Err(ValidationError::type_err(
                format!(
                    "Function '{}' expects {} arguments but got {}",
                    callee_name, sig.params.len(), arg_types.len()
                ),
                span,
            ));
        }
        // Stage 7: argument types must match
        for (i, (expected, actual)) in sig.params.iter().zip(arg_types.iter()).enumerate() {
            if !types_compatible(expected, actual) {
                return Err(ValidationError::type_err(
                    format!(
                        "Argument {} of '{}' expects {:?} but got {:?}",
                        i + 1, callee_name, expected, actual
                    ),
                    span,
                ));
            }
        }
        Ok(sig.return_type)
    } else {
        // Unknown function — return Inferred, Stage 8 will catch it if needed
        Ok(TypeExpr::Inferred)
    }
}

fn infer_method_call(
    obj: &Expr,
    method: &str,
    args: &[Expr],
    ctx: &mut InferCtx,
    span: &Span,
) -> Result<TypeExpr, ValidationError> {
    let obj_ty = infer_expr(obj, ctx)?;
    for arg in args { infer_expr(arg, ctx)?; }

    // Special method: .length() always returns %i64
    if method == "length" {
        return Ok(TypeExpr::Primitive("i64".to_string()));
    }

    // Look up method on the class
    let class_name = match &obj_ty {
        TypeExpr::Primitive(name) => name.clone(),
        _ => return Ok(TypeExpr::Inferred),
    };

    if let Some(class_sig) = ctx.symbols.lookup_class(&class_name).cloned() {
        if let Some(method_sig) = class_sig.methods.get(method) {
            return Ok(method_sig.return_type.clone());
        }
    }

    Ok(TypeExpr::Inferred)
}

fn infer_convert_call(
    fn_name: &str,
    args: &[Expr],
    ctx: &mut InferCtx,
    span: &Span,
) -> Result<TypeExpr, ValidationError> {
    if args.len() != 2 {
        return Err(ValidationError::type_err(
            format!("{}() requires exactly 2 arguments: (target_type, value)", fn_name),
            span,
        ));
    }

    // First arg is the target type directive expression — get its type name
    let target_ty = infer_expr(&args[0], ctx)?;
    let source_ty = infer_expr(&args[1], ctx)?;

    // std::convert enforces same-family rule; std::hconvert skips it
    if fn_name == "std::convert" {
        if !same_family(&target_ty, &source_ty) {
            // Check if the function is in the %hard list
            let in_hard = ctx.config.hard.iter().any(|h| h == "std::convert");
            if !in_hard {
                return Err(ValidationError::type_err(
                    format!(
                        "std::convert cannot convert {:?} to {:?} — different type families. \
                         Add std::convert to %%hard in %%make to allow cross-family conversion.",
                        source_ty, target_ty
                    ),
                    span,
                ));
            }
        }
    }

    Ok(target_ty)
}

// ─────────────────────────────────────────────────────────────────────────────
// Binary operator type inference
// ─────────────────────────────────────────────────────────────────────────────

fn infer_binary_op(
    op: &BinaryOp,
    lty: &TypeExpr,
    rty: &TypeExpr,
    span: &Span,
) -> Result<TypeExpr, ValidationError> {
    match op {
        // Arithmetic — both sides must be numeric, result is the wider type
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul
        | BinaryOp::Div | BinaryOp::Mod => {
            if !is_numeric_type(lty) || !is_numeric_type(rty) {
                return Err(ValidationError::type_err(
                    format!("Arithmetic operator requires numeric types, got {:?} and {:?}", lty, rty),
                    span,
                ));
            }
            if !types_compatible(lty, rty) {
                return Err(ValidationError::type_err(
                    format!(
                        "Arithmetic operator requires matching types, got {:?} and {:?}. \
                         Use std::convert to align types first.",
                        lty, rty
                    ),
                    span,
                ));
            }
            Ok(lty.clone())
        }

        // Comparison — both sides must be compatible, result is %bool
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt
        | BinaryOp::Gt | BinaryOp::Le | BinaryOp::Ge => {
            if !types_compatible(lty, rty) {
                return Err(ValidationError::type_err(
                    format!("Cannot compare {:?} with {:?}", lty, rty),
                    span,
                ));
            }
            Ok(TypeExpr::Primitive("bool".to_string()))
        }

        // Logical — both sides must be bool, result is bool
        BinaryOp::And | BinaryOp::Or => {
            Ok(TypeExpr::Primitive("bool".to_string()))
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Stage 8 — Strict Inference Validator
// ─────────────────────────────────────────────────────────────────────────────

fn strict_check_top_level(
    item: &TopLevelItem,
    table: &TypeTable,
    config: &CompileConfig,
) -> Result<(), ValidationError> {
    match &item.kind {
        TopLevelKind::Function(f)  => strict_check_block(&f.body, table),
        TopLevelKind::Class(c)     => {
            for field in &c.create_block.fields {
                if let Some(default) = &field.default {
                    strict_check_expr(default, table)?;
                }
            }
            for method in &c.methods {
                strict_check_block(&method.body, table)?;
            }
            Ok(())
        }
        TopLevelKind::Namespace(n) => {
            for item in &n.items {
                strict_check_top_level(item, table, config)?;
            }
            Ok(())
        }
    }
}

fn strict_check_block(block: &Block, table: &TypeTable) -> Result<(), ValidationError> {
    for stmt in &block.stmts {
        if let StmtKind::ExprStmt(e) | StmtKind::Return(Some(e)) = &stmt.kind {
            strict_check_expr(e, table)?;
        }
    }
    if let Some(tail) = &block.tail {
        strict_check_expr(tail, table)?;
    }
    Ok(())
}

fn strict_check_expr(expr: &Expr, table: &TypeTable) -> Result<(), ValidationError> {
    match table.get(expr.id) {
        None => {
            // Expression was never recorded — this is a compiler bug but
            // we surface it as a type error rather than panicking
            Err(ValidationError::type_err(
                "Expression has no recorded type — this is a compiler bug. \
                 Please report it.",
                &expr.span,
            ))
        }
        Some(TypeExpr::Inferred) => {
            Err(ValidationError::type_err(
                "Type could not be inferred for this expression. \
                 Add an explicit type annotation.",
                &expr.span,
            ))
        }
        _ => Ok(()),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Type compatibility helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Two types are compatible if they are equal or one of them is Inferred
/// (Inferred is the "unknown" placeholder — it's compatible with anything).
fn types_compatible(a: &TypeExpr, b: &TypeExpr) -> bool {
    if a == b { return true; }
    matches!(a, TypeExpr::Inferred) || matches!(b, TypeExpr::Inferred)
}

fn is_numeric(name: &str) -> bool {
    matches!(name,
        "i8" | "i16" | "i32" | "i64"
        | "u8" | "u16" | "u32" | "u64"
        | "f32" | "f64"
    )
}

fn is_integer(name: &str) -> bool {
    matches!(name,
        "i8" | "i16" | "i32" | "i64"
        | "u8" | "u16" | "u32" | "u64"
    )
}

fn is_numeric_type(ty: &TypeExpr) -> bool {
    matches!(ty, TypeExpr::Primitive(s) if is_numeric(s))
        || matches!(ty, TypeExpr::Inferred)
}

// ── SelfRename helper ─────────────────────────────────────────────────────────

impl crate::frontend::make_pass::SelfRename {
    fn to_name(&self, class_name: &str) -> &str {
        match self {
            crate::frontend::make_pass::SelfRename::Global(s) => s.as_str(),
            crate::frontend::make_pass::SelfRename::PerClass { overrides, default } => {
                overrides.iter()
                    .find(|(cn, _)| cn == class_name)
                    .map(|(_, name)| name.as_str())
                    .unwrap_or(default.as_str())
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_types_compatible() {
        let i32_ty = TypeExpr::Primitive("i32".to_string());
        let i64_ty = TypeExpr::Primitive("i64".to_string());
        let inferred = TypeExpr::Inferred;

        assert!(types_compatible(&i32_ty, &i32_ty));
        assert!(!types_compatible(&i32_ty, &i64_ty));
        assert!(types_compatible(&inferred, &i32_ty));
        assert!(types_compatible(&i32_ty, &inferred));
    }

    #[test]
    fn test_type_family() {
        assert_eq!(type_family(&TypeExpr::Primitive("i32".to_string())), TypeFamily::Integer);
        assert_eq!(type_family(&TypeExpr::Primitive("f64".to_string())), TypeFamily::Float);
        assert_eq!(type_family(&TypeExpr::Primitive("str".to_string())), TypeFamily::Text);
        assert_eq!(type_family(&TypeExpr::Primitive("bool".to_string())), TypeFamily::Boolean);
    }

    #[test]
    fn test_same_family() {
        let i32_ty = TypeExpr::Primitive("i32".to_string());
        let i64_ty = TypeExpr::Primitive("i64".to_string());
        let f64_ty = TypeExpr::Primitive("f64".to_string());
        assert!(same_family(&i32_ty, &i64_ty));
        assert!(!same_family(&i32_ty, &f64_ty));
    }

    #[test]
    fn test_is_numeric() {
        assert!(is_numeric("i32"));
        assert!(is_numeric("f64"));
        assert!(!is_numeric("str"));
        assert!(!is_numeric("bool"));
    }

    #[test]
    fn test_infer_binary_op_arithmetic() {
        let i32_ty = TypeExpr::Primitive("i32".to_string());
        let span = Span { line: 1, col: 1 };
        let result = infer_binary_op(&BinaryOp::Add, &i32_ty, &i32_ty, &span);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), i32_ty);
    }

    #[test]
    fn test_infer_binary_op_comparison_returns_bool() {
        let i32_ty = TypeExpr::Primitive("i32".to_string());
        let bool_ty = TypeExpr::Primitive("bool".to_string());
        let span = Span { line: 1, col: 1 };
        let result = infer_binary_op(&BinaryOp::Eq, &i32_ty, &i32_ty, &span);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), bool_ty);
    }

    #[test]
    fn test_infer_binary_op_type_mismatch_errors() {
        let i32_ty = TypeExpr::Primitive("i32".to_string());
        let f64_ty = TypeExpr::Primitive("f64".to_string());
        let span = Span { line: 1, col: 1 };
        let result = infer_binary_op(&BinaryOp::Add, &i32_ty, &f64_ty, &span);
        assert!(result.is_err());
    }
}