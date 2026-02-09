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

use doo_core::{
    doo_debug,
    infer::{infer_binop_result_type, infer_unaryop_result_type, BinOpKind, UnaryOpKind},
    types::{builtin, TypeId, TypeKind, TypeRegistry},
    Span,
};
use doo_frontend::ast::{
    self, BinaryOp, CompoundOp, Decorator, ElseBranch, EnumDecl, Expr, ExprKind, FunctionDecl,
    ImportDecl, IncDecOp, Item, Pattern, PatternKind, Program, Stmt, StmtKind, StructDecl,
    TypeExpr, UnaryOp,
};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::types::*;

/// AST to HIR lowering context.
pub struct Lower {
    /// Collected errors during lowering.
    errors: Vec<LowerError>,
    /// Variable type tracking for typed lowering (name -> TypeId)
    var_types: FxHashMap<String, TypeId>,
    /// Counter for generating unique internal variable names
    unique_counter: u64,
    /// Track JSON stringify sources: variable name -> type of the stringified value
    /// Used to infer JSON.parse return type when parsing a variable
    json_stringify_sources: FxHashMap<String, TypeId>,
    /// Items hoisted from inside function bodies (local struct/enum declarations).
    hoisted_items: Vec<HirItem>,
    /// Known standalone function names in the program (for disambiguating Namespace::Func from EnumVariant).
    /// Only contains functions WITHOUT an associated type (i.e., not methods like Server.get).
    known_functions: FxHashSet<String>,
    /// Known qualified methods: maps (TypeName, MethodName) pairs for associated functions.
    /// e.g., Server.get -> ("Server", "get"), Database.Postgres -> ("Database", "Postgres")
    known_qualified_methods: FxHashMap<String, FxHashSet<String>>,
}

/// Lowering error.
#[derive(Debug, Clone)]
pub struct LowerError {
    pub message: String,
    pub span: Span,
}

impl LowerError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

impl Lower {
    /// Create a new lowering context.
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
            var_types: FxHashMap::default(),
            unique_counter: 0,
            json_stringify_sources: FxHashMap::default(),
            hoisted_items: Vec::new(),
            known_functions: FxHashSet::default(),
            known_qualified_methods: FxHashMap::default(),
        }
    }

    /// Generate a unique suffix for internal variable names.
    fn unique_suffix(&mut self) -> u64 {
        let id = self.unique_counter;
        self.unique_counter += 1;
        id
    }

    /// Recursively substitute a local variable name in a statement.
    fn substitute_local_in_stmt(&self, stmt: &mut HirStmt, old_name: &str, new_name: &str) {
        self.substitute_local_in_stmt_kind(&mut stmt.kind, old_name, new_name);
    }

    fn substitute_local_in_stmt_kind(
        &self,
        kind: &mut HirStmtKind,
        old_name: &str,
        new_name: &str,
    ) {
        match kind {
            HirStmtKind::Let { value, .. } => {
                self.substitute_local_in_expr(value, old_name, new_name);
            }
            HirStmtKind::TupleLet { value, .. } => {
                self.substitute_local_in_expr(value, old_name, new_name);
            }
            HirStmtKind::Expr(expr) => {
                self.substitute_local_in_expr(expr, old_name, new_name);
            }
            HirStmtKind::Assign { target, value, .. } => {
                self.substitute_local_in_expr(target, old_name, new_name);
                self.substitute_local_in_expr(value, old_name, new_name);
            }
            HirStmtKind::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                self.substitute_local_in_expr(condition, old_name, new_name);
                for s in then_block {
                    self.substitute_local_in_stmt(s, old_name, new_name);
                }
                if let Some(else_stmts) = else_block {
                    for s in else_stmts {
                        self.substitute_local_in_stmt(s, old_name, new_name);
                    }
                }
            }
            HirStmtKind::While {
                condition, body, ..
            } => {
                self.substitute_local_in_expr(condition, old_name, new_name);
                for s in body {
                    self.substitute_local_in_stmt(s, old_name, new_name);
                }
            }
            HirStmtKind::Return(exprs) => {
                for expr in exprs {
                    self.substitute_local_in_expr(expr, old_name, new_name);
                }
            }
            HirStmtKind::ManualErrorExtract { expr, .. } => {
                self.substitute_local_in_expr(expr, old_name, new_name);
            }
            HirStmtKind::Break | HirStmtKind::Continue | HirStmtKind::Drop { .. } => {}
        }
    }

    fn substitute_local_in_expr(&self, expr: &mut HirExpr, old_name: &str, new_name: &str) {
        match &mut expr.kind {
            HirExprKind::Local { name, .. } => {
                if name == old_name {
                    *name = new_name.to_string();
                }
            }
            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.substitute_local_in_expr(lhs, old_name, new_name);
                self.substitute_local_in_expr(rhs, old_name, new_name);
            }
            HirExprKind::UnaryOp { operand, .. } => {
                self.substitute_local_in_expr(operand, old_name, new_name);
            }
            HirExprKind::Call { func, args, .. } => {
                self.substitute_local_in_expr(func, old_name, new_name);
                for arg in args {
                    self.substitute_local_in_expr(arg, old_name, new_name);
                }
            }
            HirExprKind::MethodCall { receiver, args, .. } => {
                self.substitute_local_in_expr(receiver, old_name, new_name);
                for arg in args {
                    self.substitute_local_in_expr(arg, old_name, new_name);
                }
            }
            HirExprKind::Field { object, .. } => {
                self.substitute_local_in_expr(object, old_name, new_name);
            }
            HirExprKind::Index { object, index, .. } => {
                self.substitute_local_in_expr(object, old_name, new_name);
                self.substitute_local_in_expr(index, old_name, new_name);
            }
            HirExprKind::Array(elements) => {
                for el in elements {
                    self.substitute_local_in_expr(el, old_name, new_name);
                }
            }
            HirExprKind::Map(entries) => {
                for (k, v) in entries {
                    self.substitute_local_in_expr(k, old_name, new_name);
                    self.substitute_local_in_expr(v, old_name, new_name);
                }
            }
            HirExprKind::Tuple(elements) => {
                for el in elements {
                    self.substitute_local_in_expr(el, old_name, new_name);
                }
            }
            HirExprKind::Struct { fields, .. } => {
                for (_, val) in fields {
                    self.substitute_local_in_expr(val, old_name, new_name);
                }
            }
            HirExprKind::EnumVariant { payload, .. } => {
                for p in payload {
                    self.substitute_local_in_expr(p, old_name, new_name);
                }
            }
            HirExprKind::Spread(inner) => {
                self.substitute_local_in_expr(inner, old_name, new_name);
            }
            HirExprKind::RouteBlock { routes } => {
                for route in routes {
                    self.substitute_local_in_expr(route, old_name, new_name);
                }
            }
            HirExprKind::If {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                self.substitute_local_in_expr(condition, old_name, new_name);
                self.substitute_local_in_expr(then_expr, old_name, new_name);
                if let Some(el) = else_expr {
                    self.substitute_local_in_expr(el, old_name, new_name);
                }
            }
            HirExprKind::Block { stmts, expr, .. } => {
                for s in stmts {
                    self.substitute_local_in_stmt(s, old_name, new_name);
                }
                if let Some(e) = expr {
                    self.substitute_local_in_expr(e, old_name, new_name);
                }
            }
            HirExprKind::Match { values, arms, .. } => {
                for v in values {
                    self.substitute_local_in_expr(v, old_name, new_name);
                }
                for arm in arms {
                    if let Some(guard) = &mut arm.guard {
                        self.substitute_local_in_expr(guard, old_name, new_name);
                    }
                    self.substitute_local_in_expr(&mut arm.body, old_name, new_name);
                }
            }
            HirExprKind::Range { start, end, .. } => {
                self.substitute_local_in_expr(start, old_name, new_name);
                self.substitute_local_in_expr(end, old_name, new_name);
            }
            HirExprKind::Closure { body, .. } => {
                self.substitute_local_in_expr(body, old_name, new_name);
            }
            HirExprKind::Ok(inner)
            | HirExprKind::Err(inner)
            | HirExprKind::Try(inner)
            | HirExprKind::Move(inner)
            | HirExprKind::Clone(inner) => {
                self.substitute_local_in_expr(inner, old_name, new_name);
            }
            HirExprKind::UnwrapOrPanic {
                expr: inner,
                message,
            } => {
                self.substitute_local_in_expr(inner, old_name, new_name);
                self.substitute_local_in_expr(message, old_name, new_name);
            }
            HirExprKind::Borrow { expr: inner, .. } => {
                self.substitute_local_in_expr(inner, old_name, new_name);
            }
            HirExprKind::Cast { value, .. } => {
                self.substitute_local_in_expr(value, old_name, new_name);
            }
            // Literals and constants don't have local references
            HirExprKind::Const(_) | HirExprKind::Global { .. } => {}
        }
    }

    /// Lower an entire program.
    pub fn lower_program(&mut self, program: &Program) -> HirProgram {
        // Pre-collect function names for namespace-qualified call disambiguation
        for item in &program.items {
            if let Item::Function(f) = item {
                if let Some(ref assoc_type) = f.associated_type {
                    // Associated method: track under its type namespace
                    self.known_qualified_methods
                        .entry(assoc_type.clone())
                        .or_default()
                        .insert(f.name.clone());
                } else {
                    // Standalone function only
                    self.known_functions.insert(f.name.clone());
                }
            }
        }

        let mut items: Vec<HirItem> = program
            .items
            .iter()
            .filter_map(|item| self.lower_item(item))
            .collect();

        // Append any struct/enum declarations hoisted from inside function bodies
        items.append(&mut self.hoisted_items);

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
        // Pre-collect all function names so we can disambiguate
        // Namespace::Func(args) from EnumVariant during expression lowering.
        for item in &program.items {
            if let Item::Function(f) = item {
                if let Some(ref assoc_type) = f.associated_type {
                    // Associated method: track under its type namespace
                    self.known_qualified_methods
                        .entry(assoc_type.clone())
                        .or_default()
                        .insert(f.name.clone());
                } else {
                    // Standalone function only
                    self.known_functions.insert(f.name.clone());
                }
            }
        }

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

        let mut items: Vec<HirItem> = program
            .items
            .iter()
            .filter_map(|item| self.lower_item_typed(item, registry))
            .collect();

        // Register hoisted items (local struct/enum) in type registry
        for hoisted in &self.hoisted_items {
            match hoisted {
                HirItem::Struct(s) => {
                    registry.declare_named(&s.name);
                }
                HirItem::Enum(e) => {
                    registry.declare_named(&e.name);
                }
                _ => {}
            }
        }

        // Append hoisted items from inside function bodies
        items.append(&mut self.hoisted_items);

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
        let params = f
            .params
            .iter()
            .map(|(name, _type_ann)| {
                HirParam {
                    name: name.clone(),
                    type_id: None, // Type resolution in later phase
                    span: f.span,
                }
            })
            .collect();

        let body = f.body.iter().map(|stmt| self.lower_stmt(stmt)).collect();

        let decorators = f
            .decorators
            .iter()
            .map(|d| self.lower_decorator(d))
            .collect();

        // Generate mangled name for methods: _method_{TypeName}_{MethodName}
        let func_name = if let Some(type_name) = &f.associated_type {
            format!("_method_{}_{}", type_name, f.name)
        } else {
            f.name.clone()
        };

        HirFunction {
            name: func_name,
            params,
            return_type: None,
            error_type: None,
            body,
            decorators,
            span: f.span,
        }
    }

    fn lower_function_typed(
        &mut self,
        f: &FunctionDecl,
        registry: &mut TypeRegistry,
    ) -> HirFunction {
        doo_debug!("HIR", "lower_function_typed: Lowering function: {}", f.name);
        doo_debug!(
            "HIR",
            "lower_function_typed: Function body has {} statements",
            f.body.len()
        );
        for (i, stmt) in f.body.iter().enumerate() {
            doo_debug!("HIR", "lower_function_typed:   Stmt {}: {:?}", i, stmt.kind);
        }
        // Clear variable types for new function scope
        self.var_types.clear();

        // For method functions (fn Type.method), resolve the receiver type
        let receiver_type_id = f
            .associated_type
            .as_ref()
            .and_then(|type_name| registry.lookup(type_name));

        // If this is a method, track 'self' parameter type
        if let Some(type_id) = receiver_type_id {
            self.var_types.insert("self".to_string(), type_id);
        }

        let params: Vec<HirParam> = f
            .params
            .iter()
            .map(|(name, type_ann)| {
                // Special handling for 'self' parameter in methods
                let type_id = if name == "self" {
                    // Use the receiver type for 'self'
                    receiver_type_id
                } else {
                    type_ann
                        .as_ref()
                        .map(|t| self.resolve_type_expr(t, registry))
                };
                // Track parameter types
                if let Some(tid) = type_id {
                    self.var_types.insert(name.clone(), tid);
                }
                HirParam {
                    name: name.clone(),
                    type_id,
                    span: f.span,
                }
            })
            .collect();

        let body = f
            .body
            .iter()
            .map(|stmt| self.lower_stmt_typed(stmt, registry))
            .collect();

        let decorators = f
            .decorators
            .iter()
            .map(|d| self.lower_decorator(d))
            .collect();

        // Generate mangled name for methods: _method_{TypeName}_{MethodName}
        let func_name = if let Some(type_name) = &f.associated_type {
            format!("_method_{}_{}", type_name, f.name)
        } else {
            f.name.clone()
        };

        HirFunction {
            name: func_name,
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
        let fields = s
            .fields
            .iter()
            .map(|f| HirField {
                name: f.name.clone(),
                type_id: None,
                is_public: f.is_public,
                is_optional: f.is_optional,
                default: f.default.as_ref().map(|e| self.lower_expr(e)),
                decorators: f
                    .decorators
                    .iter()
                    .map(|d| self.lower_decorator(d))
                    .collect(),
                span: f.span,
            })
            .collect();

        let decorators = s
            .decorators
            .iter()
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
                    is_public: f.is_public,
                    is_optional: f.is_optional,
                    default: f
                        .default
                        .as_ref()
                        .map(|e| self.lower_expr_typed(e, registry)),
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
                .filter_map(|f| f.type_id.map(|id| (f.name.clone(), id, f.is_public)))
                .collect(),
        );

        let decorators = s
            .decorators
            .iter()
            .map(|d| self.lower_decorator(d))
            .collect();

        HirStruct {
            name: s.name.clone(),
            fields,
            decorators,
            span: s.span,
        }
    }

    fn lower_enum(&mut self, e: &EnumDecl) -> HirEnum {
        let variants = e
            .variants
            .iter()
            .map(|v| HirVariant {
                name: v.name.clone(),
                payload: None,
                span: v.span,
            })
            .collect();

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
        let items = i
            .items
            .iter()
            .map(|item| match item {
                ast::ImportItem::Symbol(s) => HirImportItem::Symbol(s.clone()),
                ast::ImportItem::Alias { name, alias } => HirImportItem::Alias {
                    name: name.clone(),
                    alias: alias.clone(),
                },
                ast::ImportItem::Wildcard => HirImportItem::Wildcard,
            })
            .collect();

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
            StmtKind::Let {
                mutable,
                pattern,
                type_ann: _,
                value,
            } => {
                // Check if this is a tuple pattern - if so, use TupleLet
                if let PatternKind::Tuple(patterns) = &pattern.kind {
                    let names: Vec<String> =
                        patterns.iter().map(|p| self.pattern_to_name(p)).collect();
                    let type_ids: Vec<Option<TypeId>> = vec![None; names.len()];
                    HirStmtKind::TupleLet {
                        names,
                        type_ids,
                        value: self.lower_expr(value),
                        mutable: *mutable,
                    }
                } else {
                    let name = self.pattern_to_name(pattern);
                    HirStmtKind::Let {
                        name,
                        type_id: None,
                        value: self.lower_expr(value),
                        mutable: *mutable,
                        ownership: Ownership::Owned,
                    }
                }
            }

            StmtKind::Assign { target, value } => {
                doo_debug!(
                    "HIR",
                    "lower_stmt NON-TYPED: Assign statement, target pattern: {:?}",
                    target.kind
                );
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
                    HirExprKind::Local {
                        name: variable.clone(),
                    },
                    stmt.span,
                );
                let target_read = HirExpr::new(
                    HirExprKind::Local {
                        name: variable.clone(),
                    },
                    stmt.span,
                );
                let one = HirExpr::new(HirExprKind::Const(ConstValue::Int(1)), stmt.span);
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

            StmtKind::Expr(expr) => HirStmtKind::Expr(self.lower_expr(expr)),

            StmtKind::Return(values) => {
                HirStmtKind::Return(values.iter().map(|e| self.lower_expr(e)).collect())
            }

            StmtKind::Break => HirStmtKind::Break,
            StmtKind::Continue => HirStmtKind::Continue,

            StmtKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                let then_stmts = then_block.iter().map(|s| self.lower_stmt(s)).collect();
                let else_stmts = else_branch.as_ref().map(|eb| match eb {
                    ElseBranch::Block(stmts) => stmts.iter().map(|s| self.lower_stmt(s)).collect(),
                    ElseBranch::ElseIf(if_stmt) => {
                        vec![self.lower_stmt(if_stmt)]
                    }
                });

                HirStmtKind::If {
                    condition: self.lower_expr(condition),
                    then_block: then_stmts,
                    else_block: else_stmts,
                }
            }

            // === Desugaring: For Loop ===
            // Three cases:
            // 1. `for i in start..end` → range-based index loop
            // 2. `for x in array` → index-based array iteration
            // 3. `for i, x in array` → index + element iteration
            // 4. `for { ... }` → infinite loop
            StmtKind::For {
                pattern,
                iterable,
                body,
            } => self.lower_for_loop(pattern, iterable.as_ref(), body, stmt.span),

            StmtKind::Block(stmts) => {
                // Lower block as expression statement
                let lowered: Vec<_> = stmts.iter().map(|s| self.lower_stmt(s)).collect();
                if lowered.len() == 1 {
                    return lowered.into_iter().next().unwrap();
                }
                // Represent as nested block via expression
                HirStmtKind::Expr(HirExpr::new(
                    HirExprKind::Block {
                        stmts: lowered,
                        expr: None,
                    },
                    stmt.span,
                ))
            }

            StmtKind::Print(exprs) => {
                // Flatten StringInterpolation parts as separate print args
                // so composite types (Array, Map, Struct) get proper formatting
                // via the Print handler instead of broken string concat.
                let mut args = Vec::new();
                let mut has_interpolation = false;
                for e in exprs {
                    if let ExprKind::StringInterpolation(parts) = &e.kind {
                        has_interpolation = true;
                        for part in parts {
                            args.push(self.lower_string_part(part));
                        }
                    } else {
                        args.push(self.lower_expr(e));
                    }
                }
                let func_name = if has_interpolation {
                    "__print_interp"
                } else {
                    "print"
                };
                HirStmtKind::Expr(HirExpr::new(
                    HirExprKind::Call {
                        func: Box::new(HirExpr::new(
                            HirExprKind::Global {
                                name: func_name.to_string(),
                            },
                            stmt.span,
                        )),
                        args,
                    },
                    stmt.span,
                ))
            }

            StmtKind::ElementAssign {
                array,
                index,
                value,
            } => {
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

            StmtKind::FieldAssign {
                object,
                field,
                value,
            } => {
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

            StmtKind::ManualErrorExtract {
                expr,
                ok_pattern,
                error_var,
            } => {
                let ok_names = self.pattern_to_names(ok_pattern);
                HirStmtKind::ManualErrorExtract {
                    ok_names,
                    error_name: error_var.clone(),
                    expr: self.lower_expr(expr),
                }
            }

            // Local struct/enum declarations: hoist to top-level items, emit no-op in body
            StmtKind::StructDecl(s) => {
                let hir_struct = self.lower_struct(s);
                self.hoisted_items.push(HirItem::Struct(hir_struct));
                HirStmtKind::Expr(HirExpr::new(HirExprKind::Const(ConstValue::Nil), stmt.span))
            }
            StmtKind::EnumDecl(e) => {
                let hir_enum = self.lower_enum(e);
                self.hoisted_items.push(HirItem::Enum(hir_enum));
                HirStmtKind::Expr(HirExpr::new(HirExprKind::Const(ConstValue::Nil), stmt.span))
            }
        };

        HirStmt::new(kind, stmt.span)
    }

    fn lower_stmt_typed(&mut self, stmt: &Stmt, registry: &mut TypeRegistry) -> HirStmt {
        doo_debug!(
            "HIR",
            "lower_stmt_typed: Processing statement: {:?}",
            std::mem::discriminant(&stmt.kind)
        );
        let kind = match &stmt.kind {
            StmtKind::Let {
                mutable,
                pattern,
                type_ann,
                value,
            } => {
                // Check if this is a tuple pattern
                if let PatternKind::Tuple(patterns) = &pattern.kind {
                    let names: Vec<String> =
                        patterns.iter().map(|p| self.pattern_to_name(p)).collect();
                    let mut value_hir = self.lower_expr_typed(value, registry);

                    // Try to get element types from the value's tuple type
                    let mut type_ids: Vec<Option<TypeId>> = vec![None; names.len()];
                    if let Some(val_type_id) = value_hir.type_id {
                        if let Some(info) = registry.get(val_type_id) {
                            if let TypeKind::Tuple { elements } = &info.kind {
                                for (i, elem_type) in elements.iter().enumerate() {
                                    if i < type_ids.len() {
                                        type_ids[i] = Some(*elem_type);
                                        // Track each element's type
                                        self.var_types.insert(names[i].clone(), *elem_type);
                                    }
                                }
                            }
                        }
                    }

                    HirStmtKind::TupleLet {
                        names,
                        type_ids,
                        value: value_hir,
                        mutable: *mutable,
                    }
                } else {
                    let name = self.pattern_to_name(pattern);
                    let value_hir = self.lower_expr_typed(value, registry);
                    let annotated_type_id = type_ann
                        .as_ref()
                        .map(|t| self.resolve_type_expr(t, registry));
                    let inferred_type_id = annotated_type_id.or(value_hir.type_id);
                    // NOTE: Do NOT overwrite value_hir.type_id with annotated type.
                    // The value expression must keep its original type so the type checker
                    // can compare it against the annotation and detect mismatches.
                    // The Let statement's own type_id field carries the annotation.
                    // Track variable type for later lookups
                    if let Some(tid) = inferred_type_id {
                        self.var_types.insert(name.clone(), tid);
                    }

                    // Track JSON.stringify sources for later JSON.parse type inference
                    // If the value is JSON.stringify(x), remember that this variable contains JSON of type x
                    if let Some(stringify_arg_type) = self.extract_stringify_arg_type(&value_hir) {
                        self.json_stringify_sources
                            .insert(name.clone(), stringify_arg_type);
                    }

                    HirStmtKind::Let {
                        name,
                        type_id: inferred_type_id,
                        value: value_hir,
                        mutable: *mutable,
                        ownership: Ownership::Owned,
                    }
                }
            }

            StmtKind::Assign { target, value } => {
                doo_debug!(
                    "HIR",
                    "lower_stmt: Assign statement, target pattern: {:?}",
                    target.kind
                );
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
                    HirExprKind::Local {
                        name: variable.clone(),
                    },
                    stmt.span,
                );
                let target_read = HirExpr::new(
                    HirExprKind::Local {
                        name: variable.clone(),
                    },
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

            StmtKind::Return(values) => HirStmtKind::Return(
                values
                    .iter()
                    .map(|e| self.lower_expr_typed(e, registry))
                    .collect(),
            ),

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
            } => self.lower_for_loop_typed(pattern, iterable.as_ref(), body, stmt.span, registry),

            StmtKind::Block(stmts) => {
                let lowered: Vec<_> = stmts
                    .iter()
                    .map(|s| self.lower_stmt_typed(s, registry))
                    .collect();
                if lowered.len() == 1 {
                    return lowered.into_iter().next().unwrap();
                }
                HirStmtKind::Expr(HirExpr::new(
                    HirExprKind::Block {
                        stmts: lowered,
                        expr: None,
                    },
                    stmt.span,
                ))
            }

            StmtKind::Print(exprs) => {
                // Flatten StringInterpolation parts as separate print args.
                // Don't cast to STR — keep original types so the Print handler
                // uses type-specific formatters (Array, Map, Struct, etc.)
                let mut args = Vec::new();
                let mut has_interpolation = false;
                for e in exprs {
                    if let ExprKind::StringInterpolation(parts) = &e.kind {
                        has_interpolation = true;
                        for part in parts {
                            match part {
                                ast::StringPart::Literal(s) => {
                                    args.push(HirExpr::with_type(
                                        HirExprKind::Const(ConstValue::Str(s.clone())),
                                        builtin::STR,
                                        stmt.span,
                                    ));
                                }
                                ast::StringPart::Expr(expr) => {
                                    // Lower WITHOUT Cast to STR — preserve original type
                                    args.push(self.lower_expr_typed(expr, registry));
                                }
                            }
                        }
                    } else {
                        args.push(self.lower_expr_typed(e, registry));
                    }
                }
                let func_name = if has_interpolation {
                    "__print_interp"
                } else {
                    "print"
                };
                HirStmtKind::Expr(HirExpr::new(
                    HirExprKind::Call {
                        func: Box::new(HirExpr::new(
                            HirExprKind::Global {
                                name: func_name.to_string(),
                            },
                            stmt.span,
                        )),
                        args,
                    },
                    stmt.span,
                ))
            }

            StmtKind::ElementAssign {
                array,
                index,
                value,
            } => HirStmtKind::Expr(HirExpr::new(
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

            StmtKind::FieldAssign {
                object,
                field,
                value,
            } => {
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

            // Local struct/enum declarations: hoist to top-level items, emit no-op in body
            StmtKind::StructDecl(s) => {
                let hir_struct = self.lower_struct_typed(s, registry);
                self.hoisted_items.push(HirItem::Struct(hir_struct));
                HirStmtKind::Expr(HirExpr::new(HirExprKind::Const(ConstValue::Nil), stmt.span))
            }
            StmtKind::EnumDecl(e) => {
                let hir_enum = self.lower_enum_typed(e, registry);
                self.hoisted_items.push(HirItem::Enum(hir_enum));
                HirStmtKind::Expr(HirExpr::new(HirExprKind::Const(ConstValue::Nil), stmt.span))
            }
        };

        HirStmt::new(kind, stmt.span)
    }

    // ========================================================================
    // HTTP Route Middleware Helpers
    // ========================================================================

    /// Check if a method name is an HTTP route method.
    fn is_http_route_method(method: &str) -> bool {
        matches!(
            method,
            "get" | "post" | "put" | "delete" | "patch" | "head" | "options"
        )
    }

    /// Transform a route method call with middleware arguments.
    /// app.get("/path", middleware, Handler) -> HirExpr for method call with middleware array
    fn transform_route_with_middleware(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> HirExpr {
        // args format: [path, middleware1, middleware2, ..., handler]
        // Transform to: Server.{method}WithMiddleware(path, middleware_str, handler)
        let receiver = Box::new(self.lower_expr(object));
        let path = self.lower_expr(&args[0]);
        let handler = self.lower_expr(args.last().unwrap());

        // Collect middleware names (args[1..len-1]) as comma-separated string
        // Each middleware arg should be a call that returns a string (e.g., jwt() returns "jwt")
        let middleware_args: Vec<HirExpr> = args[1..args.len() - 1]
            .iter()
            .map(|a| self.lower_expr(a))
            .collect();

        // For single middleware, pass it directly
        // For multiple middlewares, we'd need to concatenate strings
        // For now, use the first middleware if any
        let middleware = if middleware_args.is_empty() {
            HirExpr::new(HirExprKind::Const(ConstValue::Str("".to_string())), span)
        } else if middleware_args.len() == 1 {
            middleware_args.into_iter().next().unwrap()
        } else {
            // TODO: Concatenate multiple middleware names
            middleware_args.into_iter().next().unwrap()
        };

        // Create method name with "WithMiddleware" suffix
        let new_method = format!("{}WithMiddleware", method);

        // Args: [path, middleware, handler]
        let new_args = vec![path, middleware, handler];

        HirExpr::new(
            HirExprKind::MethodCall {
                receiver,
                method: new_method,
                args: new_args,
            },
            span,
        )
    }

    /// Transform app.group with a route block.
    /// app.group("/api", middleware, { get(...), post(...) }) -> expand each route
    fn transform_group_with_routes(&mut self, object: &Expr, args: &[Expr], span: Span) -> HirExpr {
        // For now, just lower the group call normally, the FFI will handle it at runtime
        // args[0] = prefix, args[1..n-1] = middleware, args[n-1] = RouteBlock
        let receiver = Box::new(self.lower_expr(object));
        let lowered_args: Vec<HirExpr> = args.iter().map(|a| self.lower_expr(a)).collect();

        HirExpr::new(
            HirExprKind::MethodCall {
                receiver,
                method: "group".to_string(),
                args: lowered_args,
            },
            span,
        )
    }

    /// Typed version: Transform route with middleware
    fn transform_route_with_middleware_typed(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Expr],
        span: Span,
        registry: &mut TypeRegistry,
    ) -> HirExpr {
        // args format: [path, middleware1, middleware2, ..., handler]
        // Transform to: Server.{method}WithMiddleware(path, middleware_str, handler)
        let receiver = Box::new(self.lower_expr_typed(object, registry));
        let path = self.lower_expr_typed(&args[0], registry);
        let handler = self.lower_expr_typed(args.last().unwrap(), registry);

        // Collect middleware names (args[1..len-1])
        let middleware_args: Vec<HirExpr> = args[1..args.len() - 1]
            .iter()
            .map(|a| self.lower_expr_typed(a, registry))
            .collect();

        // For single middleware, pass it directly
        let middleware = if middleware_args.is_empty() {
            HirExpr::new(HirExprKind::Const(ConstValue::Str("".to_string())), span)
        } else if middleware_args.len() == 1 {
            middleware_args.into_iter().next().unwrap()
        } else {
            // TODO: Concatenate multiple middleware names
            middleware_args.into_iter().next().unwrap()
        };

        // Create method name with "WithMiddleware" suffix
        let new_method = format!("{}WithMiddleware", method);

        // Args: [path, middleware, handler]
        let new_args = vec![path, middleware, handler];

        HirExpr::new(
            HirExprKind::MethodCall {
                receiver,
                method: new_method,
                args: new_args,
            },
            span,
        )
    }

    /// Typed version: Transform app.group with a route block
    fn transform_group_with_routes_typed(
        &mut self,
        object: &Expr,
        args: &[Expr],
        span: Span,
        registry: &mut TypeRegistry,
    ) -> HirExpr {
        // For now, just lower the group call normally
        let receiver = Box::new(self.lower_expr_typed(object, registry));
        let lowered_args: Vec<HirExpr> = args
            .iter()
            .map(|a| self.lower_expr_typed(a, registry))
            .collect();

        HirExpr::new(
            HirExprKind::MethodCall {
                receiver,
                method: "group".to_string(),
                args: lowered_args,
            },
            span,
        )
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

            ExprKind::Binary { left, op, right } => HirExprKind::BinOp {
                op: self.lower_binop(*op),
                lhs: Box::new(self.lower_expr(left)),
                rhs: Box::new(self.lower_expr(right)),
            },

            ExprKind::Unary { op, expr: inner } => HirExprKind::UnaryOp {
                op: self.lower_unaryop(*op),
                operand: Box::new(self.lower_expr(inner)),
            },

            ExprKind::Call { func, args } => HirExprKind::Call {
                func: Box::new(self.lower_expr(func)),
                args: args.iter().map(|a| self.lower_expr(a)).collect(),
            },

            ExprKind::MethodCall {
                object,
                method,
                args,
            } => {
                // Transform HTTP route methods with middleware arguments
                // app.get("/path", middleware, Handler) -> app.getWithMiddleware("/path", "middleware", Handler)
                // app.get("/path", m1, m2, Handler) -> app.getWithMiddleware("/path", "m1,m2", Handler)
                if Self::is_http_route_method(method) && args.len() > 2 {
                    return self.transform_route_with_middleware(object, method, args, expr.span);
                }

                // Transform app.group with route block
                // app.group("/api", middleware, { routes }) -> expand routes with prefix and middleware
                if method == "group" && args.len() >= 2 {
                    if let Some(route_block_arg) = args.last() {
                        if matches!(
                            route_block_arg,
                            Expr {
                                kind: ExprKind::RouteBlock { .. },
                                ..
                            }
                        ) {
                            return self.transform_group_with_routes(object, args, expr.span);
                        }
                    }
                }

                HirExprKind::MethodCall {
                    receiver: Box::new(self.lower_expr(object)),
                    method: method.clone(),
                    args: args.iter().map(|a| self.lower_expr(a)).collect(),
                }
            }

            ExprKind::Field { object, field } => HirExprKind::Field {
                object: Box::new(self.lower_expr(object)),
                field: field.clone(),
            },

            ExprKind::Index { object, index } => HirExprKind::Index {
                object: Box::new(self.lower_expr(object)),
                index: Box::new(self.lower_expr(index)),
            },

            ExprKind::ArrayLit(elements) => {
                HirExprKind::Array(elements.iter().map(|e| self.lower_expr(e)).collect())
            }

            ExprKind::MapLit(entries) => HirExprKind::Map(
                entries
                    .iter()
                    .map(|(k, v)| (self.lower_expr(k), self.lower_expr(v)))
                    .collect(),
            ),

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
                    fields: fields
                        .iter()
                        .map(|(k, v)| (k.clone(), self.lower_expr(v)))
                        .collect(),
                }
            }

            ExprKind::EnumVariant {
                enum_name,
                variant,
                payload,
            } => {
                // Check if this is a qualified method call: Type::method(args)
                // e.g., Database::get() should resolve to the Database.get associated method
                let is_qualified_method = self
                    .known_qualified_methods
                    .get(enum_name)
                    .map(|methods| methods.contains(variant))
                    .unwrap_or(false);

                if is_qualified_method || self.known_functions.contains(variant) {
                    // Namespace-qualified function call: Namespace::Func(args) -> Func(args)
                    let func_expr = HirExpr::new(
                        HirExprKind::Global {
                            name: variant.clone(),
                        },
                        expr.span,
                    );
                    HirExprKind::Call {
                        func: Box::new(func_expr),
                        args: payload.iter().map(|e| self.lower_expr(e)).collect(),
                    }
                } else {
                    HirExprKind::EnumVariant {
                        enum_name: enum_name.clone(),
                        variant: variant.clone(),
                        payload: payload.iter().map(|e| self.lower_expr(e)).collect(),
                    }
                }
            }

            ExprKind::Range {
                start,
                end,
                inclusive,
            } => HirExprKind::Range {
                start: Box::new(self.lower_expr(start)),
                end: Box::new(self.lower_expr(end)),
                inclusive: *inclusive,
            },

            ExprKind::IfExpr {
                condition,
                then_branch,
                else_branch,
            } => HirExprKind::If {
                condition: Box::new(self.lower_expr(condition)),
                then_expr: Box::new(self.lower_expr(then_branch)),
                else_expr: else_branch.as_ref().map(|e| Box::new(self.lower_expr(e))),
            },

            ExprKind::Block(stmts, final_expr) => HirExprKind::Block {
                stmts: stmts.iter().map(|s| self.lower_stmt(s)).collect(),
                expr: final_expr.as_ref().map(|e| Box::new(self.lower_expr(e))),
            },

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

            ExprKind::Err(inner) => HirExprKind::Err(Box::new(self.lower_expr(inner))),

            ExprKind::Try(inner) => HirExprKind::Try(Box::new(self.lower_expr(inner))),

            ExprKind::UnwrapOrPanic {
                expr: inner,
                message,
            } => HirExprKind::UnwrapOrPanic {
                expr: Box::new(self.lower_expr(inner)),
                message: Box::new(self.lower_expr(message)),
            },

            ExprKind::Closure { params, body, .. } => HirExprKind::Closure {
                params: params.iter().map(|(n, _)| (n.clone(), None)).collect(),
                body: Box::new(self.lower_expr(body)),
            },

            ExprKind::Match { values, arms } => {
                let lowered_values: Vec<HirExpr> =
                    values.iter().map(|v| self.lower_expr(v)).collect();
                let lowered_arms: Vec<HirMatchArm> = arms
                    .iter()
                    .map(|a| {
                        let mut pattern = self.lower_match_pattern(&a.pattern);
                        // Convert struct literal patterns to field-by-field comparisons
                        if let HirMatchPattern::Condition(ref cond_expr) = pattern {
                            if let HirExprKind::Struct { fields, .. } = &cond_expr.kind {
                                if let Some(matched_val) = lowered_values.first() {
                                    pattern = self.struct_pattern_to_condition(
                                        matched_val,
                                        fields,
                                        cond_expr.span,
                                    );
                                }
                            }
                        }
                        HirMatchArm {
                            pattern,
                            guard: a.guard.as_ref().map(|g| self.lower_expr(g)),
                            body: self.lower_expr(&a.body),
                            span: a.span,
                        }
                    })
                    .collect();
                HirExprKind::Match {
                    values: lowered_values,
                    arms: lowered_arms,
                }
            }

            ExprKind::Spread(inner) => HirExprKind::Spread(Box::new(self.lower_expr(inner))),

            ExprKind::RouteBlock { routes } => HirExprKind::RouteBlock {
                routes: routes.iter().map(|r| self.lower_expr(r)).collect(),
            },

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

            ExprKind::Ident(name) => {
                // Look up the variable type if tracked
                let kind = HirExprKind::Local { name: name.clone() };
                if let Some(&type_id) = self.var_types.get(name) {
                    return HirExpr::with_type(kind, type_id, expr.span);
                }
                kind
            }

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
                args: args
                    .iter()
                    .map(|a| self.lower_expr_typed(a, registry))
                    .collect(),
            },

            ExprKind::MethodCall {
                object,
                method,
                args,
            } => {
                // Transform HTTP route methods with middleware arguments
                // app.get("/path", middleware, Handler) -> app.getWithMiddleware("/path", "middleware", Handler)
                if Self::is_http_route_method(method) && args.len() > 2 {
                    return self.transform_route_with_middleware_typed(
                        object, method, args, expr.span, registry,
                    );
                }

                // Transform app.group with route block
                if method == "group" && args.len() >= 2 {
                    if let Some(route_block_arg) = args.last() {
                        if matches!(
                            route_block_arg,
                            Expr {
                                kind: ExprKind::RouteBlock { .. },
                                ..
                            }
                        ) {
                            return self.transform_group_with_routes_typed(
                                object, args, expr.span, registry,
                            );
                        }
                    }
                }

                HirExprKind::MethodCall {
                    receiver: Box::new(self.lower_expr_typed(object, registry)),
                    method: method.clone(),
                    args: args
                        .iter()
                        .map(|a| self.lower_expr_typed(a, registry))
                        .collect(),
                }
            }

            ExprKind::Field { object, field } => HirExprKind::Field {
                object: Box::new(self.lower_expr_typed(object, registry)),
                field: field.clone(),
            },

            ExprKind::Index { object, index } => HirExprKind::Index {
                object: Box::new(self.lower_expr_typed(object, registry)),
                index: Box::new(self.lower_expr_typed(index, registry)),
            },

            ExprKind::ArrayLit(elements) => HirExprKind::Array(
                elements
                    .iter()
                    .map(|e| self.lower_expr_typed(e, registry))
                    .collect(),
            ),

            ExprKind::MapLit(entries) => HirExprKind::Map(
                entries
                    .iter()
                    .map(|(k, v)| {
                        (
                            self.lower_expr_typed(k, registry),
                            self.lower_expr_typed(v, registry),
                        )
                    })
                    .collect(),
            ),

            ExprKind::TupleLit(elements) => HirExprKind::Tuple(
                elements
                    .iter()
                    .map(|e| self.lower_expr_typed(e, registry))
                    .collect(),
            ),

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
            } => {
                // Check if this is actually a static method call on a struct
                // e.g., Database::Postgres() should be MethodCall, not EnumVariant
                let type_id = registry.lookup(enum_name);
                let is_struct = type_id
                    .and_then(|tid| registry.get(tid))
                    .map(|info| matches!(info.kind, TypeKind::Struct { .. }))
                    .unwrap_or(false);
                let is_enum = type_id
                    .and_then(|tid| registry.get(tid))
                    .map(|info| matches!(info.kind, TypeKind::Enum { .. }))
                    .unwrap_or(false);

                if is_struct {
                    // Convert to MethodCall: Type::method(args) -> Type.method(args)
                    let receiver = HirExpr::with_type(
                        HirExprKind::Local {
                            name: enum_name.clone(),
                        },
                        type_id.unwrap(),
                        expr.span,
                    );
                    HirExprKind::MethodCall {
                        receiver: Box::new(receiver),
                        method: variant.clone(),
                        args: payload
                            .iter()
                            .map(|e| self.lower_expr_typed(e, registry))
                            .collect(),
                    }
                } else if self
                    .known_qualified_methods
                    .get(enum_name)
                    .map(|methods| methods.contains(variant))
                    .unwrap_or(false)
                    || self.known_functions.contains(variant)
                {
                    // Namespace-qualified function call: Array::Sum(args) -> Sum(args)
                    // or Type::method(args) -> method(args) for associated functions
                    // The parser treats Name::Name(args) as EnumVariant, but when
                    // the "variant" name matches a known function and the "enum_name"
                    // is not a real enum, this is a qualified call through a namespace
                    // import (e.g., `import std::Array` then `Array::Sum(...)`).
                    let func_expr = HirExpr::new(
                        HirExprKind::Global {
                            name: variant.clone(),
                        },
                        expr.span,
                    );
                    HirExprKind::Call {
                        func: Box::new(func_expr),
                        args: payload
                            .iter()
                            .map(|e| self.lower_expr_typed(e, registry))
                            .collect(),
                    }
                } else {
                    // It's a real enum variant
                    HirExprKind::EnumVariant {
                        enum_name: enum_name.clone(),
                        variant: variant.clone(),
                        payload: payload
                            .iter()
                            .map(|e| self.lower_expr_typed(e, registry))
                            .collect(),
                    }
                }
            }

            ExprKind::Range {
                start,
                end,
                inclusive,
            } => HirExprKind::Range {
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
                stmts: stmts
                    .iter()
                    .map(|s| self.lower_stmt_typed(s, registry))
                    .collect(),
                expr: final_expr
                    .as_ref()
                    .map(|e| Box::new(self.lower_expr_typed(e, registry))),
            },

            ExprKind::Ok(values) => {
                let inner = if values.len() == 1 {
                    self.lower_expr_typed(&values[0], registry)
                } else {
                    HirExpr::new(
                        HirExprKind::Tuple(
                            values
                                .iter()
                                .map(|e| self.lower_expr_typed(e, registry))
                                .collect(),
                        ),
                        expr.span,
                    )
                };
                HirExprKind::Ok(Box::new(inner))
            }

            ExprKind::Err(inner) => {
                HirExprKind::Err(Box::new(self.lower_expr_typed(inner, registry)))
            }
            ExprKind::Try(inner) => {
                HirExprKind::Try(Box::new(self.lower_expr_typed(inner, registry)))
            }

            ExprKind::UnwrapOrPanic {
                expr: inner,
                message,
            } => HirExprKind::UnwrapOrPanic {
                expr: Box::new(self.lower_expr_typed(inner, registry)),
                message: Box::new(self.lower_expr_typed(message, registry)),
            },

            ExprKind::Closure {
                params,
                body,
                return_type,
                ..
            } => {
                let mut body_hir = self.lower_expr_typed(body, registry);
                if let Some(ret_type) = return_type {
                    body_hir.type_id = Some(self.resolve_type_expr(ret_type, registry));
                }
                HirExprKind::Closure {
                    params: params
                        .iter()
                        .map(|(n, t)| {
                            (
                                n.clone(),
                                t.as_ref().map(|tt| self.resolve_type_expr(tt, registry)),
                            )
                        })
                        .collect(),
                    body: Box::new(body_hir),
                }
            }

            ExprKind::Match { values, arms } => {
                let lowered_values: Vec<HirExpr> = values
                    .iter()
                    .map(|v| self.lower_expr_typed(v, registry))
                    .collect();
                let lowered_arms: Vec<HirMatchArm> = arms
                    .iter()
                    .map(|a| {
                        let mut pattern = self.lower_match_pattern_typed(&a.pattern, registry);
                        // Convert struct literal patterns to field-by-field comparisons
                        if let HirMatchPattern::Condition(ref cond_expr) = pattern {
                            if let HirExprKind::Struct { fields, .. } = &cond_expr.kind {
                                if let Some(matched_val) = lowered_values.first() {
                                    pattern = self.struct_pattern_to_condition(
                                        matched_val,
                                        fields,
                                        cond_expr.span,
                                    );
                                }
                            }
                        }
                        HirMatchArm {
                            pattern,
                            guard: a.guard.as_ref().map(|g| self.lower_expr_typed(g, registry)),
                            body: self.lower_expr_typed(&a.body, registry),
                            span: a.span,
                        }
                    })
                    .collect();
                HirExprKind::Match {
                    values: lowered_values,
                    arms: lowered_arms,
                }
            }

            ExprKind::Spread(inner) => {
                HirExprKind::Spread(Box::new(self.lower_expr_typed(inner, registry)))
            }

            ExprKind::RouteBlock { routes } => HirExprKind::RouteBlock {
                routes: routes
                    .iter()
                    .map(|r| self.lower_expr_typed(r, registry))
                    .collect(),
            },

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

            ExprKind::Cast {
                expr: inner,
                target,
            } => {
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
                // Get element type, handling Spread elements specially
                let elem_type = self.common_array_elem_type(elements, registry);
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
                out.type_id = Some(
                    registry
                        .lookup(name)
                        .unwrap_or_else(|| registry.declare_named(name)),
                );
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
            HirExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                // Check if this is a module method call (e.g., JSON.parse, JSON.stringify)
                if let Some(return_type) =
                    self.infer_module_method_type(receiver, method, args, registry)
                {
                    out.type_id = Some(return_type);
                } else {
                    let receiver_type = receiver.type_id.unwrap_or(builtin::ANY);
                    if let Some(return_type) =
                        self.infer_method_call_type(receiver_type, method, args, registry)
                    {
                        out.type_id = Some(return_type);
                    }
                }
            }
            HirExprKind::Cast { to_type, .. } => {
                out.type_id = Some(*to_type);
            }
            HirExprKind::UnaryOp { op, operand } => {
                // Infer type for unary operations (e.g., -7 should be Int)
                let operand_type = operand.type_id.unwrap_or(builtin::ANY);
                let op_kind = hir_unaryop_to_kind(*op);
                out.type_id = Some(infer_unaryop_result_type(op_kind, operand_type));
            }
            HirExprKind::BinOp { op, lhs, rhs } => {
                // Infer type for binary operations
                let lhs_type = lhs.type_id.unwrap_or(builtin::ANY);
                let rhs_type = rhs.type_id.unwrap_or(builtin::ANY);
                let op_kind = hir_binop_to_kind(*op);
                out.type_id = Some(infer_binop_result_type(op_kind, lhs_type, rhs_type));
            }
            HirExprKind::Field { object, field } => {
                // Infer field access type from struct type
                if let Some(obj_type) = object.type_id {
                    if let Some(info) = registry.get(obj_type) {
                        if let TypeKind::Struct { fields, .. } = &info.kind {
                            if let Some((_, field_type, _)) =
                                fields.iter().find(|(n, _, _)| n == field)
                            {
                                out.type_id = Some(*field_type);
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        out
    }

    // ========================================================================
    // Helpers
    // ========================================================================

    // ========================================================================
    // For-Loop Desugaring
    // ========================================================================

    /// Lower a for-loop to HIR.
    ///
    /// ## Desugaring Rules
    ///
    /// ### Range iteration: `for i in start..end { body }`
    /// ```text
    /// let __i = start
    /// while __i < end {
    ///     let i = __i
    ///     body
    ///     __i = __i + 1
    /// }
    /// ```
    ///
    /// ### Array iteration: `for x in array { body }`
    /// ```text
    /// let __arr = array
    /// let __idx = 0
    /// while __idx < __arr.len() {
    ///     let x = __arr[__idx]
    ///     body
    ///     __idx = __idx + 1
    /// }
    /// ```
    ///
    /// ### Array iteration with index: `for i, x in array { body }`
    /// ```text
    /// let __arr = array
    /// let __idx = 0
    /// while __idx < __arr.len() {
    ///     let i = __idx
    ///     let x = __arr[__idx]
    ///     body
    ///     __idx = __idx + 1
    /// }
    /// ```
    fn lower_for_loop(
        &mut self,
        pattern: &Pattern,
        iterable: Option<&Expr>,
        body: &[Stmt],
        span: Span,
    ) -> HirStmtKind {
        // No iterable = infinite loop
        let Some(iter_expr) = iterable else {
            let body_stmts: Vec<_> = body.iter().map(|s| self.lower_stmt(s)).collect();
            return HirStmtKind::While {
                condition: HirExpr::new(HirExprKind::Const(ConstValue::Bool(true)), span),
                body: body_stmts,
            };
        };

        // Check if iterating over a range expression
        if let ExprKind::Range {
            start,
            end,
            inclusive,
        } = &iter_expr.kind
        {
            return self.lower_range_for_loop(pattern, start, end, *inclusive, body, span);
        }

        // Check if iterating over a map literal
        if matches!(&iter_expr.kind, ExprKind::MapLit(_)) {
            return self.lower_map_for_loop(pattern, iter_expr, body, span);
        }

        // Array/collection iteration
        self.lower_array_for_loop(pattern, iter_expr, body, span)
    }

    /// Lower range-based for-loop: `for i in start..end`
    fn lower_range_for_loop(
        &mut self,
        pattern: &Pattern,
        start: &Expr,
        end: &Expr,
        inclusive: bool,
        body: &[Stmt],
        span: Span,
    ) -> HirStmtKind {
        let iter_var = self.pattern_to_name(pattern);
        let internal_idx = format!("__{}_idx", iter_var);

        // Lower body statements
        let mut body_stmts: Vec<_> = body.iter().map(|s| self.lower_stmt(s)).collect();

        // Prepend: let iter_var = __idx
        body_stmts.insert(
            0,
            HirStmt::new(
                HirStmtKind::Let {
                    name: iter_var.clone(),
                    type_id: None,
                    value: HirExpr::new(
                        HirExprKind::Local {
                            name: internal_idx.clone(),
                        },
                        span,
                    ),
                    mutable: false,
                    ownership: Ownership::Owned,
                },
                span,
            ),
        );

        // Append: __idx = __idx + 1
        body_stmts.push(HirStmt::new(
            HirStmtKind::Assign {
                target: HirExpr::new(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    span,
                ),
                value: HirExpr::new(
                    HirExprKind::BinOp {
                        op: HirBinOp::Add,
                        lhs: Box::new(HirExpr::new(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            span,
                        )),
                        rhs: Box::new(HirExpr::new(HirExprKind::Const(ConstValue::Int(1)), span)),
                    },
                    span,
                ),
            },
            span,
        ));

        // Condition: __idx < end (or __idx <= end for inclusive)
        let cmp_op = if inclusive {
            HirBinOp::LtEq
        } else {
            HirBinOp::Lt
        };
        let condition = HirExpr::new(
            HirExprKind::BinOp {
                op: cmp_op,
                lhs: Box::new(HirExpr::new(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    span,
                )),
                rhs: Box::new(self.lower_expr(end)),
            },
            span,
        );

        // Build desugared block:
        // {
        //     let __idx = start
        //     while __idx < end { let i = __idx; body; __idx++ }
        // }
        let init_stmt = HirStmt::new(
            HirStmtKind::Let {
                name: internal_idx,
                type_id: None,
                value: self.lower_expr(start),
                mutable: true,
                ownership: Ownership::Owned,
            },
            span,
        );

        let while_stmt = HirStmt::new(
            HirStmtKind::While {
                condition,
                body: body_stmts,
            },
            span,
        );

        // Return as block expression containing init + while
        HirStmtKind::Expr(HirExpr::new(
            HirExprKind::Block {
                stmts: vec![init_stmt, while_stmt],
                expr: None,
            },
            span,
        ))
    }

    /// Lower array-based for-loop: `for x in array` or `for i, x in array`
    fn lower_array_for_loop(
        &mut self,
        pattern: &Pattern,
        array_expr: &Expr,
        body: &[Stmt],
        span: Span,
    ) -> HirStmtKind {
        // Check if pattern is tuple (i, x) or single (x)
        let (index_var, elem_var) = match &pattern.kind {
            PatternKind::Tuple(patterns) if patterns.len() == 2 => {
                let idx = self.pattern_to_name(&patterns[0]);
                let elem = self.pattern_to_name(&patterns[1]);
                (Some(idx), elem)
            }
            _ => (None, self.pattern_to_name(pattern)),
        };

        let internal_idx = format!("__{}_idx", elem_var);
        let internal_arr = format!("__{}_arr", elem_var);

        // Lower body statements
        let mut body_stmts: Vec<_> = body.iter().map(|s| self.lower_stmt(s)).collect();

        // Prepend index assignment if pattern has index: let i = __idx
        if let Some(idx_name) = &index_var {
            body_stmts.insert(
                0,
                HirStmt::new(
                    HirStmtKind::Let {
                        name: idx_name.clone(),
                        type_id: None,
                        value: HirExpr::new(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            span,
                        ),
                        mutable: false,
                        ownership: Ownership::Owned,
                    },
                    span,
                ),
            );
        }

        // Prepend element extraction: let x = __arr[__idx]
        let elem_extraction = HirStmt::new(
            HirStmtKind::Let {
                name: elem_var.clone(),
                type_id: None,
                value: HirExpr::new(
                    HirExprKind::Index {
                        object: Box::new(HirExpr::new(
                            HirExprKind::Local {
                                name: internal_arr.clone(),
                            },
                            span,
                        )),
                        index: Box::new(HirExpr::new(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            span,
                        )),
                    },
                    span,
                ),
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );
        // Insert after index assignment if present, else at start
        let insert_pos = if index_var.is_some() { 1 } else { 0 };
        body_stmts.insert(insert_pos, elem_extraction);

        // Append: __idx = __idx + 1
        body_stmts.push(HirStmt::new(
            HirStmtKind::Assign {
                target: HirExpr::new(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    span,
                ),
                value: HirExpr::new(
                    HirExprKind::BinOp {
                        op: HirBinOp::Add,
                        lhs: Box::new(HirExpr::new(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            span,
                        )),
                        rhs: Box::new(HirExpr::new(HirExprKind::Const(ConstValue::Int(1)), span)),
                    },
                    span,
                ),
            },
            span,
        ));

        // Condition: __idx < __arr.len()
        let condition = HirExpr::new(
            HirExprKind::BinOp {
                op: HirBinOp::Lt,
                lhs: Box::new(HirExpr::new(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    span,
                )),
                rhs: Box::new(HirExpr::new(
                    HirExprKind::MethodCall {
                        receiver: Box::new(HirExpr::new(
                            HirExprKind::Local {
                                name: internal_arr.clone(),
                            },
                            span,
                        )),
                        method: "len".to_string(),
                        args: vec![],
                    },
                    span,
                )),
            },
            span,
        );

        // Build desugared block:
        // {
        //     let __arr = array
        //     let __idx = 0
        //     while __idx < __arr.len() { ... }
        // }
        let arr_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_arr,
                type_id: None,
                value: self.lower_expr(array_expr),
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );

        let idx_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_idx,
                type_id: None,
                value: HirExpr::new(HirExprKind::Const(ConstValue::Int(0)), span),
                mutable: true,
                ownership: Ownership::Owned,
            },
            span,
        );

        let while_stmt = HirStmt::new(
            HirStmtKind::While {
                condition,
                body: body_stmts,
            },
            span,
        );

        HirStmtKind::Expr(HirExpr::new(
            HirExprKind::Block {
                stmts: vec![arr_init, idx_init, while_stmt],
                expr: None,
            },
            span,
        ))
    }

    /// Lower map-based for-loop: `for key, value in map` or `for key in map`
    ///
    /// Desugars to:
    /// ```text
    /// let __map = map
    /// let __keys = __map.keys()
    /// let __idx = 0
    /// while __idx < __keys.len() {
    ///     let key = __keys[__idx]
    ///     let value = __map.get(key)
    ///     body
    ///     __idx = __idx + 1
    /// }
    /// ```
    fn lower_map_for_loop(
        &mut self,
        pattern: &Pattern,
        map_expr: &Expr,
        body: &[Stmt],
        span: Span,
    ) -> HirStmtKind {
        // Check if pattern is tuple (key, value) or single (key)
        let (key_var, value_var) = match &pattern.kind {
            PatternKind::Tuple(patterns) if patterns.len() == 2 => {
                let key = self.pattern_to_name(&patterns[0]);
                let val = self.pattern_to_name(&patterns[1]);
                (key, Some(val))
            }
            _ => (self.pattern_to_name(pattern), None),
        };

        // Generate unique internal variable names to avoid conflicts with multiple loops
        let uid = self.unique_suffix();
        let internal_map = format!("__{}map{}", key_var, uid);
        let internal_keys = format!("__{}keys{}", key_var, uid);
        let internal_idx = format!("__{}idx{}", key_var, uid);

        // Lower body statements
        let mut body_stmts: Vec<_> = body.iter().map(|s| self.lower_stmt(s)).collect();

        // Prepend: let key = __keys[__idx]
        body_stmts.insert(
            0,
            HirStmt::new(
                HirStmtKind::Let {
                    name: key_var.clone(),
                    type_id: None,
                    value: HirExpr::new(
                        HirExprKind::Index {
                            object: Box::new(HirExpr::new(
                                HirExprKind::Local {
                                    name: internal_keys.clone(),
                                },
                                span,
                            )),
                            index: Box::new(HirExpr::new(
                                HirExprKind::Local {
                                    name: internal_idx.clone(),
                                },
                                span,
                            )),
                        },
                        span,
                    ),
                    mutable: false,
                    ownership: Ownership::Owned,
                },
                span,
            ),
        );

        // Prepend: let value = __map.get(key) if we have a value variable
        if let Some(val_name) = &value_var {
            body_stmts.insert(
                1,
                HirStmt::new(
                    HirStmtKind::Let {
                        name: val_name.clone(),
                        type_id: None,
                        value: HirExpr::new(
                            HirExprKind::Index {
                                object: Box::new(HirExpr::new(
                                    HirExprKind::Local {
                                        name: internal_map.clone(),
                                    },
                                    span,
                                )),
                                index: Box::new(HirExpr::new(
                                    HirExprKind::Local {
                                        name: key_var.clone(),
                                    },
                                    span,
                                )),
                            },
                            span,
                        ),
                        mutable: false,
                        ownership: Ownership::Owned,
                    },
                    span,
                ),
            );
        }

        // Append: __idx = __idx + 1
        body_stmts.push(HirStmt::new(
            HirStmtKind::Assign {
                target: HirExpr::new(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    span,
                ),
                value: HirExpr::new(
                    HirExprKind::BinOp {
                        op: HirBinOp::Add,
                        lhs: Box::new(HirExpr::new(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            span,
                        )),
                        rhs: Box::new(HirExpr::new(HirExprKind::Const(ConstValue::Int(1)), span)),
                    },
                    span,
                ),
            },
            span,
        ));

        // Condition: __idx < __keys.len()
        let condition = HirExpr::new(
            HirExprKind::BinOp {
                op: HirBinOp::Lt,
                lhs: Box::new(HirExpr::new(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    span,
                )),
                rhs: Box::new(HirExpr::new(
                    HirExprKind::MethodCall {
                        receiver: Box::new(HirExpr::new(
                            HirExprKind::Local {
                                name: internal_keys.clone(),
                            },
                            span,
                        )),
                        method: "len".to_string(),
                        args: vec![],
                    },
                    span,
                )),
            },
            span,
        );

        // Build desugared block:
        // {
        //     let __map = map
        //     let __keys = __map.keys()
        //     let __idx = 0
        //     while __idx < __keys.len() { ... }
        // }
        let map_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_map.clone(),
                type_id: None,
                value: self.lower_expr(map_expr),
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );

        let keys_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_keys,
                type_id: None,
                value: HirExpr::new(
                    HirExprKind::MethodCall {
                        receiver: Box::new(HirExpr::new(
                            HirExprKind::Local { name: internal_map },
                            span,
                        )),
                        method: "keys".to_string(),
                        args: vec![],
                    },
                    span,
                ),
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );

        let idx_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_idx,
                type_id: None,
                value: HirExpr::new(HirExprKind::Const(ConstValue::Int(0)), span),
                mutable: true,
                ownership: Ownership::Owned,
            },
            span,
        );

        let while_stmt = HirStmt::new(
            HirStmtKind::While {
                condition,
                body: body_stmts,
            },
            span,
        );

        HirStmtKind::Expr(HirExpr::new(
            HirExprKind::Block {
                stmts: vec![map_init, keys_init, idx_init, while_stmt],
                expr: None,
            },
            span,
        ))
    }

    /// Lower a for-loop with type information.
    fn lower_for_loop_typed(
        &mut self,
        pattern: &Pattern,
        iterable: Option<&Expr>,
        body: &[Stmt],
        span: Span,
        registry: &mut TypeRegistry,
    ) -> HirStmtKind {
        // No iterable = infinite loop
        let Some(iter_expr) = iterable else {
            let body_stmts: Vec<_> = body
                .iter()
                .map(|s| self.lower_stmt_typed(s, registry))
                .collect();
            return HirStmtKind::While {
                condition: HirExpr::with_type(
                    HirExprKind::Const(ConstValue::Bool(true)),
                    builtin::BOOL,
                    span,
                ),
                body: body_stmts,
            };
        };

        // Check if iterating over a range expression
        if let ExprKind::Range {
            start,
            end,
            inclusive,
        } = &iter_expr.kind
        {
            return self
                .lower_range_for_loop_typed(pattern, start, end, *inclusive, body, span, registry);
        }

        // Check if iterating over a map - either by literal or by type
        // First check for map literal
        if matches!(&iter_expr.kind, ExprKind::MapLit(_)) {
            return self.lower_map_for_loop_typed(pattern, iter_expr, body, span, registry);
        }

        // Then check if the iterable expression has a Map type
        // We need to lower it first to get its type
        let lowered = self.lower_expr_typed(iter_expr, registry);
        let is_map = lowered.type_id.map_or(false, |tid| {
            registry
                .get(tid)
                .map_or(false, |info| matches!(info.kind, TypeKind::Map { .. }))
        });

        if is_map {
            // Re-lower using map-specific lowering
            // Since we already lowered, we can use that result
            return self
                .lower_map_for_loop_typed_with_lowered(pattern, lowered, body, span, registry);
        }

        // Array/collection iteration
        self.lower_array_for_loop_typed(pattern, iter_expr, body, span, registry)
    }

    /// Lower range-based for-loop with type info: `for i in start..end`
    fn lower_range_for_loop_typed(
        &mut self,
        pattern: &Pattern,
        start: &Expr,
        end: &Expr,
        inclusive: bool,
        body: &[Stmt],
        span: Span,
        registry: &mut TypeRegistry,
    ) -> HirStmtKind {
        let iter_var = self.pattern_to_name(pattern);
        let internal_idx = format!("__{}_idx", iter_var);

        // Lower body statements
        let mut body_stmts: Vec<_> = body
            .iter()
            .map(|s| self.lower_stmt_typed(s, registry))
            .collect();

        // Prepend: let iter_var = __idx
        body_stmts.insert(
            0,
            HirStmt::new(
                HirStmtKind::Let {
                    name: iter_var.clone(),
                    type_id: Some(builtin::INT),
                    value: HirExpr::with_type(
                        HirExprKind::Local {
                            name: internal_idx.clone(),
                        },
                        builtin::INT,
                        span,
                    ),
                    mutable: false,
                    ownership: Ownership::Owned,
                },
                span,
            ),
        );

        // Append: __idx = __idx + 1
        body_stmts.push(HirStmt::new(
            HirStmtKind::Assign {
                target: HirExpr::with_type(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    builtin::INT,
                    span,
                ),
                value: HirExpr::with_type(
                    HirExprKind::BinOp {
                        op: HirBinOp::Add,
                        lhs: Box::new(HirExpr::with_type(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            builtin::INT,
                            span,
                        )),
                        rhs: Box::new(HirExpr::with_type(
                            HirExprKind::Const(ConstValue::Int(1)),
                            builtin::INT,
                            span,
                        )),
                    },
                    builtin::INT,
                    span,
                ),
            },
            span,
        ));

        // Condition: __idx < end (or __idx <= end for inclusive)
        let cmp_op = if inclusive {
            HirBinOp::LtEq
        } else {
            HirBinOp::Lt
        };
        let condition = HirExpr::with_type(
            HirExprKind::BinOp {
                op: cmp_op,
                lhs: Box::new(HirExpr::with_type(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    builtin::INT,
                    span,
                )),
                rhs: Box::new(self.lower_expr_typed(end, registry)),
            },
            builtin::BOOL,
            span,
        );

        // Build desugared block
        let init_stmt = HirStmt::new(
            HirStmtKind::Let {
                name: internal_idx,
                type_id: Some(builtin::INT),
                value: self.lower_expr_typed(start, registry),
                mutable: true,
                ownership: Ownership::Owned,
            },
            span,
        );

        let while_stmt = HirStmt::new(
            HirStmtKind::While {
                condition,
                body: body_stmts,
            },
            span,
        );

        HirStmtKind::Expr(HirExpr::new(
            HirExprKind::Block {
                stmts: vec![init_stmt, while_stmt],
                expr: None,
            },
            span,
        ))
    }

    /// Lower array-based for-loop with type info: `for x in array` or `for i, x in array`
    fn lower_array_for_loop_typed(
        &mut self,
        pattern: &Pattern,
        array_expr: &Expr,
        body: &[Stmt],
        span: Span,
        registry: &mut TypeRegistry,
    ) -> HirStmtKind {
        // Check if pattern is tuple (i, x) or single (x)
        let (index_var, elem_var) = match &pattern.kind {
            PatternKind::Tuple(patterns) if patterns.len() == 2 => {
                let idx = self.pattern_to_name(&patterns[0]);
                let elem = self.pattern_to_name(&patterns[1]);
                (Some(idx), elem)
            }
            _ => (None, self.pattern_to_name(pattern)),
        };

        let internal_idx = format!("__{}_idx", elem_var);
        let internal_arr = format!("__{}_arr", elem_var);

        // Lower the array expression to get its type
        let lowered_arr = self.lower_expr_typed(array_expr, registry);
        let arr_type = lowered_arr.type_id;

        // Infer element type from array type
        let elem_type = arr_type.and_then(|tid| {
            registry.get(tid).and_then(|info| match &info.kind {
                TypeKind::Array { element } => Some(*element),
                _ => None,
            })
        });

        // Lower body statements
        let mut body_stmts: Vec<_> = body
            .iter()
            .map(|s| self.lower_stmt_typed(s, registry))
            .collect();

        // Prepend index assignment if pattern has index: let i = __idx
        if let Some(idx_name) = &index_var {
            body_stmts.insert(
                0,
                HirStmt::new(
                    HirStmtKind::Let {
                        name: idx_name.clone(),
                        type_id: Some(builtin::INT),
                        value: HirExpr::with_type(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            builtin::INT,
                            span,
                        ),
                        mutable: false,
                        ownership: Ownership::Owned,
                    },
                    span,
                ),
            );
        }

        // Prepend element extraction: let x = __arr[__idx]
        // Create array reference with proper type
        let arr_ref = if let Some(t) = arr_type {
            HirExpr::with_type(
                HirExprKind::Local {
                    name: internal_arr.clone(),
                },
                t,
                span,
            )
        } else {
            HirExpr::new(
                HirExprKind::Local {
                    name: internal_arr.clone(),
                },
                span,
            )
        };

        // Create index expression with proper element type
        let index_expr = if let Some(t) = elem_type {
            HirExpr::with_type(
                HirExprKind::Index {
                    object: Box::new(arr_ref.clone()),
                    index: Box::new(HirExpr::with_type(
                        HirExprKind::Local {
                            name: internal_idx.clone(),
                        },
                        builtin::INT,
                        span,
                    )),
                },
                t,
                span,
            )
        } else {
            HirExpr::new(
                HirExprKind::Index {
                    object: Box::new(arr_ref.clone()),
                    index: Box::new(HirExpr::with_type(
                        HirExprKind::Local {
                            name: internal_idx.clone(),
                        },
                        builtin::INT,
                        span,
                    )),
                },
                span,
            )
        };

        let elem_extraction = HirStmt::new(
            HirStmtKind::Let {
                name: elem_var.clone(),
                type_id: elem_type,
                value: index_expr,
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );
        let insert_pos = if index_var.is_some() { 1 } else { 0 };
        body_stmts.insert(insert_pos, elem_extraction);

        // Append: __idx = __idx + 1
        body_stmts.push(HirStmt::new(
            HirStmtKind::Assign {
                target: HirExpr::with_type(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    builtin::INT,
                    span,
                ),
                value: HirExpr::with_type(
                    HirExprKind::BinOp {
                        op: HirBinOp::Add,
                        lhs: Box::new(HirExpr::with_type(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            builtin::INT,
                            span,
                        )),
                        rhs: Box::new(HirExpr::with_type(
                            HirExprKind::Const(ConstValue::Int(1)),
                            builtin::INT,
                            span,
                        )),
                    },
                    builtin::INT,
                    span,
                ),
            },
            span,
        ));

        // Condition: __idx < __arr.len()
        // Create array reference with proper type for len() call
        let arr_ref_for_len = if let Some(t) = arr_type {
            HirExpr::with_type(
                HirExprKind::Local {
                    name: internal_arr.clone(),
                },
                t,
                span,
            )
        } else {
            HirExpr::new(
                HirExprKind::Local {
                    name: internal_arr.clone(),
                },
                span,
            )
        };

        let condition = HirExpr::with_type(
            HirExprKind::BinOp {
                op: HirBinOp::Lt,
                lhs: Box::new(HirExpr::with_type(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    builtin::INT,
                    span,
                )),
                rhs: Box::new(HirExpr::with_type(
                    HirExprKind::MethodCall {
                        receiver: Box::new(arr_ref_for_len),
                        method: "len".to_string(),
                        args: vec![],
                    },
                    builtin::INT,
                    span,
                )),
            },
            builtin::BOOL,
            span,
        );

        // Build desugared block
        let arr_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_arr,
                type_id: arr_type,
                value: lowered_arr,
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );

        let idx_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_idx,
                type_id: Some(builtin::INT),
                value: HirExpr::with_type(
                    HirExprKind::Const(ConstValue::Int(0)),
                    builtin::INT,
                    span,
                ),
                mutable: true,
                ownership: Ownership::Owned,
            },
            span,
        );

        let while_stmt = HirStmt::new(
            HirStmtKind::While {
                condition,
                body: body_stmts,
            },
            span,
        );

        HirStmtKind::Expr(HirExpr::new(
            HirExprKind::Block {
                stmts: vec![arr_init, idx_init, while_stmt],
                expr: None,
            },
            span,
        ))
    }

    /// Lower map-based for-loop with type info: `for key, value in map` or `for key in map`
    fn lower_map_for_loop_typed(
        &mut self,
        pattern: &Pattern,
        map_expr: &Expr,
        body: &[Stmt],
        span: Span,
        registry: &mut TypeRegistry,
    ) -> HirStmtKind {
        // Check if pattern is tuple (key, value) or single (key)
        let (orig_key_var, orig_value_var) = match &pattern.kind {
            PatternKind::Tuple(patterns) if patterns.len() == 2 => {
                let key = self.pattern_to_name(&patterns[0]);
                let val = self.pattern_to_name(&patterns[1]);
                (key, Some(val))
            }
            _ => (self.pattern_to_name(pattern), None),
        };

        // Generate unique internal variable names to avoid conflicts with multiple loops
        let uid = self.unique_suffix();
        let internal_map = format!("__{}map{}", orig_key_var, uid);
        let internal_keys = format!("__{}keys{}", orig_key_var, uid);
        let internal_idx = format!("__{}idx{}", orig_key_var, uid);

        // Also make the iteration variables unique to avoid type conflicts
        // Use k/v prefixes so key and value don't collide when both are _ (wildcard)
        let key_var = format!("__k{}_{}", orig_key_var, uid);
        let value_var = orig_value_var.as_ref().map(|v| format!("__v{}_{}", v, uid));

        // Lower the map expression FIRST to get its type for proper propagation in body
        let lowered_map_early = self.lower_expr_typed(map_expr, registry);
        let map_type_early = lowered_map_early.type_id;

        // Lower body statements, substituting the original variable names with unique ones
        let mut body_stmts: Vec<_> = body
            .iter()
            .map(|s| {
                let mut lowered = self.lower_stmt_typed(s, registry);
                // Substitute variable references
                self.substitute_local_in_stmt(&mut lowered, &orig_key_var, &key_var);
                if let (Some(ref orig_val), Some(ref new_val)) = (&orig_value_var, &value_var) {
                    self.substitute_local_in_stmt(&mut lowered, orig_val, new_val);
                }
                lowered
            })
            .collect();

        // Prepend: let key = __keys[__idx]
        body_stmts.insert(
            0,
            HirStmt::new(
                HirStmtKind::Let {
                    name: key_var.clone(),
                    type_id: None,
                    value: HirExpr::new(
                        HirExprKind::Index {
                            object: Box::new(HirExpr::new(
                                HirExprKind::Local {
                                    name: internal_keys.clone(),
                                },
                                span,
                            )),
                            index: Box::new(HirExpr::with_type(
                                HirExprKind::Local {
                                    name: internal_idx.clone(),
                                },
                                builtin::INT,
                                span,
                            )),
                        },
                        span,
                    ),
                    mutable: false,
                    ownership: Ownership::Owned,
                },
                span,
            ),
        );

        // Prepend: let value = __map[key] if we have a value variable
        if let Some(val_name) = &value_var {
            body_stmts.insert(
                1,
                HirStmt::new(
                    HirStmtKind::Let {
                        name: val_name.clone(),
                        type_id: None,
                        value: HirExpr::new(
                            HirExprKind::Index {
                                object: Box::new(match map_type_early {
                                    Some(t) => HirExpr::with_type(
                                        HirExprKind::Local {
                                            name: internal_map.clone(),
                                        },
                                        t,
                                        span,
                                    ),
                                    None => HirExpr::new(
                                        HirExprKind::Local {
                                            name: internal_map.clone(),
                                        },
                                        span,
                                    ),
                                }),
                                index: Box::new(HirExpr::new(
                                    HirExprKind::Local {
                                        name: key_var.clone(),
                                    },
                                    span,
                                )),
                            },
                            span,
                        ),
                        mutable: false,
                        ownership: Ownership::Owned,
                    },
                    span,
                ),
            );
        }

        // Append: __idx = __idx + 1
        body_stmts.push(HirStmt::new(
            HirStmtKind::Assign {
                target: HirExpr::with_type(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    builtin::INT,
                    span,
                ),
                value: HirExpr::with_type(
                    HirExprKind::BinOp {
                        op: HirBinOp::Add,
                        lhs: Box::new(HirExpr::with_type(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            builtin::INT,
                            span,
                        )),
                        rhs: Box::new(HirExpr::with_type(
                            HirExprKind::Const(ConstValue::Int(1)),
                            builtin::INT,
                            span,
                        )),
                    },
                    builtin::INT,
                    span,
                ),
            },
            span,
        ));

        // Condition: __idx < __keys.len()
        let condition = HirExpr::with_type(
            HirExprKind::BinOp {
                op: HirBinOp::Lt,
                lhs: Box::new(HirExpr::with_type(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    builtin::INT,
                    span,
                )),
                rhs: Box::new(HirExpr::with_type(
                    HirExprKind::MethodCall {
                        receiver: Box::new(HirExpr::new(
                            HirExprKind::Local {
                                name: internal_keys.clone(),
                            },
                            span,
                        )),
                        method: "len".to_string(),
                        args: vec![],
                    },
                    builtin::INT,
                    span,
                )),
            },
            builtin::BOOL,
            span,
        );

        // Build desugared block - use the already lowered map expression
        let map_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_map.clone(),
                type_id: map_type_early,
                value: lowered_map_early.clone(),
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );

        let keys_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_keys.clone(),
                type_id: None,
                value: {
                    // CRITICAL: The receiver must have the map type set for proper type propagation
                    let receiver = match map_type_early {
                        Some(t) => HirExpr::with_type(
                            HirExprKind::Local {
                                name: internal_map.clone(),
                            },
                            t,
                            span,
                        ),
                        None => HirExpr::new(
                            HirExprKind::Local {
                                name: internal_map.clone(),
                            },
                            span,
                        ),
                    };
                    let keys_call = HirExpr::new(
                        HirExprKind::MethodCall {
                            receiver: Box::new(receiver),
                            method: "keys".to_string(),
                            args: vec![],
                        },
                        span,
                    );

                    // Infer the return type of keys() method
                    if let Some(map_ty) = map_type_early {
                        if let Some(keys_ty) =
                            self.infer_method_call_type(map_ty, "keys", &mut [], registry)
                        {
                            HirExpr::with_type(keys_call.kind, keys_ty, span)
                        } else {
                            keys_call
                        }
                    } else {
                        keys_call
                    }
                },
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );

        let idx_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_idx,
                type_id: Some(builtin::INT),
                value: HirExpr::with_type(
                    HirExprKind::Const(ConstValue::Int(0)),
                    builtin::INT,
                    span,
                ),
                mutable: true,
                ownership: Ownership::Owned,
            },
            span,
        );

        let while_stmt = HirStmt::new(
            HirStmtKind::While {
                condition,
                body: body_stmts,
            },
            span,
        );

        HirStmtKind::Expr(HirExpr::new(
            HirExprKind::Block {
                stmts: vec![map_init, keys_init, idx_init, while_stmt],
                expr: None,
            },
            span,
        ))
    }

    /// Lower map-based for-loop with an already-lowered map expression
    /// This variant is used when we detect the map type after lowering the expression
    fn lower_map_for_loop_typed_with_lowered(
        &mut self,
        pattern: &Pattern,
        lowered_map: HirExpr,
        body: &[Stmt],
        span: Span,
        registry: &mut TypeRegistry,
    ) -> HirStmtKind {
        // Check if pattern is tuple (key, value) or single (key)
        let (orig_key_var, orig_value_var) = match &pattern.kind {
            PatternKind::Tuple(patterns) if patterns.len() == 2 => {
                let key = self.pattern_to_name(&patterns[0]);
                let val = self.pattern_to_name(&patterns[1]);
                (key, Some(val))
            }
            _ => (self.pattern_to_name(pattern), None),
        };

        // Generate unique internal variable names to avoid conflicts with multiple loops
        let uid = self.unique_suffix();
        let internal_map = format!("__{}map{}", orig_key_var, uid);
        let internal_keys = format!("__{}keys{}", orig_key_var, uid);
        let internal_idx = format!("__{}idx{}", orig_key_var, uid);

        // Also make the iteration variables unique to avoid type conflicts
        // Use k/v prefixes so key and value don't collide when both are _ (wildcard)
        let key_var = format!("__k{}_{}", orig_key_var, uid);
        let value_var = orig_value_var.as_ref().map(|v| format!("__v{}_{}", v, uid));

        // Get the map type from the already-lowered map expression
        let map_type = lowered_map.type_id;

        // Lower body statements, substituting the original variable names with unique ones
        let mut body_stmts: Vec<_> = body
            .iter()
            .map(|s| {
                let mut lowered = self.lower_stmt_typed(s, registry);
                // Substitute variable references
                self.substitute_local_in_stmt(&mut lowered, &orig_key_var, &key_var);
                if let (Some(ref orig_val), Some(ref new_val)) = (&orig_value_var, &value_var) {
                    self.substitute_local_in_stmt(&mut lowered, orig_val, new_val);
                }
                lowered
            })
            .collect();

        // Prepend: let key = __keys[__idx]
        body_stmts.insert(
            0,
            HirStmt::new(
                HirStmtKind::Let {
                    name: key_var.clone(),
                    type_id: None,
                    value: HirExpr::new(
                        HirExprKind::Index {
                            object: Box::new(HirExpr::new(
                                HirExprKind::Local {
                                    name: internal_keys.clone(),
                                },
                                span,
                            )),
                            index: Box::new(HirExpr::with_type(
                                HirExprKind::Local {
                                    name: internal_idx.clone(),
                                },
                                builtin::INT,
                                span,
                            )),
                        },
                        span,
                    ),
                    mutable: false,
                    ownership: Ownership::Owned,
                },
                span,
            ),
        );

        // Prepend: let value = __map[key] if we have a value variable
        if let Some(val_name) = &value_var {
            body_stmts.insert(
                1,
                HirStmt::new(
                    HirStmtKind::Let {
                        name: val_name.clone(),
                        type_id: None,
                        value: HirExpr::new(
                            HirExprKind::Index {
                                object: Box::new(match map_type {
                                    Some(t) => HirExpr::with_type(
                                        HirExprKind::Local {
                                            name: internal_map.clone(),
                                        },
                                        t,
                                        span,
                                    ),
                                    None => HirExpr::new(
                                        HirExprKind::Local {
                                            name: internal_map.clone(),
                                        },
                                        span,
                                    ),
                                }),
                                index: Box::new(HirExpr::new(
                                    HirExprKind::Local {
                                        name: key_var.clone(),
                                    },
                                    span,
                                )),
                            },
                            span,
                        ),
                        mutable: false,
                        ownership: Ownership::Owned,
                    },
                    span,
                ),
            );
        }

        // Append: __idx = __idx + 1
        body_stmts.push(HirStmt::new(
            HirStmtKind::Assign {
                target: HirExpr::with_type(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    builtin::INT,
                    span,
                ),
                value: HirExpr::with_type(
                    HirExprKind::BinOp {
                        op: HirBinOp::Add,
                        lhs: Box::new(HirExpr::with_type(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            builtin::INT,
                            span,
                        )),
                        rhs: Box::new(HirExpr::with_type(
                            HirExprKind::Const(ConstValue::Int(1)),
                            builtin::INT,
                            span,
                        )),
                    },
                    builtin::INT,
                    span,
                ),
            },
            span,
        ));

        // Condition: __idx < __keys.len()
        let condition = HirExpr::with_type(
            HirExprKind::BinOp {
                op: HirBinOp::Lt,
                lhs: Box::new(HirExpr::with_type(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    builtin::INT,
                    span,
                )),
                rhs: Box::new(HirExpr::with_type(
                    HirExprKind::MethodCall {
                        receiver: Box::new(HirExpr::new(
                            HirExprKind::Local {
                                name: internal_keys.clone(),
                            },
                            span,
                        )),
                        method: "len".to_string(),
                        args: vec![],
                    },
                    builtin::INT,
                    span,
                )),
            },
            builtin::BOOL,
            span,
        );

        // Build desugared block using the already-lowered map expression
        let map_type = lowered_map.type_id;
        let map_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_map.clone(),
                type_id: map_type,
                value: lowered_map.clone(),
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );

        let keys_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_keys.clone(),
                type_id: None,
                value: {
                    // CRITICAL: The receiver must have the map type set for proper type propagation
                    let receiver = match map_type {
                        Some(t) => HirExpr::with_type(
                            HirExprKind::Local {
                                name: internal_map.clone(),
                            },
                            t,
                            span,
                        ),
                        None => HirExpr::new(
                            HirExprKind::Local {
                                name: internal_map.clone(),
                            },
                            span,
                        ),
                    };
                    let keys_call = HirExpr::new(
                        HirExprKind::MethodCall {
                            receiver: Box::new(receiver),
                            method: "keys".to_string(),
                            args: vec![],
                        },
                        span,
                    );

                    // Infer the return type of keys() method
                    if let Some(map_ty) = map_type {
                        if let Some(keys_ty) =
                            self.infer_method_call_type(map_ty, "keys", &mut [], registry)
                        {
                            HirExpr::with_type(keys_call.kind, keys_ty, span)
                        } else {
                            keys_call
                        }
                    } else {
                        keys_call
                    }
                },
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );

        let idx_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_idx,
                type_id: Some(builtin::INT),
                value: HirExpr::with_type(
                    HirExprKind::Const(ConstValue::Int(0)),
                    builtin::INT,
                    span,
                ),
                mutable: true,
                ownership: Ownership::Owned,
            },
            span,
        );

        let while_stmt = HirStmt::new(
            HirStmtKind::While {
                condition,
                body: body_stmts,
            },
            span,
        );

        HirStmtKind::Expr(HirExpr::new(
            HirExprKind::Block {
                stmts: vec![map_init, keys_init, idx_init, while_stmt],
                expr: None,
            },
            span,
        ))
    }

    /// Infer the return type of module-level method calls (e.g., JSON.stringify, JSON.parse)
    /// For JSON.parse, if the argument is JSON.stringify(x), we can infer the type from x.
    /// Also tracks variables that were assigned from JSON.stringify for indirect inference.
    fn infer_module_method_type(
        &self,
        receiver: &HirExpr,
        method: &str,
        args: &[HirExpr],
        registry: &mut TypeRegistry,
    ) -> Option<TypeId> {
        // Check if receiver is a module identifier
        let module_name = match &receiver.kind {
            HirExprKind::Local { name } => name.as_str(),
            _ => return None,
        };

        match module_name {
            "JSON" => match method {
                "stringify" => Some(builtin::STR),
                "parse" => {
                    // For JSON.parse, try to infer type from the argument
                    if let Some(first_arg) = args.first() {
                        // Case 1: JSON.parse(JSON.stringify(x)) - the arg is a stringify call
                        if let Some(inner_type) = self.extract_stringify_arg_type(first_arg) {
                            return Some(inner_type);
                        }

                        // Case 2: JSON.parse(variable) where variable was assigned from JSON.stringify(x)
                        if let HirExprKind::Local { name } = &first_arg.kind {
                            if let Some(stringify_arg_type) = self.json_stringify_sources.get(name)
                            {
                                return Some(*stringify_arg_type);
                            }
                        }
                    }
                    // Fallback: JSON.parse returns ANY since we can't know the target type here
                    // The actual type will be determined by the context (assignment, return, etc.)
                    Some(builtin::ANY)
                }
                _ => None,
            },
            "Math" => match method {
                "abs" | "floor" | "ceil" | "round" | "sqrt" | "pow" | "sin" | "cos" | "tan"
                | "log" | "exp" | "min" | "max" => Some(builtin::FLOAT),
                "random" => Some(builtin::FLOAT),
                _ => None,
            },
            "Array" => match method {
                "new" => {
                    // Array.new() returns Array[Any]
                    Some(registry.register_array(builtin::ANY))
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Extract the type of the argument to JSON.stringify if the expression is a stringify call.
    /// Returns None if the expression is not a JSON.stringify call or the type cannot be determined.
    fn extract_stringify_arg_type(&self, expr: &HirExpr) -> Option<TypeId> {
        if let HirExprKind::MethodCall {
            receiver,
            method,
            args,
        } = &expr.kind
        {
            // Check if this is JSON.stringify
            if method == "stringify" {
                if let HirExprKind::Local { name } = &receiver.kind {
                    if name == "JSON" {
                        // Get the type of the first argument to stringify
                        if let Some(first_arg) = args.first() {
                            return first_arg.type_id;
                        }
                    }
                }
            }
        }
        None
    }

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
            TypeKind::Map { key, value } => {
                self.infer_map_method_type(*key, *value, method, args, registry)
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
                let closure_return = args.get_mut(0).and_then(|arg| {
                    self.apply_closure_signature(arg, &[elem_type], None, registry)
                });
                let out_elem = closure_return.unwrap_or(builtin::ANY);
                Some(registry.register_array(out_elem))
            }
            "filter" => {
                let _ = args.get_mut(0).and_then(|arg| {
                    self.apply_closure_signature(arg, &[elem_type], Some(builtin::BOOL), registry)
                });
                Some(registry.register_array(elem_type))
            }
            "reduce" => {
                let init_type = args
                    .get(0)
                    .and_then(|arg| arg.type_id)
                    .unwrap_or(builtin::ANY);
                let closure_return = args.get_mut(1).and_then(|arg| {
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

    fn infer_map_method_type(
        &mut self,
        key_type: TypeId,
        value_type: TypeId,
        method: &str,
        _args: &mut [HirExpr],
        registry: &mut TypeRegistry,
    ) -> Option<TypeId> {
        match method {
            "keys" => {
                // keys() returns Array<K>
                Some(registry.register_array(key_type))
            }
            "values" => {
                // values() returns Array<V>
                Some(registry.register_array(value_type))
            }
            "has" => {
                // has(key) returns bool
                Some(builtin::BOOL)
            }
            "len" => {
                // len() returns int
                Some(builtin::INT)
            }
            "isEmpty" => {
                // isEmpty() returns bool
                Some(builtin::BOOL)
            }
            "clear" | "remove" => {
                // clear() and remove(key) return void
                Some(builtin::VOID)
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
                // Apply param types from context
                for (idx, (_, param_type)) in params.iter_mut().enumerate() {
                    if param_type.is_none() {
                        if let Some(ty) = param_types.get(idx) {
                            *param_type = Some(*ty);
                        }
                    }
                }

                // Build local variable types map for body inference
                let mut locals: FxHashMap<String, TypeId> = FxHashMap::default();
                for (name, type_id) in params.iter() {
                    if let Some(tid) = type_id {
                        locals.insert(name.clone(), *tid);
                    }
                }

                // Infer body type if not set or if it was initially inferred as ANY
                // (which happens when closure param types weren't known during initial lowering,
                // e.g., `(x) => x * x` where both operands are untyped params → Mul(ANY, ANY) = ANY)
                if body.type_id.is_none() || body.type_id == Some(builtin::ANY) {
                    if let Some(ret) = return_type_hint {
                        body.type_id = Some(ret);
                    } else {
                        // Use centralized inference from doo_core
                        let inferred = self.infer_closure_body_type(body, &locals, registry);
                        body.type_id = Some(inferred);
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

    /// Infer the type of a closure body expression.
    /// Uses centralized inference rules from doo_core::infer (single source of truth).
    fn infer_closure_body_type(
        &self,
        expr: &HirExpr,
        locals: &FxHashMap<String, TypeId>,
        registry: &mut TypeRegistry,
    ) -> TypeId {
        // If already has type, return it
        if let Some(tid) = expr.type_id {
            return tid;
        }

        match &expr.kind {
            HirExprKind::Const(c) => c.type_id(),

            HirExprKind::Local { name } => locals.get(name).copied().unwrap_or(builtin::ANY),

            HirExprKind::BinOp { op, lhs, rhs } => {
                let lhs_type = self.infer_closure_body_type(lhs, locals, registry);
                let rhs_type = self.infer_closure_body_type(rhs, locals, registry);
                // Convert HirBinOp to BinOpKind and use centralized inference
                let op_kind = hir_binop_to_kind(*op);
                infer_binop_result_type(op_kind, lhs_type, rhs_type)
            }

            HirExprKind::UnaryOp { op, operand } => {
                let operand_type = self.infer_closure_body_type(operand, locals, registry);
                let op_kind = hir_unaryop_to_kind(*op);
                infer_unaryop_result_type(op_kind, operand_type)
            }

            HirExprKind::Index { object, .. } => {
                let obj_type = self.infer_closure_body_type(object, locals, registry);
                if let Some(info) = registry.get(obj_type) {
                    match &info.kind {
                        TypeKind::Array { element } => *element,
                        TypeKind::Map { value, .. } => *value,
                        TypeKind::Str => builtin::STR,
                        _ => builtin::ANY,
                    }
                } else {
                    builtin::ANY
                }
            }

            HirExprKind::Block { stmts, expr } => {
                // Build extended locals map including block-local let bindings
                let mut block_locals = locals.clone();
                for stmt in stmts.iter() {
                    if let HirStmtKind::Let { name, value, .. } = &stmt.kind {
                        // Infer the type of the let value and add to block_locals
                        let val_type = self.infer_closure_body_type(value, &block_locals, registry);
                        block_locals.insert(name.clone(), val_type);
                    }
                }

                // Priority 1: trailing expression
                if let Some(expr) = expr {
                    return self.infer_closure_body_type(expr, &block_locals, registry);
                }
                // Priority 2: look for return statements (for block closures with explicit return)
                for stmt in stmts.iter() {
                    if let HirStmtKind::Return(values) = &stmt.kind {
                        if let Some(val) = values.first() {
                            return self.infer_closure_body_type(val, &block_locals, registry);
                        }
                    }
                }
                // Priority 3: last expression statement
                if let Some(last) = stmts.last() {
                    if let HirStmtKind::Expr(e) = &last.kind {
                        return self.infer_closure_body_type(e, &block_locals, registry);
                    }
                }
                builtin::VOID
            }

            HirExprKind::If {
                then_expr,
                else_expr,
                ..
            } => {
                let then_type = self.infer_closure_body_type(then_expr, locals, registry);
                if let Some(else_expr) = else_expr {
                    let else_type = self.infer_closure_body_type(else_expr, locals, registry);
                    if then_type == else_type {
                        then_type
                    } else if then_type == builtin::ANY {
                        else_type
                    } else if else_type == builtin::ANY {
                        then_type
                    } else {
                        builtin::ANY
                    }
                } else {
                    builtin::VOID
                }
            }

            _ => builtin::ANY,
        }
    }

    fn pattern_to_name(&self, pattern: &Pattern) -> String {
        match &pattern.kind {
            PatternKind::Ident(name) => name.clone(),
            PatternKind::Wildcard => "_".to_string(),
            PatternKind::Tuple(_) => "__tuple".to_string(),
            PatternKind::Index { object, .. } => {
                // For index patterns, use the base object name
                self.pattern_to_name(object)
            }
            PatternKind::Field { object, .. } => {
                // For field patterns, use the base object name
                self.pattern_to_name(object)
            }
        }
    }

    fn pattern_to_expr(&mut self, pattern: &Pattern) -> HirExpr {
        match &pattern.kind {
            PatternKind::Ident(name) => {
                HirExpr::new(HirExprKind::Local { name: name.clone() }, pattern.span)
            }
            PatternKind::Wildcard => HirExpr::new(
                HirExprKind::Local {
                    name: "_".to_string(),
                },
                pattern.span,
            ),
            PatternKind::Tuple(patterns) => {
                let exprs = patterns.iter().map(|p| self.pattern_to_expr(p)).collect();
                HirExpr::new(HirExprKind::Tuple(exprs), pattern.span)
            }
            PatternKind::Index { object, index } => {
                let object_expr = self.pattern_to_expr(object);
                let index_expr = self.lower_expr(index);
                HirExpr::new(
                    HirExprKind::Index {
                        object: Box::new(object_expr),
                        index: Box::new(index_expr),
                    },
                    pattern.span,
                )
            }
            PatternKind::Field { object, field } => {
                let object_expr = self.pattern_to_expr(object);
                HirExpr::new(
                    HirExprKind::Field {
                        object: Box::new(object_expr),
                        field: field.clone(),
                    },
                    pattern.span,
                )
            }
        }
    }

    fn pattern_to_names(&self, pattern: &Pattern) -> Vec<String> {
        match &pattern.kind {
            PatternKind::Tuple(patterns) => {
                patterns.iter().map(|p| self.pattern_to_name(p)).collect()
            }
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
            ast::MatchPattern::Condition(e) => {
                HirMatchPattern::Condition(Box::new(self.lower_expr(e)))
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

    /// Convert a struct literal match pattern to a field-by-field comparison condition.
    ///
    /// Transforms `Point { x: 10, y: 20 }` into `matched.x == 10 && matched.y == 20`.
    fn struct_pattern_to_condition(
        &self,
        matched_val: &HirExpr,
        fields: &[(String, HirExpr)],
        span: Span,
    ) -> HirMatchPattern {
        if fields.is_empty() {
            // No fields to compare → always matches
            return HirMatchPattern::Wildcard;
        }

        // Build comparisons: matched.field == value
        let mut comparisons: Vec<HirExpr> = Vec::new();
        for (field_name, field_val) in fields {
            let field_access = HirExpr::new(
                HirExprKind::Field {
                    object: Box::new(matched_val.clone()),
                    field: field_name.clone(),
                },
                span,
            );
            let eq_check = HirExpr::new(
                HirExprKind::BinOp {
                    op: HirBinOp::Eq,
                    lhs: Box::new(field_access),
                    rhs: Box::new(field_val.clone()),
                },
                span,
            );
            comparisons.push(eq_check);
        }

        // Chain comparisons with &&
        let mut result = comparisons.remove(0);
        for comp in comparisons {
            result = HirExpr::new(
                HirExprKind::BinOp {
                    op: HirBinOp::And,
                    lhs: Box::new(result),
                    rhs: Box::new(comp),
                },
                span,
            );
        }

        HirMatchPattern::Condition(Box::new(result))
    }

    fn resolve_type_expr(&mut self, ty: &TypeExpr, registry: &mut TypeRegistry) -> TypeId {
        match &ty.kind {
            doo_frontend::ast::TypeExprKind::Named(name) => registry
                .lookup(name)
                .unwrap_or_else(|| registry.declare_named(name)),
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
                let elements = parts
                    .iter()
                    .map(|p| self.resolve_type_expr(p, registry))
                    .collect();
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
                let params_ids = params
                    .iter()
                    .map(|p| self.resolve_type_expr(p, registry))
                    .collect();
                let returns_id = self.resolve_type_expr(returns, registry);
                registry.register_function(params_ids, returns_id)
            }
            doo_frontend::ast::TypeExprKind::Range(_inner) => registry.declare_named("Range"),
            doo_frontend::ast::TypeExprKind::Any => builtin::ANY,
            doo_frontend::ast::TypeExprKind::Void => builtin::VOID,
            doo_frontend::ast::TypeExprKind::Error => builtin::ERROR,
        }
    }

    /// Determine the common element type for an array literal.
    /// Handles Spread elements by extracting the element type from the spread source.
    fn common_array_elem_type(&self, elements: &[HirExpr], registry: &TypeRegistry) -> TypeId {
        let mut current: Option<TypeId> = None;
        for e in elements {
            // For Spread elements, extract the element type from the inner array
            let elem_type = if let HirExprKind::Spread(inner) = &e.kind {
                // Get the inner expression's type (should be an array)
                inner.type_id.and_then(|arr_type| {
                    registry.get(arr_type).and_then(|info| {
                        if let TypeKind::Array { element } = &info.kind {
                            Some(*element)
                        } else {
                            None
                        }
                    })
                })
            } else {
                // For non-spread elements, use the element's type directly
                e.type_id
            };

            let Some(t) = elem_type else {
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
                    // Cast to String — use the expression's span for error reporting
                    let span = expr_hir.span;
                    HirExpr::with_type(
                        HirExprKind::Cast {
                            value: Box::new(expr_hir),
                            to_type: builtin::STR,
                        },
                        builtin::STR,
                        span,
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
                assert!(matches!(
                    value.kind,
                    HirExprKind::BinOp {
                        op: HirBinOp::Add,
                        ..
                    }
                ));
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
                assert!(matches!(
                    value.kind,
                    HirExprKind::BinOp {
                        op: HirBinOp::Add,
                        ..
                    }
                ));
            } else {
                panic!("Expected assign from increment");
            }
        }
    }

    #[test]
    fn test_desugar_infinite_for_loop() {
        // Note: New parser requires pattern; infinite loop uses wildcard or ident + no iterable
        let hir = parse_and_lower("fn test() { for _ { break } }");
        if let HirItem::Function(f) = &hir.items[0] {
            assert_eq!(f.body.len(), 1);
            if let HirStmtKind::While { condition, body } = &f.body[0].kind {
                // Infinite loop has condition = true
                assert!(matches!(
                    condition.kind,
                    HirExprKind::Const(ConstValue::Bool(true))
                ));
                // Body contains break
                assert_eq!(body.len(), 1);
                assert!(matches!(body[0].kind, HirStmtKind::Break));
            } else {
                panic!("Expected while loop for desugared for loop");
            }
        }
    }

    #[test]
    fn test_desugar_range_for_loop() {
        let hir = parse_and_lower("fn test() { for i in 0..10 { print(i) } }");
        if let HirItem::Function(f) = &hir.items[0] {
            assert_eq!(f.body.len(), 1);
            // Desugared to block expression containing: let __idx = 0; while __idx < 10 { ... }
            if let HirStmtKind::Expr(expr) = &f.body[0].kind {
                if let HirExprKind::Block { stmts, .. } = &expr.kind {
                    // First stmt: let __i_idx = 0
                    assert_eq!(stmts.len(), 2);
                    if let HirStmtKind::Let { name, mutable, .. } = &stmts[0].kind {
                        assert!(name.contains("_idx"));
                        assert!(*mutable); // Index is mutable
                    } else {
                        panic!("Expected let statement for index initialization");
                    }
                    // Second stmt: while loop
                    assert!(matches!(stmts[1].kind, HirStmtKind::While { .. }));
                } else {
                    panic!("Expected block expression");
                }
            } else {
                panic!("Expected expression statement");
            }
        }
    }

    #[test]
    fn test_desugar_inclusive_range_for_loop() {
        let hir = parse_and_lower("fn test() { for i in 0..=5 { print(i) } }");
        if let HirItem::Function(f) = &hir.items[0] {
            if let HirStmtKind::Expr(expr) = &f.body[0].kind {
                if let HirExprKind::Block { stmts, .. } = &expr.kind {
                    // Check while condition uses LtEq for inclusive range
                    if let HirStmtKind::While { condition, .. } = &stmts[1].kind {
                        if let HirExprKind::BinOp { op, .. } = &condition.kind {
                            assert_eq!(*op, HirBinOp::LtEq);
                        } else {
                            panic!("Expected BinOp condition");
                        }
                    } else {
                        panic!("Expected while loop");
                    }
                }
            }
        }
    }

    #[test]
    fn test_desugar_array_for_loop() {
        let hir = parse_and_lower("fn test() { let arr = [1, 2, 3]\n for x in arr { print(x) } }");
        if let HirItem::Function(f) = &hir.items[0] {
            assert_eq!(f.body.len(), 2); // let arr; for x in arr
                                         // Second statement is desugared for loop
            if let HirStmtKind::Expr(expr) = &f.body[1].kind {
                if let HirExprKind::Block { stmts, .. } = &expr.kind {
                    // Should have: let __arr = arr; let __idx = 0; while __idx < __arr.len() { ... }
                    assert_eq!(stmts.len(), 3);
                    // First: let __arr = arr
                    if let HirStmtKind::Let { name, mutable, .. } = &stmts[0].kind {
                        assert!(name.contains("_arr"));
                        assert!(!*mutable);
                    }
                    // Second: let __idx = 0
                    if let HirStmtKind::Let { name, mutable, .. } = &stmts[1].kind {
                        assert!(name.contains("_idx"));
                        assert!(*mutable);
                    }
                    // Third: while loop
                    if let HirStmtKind::While { condition, body } = &stmts[2].kind {
                        // Condition should be __idx < __arr.len()
                        if let HirExprKind::BinOp { op, .. } = &condition.kind {
                            assert_eq!(*op, HirBinOp::Lt);
                        }
                        // Body should have: let x = __arr[__idx]; print(x); __idx++
                        assert!(body.len() >= 2);
                    }
                }
            }
        }
    }

    #[test]
    fn test_desugar_indexed_array_for_loop() {
        let hir = parse_and_lower(
            "fn test() { let arr = [10, 20, 30]\n for i, x in arr { print(i, x) } }",
        );
        if let HirItem::Function(f) = &hir.items[0] {
            if let HirStmtKind::Expr(expr) = &f.body[1].kind {
                if let HirExprKind::Block { stmts, .. } = &expr.kind {
                    if let HirStmtKind::While { body, .. } = &stmts[2].kind {
                        // Body should have:
                        // 1. let i = __idx
                        // 2. let x = __arr[__idx]
                        // 3. print(i, x)
                        // 4. __idx++
                        assert!(body.len() >= 3);

                        // First should be index assignment
                        if let HirStmtKind::Let { name, .. } = &body[0].kind {
                            assert_eq!(name, "i");
                        } else {
                            panic!("Expected let statement for index");
                        }

                        // Second should be element extraction
                        if let HirStmtKind::Let { name, value, .. } = &body[1].kind {
                            assert_eq!(name, "x");
                            assert!(matches!(value.kind, HirExprKind::Index { .. }));
                        } else {
                            panic!("Expected let statement for element");
                        }
                    }
                }
            }
        }
    }
}

/// Convert HirBinOp to BinOpKind for centralized type inference.
fn hir_binop_to_kind(op: HirBinOp) -> BinOpKind {
    match op {
        HirBinOp::Add => BinOpKind::Add,
        HirBinOp::Sub => BinOpKind::Sub,
        HirBinOp::Mul => BinOpKind::Mul,
        HirBinOp::Div => BinOpKind::Div,
        HirBinOp::Mod => BinOpKind::Mod,
        HirBinOp::Eq => BinOpKind::Eq,
        HirBinOp::NotEq => BinOpKind::Ne,
        HirBinOp::Lt => BinOpKind::Lt,
        HirBinOp::Gt => BinOpKind::Gt,
        HirBinOp::LtEq => BinOpKind::Le,
        HirBinOp::GtEq => BinOpKind::Ge,
        HirBinOp::And => BinOpKind::And,
        HirBinOp::Or => BinOpKind::Or,
        // In and BitAnd/BitOr don't have direct equivalents, default to appropriate
        HirBinOp::In => BinOpKind::Eq, // Comparison semantics
        HirBinOp::BitAnd | HirBinOp::BitOr => BinOpKind::And, // Logical semantics for type inference
    }
}

/// Convert HirUnaryOp to UnaryOpKind for centralized type inference.
fn hir_unaryop_to_kind(op: HirUnaryOp) -> UnaryOpKind {
    match op {
        HirUnaryOp::Neg => UnaryOpKind::Neg,
        HirUnaryOp::Not => UnaryOpKind::Not,
    }
}
