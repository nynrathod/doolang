//! Item lowering: functions, structs, enums, imports, decorators, consts.

use super::Lower;
use crate::types::*;
use doo_core::types::{builtin, TypeId, TypeKind, TypeRegistry};
use doo_frontend::ast::{
    self, Decorator, EnumDecl, FunctionDecl, ImportDecl, InterfaceDecl, Item, StaticDecl,
    StructDecl,
};

impl Lower {
    pub(crate) fn lower_item(&mut self, item: &Item) -> Option<HirItem> {
        match item {
            Item::Const(c) => Some(HirItem::Const(self.lower_const(c))),
            Item::Static(s) => Some(HirItem::Static(self.lower_static(s))),
            Item::Function(f) => Some(HirItem::Function(self.lower_function(f))),
            Item::Struct(s) => Some(HirItem::Struct(self.lower_struct(s))),
            Item::Enum(e) => Some(HirItem::Enum(self.lower_enum(e))),
            Item::Interface(i) => Some(HirItem::Interface(self.lower_interface(i))),
            Item::Import(i) => Some(HirItem::Import(self.lower_import(i))),
            Item::Policy(p) => Some(HirItem::Policy(self.lower_policy(p))),
            Item::Impl(impl_decl) => {
                for method in &impl_decl.methods {
                    let hir_func = self.lower_function(method);
                    self.hoisted_items.push(HirItem::Function(hir_func));
                }
                None
            }
            Item::Statement(_stmt) => None,
        }
    }

    pub(crate) fn lower_item_typed(
        &mut self,
        item: &Item,
        registry: &mut TypeRegistry,
    ) -> Option<HirItem> {
        match item {
            Item::Const(c) => Some(HirItem::Const(self.lower_const_typed(c, registry))),
            Item::Static(s) => Some(HirItem::Static(self.lower_static_typed(s, registry))),
            Item::Function(f) => Some(HirItem::Function(self.lower_function_typed(f, registry))),
            Item::Struct(s) => Some(HirItem::Struct(self.lower_struct_typed(s, registry))),
            Item::Enum(e) => Some(HirItem::Enum(self.lower_enum_typed(e, registry))),
            Item::Interface(i) => Some(HirItem::Interface(self.lower_interface_typed(i, registry))),
            Item::Import(i) => Some(HirItem::Import(self.lower_import(i))),
            Item::Policy(p) => Some(HirItem::Policy(self.lower_policy(p))),
            Item::Impl(impl_decl) => {
                for method in &impl_decl.methods {
                    let hir_func = self.lower_function_typed(method, registry);
                    self.hoisted_items.push(HirItem::Function(hir_func));
                }
                None
            }
            Item::Statement(_stmt) => None,
        }
    }

    /// Lower a const declaration without type resolution.
    pub(crate) fn lower_const(&mut self, c: &ast::ConstDecl) -> HirConst {
        let value_expr = self.lower_expr(&c.value);
        let prim = extract_const_value(&value_expr);
        let type_id = prim.as_ref().map(|v| v.type_id()).unwrap_or(builtin::ANY);
        HirConst {
            name: c.name.clone(),
            is_public: c.is_public,
            value: prim,
            value_expr,
            type_id,
            span: c.span,
        }
    }

    /// Lower a const declaration with full type resolution.
    pub(crate) fn lower_const_typed(
        &mut self,
        c: &ast::ConstDecl,
        registry: &mut TypeRegistry,
    ) -> HirConst {
        let value_expr = self.lower_expr_typed(&c.value, registry);
        let prim = extract_const_value(&value_expr);
        let type_id = value_expr
            .type_id
            .unwrap_or_else(|| prim.as_ref().map(|v| v.type_id()).unwrap_or(builtin::ANY));
        HirConst {
            name: c.name.clone(),
            is_public: c.is_public,
            value: prim,
            value_expr,
            type_id,
            span: c.span,
        }
    }

    /// Lower a static declaration without type resolution.
    pub(crate) fn lower_static(&mut self, s: &StaticDecl) -> HirStatic {
        HirStatic {
            name: s.name.clone(),
            is_public: s.is_public,
            type_id: None, // Resolved later
            span: s.span,
        }
    }

    /// Lower a static declaration with full type resolution.
    pub(crate) fn lower_static_typed(
        &mut self,
        s: &StaticDecl,
        registry: &mut TypeRegistry,
    ) -> HirStatic {
        let type_id = self.resolve_type_expr(&s.type_expr, registry);
        HirStatic {
            name: s.name.clone(),
            is_public: s.is_public,
            type_id: Some(type_id),
            span: s.span,
        }
    }

    pub(crate) fn lower_function(&mut self, f: &FunctionDecl) -> HirFunction {
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
            type_params: f.type_params.iter().map(|tp| tp.name.clone()).collect(),
            params,
            return_type: None,
            error_type: None,
            body,
            decorators,
            is_async: f.is_async,
            span: f.span,
        }
    }

    pub(crate) fn lower_function_typed(
        &mut self,
        f: &FunctionDecl,
        registry: &mut TypeRegistry,
    ) -> HirFunction {
        for (i, stmt) in f.body.iter().enumerate() {}
        // Clear variable types for new function scope
        self.var_types.clear();

        // Register type parameters as placeholders in the registry
        for tp in &f.type_params {
            registry.register_type_param(&tp.name);
        }
        let type_param_names: Vec<String> =
            f.type_params.iter().map(|tp| tp.name.clone()).collect();

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
            type_params: type_param_names,
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
            is_async: f.is_async,
            span: f.span,
        }
    }

    pub(crate) fn lower_struct(&mut self, s: &StructDecl) -> HirStruct {
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
            type_params: s.type_params.iter().map(|tp| tp.name.clone()).collect(),
            fields,
            decorators,
            span: s.span,
        }
    }

    pub(crate) fn lower_struct_typed(
        &mut self,
        s: &StructDecl,
        registry: &mut TypeRegistry,
    ) -> HirStruct {
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

        // Build field_json_names from @json("key") decorators on fields
        let mut field_json_names: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for f in &fields {
            for d in &f.decorators {
                if d.name == "json" {
                    if let Some(arg) = d.args.first() {
                        if let HirExprKind::Const(ConstValue::Str(json_name)) = &arg.kind {
                            field_json_names.insert(f.name.clone(), json_name.clone());
                        }
                    }
                }
            }
        }

        registry.define_struct(
            &s.name,
            fields
                .iter()
                .filter_map(|f| f.type_id.map(|id| (f.name.clone(), id, f.is_public)))
                .collect(),
            field_json_names,
        );

        let decorators = s
            .decorators
            .iter()
            .map(|d| self.lower_decorator(d))
            .collect();

        HirStruct {
            name: s.name.clone(),
            type_params: s.type_params.iter().map(|tp| tp.name.clone()).collect(),
            fields,
            decorators,
            span: s.span,
        }
    }

    pub(crate) fn lower_enum(&mut self, e: &EnumDecl) -> HirEnum {
        let variants = e
            .variants
            .iter()
            .map(|v| HirVariant {
                name: v.name.clone(),
                payload: None,
                decorators: v
                    .decorators
                    .iter()
                    .map(|d| self.lower_decorator(d))
                    .collect(),
                span: v.span,
            })
            .collect();

        HirEnum {
            name: e.name.clone(),
            variants,
            span: e.span,
        }
    }

    pub(crate) fn lower_enum_typed(
        &mut self,
        e: &EnumDecl,
        registry: &mut TypeRegistry,
    ) -> HirEnum {
        let variants: Vec<HirVariant> = e
            .variants
            .iter()
            .map(|v| HirVariant {
                name: v.name.clone(),
                payload: v
                    .payload
                    .as_ref()
                    .map(|t| self.resolve_type_expr(t, registry)),
                decorators: v
                    .decorators
                    .iter()
                    .map(|d| self.lower_decorator(d))
                    .collect(),
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

    pub(crate) fn lower_interface(&mut self, i: &InterfaceDecl) -> HirInterface {
        let methods = i
            .methods
            .iter()
            .map(|m| HirInterfaceMethod {
                name: m.name.clone(),
                params: m
                    .params
                    .iter()
                    .map(|(name, _type_ann)| HirParam {
                        name: name.clone(),
                        type_id: None,
                        span: m.span,
                    })
                    .collect(),
                return_type: None,
                error_type: None,
                span: m.span,
            })
            .collect();

        HirInterface {
            name: i.name.clone(),
            methods,
            span: i.span,
        }
    }

    pub(crate) fn lower_interface_typed(
        &mut self,
        i: &InterfaceDecl,
        registry: &mut TypeRegistry,
    ) -> HirInterface {
        let methods: Vec<HirInterfaceMethod> = i
            .methods
            .iter()
            .map(|m| {
                let params: Vec<HirParam> = m
                    .params
                    .iter()
                    .map(|(name, type_ann)| {
                        let type_id = type_ann
                            .as_ref()
                            .map(|t| self.resolve_type_expr(t, registry));
                        HirParam {
                            name: name.clone(),
                            type_id,
                            span: m.span,
                        }
                    })
                    .collect();

                HirInterfaceMethod {
                    name: m.name.clone(),
                    params,
                    return_type: m
                        .return_type
                        .as_ref()
                        .map(|t| self.resolve_type_expr(t, registry)),
                    error_type: m
                        .error_type
                        .as_ref()
                        .map(|t| self.resolve_type_expr(t, registry)),
                    span: m.span,
                }
            })
            .collect();

        // Register the interface type in the registry
        let method_sigs: Vec<(String, Vec<TypeId>, Option<TypeId>, Option<TypeId>)> = methods
            .iter()
            .map(|m| {
                (
                    m.name.clone(),
                    m.params.iter().filter_map(|p| p.type_id).collect(),
                    m.return_type,
                    m.error_type,
                )
            })
            .collect();
        registry.define_interface(&i.name, method_sigs);

        HirInterface {
            name: i.name.clone(),
            methods,
            span: i.span,
        }
    }

    pub(crate) fn lower_import(&mut self, i: &ImportDecl) -> HirImport {
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

    pub(crate) fn lower_decorator(&mut self, d: &Decorator) -> HirDecorator {
        HirDecorator {
            name: d.name.clone(),
            args: d.args.iter().map(|e| self.lower_expr(e)).collect(),
            span: d.span,
        }
    }

    /// Lower a `PolicyDecl` to `HirPolicy`.
    pub(crate) fn lower_policy(&mut self, p: &doo_frontend::ast::PolicyDecl) -> HirPolicy {
        HirPolicy {
            name: p.name.clone(),
            for_struct: p.for_struct.clone(),
            rules: p.rules.clone(),
            span: p.span,
        }
    }
}

/// Extract a primitive `ConstValue` from a lowered HIR expression, if possible.
/// Returns `None` for complex types (arrays, maps) — those are handled as inline expressions.
fn extract_const_value(expr: &HirExpr) -> Option<ConstValue> {
    match &expr.kind {
        HirExprKind::Const(cv) => Some(cv.clone()),
        HirExprKind::UnaryOp {
            op: HirUnaryOp::Neg,
            operand,
        } => match &operand.kind {
            HirExprKind::Const(ConstValue::Int(v)) => Some(ConstValue::Int(-v)),
            HirExprKind::Const(ConstValue::Float(v)) => Some(ConstValue::Float(-v)),
            _ => None,
        },
        _ => None,
    }
}
