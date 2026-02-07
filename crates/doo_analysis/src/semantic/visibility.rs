//! Visibility Checker
//!
//! Validates that access to non-public items respects visibility rules.
//!
//! ## Rules:
//!
//! - `pub` items are accessible from any module
//! - Non-`pub` items are only accessible within the same module
//! - Struct fields follow the struct's visibility by default
//! - Enum variants are always accessible if the enum is accessible

use doo_core::doo_debug;
use doo_core::Span;
use std::collections::HashMap;

/// Visibility of an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// Public - accessible from any module.
    Public,
    /// Private - only accessible within the same module.
    Private,
}

impl Default for Visibility {
    fn default() -> Self {
        Self::Private
    }
}

/// A symbol with its visibility and location.
#[derive(Debug, Clone)]
pub struct VisibleSymbol {
    /// Name of the symbol.
    pub name: String,
    /// Visibility level.
    pub visibility: Visibility,
    /// Module where the symbol is defined.
    pub module_path: String,
    /// Span of the definition.
    pub span: Span,
}

/// Visibility error.
#[derive(Debug, Clone)]
pub struct VisibilityError {
    /// The symbol being accessed.
    pub symbol: String,
    /// Module where the symbol is defined.
    pub defined_in: String,
    /// Module where the access occurs.
    pub accessed_from: String,
    /// Span of the access.
    pub span: Span,
}

impl std::fmt::Display for VisibilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cannot access private symbol `{}` from module `{}` (defined in `{}`)",
            self.symbol, self.accessed_from, self.defined_in
        )
    }
}

impl std::error::Error for VisibilityError {}

/// Visibility checker.
pub struct VisibilityChecker {
    /// Map of symbol name to visibility info.
    symbols: HashMap<String, VisibleSymbol>,
    /// Current module path being analyzed.
    current_module: String,
}

impl VisibilityChecker {
    /// Create a new visibility checker.
    pub fn new() -> Self {
        Self {
            symbols: HashMap::new(),
            current_module: String::new(),
        }
    }

    /// Set the current module being analyzed.
    pub fn set_current_module(&mut self, module_path: &str) {
        self.current_module = module_path.to_string();
    }

    /// Register a symbol with its visibility.
    pub fn register_symbol(
        &mut self,
        name: &str,
        visibility: Visibility,
        module_path: &str,
        span: Span,
    ) {
        self.symbols.insert(
            name.to_string(),
            VisibleSymbol {
                name: name.to_string(),
                visibility,
                module_path: module_path.to_string(),
                span,
            },
        );
    }

    /// Register a public symbol.
    pub fn register_public(&mut self, name: &str, module_path: &str, span: Span) {
        self.register_symbol(name, Visibility::Public, module_path, span);
    }

    /// Register a private symbol.
    pub fn register_private(&mut self, name: &str, module_path: &str, span: Span) {
        self.register_symbol(name, Visibility::Private, module_path, span);
    }

    /// Check if an access is allowed.
    pub fn check_access(
        &self,
        symbol_name: &str,
        access_span: Span,
    ) -> Result<(), VisibilityError> {
        if let Some(symbol) = self.symbols.get(symbol_name) {
            // Public symbols are always accessible
            if symbol.visibility == Visibility::Public {
                return Ok(());
            }

            // Private symbols are only accessible in the same module
            if symbol.module_path == self.current_module {
                return Ok(());
            }

            // Cross-module access to private symbol is an error
            return Err(VisibilityError {
                symbol: symbol_name.to_string(),
                defined_in: symbol.module_path.clone(),
                accessed_from: self.current_module.clone(),
                span: access_span,
            });
        }

        // Symbol not found - let name resolution handle this error
        Ok(())
    }

    /// Check if a symbol is accessible from a given module.
    pub fn is_accessible(&self, symbol_name: &str, from_module: &str) -> bool {
        if let Some(symbol) = self.symbols.get(symbol_name) {
            symbol.visibility == Visibility::Public || symbol.module_path == from_module
        } else {
            // Symbol not registered - assume accessible (let resolution handle it)
            true
        }
    }

    /// Get all public symbols.
    pub fn public_symbols(&self) -> impl Iterator<Item = &VisibleSymbol> {
        self.symbols
            .values()
            .filter(|s| s.visibility == Visibility::Public)
    }

    /// Get all symbols in a module.
    pub fn module_symbols<'a>(
        &'a self,
        module_path: &'a str,
    ) -> impl Iterator<Item = &'a VisibleSymbol> + 'a {
        self.symbols
            .values()
            .filter(move |s| s.module_path == module_path)
    }

    /// Get exported symbols (public symbols from a module).
    pub fn exports(&self, module_path: &str) -> Vec<String> {
        self.symbols
            .values()
            .filter(|s| s.module_path == module_path && s.visibility == Visibility::Public)
            .map(|s| s.name.clone())
            .collect()
    }

    /// Clear all symbols (for fresh analysis).
    pub fn clear(&mut self) {
        self.symbols.clear();
    }
}

impl Default for VisibilityChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to determine visibility from `is_public` flag.
pub fn visibility_from_flag(is_public: bool) -> Visibility {
    if is_public {
        Visibility::Public
    } else {
        Visibility::Private
    }
}

// ============================================================================
// Field Visibility Checker (HIR-level)
// ============================================================================

use doo_core::constants::ffi_names::is_self_returning_method;
use doo_core::types::{TypeKind, TypeRegistry};
use doo_hir::{HirExprKind, HirItem, HirProgram, HirStmtKind};
use std::collections::HashSet;
use std::sync::Arc;

/// Error for private field access
#[derive(Debug, Clone)]
pub struct FieldVisibilityError {
    /// Name of the field being accessed
    pub field_name: String,
    /// Name of the struct containing the field
    pub struct_name: String,
    /// Span where the access occurs
    pub span: Span,
}

impl std::fmt::Display for FieldVisibilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "E0100 PRIVATE FIELD ACCESS: Cannot access private field '{}' on struct '{}'. \
            Private fields (camelCase) are not accessible outside their module.",
            self.field_name, self.struct_name
        )
    }
}

impl std::error::Error for FieldVisibilityError {}

/// Field visibility checker that validates access to struct fields.
///
/// This uses the TypeRegistry as the single source of truth for field visibility.
/// Fields have `is_public` set during HIR lowering based on naming convention:
/// - PascalCase = public
/// - camelCase = private
pub struct FieldVisibilityChecker<'a> {
    type_registry: &'a TypeRegistry,
    imported_structs: &'a HashSet<String>,
    errors: Vec<FieldVisibilityError>,
    /// Map of local variable name -> struct name (if it's a struct type)
    local_struct_types: HashMap<String, String>,
}

impl<'a> FieldVisibilityChecker<'a> {
    /// Create a new field visibility checker
    pub fn new(type_registry: &'a TypeRegistry, imported_structs: &'a HashSet<String>) -> Self {
        Self {
            type_registry,
            imported_structs,
            errors: Vec::new(),
            local_struct_types: HashMap::new(),
        }
    }

    /// Check an entire HIR program for private field access violations
    pub fn check_program(&mut self, hir: &HirProgram) {
        for item in &hir.items {
            if let HirItem::Function(func) = item {
                // Build local struct type map from function body
                self.local_struct_types.clear();
                self.collect_local_struct_types(&func.body);

                // Now check for visibility violations
                for stmt in &func.body {
                    self.check_stmt(stmt);
                }
            }
        }
    }

    /// Collect local variable -> struct name mappings from statements
    fn collect_local_struct_types(&mut self, stmts: &[doo_hir::HirStmt]) {
        use doo_hir::HirStmtKind;

        for stmt in stmts {
            match &stmt.kind {
                HirStmtKind::Let {
                    name,
                    type_id,
                    value,
                    ..
                } => {
                    if std::env::var("DOO_DEBUG").is_ok() {
                        doo_debug!("VISIBILITY", "Processing Let: {} type_id={:?}",
                            name, type_id
                        );
                    }
                    // Try to determine struct type from type_id first
                    if let Some(tid) = type_id {
                        if let Some(info) = self.type_registry.get(*tid) {
                            if let TypeKind::Struct {
                                name: struct_name, ..
                            } = &info.kind
                            {
                                if std::env::var("DOO_DEBUG").is_ok() {
                                    doo_debug!("VISIBILITY", "Let {} has struct type {} from type_id",
                                        name, struct_name
                                    );
                                }
                                self.local_struct_types
                                    .insert(name.clone(), struct_name.clone());
                            }
                        }
                    } else {
                        // Try to infer from value expression
                        if let Some(struct_name) = self.get_expr_struct_type(value) {
                            if std::env::var("DOO_DEBUG").is_ok() {
                                doo_debug!("VISIBILITY", "Let {} has struct type {} from expression",
                                    name, struct_name
                                );
                            }
                            self.local_struct_types.insert(name.clone(), struct_name);
                        } else if std::env::var("DOO_DEBUG").is_ok() {
                            doo_debug!("VISIBILITY", "Let {} could not determine struct type, value kind={:?}", name, std::mem::discriminant(&value.kind));
                        }
                    }
                }
                HirStmtKind::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    self.collect_local_struct_types(then_block);
                    if let Some(else_stmts) = else_block {
                        self.collect_local_struct_types(else_stmts);
                    }
                }
                HirStmtKind::While { body, .. } => {
                    self.collect_local_struct_types(body);
                }
                _ => {}
            }
        }
    }

    /// Try to determine the struct type name from an expression
    fn get_expr_struct_type(&self, expr: &doo_hir::HirExpr) -> Option<String> {
        use doo_hir::HirExprKind;

        match &expr.kind {
            // Call to a function that returns a struct
            HirExprKind::Call { func, .. } => {
                // Check if function is a Global or Local reference
                let func_name = match &func.kind {
                    HirExprKind::Global { name } => Some(name.as_str()),
                    HirExprKind::Local { name, .. } => Some(name.as_str()),
                    _ => {
                        if std::env::var("DOO_DEBUG").is_ok() {
                            doo_debug!("VISIBILITY", "Call func is not Global/Local, kind={:?}",
                                std::mem::discriminant(&func.kind)
                            );
                        }
                        None
                    }
                };

                if let Some(name) = func_name {
                    if std::env::var("DOO_DEBUG").is_ok() {
                        doo_debug!("VISIBILITY", "Call to func '{}', imported_structs={:?}",
                            name, self.imported_structs
                        );
                    }

                    // Check various naming conventions:
                    // 1. CreateFoo() returns Foo
                    // 2. CreateFooBar() returns FooBar (check imported structs)
                    // 3. Foo() returns Foo (constructor named like struct)

                    // First: direct match - function name is struct name
                    if self.imported_structs.contains(name) {
                        return Some(name.to_string());
                    }

                    // Second: CreateX pattern - check if any imported struct is a suffix
                    if name.starts_with("Create") && name.len() > 6 {
                        let after_create = &name[6..];
                        // Check if after_create matches any imported struct exactly
                        if self.imported_structs.contains(after_create) {
                            return Some(after_create.to_string());
                        }
                        // Check if any imported struct name ends with after_create
                        // e.g., CreateUser -> User, but struct is PublicUser
                        for struct_name in self.imported_structs.iter() {
                            if struct_name.ends_with(after_create) {
                                if std::env::var("DOO_DEBUG").is_ok() {
                                    doo_debug!("VISIBILITY", "Matched Create{} to struct {} (ends_with)",
                                        after_create, struct_name
                                    );
                                }
                                return Some(struct_name.clone());
                            }
                        }
                    }
                }
                None
            }
            // Struct literal
            HirExprKind::Struct { name, .. } => Some(name.clone()),
            // Variable reference - look up from our map
            HirExprKind::Local { name, .. } => self.local_struct_types.get(name).cloned(),
            // Clone/Move pass through the inner type
            HirExprKind::Clone(inner) | HirExprKind::Move(inner) => {
                if std::env::var("DOO_DEBUG").is_ok() {
                    doo_debug!("VISIBILITY", "Looking through Clone/Move wrapper");
                }
                self.get_expr_struct_type(inner)
            }
            // Try expression - unwrap the inner Result type
            HirExprKind::Try(inner) => {
                if std::env::var("DOO_DEBUG").is_ok() {
                    doo_debug!("VISIBILITY", "Looking through Try wrapper");
                }
                // First, try to get struct type from the Try expression's type_id (the unwrapped ok type)
                if let Some(type_id) = expr.type_id {
                    if let Some(info) = self.type_registry.get(type_id) {
                        if let TypeKind::Struct { name, .. } = &info.kind {
                            return Some(name.clone());
                        }
                    }
                }
                // Fallback: if inner has type_id that's a Result, extract the ok type
                if let Some(inner_type_id) = inner.type_id {
                    if let Some(info) = self.type_registry.get(inner_type_id) {
                        if let TypeKind::Result { ok, .. } = &info.kind {
                            if let Some(ok_info) = self.type_registry.get(*ok) {
                                if let TypeKind::Struct { name, .. } = &ok_info.kind {
                                    return Some(name.clone());
                                }
                            }
                        }
                    }
                }
                // Last resort: recurse into inner (for cases like nested Try or method chains)
                self.get_expr_struct_type(inner)
            }
            // Method call - infer struct type from receiver and method pattern
            HirExprKind::MethodCall {
                receiver, method, ..
            } => {
                if std::env::var("DOO_DEBUG").is_ok() {
                    doo_debug!("VISIBILITY", "Inferring struct type from MethodCall .{}, receiver_kind={:?}", 
                        method, std::mem::discriminant(&receiver.kind));
                }
                // First check the expression's type_id directly
                if let Some(type_id) = expr.type_id {
                    if let Some(info) = self.type_registry.get(type_id) {
                        if let TypeKind::Struct { name, .. } = &info.kind {
                            return Some(name.clone());
                        }
                        // Handle Result type (for failable methods)
                        if let TypeKind::Result { ok, .. } = &info.kind {
                            if let Some(ok_info) = self.type_registry.get(*ok) {
                                if let TypeKind::Struct { name, .. } = &ok_info.kind {
                                    return Some(name.clone());
                                }
                            }
                        }
                    }
                }
                // For self-returning patterns: if receiver is Global with struct name
                // and method is a known self-returning pattern, return receiver name
                // Uses centralized list from doo_core::constants::ffi_names
                if let HirExprKind::Global { name: recv_name } = &receiver.kind {
                    if std::env::var("DOO_DEBUG").is_ok() {
                        doo_debug!("VISIBILITY", "MethodCall receiver is Global({}), imported_structs contains={}", 
                            recv_name, self.imported_structs.contains(recv_name));
                    }
                    // Check if receiver name is an imported struct
                    if self.imported_structs.contains(recv_name) {
                        // Check against centralized self-returning method patterns
                        if is_self_returning_method(method) {
                            if std::env::var("DOO_DEBUG").is_ok() {
                                doo_debug!("VISIBILITY", "MethodCall {}.{}() returns {}",
                                    recv_name, method, recv_name
                                );
                            }
                            return Some(recv_name.clone());
                        }
                    }
                }
                None
            }
            _ => {
                if std::env::var("DOO_DEBUG").is_ok() {
                    doo_debug!("VISIBILITY", "Unknown expr kind for struct type inference: {:?}",
                        std::mem::discriminant(&expr.kind)
                    );
                }
                None
            }
        }
    }

    /// Get the collected errors
    pub fn into_errors(self) -> Vec<FieldVisibilityError> {
        self.errors
    }

    /// Get errors as formatted strings
    pub fn errors_as_strings(&self) -> Vec<String> {
        self.errors.iter().map(|e| e.to_string()).collect()
    }

    // Check if a field name is private (starts with lowercase)
    fn is_private_field(name: &str) -> bool {
        name.chars()
            .next()
            .map(|c| c.is_lowercase())
            .unwrap_or(false)
    }

    fn check_expr(&mut self, expr: &doo_hir::HirExpr) {
        match &expr.kind {
            HirExprKind::Field { object, field } => {
                // First check the object recursively
                self.check_expr(object);

                // Debug output
                if std::env::var("DOO_DEBUG").is_ok() {
                    doo_debug!("VISIBILITY", "Checking field access: .{} on type_id={:?}, object_kind={:?}",
                        field,
                        object.type_id,
                        std::mem::discriminant(&object.kind)
                    );
                }

                // Then check if this is a private field access on an imported struct
                if Self::is_private_field(field) {
                    // Try to get struct name from type_id first
                    let struct_name = if let Some(type_id) = object.type_id {
                        if let Some(info) = self.type_registry.get(type_id) {
                            if let TypeKind::Struct { name, .. } = &info.kind {
                                Some(name.clone())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        // Fallback: try to infer from expression
                        self.get_expr_struct_type(object)
                    };

                    if let Some(struct_name) = struct_name {
                        if std::env::var("DOO_DEBUG").is_ok() {
                            doo_debug!("VISIBILITY", "Field '{}' is private (camelCase), struct='{}', imported={}", 
                                field, struct_name, self.imported_structs.contains(&struct_name));
                        }

                        // Only check imported structs
                        if self.imported_structs.contains(&struct_name) {
                            // Verify field is actually private in type registry
                            if let Some(struct_type_id) = self.type_registry.lookup(&struct_name) {
                                if let Some(info) = self.type_registry.get(struct_type_id) {
                                    if let TypeKind::Struct { fields, .. } = &info.kind {
                                        for (fname, _ftype, is_public) in fields {
                                            if fname == field && !is_public {
                                                if std::env::var("DOO_DEBUG").is_ok() {
                                                    doo_debug!("VISIBILITY", "ERROR: Private field '{}' accessed on imported struct '{}'", field, struct_name);
                                                }
                                                self.errors.push(FieldVisibilityError {
                                                    field_name: field.clone(),
                                                    struct_name: struct_name.clone(),
                                                    span: expr.span,
                                                });
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else if std::env::var("DOO_DEBUG").is_ok() {
                        doo_debug!("VISIBILITY", "Could not determine struct type for field '{}' access",
                            field
                        );
                    }
                }
            }
            // Recursively check sub-expressions
            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.check_expr(lhs);
                self.check_expr(rhs);
            }
            HirExprKind::UnaryOp { operand, .. } => {
                self.check_expr(operand);
            }
            HirExprKind::Call { func, args } => {
                self.check_expr(func);
                for arg in args {
                    self.check_expr(arg);
                }
            }
            HirExprKind::MethodCall { receiver, args, .. } => {
                self.check_expr(receiver);
                for arg in args {
                    self.check_expr(arg);
                }
            }
            HirExprKind::Index { object, index } => {
                self.check_expr(object);
                self.check_expr(index);
            }
            HirExprKind::Array(elements) | HirExprKind::Tuple(elements) => {
                for elem in elements {
                    self.check_expr(elem);
                }
            }
            HirExprKind::Map(entries) => {
                for (k, v) in entries {
                    self.check_expr(k);
                    self.check_expr(v);
                }
            }
            HirExprKind::Struct { fields, .. } => {
                for (_, value) in fields {
                    self.check_expr(value);
                }
            }
            HirExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                self.check_expr(condition);
                self.check_expr(then_expr);
                if let Some(else_e) = else_expr {
                    self.check_expr(else_e);
                }
            }
            HirExprKind::Block { stmts, expr } => {
                for stmt in stmts {
                    self.check_stmt(stmt);
                }
                if let Some(e) = expr {
                    self.check_expr(e);
                }
            }
            HirExprKind::Match { values, arms } => {
                for v in values {
                    self.check_expr(v);
                }
                for arm in arms {
                    if let Some(g) = &arm.guard {
                        self.check_expr(g);
                    }
                    self.check_expr(&arm.body);
                }
            }
            HirExprKind::Range { start, end, .. } => {
                self.check_expr(start);
                self.check_expr(end);
            }
            HirExprKind::Ok(inner)
            | HirExprKind::Err(inner)
            | HirExprKind::Try(inner)
            | HirExprKind::Move(inner)
            | HirExprKind::Clone(inner)
            | HirExprKind::Borrow { expr: inner, .. }
            | HirExprKind::Spread(inner)
            | HirExprKind::Cast { value: inner, .. } => {
                self.check_expr(inner);
            }
            HirExprKind::UnwrapOrPanic {
                expr: inner,
                message,
            } => {
                self.check_expr(inner);
                self.check_expr(message);
            }
            HirExprKind::Closure { body, .. } => {
                self.check_expr(body);
            }
            HirExprKind::EnumVariant { payload, .. } => {
                for p in payload {
                    self.check_expr(p);
                }
            }
            HirExprKind::RouteBlock { routes } => {
                for route in routes {
                    self.check_expr(route);
                }
            }
            // Leaf nodes - no sub-expressions to check
            HirExprKind::Const(_) | HirExprKind::Local { .. } | HirExprKind::Global { .. } => {}
        }
    }

    fn check_stmt(&mut self, stmt: &doo_hir::HirStmt) {
        match &stmt.kind {
            HirStmtKind::Let { value, .. } | HirStmtKind::TupleLet { value, .. } => {
                self.check_expr(value);
            }
            HirStmtKind::ManualErrorExtract { expr, .. } => {
                self.check_expr(expr);
            }
            HirStmtKind::Assign { target, value } => {
                self.check_expr(target);
                self.check_expr(value);
            }
            HirStmtKind::Expr(expr) => {
                self.check_expr(expr);
            }
            HirStmtKind::Return(values) => {
                for v in values {
                    self.check_expr(v);
                }
            }
            HirStmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                self.check_expr(condition);
                for s in then_block {
                    self.check_stmt(s);
                }
                if let Some(else_stmts) = else_block {
                    for s in else_stmts {
                        self.check_stmt(s);
                    }
                }
            }
            HirStmtKind::While { condition, body } => {
                self.check_expr(condition);
                for s in body {
                    self.check_stmt(s);
                }
            }
            HirStmtKind::Break | HirStmtKind::Continue | HirStmtKind::Drop { .. } => {}
        }
    }
}

/// Convenience function to check field visibility in a HIR program.
///
/// Returns a list of error messages for private field access violations.
/// This is the primary entry point for compile.rs.
pub fn check_field_visibility(
    hir: &HirProgram,
    type_registry: &Arc<TypeRegistry>,
    imported_structs: &HashSet<String>,
) -> Vec<String> {
    let mut checker = FieldVisibilityChecker::new(type_registry, imported_structs);
    checker.check_program(hir);
    checker.errors_as_strings()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_public_access() {
        let mut checker = VisibilityChecker::new();
        let span = Span::new(0, 0, 0);

        checker.register_public("foo", "module_a", span);
        checker.set_current_module("module_b");

        assert!(checker.check_access("foo", span).is_ok());
    }

    #[test]
    fn test_private_same_module() {
        let mut checker = VisibilityChecker::new();
        let span = Span::new(0, 0, 0);

        checker.register_private("foo", "module_a", span);
        checker.set_current_module("module_a");

        assert!(checker.check_access("foo", span).is_ok());
    }

    #[test]
    fn test_private_cross_module() {
        let mut checker = VisibilityChecker::new();
        let span = Span::new(0, 0, 0);

        checker.register_private("foo", "module_a", span);
        checker.set_current_module("module_b");

        assert!(checker.check_access("foo", span).is_err());
    }

    #[test]
    fn test_is_accessible() {
        let mut checker = VisibilityChecker::new();
        let span = Span::new(0, 0, 0);

        checker.register_public("pub_fn", "module_a", span);
        checker.register_private("priv_fn", "module_a", span);

        assert!(checker.is_accessible("pub_fn", "module_b"));
        assert!(!checker.is_accessible("priv_fn", "module_b"));
        assert!(checker.is_accessible("priv_fn", "module_a"));
    }
}
