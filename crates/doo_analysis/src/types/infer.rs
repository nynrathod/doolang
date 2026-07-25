//! Type Inference
//!
//! Infers types for expressions and statements, with special handling for closures.
//!
//! ## Closure Type Inference
//!
//! Closures in Doo can have their types inferred from context:
//! - Parameter types from array element types (for map/filter/reduce)
//! - Return types from the closure body expression
//!
//! Example:
//! ```doo
//! let nums = [1, 2, 3];
//! let doubled = nums.map((x) => x * 2);  // x: Int inferred, return Int inferred
//! ```

use doo_core::types::composite::FunctionSig;
use doo_core::types::{builtin, TypeId, TypeKind, TypeRegistry};
use doo_hir::{HirBinOp, HirExpr, HirExprKind, HirStmt, HirStmtKind, HirUnaryOp};
use std::collections::HashMap;

/// Type inference engine.
pub struct TypeInference {
    /// Type constraints collected during inference.
    constraints: Vec<TypeConstraint>,
    /// Local variable types (for closure body inference).
    locals: HashMap<String, TypeId>,
    /// Function return types (name -> return type)
    functions: HashMap<String, TypeId>,
}

/// A type constraint.
#[derive(Debug, Clone)]
pub struct TypeConstraint {
    pub lhs: TypeId,
    pub rhs: TypeId,
}

/// Context for closure type inference.
#[derive(Debug, Clone)]
pub struct ClosureContext {
    /// Expected parameter types (from array element type, etc.)
    pub param_types: Vec<TypeId>,
    /// Expected return type (if known)
    pub expected_return: Option<TypeId>,
}

impl TypeInference {
    pub fn new() -> Self {
        Self {
            constraints: Vec::new(),
            locals: HashMap::new(),
            functions: HashMap::new(),
        }
    }

    /// Register a function's return type for lookup during call inference.
    pub fn register_function(&mut self, name: String, return_type: TypeId) {
        self.functions.insert(name, return_type);
    }

    /// Add a constraint: lhs must be compatible with rhs.
    pub fn constrain(&mut self, lhs: TypeId, rhs: TypeId) {
        self.constraints.push(TypeConstraint { lhs, rhs });
    }

    /// Solve constraints and return unified types.
    pub fn solve(&self) -> Result<(), InferenceError> {
        // Simple unification - full implementation would use union-find
        for c in &self.constraints {
            if !self.unify(c.lhs, c.rhs) {
                return Err(InferenceError::Mismatch(c.lhs, c.rhs));
            }
        }
        Ok(())
    }

    fn unify(&self, a: TypeId, b: TypeId) -> bool {
        // Same type always unifies
        if a == b {
            return true;
        }
        // ANY unifies with everything
        if a == builtin::ANY || b == builtin::ANY {
            return true;
        }
        false
    }

    // ========================================================================
    // Closure Type Inference
    // ========================================================================

    /// Infer types for a closure expression, applying context from the calling site.
    ///
    /// This modifies the closure in-place, filling in:
    /// - Parameter types (from context or existing annotations)
    /// - Return type (from body expression)
    /// - The closure's overall function type
    pub fn infer_closure(
        &mut self,
        expr: &mut HirExpr,
        ctx: &ClosureContext,
        registry: &mut TypeRegistry,
    ) -> Result<TypeId, InferenceError> {
        if let HirExprKind::Closure { params, body } = &mut expr.kind {
            // 1. Infer parameter types
            let param_types = self.infer_closure_params(params, &ctx.param_types);

            // 2. Add parameters to local scope for body inference
            let old_locals = self.locals.clone();
            for (name, type_id) in params.iter() {
                if let Some(tid) = type_id {
                    self.locals.insert(name.clone(), *tid);
                }
            }

            // 3. Infer return type from body
            let return_type = self.infer_closure_return(body, ctx.expected_return, registry)?;

            // 4. Restore old locals
            self.locals = old_locals;

            // 5. Set the body's type_id if not already set
            if body.type_id.is_none() {
                body.type_id = Some(return_type);
            }

            // 6. Create function type and set on closure expression
            let fn_type = registry.register_function(FunctionSig {
                params: param_types,
                return_type: return_type,
                error_type: None,
                is_closure: false,
            });
            expr.type_id = Some(fn_type);

            Ok(return_type)
        } else {
            // Not a closure - try to get return type from existing function type
            if let Some(type_id) = expr.type_id {
                if let Some(info) = registry.get(type_id) {
                    if let TypeKind::Function { sig, .. } = &info.kind {
                        return Ok(sig.return_type);
                    }
                }
            }
            Err(InferenceError::NotAClosure)
        }
    }

    /// Infer closure parameter types from context and existing annotations.
    fn infer_closure_params(
        &self,
        params: &mut [(String, Option<TypeId>)],
        context_types: &[TypeId],
    ) -> Vec<TypeId> {
        let mut result = Vec::with_capacity(params.len());

        for (idx, (_, param_type)) in params.iter_mut().enumerate() {
            let inferred = if let Some(explicit) = *param_type {
                // Parameter has explicit type annotation
                explicit
            } else if let Some(&ctx_type) = context_types.get(idx) {
                // Infer from context (e.g., array element type)
                *param_type = Some(ctx_type);
                ctx_type
            } else {
                // No type available, default to Any
                *param_type = Some(builtin::ANY);
                builtin::ANY
            };
            result.push(inferred);
        }

        result
    }

    /// Infer closure return type from the body expression.
    fn infer_closure_return(
        &mut self,
        body: &mut HirExpr,
        expected: Option<TypeId>,
        registry: &mut TypeRegistry,
    ) -> Result<TypeId, InferenceError> {
        // If body already has a type, use it
        if let Some(type_id) = body.type_id {
            return Ok(type_id);
        }

        // Infer from the body expression
        let inferred = self.infer_expr_type(body, registry);

        // If we have an expected type and inferred doesn't match, check compatibility
        if let Some(exp) = expected {
            if inferred != exp && inferred != builtin::ANY && exp != builtin::ANY {
                // For now, trust the expected type for constraint purposes
                // A full implementation would add a constraint and solve later
                self.constrain(inferred, exp);
            }
        }

        Ok(inferred)
    }

    /// Infer the type of an expression.
    pub fn infer_expr_type(&mut self, expr: &mut HirExpr, registry: &mut TypeRegistry) -> TypeId {
        // If already typed, return it
        if let Some(type_id) = expr.type_id {
            return type_id;
        }

        let inferred = match &mut expr.kind {
            // Constants have known types
            HirExprKind::Const(c) => c.type_id(),

            // Local variable - lookup in scope
            HirExprKind::Local { name } => self.locals.get(name).copied().unwrap_or(builtin::ANY),

            // Binary operations
            HirExprKind::BinOp { op, lhs, rhs } => {
                let lhs_type = self.infer_expr_type(lhs, registry);
                let rhs_type = self.infer_expr_type(rhs, registry);
                self.infer_binop_type(*op, lhs_type, rhs_type)
            }

            // Unary operations
            HirExprKind::UnaryOp { op, operand } => {
                let operand_type = self.infer_expr_type(operand, registry);
                self.infer_unaryop_type(*op, operand_type)
            }

            // Array literal - infer element type from all elements, verify consistency
            HirExprKind::Array(elements) => {
                if elements.is_empty() {
                    // Empty array defaults to [Any]
                    return registry.register_array(builtin::ANY);
                }

                // Infer type from first element
                let first_type = self.infer_expr_type(&mut elements[0].clone(), registry);

                // Verify all elements have compatible types
                for elem in elements.iter_mut().skip(1) {
                    let elem_type = self.infer_expr_type(elem, registry);
                    // Check for type consistency - if mismatch, add constraint
                    if elem_type != first_type
                        && elem_type != builtin::ANY
                        && first_type != builtin::ANY
                    {
                        self.constrain(elem_type, first_type);
                    }
                }

                registry.register_array(first_type)
            }

            // Index access - infer element type from array/map/tuple
            HirExprKind::Index { object, index } => {
                let object_type = self.infer_expr_type(object, registry);
                let _index_type = self.infer_expr_type(index, registry);

                if let Some(info) = registry.get(object_type) {
                    match &info.kind {
                        // Array[Int] -> element type
                        TypeKind::Array { element } => *element,
                        // Map[Key] -> value type
                        TypeKind::Map { value, .. } => *value,
                        // String[Int] -> String (single char)
                        TypeKind::Str => builtin::STR,
                        // Tuple[Int] -> element type at that index
                        TypeKind::Tuple { elements } => {
                            // If the index is a constant int, we can resolve the exact type
                            if let HirExprKind::Const(doo_hir::ConstValue::Int(idx)) = &index.kind {
                                let idx = *idx as usize;
                                if idx < elements.len() {
                                    return elements[idx];
                                }
                            }
                            // If we can't determine the index statically, return Any
                            // (tuples are heterogeneous, so we can't know the type)
                            builtin::ANY
                        }
                        _ => builtin::ANY,
                    }
                } else {
                    builtin::ANY
                }
            }

            // Map literal - infer key and value types
            HirExprKind::Map(entries) => {
                if entries.is_empty() {
                    // Empty map defaults to Map<Any, Any>
                    return registry.register_map(builtin::ANY, builtin::ANY);
                }

                // Infer types from first entry
                let (first_key, first_val) = &mut entries[0].clone();
                let key_type = self.infer_expr_type(first_key, registry);
                let val_type = self.infer_expr_type(first_val, registry);

                // Verify all entries have compatible types
                for (k, v) in entries.iter_mut().skip(1) {
                    let k_type = self.infer_expr_type(k, registry);
                    let v_type = self.infer_expr_type(v, registry);
                    if k_type != key_type && k_type != builtin::ANY && key_type != builtin::ANY {
                        self.constrain(k_type, key_type);
                    }
                    if v_type != val_type && v_type != builtin::ANY && val_type != builtin::ANY {
                        self.constrain(v_type, val_type);
                    }
                }

                registry.register_map(key_type, val_type)
            }

            // Field access - lookup field type on struct or tuple
            HirExprKind::Field { object, field } => {
                let object_type = self.infer_expr_type(object, registry);
                if let Some(info) = registry.get(object_type) {
                    match &info.kind {
                        TypeKind::Struct { def, .. } => {
                            for field_def in &def.fields {
                                let fname = field_def.name.resolve();
                                if fname == field.as_str() {
                                    // Check visibility: private fields (camelCase) cannot be accessed from outside
                                    if !field_def.is_public {
                                        // Field is private - report error
                                        // For now, we'll still return the type but the codegen/semantic
                                        // analysis should catch this. We could add an error here later.
                                        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG)
                                            .is_ok()
                                        {}
                                    }
                                    return field_def.type_id;
                                }
                            }
                        }
                        // Tuple field access: tuple.0, tuple.1, etc.
                        TypeKind::Tuple { elements } => {
                            if let Ok(idx) = field.parse::<usize>() {
                                if idx < elements.len() {
                                    return elements[idx];
                                }
                            }
                        }
                        _ => {}
                    }
                }
                builtin::ANY
            }

            // Global variable/function reference - type should be set already
            HirExprKind::Global { .. } => {
                // Global type should be pre-set from function table or global scope
                builtin::ANY
            }

            // Block - type is the type of the final expression
            HirExprKind::Block {
                stmts,
                expr: final_expr,
            } => {
                // Process statements to build up local scope
                for stmt in stmts.iter_mut() {
                    self.process_stmt_for_inference(stmt, registry);
                }
                // Type of block is type of final expression (or Void)
                if let Some(final_e) = final_expr {
                    self.infer_expr_type(final_e, registry)
                } else {
                    builtin::VOID
                }
            }

            // If expression - unify then/else types
            HirExprKind::If {
                then_expr,
                else_expr,
                ..
            } => {
                let then_type = self.infer_expr_type(then_expr, registry);
                if let Some(else_e) = else_expr {
                    let else_type = self.infer_expr_type(else_e, registry);
                    // Return the more specific type (prefer non-Any)
                    if then_type == builtin::ANY {
                        else_type
                    } else {
                        then_type
                    }
                } else {
                    then_type
                }
            }

            // Method call - handled by caller typically
            HirExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                let recv_type = self.infer_expr_type(receiver, registry);
                self.infer_method_return_type(recv_type, method, args, registry)
            }

            // Closure - infer with empty context if no context provided
            HirExprKind::Closure { params, body } => {
                // Build param types from what we have
                let param_types: Vec<TypeId> = params
                    .iter()
                    .map(|(_, t)| t.unwrap_or(builtin::ANY))
                    .collect();
                let return_type = self.infer_expr_type(body, registry);
                registry.register_function(FunctionSig {
                    params: param_types,
                    return_type,
                    error_type: None,
                    is_closure: false,
                })
            }

            // Tuple
            HirExprKind::Tuple(elements) => {
                let elem_types: Vec<TypeId> = elements
                    .iter_mut()
                    .map(|e| self.infer_expr_type(e, registry))
                    .collect();
                registry.register_tuple(elem_types)
            }

            // Ok/Err wrappers
            HirExprKind::Ok(inner) => {
                let inner_type = self.infer_expr_type(inner, registry);
                registry.register_result(inner_type, builtin::ERROR)
            }
            HirExprKind::Err(inner) => {
                let inner_type = self.infer_expr_type(inner, registry);
                registry.register_result(builtin::ANY, inner_type)
            }

            // Try - unwraps Result
            HirExprKind::Try(inner) => {
                let inner_type = self.infer_expr_type(inner, registry);
                if let Some(info) = registry.get(inner_type) {
                    if let TypeKind::Result { ok, .. } = &info.kind {
                        return *ok;
                    }
                }
                builtin::ANY
            }

            // Cast expression - type is the target type
            HirExprKind::Cast { value, to_type } => {
                // Infer the value type (even though we don't use it for the result)
                self.infer_expr_type(value, registry);
                // Return the target type
                *to_type
            }

            // Function call - infer argument types and return type
            HirExprKind::Call { func, args } => {
                // Infer all argument types (important for typeOf and other builtins)
                for arg in args.iter_mut() {
                    self.infer_expr_type(arg, registry);
                }

                // Check if this is a builtin with known return type
                if let HirExprKind::Local { name } | HirExprKind::Global { name } = &func.kind {
                    match name.as_str() {
                        "typeOf" => return builtin::STR,
                        "print" | "println" | "__print_interp" => return builtin::VOID,
                        _ => {
                            // Look up function return type from our registry
                            if let Some(&ret_type) = self.functions.get(name) {
                                return ret_type;
                            }
                        }
                    }
                }

                // For other functions, default to ANY
                builtin::ANY
            }

            // Default for unhandled cases
            _ => builtin::ANY,
        };

        // Cache the inferred type
        expr.type_id = Some(inferred);
        inferred
    }

    /// Process a statement to update local variable types.
    fn process_stmt_for_inference(&mut self, stmt: &mut HirStmt, registry: &mut TypeRegistry) {
        match &mut stmt.kind {
            HirStmtKind::Let {
                name,
                value,
                type_id,
                ..
            } => {
                // Infer value type
                let value_type = self.infer_expr_type(value, registry);
                // Use explicit type if provided, otherwise use inferred
                let var_type = type_id.unwrap_or(value_type);
                self.locals.insert(name.clone(), var_type);
                // Update type_id if not set
                if type_id.is_none() {
                    *type_id = Some(value_type);
                }
            }
            HirStmtKind::TupleLet {
                names,
                value,
                type_ids,
                ..
            } => {
                // Infer value type (should be a tuple)
                let value_type = self.infer_expr_type(value, registry);

                // Try to get element types from the tuple type
                let element_types: Vec<TypeId> = if let Some(info) = registry.get(value_type) {
                    if let TypeKind::Tuple { elements } = &info.kind {
                        elements.clone()
                    } else {
                        vec![builtin::ANY; names.len()]
                    }
                } else {
                    vec![builtin::ANY; names.len()]
                };

                // Register each variable with its inferred type
                for (i, name) in names.iter().enumerate() {
                    let var_type = type_ids
                        .get(i)
                        .and_then(|t| *t)
                        .unwrap_or_else(|| element_types.get(i).copied().unwrap_or(builtin::ANY));
                    self.locals.insert(name.clone(), var_type);

                    // Update type_ids if not set
                    if type_ids.get(i).map(|t| t.is_none()).unwrap_or(true) {
                        if i < type_ids.len() {
                            type_ids[i] = Some(var_type);
                        }
                    }
                }
            }
            HirStmtKind::Expr(expr) => {
                self.infer_expr_type(expr, registry);
            }
            HirStmtKind::Return(exprs) => {
                for expr in exprs.iter_mut() {
                    self.infer_expr_type(expr, registry);
                }
            }
            _ => {}
        }
    }

    /// Infer the result type of a binary operation.
    fn infer_binop_type(&self, op: HirBinOp, lhs: TypeId, rhs: TypeId) -> TypeId {
        match op {
            // Arithmetic - preserve numeric type
            HirBinOp::Add | HirBinOp::Sub | HirBinOp::Mul | HirBinOp::Div | HirBinOp::Mod => {
                // String concatenation: Str + anything → Str (auto-coerce rhs)
                if lhs == builtin::STR {
                    return builtin::STR;
                }
                // Float if either operand is Float
                if lhs == builtin::FLOAT || rhs == builtin::FLOAT {
                    builtin::FLOAT
                } else {
                    builtin::INT
                }
            }
            // Comparison - always Bool
            HirBinOp::Eq
            | HirBinOp::NotEq
            | HirBinOp::Lt
            | HirBinOp::Gt
            | HirBinOp::LtEq
            | HirBinOp::GtEq
            | HirBinOp::In => builtin::BOOL,
            // Logical - always Bool
            HirBinOp::And | HirBinOp::Or => builtin::BOOL,
            // Bitwise - preserve Int
            HirBinOp::BitAnd | HirBinOp::BitOr => builtin::INT,
            // Nil coalescing: result type is the non-nil operand type
            HirBinOp::NullCoalesce => {
                if lhs == builtin::VOID || lhs == builtin::ANY {
                    rhs
                } else {
                    lhs
                }
            }
        }
    }

    /// Infer the result type of a unary operation.
    fn infer_unaryop_type(&self, op: HirUnaryOp, operand: TypeId) -> TypeId {
        match op {
            HirUnaryOp::Neg => operand,       // -x has same type as x
            HirUnaryOp::Not => builtin::BOOL, // !x is always Bool
        }
    }

    /// Infer method call return type.
    fn infer_method_return_type(
        &mut self,
        receiver_type: TypeId,
        method: &str,
        args: &mut [HirExpr],
        registry: &mut TypeRegistry,
    ) -> TypeId {
        if let Some(info) = registry.get(receiver_type) {
            match &info.kind {
                TypeKind::Array { element } => {
                    let elem_type = *element;
                    return self.infer_array_method_type(elem_type, method, args, registry);
                }
                TypeKind::Str => {
                    return self.infer_string_method_type(method, registry);
                }
                TypeKind::Map { key, value } => {
                    return self.infer_map_method_type(*key, *value, method, registry);
                }
                _ => {}
            }
        }
        builtin::ANY
    }

    /// Infer array method return type with closure type inference.
    fn infer_array_method_type(
        &mut self,
        elem_type: TypeId,
        method: &str,
        args: &mut [HirExpr],
        registry: &mut TypeRegistry,
    ) -> TypeId {
        match method {
            // Methods that return Int
            "len" | "indexOf" => builtin::INT,

            // Methods that return Bool
            "isEmpty" | "contains" => builtin::BOOL,

            // Methods that return Str
            "join" => builtin::STR,

            // Methods that return element type
            "first" | "last" | "pop" => elem_type,

            // Methods that return the same array type
            "slice" => registry.register_array(elem_type),

            // Methods that return Void (mutating)
            "push" | "clear" | "sort" | "reverse" => builtin::VOID,

            // map - infer closure return type, return array of that type
            "map" => {
                if let Some(closure) = args.get_mut(0) {
                    let ctx = ClosureContext {
                        param_types: vec![elem_type],
                        expected_return: None,
                    };
                    if let Ok(return_type) = self.infer_closure(closure, &ctx, registry) {
                        return registry.register_array(return_type);
                    }
                }
                registry.register_array(builtin::ANY)
            }

            // filter - closure must return Bool, array type preserved
            "filter" => {
                if let Some(closure) = args.get_mut(0) {
                    let ctx = ClosureContext {
                        param_types: vec![elem_type],
                        expected_return: Some(builtin::BOOL),
                    };
                    let _ = self.infer_closure(closure, &ctx, registry);
                }
                registry.register_array(elem_type)
            }

            // reduce - closure takes (acc, elem), returns acc type
            "reduce" => {
                // First arg is initial value
                let init_type = if let Some(init) = args.get_mut(0) {
                    self.infer_expr_type(init, registry)
                } else {
                    builtin::ANY
                };

                // Second arg is closure (acc, elem) -> acc
                if let Some(closure) = args.get_mut(1) {
                    let ctx = ClosureContext {
                        param_types: vec![init_type, elem_type],
                        expected_return: Some(init_type),
                    };
                    let _ = self.infer_closure(closure, &ctx, registry);
                }

                init_type
            }

            _ => builtin::ANY,
        }
    }

    /// Infer string method return type.
    fn infer_string_method_type(&self, method: &str, registry: &mut TypeRegistry) -> TypeId {
        match method {
            // Methods that return Int
            "len" | "indexOf" | "charCode" | "countSubstr" => builtin::INT,

            // Methods that return Bool
            "contains" | "startsWith" | "endsWith" | "isEmpty" => builtin::BOOL,

            // Methods that return Str
            "trim" | "toUpper" | "toLower" | "substring" | "replace" | "charAt" | "concat"
            | "reverse" | "repeat" => builtin::STR,

            // Methods that return Array of Str
            "split" => registry.register_array(builtin::STR),

            _ => builtin::ANY,
        }
    }

    /// Infer map method return type.
    fn infer_map_method_type(
        &self,
        key_type: TypeId,
        value_type: TypeId,
        method: &str,
        registry: &mut TypeRegistry,
    ) -> TypeId {
        match method {
            // Methods that return Int
            "len" | "size" => builtin::INT,

            // Methods that return Bool
            "isEmpty" | "has" | "remove" | "clear" | "containsKey" | "containsValue" => {
                builtin::BOOL
            }

            // Methods that return the value type
            "get" => value_type,

            // Methods that return arrays
            "keys" => registry.register_array(key_type),
            "values" => registry.register_array(value_type),

            _ => builtin::ANY,
        }
    }

    /// Define a local variable type (for use during analysis).
    pub fn define_local(&mut self, name: String, type_id: TypeId) {
        self.locals.insert(name, type_id);
    }

    /// Lookup a local variable type.
    pub fn lookup_local(&self, name: &str) -> Option<TypeId> {
        self.locals.get(name).copied()
    }

    /// Enter a new scope (saves current locals).
    pub fn push_scope(&self) -> HashMap<String, TypeId> {
        self.locals.clone()
    }

    /// Exit a scope (restores saved locals).
    pub fn pop_scope(&mut self, saved: HashMap<String, TypeId>) {
        self.locals = saved;
    }
}

impl Default for TypeInference {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum InferenceError {
    /// Type mismatch between expected and found.
    Mismatch(TypeId, TypeId),
    /// Expression is not a closure.
    NotAClosure,
}

#[cfg(test)]
mod tests {
    use super::*;
    use doo_hir::HirExpr;

    #[test]
    fn test_infer_binop_int() {
        let inf = TypeInference::new();
        assert_eq!(
            inf.infer_binop_type(HirBinOp::Add, builtin::INT, builtin::INT),
            builtin::INT
        );
    }

    #[test]
    fn test_infer_binop_float() {
        let inf = TypeInference::new();
        assert_eq!(
            inf.infer_binop_type(HirBinOp::Mul, builtin::INT, builtin::FLOAT),
            builtin::FLOAT
        );
    }

    #[test]
    fn test_infer_comparison_bool() {
        let inf = TypeInference::new();
        assert_eq!(
            inf.infer_binop_type(HirBinOp::Lt, builtin::INT, builtin::INT),
            builtin::BOOL
        );
    }

    #[test]
    fn test_infer_unary_neg() {
        let inf = TypeInference::new();
        assert_eq!(
            inf.infer_unaryop_type(HirUnaryOp::Neg, builtin::FLOAT),
            builtin::FLOAT
        );
    }

    #[test]
    fn test_infer_unary_not() {
        let inf = TypeInference::new();
        assert_eq!(
            inf.infer_unaryop_type(HirUnaryOp::Not, builtin::BOOL),
            builtin::BOOL
        );
    }

    #[test]
    fn test_infer_array_index_type() {
        use doo_core::Span;
        use doo_hir::ConstValue;

        let mut registry = TypeRegistry::new();
        let mut inf = TypeInference::new();

        // Create an array type [Int]
        let array_type = registry.register_array(builtin::INT);

        // Simulate: arr[0] where arr is [Int]
        // The index expression should return Int (element type)
        let mut arr_expr = HirExpr::new(
            HirExprKind::Local {
                name: "arr".to_string(),
            },
            Span::default(),
        );
        arr_expr.type_id = Some(array_type);

        let index_expr = HirExpr::new(HirExprKind::Const(ConstValue::Int(0)), Span::default());

        let mut index_access = HirExpr::new(
            HirExprKind::Index {
                object: Box::new(arr_expr),
                index: Box::new(index_expr),
            },
            Span::default(),
        );

        let result_type = inf.infer_expr_type(&mut index_access, &mut registry);
        assert_eq!(result_type, builtin::INT);
    }

    #[test]
    fn test_infer_array_literal_type() {
        use doo_core::Span;
        use doo_hir::ConstValue;

        let mut registry = TypeRegistry::new();
        let mut inf = TypeInference::new();

        // Create array literal [1, 2, 3]
        let elements = vec![
            HirExpr::new(HirExprKind::Const(ConstValue::Int(1)), Span::default()),
            HirExpr::new(HirExprKind::Const(ConstValue::Int(2)), Span::default()),
            HirExpr::new(HirExprKind::Const(ConstValue::Int(3)), Span::default()),
        ];

        let mut array_expr = HirExpr::new(HirExprKind::Array(elements), Span::default());

        let result_type = inf.infer_expr_type(&mut array_expr, &mut registry);

        // Should be Array<Int>
        if let Some(info) = registry.get(result_type) {
            match &info.kind {
                TypeKind::Array { element } => {
                    assert_eq!(*element, builtin::INT);
                }
                _ => panic!("Expected Array type"),
            }
        } else {
            panic!("Type not found in registry");
        }
    }

    #[test]
    fn test_infer_map_index_type() {
        use doo_core::Span;
        use doo_hir::ConstValue;

        let mut registry = TypeRegistry::new();
        let mut inf = TypeInference::new();

        // Create a map type Map<Str, Int>
        let map_type = registry.register_map(builtin::STR, builtin::INT);

        // Simulate: map["key"] where map is Map<Str, Int>
        // The index expression should return Int (value type)
        let mut map_expr = HirExpr::new(
            HirExprKind::Local {
                name: "map".to_string(),
            },
            Span::default(),
        );
        map_expr.type_id = Some(map_type);

        let key_expr = HirExpr::new(
            HirExprKind::Const(ConstValue::Str("key".to_string())),
            Span::default(),
        );

        let mut index_access = HirExpr::new(
            HirExprKind::Index {
                object: Box::new(map_expr),
                index: Box::new(key_expr),
            },
            Span::default(),
        );

        let result_type = inf.infer_expr_type(&mut index_access, &mut registry);
        assert_eq!(result_type, builtin::INT);
    }

    #[test]
    fn test_infer_string_index_type() {
        use doo_core::Span;
        use doo_hir::ConstValue;

        let mut registry = TypeRegistry::new();
        let mut inf = TypeInference::new();

        // Simulate: str[0] where str is Str
        // The index expression should return Str (single char)
        let mut str_expr = HirExpr::new(
            HirExprKind::Local {
                name: "s".to_string(),
            },
            Span::default(),
        );
        str_expr.type_id = Some(builtin::STR);

        let index_expr = HirExpr::new(HirExprKind::Const(ConstValue::Int(0)), Span::default());

        let mut index_access = HirExpr::new(
            HirExprKind::Index {
                object: Box::new(str_expr),
                index: Box::new(index_expr),
            },
            Span::default(),
        );

        let result_type = inf.infer_expr_type(&mut index_access, &mut registry);
        assert_eq!(result_type, builtin::STR);
    }

    #[test]
    fn test_infer_map_literal_type() {
        use doo_core::Span;
        use doo_hir::ConstValue;

        let mut registry = TypeRegistry::new();
        let mut inf = TypeInference::new();

        // Create map literal {"name": "Alice", "role": "admin"}
        let entries = vec![
            (
                HirExpr::new(
                    HirExprKind::Const(ConstValue::Str("name".to_string())),
                    Span::default(),
                ),
                HirExpr::new(
                    HirExprKind::Const(ConstValue::Str("Alice".to_string())),
                    Span::default(),
                ),
            ),
            (
                HirExpr::new(
                    HirExprKind::Const(ConstValue::Str("role".to_string())),
                    Span::default(),
                ),
                HirExpr::new(
                    HirExprKind::Const(ConstValue::Str("admin".to_string())),
                    Span::default(),
                ),
            ),
        ];

        let mut map_expr = HirExpr::new(HirExprKind::Map(entries), Span::default());

        let result_type = inf.infer_expr_type(&mut map_expr, &mut registry);

        // Should be Map<Str, Str>
        if let Some(info) = registry.get(result_type) {
            match &info.kind {
                TypeKind::Map { key, value } => {
                    assert_eq!(*key, builtin::STR);
                    assert_eq!(*value, builtin::STR);
                }
                _ => panic!("Expected Map type"),
            }
        } else {
            panic!("Type not found in registry");
        }
    }

    #[test]
    fn test_infer_map_int_keys_literal_type() {
        use doo_core::Span;
        use doo_hir::ConstValue;

        let mut registry = TypeRegistry::new();
        let mut inf = TypeInference::new();

        // Create map literal {1: 10, 2: 20}
        let entries = vec![
            (
                HirExpr::new(HirExprKind::Const(ConstValue::Int(1)), Span::default()),
                HirExpr::new(HirExprKind::Const(ConstValue::Int(10)), Span::default()),
            ),
            (
                HirExpr::new(HirExprKind::Const(ConstValue::Int(2)), Span::default()),
                HirExpr::new(HirExprKind::Const(ConstValue::Int(20)), Span::default()),
            ),
        ];

        let mut map_expr = HirExpr::new(HirExprKind::Map(entries), Span::default());

        let result_type = inf.infer_expr_type(&mut map_expr, &mut registry);

        // Should be Map<Int, Int>
        if let Some(info) = registry.get(result_type) {
            match &info.kind {
                TypeKind::Map { key, value } => {
                    assert_eq!(*key, builtin::INT);
                    assert_eq!(*value, builtin::INT);
                }
                _ => panic!("Expected Map type"),
            }
        } else {
            panic!("Type not found in registry");
        }
    }

    #[test]
    fn test_infer_map_method_keys() {
        let mut registry = TypeRegistry::new();
        let inf = TypeInference::new();

        // map.keys() on Map<Str, Int> should return [Str]
        let result = inf.infer_map_method_type(builtin::STR, builtin::INT, "keys", &mut registry);

        if let Some(info) = registry.get(result) {
            match &info.kind {
                TypeKind::Array { element } => {
                    assert_eq!(*element, builtin::STR);
                }
                _ => panic!("Expected Array type"),
            }
        } else {
            panic!("Type not found in registry");
        }
    }

    #[test]
    fn test_infer_map_method_values() {
        let mut registry = TypeRegistry::new();
        let inf = TypeInference::new();

        // map.values() on Map<Str, Int> should return [Int]
        let result = inf.infer_map_method_type(builtin::STR, builtin::INT, "values", &mut registry);

        if let Some(info) = registry.get(result) {
            match &info.kind {
                TypeKind::Array { element } => {
                    assert_eq!(*element, builtin::INT);
                }
                _ => panic!("Expected Array type"),
            }
        } else {
            panic!("Type not found in registry");
        }
    }

    #[test]
    fn test_infer_map_method_has() {
        let mut registry = TypeRegistry::new();
        let inf = TypeInference::new();

        // map.has(key) should return Bool
        let result = inf.infer_map_method_type(builtin::STR, builtin::INT, "has", &mut registry);
        assert_eq!(result, builtin::BOOL);
    }

    #[test]
    fn test_infer_map_method_len() {
        let mut registry = TypeRegistry::new();
        let inf = TypeInference::new();

        // map.len() should return Int
        let result = inf.infer_map_method_type(builtin::STR, builtin::INT, "len", &mut registry);
        assert_eq!(result, builtin::INT);
    }

    #[test]
    fn test_infer_empty_map_type() {
        use doo_core::Span;

        let mut registry = TypeRegistry::new();
        let mut inf = TypeInference::new();

        // Create empty map literal {}
        let entries: Vec<(HirExpr, HirExpr)> = vec![];
        let mut map_expr = HirExpr::new(HirExprKind::Map(entries), Span::default());

        let result_type = inf.infer_expr_type(&mut map_expr, &mut registry);

        // Empty map defaults to Map<Any, Any>
        if let Some(info) = registry.get(result_type) {
            match &info.kind {
                TypeKind::Map { key, value } => {
                    assert_eq!(*key, builtin::ANY);
                    assert_eq!(*value, builtin::ANY);
                }
                _ => panic!("Expected Map type"),
            }
        } else {
            panic!("Type not found in registry");
        }
    }
}
