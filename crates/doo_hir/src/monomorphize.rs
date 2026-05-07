//! Monomorphization Pass
//!
//! Transforms generic function/struct templates into concrete instantiations.
//!
//! ## How It Works
//!
//! 1. Collect all generic function templates (functions with non-empty `type_params`)
//! 2. Walk all concrete function bodies to find calls to generic functions
//! 3. For each call site, infer the concrete types from argument types
//! 4. Create a specialized clone of the generic function with all TypeParam
//!    TypeIds replaced by the inferred concrete TypeIds
//! 5. Give it a mangled name (e.g. `identity__Int`)
//! 6. Rewrite the call to target the mangled name
//! 7. Add all specialized functions to the program
//!
//! This runs AFTER HIR lowering and BEFORE semantic analysis.

use doo_core::types::{TypeId, TypeKind, TypeRegistry};
use rustc_hash::FxHashMap;

use crate::types::*;

/// Monomorphization context.
pub struct Monomorphizer<'a> {
    /// Type registry for looking up and creating types.
    registry: &'a mut TypeRegistry,
    /// Generic function templates: name → HirFunction.
    generic_functions: FxHashMap<String, HirFunction>,
    /// Generic struct templates: name → HirStruct.
    generic_structs: FxHashMap<String, HirStruct>,
    /// Already-generated specializations: (generic_name, concrete_types) → mangled_name.
    /// Prevents duplicate generation of the same specialization.
    specializations: FxHashMap<(String, Vec<TypeId>), String>,
    /// New concrete functions to add to the program.
    new_functions: Vec<HirFunction>,
    /// New concrete structs to add to the program.
    new_structs: Vec<HirStruct>,
    /// Per-function binding name → concretized TypeId.
    /// Tracks each `let name = ...` so subsequent `Local { name }` references
    /// can have their abstract (TypeParam-laden) type_ids overwritten with the
    /// concrete types inferred at the binding's RHS. Cleared per function.
    binding_types: FxHashMap<String, TypeId>,
    /// Struct specialization cache: (generic_name, concrete_types) → (mangled, new_type_id).
    struct_specializations: FxHashMap<(String, Vec<TypeId>), (String, TypeId)>,
}

impl<'a> Monomorphizer<'a> {
    pub fn new(registry: &'a mut TypeRegistry) -> Self {
        Self {
            registry,
            generic_functions: FxHashMap::default(),
            generic_structs: FxHashMap::default(),
            specializations: FxHashMap::default(),
            new_functions: Vec::new(),
            new_structs: Vec::new(),
            binding_types: FxHashMap::default(),
            struct_specializations: FxHashMap::default(),
        }
    }

    /// Deeply substitute a TypeId using a TypeParam→concrete map.
    ///
    /// - If `tid` is in the map, returns the mapped concrete TypeId.
    /// - If `tid` is a composite type (Tuple/Array/Map/Optional/Result/Function),
    ///   recursively substitutes its components and registers a new composite
    ///   TypeId via the registry. The registry interns by structural name, so
    ///   identical composites dedup automatically — no hardcoding.
    /// - Otherwise returns `tid` unchanged.
    fn substitute_type_id(
        &mut self,
        tid: TypeId,
        type_map: &FxHashMap<TypeId, TypeId>,
    ) -> TypeId {
        if let Some(&concrete) = type_map.get(&tid) {
            return concrete;
        }
        let kind = match self.registry.get(tid) {
            Some(info) => info.kind.clone(),
            None => return tid,
        };
        match kind {
            TypeKind::Tuple { elements } => {
                let new_elements: Vec<TypeId> = elements
                    .iter()
                    .map(|&e| self.substitute_type_id(e, type_map))
                    .collect();
                if new_elements == elements {
                    tid
                } else {
                    self.registry.register_tuple(new_elements)
                }
            }
            TypeKind::Array { element } => {
                let ne = self.substitute_type_id(element, type_map);
                if ne == element {
                    tid
                } else {
                    self.registry.register_array(ne)
                }
            }
            TypeKind::Map { key, value } => {
                let nk = self.substitute_type_id(key, type_map);
                let nv = self.substitute_type_id(value, type_map);
                if nk == key && nv == value {
                    tid
                } else {
                    self.registry.register_map(nk, nv)
                }
            }
            TypeKind::Optional { inner } => {
                let ni = self.substitute_type_id(inner, type_map);
                if ni == inner {
                    tid
                } else {
                    self.registry.register_optional(ni)
                }
            }
            TypeKind::Result { ok, err } => {
                let no = self.substitute_type_id(ok, type_map);
                let ne = self.substitute_type_id(err, type_map);
                if no == ok && ne == err {
                    tid
                } else {
                    self.registry.register_result(no, ne)
                }
            }
            TypeKind::Function { params, returns } => {
                let np: Vec<TypeId> = params
                    .iter()
                    .map(|&p| self.substitute_type_id(p, type_map))
                    .collect();
                let nr = self.substitute_type_id(returns, type_map);
                if np == params && nr == returns {
                    tid
                } else {
                    self.registry.register_function(np, nr)
                }
            }
            _ => tid,
        }
    }

    /// Build the TypeParam→concrete substitution map for a (generic, concrete_types) pair.
    fn build_type_map(
        &self,
        generic_func: &HirFunction,
        concrete_types: &[TypeId],
    ) -> FxHashMap<TypeId, TypeId> {
        let mut type_map: FxHashMap<TypeId, TypeId> = FxHashMap::default();
        for (tp_name, &concrete_id) in generic_func.type_params.iter().zip(concrete_types.iter()) {
            let tp_key = format!("__typeparam_{}", tp_name);
            if let Some(tp_id) = self.registry.lookup(&tp_key) {
                type_map.insert(tp_id, concrete_id);
            }
        }
        type_map
    }

    /// Same as build_type_map but for a generic struct's type params.
    fn build_struct_type_map(
        &self,
        generic_struct: &HirStruct,
        concrete_types: &[TypeId],
    ) -> FxHashMap<TypeId, TypeId> {
        let mut type_map: FxHashMap<TypeId, TypeId> = FxHashMap::default();
        for (tp_name, &concrete_id) in generic_struct
            .type_params
            .iter()
            .zip(concrete_types.iter())
        {
            let tp_key = format!("__typeparam_{}", tp_name);
            if let Some(tp_id) = self.registry.lookup(&tp_key) {
                type_map.insert(tp_id, concrete_id);
            }
        }
        type_map
    }

    /// Infer the concrete types for a generic struct's type params from a literal's
    /// field values. Each generic field whose declared type is (or contains) a
    /// TypeParam is matched against the corresponding value's type to deduce T.
    fn infer_types_from_struct_fields(
        &self,
        generic_struct: &HirStruct,
        fields: &[(String, HirExpr)],
    ) -> Option<Vec<TypeId>> {
        let type_params = &generic_struct.type_params;
        if type_params.is_empty() {
            return None;
        }

        let mut substitution: FxHashMap<String, TypeId> = FxHashMap::default();

        for (field_name, value_expr) in fields {
            let Some(field_def) = generic_struct
                .fields
                .iter()
                .find(|f| &f.name == field_name)
            else {
                continue;
            };
            let Some(field_tid) = field_def.type_id else {
                continue;
            };
            let Some(value_tid) = value_expr.type_id else {
                continue;
            };
            self.unify_type_param(field_tid, value_tid, &mut substitution);
        }

        let mut result = Vec::with_capacity(type_params.len());
        for tp in type_params {
            if let Some(&concrete) = substitution.get(tp) {
                result.push(concrete);
            } else {
                result.push(doo_core::types::builtin::ANY);
            }
        }
        Some(result)
    }

    /// Walk paired (declared, actual) types to discover TypeParam→concrete bindings.
    /// Recurses into composites so `[T]` matched with `[Int]` yields T=Int.
    fn unify_type_param(
        &self,
        decl: TypeId,
        actual: TypeId,
        out: &mut FxHashMap<String, TypeId>,
    ) {
        if let Some(name) = self.registry.type_param_name(decl) {
            // Don't pollute the binding with another TypeParam or ANY.
            if !self.registry.is_type_param(actual)
                && actual != doo_core::types::builtin::ANY
            {
                out.insert(name.to_string(), actual);
            }
            return;
        }
        let Some(decl_kind) = self.registry.get(decl).map(|i| i.kind.clone()) else {
            return;
        };
        let Some(actual_kind) = self.registry.get(actual).map(|i| i.kind.clone()) else {
            return;
        };
        match (decl_kind, actual_kind) {
            (TypeKind::Array { element: a }, TypeKind::Array { element: b }) => {
                self.unify_type_param(a, b, out);
            }
            (
                TypeKind::Map { key: k1, value: v1 },
                TypeKind::Map { key: k2, value: v2 },
            ) => {
                self.unify_type_param(k1, k2, out);
                self.unify_type_param(v1, v2, out);
            }
            (TypeKind::Tuple { elements: a }, TypeKind::Tuple { elements: b })
                if a.len() == b.len() =>
            {
                for (x, y) in a.iter().zip(b.iter()) {
                    self.unify_type_param(*x, *y, out);
                }
            }
            (TypeKind::Optional { inner: a }, TypeKind::Optional { inner: b }) => {
                self.unify_type_param(a, b, out);
            }
            (
                TypeKind::Result { ok: o1, err: e1 },
                TypeKind::Result { ok: o2, err: e2 },
            ) => {
                self.unify_type_param(o1, o2, out);
                self.unify_type_param(e1, e2, out);
            }
            _ => {}
        }
    }

    /// Get or create a concrete specialization of a generic struct.
    /// Returns the mangled name and the new TypeId registered in the type registry.
    fn get_or_create_struct_specialization(
        &mut self,
        generic_struct: &HirStruct,
        concrete_types: &[TypeId],
    ) -> (String, TypeId) {
        let key = (generic_struct.name.clone(), concrete_types.to_vec());
        if let Some(entry) = self.struct_specializations.get(&key) {
            return entry.clone();
        }

        let type_names: Vec<String> = concrete_types
            .iter()
            .map(|tid| self.registry.display_name(*tid))
            .collect();
        let mangled = format!("{}__{}", generic_struct.name, type_names.join("_"));

        let type_map = self.build_struct_type_map(generic_struct, concrete_types);

        // Clone the generic struct, substitute field types, retire the type_params.
        let mut specialized = generic_struct.clone();
        specialized.name = mangled.clone();
        specialized.type_params = vec![];
        for field in &mut specialized.fields {
            if let Some(ref mut tid) = field.type_id {
                *tid = self.substitute_type_id(*tid, &type_map);
            }
            if let Some(ref mut def) = field.default {
                self.substitute_expr(def, &type_map);
            }
        }

        // Register the concretized struct in the type registry as a single source
        // of truth. Codegen and the type system look it up via this TypeId.
        let registry_fields: Vec<(String, TypeId, bool)> = specialized
            .fields
            .iter()
            .map(|f| {
                (
                    f.name.clone(),
                    f.type_id.unwrap_or(doo_core::types::builtin::ANY),
                    f.is_public,
                )
            })
            .collect();
        let new_type_id = self
            .registry
            .register_struct(&mangled, registry_fields, std::collections::HashMap::new());

        self.struct_specializations
            .insert(key, (mangled.clone(), new_type_id));
        self.new_structs.push(specialized);

        (mangled, new_type_id)
    }

    /// Run monomorphization on the entire HIR program.
    ///
    /// - Generic templates are removed from the item list
    /// - Concrete specializations are generated and appended
    /// - Call sites are rewritten to target the mangled names
    pub fn monomorphize(&mut self, program: &mut HirProgram) {
        // Phase 1: Extract generic templates from the program
        let mut concrete_items = Vec::new();
        for item in program.items.drain(..) {
            match item {
                HirItem::Function(ref f) if !f.type_params.is_empty() => {
                    self.generic_functions.insert(f.name.clone(), f.clone());
                }
                HirItem::Struct(ref s) if !s.type_params.is_empty() => {
                    self.generic_structs.insert(s.name.clone(), s.clone());
                }
                other => concrete_items.push(other),
            }
        }
        program.items = concrete_items;

        // Phase 2: Walk all concrete function bodies and rewrite calls
        for item in &mut program.items {
            if let HirItem::Function(f) = item {
                self.process_function_body(f);
            }
        }

        // Phase 3: Iteratively process newly generated specializations.
        // A specialized function may itself call another generic function
        // (e.g., wrap<T> calls identity<T>), so we must keep processing
        // until no new specializations are created.
        loop {
            let batch: Vec<HirFunction> = self.new_functions.drain(..).collect();
            if batch.is_empty() {
                break;
            }
            let mut processed_batch = Vec::with_capacity(batch.len());
            for mut func in batch {
                self.process_function_body(&mut func);
                processed_batch.push(func);
            }
            // Append processed functions to the program
            for func in processed_batch {
                program.items.push(HirItem::Function(func));
            }
        }

        // Phase 4: Append any remaining structs
        for strct in self.new_structs.drain(..) {
            program.items.push(HirItem::Struct(strct));
        }
    }

    /// Walk a function body, find calls to generic functions, and rewrite them.
    fn process_function_body(&mut self, func: &mut HirFunction) {
        // Reset binding scope and seed with the function's parameters so Local
        // references to params resolve to the (possibly substituted) param types.
        self.binding_types.clear();
        for param in &func.params {
            if let Some(tid) = param.type_id {
                self.binding_types.insert(param.name.clone(), tid);
            }
        }
        for stmt in &mut func.body {
            self.process_stmt(stmt);
        }
    }

    /// Process a statement, looking for calls to generic functions.
    fn process_stmt(&mut self, stmt: &mut HirStmt) {
        match &mut stmt.kind {
            HirStmtKind::Expr(expr) => {
                self.process_expr(expr);
            }
            HirStmtKind::Let {
                name,
                value,
                type_id,
                ..
            } => {
                self.process_expr(value);
                // Inherit the (possibly concretized) type from the value so the
                // Let binding's recorded type matches what RHS produced.
                // BUT: only overwrite if the value type is known (not Any).
                // Any is a sentinel for "unknown type" — the type annotation
                // (if present) should take priority.
                if let Some(value_tid) = value.type_id {
                    if value_tid != doo_core::types::builtin::ANY {
                        *type_id = Some(value_tid);
                    }
                }
                if let Some(tid) = *type_id {
                    self.binding_types.insert(name.clone(), tid);
                }
            }
            HirStmtKind::TupleLet {
                names,
                value,
                type_ids,
                ..
            } => {
                self.process_expr(value);
                // If the value's type_id is a concrete Tuple, propagate per-element
                // types into the individual bindings — purely via registry lookup,
                // no hardcoded shapes.
                if let Some(value_tid) = value.type_id {
                    if let Some(info) = self.registry.get(value_tid) {
                        if let TypeKind::Tuple { elements } = &info.kind {
                            for (i, elem) in elements.iter().enumerate() {
                                if let Some(slot) = type_ids.get_mut(i) {
                                    *slot = Some(*elem);
                                }
                                if let Some(name) = names.get(i) {
                                    self.binding_types.insert(name.clone(), *elem);
                                }
                            }
                        }
                    }
                }
            }
            HirStmtKind::Assign { target, value, .. } => {
                self.process_expr(target);
                self.process_expr(value);
            }
            HirStmtKind::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                self.process_expr(condition);
                for s in then_block {
                    self.process_stmt(s);
                }
                if let Some(else_stmts) = else_block {
                    for s in else_stmts {
                        self.process_stmt(s);
                    }
                }
            }
            HirStmtKind::While {
                condition,
                body,
                increment,
            } => {
                self.process_expr(condition);
                for s in body {
                    self.process_stmt(s);
                }
                for s in increment {
                    self.process_stmt(s);
                }
            }
            HirStmtKind::Return(exprs) => {
                for e in exprs {
                    self.process_expr(e);
                }
            }
            HirStmtKind::ManualErrorExtract { expr, .. } => {
                self.process_expr(expr);
            }
            HirStmtKind::Break | HirStmtKind::Continue | HirStmtKind::Drop { .. } => {}
        }
    }

    /// Process an expression. If it's a call to a generic function, rewrite it.
    fn process_expr(&mut self, expr: &mut HirExpr) {
        match &mut expr.kind {
            HirExprKind::Call { func, args } => {
                // Process arguments first (they may themselves contain generic calls)
                for arg in args.iter_mut() {
                    self.process_expr(arg);
                }
                self.process_expr(func);

                // Check if func is a call to a generic function
                if let HirExprKind::Local { name } = &func.kind {
                    if let Some(generic_func) = self.generic_functions.get(name).cloned() {
                        // Infer concrete types from argument types
                        if let Some(concrete_types) =
                            self.infer_types_from_args(&generic_func, args)
                        {
                            let mangled = self.get_or_create_specialization(
                                &generic_func,
                                &concrete_types,
                            );
                            // Rewrite the call target to the mangled name
                            *func = Box::new(HirExpr {
                                kind: HirExprKind::Local { name: mangled },
                                span: func.span,
                                type_id: func.type_id,
                            });

                            // Concretize the call expression's result type. Type
                            // inference may have left it as a composite containing
                            // TypeParams (e.g. Tuple<TypeParam(A), TypeParam(B)>);
                            // deep-substitute it so downstream codegen sees the
                            // real element types and lays out structs correctly.
                            let type_map = self.build_type_map(&generic_func, &concrete_types);
                            if let Some(tid) = expr.type_id {
                                expr.type_id = Some(self.substitute_type_id(tid, &type_map));
                            }
                            for arg in args.iter_mut() {
                                if let Some(tid) = arg.type_id {
                                    arg.type_id = Some(self.substitute_type_id(tid, &type_map));
                                }
                            }
                        }
                    }
                }
            }
            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.process_expr(lhs);
                self.process_expr(rhs);
            }
            HirExprKind::UnaryOp { operand, .. } => {
                self.process_expr(operand);
            }
            HirExprKind::MethodCall { receiver, args, .. } => {
                self.process_expr(receiver);
                for arg in args {
                    self.process_expr(arg);
                }
            }
            HirExprKind::Field { object, field } => {
                self.process_expr(object);
                // After the receiver is concretized, refresh this field expr's
                // type_id from the concrete struct definition. Otherwise it stays
                // pinned to the generic-struct's TypeParam-typed field.
                if let Some(obj_tid) = object.type_id {
                    if let Some(info) = self.registry.get(obj_tid) {
                        if let TypeKind::Struct { fields, .. } = &info.kind {
                            if let Some((_, ftid, _)) =
                                fields.iter().find(|(fname, _, _)| fname == field)
                            {
                                expr.type_id = Some(*ftid);
                            }
                        }
                    }
                }
            }
            HirExprKind::Index { object, index, .. } => {
                self.process_expr(object);
                self.process_expr(index);
            }
            HirExprKind::Array(elements) => {
                for el in elements {
                    self.process_expr(el);
                }
            }
            HirExprKind::Map(entries) => {
                for (k, v) in entries {
                    self.process_expr(k);
                    self.process_expr(v);
                }
            }
            HirExprKind::Tuple(elements) => {
                for el in elements {
                    self.process_expr(el);
                }
            }
            HirExprKind::Struct { name, fields } => {
                // Process child values first so their type_ids are concretized.
                for (_, val) in fields.iter_mut() {
                    self.process_expr(val);
                }

                // If this is a generic struct, infer the type-param substitution
                // from the field values' types and rewrite to a specialized name.
                if let Some(generic_struct) = self.generic_structs.get(name).cloned() {
                    if let Some(concrete_types) =
                        self.infer_types_from_struct_fields(&generic_struct, fields)
                    {
                        let (mangled, new_type_id) = self
                            .get_or_create_struct_specialization(
                                &generic_struct,
                                &concrete_types,
                            );
                        *name = mangled;
                        expr.type_id = Some(new_type_id);
                    }
                }
            }
            HirExprKind::EnumVariant { payload, .. } => {
                for p in payload {
                    self.process_expr(p);
                }
            }
            HirExprKind::Spread(inner) => {
                self.process_expr(inner);
            }
            HirExprKind::If {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                self.process_expr(condition);
                self.process_expr(then_expr);
                if let Some(el) = else_expr {
                    self.process_expr(el);
                }
            }
            HirExprKind::Block { stmts, expr, .. } => {
                for s in stmts {
                    self.process_stmt(s);
                }
                if let Some(e) = expr {
                    self.process_expr(e);
                }
            }
            HirExprKind::Match { values, arms, .. } => {
                for v in values {
                    self.process_expr(v);
                }
                for arm in arms {
                    if let Some(guard) = &mut arm.guard {
                        self.process_expr(guard);
                    }
                    self.process_expr(&mut arm.body);
                }
            }
            HirExprKind::Range { start, end, .. } => {
                self.process_expr(start);
                self.process_expr(end);
            }
            HirExprKind::Closure { body, .. } => {
                self.process_expr(body);
            }
            HirExprKind::Ok(inner)
            | HirExprKind::Err(inner)
            | HirExprKind::Try(inner)
            | HirExprKind::Move(inner)
            | HirExprKind::Clone(inner) => {
                self.process_expr(inner);
            }
            HirExprKind::UnwrapOrPanic { expr: inner, message } => {
                self.process_expr(inner);
                self.process_expr(message);
            }
            HirExprKind::Borrow { expr: inner, .. } => {
                self.process_expr(inner);
            }
            HirExprKind::Cast { value, .. } => {
                self.process_expr(value);
            }
            HirExprKind::Await(inner) | HirExprKind::Spawn { body: inner } => {
                self.process_expr(inner);
            }
            HirExprKind::ScopeBlock { stmts } => {
                for s in stmts {
                    self.process_stmt(s);
                }
            }
            HirExprKind::RouteBlock { routes } => {
                for r in routes {
                    self.process_expr(r);
                }
            }
            // For Local references, refresh the type_id from the current
            // binding scope. Type inference may have stamped the Local with
            // an abstract (TypeParam-laden) type that became concrete after
            // the binding's RHS was processed.
            HirExprKind::Local { name } => {
                if let Some(&tid) = self.binding_types.get(name) {
                    expr.type_id = Some(tid);
                }
            }
            // Literals and globals — nothing to process here.
            HirExprKind::Const(_) | HirExprKind::Global { .. } => {}
        }
    }

    /// Infer concrete type substitutions from the arguments at a call site.
    ///
    /// For `fn identity<T>(x: T) -> T` called as `identity(42)`:
    ///   - x has type_id = TypeParam("T")
    ///   - arg[0] has type_id = Some(INT)
    ///   → T maps to INT
    ///
    /// Handles composite param types via `unify_type_param`, so e.g. `items: [T]`
    /// matched against `[Int]` infers T=Int.
    fn infer_types_from_args(
        &self,
        generic_func: &HirFunction,
        args: &[HirExpr],
    ) -> Option<Vec<TypeId>> {
        let type_params = &generic_func.type_params;
        if type_params.is_empty() {
            return None;
        }

        let mut substitution: FxHashMap<String, TypeId> = FxHashMap::default();

        for (param, arg) in generic_func.params.iter().zip(args.iter()) {
            if let (Some(param_tid), Some(arg_tid)) = (param.type_id, arg.type_id) {
                self.unify_type_param(param_tid, arg_tid, &mut substitution);
            }
        }

        let mut result = Vec::with_capacity(type_params.len());
        for tp in type_params {
            if let Some(&concrete) = substitution.get(tp) {
                result.push(concrete);
            } else {
                result.push(doo_core::types::builtin::ANY);
            }
        }

        Some(result)
    }

    /// Get or create a concrete specialization of a generic function.
    /// Returns the mangled name of the specialization.
    fn get_or_create_specialization(
        &mut self,
        generic_func: &HirFunction,
        concrete_types: &[TypeId],
    ) -> String {
        let key = (generic_func.name.clone(), concrete_types.to_vec());
        if let Some(mangled) = self.specializations.get(&key) {
            return mangled.clone();
        }

        // Generate mangled name: funcName__Type1_Type2
        let type_names: Vec<String> = concrete_types
            .iter()
            .map(|tid| self.registry.display_name(*tid))
            .collect();
        let mangled = format!("{}__{}", generic_func.name, type_names.join("_"));

        let type_map = self.build_type_map(generic_func, concrete_types);

        // Clone the generic function and substitute types
        let mut specialized = generic_func.clone();
        specialized.name = mangled.clone();
        specialized.type_params = vec![]; // No longer generic

        // Substitute param types (deeply, so composites with TypeParams resolve too)
        for param in &mut specialized.params {
            if let Some(ref mut tid) = param.type_id {
                *tid = self.substitute_type_id(*tid, &type_map);
            }
        }

        // Substitute return type
        if let Some(ref mut ret) = specialized.return_type {
            *ret = self.substitute_type_id(*ret, &type_map);
        }

        // Substitute error type
        if let Some(ref mut err) = specialized.error_type {
            *err = self.substitute_type_id(*err, &type_map);
        }

        // Substitute types in the body
        self.substitute_stmts(&mut specialized.body, &type_map);

        // Cache and store
        self.specializations.insert(key, mangled.clone());
        self.new_functions.push(specialized);

        mangled
    }

    /// Substitute TypeParam TypeIds with concrete TypeIds in a list of statements.
    fn substitute_stmts(&mut self, stmts: &mut [HirStmt], type_map: &FxHashMap<TypeId, TypeId>) {
        for stmt in stmts {
            self.substitute_stmt(stmt, type_map);
        }
    }

    fn substitute_stmt(&mut self, stmt: &mut HirStmt, type_map: &FxHashMap<TypeId, TypeId>) {
        match &mut stmt.kind {
            HirStmtKind::Let {
                value, type_id, ..
            } => {
                if let Some(ref mut tid) = type_id {
                    *tid = self.substitute_type_id(*tid, type_map);
                }
                self.substitute_expr(value, type_map);
            }
            HirStmtKind::TupleLet {
                value, type_ids, ..
            } => {
                for tid in type_ids.iter_mut().flatten() {
                    *tid = self.substitute_type_id(*tid, type_map);
                }
                self.substitute_expr(value, type_map);
            }
            HirStmtKind::Expr(expr) => {
                self.substitute_expr(expr, type_map);
            }
            HirStmtKind::Assign { target, value, .. } => {
                self.substitute_expr(target, type_map);
                self.substitute_expr(value, type_map);
            }
            HirStmtKind::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                self.substitute_expr(condition, type_map);
                self.substitute_stmts(then_block, type_map);
                if let Some(else_stmts) = else_block {
                    self.substitute_stmts(else_stmts, type_map);
                }
            }
            HirStmtKind::While {
                condition,
                body,
                increment,
            } => {
                self.substitute_expr(condition, type_map);
                self.substitute_stmts(body, type_map);
                self.substitute_stmts(increment, type_map);
            }
            HirStmtKind::Return(exprs) => {
                for e in exprs {
                    self.substitute_expr(e, type_map);
                }
            }
            HirStmtKind::ManualErrorExtract { expr, .. } => {
                self.substitute_expr(expr, type_map);
            }
            HirStmtKind::Break | HirStmtKind::Continue | HirStmtKind::Drop { .. } => {}
        }
    }

    fn substitute_expr(&mut self, expr: &mut HirExpr, type_map: &FxHashMap<TypeId, TypeId>) {
        // Substitute the expr's own type_id (deep — handles composites)
        if let Some(ref mut tid) = expr.type_id {
            *tid = self.substitute_type_id(*tid, type_map);
        }

        match &mut expr.kind {
            HirExprKind::Call { func, args } => {
                self.substitute_expr(func, type_map);
                for arg in args {
                    self.substitute_expr(arg, type_map);
                }
            }
            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.substitute_expr(lhs, type_map);
                self.substitute_expr(rhs, type_map);
            }
            HirExprKind::UnaryOp { operand, .. } => {
                self.substitute_expr(operand, type_map);
            }
            HirExprKind::MethodCall { receiver, args, .. } => {
                self.substitute_expr(receiver, type_map);
                for arg in args {
                    self.substitute_expr(arg, type_map);
                }
            }
            HirExprKind::Field { object, .. } => {
                self.substitute_expr(object, type_map);
            }
            HirExprKind::Index { object, index, .. } => {
                self.substitute_expr(object, type_map);
                self.substitute_expr(index, type_map);
            }
            HirExprKind::Array(elements) => {
                for el in elements {
                    self.substitute_expr(el, type_map);
                }
            }
            HirExprKind::Map(entries) => {
                for (k, v) in entries {
                    self.substitute_expr(k, type_map);
                    self.substitute_expr(v, type_map);
                }
            }
            HirExprKind::Tuple(elements) => {
                for el in elements {
                    self.substitute_expr(el, type_map);
                }
            }
            HirExprKind::Struct { fields, .. } => {
                for (_, val) in fields {
                    self.substitute_expr(val, type_map);
                }
            }
            HirExprKind::EnumVariant { payload, .. } => {
                for p in payload {
                    self.substitute_expr(p, type_map);
                }
            }
            HirExprKind::Spread(inner) => {
                self.substitute_expr(inner, type_map);
            }
            HirExprKind::If {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                self.substitute_expr(condition, type_map);
                self.substitute_expr(then_expr, type_map);
                if let Some(el) = else_expr {
                    self.substitute_expr(el, type_map);
                }
            }
            HirExprKind::Block { stmts, expr, .. } => {
                self.substitute_stmts(stmts, type_map);
                if let Some(e) = expr {
                    self.substitute_expr(e, type_map);
                }
            }
            HirExprKind::Match { values, arms, .. } => {
                for v in values {
                    self.substitute_expr(v, type_map);
                }
                for arm in arms {
                    if let Some(guard) = &mut arm.guard {
                        self.substitute_expr(guard, type_map);
                    }
                    self.substitute_expr(&mut arm.body, type_map);
                }
            }
            HirExprKind::Range { start, end, .. } => {
                self.substitute_expr(start, type_map);
                self.substitute_expr(end, type_map);
            }
            HirExprKind::Closure { body, .. } => {
                self.substitute_expr(body, type_map);
            }
            HirExprKind::Ok(inner)
            | HirExprKind::Err(inner)
            | HirExprKind::Try(inner)
            | HirExprKind::Move(inner)
            | HirExprKind::Clone(inner) => {
                self.substitute_expr(inner, type_map);
            }
            HirExprKind::UnwrapOrPanic { expr: inner, message } => {
                self.substitute_expr(inner, type_map);
                self.substitute_expr(message, type_map);
            }
            HirExprKind::Borrow { expr: inner, .. } => {
                self.substitute_expr(inner, type_map);
            }
            HirExprKind::Cast { value, .. } => {
                self.substitute_expr(value, type_map);
            }
            HirExprKind::Await(inner) | HirExprKind::Spawn { body: inner } => {
                self.substitute_expr(inner, type_map);
            }
            HirExprKind::ScopeBlock { stmts } => {
                self.substitute_stmts(stmts, type_map);
            }
            HirExprKind::RouteBlock { routes } => {
                for r in routes {
                    self.substitute_expr(r, type_map);
                }
            }
            // Literals, locals, globals — no types to substitute
            HirExprKind::Const(_) | HirExprKind::Local { .. } | HirExprKind::Global { .. } => {}
        }
    }
}
