//! Type Checker
//!
//! Validates types and infers missing types.

use doo_core::{Span, types::{TypeId, builtin}};
use doo_hir::{HirProgram, HirItem, HirFunction, HirStmt, HirExpr, HirExprKind};
use super::scope::{ScopeManager, Symbol, SymbolKind};

/// Type checking error.
#[derive(Debug, Clone)]
pub struct TypeError {
    pub kind: TypeErrorKind,
    pub span: Span,
}

/// Kinds of type errors.
#[derive(Debug, Clone)]
pub enum TypeErrorKind {
    /// Type mismatch (expected X, found Y).
    Mismatch { expected: TypeId, found: TypeId },
    /// Undefined variable.
    Undefined(String),
    /// Invalid operation for type.
    InvalidOp(String),
    /// Function argument mismatch.
    ArgMismatch { expected: usize, found: usize },
}

/// The type checker.
pub struct TypeChecker {
    /// Scope manager for symbol tracking.
    scopes: ScopeManager,
    /// Collected errors.
    errors: Vec<TypeError>,
}

impl TypeChecker {
    /// Create a new type checker.
    pub fn new() -> Self {
        Self {
            scopes: ScopeManager::new(),
            errors: Vec::new(),
        }
    }

    /// Check an entire program.
    pub fn check(&mut self, program: &HirProgram) -> Result<(), Vec<TypeError>> {
        for item in &program.items {
            if let HirItem::Function(func) = item {
                self.check_function(func);
            }
        }
        
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors.clone())
        }
    }

    /// Check a function.
    fn check_function(&mut self, func: &HirFunction) {
        // Register function parameters in scope
        self.scopes.enter_scope(super::scope::ScopeKind::Function);
        
        for param in &func.params {
            let _ = self.scopes.define(Symbol {
                name: param.name.clone(),
                kind: SymbolKind::Parameter,
                type_id: param.type_id.or(Some(builtin::ANY)), // Default to Any if unknown
                mutable: false,
                span: param.span,
                used: false,
            });
        }

        // Check body statements
        for stmt in &func.body {
            self.check_stmt(stmt);
        }

        self.scopes.exit_scope();
    }

    // Placeholder for statement checking - full implementation would match HirStmtKind
    fn check_stmt(&mut self, _stmt: &HirStmt) {
        // Logic to check statements and recurse into expressions
    }

    // Placeholder for expression checking - full implementation would infer types
    fn check_expr(&mut self, expr: &HirExpr) -> TypeId {
        match &expr.kind {
            HirExprKind::Const(c) => c.type_id(),
            HirExprKind::Local { name } => {
                if let Some(sym) = self.scopes.lookup(name) {
                    sym.type_id.unwrap_or(builtin::ANY)
                } else {
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::Undefined(name.clone()),
                        span: expr.span,
                    });
                    builtin::ANY
                }
            }
            // ... other expression kinds
            _ => builtin::ANY,
        }
    }
}
