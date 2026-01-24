//! AST to HIR Lowering
//!
//! Converts AST nodes to HIR with desugaring of complex constructs.
//!
//! ## Desugaring
//!
//! - `x += 1` → `x = x + 1`
//! - `x++` / `x--` → `x = x + 1` / `x = x - 1`
//! - `for x in iter` → while loop (basic structure)
//! - Range expressions → Range construction

use doo_core::{Span, types::{TypeId, TypeKind, TypeRegistry, builtin}};
use doo_frontend::ast::{
    self, Expr, ExprKind, Stmt, StmtKind, Program, Item,
    BinaryOp, UnaryOp, CompoundOp, IncDecOp,
    FunctionDecl, StructDecl, EnumDecl, ImportDecl, Decorator,
    TypeExpr, Pattern, PatternKind, ElseBranch,
};

use crate::types::*;

/// AST to HIR lowering context.
pub struct Lower {
    /// Collected errors during lowering.
    errors: Vec<LowerError>,
}

/// Lowering error.
#[derive(Debug, Clone)]
pub struct LowerError {
    pub message: String,
    pub span: Span,
}

impl LowerError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self { message: message.into(), span }
    }
}

impl Lower {
    /// Create a new lowering context.
    pub fn new() -> Self {
        Self { errors: Vec::new() }
    }

    /// Lower an entire program.
    pub fn lower_program(&mut self, program: &Program) -> HirProgram {
        let items = program.items.iter()
            .filter_map(|item| self.lower_item(item))
            .collect();
        
        HirProgram {
            items,
            span: program.span,
        }
    }

    pub fn lower_program_typed(
        &mut self,
        program: &Program,
        registry: &mut TypeRegistry,
    ) -> HirProgram {
        for item in &program.items {
            match item {
                Item::Struct(s) => {
                    registry.declare_named(&s.name);
                }
                Item::Enum(e) => {
                    registry.declare_named(&e.name);
                }
                _ => {}
            }
        }

        let items = program
            .items
            .iter()
            .filter_map(|item| self.lower_item_typed(item, registry))
            .collect();

        HirProgram {
            items,
            span: program.span,
        }
    }

    /// Get collected errors.
    pub fn errors(&self) -> &[LowerError] {
        &self.errors
    }

    /// Check if lowering had errors.
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    // ========================================================================
    // Items
    // ========================================================================

    fn lower_item(&mut self, item: &Item) -> Option<HirItem> {
        match item {
            Item::Function(f) => Some(HirItem::Function(self.lower_function(f))),
            Item::Struct(s) => Some(HirItem::Struct(self.lower_struct(s))),
            Item::Enum(e) => Some(HirItem::Enum(self.lower_enum(e))),
            Item::Import(i) => Some(HirItem::Import(self.lower_import(i))),
            Item::Statement(_stmt) => {
                // Top-level statements not supported in HIR yet
                None
            }
        }
    }

    fn lower_item_typed(&mut self, item: &Item, registry: &mut TypeRegistry) -> Option<HirItem> {
        match item {
            Item::Function(f) => Some(HirItem::Function(self.lower_function_typed(f, registry))),
            Item::Struct(s) => Some(HirItem::Struct(self.lower_struct_typed(s, registry))),
            Item::Enum(e) => Some(HirItem::Enum(self.lower_enum_typed(e, registry))),
            Item::Import(i) => Some(HirItem::Import(self.lower_import(i))),
            Item::Statement(_stmt) => {
                // Top-level statements not supported in HIR yet
                None
            }
        }
    }

    fn lower_function(&mut self, f: &FunctionDecl) -> HirFunction {
        let params = f.params.iter().map(|(name, _type_ann)| {
            HirParam {
                name: name.clone(),
                type_id: None, // Type resolution in later phase
                span: f.span,
            }
        }).collect();

        let body = f.body.iter()
            .map(|stmt| self.lower_stmt(stmt))
            .collect();

        let decorators = f.decorators.iter()
            .map(|d| self.lower_decorator(d))
            .collect();

        HirFunction {
            name: f.name.clone(),
            params,
            return_type: None,
            error_type: None,
            body,
            decorators,
            span: f.span,
        }
    }

    fn lower_function_typed(&mut self, f: &FunctionDecl, registry: &mut TypeRegistry) -> HirFunction {
        let params = f
            .params
            .iter()
            .map(|(name, type_ann)| HirParam {
                name: name.clone(),
                type_id: type_ann.as_ref().map(|t| self.resolve_type_expr(t, registry)),
                span: f.span,
            })
            .collect();

        let body = f.body.iter().map(|stmt| self.lower_stmt_typed(stmt, registry)).collect();

        let decorators = f.decorators.iter().map(|d| self.lower_decorator(d)).collect();

        HirFunction {
            name: f.name.clone(),
            params,
            return_type: f
                .return_type
                .as_ref()
                .map(|t| self.resolve_type_expr(t, registry)),
            error_type: f
                .error_type
                .as_ref()
                .map(|t| self.resolve_type_expr(t, registry)),
            body,
            decorators,
            span: f.span,
        }
    }

    fn lower_struct(&mut self, s: &StructDecl) -> HirStruct {
        let fields = s.fields.iter().map(|f| {
            HirField {
                name: f.name.clone(),
                type_id: None,
                is_optional: f.is_optional,
                default: f.default.as_ref().map(|e| self.lower_expr(e)),
                decorators: f.decorators.iter().map(|d| self.lower_decorator(d)).collect(),
                span: f.span,
            }
        }).collect();

        let decorators = s.decorators.iter()
            .map(|d| self.lower_decorator(d))
            .collect();

        HirStruct {
            name: s.name.clone(),
            fields,
            decorators,
            span: s.span,
        }
    }

    fn lower_struct_typed(&mut self, s: &StructDecl, registry: &mut TypeRegistry) -> HirStruct {
        let fields: Vec<HirField> = s
            .fields
            .iter()
            .map(|f| {
                let mut type_id = self.resolve_type_expr(&f.type_expr, registry);
                if f.is_optional {
                    let already_optional = registry
                        .get(type_id)
                        .map(|info| matches!(info.kind, TypeKind::Optional { .. }))
                        .unwrap_or(false);
                    if !already_optional {
                        type_id = registry.register_optional(type_id);
                    }
                }

                HirField {
                    name: f.name.clone(),
                    type_id: Some(type_id),
                    is_optional: f.is_optional,
                    default: f.default.as_ref().map(|e| self.lower_expr_typed(e, registry)),
                    decorators: f
                        .decorators
                        .iter()
                        .map(|d| self.lower_decorator(d))
                        .collect(),
                    span: f.span,
                }
            })
            .collect();

        registry.define_struct(
            &s.name,
            fields
                .iter()
                .filter_map(|f| f.type_id.map(|id| (f.name.clone(), id)))
                .collect(),
        );

        let decorators = s.decorators.iter().map(|d| self.lower_decorator(d)).collect();

        HirStruct {
            name: s.name.clone(),
            fields,
            decorators,
            span: s.span,
        }
    }

    fn lower_enum(&mut self, e: &EnumDecl) -> HirEnum {
        let variants = e.variants.iter().map(|v| {
            HirVariant {
                name: v.name.clone(),
                payload: None,
                span: v.span,
            }
        }).collect();

        HirEnum {
            name: e.name.clone(),
            variants,
            span: e.span,
        }
    }

    fn lower_enum_typed(&mut self, e: &EnumDecl, registry: &mut TypeRegistry) -> HirEnum {
        let variants: Vec<HirVariant> = e
            .variants
            .iter()
            .map(|v| HirVariant {
                name: v.name.clone(),
                payload: v
                    .payload
                    .as_ref()
                    .map(|t| self.resolve_type_expr(t, registry)),
                span: v.span,
            })
            .collect();

        registry.define_enum(
            &e.name,
            variants
                .iter()
                .map(|v| (v.name.clone(), v.payload))
                .collect(),
        );

        HirEnum {
            name: e.name.clone(),
            variants,
            span: e.span,
        }
    }

    fn lower_import(&mut self, i: &ImportDecl) -> HirImport {
        let items = i.items.iter().map(|item| {
            match item {
                ast::ImportItem::Symbol(s) => HirImportItem::Symbol(s.clone()),
                ast::ImportItem::Alias { name, alias } => {
                    HirImportItem::Alias { name: name.clone(), alias: alias.clone() }
                }
                ast::ImportItem::Wildcard => HirImportItem::Wildcard,
            }
        }).collect();

        HirImport {
            path: i.path.clone(),
            items,
            span: i.span,
        }
    }

    fn lower_decorator(&mut self, d: &Decorator) -> HirDecorator {
        HirDecorator {
            name: d.name.clone(),
            args: d.args.iter().map(|e| self.lower_expr(e)).collect(),
            span: d.span,
        }
    }

    // ========================================================================
    // Statements
    // ========================================================================

    fn lower_stmt(&mut self, stmt: &Stmt) -> HirStmt {
        let kind = match &stmt.kind {
            StmtKind::Let { mutable, pattern, type_ann: _, value } => {
                let name = self.pattern_to_name(pattern);
                HirStmtKind::Let {
                    name,
                    type_id: None,
                    value: self.lower_expr(value),
                    mutable: *mutable,
                    ownership: Ownership::Owned,
                }
            }

            StmtKind::Assign { target, value } => {
                let target_expr = self.pattern_to_expr(target);
                HirStmtKind::Assign {
                    target: target_expr,
                    value: self.lower_expr(value),
                }
            }

            // === Desugaring: Compound Assignment ===
            // `x += 1` → `x = x + 1`
            StmtKind::CompoundAssign { target, op, value } => {
                let target_expr = self.pattern_to_expr(target);
                let target_read = self.pattern_to_expr(target);
                let hir_op = self.compound_op_to_binop(*op);
                
                let binop_expr = HirExpr::new(
                    HirExprKind::BinOp {
                        op: hir_op,
                        lhs: Box::new(target_read),
                        rhs: Box::new(self.lower_expr(value)),
                    },
                    stmt.span,
                );

                HirStmtKind::Assign {
                    target: target_expr,
                    value: binop_expr,
                }
            }

            // === Desugaring: Increment/Decrement ===
            // `x++` → `x = x + 1`
            // `x--` → `x = x - 1`
            StmtKind::IncDec { variable, op } => {
                let target = HirExpr::new(
                    HirExprKind::Local { name: variable.clone() },
                    stmt.span,
                );
                let target_read = HirExpr::new(
                    HirExprKind::Local { name: variable.clone() },
                    stmt.span,
                );
                let one = HirExpr::new(
                    HirExprKind::Const(ConstValue::Int(1)),
                    stmt.span,
                );
                let hir_op = match op {
                    IncDecOp::Inc => HirBinOp::Add,
                    IncDecOp::Dec => HirBinOp::Sub,
                };

                let binop_expr = HirExpr::new(
                    HirExprKind::BinOp {
                        op: hir_op,
                        lhs: Box::new(target_read),
                        rhs: Box::new(one),
                    },
                    stmt.span,
                );

                HirStmtKind::Assign {
                    target,
                    value: binop_expr,
                }
            }

            StmtKind::Expr(expr) => {
                HirStmtKind::Expr(self.lower_expr(expr))
            }

            StmtKind::Return(values) => {
                HirStmtKind::Return(values.iter().map(|e| self.lower_expr(e)).collect())
            }

            StmtKind::Break => HirStmtKind::Break,
            StmtKind::Continue => HirStmtKind::Continue,

            StmtKind::If { condition, then_block, else_branch } => {
                let then_stmts = then_block.iter().map(|s| self.lower_stmt(s)).collect();
                let else_stmts = else_branch.as_ref().map(|eb| {
                    match eb {
                        ElseBranch::Block(stmts) => {
                            stmts.iter().map(|s| self.lower_stmt(s)).collect()
                        }
                        ElseBranch::ElseIf(if_stmt) => {
                            vec![self.lower_stmt(if_stmt)]
                        }
                    }
                });

                HirStmtKind::If {
                    condition: self.lower_expr(condition),
                    then_block: then_stmts,
                    else_block: else_stmts,
                }
            }

            // === Desugaring: For Loop ===
            // `for x in iter { body }` → `while iterator pattern { body }`
            // Simplified: just convert to while with iter expression
            StmtKind::For { pattern, iterable, body } => {
                let iter_name = self.pattern_to_name(pattern);
                
                // For now, we create a simple while-true with the body
                // Full iterator desugaring requires runtime support
                let body_stmts: Vec<_> = body.iter().map(|s| self.lower_stmt(s)).collect();
                
                // If there's an iterable, we'd need proper iterator protocol
                // For now, treat as conditional loop structure
                if let Some(iter_expr) = iterable {
                    // Create: let __iter = iterable; while __iter.has_next() { let x = __iter.next(); ... }
                    // Simplified: just preserve structure for now
                    HirStmtKind::While {
                        condition: self.lower_expr(iter_expr),
                        body: body_stmts,
                    }
                } else {
                    // Infinite loop: for { ... }
                    HirStmtKind::While {
                        condition: HirExpr::new(
                            HirExprKind::Const(ConstValue::Bool(true)),
                            stmt.span,
                        ),
                        body: body_stmts,
                    }
                }
            }

            StmtKind::Block(stmts) => {
                // Lower block as expression statement
                let lowered: Vec<_> = stmts.iter().map(|s| self.lower_stmt(s)).collect();
                if lowered.len() == 1 {
                    return lowered.into_iter().next().unwrap();
                }
                // Represent as nested block via expression
                HirStmtKind::Expr(HirExpr::new(
                    HirExprKind::Block { stmts: lowered, expr: None },
                    stmt.span,
                ))
            }

            StmtKind::Print(exprs) => {
                // Lower print as function call
                let args = exprs.iter().map(|e| self.lower_expr(e)).collect();
                HirStmtKind::Expr(HirExpr::new(
                    HirExprKind::Call {
                        func: Box::new(HirExpr::new(
                            HirExprKind::Global { name: "print".to_string() },
                            stmt.span,
                        )),
                        args,
                    },
                    stmt.span,
                ))
            }

            StmtKind::ElementAssign { array, index, value } => {
                // array[idx] = value → __array_set(array, idx, value)
                // For now, lower as method call
                HirStmtKind::Expr(HirExpr::new(
                    HirExprKind::MethodCall {
                        receiver: Box::new(self.lower_expr(array)),
                        method: "__set".to_string(),
                        args: vec![self.lower_expr(index), self.lower_expr(value)],
                    },
                    stmt.span,
                ))
            }

            StmtKind::FieldAssign { object, field, value } => {
                // obj.field = value → direct assignment
                let target = HirExpr::new(
                    HirExprKind::Field {
                        object: Box::new(self.lower_expr(object)),
                        field: field.clone(),
                    },
                    stmt.span,
                );
                HirStmtKind::Assign {
                    target,
                    value: self.lower_expr(value),
                }
            }

            StmtKind::ManualErrorExtract { expr, ok_pattern, error_var } => {
                let ok_names = self.pattern_to_names(ok_pattern);
                HirStmtKind::ManualErrorExtract {
                    ok_names,
                    error_name: error_var.clone(),
                    expr: self.lower_expr(expr),
                }
            }
        };

        HirStmt::new(kind, stmt.span)
    }

    fn lower_stmt_typed(&mut self, stmt: &Stmt, registry: &mut TypeRegistry) -> HirStmt {
        let kind = match &stmt.kind {
            StmtKind::Let {
                mutable,
                pattern,
                type_ann,
                value,
            } => {
                let name = self.pattern_to_name(pattern);
                let mut value_hir = self.lower_expr_typed(value, registry);
                let annotated_type_id = type_ann.as_ref().map(|t| self.resolve_type_expr(t, registry));
                let inferred_type_id = annotated_type_id.or(value_hir.type_id);
                if annotated_type_id.is_some() {
                    value_hir.type_id = inferred_type_id;
                }
                HirStmtKind::Let {
                    name,
                    type_id: inferred_type_id,
                    value: value_hir,
                    mutable: *mutable,
                    ownership: Ownership::Owned,
                }
            }

            StmtKind::Assign { target, value } => {
                let target_expr = self.pattern_to_expr(target);
                HirStmtKind::Assign {
                    target: target_expr,
                    value: self.lower_expr_typed(value, registry),
                }
            }

            StmtKind::CompoundAssign { target, op, value } => {
                let target_expr = self.pattern_to_expr(target);
                let target_read = self.pattern_to_expr(target);
                let hir_op = self.compound_op_to_binop(*op);

                let binop_expr = HirExpr::new(
                    HirExprKind::BinOp {
                        op: hir_op,
                        lhs: Box::new(target_read),
                        rhs: Box::new(self.lower_expr_typed(value, registry)),
                    },
                    stmt.span,
                );

                HirStmtKind::Assign {
                    target: target_expr,
                    value: binop_expr,
                }
            }

            StmtKind::IncDec { variable, op } => {
                let target = HirExpr::new(
                    HirExprKind::Local { name: variable.clone() },
                    stmt.span,
                );
                let target_read = HirExpr::new(
                    HirExprKind::Local { name: variable.clone() },
                    stmt.span,
                );
                let one = HirExpr::with_type(
                    HirExprKind::Const(ConstValue::Int(1)),
                    builtin::INT,
                    stmt.span,
                );
                let hir_op = match op {
                    IncDecOp::Inc => HirBinOp::Add,
                    IncDecOp::Dec => HirBinOp::Sub,
                };

                let binop_expr = HirExpr::new(
                    HirExprKind::BinOp {
                        op: hir_op,
                        lhs: Box::new(target_read),
                        rhs: Box::new(one),
                    },
                    stmt.span,
                );

                HirStmtKind::Assign {
                    target,
                    value: binop_expr,
                }
            }

            StmtKind::Expr(expr) => HirStmtKind::Expr(self.lower_expr_typed(expr, registry)),

            StmtKind::Return(values) => {
                HirStmtKind::Return(values.iter().map(|e| self.lower_expr_typed(e, registry)).collect())
            }

            StmtKind::Break => HirStmtKind::Break,
            StmtKind::Continue => HirStmtKind::Continue,

            StmtKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                let then_stmts = then_block
                    .iter()
                    .map(|s| self.lower_stmt_typed(s, registry))
                    .collect();
                let else_stmts = else_branch.as_ref().map(|eb| match eb {
                    ElseBranch::Block(stmts) => stmts
                        .iter()
                        .map(|s| self.lower_stmt_typed(s, registry))
                        .collect(),
                    ElseBranch::ElseIf(if_stmt) => vec![self.lower_stmt_typed(if_stmt, registry)],
                });

                HirStmtKind::If {
                    condition: self.lower_expr_typed(condition, registry),
                    then_block: then_stmts,
                    else_block: else_stmts,
                }
            }

            StmtKind::For {
                pattern,
                iterable,
                body,
            } => {
                let _iter_name = self.pattern_to_name(pattern);

                let body_stmts: Vec<_> = body
                    .iter()
                    .map(|s| self.lower_stmt_typed(s, registry))
                    .collect();

                if let Some(iter_expr) = iterable {
                    HirStmtKind::While {
                        condition: self.lower_expr_typed(iter_expr, registry),
                        body: body_stmts,
                    }
                } else {
                    HirStmtKind::While {
                        condition: HirExpr::with_type(
                            HirExprKind::Const(ConstValue::Bool(true)),
                            builtin::BOOL,
                            stmt.span,
                        ),
                        body: body_stmts,
                    }
                }
            }

            StmtKind::Block(stmts) => {
                let lowered: Vec<_> = stmts
                    .iter()
                    .map(|s| self.lower_stmt_typed(s, registry))
                    .collect();
                if lowered.len() == 1 {
                    return lowered.into_iter().next().unwrap();
                }
                HirStmtKind::Expr(HirExpr::new(
                    HirExprKind::Block { stmts: lowered, expr: None },
                    stmt.span,
                ))
            }

            StmtKind::Print(exprs) => {
                let args = exprs.iter().map(|e| self.lower_expr_typed(e, registry)).collect();
                HirStmtKind::Expr(HirExpr::new(
                    HirExprKind::Call {
                        func: Box::new(HirExpr::new(
                            HirExprKind::Global { name: "print".to_string() },
                            stmt.span,
                        )),
                        args,
                    },
                    stmt.span,
                ))
            }

            StmtKind::ElementAssign { array, index, value } => HirStmtKind::Expr(HirExpr::new(
                HirExprKind::MethodCall {
                    receiver: Box::new(self.lower_expr_typed(array, registry)),
                    method: "__set".to_string(),
                    args: vec![
                        self.lower_expr_typed(index, registry),
                        self.lower_expr_typed(value, registry),
                    ],
                },
                stmt.span,
            )),

            StmtKind::FieldAssign { object, field, value } => {
                let target = HirExpr::new(
                    HirExprKind::Field {
                        object: Box::new(self.lower_expr_typed(object, registry)),
                        field: field.clone(),
                    },
                    stmt.span,
                );
                HirStmtKind::Assign {
                    target,
                    value: self.lower_expr_typed(value, registry),
                }
            }

            StmtKind::ManualErrorExtract {
                expr,
                ok_pattern,
                error_var,
            } => {
                let ok_names = self.pattern_to_names(ok_pattern);
                HirStmtKind::ManualErrorExtract {
                    ok_names,
                    error_name: error_var.clone(),
                    expr: self.lower_expr_typed(expr, registry),
                }
            }
        };

        HirStmt::new(kind, stmt.span)
    }

    // ========================================================================
    // Expressions
    // ========================================================================

    fn lower_expr(&mut self, expr: &Expr) -> HirExpr {
        let kind = match &expr.kind {
            ExprKind::IntLit(v) => HirExprKind::Const(ConstValue::Int(*v)),
            ExprKind::FloatLit(v) => HirExprKind::Const(ConstValue::Float(*v)),
            ExprKind::BoolLit(v) => HirExprKind::Const(ConstValue::Bool(*v)),
            ExprKind::StrLit(v) => HirExprKind::Const(ConstValue::Str(v.clone())),
            ExprKind::Nil => HirExprKind::Const(ConstValue::Nil),

            ExprKind::Ident(name) => HirExprKind::Local { name: name.clone() },

            ExprKind::Binary { left, op, right } => {
                HirExprKind::BinOp {
                    op: self.lower_binop(*op),
                    lhs: Box::new(self.lower_expr(left)),
                    rhs: Box::new(self.lower_expr(right)),
                }
            }

            ExprKind::Unary { op, expr: inner } => {
                HirExprKind::UnaryOp {
                    op: self.lower_unaryop(*op),
                    operand: Box::new(self.lower_expr(inner)),
                }
            }

            ExprKind::Call { func, args } => {
                HirExprKind::Call {
                    func: Box::new(self.lower_expr(func)),
                    args: args.iter().map(|a| self.lower_expr(a)).collect(),
                }
            }

            ExprKind::MethodCall { object, method, args } => {
                HirExprKind::MethodCall {
                    receiver: Box::new(self.lower_expr(object)),
                    method: method.clone(),
                    args: args.iter().map(|a| self.lower_expr(a)).collect(),
                }
            }

            ExprKind::Field { object, field } => {
                HirExprKind::Field {
                    object: Box::new(self.lower_expr(object)),
                    field: field.clone(),
                }
            }

            ExprKind::Index { object, index } => {
                HirExprKind::Index {
                    object: Box::new(self.lower_expr(object)),
                    index: Box::new(self.lower_expr(index)),
                }
            }

            ExprKind::ArrayLit(elements) => {
                HirExprKind::Array(elements.iter().map(|e| self.lower_expr(e)).collect())
            }

            ExprKind::MapLit(entries) => {
                HirExprKind::Map(
                    entries
                        .iter()
                        .map(|(k, v)| (self.lower_expr(k), self.lower_expr(v)))
                        .collect(),
                )
            }

            ExprKind::TupleLit(elements) => {
                HirExprKind::Tuple(elements.iter().map(|e| self.lower_expr(e)).collect())
            }

            ExprKind::ObjectLit(fields) | ExprKind::StructLit { fields, .. } => {
                let name = if let ExprKind::StructLit { name, .. } = &expr.kind {
                    name.clone()
                } else {
                    "__anon".to_string()
                };
                HirExprKind::Struct {
                    name,
                    fields: fields.iter().map(|(k, v)| (k.clone(), self.lower_expr(v))).collect(),
                }
            }

            ExprKind::EnumVariant { enum_name, variant, payload } => {
                HirExprKind::EnumVariant {
                    enum_name: enum_name.clone(),
                    variant: variant.clone(),
                    payload: payload.iter().map(|e| self.lower_expr(e)).collect(),
                }
            }

            ExprKind::Range { start, end, inclusive } => {
                HirExprKind::Range {
                    start: Box::new(self.lower_expr(start)),
                    end: Box::new(self.lower_expr(end)),
                    inclusive: *inclusive,
                }
            }

            ExprKind::IfExpr { condition, then_branch, else_branch } => {
                HirExprKind::If {
                    condition: Box::new(self.lower_expr(condition)),
                    then_expr: Box::new(self.lower_expr(then_branch)),
                    else_expr: else_branch.as_ref().map(|e| Box::new(self.lower_expr(e))),
                }
            }

            ExprKind::Block(stmts, final_expr) => {
                HirExprKind::Block {
                    stmts: stmts.iter().map(|s| self.lower_stmt(s)).collect(),
                    expr: final_expr.as_ref().map(|e| Box::new(self.lower_expr(e))),
                }
            }

            ExprKind::Ok(values) => {
                let inner = if values.len() == 1 {
                    self.lower_expr(&values[0])
                } else {
                    HirExpr::new(
                        HirExprKind::Tuple(values.iter().map(|e| self.lower_expr(e)).collect()),
                        expr.span,
                    )
                };
                HirExprKind::Ok(Box::new(inner))
            }

            ExprKind::Err(inner) => {
                HirExprKind::Err(Box::new(self.lower_expr(inner)))
            }

            ExprKind::Try(inner) => {
                HirExprKind::Try(Box::new(self.lower_expr(inner)))
            }

             ExprKind::UnwrapOrPanic { expr: inner, message } => {
                 HirExprKind::UnwrapOrPanic {
                     expr: Box::new(self.lower_expr(inner)),
                     message: Box::new(self.lower_expr(message)),
                 }
             }

            ExprKind::Closure { params, body, .. } => {
                HirExprKind::Closure {
                    params: params.iter().map(|(n, _)| (n.clone(), None)).collect(),
                    body: Box::new(self.lower_expr(body)),
                }
            }

            ExprKind::Match { values, arms } => {
                HirExprKind::Match {
                    values: values.iter().map(|v| self.lower_expr(v)).collect(),
                    arms: arms.iter().map(|a| HirMatchArm {
                        pattern: self.lower_match_pattern(&a.pattern),
                        guard: a.guard.as_ref().map(|g| self.lower_expr(g)),
                        body: self.lower_expr(&a.body),
                        span: a.span,
                    }).collect(),
                }
            }

            ExprKind::Spread(inner) => HirExprKind::Spread(Box::new(self.lower_expr(inner))),

            ExprKind::StringInterpolation(parts) => {
                // Desugar: "a ${b} c" -> "a" + (b as Str) + "c"
                if parts.is_empty() {
                    HirExprKind::Const(ConstValue::Str(String::new()))
                } else {
                    let mut current = self.lower_string_part(&parts[0]);
                    for part in &parts[1..] {
                        let next = self.lower_string_part(part);
                        current = HirExpr::new(
                            HirExprKind::BinOp {
                                op: HirBinOp::Add,
                                lhs: Box::new(current),
                                rhs: Box::new(next),
                            },
                            expr.span,
                        );
                    }
                    current.kind
                }
            }

            ExprKind::Ternary { .. } | ExprKind::Cast { .. } => {
                // Defer complex constructs
                self.errors.push(LowerError::new(
                    "Complex expression not yet lowered",
                    expr.span,
                ));
                HirExprKind::Const(ConstValue::Nil)
            }
        };

        let mut out = HirExpr::new(kind, expr.span);
        if let HirExprKind::Const(c) = &out.kind {
            out.type_id = Some(c.type_id());
        }
        out
    }

    fn lower_expr_typed(&mut self, expr: &Expr, registry: &mut TypeRegistry) -> HirExpr {
        let kind = match &expr.kind {
            ExprKind::IntLit(v) => HirExprKind::Const(ConstValue::Int(*v)),
            ExprKind::FloatLit(v) => HirExprKind::Const(ConstValue::Float(*v)),
            ExprKind::BoolLit(v) => HirExprKind::Const(ConstValue::Bool(*v)),
            ExprKind::StrLit(v) => HirExprKind::Const(ConstValue::Str(v.clone())),
            ExprKind::Nil => HirExprKind::Const(ConstValue::Nil),

            ExprKind::Ident(name) => HirExprKind::Local { name: name.clone() },

            ExprKind::Binary { left, op, right } => HirExprKind::BinOp {
                op: self.lower_binop(*op),
                lhs: Box::new(self.lower_expr_typed(left, registry)),
                rhs: Box::new(self.lower_expr_typed(right, registry)),
            },

            ExprKind::Unary { op, expr: inner } => HirExprKind::UnaryOp {
                op: self.lower_unaryop(*op),
                operand: Box::new(self.lower_expr_typed(inner, registry)),
            },

            ExprKind::Call { func, args } => HirExprKind::Call {
                func: Box::new(self.lower_expr_typed(func, registry)),
                args: args.iter().map(|a| self.lower_expr_typed(a, registry)).collect(),
            },

            ExprKind::MethodCall { object, method, args } => HirExprKind::MethodCall {
                receiver: Box::new(self.lower_expr_typed(object, registry)),
                method: method.clone(),
                args: args.iter().map(|a| self.lower_expr_typed(a, registry)).collect(),
            },

            ExprKind::Field { object, field } => HirExprKind::Field {
                object: Box::new(self.lower_expr_typed(object, registry)),
                field: field.clone(),
            },

            ExprKind::Index { object, index } => HirExprKind::Index {
                object: Box::new(self.lower_expr_typed(object, registry)),
                index: Box::new(self.lower_expr_typed(index, registry)),
            },

            ExprKind::ArrayLit(elements) => {
                HirExprKind::Array(elements.iter().map(|e| self.lower_expr_typed(e, registry)).collect())
            }

            ExprKind::MapLit(entries) => HirExprKind::Map(
                entries
                    .iter()
                    .map(|(k, v)| (self.lower_expr_typed(k, registry), self.lower_expr_typed(v, registry)))
                    .collect(),
            ),

            ExprKind::TupleLit(elements) => {
                HirExprKind::Tuple(elements.iter().map(|e| self.lower_expr_typed(e, registry)).collect())
            }

            ExprKind::ObjectLit(fields) | ExprKind::StructLit { fields, .. } => {
                let name = if let ExprKind::StructLit { name, .. } = &expr.kind {
                    name.clone()
                } else {
                    "__anon".to_string()
                };
                HirExprKind::Struct {
                    name,
                    fields: fields
                        .iter()
                        .map(|(k, v)| (k.clone(), self.lower_expr_typed(v, registry)))
                        .collect(),
                }
            }

            ExprKind::EnumVariant {
                enum_name,
                variant,
                payload,
            } => HirExprKind::EnumVariant {
                enum_name: enum_name.clone(),
                variant: variant.clone(),
                payload: payload.iter().map(|e| self.lower_expr_typed(e, registry)).collect(),
            },

            ExprKind::Range { start, end, inclusive } => HirExprKind::Range {
                start: Box::new(self.lower_expr_typed(start, registry)),
                end: Box::new(self.lower_expr_typed(end, registry)),
                inclusive: *inclusive,
            },

            ExprKind::IfExpr {
                condition,
                then_branch,
                else_branch,
            } => HirExprKind::If {
                condition: Box::new(self.lower_expr_typed(condition, registry)),
                then_expr: Box::new(self.lower_expr_typed(then_branch, registry)),
                else_expr: else_branch
                    .as_ref()
                    .map(|e| Box::new(self.lower_expr_typed(e, registry))),
            },

            ExprKind::Block(stmts, final_expr) => HirExprKind::Block {
                stmts: stmts.iter().map(|s| self.lower_stmt_typed(s, registry)).collect(),
                expr: final_expr
                    .as_ref()
                    .map(|e| Box::new(self.lower_expr_typed(e, registry))),
            },

            ExprKind::Ok(values) => {
                let inner = if values.len() == 1 {
                    self.lower_expr_typed(&values[0], registry)
                } else {
                    HirExpr::new(
                        HirExprKind::Tuple(values.iter().map(|e| self.lower_expr_typed(e, registry)).collect()),
                        expr.span,
                    )
                };
                HirExprKind::Ok(Box::new(inner))
            }

            ExprKind::Err(inner) => HirExprKind::Err(Box::new(self.lower_expr_typed(inner, registry))),
            ExprKind::Try(inner) => HirExprKind::Try(Box::new(self.lower_expr_typed(inner, registry))),

            ExprKind::UnwrapOrPanic { expr: inner, message } => HirExprKind::UnwrapOrPanic {
                expr: Box::new(self.lower_expr_typed(inner, registry)),
                message: Box::new(self.lower_expr_typed(message, registry)),
            },

            ExprKind::Closure { params, body, return_type, .. } => {
                let mut body_hir = self.lower_expr_typed(body, registry);
                if let Some(ret_type) = return_type {
                    body_hir.type_id = Some(self.resolve_type_expr(ret_type, registry));
                }
                HirExprKind::Closure {
                    params: params
                        .iter()
                        .map(|(n, t)| (n.clone(), t.as_ref().map(|tt| self.resolve_type_expr(tt, registry))))
                        .collect(),
                    body: Box::new(body_hir),
                }
            }

            ExprKind::Match { values, arms } => HirExprKind::Match {
                values: values.iter().map(|v| self.lower_expr_typed(v, registry)).collect(),
                arms: arms
                    .iter()
                    .map(|a| HirMatchArm {
                        pattern: self.lower_match_pattern_typed(&a.pattern, registry),
                        guard: a.guard.as_ref().map(|g| self.lower_expr_typed(g, registry)),
                        body: self.lower_expr_typed(&a.body, registry),
                        span: a.span,
                    })
                    .collect(),
            },

            ExprKind::Spread(inner) => HirExprKind::Spread(Box::new(self.lower_expr_typed(inner, registry))),

            ExprKind::StringInterpolation(parts) => {
                if parts.is_empty() {
                    HirExprKind::Const(ConstValue::Str(String::new()))
                } else {
                    let mut current = self.lower_string_part_typed(&parts[0], registry);
                    for part in &parts[1..] {
                        let next = self.lower_string_part_typed(part, registry);
                        current = HirExpr::new(
                            HirExprKind::BinOp {
                                op: HirBinOp::Add,
                                lhs: Box::new(current),
                                rhs: Box::new(next),
                            },
                            expr.span,
                        );
                        current.type_id = Some(builtin::STR);
                    }
                    current.kind
                }
            }

            ExprKind::Cast { expr: inner, target } => {
                let inner_hir = self.lower_expr_typed(inner, registry);
                let target_type = self.resolve_type_expr(target, registry);
                HirExprKind::Cast {
                    value: Box::new(inner_hir),
                    to_type: target_type,
                }
            }

            ExprKind::Ternary { .. } => {
                self.errors.push(LowerError::new(
                    "Complex expression not yet lowered",
                    expr.span,
                ));
                HirExprKind::Const(ConstValue::Nil)
            }
        };

        let mut out = HirExpr::new(kind, expr.span);
        if let HirExprKind::Const(c) = &out.kind {
            out.type_id = Some(c.type_id());
        }

        match &mut out.kind {
            HirExprKind::Array(elements) => {
                let elem_type = self.common_type_or_any(elements);
                out.type_id = Some(registry.register_array(elem_type));
            }
            HirExprKind::Map(entries) => {
                let keys: Vec<HirExpr> = entries.iter().map(|(k, _)| k.clone()).collect();
                let vals: Vec<HirExpr> = entries.iter().map(|(_, v)| v.clone()).collect();
                let key_type = self.common_type_or_any(&keys);
                let val_type = self.common_type_or_any(&vals);
                out.type_id = Some(registry.register_map(key_type, val_type));
            }
            HirExprKind::Tuple(elements) => {
                let element_types: Vec<TypeId> = elements
                    .iter()
                    .map(|e| e.type_id.unwrap_or(builtin::ANY))
                    .collect();
                out.type_id = Some(registry.register_tuple(element_types));
            }
            HirExprKind::Struct { name, .. } => {
                out.type_id = Some(registry.lookup(name).unwrap_or_else(|| registry.declare_named(name)));
            }
            HirExprKind::EnumVariant { enum_name, .. } => {
                out.type_id = Some(
                    registry
                        .lookup(enum_name)
                        .unwrap_or_else(|| registry.declare_named(enum_name)),
                );
            }
            HirExprKind::Closure { params, body } => {
                let param_types: Vec<TypeId> = params
                    .iter()
                    .map(|(_, t)| t.unwrap_or(builtin::ANY))
                    .collect();
                let return_type = body.type_id.unwrap_or(builtin::ANY);
                out.type_id = Some(registry.register_function(param_types, return_type));
            }
            HirExprKind::MethodCall { receiver, method, args } => {
                let receiver_type = receiver.type_id.unwrap_or(builtin::ANY);
                if let Some(return_type) =
                    self.infer_method_call_type(receiver_type, method, args, registry)
                {
                    out.type_id = Some(return_type);
                }
            }
            _ => {}
        }

        out
    }

    // ========================================================================
    // Helpers
    // ========================================================================

    fn infer_method_call_type(
        &mut self,
        receiver_type: TypeId,
        method: &str,
        args: &mut [HirExpr],
        registry: &mut TypeRegistry,
    ) -> Option<TypeId> {
        let receiver_info = registry.get(receiver_type)?;
        match &receiver_info.kind {
            TypeKind::Array { element } => {
                self.infer_array_method_type(*element, method, args, registry)
            }
            _ => None,
        }
    }

    fn infer_array_method_type(
        &mut self,
        elem_type: TypeId,
        method: &str,
        args: &mut [HirExpr],
        registry: &mut TypeRegistry,
    ) -> Option<TypeId> {
        match method {
            "len" | "indexOf" => Some(builtin::INT),
            "isEmpty" | "contains" => Some(builtin::BOOL),
            "join" => Some(builtin::STR),
            "first" | "last" | "pop" => Some(elem_type),
            "slice" => Some(registry.register_array(elem_type)),
            "push" | "clear" | "sort" | "reverse" => Some(builtin::VOID),
            "map" => {
                let closure_return = args
                    .get_mut(0)
                    .and_then(|arg| self.apply_closure_signature(arg, &[elem_type], None, registry));
                let out_elem = closure_return.unwrap_or(builtin::ANY);
                Some(registry.register_array(out_elem))
            }
            "filter" => {
                let _ = args
                    .get_mut(0)
                    .and_then(|arg| {
                        self.apply_closure_signature(arg, &[elem_type], Some(builtin::BOOL), registry)
                    });
                Some(registry.register_array(elem_type))
            }
            "reduce" => {
                let init_type = args
                    .get(0)
                    .and_then(|arg| arg.type_id)
                    .unwrap_or(builtin::ANY);
                let closure_return = args
                    .get_mut(1)
                    .and_then(|arg| {
                        self.apply_closure_signature(
                            arg,
                            &[init_type, elem_type],
                            Some(init_type),
                            registry,
                        )
                    });
                if init_type != builtin::ANY {
                    Some(init_type)
                } else {
                    Some(closure_return.unwrap_or(builtin::ANY))
                }
            }
            _ => None,
        }
    }

    fn apply_closure_signature(
        &mut self,
        expr: &mut HirExpr,
        param_types: &[TypeId],
        return_type_hint: Option<TypeId>,
        registry: &mut TypeRegistry,
    ) -> Option<TypeId> {
        match &mut expr.kind {
            HirExprKind::Closure { params, body } => {
                for (idx, (_, param_type)) in params.iter_mut().enumerate() {
                    if param_type.is_none() {
                        if let Some(ty) = param_types.get(idx) {
                            *param_type = Some(*ty);
                        }
                    }
                }

                if body.type_id.is_none() {
                    if let Some(ret) = return_type_hint {
                        body.type_id = Some(ret);
                    }
                }

                let param_ids: Vec<TypeId> = params
                    .iter()
                    .map(|(_, t)| t.unwrap_or(builtin::ANY))
                    .collect();
                let return_type = body.type_id.unwrap_or(builtin::ANY);
                expr.type_id = Some(registry.register_function(param_ids, return_type));
                Some(return_type)
            }
            _ => expr
                .type_id
                .and_then(|type_id| match registry.get(type_id) {
                    Some(info) => match info.kind {
                        TypeKind::Function { returns, .. } => Some(returns),
                        _ => None,
                    },
                    None => None,
                }),
        }
    }

    fn pattern_to_name(&self, pattern: &Pattern) -> String {
        match &pattern.kind {
            PatternKind::Ident(name) => name.clone(),
            PatternKind::Wildcard => "_".to_string(),
            PatternKind::Tuple(_) => "__tuple".to_string(),
        }
    }

    fn pattern_to_expr(&mut self, pattern: &Pattern) -> HirExpr {
        match &pattern.kind {
            PatternKind::Ident(name) => {
                HirExpr::new(HirExprKind::Local { name: name.clone() }, pattern.span)
            }
            PatternKind::Wildcard => {
                HirExpr::new(HirExprKind::Local { name: "_".to_string() }, pattern.span)
            }
            PatternKind::Tuple(patterns) => {
                let exprs = patterns.iter().map(|p| self.pattern_to_expr(p)).collect();
                HirExpr::new(HirExprKind::Tuple(exprs), pattern.span)
            }
        }
    }

     fn pattern_to_names(&self, pattern: &Pattern) -> Vec<String> {
         match &pattern.kind {
             PatternKind::Tuple(patterns) => patterns.iter().map(|p| self.pattern_to_name(p)).collect(),
             _ => vec![self.pattern_to_name(pattern)],
         }
     }

    fn lower_binop(&self, op: BinaryOp) -> HirBinOp {
        match op {
            BinaryOp::Add => HirBinOp::Add,
            BinaryOp::Sub => HirBinOp::Sub,
            BinaryOp::Mul => HirBinOp::Mul,
            BinaryOp::Div => HirBinOp::Div,
            BinaryOp::Mod => HirBinOp::Mod,
            BinaryOp::Eq => HirBinOp::Eq,
            BinaryOp::NotEq => HirBinOp::NotEq,
            BinaryOp::Lt => HirBinOp::Lt,
            BinaryOp::Gt => HirBinOp::Gt,
            BinaryOp::LtEq => HirBinOp::LtEq,
            BinaryOp::GtEq => HirBinOp::GtEq,
            BinaryOp::And => HirBinOp::And,
            BinaryOp::Or => HirBinOp::Or,
            BinaryOp::BitAnd => HirBinOp::BitAnd,
            BinaryOp::BitOr => HirBinOp::BitOr,
            BinaryOp::NullCoalesce => HirBinOp::Or, // Simplify for now
            BinaryOp::In => HirBinOp::In,
        }
    }

    fn lower_match_pattern(&mut self, p: &ast::MatchPattern) -> HirMatchPattern {
        match p {
            ast::MatchPattern::Literal(e) => HirMatchPattern::Literal(Box::new(self.lower_expr(e))),
            ast::MatchPattern::Condition(e) => HirMatchPattern::Condition(Box::new(self.lower_expr(e))),
            ast::MatchPattern::Wildcard => HirMatchPattern::Wildcard,
            ast::MatchPattern::EnumVariant { enum_name, variant } => {
                HirMatchPattern::EnumVariant {
                    enum_name: enum_name.clone(),
                    variant: variant.clone(),
                }
            }
            ast::MatchPattern::EnumVariantPayload { enum_name, variant, bindings } => {
                HirMatchPattern::EnumVariantPayload {
                    enum_name: enum_name.clone(),
                    variant: variant.clone(),
                    bindings: bindings.clone(),
                }
            }
            ast::MatchPattern::Tuple(parts) => {
                HirMatchPattern::Tuple(parts.iter().map(|x| self.lower_match_pattern(x)).collect())
            }
        }
    }

    fn lower_match_pattern_typed(
        &mut self,
        p: &ast::MatchPattern,
        registry: &mut TypeRegistry,
    ) -> HirMatchPattern {
        match p {
            ast::MatchPattern::Literal(e) => {
                HirMatchPattern::Literal(Box::new(self.lower_expr_typed(e, registry)))
            }
            ast::MatchPattern::Condition(e) => {
                HirMatchPattern::Condition(Box::new(self.lower_expr_typed(e, registry)))
            }
            ast::MatchPattern::Wildcard => HirMatchPattern::Wildcard,
            ast::MatchPattern::EnumVariant { enum_name, variant } => HirMatchPattern::EnumVariant {
                enum_name: enum_name.clone(),
                variant: variant.clone(),
            },
            ast::MatchPattern::EnumVariantPayload {
                enum_name,
                variant,
                bindings,
            } => HirMatchPattern::EnumVariantPayload {
                enum_name: enum_name.clone(),
                variant: variant.clone(),
                bindings: bindings.clone(),
            },
            ast::MatchPattern::Tuple(parts) => HirMatchPattern::Tuple(
                parts
                    .iter()
                    .map(|x| self.lower_match_pattern_typed(x, registry))
                    .collect(),
            ),
        }
    }

    fn resolve_type_expr(&mut self, ty: &TypeExpr, registry: &mut TypeRegistry) -> TypeId {
        match &ty.kind {
            doo_frontend::ast::TypeExprKind::Named(name) => registry.lookup(name).unwrap_or_else(|| registry.declare_named(name)),
            doo_frontend::ast::TypeExprKind::Array(inner) => {
                let elem = self.resolve_type_expr(inner, registry);
                registry.register_array(elem)
            }
            doo_frontend::ast::TypeExprKind::Map(k, v) => {
                let key = self.resolve_type_expr(k, registry);
                let value = self.resolve_type_expr(v, registry);
                registry.register_map(key, value)
            }
            doo_frontend::ast::TypeExprKind::Tuple(parts) => {
                let elements = parts.iter().map(|p| self.resolve_type_expr(p, registry)).collect();
                registry.register_tuple(elements)
            }
            doo_frontend::ast::TypeExprKind::Optional(inner) => {
                let inner_id = self.resolve_type_expr(inner, registry);
                registry.register_optional(inner_id)
            }
            doo_frontend::ast::TypeExprKind::Result(ok, err) => {
                let ok_id = self.resolve_type_expr(ok, registry);
                let err_id = self.resolve_type_expr(err, registry);
                registry.register_result(ok_id, err_id)
            }
            doo_frontend::ast::TypeExprKind::Function { params, returns } => {
                let params_ids = params.iter().map(|p| self.resolve_type_expr(p, registry)).collect();
                let returns_id = self.resolve_type_expr(returns, registry);
                registry.register_function(params_ids, returns_id)
            }
            doo_frontend::ast::TypeExprKind::Range(_inner) => registry.declare_named("Range"),
            doo_frontend::ast::TypeExprKind::Any => builtin::ANY,
            doo_frontend::ast::TypeExprKind::Void => builtin::VOID,
            doo_frontend::ast::TypeExprKind::Error => builtin::ERROR,
        }
    }

    fn common_type_or_any(&self, exprs: &[HirExpr]) -> TypeId {
        let mut current: Option<TypeId> = None;
        for e in exprs {
            let Some(t) = e.type_id else {
                return builtin::ANY;
            };
            match current {
                None => current = Some(t),
                Some(existing) if existing == t => {}
                Some(_) => return builtin::ANY,
            }
        }
        current.unwrap_or(builtin::ANY)
    }

    fn lower_unaryop(&self, op: UnaryOp) -> HirUnaryOp {
        match op {
            UnaryOp::Neg => HirUnaryOp::Neg,
            UnaryOp::Not => HirUnaryOp::Not,
        }
    }

    fn compound_op_to_binop(&self, op: CompoundOp) -> HirBinOp {
        match op {
            CompoundOp::Add => HirBinOp::Add,
            CompoundOp::Sub => HirBinOp::Sub,
            CompoundOp::Mul => HirBinOp::Mul,
            CompoundOp::Div => HirBinOp::Div,
            CompoundOp::Mod => HirBinOp::Mod,
        }
    }

    fn lower_string_part(&mut self, part: &ast::StringPart) -> HirExpr {
        match part {
            ast::StringPart::Literal(s) => HirExpr::new(
                HirExprKind::Const(ConstValue::Str(s.clone())),
                Span::dummy(),
            ),
            ast::StringPart::Expr(e) => {
                // Should cast to string, but Cast not fully lowered in untyped pass.
                // Assuming untyped pass relies on implicit behavior or basic lowering.
                self.lower_expr(e)
            }
        }
    }

    fn lower_string_part_typed(
        &mut self,
        part: &ast::StringPart,
        registry: &mut TypeRegistry,
    ) -> HirExpr {
        match part {
            ast::StringPart::Literal(s) => HirExpr::with_type(
                HirExprKind::Const(ConstValue::Str(s.clone())),
                builtin::STR,
                Span::dummy(),
            ),
            ast::StringPart::Expr(e) => {
                let expr_hir = self.lower_expr_typed(e, registry);
                if expr_hir.type_id == Some(builtin::STR) {
                    expr_hir
                } else {
                    // Cast to String
                    HirExpr::with_type(
                        HirExprKind::Cast {
                            value: Box::new(expr_hir),
                            to_type: builtin::STR,
                        },
                        builtin::STR,
                        Span::dummy(),
                    )
                }
            }
        }
    }
}

impl Default for Lower {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use doo_frontend::Parser;

    fn parse_and_lower(source: &str) -> HirProgram {
        let mut parser = Parser::new(source, 0);
        let program = parser.parse_program().unwrap();
        let mut lower = Lower::new();
        lower.lower_program(&program)
    }

    #[test]
    fn test_lower_int_literal() {
        let hir = parse_and_lower("let x = 42");
        assert_eq!(hir.items.len(), 0); // Top-level statements not supported in HIR items
    }

    #[test]
    fn test_lower_function() {
        let hir = parse_and_lower("fn add(a: Int, b: Int) -> Int { return a + b }");
        assert_eq!(hir.items.len(), 1);
        if let HirItem::Function(f) = &hir.items[0] {
            assert_eq!(f.name, "add");
            assert_eq!(f.params.len(), 2);
        } else {
            panic!("Expected function");
        }
    }

    #[test]
    fn test_lower_struct() {
        let hir = parse_and_lower("struct User { name: Str, age: Int }");
        assert_eq!(hir.items.len(), 1);
        if let HirItem::Struct(s) = &hir.items[0] {
            assert_eq!(s.name, "User");
            assert_eq!(s.fields.len(), 2);
        } else {
            panic!("Expected struct");
        }
    }

    #[test]
    fn test_desugar_compound_assign() {
        let hir = parse_and_lower("fn test() { let mut x = 1\n x += 2 }");
        if let HirItem::Function(f) = &hir.items[0] {
            // Second statement should be desugared assignment
            assert_eq!(f.body.len(), 2);
            if let HirStmtKind::Assign { value, .. } = &f.body[1].kind {
                assert!(matches!(value.kind, HirExprKind::BinOp { op: HirBinOp::Add, .. }));
            } else {
                panic!("Expected assign");
            }
        }
    }

    #[test]
    fn test_desugar_increment() {
        let hir = parse_and_lower("fn test() { let mut x = 0\n x++ }");
        if let HirItem::Function(f) = &hir.items[0] {
            assert_eq!(f.body.len(), 2);
            if let HirStmtKind::Assign { value, .. } = &f.body[1].kind {
                assert!(matches!(value.kind, HirExprKind::BinOp { op: HirBinOp::Add, .. }));
            } else {
                panic!("Expected assign from increment");
            }
        }
    }
}
