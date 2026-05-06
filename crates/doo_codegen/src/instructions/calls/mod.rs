//! Call Instruction Handler
//!
//! Handles: Call, MethodCall, FfiCall, Print
//!
//! Split into sub-modules:
//! - call_utils: operand conversion, type coercion, result struct loading
//! - call_print: print value/array/map/struct/enum codegen
//! - call_ffi: FFI signatures, declarations, and call emission
//! - call_wrappers: HTTP and WebSocket handler wrapper generation
//! - call_metadata: metadata registration and error helpers

pub(crate) mod call_ffi;
pub(crate) mod call_metadata;
mod call_print;
pub(crate) mod call_utils;
pub(crate) mod call_wrappers;

use call_ffi::emit_ffi_call;
use call_print::{
    emit_print_array, emit_print_enum, emit_print_enum_value, emit_print_map, emit_print_struct,
    emit_print_value,
};
use call_utils::{coerce_arg_to_param_type, load_result_struct, operand_to_value, value_to_ptr};

use super::InstructionHandler;
use crate::builtins::{ArrayBuiltins, JsonBuiltins, MapBuiltins, StringBuiltins};
use crate::context::CodegenContext;
use doo_core::constants::ffi_names;
use doo_core::doo_debug;
use doo_core::types::builtin;
use doo_core::types::TypeKind;
use doo_mir::sym::resolve;
use doo_mir::{MirInstr, MirInstrKind, MirOperand};
use inkwell::module::Linkage;
use inkwell::types::{BasicType, BasicTypeEnum, BasicMetadataTypeEnum};
use inkwell::values::BasicValueEnum;
use inkwell::IntPredicate;
/// Route context for handler wrapper generation.
/// Provides information about the route pattern and middleware to determine
/// how to extract handler parameters from the request.
#[derive(Debug, Clone, Default)]
pub(crate) struct RouteContext {
    /// Route path pattern (e.g., "/api/user/:authorId/posts")
    pub route_path: Option<String>,
    /// Middleware names (e.g., "jwt", "auth")
    pub middleware_names: Vec<String>,
    /// HTTP method (e.g., "GET", "POST")
    pub http_method: Option<String>,
}

impl RouteContext {
    /// Extract path parameter names from route pattern.
    /// E.g., "/api/user/:authorId/posts" -> ["authorId"]
    /// Handles both :param and {param} styles.
    pub fn path_param_names(&self) -> Vec<String> {
        let Some(path) = &self.route_path else {
            return vec![];
        };
        let mut params = Vec::new();
        for segment in path.split('/') {
            if segment.starts_with(':') {
                params.push(segment[1..].to_string());
            } else if segment.starts_with('{') && segment.ends_with('}') {
                params.push(segment[1..segment.len() - 1].to_string());
            }
        }
        params
    }

    /// Check if a parameter name matches a path parameter.
    pub fn is_path_param(&self, param_name: &str) -> bool {
        let path_params = self.path_param_names();
        path_params.iter().any(|p| {
            // Case-insensitive match (authorId == AuthorId == authorid)
            p.eq_ignore_ascii_case(param_name)
        })
    }

    /// Check if handler uses JWT middleware.
    pub fn has_jwt_middleware(&self) -> bool {
        self.middleware_names
            .iter()
            .any(|m| crate::packages::http::is_auth_middleware(m))
    }

    /// Determine the source field index in DooRequest for a given parameter.
    /// DooRequest layout: { *method(0), *path(1), *body(2), *headers(3), *params(4), *query(5), *user_id(6) }
    pub fn param_source_index(
        &self,
        param_name: &str,
        _param_idx: usize,
        total_params: usize,
    ) -> u32 {
        // Special case: JWT user ID injection
        // If middleware is jwt and param name suggests user ID, use user_id field
        if self.has_jwt_middleware() {
            let lower = param_name.to_lowercase();
            if lower == "userid" || lower == "user_id" || lower == "id" && total_params == 1 {
                return 6; // user_id
            }
        }

        // If param name matches a path parameter, use params field
        if self.is_path_param(param_name) {
            return 4; // params (path params as JSON)
        }

        // For GET requests, use query or params
        if self.http_method.as_deref() == Some("GET") {
            // Single param on GET usually comes from path or query params
            // The FFI layer populates body with merged params for GET
            return 2; // body (which FFI populates with merged query/path params for GET)
        }

        // Default: use body for POST/PUT/PATCH
        2 // body
    }
}

/// Call/invocation instruction handler.
pub struct CallHandler;

impl<'ctx> InstructionHandler<'ctx> for CallHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(
            instr.kind,
            MirInstrKind::Call { .. }
                | MirInstrKind::MethodCall { .. }
                | MirInstrKind::FfiCall { .. }
                | MirInstrKind::Print { .. }
                | MirInstrKind::TypeOf { .. }
                | MirInstrKind::InterfaceConstruct { .. }
                | MirInstrKind::WrapOk { .. }
                | MirInstrKind::WrapErr { .. }
                | MirInstrKind::IsOk { .. }
                | MirInstrKind::UnwrapOk { .. }
                | MirInstrKind::UnwrapErr { .. }
                | MirInstrKind::ManualErrorExtract { .. }
        )
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            MirInstrKind::Call { dest, func, args } => {
                let func_s = resolve(*func);
                // Handle __black_box builtin
                if func_s == "__black_box" {
                    if let Some(arg) = args.first() {
                        if let Some(val) = operand_to_value(ctx, arg) {
                            let result = crate::builtins::emit_black_box(ctx, val);
                            if let (Some(r), Some(dst)) = (result, dest) {
                                ctx.set_temp(&resolve(*dst), r);
                            }
                            return result;
                        }
                    }
                    return None;
                }

                let func_val = match ctx.get_function(&func_s) {
                    Some(f) => f,
                    None => {
                        doo_debug!(
                            "CODEGEN",
                            "warning: undefined function '{}' — call silently dropped",
                            func_s
                        );
                        return None;
                    }
                };

                // Coerce arguments to match function parameter types
                // This handles cases like enum StructValues that need to be boxed to pointers
                let param_types = func_val.get_type().get_param_types();
                let arg_vals: Vec<_> = args
                    .iter()
                    .enumerate()
                    .filter_map(|(i, a)| {
                        let val = match operand_to_value(ctx, a) {
                            Some(v) => v,
                            None => {
                                doo_debug!(
                                    "CODEGEN",
                                    "WARNING: Call to '{}' — arg {} ({:?}) resolved to None, dropping",
                                    func_s, i, a
                                );
                                return None;
                            }
                        };
                        // Get expected parameter type from function signature
                        // Convert BasicMetadataTypeEnum to BasicTypeEnum if possible
                        let param_type: Option<BasicTypeEnum> =
                            param_types.get(i).and_then(|t| match t {
                                inkwell::types::BasicMetadataTypeEnum::ArrayType(t) => {
                                    Some((*t).into())
                                }
                                inkwell::types::BasicMetadataTypeEnum::FloatType(t) => {
                                    Some((*t).into())
                                }
                                inkwell::types::BasicMetadataTypeEnum::IntType(t) => {
                                    Some((*t).into())
                                }
                                inkwell::types::BasicMetadataTypeEnum::PointerType(t) => {
                                    Some((*t).into())
                                }
                                inkwell::types::BasicMetadataTypeEnum::StructType(t) => {
                                    Some((*t).into())
                                }
                                inkwell::types::BasicMetadataTypeEnum::VectorType(t) => {
                                    Some((*t).into())
                                }
                                inkwell::types::BasicMetadataTypeEnum::ScalableVectorType(t) => {
                                    Some((*t).into())
                                }
                                inkwell::types::BasicMetadataTypeEnum::MetadataType(_) => None,
                            });
                        Some(coerce_arg_to_param_type(ctx, val, param_type))
                    })
                    .collect();

                let call_site = ctx.builder.build_call(func_val, &arg_vals, "call").ok()?;

                if let Some(dest_name) = dest {
                    if let Some(ret_val) = call_site.try_as_basic_value().basic() {
                        let dest_s = resolve(*dest_name);
                        ctx.set_temp(&dest_s, ret_val);
                        // CRITICAL: Set variable type and struct type from function return type
                        // This enables FieldGet to work on return values (e.g., CreateUser().Email)
                        if let Some(rt) = ctx.get_function_return_type(&func_s) {
                            ctx.set_variable_type(&dest_s, rt);
                            if let Some(struct_name) = ctx.get_struct_name_from_type_id(rt) {
                                ctx.set_temp_struct_type(&dest_s, &struct_name);
                            }
                        }
                        return Some(ret_val);
                    }
                }
                None
            }

            MirInstrKind::MethodCall {
                dest,
                receiver,
                receiver_type,
                method,
                args,
                arg_types,
                return_type,
            } => {
                let method_s = resolve(*method);
                let dest_s = dest.as_ref().map(|d| resolve(*d));
                if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                    doo_debug!(
                        "CODEGEN",
                        "MethodCall: {:?}.{} -> {:?}, return_type={:?}",
                        receiver,
                        method_s,
                        dest,
                        return_type
                    );
                }
                // Intercept JSON.stringify and JSON.parse (Static Specialization)
                // Check for both Local("JSON") and Global("JSON") for module calls
                let is_json_module = matches!(receiver,
                    MirOperand::Local(name) | MirOperand::Global(name) if resolve(*name) == ffi_names::MODULE_JSON);

                if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok()
                    && method_s == "parse"
                {
                    doo_debug!(
                        "CODEGEN",
                        "JSON.parse check: is_json_module={}, receiver={:?}",
                        is_json_module,
                        receiver
                    );
                }

                if is_json_module {
                    if method_s == "stringify" {
                        if let (Some(arg_op), Some(&arg_type)) = (args.first(), arg_types.first()) {
                            if let Some(val) = operand_to_value(ctx, arg_op) {
                                // Dispatch to JSON codegen
                                let result = JsonBuiltins::emit_stringify(ctx, val, arg_type);
                                if let (Some(r), Some(dst)) = (result, dest) {
                                    ctx.set_temp(&resolve(*dst), r);
                                }
                                return result;
                            }
                        }
                        return None;
                    } else if method_s == "parse" {
                        if let Some(arg_op) = args.first() {
                            if let Some(val) = operand_to_value(ctx, arg_op) {
                                // Pass return_type to emit_parse for type-specific parsing
                                let result = JsonBuiltins::emit_parse(ctx, val, *return_type);
                                if let (Some(r), Some(dst)) = (result, dest) {
                                    ctx.set_temp(&resolve(*dst), r);
                                }
                                return result;
                            }
                        }
                        return None;
                    }
                }

                let recv_val = operand_to_value(ctx, receiver);
                if recv_val.is_none()
                    && std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok()
                {
                    doo_debug!(
                        "CODEGEN",
                        "MethodCall: failed to get receiver value for {:?}",
                        receiver
                    );
                    return None;
                }
                let recv_val = recv_val?;

                let arg_vals: Vec<_> = args
                    .iter()
                    .filter_map(|a| operand_to_value(ctx, a))
                    .collect();

                let receiver_name_owned = match receiver {
                    MirOperand::Local(name) | MirOperand::Temp(name) => Some(resolve(*name)),
                    _ => None,
                };
                let receiver_name = receiver_name_owned.as_deref();

                if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                    doo_debug!(
                        "CODEGEN",
                        "MethodCall: recv_val type: is_pointer={}, is_int={}, recv_val={:?}",
                        recv_val.is_pointer_value(),
                        recv_val.is_int_value(),
                        recv_val
                    );
                }

                // Builtin dispatch (single source of truth via TypeRegistry)
                if recv_val.is_pointer_value() {
                    let recv_ptr = recv_val.into_pointer_value();
                    if let Some(kind) = ctx.get_type_kind(*receiver_type) {
                        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                            doo_debug!(
                                "CODEGEN",
                                "MethodCall: receiver type {:?} -> kind {:?}",
                                receiver_type,
                                kind
                            );
                        }
                        let builtin_result = match kind {
                            TypeKind::Str => StringBuiltins::dispatch(
                                ctx,
                                dest_s.as_deref(),
                                recv_ptr,
                                &method_s,
                                &arg_vals,
                            ),
                            TypeKind::Array { .. } => ArrayBuiltins::dispatch(
                                ctx,
                                dest_s.as_deref(),
                                receiver_name,
                                *receiver_type,
                                recv_ptr,
                                &method_s,
                                &arg_vals,
                            ),
                            TypeKind::Map { .. } => MapBuiltins::dispatch(
                                ctx,
                                dest_s.as_deref(),
                                receiver_name,
                                *receiver_type,
                                recv_ptr,
                                &method_s,
                                &arg_vals,
                            ),
                            // For ANY type, try array builtins for common methods
                            TypeKind::Any => {
                                if matches!(
                                    method_s.as_str(),
                                    "len"
                                        | "push"
                                        | "pop"
                                        | "get"
                                        | "set"
                                        | "contains"
                                        | "slice"
                                        | "map"
                                        | "filter"
                                ) {
                                    ArrayBuiltins::dispatch(
                                        ctx,
                                        dest_s.as_deref(),
                                        receiver_name,
                                        *receiver_type,
                                        recv_ptr,
                                        &method_s,
                                        &arg_vals,
                                    )
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        };
                        if builtin_result.is_some() {
                            return builtin_result;
                        }
                    } else {
                        // Fallback for unknown type
                        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                            doo_debug!("CODEGEN", "MethodCall: fallback to array dispatch for {} (receiver_type: {:?})", method_s, receiver_type);
                        }
                        if matches!(
                            method_s.as_str(),
                            "len" | "push" | "pop" | "get" | "set" | "contains" | "slice"
                        ) {
                            let result = ArrayBuiltins::dispatch(
                                ctx,
                                dest_s.as_deref(),
                                receiver_name,
                                *receiver_type,
                                recv_ptr,
                                &method_s,
                                &arg_vals,
                            );
                            if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                                doo_debug!(
                                    "CODEGEN",
                                    "MethodCall: array dispatch result: {:?}",
                                    result.is_some()
                                );
                            }
                            if result.is_some() {
                                return result;
                            }
                        }
                    }
                }

                // ================================================================
                // Interface dispatch: vtable-based indirect call
                // ================================================================
                if let Some(TypeKind::Interface { name: iface_name, methods: iface_methods }) = ctx.get_type_kind(*receiver_type) {
                    // Find the method index in the interface
                    let method_idx = iface_methods.iter().position(|(m, _, _, _)| m == &method_s);
                    if let Some(idx) = method_idx {
                        // Extract data_ptr and vtable_ptr from the fat pointer struct
                        // Re-get recv_val to avoid move issues with extract_value
                        let recv_val2 = operand_to_value(ctx, receiver)?;
                        let fat_struct = recv_val2.into_struct_value();
                        let data_ptr = ctx.builder.build_extract_value(fat_struct, 0, "iface_data_ptr")
                            .ok()?.into_pointer_value();
                        let vtable_ptr = ctx.builder.build_extract_value(fat_struct, 1, "iface_vtable_ptr")
                            .ok()?.into_pointer_value();

                        // vtable is an array of function pointers stored as ptr*
                        // Load the function pointer at index `idx`
                        let fn_ptr_ptr = unsafe {
                            ctx.builder.build_in_bounds_gep(
                                ctx.ptr_type(),
                                vtable_ptr,
                                &[ctx.i64_type().const_int(idx as u64, false)],
                                "vtable_entry_ptr",
                            ).ok()?
                        };
                        let fn_ptr = ctx.builder.build_load(ctx.ptr_type(), fn_ptr_ptr, "vtable_fn_ptr")
                            .ok()?.into_pointer_value();

                        // Build the function type for the indirect call from interface method signature
                        let (_, ref param_type_ids, ref ret_type_id, ref err_type_id) = iface_methods[idx];
                        let has_error = err_type_id.is_some();

                        // Build parameter types: first param is always ptr (self/data_ptr)
                        let mut param_llvm_types: Vec<BasicMetadataTypeEnum> = vec![ctx.ptr_type().into()];
                        for pt in param_type_ids {
                            let lt = ctx.get_llvm_type(*pt);
                            param_llvm_types.push(lt.into());
                        }

                        // Build return type
                        let fn_type = if has_error {
                            // Result return: { i64, ptr }
                            let result_type = ctx.context.struct_type(
                                &[ctx.i64_type().into(), ctx.ptr_type().into()], false
                            );
                            result_type.fn_type(&param_llvm_types, false)
                        } else if let Some(rt) = ret_type_id {
                            let ret_llvm = ctx.get_llvm_type(*rt);
                            match ret_llvm {
                                BasicTypeEnum::IntType(t) => t.fn_type(&param_llvm_types, false),
                                BasicTypeEnum::FloatType(t) => t.fn_type(&param_llvm_types, false),
                                BasicTypeEnum::PointerType(t) => t.fn_type(&param_llvm_types, false),
                                BasicTypeEnum::StructType(t) => t.fn_type(&param_llvm_types, false),
                                BasicTypeEnum::ArrayType(t) => t.fn_type(&param_llvm_types, false),
                                BasicTypeEnum::VectorType(t) => t.fn_type(&param_llvm_types, false),
                                _ => ctx.ptr_type().fn_type(&param_llvm_types, false),
                            }
                        } else {
                            ctx.context.void_type().fn_type(&param_llvm_types, false)
                        };

                        // Build args: data_ptr as self, then method args
                        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = vec![data_ptr.into()];
                        for v in &arg_vals {
                            call_args.push((*v).into());
                        }

                        let call_site = ctx.builder.build_indirect_call(
                            fn_type, fn_ptr, &call_args, "iface_call"
                        ).ok()?;

                        if let Some(dest_name) = dest {
                            if let Some(ret_val) = call_site.try_as_basic_value().basic() {
                                let dest_s2 = resolve(*dest_name);
                                ctx.set_temp(&dest_s2, ret_val);
                                if let Some(rt) = return_type {
                                    ctx.set_variable_type(&dest_s2, *rt);
                                }
                                return Some(ret_val);
                            }
                        }
                        return None;
                    } else {
                        doo_debug!("CODEGEN", "Interface {} has no method {}", iface_name, method_s);
                        return None;
                    }
                }

                // Fallback: lookup method function, prepend receiver to args
                // Format: _method_{TypeName}_{MethodName}
                let type_name = if let Some(kind) = ctx.get_type_kind(*receiver_type) {
                    match kind {
                        TypeKind::Struct { name, .. } => Some(name),
                        TypeKind::Enum { name, .. } => Some(name),
                        TypeKind::TypeRef { name } => {
                            // Resolve TypeRef to its underlying struct/enum name
                            if let Some(resolved_tid) = ctx.type_registry.lookup(&name) {
                                if let Some(resolved_kind) = ctx.get_type_kind(resolved_tid) {
                                    match resolved_kind {
                                        TypeKind::Struct { name: n, .. } => Some(n),
                                        TypeKind::Enum { name: n, .. } => Some(n),
                                        _ => Some(name),
                                    }
                                } else {
                                    Some(name)
                                }
                            } else {
                                Some(name)
                            }
                        }
                        _ => None,
                    }
                } else {
                    None
                };

                if let Some(tname) = type_name {
                    let method_name = format!("_method_{}_{}", tname, method_s);
                    if let Some(func_val) = ctx.get_function(&method_name) {
                        // CRITICAL FIX: Apply type coercion to method call arguments
                        // Get parameter types from function signature for proper coercion
                        let param_types = func_val.get_type().get_param_types();

                        // Receiver is always first param (self), args follow
                        let mut all_args: Vec<inkwell::values::BasicMetadataValueEnum> =
                            Vec::with_capacity(1 + arg_vals.len());

                        // Coerce receiver (self)
                        let recv_param_type: Option<BasicTypeEnum> =
                            param_types.get(0).and_then(|t| match t {
                                inkwell::types::BasicMetadataTypeEnum::ArrayType(t) => {
                                    Some((*t).into())
                                }
                                inkwell::types::BasicMetadataTypeEnum::FloatType(t) => {
                                    Some((*t).into())
                                }
                                inkwell::types::BasicMetadataTypeEnum::IntType(t) => {
                                    Some((*t).into())
                                }
                                inkwell::types::BasicMetadataTypeEnum::PointerType(t) => {
                                    Some((*t).into())
                                }
                                inkwell::types::BasicMetadataTypeEnum::StructType(t) => {
                                    Some((*t).into())
                                }
                                inkwell::types::BasicMetadataTypeEnum::VectorType(t) => {
                                    Some((*t).into())
                                }
                                inkwell::types::BasicMetadataTypeEnum::ScalableVectorType(t) => {
                                    Some((*t).into())
                                }
                                inkwell::types::BasicMetadataTypeEnum::MetadataType(_) => None,
                            });
                        all_args.push(coerce_arg_to_param_type(ctx, recv_val, recv_param_type));

                        // Coerce remaining args
                        for (i, v) in arg_vals.iter().enumerate() {
                            let param_type: Option<BasicTypeEnum> =
                                param_types.get(i + 1).and_then(|t| match t {
                                    inkwell::types::BasicMetadataTypeEnum::ArrayType(t) => {
                                        Some((*t).into())
                                    }
                                    inkwell::types::BasicMetadataTypeEnum::FloatType(t) => {
                                        Some((*t).into())
                                    }
                                    inkwell::types::BasicMetadataTypeEnum::IntType(t) => {
                                        Some((*t).into())
                                    }
                                    inkwell::types::BasicMetadataTypeEnum::PointerType(t) => {
                                        Some((*t).into())
                                    }
                                    inkwell::types::BasicMetadataTypeEnum::StructType(t) => {
                                        Some((*t).into())
                                    }
                                    inkwell::types::BasicMetadataTypeEnum::VectorType(t) => {
                                        Some((*t).into())
                                    }
                                    inkwell::types::BasicMetadataTypeEnum::ScalableVectorType(
                                        t,
                                    ) => Some((*t).into()),
                                    inkwell::types::BasicMetadataTypeEnum::MetadataType(_) => None,
                                });
                            all_args.push(coerce_arg_to_param_type(ctx, *v, param_type));
                        }

                        let call_site =
                            ctx.builder.build_call(func_val, &all_args, "mcall").ok()?;

                        if let Some(dest_name) = dest {
                            if let Some(ret_val) = call_site.try_as_basic_value().basic() {
                                let dest_s2 = resolve(*dest_name);
                                ctx.set_temp(&dest_s2, ret_val);
                                // CRITICAL: Set variable type from return_type for proper type tracking
                                // This enables field access on method return values (e.g., dir.list()[0].Name)
                                if let Some(rt) = return_type {
                                    ctx.set_variable_type(&dest_s2, *rt);
                                    // If return type is a struct, also set temp_struct_type
                                    if let Some(struct_name) = ctx.get_struct_name_from_type_id(*rt)
                                    {
                                        ctx.set_temp_struct_type(&dest_s2, &struct_name);
                                    }
                                }
                                return Some(ret_val);
                            }
                        }
                        return None; // Void return
                    }
                }
                None
            }

            MirInstrKind::FfiCall {
                dest,
                lib: _,
                symbol,
                args,
            } => {
                // FFI call: declare external function if needed and call
                // Symbol is the C function name (e.g., "doo_file_read", "doo_http_server_new")
                let dest_ffi = dest.as_ref().map(|d| resolve(*d));
                let symbol_s = resolve(*symbol);
                emit_ffi_call(ctx, dest_ffi.as_deref(), &symbol_s, args)
            }

            MirInstrKind::Print {
                values,
                value_types,
                separator,
            } => {
                // Print built-in: call printf or custom print function
                // Declare printf if not already declared
                let printf = ctx.get_function(ffi_names::PRINTF).unwrap_or_else(|| {
                    let i32_ty = ctx.context.i32_type();
                    let ptr_ty = ctx.context.ptr_type(inkwell::AddressSpace::default());
                    let fn_ty = i32_ty.fn_type(&[ptr_ty.into()], true); // variadic
                    ctx.module.add_function(ffi_names::PRINTF, fn_ty, None)
                });

                let debug = std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok();

                for (i, val) in values.iter().enumerate() {
                    let mut ty = value_types
                        .get(i)
                        .copied()
                        .unwrap_or(doo_core::types::builtin::ANY);
                    let is_last = i + 1 == values.len();

                    // Get operand name for array_element_types lookup
                    let operand_name_owned = match val {
                        MirOperand::Temp(name) | MirOperand::Local(name) => Some(resolve(*name)),
                        _ => None,
                    };
                    let operand_name = operand_name_owned.as_deref();

                    // If MIR type is ANY, try to get actual type from variable tracking
                    // This handles cases where MIR type inference failed but codegen tracked the type
                    if ty == builtin::ANY {
                        if let Some(name) = operand_name {
                            if let Some(var_type) = ctx.get_variable_type(name) {
                                ty = var_type;
                            }
                        }
                    }

                    if let Some(v) = operand_to_value(ctx, val) {
                        if debug {
                            let type_kind = ctx.get_type_kind(ty);
                            let blk = ctx
                                .builder
                                .get_insert_block()
                                .map(|b| b.get_name().to_string_lossy().to_string());
                            doo_debug!(
                                "CODEGEN",
                                "Print value {}: {:?} type={:?} kind={:?} llvm_type={:?} in block {:?}",
                                i,
                                val,
                                ty,
                                type_kind,
                                v.get_type(),
                                blk
                            );
                        }

                        // Check array_element_types first for accurate element type
                        // This handles results from map/filter/slice which track element types
                        if let Some(name) = operand_name {
                            if let Some(&elem_type) = ctx.array_element_types.get(name) {
                                if v.is_pointer_value() {
                                    emit_print_array(
                                        ctx,
                                        printf,
                                        v.into_pointer_value(),
                                        elem_type,
                                    );
                                    if !is_last && *separator {
                                        let fmt = ctx.const_string("%s");
                                        let space = ctx.const_string(" ");
                                        ctx.builder
                                            .build_call(
                                                printf,
                                                &[fmt.into(), space.into()],
                                                "print_space",
                                            )
                                            .ok();
                                    }
                                    continue;
                                }
                            }
                        }

                        if let Some(kind) = ctx.get_type_kind(ty) {
                            match kind {
                                TypeKind::Str => {
                                    emit_print_value(ctx, printf, ty, v, false, false);
                                }
                                TypeKind::Bool => {
                                    emit_print_value(ctx, printf, ty, v, false, false);
                                }
                                TypeKind::Int | TypeKind::Float => {
                                    emit_print_value(ctx, printf, ty, v, false, false);
                                }
                                TypeKind::Array { element } => {
                                    if v.is_pointer_value() {
                                        emit_print_array(
                                            ctx,
                                            printf,
                                            v.into_pointer_value(),
                                            element,
                                        );
                                    } else {
                                        emit_print_value(
                                            ctx,
                                            printf,
                                            builtin::ANY,
                                            v,
                                            false,
                                            false,
                                        );
                                    }
                                }
                                TypeKind::Map { key, value } => {
                                    if v.is_pointer_value() {
                                        emit_print_map(
                                            ctx,
                                            printf,
                                            v.into_pointer_value(),
                                            key,
                                            value,
                                        );
                                    } else {
                                        emit_print_value(
                                            ctx,
                                            printf,
                                            builtin::ANY,
                                            v,
                                            false,
                                            false,
                                        );
                                    }
                                }
                                TypeKind::Struct { name, fields } => {
                                    if v.is_pointer_value() {
                                        // Extract just name and type for printing (visibility not needed)
                                        let field_pairs: Vec<_> = fields
                                            .iter()
                                            .map(|(n, t, _)| (n.clone(), *t))
                                            .collect();
                                        emit_print_struct(
                                            ctx,
                                            printf,
                                            v.into_pointer_value(),
                                            &name,
                                            &field_pairs,
                                        );
                                    } else {
                                        emit_print_value(ctx, printf, ty, v, false, false);
                                    }
                                }
                                TypeKind::Enum { name, variants } => {
                                    if v.is_pointer_value() {
                                        emit_print_enum(
                                            ctx,
                                            printf,
                                            v.into_pointer_value(),
                                            &name,
                                            &variants,
                                        );
                                    } else if v.is_struct_value() {
                                        // Enum as StructValue (inline) - use direct extraction
                                        emit_print_enum_value(
                                            ctx,
                                            printf,
                                            v.into_struct_value(),
                                            &name,
                                            &variants,
                                        );
                                    } else {
                                        // Fallback for other cases
                                        emit_print_value(ctx, printf, ty, v, false, false);
                                    }
                                }
                                _ => {
                                    emit_print_value(ctx, printf, ty, v, false, false);
                                }
                            }
                        } else {
                            emit_print_value(ctx, printf, ty, v, false, false);
                        }

                        if !is_last && *separator {
                            let fmt = ctx.const_string("%s");
                            let space = ctx.const_string(" ");
                            ctx.builder
                                .build_call(printf, &[fmt.into(), space.into()], "print_space")
                                .ok();
                        }
                    } else if debug {
                        doo_debug!(
                            "CODEGEN",
                            "WARNING: Print value {} operand_to_value returned None for {:?}",
                            i,
                            val
                        );
                    }
                }

                // Single newline at the end of the print call
                let fmt = ctx.const_string("%s");
                let nl = ctx.const_string("\n");
                ctx.builder
                    .build_call(printf, &[fmt.into(), nl.into()], "print_nl")
                    .ok();

                None
            }

            MirInstrKind::WrapOk { dest, value } => {
                // Result::Ok = { i64 tag=0, ptr payload }
                // Using ptr for payload preserves pointer provenance through LLVM O3
                // Allocate Result struct, set tag=0, box value in payload
                let val = operand_to_value(ctx, value)?;

                // Convert value to pointer representation
                let value_ptr = value_to_ptr(ctx, val)?;

                // Create Result struct type: { i64 tag, ptr payload }
                let result_struct_type = ctx
                    .context
                    .struct_type(&[ctx.i64_type().into(), ctx.ptr_type().into()], false);

                // Allocate Result struct on stack
                let result_alloca = ctx.alloca_in_entry_block(result_struct_type, "result_ok")?;

                // Set tag = 0 (Ok)
                let tag_ptr = ctx
                    .builder
                    .build_struct_gep(result_struct_type, result_alloca, 0, "ok_tag_ptr")
                    .ok()?;
                ctx.builder
                    .build_store(tag_ptr, ctx.i64_type().const_int(0, false))
                    .ok()?;

                // Store pointer directly as payload (preserves provenance)
                let payload_ptr = ctx
                    .builder
                    .build_struct_gep(result_struct_type, result_alloca, 1, "ok_payload_ptr")
                    .ok()?;
                ctx.builder.build_store(payload_ptr, value_ptr).ok()?;

                // Load and return the struct
                let result_struct = ctx
                    .builder
                    .build_load(result_struct_type, result_alloca, "result_ok_struct")
                    .ok()?;

                ctx.set_temp(&resolve(*dest), result_struct);
                Some(result_struct)
            }

            MirInstrKind::WrapErr { dest, value } => {
                // Result::Err = { i64 tag=1, ptr payload }
                // Using ptr for payload preserves pointer provenance through LLVM O3
                // Allocate Result struct, set tag=1, box error in payload
                let val = operand_to_value(ctx, value)?;

                // Convert value to pointer representation
                let value_ptr = value_to_ptr(ctx, val)?;

                // Create Result struct type: { i64 tag, ptr payload }
                let result_struct_type = ctx
                    .context
                    .struct_type(&[ctx.i64_type().into(), ctx.ptr_type().into()], false);

                // Allocate Result struct on stack
                let result_alloca = ctx.alloca_in_entry_block(result_struct_type, "result_err")?;

                // Set tag = 1 (Err)
                let tag_ptr = ctx
                    .builder
                    .build_struct_gep(result_struct_type, result_alloca, 0, "err_tag_ptr")
                    .ok()?;
                ctx.builder
                    .build_store(tag_ptr, ctx.i64_type().const_int(1, false))
                    .ok()?;

                // Store pointer directly as payload (preserves provenance)
                let payload_ptr = ctx
                    .builder
                    .build_struct_gep(result_struct_type, result_alloca, 1, "err_payload_ptr")
                    .ok()?;
                ctx.builder.build_store(payload_ptr, value_ptr).ok()?;

                // Load and return the struct
                let result_struct = ctx
                    .builder
                    .build_load(result_struct_type, result_alloca, "result_err_struct")
                    .ok()?;

                ctx.set_temp(&resolve(*dest), result_struct);
                Some(result_struct)
            }

            MirInstrKind::IsOk { dest, value } => {
                // Check if result is Ok (tag == 0)
                let result_val = operand_to_value(ctx, value)?;

                // Try to get the Result struct (load if pointer)
                if let Some(result_struct) = load_result_struct(ctx, result_val) {
                    // If we loaded from a heap pointer, free the outer DooResult shell
                    // and update the temp map so UnwrapOk/UnwrapErr see a struct value
                    // (not the freed pointer). This prevents 16-byte leaks per FFI call.
                    if result_val.is_pointer_value() && !result_val.is_struct_value() {
                        // Free the heap-allocated DooResult outer shell
                        let doo_free_fn =
                            ctx.get_function(ffi_names::DOO_FREE).unwrap_or_else(|| {
                                let free_type = ctx
                                    .context
                                    .void_type()
                                    .fn_type(&[ctx.ptr_type().into()], false);
                                ctx.module
                                    .add_function(ffi_names::DOO_FREE, free_type, None)
                            });
                        let _ = ctx.builder.build_call(
                            doo_free_fn,
                            &[result_val.into_pointer_value().into()],
                            "free_result_shell",
                        );

                        // Update the operand's value in the temp map to the loaded struct
                        // so that subsequent UnwrapOk/UnwrapErr reads the struct directly
                        // instead of the now-freed pointer
                        let operand_name_owned2 = match value {
                            MirOperand::Local(n) | MirOperand::Temp(n) | MirOperand::Global(n) => {
                                Some(resolve(*n))
                            }
                            _ => None,
                        };
                        if let Some(ref name) = operand_name_owned2 {
                            ctx.set_temp(name, result_struct.into());
                        }
                    }

                    // Extract tag (field 0) - i64 for ABI compatibility
                    let tag = ctx
                        .builder
                        .build_extract_value(result_struct, 0, "result_tag")
                        .ok()?
                        .into_int_value();

                    // DEBUG: Print tag value at runtime to diagnose ABI issues
                    if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                        let printf = ctx.get_function(ffi_names::PRINTF).unwrap_or_else(|| {
                            let printf_type =
                                ctx.i32_type().fn_type(&[ctx.ptr_type().into()], true);
                            ctx.module
                                .add_function(ffi_names::PRINTF, printf_type, None)
                        });
                        let fmt = ctx.const_string("[DEBUG] IsOk: tag=%lld\n");
                        let _ = ctx.builder.build_call(
                            printf,
                            &[fmt.into(), tag.into()],
                            "debug_print",
                        );
                    }

                    // Check if tag == 0 (Ok) - use i64 constant to match tag type
                    let is_ok = ctx
                        .builder
                        .build_int_compare(
                            IntPredicate::EQ,
                            tag,
                            ctx.i64_type().const_int(0, false),
                            "is_ok",
                        )
                        .ok()?;

                    ctx.set_temp(&resolve(*dest), is_ok.into());
                    Some(is_ok.into())
                } else {
                    // Not a Result type - check for Optional/nil
                    // Optional values: nil = 0/null, non-nil = has value
                    let is_ok: BasicValueEnum = if result_val.is_pointer_value() {
                        BasicValueEnum::IntValue(
                            ctx.builder
                                .build_is_not_null(result_val.into_pointer_value(), "is_not_nil")
                                .ok()?,
                        )
                    } else if result_val.is_int_value() {
                        let int_val = result_val.into_int_value();
                        BasicValueEnum::IntValue(
                            ctx.builder
                                .build_int_compare(
                                    IntPredicate::NE,
                                    int_val,
                                    int_val.get_type().const_zero(),
                                    "is_not_nil",
                                )
                                .ok()?,
                        )
                    } else {
                        // Unknown type - assume always Ok
                        BasicValueEnum::IntValue(ctx.const_bool(true))
                    };
                    ctx.set_temp(&resolve(*dest), is_ok);
                    Some(is_ok)
                }
            }

            MirInstrKind::UnwrapOk {
                dest,
                value,
                expected_type,
            } => {
                // Extract Ok value from Result
                // The branching (error check) is now handled at MIR level
                // This just extracts the value from an Ok result
                let result_val = operand_to_value(ctx, value)?;

                // Try to get the Result struct (load if pointer)
                if let Some(result_struct) = load_result_struct(ctx, result_val) {
                    // Extract value as ptr (field 1) - stored as ptr to preserve provenance
                    let value_ptr = ctx
                        .builder
                        .build_extract_value(result_struct, 1, "ok_value_ptr")
                        .ok()?
                        .into_pointer_value();

                    // Convert the pointer back to the expected type
                    // value_to_ptr heap-boxes scalars: malloc(8) + store, so we load from pointer
                    let final_value: BasicValueEnum = match expected_type {
                        Some(type_id) if *type_id == builtin::INT => {
                            // Load i64 from heap-boxed pointer
                            ctx.builder
                                .build_load(ctx.i64_type(), value_ptr, "unbox_int")
                                .ok()?
                        }
                        Some(type_id) if *type_id == builtin::FLOAT => {
                            // Load f64 from heap-boxed pointer
                            ctx.builder
                                .build_load(ctx.f64_type(), value_ptr, "unbox_float")
                                .ok()?
                        }
                        Some(type_id) if *type_id == builtin::BOOL => {
                            // Load i64 from heap-boxed pointer, then truncate to bool
                            let i64_val = ctx
                                .builder
                                .build_load(ctx.i64_type(), value_ptr, "unbox_bool_i64")
                                .ok()?
                                .into_int_value();
                            ctx.builder
                                .build_int_truncate(i64_val, ctx.bool_type(), "to_bool")
                                .ok()?
                                .into()
                        }
                        _ => {
                            // For pointers (string, array, struct), the pointer is the value
                            value_ptr.into()
                        }
                    };

                    ctx.set_temp(&resolve(*dest), final_value);
                    Some(final_value)
                } else {
                    // Not a Result type - pass through the value as-is
                    // This handles the case where ? is used on non-Result values
                    ctx.set_temp(&resolve(*dest), result_val);
                    Some(result_val)
                }
            }

            MirInstrKind::UnwrapErr { dest, value } => {
                // Extract Err value from Result
                // The MIR is responsible for checking IsOk before calling UnwrapErr,
                // so we don't need to check again here - just extract the payload.
                let result_val = operand_to_value(ctx, value)?;

                // Try to get the Result struct (load if pointer)
                if let Some(result_struct) = load_result_struct(ctx, result_val) {
                    // Extract payload as ptr (field 1) - stored as ptr to preserve provenance
                    let value_ptr = ctx
                        .builder
                        .build_extract_value(result_struct, 1, "err_value_ptr")
                        .ok()?
                        .into_pointer_value();

                    // The payload is now a pointer - store it as the value
                    ctx.set_temp(&resolve(*dest), value_ptr.into());
                    Some(value_ptr.into())
                } else {
                    // Not a Result type - return null pointer as error
                    // This shouldn't normally happen but provides fallback
                    let null_ptr = ctx.ptr_type().const_null();
                    ctx.set_temp(&resolve(*dest), null_ptr.into());
                    Some(null_ptr.into())
                }
            }

            MirInstrKind::ManualErrorExtract {
                ok_names,
                error_name,
                result,
                ok_type,
                err_type,
                is_ffi,
            } => {
                // Manual error extraction: let a, b, err = expr;
                // Result struct layout: { i32 tag, void* value }
                // tag == 0 means Ok, tag == 1 means Err

                // FFI error data is ALWAYS a raw C string (from err_str/make_err_rfc7807),
                // regardless of the declared Doo error struct type (e.g. GitError).
                // We register the struct association for potential field access, but
                // MUST override variable_type to Str so Clone/Drop treat it correctly.
                //
                // For Doo-native functions, errors ARE actual struct/primitive values,
                // so we use the declared err_type directly.
                let effective_err_type = if *is_ffi {
                    // FFI errors are always C strings - use STR for Clone/Drop
                    if matches!(ctx.get_type_kind(*err_type), Some(TypeKind::Struct { .. })) {
                        builtin::STR
                    } else {
                        *err_type
                    }
                } else {
                    *err_type
                };

                let error_name_s = resolve(*error_name);
                if error_name_s != "_" {
                    if let Some(TypeKind::Struct { name, .. }) = ctx.get_type_kind(*err_type) {
                        ctx.set_temp_struct_type(&error_name_s, &name);
                        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                            doo_debug!(
                                "CODEGEN",
                                "Registered error {} as struct type {}",
                                error_name_s,
                                name
                            );
                        }
                    }
                    ctx.set_variable_type(&error_name_s, effective_err_type);
                }

                let result_val = operand_to_value(ctx, result)?;

                // Result must be a struct value or pointer to struct
                // If it's a pointer, load the struct first
                let result_struct = if result_val.is_pointer_value() {
                    let result_ptr = result_val.into_pointer_value();
                    // Result struct: { i64 tag, ptr value } — ptr preserves provenance
                    let result_struct_type = ctx
                        .context
                        .struct_type(&[ctx.i64_type().into(), ctx.ptr_type().into()], false);
                    let loaded = ctx
                        .builder
                        .build_load(result_struct_type, result_ptr, "result_struct_load")
                        .ok()?
                        .into_struct_value();

                    // Free the heap-allocated DooResult outer shell after loading
                    let doo_free_fn = ctx.get_function(ffi_names::DOO_FREE).unwrap_or_else(|| {
                        let free_type = ctx
                            .context
                            .void_type()
                            .fn_type(&[ctx.ptr_type().into()], false);
                        ctx.module
                            .add_function(ffi_names::DOO_FREE, free_type, None)
                    });
                    let _ = ctx.builder.build_call(
                        doo_free_fn,
                        &[result_ptr.into()],
                        "free_result_shell",
                    );

                    loaded
                } else if result_val.is_struct_value() {
                    result_val.into_struct_value()
                } else {
                    // Not a Result - just assign the value to all destinations
                    for ok_name in ok_names {
                        ctx.set_temp(&resolve(*ok_name), result_val);
                    }
                    if error_name_s != "_" {
                        // Set error to nil (null pointer)
                        let nil = ctx.ptr_type().const_null();
                        ctx.set_temp(&error_name_s, nil.into());
                    }
                    return Some(result_val);
                };

                // Extract tag (field 0) - i64 for ABI compatibility
                let tag = ctx
                    .builder
                    .build_extract_value(result_struct, 0, "result_tag")
                    .ok()?
                    .into_int_value();

                // Check if tag == 0 (Ok) - use i64 constant to match tag type
                let is_ok = ctx
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        tag,
                        ctx.i64_type().const_int(0, false),
                        "is_ok",
                    )
                    .ok()?;

                // Extract value as ptr (field 1) — ptr preserves provenance
                let value_ptr = ctx
                    .builder
                    .build_extract_value(result_struct, 1, "result_value_ptr")
                    .ok()?
                    .into_pointer_value();

                // Create blocks for ok and err paths
                let func = ctx.builder.get_insert_block()?.get_parent()?;
                let ok_block = ctx.context.append_basic_block(func, "manual_ok");
                let err_block = ctx.context.append_basic_block(func, "manual_err");
                let cont_block = ctx.context.append_basic_block(func, "manual_cont");

                ctx.builder
                    .build_conditional_branch(is_ok, ok_block, err_block)
                    .ok()?;

                // Check if ok_type is a scalar (Int, Float, Bool) - these need special handling
                // because Result stores scalars as inttoptr(value) which needs ptrtoint to extract
                let is_int = *ok_type == builtin::INT;
                let is_float = *ok_type == builtin::FLOAT;
                let is_bool = *ok_type == builtin::BOOL;
                let is_scalar_ok = is_int || is_float || is_bool;
                let error_ignored = error_name_s == "_";

                if is_scalar_ok {
                    // === SCALAR VALUE PATH ===
                    // For scalar ok types (Int, Float, Bool), we MUST extract actual values
                    // because value_to_ptr heap-boxes scalars: malloc(8) + store.
                    // This applies whether error is captured or ignored.

                    let ok_llvm_type = ctx.get_llvm_type(*ok_type);

                    // === Ok path ===
                    ctx.builder.position_at_end(ok_block);

                    // Load value from heap-boxed pointer (same logic as UnwrapOk)
                    let ok_extracted_val: BasicValueEnum = if is_int {
                        // Load i64 from heap-boxed pointer
                        ctx.builder
                            .build_load(ctx.i64_type(), value_ptr, "unbox_int")
                            .ok()?
                    } else if is_float {
                        // Load f64 from heap-boxed pointer
                        ctx.builder
                            .build_load(ctx.f64_type(), value_ptr, "unbox_float")
                            .ok()?
                    } else {
                        // Bool: Load i64 from heap-boxed pointer, then truncate to bool
                        let i64_val = ctx
                            .builder
                            .build_load(ctx.i64_type(), value_ptr, "unbox_bool_i64")
                            .ok()?
                            .into_int_value();
                        ctx.builder
                            .build_int_truncate(i64_val, ctx.bool_type(), "to_bool")
                            .ok()?
                            .into()
                    };

                    let ok_block_end = ctx.builder.get_insert_block()?;
                    ctx.builder.build_unconditional_branch(cont_block).ok()?;

                    // === Err path ===
                    ctx.builder.position_at_end(err_block);
                    // Use default value for the ok type (0 for Int, 0.0 for Float, false for Bool)
                    let default_val = crate::utils::default_for_type(ctx, ok_llvm_type);
                    let err_block_end = ctx.builder.get_insert_block()?;
                    ctx.builder.build_unconditional_branch(cont_block).ok()?;

                    // === Continue block - merge with phi nodes ===
                    ctx.builder.position_at_end(cont_block);

                    // Create phi node for the ok VALUE (not pointer)
                    let ok_phi = ctx.builder.build_phi(ok_llvm_type, "ok_val_phi").ok()?;
                    ok_phi.add_incoming(&[
                        (&ok_extracted_val, ok_block_end),
                        (&default_val, err_block_end),
                    ]);
                    let ok_result = ok_phi.as_basic_value();

                    // Store ok value(s) to all ok_names
                    for ok_name in ok_names {
                        let ok_name_s = resolve(*ok_name);
                        ctx.set_temp(&ok_name_s, ok_result);
                        ctx.set_variable_type(&ok_name_s, *ok_type);
                    }

                    // Create phi node for error value (if not ignored)
                    if !error_ignored {
                        // Error is a pointer - use pointer phi
                        let err_val_from_ok = ctx.ptr_type().const_null();
                        let err_val_from_err = value_ptr;
                        let err_phi = ctx.builder.build_phi(ctx.ptr_type(), "err_phi").ok()?;
                        err_phi.add_incoming(&[
                            (&err_val_from_ok, ok_block_end),
                            (&err_val_from_err, err_block_end),
                        ]);
                        let err_result = err_phi.as_basic_value();
                        ctx.set_temp(&error_name_s, err_result);
                        // Use effective_err_type: STR for FFI struct errors, actual for Doo
                        ctx.set_variable_type(&error_name_s, effective_err_type);
                    }

                    Some(ok_result)
                } else {
                    // === POINTER PATH (original behavior) ===
                    // Used when error is NOT ignored, or ok type is not scalar

                    // === Ok path ===
                    ctx.builder.position_at_end(ok_block);

                    // For Ok path: ok values get the actual value, error gets nil
                    let ok_val_from_ok = value_ptr;
                    let err_val_from_ok = ctx.ptr_type().const_null();

                    ctx.builder.build_unconditional_branch(cont_block).ok()?;

                    // === Err path ===
                    ctx.builder.position_at_end(err_block);

                    // For Err path: ok values get nil, error gets the actual error
                    let ok_val_from_err = ctx.ptr_type().const_null();
                    let err_val_from_err = value_ptr;

                    ctx.builder.build_unconditional_branch(cont_block).ok()?;

                    // === Continue block - merge with phi nodes ===
                    ctx.builder.position_at_end(cont_block);

                    // Create phi node for ok value
                    let ok_phi = ctx.builder.build_phi(ctx.ptr_type(), "ok_phi").ok()?;
                    ok_phi.add_incoming(&[
                        (&ok_val_from_ok, ok_block),
                        (&ok_val_from_err, err_block),
                    ]);
                    let ok_result = ok_phi.as_basic_value();

                    // Store ok value(s) to all ok_names with type tracking
                    for ok_name in ok_names {
                        let ok_name_s = resolve(*ok_name);
                        ctx.set_temp(&ok_name_s, ok_result);
                        ctx.set_variable_type(&ok_name_s, *ok_type);
                    }

                    // Create phi node for error value (if not ignored)
                    if !error_ignored {
                        let err_phi = ctx.builder.build_phi(ctx.ptr_type(), "err_phi").ok()?;
                        err_phi.add_incoming(&[
                            (&err_val_from_ok, ok_block),
                            (&err_val_from_err, err_block),
                        ]);
                        let err_result = err_phi.as_basic_value();
                        ctx.set_temp(&error_name_s, err_result);
                        // Use effective_err_type: STR for FFI struct errors, actual for Doo
                        ctx.set_variable_type(&error_name_s, effective_err_type);
                    }

                    Some(ok_result)
                }
            }

            MirInstrKind::TypeOf {
                dest,
                value: _,
                value_type,
            } => {
                // Get the type name string based on the type
                let type_name: String = if let Some(kind) = ctx.get_type_kind(*value_type) {
                    match kind {
                        TypeKind::Int => "Int".to_string(),
                        TypeKind::Float => "Float".to_string(),
                        TypeKind::Bool => "Bool".to_string(),
                        TypeKind::Str => "Str".to_string(),
                        TypeKind::Void => "Nil".to_string(),
                        TypeKind::Array { .. } => "Array".to_string(),
                        TypeKind::Map { .. } => "Map".to_string(),
                        TypeKind::Tuple { .. } => "Tuple".to_string(),
                        TypeKind::Struct { name, .. } => name,
                        TypeKind::Enum { name, .. } => name,
                        TypeKind::Interface { name, .. } => name,
                        TypeKind::Function { .. } => "Function".to_string(),
                        TypeKind::Result { .. } => "Result".to_string(),
                        TypeKind::Optional { .. } => "Optional".to_string(),
                        TypeKind::Any => "Any".to_string(),
                        TypeKind::TypeRef { name } => name,
                        TypeKind::Error => "Error".to_string(),
                        TypeKind::TypeParam { name } => name,
                    }
                } else {
                    "Unknown".to_string()
                };

                let type_str = ctx.const_string(&type_name);
                ctx.set_temp(&resolve(*dest), type_str.into());
                Some(type_str.into())
            }

            MirInstrKind::InterfaceConstruct {
                dest,
                value,
                concrete_type,
                interface_type,
            } => {
                // Build interface fat pointer: { data_ptr, vtable_ptr }
                // vtable is an array of function pointers, one per interface method.
                let iface_llvm_type = ctx.get_llvm_type(*interface_type);
                let struct_type = if let BasicTypeEnum::StructType(st) = iface_llvm_type {
                    st
                } else {
                    let ptr_type = ctx.context.i8_type().ptr_type(inkwell::AddressSpace::default());
                    ctx.context.struct_type(&[ptr_type.into(), ptr_type.into()], false)
                };

                // Get the concrete struct pointer
                let data_ptr = match operand_to_value(ctx, value) {
                    Some(val) => {
                        if val.is_pointer_value() {
                            val.into_pointer_value()
                        } else if val.is_struct_value() {
                            let alloca = ctx
                                .alloca_in_entry_block(val.get_type(), "iface_box")
                                .unwrap();
                            ctx.builder.build_store(alloca, val).ok();
                            alloca
                        } else {
                            ctx.context.i8_type().ptr_type(inkwell::AddressSpace::default()).const_null()
                        }
                    }
                    None => ctx.context.i8_type().ptr_type(inkwell::AddressSpace::default()).const_null(),
                };

                let i8_ptr_type = ctx.context.i8_type().ptr_type(inkwell::AddressSpace::default());
                let data_i8_ptr = ctx.builder
                    .build_pointer_cast(data_ptr, i8_ptr_type, "iface_data")
                    .ok()
                    .unwrap_or(i8_ptr_type.const_null());

                // Build vtable: look up each interface method on the concrete type
                // and store function pointers in a global constant array.
                let concrete_name = ctx.get_type_kind(*concrete_type)
                    .and_then(|k| match k {
                        TypeKind::Struct { name, .. } => Some(name),
                        TypeKind::Enum { name, .. } => Some(name),
                        _ => None,
                    });
                let iface_methods = ctx.get_type_kind(*interface_type)
                    .and_then(|k| match k {
                        TypeKind::Interface { methods, .. } => Some(methods),
                        _ => None,
                    });

                let vtable_ptr = if let (Some(cname), Some(methods)) = (concrete_name, iface_methods) {
                    // Build array of function pointers for this concrete type
                    let mut fn_ptrs: Vec<inkwell::values::PointerValue<'ctx>> = Vec::new();
                    for (method_name, _, _, _) in &methods {
                        let mangled = format!("_method_{}_{}", cname, method_name);
                        if let Some(func) = ctx.get_function(&mangled) {
                            let fptr = func.as_global_value().as_pointer_value();
                            let cast = ctx.builder
                                .build_pointer_cast(fptr, i8_ptr_type, "vtable_fn")
                                .unwrap_or(i8_ptr_type.const_null());
                            fn_ptrs.push(cast);
                        } else {
                            doo_debug!("CODEGEN", "WARNING: vtable method {} not found for {}", mangled, cname);
                            fn_ptrs.push(i8_ptr_type.const_null());
                        }
                    }
                    // Create a global constant array for the vtable
                    let vtable_array_type = i8_ptr_type.array_type(fn_ptrs.len() as u32);
                    let vtable_const = i8_ptr_type.const_array(&fn_ptrs);
                    let vtable_name = format!("__vtable_{}_as_{}",
                        cname,
                        ctx.get_type_kind(*interface_type)
                            .and_then(|k| match k { TypeKind::Interface { name, .. } => Some(name), _ => None })
                            .unwrap_or_default()
                    );
                    // Check if vtable global already exists
                    let vtable_global = ctx.module.get_global(&vtable_name).unwrap_or_else(|| {
                        let g = ctx.module.add_global(vtable_array_type, None, &vtable_name);
                        g.set_initializer(&vtable_const);
                        g.set_constant(true);
                        g.set_linkage(Linkage::Private);
                        g
                    });
                    // Get pointer to first element of vtable array
                    let vtable_base = ctx.builder.build_pointer_cast(
                        vtable_global.as_pointer_value(), i8_ptr_type, "vtable_base"
                    ).unwrap_or(i8_ptr_type.const_null());
                    vtable_base
                } else {
                    i8_ptr_type.const_null()
                };

                // Build the fat pointer struct { data_ptr, vtable_ptr }
                let mut fat_ptr = struct_type.get_undef();
                if let Ok(s) = ctx.builder.build_insert_value(fat_ptr, data_i8_ptr, 0, "iface_data_field") {
                    fat_ptr = s.into_struct_value();
                }
                if let Ok(s) = ctx.builder.build_insert_value(fat_ptr, vtable_ptr, 1, "iface_vtable_field") {
                    fat_ptr = s.into_struct_value();
                }

                let result = BasicValueEnum::StructValue(fat_ptr);
                ctx.set_temp(&resolve(*dest), result);
                Some(result)
            }

            _ => None,
        }
    }
}
