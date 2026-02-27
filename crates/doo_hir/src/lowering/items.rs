//! Item lowering: functions, structs, enums, imports, decorators.

use doo_core::{
    doo_debug,
    types::{TypeKind, TypeRegistry},
};
use doo_frontend::ast::{
    self, Decorator, EnumDecl, FunctionDecl,
    ImportDecl, Item, StructDecl,
};
use crate::types::*;
use super::Lower;

impl Lower {
    pub(crate) fn lower_item(&mut self, item: &Item) -> Option<HirItem> {
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

    pub(crate) fn lower_item_typed(&mut self, item: &Item, registry: &mut TypeRegistry) -> Option<HirItem> {
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
            fields,
            decorators,
            span: s.span,
        }
    }

    pub(crate) fn lower_struct_typed(&mut self, s: &StructDecl, registry: &mut TypeRegistry) -> HirStruct {
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

    pub(crate) fn lower_enum(&mut self, e: &EnumDecl) -> HirEnum {
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

    pub(crate) fn lower_enum_typed(&mut self, e: &EnumDecl, registry: &mut TypeRegistry) -> HirEnum {
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
}
