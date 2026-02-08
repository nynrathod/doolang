//! Call Instruction Handler
//!
//! Handles: Call, MethodCall, FfiCall, Print

use super::InstructionHandler;
use crate::builtins::{ArrayBuiltins, JsonBuiltins, MapBuiltins, StringBuiltins};
use crate::context::CodegenContext;
use crate::layout::load_len_i32;
use doo_core::constants::ffi_names;
use doo_core::doo_debug;
use doo_core::types::builtin;
use doo_core::types::TypeKind;
use doo_mir::{MirConst, MirInstr, MirInstrKind, MirOperand};
use inkwell::module::Linkage;
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

/// Route context for handler wrapper generation.
/// Provides information about the route pattern and middleware to determine
/// how to extract handler parameters from the request.
#[derive(Debug, Clone, Default)]
struct RouteContext {
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
            .any(|m| ffi_names::is_auth_middleware(m))
    }

    /// Determine the source field index in DooRequest for a given parameter.
    /// DooRequest layout: { *method(0), *path(1), *body(2), *headers(3), *params(4), *query(5), *user_id(6) }
    pub fn param_source_index(
        &self,
        param_name: &str,
        param_idx: usize,
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
                // Handle __black_box builtin
                if func == "__black_box" {
                    if let Some(arg) = args.first() {
                        if let Some(val) = operand_to_value(ctx, arg) {
                            let result = crate::builtins::emit_black_box(ctx, val);
                            if let (Some(r), Some(dst)) = (result, dest) {
                                ctx.set_temp(dst, r);
                            }
                            return result;
                        }
                    }
                    return None;
                }

                let func_val = ctx.get_function(func)?;

                // Coerce arguments to match function parameter types
                // This handles cases like enum StructValues that need to be boxed to pointers
                let param_types = func_val.get_type().get_param_types();
                let arg_vals: Vec<_> = args
                    .iter()
                    .enumerate()
                    .filter_map(|(i, a)| {
                        let val = operand_to_value(ctx, a)?;
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
                        ctx.set_temp(dest_name, ret_val);
                        // CRITICAL: Set variable type and struct type from function return type
                        // This enables FieldGet to work on return values (e.g., CreateUser().Email)
                        if let Some(rt) = ctx.get_function_return_type(func) {
                            ctx.set_variable_type(dest_name, rt);
                            if let Some(struct_name) = ctx.get_struct_name_from_type_id(rt) {
                                ctx.set_temp_struct_type(dest_name, &struct_name);
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
                if std::env::var("DOO_DEBUG").is_ok() {
                    doo_debug!(
                        "CODEGEN",
                        "MethodCall: {:?}.{} -> {:?}, return_type={:?}",
                        receiver,
                        method,
                        dest,
                        return_type
                    );
                }
                // Intercept JSON.stringify and JSON.parse (Static Specialization)
                // Check for both Local("JSON") and Global("JSON") for module calls
                let is_json_module = matches!(receiver,
                    MirOperand::Local(name) | MirOperand::Global(name) if name == ffi_names::MODULE_JSON);

                if std::env::var("DOO_DEBUG").is_ok() && method == "parse" {
                    doo_debug!(
                        "CODEGEN",
                        "JSON.parse check: is_json_module={}, receiver={:?}",
                        is_json_module,
                        receiver
                    );
                }

                if is_json_module {
                    if method == "stringify" {
                        if let (Some(arg_op), Some(&arg_type)) = (args.first(), arg_types.first()) {
                            if let Some(val) = operand_to_value(ctx, arg_op) {
                                // Dispatch to JSON codegen
                                let result = JsonBuiltins::emit_stringify(ctx, val, arg_type);
                                if let (Some(r), Some(dst)) = (result, dest) {
                                    ctx.set_temp(dst, r);
                                }
                                return result;
                            }
                        }
                        return None;
                    } else if method == "parse" {
                        if let Some(arg_op) = args.first() {
                            if let Some(val) = operand_to_value(ctx, arg_op) {
                                // Pass return_type to emit_parse for type-specific parsing
                                let result = JsonBuiltins::emit_parse(ctx, val, *return_type);
                                if let (Some(r), Some(dst)) = (result, dest) {
                                    ctx.set_temp(dst, r);
                                }
                                return result;
                            }
                        }
                        return None;
                    }
                }

                let recv_val = operand_to_value(ctx, receiver);
                if recv_val.is_none() && std::env::var("DOO_DEBUG").is_ok() {
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

                let receiver_name = match receiver {
                    MirOperand::Local(name) | MirOperand::Temp(name) => Some(name.as_str()),
                    _ => None,
                };

                if std::env::var("DOO_DEBUG").is_ok() {
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
                        if std::env::var("DOO_DEBUG").is_ok() {
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
                                dest.as_deref(),
                                recv_ptr,
                                method,
                                &arg_vals,
                            ),
                            TypeKind::Array { .. } => ArrayBuiltins::dispatch(
                                ctx,
                                dest.as_deref(),
                                receiver_name,
                                *receiver_type,
                                recv_ptr,
                                method,
                                &arg_vals,
                            ),
                            TypeKind::Map { .. } => MapBuiltins::dispatch(
                                ctx,
                                dest.as_deref(),
                                receiver_name,
                                *receiver_type,
                                recv_ptr,
                                method,
                                &arg_vals,
                            ),
                            // For ANY type, try array builtins for common methods
                            TypeKind::Any => {
                                if matches!(
                                    method.as_str(),
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
                                        dest.as_deref(),
                                        receiver_name,
                                        *receiver_type,
                                        recv_ptr,
                                        method,
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
                        if std::env::var("DOO_DEBUG").is_ok() {
                            doo_debug!("CODEGEN", "MethodCall: fallback to array dispatch for {} (receiver_type: {:?})", method, receiver_type);
                        }
                        if matches!(
                            method.as_str(),
                            "len" | "push" | "pop" | "get" | "set" | "contains" | "slice"
                        ) {
                            let result = ArrayBuiltins::dispatch(
                                ctx,
                                dest.as_deref(),
                                receiver_name,
                                *receiver_type,
                                recv_ptr,
                                method,
                                &arg_vals,
                            );
                            if std::env::var("DOO_DEBUG").is_ok() {
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

                // Fallback: lookup method function, prepend receiver to args
                // Format: _method_{TypeName}_{MethodName}
                let type_name = if let Some(kind) = ctx.get_type_kind(*receiver_type) {
                    match kind {
                        TypeKind::Struct { name, .. } => Some(name),
                        TypeKind::Enum { name, .. } => Some(name),
                        _ => None,
                    }
                } else {
                    None
                };

                if let Some(tname) = type_name {
                    let method_name = format!("_method_{}_{}", tname, method);
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
                                ctx.set_temp(dest_name, ret_val);
                                // CRITICAL: Set variable type from return_type for proper type tracking
                                // This enables field access on method return values (e.g., dir.list()[0].Name)
                                if let Some(rt) = return_type {
                                    ctx.set_variable_type(dest_name, *rt);
                                    // If return type is a struct, also set temp_struct_type
                                    if let Some(struct_name) = ctx.get_struct_name_from_type_id(*rt)
                                    {
                                        ctx.set_temp_struct_type(dest_name, &struct_name);
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
                emit_ffi_call(ctx, dest.as_deref(), symbol, args)
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

                let debug = std::env::var("DOO_DEBUG").is_ok();

                for (i, val) in values.iter().enumerate() {
                    let mut ty = value_types
                        .get(i)
                        .copied()
                        .unwrap_or(doo_core::types::builtin::ANY);
                    let is_last = i + 1 == values.len();

                    // Get operand name for array_element_types lookup
                    let operand_name = match val {
                        MirOperand::Temp(name) | MirOperand::Local(name) => Some(name.as_str()),
                        _ => None,
                    };

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
                // Result::Ok = { i64 tag=0, i64 payload }
                // Using i64 for both fields for consistent ABI with FFI SimpleResult
                // Allocate Result struct, set tag=0, box value in payload
                let val = operand_to_value(ctx, value)?;

                // Convert value to pointer representation
                let value_ptr = value_to_ptr(ctx, val)?;

                // Create Result struct type: { i64 tag, i64 payload }
                let result_struct_type = ctx
                    .context
                    .struct_type(&[ctx.i64_type().into(), ctx.i64_type().into()], false);

                // Allocate Result struct on stack
                let result_alloca = ctx
                    .builder
                    .build_alloca(result_struct_type, "result_ok")
                    .ok()?;

                // Set tag = 0 (Ok)
                let tag_ptr = ctx
                    .builder
                    .build_struct_gep(result_struct_type, result_alloca, 0, "ok_tag_ptr")
                    .ok()?;
                ctx.builder
                    .build_store(tag_ptr, ctx.i64_type().const_int(0, false))
                    .ok()?;

                // Convert pointer to i64 and set payload
                let value_i64 = ctx
                    .builder
                    .build_ptr_to_int(value_ptr, ctx.i64_type(), "ptr_to_i64")
                    .ok()?;
                let payload_ptr = ctx
                    .builder
                    .build_struct_gep(result_struct_type, result_alloca, 1, "ok_payload_ptr")
                    .ok()?;
                ctx.builder.build_store(payload_ptr, value_i64).ok()?;

                // Load and return the struct
                let result_struct = ctx
                    .builder
                    .build_load(result_struct_type, result_alloca, "result_ok_struct")
                    .ok()?;

                ctx.set_temp(dest, result_struct);
                Some(result_struct)
            }

            MirInstrKind::WrapErr { dest, value } => {
                // Result::Err = { i64 tag=1, i64 payload }
                // Using i64 for both fields for consistent ABI with FFI SimpleResult
                // Allocate Result struct, set tag=1, box error in payload
                let val = operand_to_value(ctx, value)?;

                // Convert value to pointer representation
                let value_ptr = value_to_ptr(ctx, val)?;

                // Create Result struct type: { i64 tag, i64 payload }
                let result_struct_type = ctx
                    .context
                    .struct_type(&[ctx.i64_type().into(), ctx.i64_type().into()], false);

                // Allocate Result struct on stack
                let result_alloca = ctx
                    .builder
                    .build_alloca(result_struct_type, "result_err")
                    .ok()?;

                // Set tag = 1 (Err)
                let tag_ptr = ctx
                    .builder
                    .build_struct_gep(result_struct_type, result_alloca, 0, "err_tag_ptr")
                    .ok()?;
                ctx.builder
                    .build_store(tag_ptr, ctx.i64_type().const_int(1, false))
                    .ok()?;

                // Convert pointer to i64 and set payload
                let value_i64 = ctx
                    .builder
                    .build_ptr_to_int(value_ptr, ctx.i64_type(), "ptr_to_i64")
                    .ok()?;
                let payload_ptr = ctx
                    .builder
                    .build_struct_gep(result_struct_type, result_alloca, 1, "err_payload_ptr")
                    .ok()?;
                ctx.builder.build_store(payload_ptr, value_i64).ok()?;

                // Load and return the struct
                let result_struct = ctx
                    .builder
                    .build_load(result_struct_type, result_alloca, "result_err_struct")
                    .ok()?;

                ctx.set_temp(dest, result_struct);
                Some(result_struct)
            }

            MirInstrKind::IsOk { dest, value } => {
                // Check if result is Ok (tag == 0)
                let result_val = operand_to_value(ctx, value)?;

                // Try to get the Result struct (load if pointer)
                if let Some(result_struct) = load_result_struct(ctx, result_val) {
                    // Extract tag (field 0) - i64 for ABI compatibility
                    let tag = ctx
                        .builder
                        .build_extract_value(result_struct, 0, "result_tag")
                        .ok()?
                        .into_int_value();

                    // DEBUG: Print tag value at runtime to diagnose ABI issues
                    if std::env::var("DOO_DEBUG").is_ok() {
                        let printf = ctx.get_function("printf").unwrap_or_else(|| {
                            let printf_type =
                                ctx.i32_type().fn_type(&[ctx.ptr_type().into()], true);
                            ctx.module.add_function("printf", printf_type, None)
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

                    ctx.set_temp(dest, is_ok.into());
                    Some(is_ok.into())
                } else {
                    // Not a Result type - treat as always Ok
                    // This handles the case where ? is used on non-Result values
                    let is_ok = ctx.const_bool(true);
                    ctx.set_temp(dest, is_ok.into());
                    Some(is_ok.into())
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
                    // Extract value as i64 (field 1) - stored as i64 for ABI compatibility
                    let value_i64 = ctx
                        .builder
                        .build_extract_value(result_struct, 1, "ok_value_i64")
                        .ok()?
                        .into_int_value();

                    // Convert i64 back to pointer using inttoptr
                    let value_ptr = ctx
                        .builder
                        .build_int_to_ptr(value_i64, ctx.ptr_type(), "ok_value_ptr")
                        .ok()?;

                    // Convert the pointer back to the expected type
                    // The payload was created using value_to_ptr which uses inttoptr for primitives
                    let final_value: BasicValueEnum = match expected_type {
                        Some(type_id) if *type_id == builtin::INT => {
                            // Convert pointer back to i64 using ptrtoint
                            ctx.builder
                                .build_ptr_to_int(value_ptr, ctx.i64_type(), "ptr_to_int")
                                .ok()?
                                .into()
                        }
                        Some(type_id) if *type_id == builtin::FLOAT => {
                            // Convert pointer to float (reverse of value_to_ptr)
                            let i64_val = ctx
                                .builder
                                .build_ptr_to_int(value_ptr, ctx.i64_type(), "ptr_to_i64")
                                .ok()?;
                            let tmp = ctx.builder.build_alloca(ctx.i64_type(), "f_tmp").ok()?;
                            ctx.builder.build_store(tmp, i64_val).ok()?;
                            let f_ptr = ctx
                                .builder
                                .build_pointer_cast(
                                    tmp,
                                    ctx.context.ptr_type(inkwell::AddressSpace::default()),
                                    "f_ptr",
                                )
                                .ok()?;
                            ctx.builder
                                .build_load(ctx.f64_type(), f_ptr, "f_val")
                                .ok()?
                        }
                        Some(type_id) if *type_id == builtin::BOOL => {
                            // Convert pointer back to bool
                            let i64_val = ctx
                                .builder
                                .build_ptr_to_int(value_ptr, ctx.i64_type(), "ptr_to_i64")
                                .ok()?;
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

                    ctx.set_temp(dest, final_value);
                    Some(final_value)
                } else {
                    // Not a Result type - pass through the value as-is
                    // This handles the case where ? is used on non-Result values
                    ctx.set_temp(dest, result_val);
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
                    // Extract payload as i64 (field 1) - stored as i64 for ABI compatibility
                    let value_i64 = ctx
                        .builder
                        .build_extract_value(result_struct, 1, "err_value_i64")
                        .ok()?
                        .into_int_value();

                    // Convert i64 back to pointer using inttoptr
                    let value_ptr = ctx
                        .builder
                        .build_int_to_ptr(value_i64, ctx.ptr_type(), "err_value_ptr")
                        .ok()?;

                    // The payload is now a pointer - store it as the value
                    ctx.set_temp(dest, value_ptr.into());
                    Some(value_ptr.into())
                } else {
                    // Not a Result type - return null pointer as error
                    // This shouldn't normally happen but provides fallback
                    let null_ptr = ctx.ptr_type().const_null();
                    ctx.set_temp(dest, null_ptr.into());
                    Some(null_ptr.into())
                }
            }

            MirInstrKind::ManualErrorExtract {
                ok_names,
                error_name,
                result,
                ok_type,
                err_type,
            } => {
                // Manual error extraction: let a, b, err = expr;
                // Result struct layout: { i32 tag, void* value }
                // tag == 0 means Ok, tag == 1 means Err

                // IMPORTANT: Register error struct type association for field access
                // This is needed so that FieldGet on the error can resolve field names
                if error_name != "_" {
                    if let Some(TypeKind::Struct { name, .. }) = ctx.get_type_kind(*err_type) {
                        ctx.set_temp_struct_type(error_name, &name);
                        if std::env::var("DOO_DEBUG").is_ok() {
                            doo_debug!(
                                "CODEGEN",
                                "Registered error {} as struct type {}",
                                error_name,
                                name
                            );
                        }
                    }
                }

                let result_val = operand_to_value(ctx, result)?;

                // Result must be a struct value or pointer to struct
                // If it's a pointer, load the struct first
                let result_struct = if result_val.is_pointer_value() {
                    let result_ptr = result_val.into_pointer_value();
                    // Result struct: { i64 tag, i64 value } for ABI compatibility
                    let result_struct_type = ctx
                        .context
                        .struct_type(&[ctx.i64_type().into(), ctx.i64_type().into()], false);
                    ctx.builder
                        .build_load(result_struct_type, result_ptr, "result_struct_load")
                        .ok()?
                        .into_struct_value()
                } else if result_val.is_struct_value() {
                    result_val.into_struct_value()
                } else {
                    // Not a Result - just assign the value to all destinations
                    for ok_name in ok_names {
                        ctx.set_temp(ok_name, result_val);
                    }
                    if error_name != "_" {
                        // Set error to nil (null pointer)
                        let nil = ctx.ptr_type().const_null();
                        ctx.set_temp(error_name, nil.into());
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

                // Extract value as i64 (field 1) and convert to pointer
                let value_i64 = ctx
                    .builder
                    .build_extract_value(result_struct, 1, "result_value_i64")
                    .ok()?
                    .into_int_value();

                let value_ptr = ctx
                    .builder
                    .build_int_to_ptr(value_i64, ctx.ptr_type(), "result_value_ptr")
                    .ok()?;

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
                let error_ignored = error_name == "_";

                if is_scalar_ok {
                    // === SCALAR VALUE PATH ===
                    // For scalar ok types (Int, Float, Bool), we MUST extract actual values
                    // because Result stores scalars as inttoptr(value).
                    // This applies whether error is captured or ignored.

                    let ok_llvm_type = ctx.get_llvm_type(*ok_type);

                    // === Ok path ===
                    ctx.builder.position_at_end(ok_block);

                    // Convert pointer to value (same logic as UnwrapOk)
                    let ok_extracted_val: BasicValueEnum = if is_int {
                        // Convert pointer back to i64 using ptrtoint
                        ctx.builder
                            .build_ptr_to_int(value_ptr, ctx.i64_type(), "ptr_to_int")
                            .ok()?
                            .into()
                    } else if is_float {
                        // Convert pointer to float (reverse of value_to_ptr)
                        let i64_val = ctx
                            .builder
                            .build_ptr_to_int(value_ptr, ctx.i64_type(), "ptr_to_i64")
                            .ok()?;
                        let tmp = ctx.builder.build_alloca(ctx.i64_type(), "f_tmp").ok()?;
                        ctx.builder.build_store(tmp, i64_val).ok()?;
                        let f_ptr = ctx
                            .builder
                            .build_pointer_cast(
                                tmp,
                                ctx.context.ptr_type(inkwell::AddressSpace::default()),
                                "f_ptr",
                            )
                            .ok()?;
                        ctx.builder
                            .build_load(ctx.f64_type(), f_ptr, "f_val")
                            .ok()?
                    } else {
                        // Bool: Convert pointer back to bool
                        let i64_val = ctx
                            .builder
                            .build_ptr_to_int(value_ptr, ctx.i64_type(), "ptr_to_i64")
                            .ok()?;
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
                        ctx.set_temp(ok_name, ok_result);
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
                        ctx.set_temp(error_name, err_result);
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

                    // Store ok value(s) to all ok_names
                    for ok_name in ok_names {
                        ctx.set_temp(ok_name, ok_result);
                    }

                    // Create phi node for error value (if not ignored)
                    if !error_ignored {
                        let err_phi = ctx.builder.build_phi(ctx.ptr_type(), "err_phi").ok()?;
                        err_phi.add_incoming(&[
                            (&err_val_from_ok, ok_block),
                            (&err_val_from_err, err_block),
                        ]);
                        let err_result = err_phi.as_basic_value();
                        ctx.set_temp(error_name, err_result);
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
                        TypeKind::Function { .. } => "Function".to_string(),
                        TypeKind::Result { .. } => "Result".to_string(),
                        TypeKind::Optional { .. } => "Optional".to_string(),
                        TypeKind::Any => "Any".to_string(),
                        TypeKind::TypeRef { name } => name,
                        TypeKind::Error => "Error".to_string(),
                    }
                } else {
                    "Unknown".to_string()
                };

                let type_str = ctx.const_string(&type_name);
                ctx.set_temp(dest, type_str.into());
                Some(type_str.into())
            }

            _ => None,
        }
    }
}

/// Convert MirOperand to LLVM value.
fn operand_to_value<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    operand: &MirOperand,
) -> Option<BasicValueEnum<'ctx>> {
    match operand {
        MirOperand::Const(c) => Some(const_to_value(ctx, c)),
        MirOperand::Local(name) | MirOperand::Temp(name) | MirOperand::Global(name) => {
            // First try to get as a value (local variable, temp, etc.)
            if let Some(val) = ctx.get_value(name) {
                return Some(val);
            }
            // Fall back to function reference - convert function to pointer value
            // This handles cases like passing `getUserHandler` as a callback argument
            if let Some(func) = ctx.get_function(name) {
                return Some(func.as_global_value().as_pointer_value().into());
            }
            None
        }
        MirOperand::FuncRef(name) => {
            // Explicit function reference - return function as pointer value
            // Used when passing functions to FFI (e.g., app.get("/users", getUserHandler))
            if let Some(func) = ctx.get_function(name) {
                return Some(func.as_global_value().as_pointer_value().into());
            }
            None
        }
    }
}

/// Coerce an argument value to match the expected function parameter type.
///
/// This handles type mismatches between how values are produced (e.g., enum StructValues)
/// and how function parameters are declared (e.g., pointers for composite types).
fn coerce_arg_to_param_type<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
    expected_type: Option<BasicTypeEnum<'ctx>>,
) -> inkwell::values::BasicMetadataValueEnum<'ctx> {
    // If no expected type info, pass value as-is
    let Some(expected) = expected_type else {
        return val.into();
    };

    // If types already match, pass as-is
    if val.get_type() == expected {
        return val.into();
    }

    // Special case: StructValue passed where pointer is expected
    // This happens with enums: EnumCreate returns { i32, ptr } but function params expect ptr
    if val.is_struct_value() && expected.is_pointer_type() {
        // Box the struct value: allocate, store, return pointer
        let alloca = ctx.builder.build_alloca(val.get_type(), "arg_box").unwrap();
        ctx.builder.build_store(alloca, val).ok();
        return alloca.into();
    }

    // Special case: PointerValue passed where struct is expected
    // This happens when JSON.parse returns a pointer to enum but function expects struct by value
    if val.is_pointer_value() && expected.is_struct_type() {
        // Load the struct from the pointer
        let loaded = ctx
            .builder
            .build_load(expected, val.into_pointer_value(), "arg_load")
            .ok();
        if let Some(v) = loaded {
            return v.into();
        }
    }

    // CRITICAL FIX: IntValue passed where pointer is expected
    // This can happen when:
    // 1. Struct type info is lost during tuple extraction (TupleGet fallback)
    // 2. Field load defaults to i64 when struct type lookup fails
    // The value is actually a pointer stored as i64, convert it back.
    // This is a defensive measure - the proper fix is ensuring type info flows correctly.
    if val.is_int_value() && expected.is_pointer_type() {
        let int_val = val.into_int_value();
        if let Ok(ptr) =
            ctx.builder
                .build_int_to_ptr(int_val, expected.into_pointer_type(), "int_to_ptr_coerce")
        {
            return ptr.into();
        }
    }

    // Default: pass value as-is
    val.into()
}

/// Convert MirConst to LLVM value.
fn const_to_value<'ctx>(ctx: &CodegenContext<'ctx>, c: &MirConst) -> BasicValueEnum<'ctx> {
    match c {
        MirConst::Int(v) => ctx.const_i64(*v).into(),
        MirConst::Float(v) => ctx.const_f64(*v).into(),
        MirConst::Bool(v) => ctx.const_bool(*v).into(),
        MirConst::Nil => ctx.const_i64(0).into(),
        MirConst::Str(s) => ctx.const_string(s).into(),
    }
}

fn emit_print_value<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    type_id: doo_core::types::TypeId,
    val: BasicValueEnum<'ctx>,
    newline: bool,
    quote_strings: bool,
) {
    // Handle ANY type by inferring from LLVM value type
    if type_id == builtin::ANY {
        if val.is_int_value() {
            // Integer - print as number
            let fmt = if newline { "%lld\n" } else { "%lld" };
            let fmt = ctx.const_string(fmt);
            let i64v = ctx
                .builder
                .build_int_z_extend_or_bit_cast(val.into_int_value(), ctx.i64_type(), "print_i64")
                .ok();
            if let Some(i64v) = i64v {
                ctx.builder
                    .build_call(printf, &[fmt.into(), i64v.into()], "print_i")
                    .ok();
            }
            return;
        } else if val.is_float_value() {
            // Float - print as decimal
            let fmt = if newline { "%f\n" } else { "%f" };
            let fmt = ctx.const_string(fmt);
            ctx.builder
                .build_call(printf, &[fmt.into(), val.into()], "print_f")
                .ok();
            return;
        } else if val.is_pointer_value() {
            // Pointer - assume string (most common case for ANY)
            let fmt = if newline { "%s\n" } else { "%s" };
            let fmt = ctx.const_string(fmt);
            ctx.builder
                .build_call(printf, &[fmt.into(), val.into()], "print_str")
                .ok();
            return;
        }
        // Fallthrough to generic handling
    }

    if type_id == builtin::STR {
        if val.is_pointer_value() {
            if quote_strings {
                // Print string with surrounding quotes for collection display
                let open_quote = ctx.const_string("\"");
                let close_quote = if newline {
                    ctx.const_string("\"\n")
                } else {
                    ctx.const_string("\"")
                };
                let fmt = ctx.const_string("%s");
                ctx.builder
                    .build_call(printf, &[fmt.into(), open_quote.into()], "print_quote_open")
                    .ok();
                ctx.builder
                    .build_call(printf, &[fmt.into(), val.into()], "print_str")
                    .ok();
                ctx.builder
                    .build_call(
                        printf,
                        &[fmt.into(), close_quote.into()],
                        "print_quote_close",
                    )
                    .ok();
            } else {
                let fmt = if newline { "%s\n" } else { "%s" };
                let fmt = ctx.const_string(fmt);
                ctx.builder
                    .build_call(printf, &[fmt.into(), val.into()], "print_str")
                    .ok();
            }
        }
        return;
    }

    if type_id == builtin::BOOL {
        if val.is_int_value() {
            let v = val.into_int_value();
            let is_true = ctx
                .builder
                .build_int_compare(IntPredicate::NE, v, v.get_type().const_zero(), "is_true")
                .ok();
            if let Some(is_true) = is_true {
                let true_s = ctx.const_string(if newline { "true\n" } else { "true" });
                let false_s = ctx.const_string(if newline { "false\n" } else { "false" });
                let out = ctx
                    .builder
                    .build_select(is_true, true_s, false_s, "bool_s")
                    .ok();
                if let Some(out) = out {
                    let fmt = ctx.const_string("%s");
                    ctx.builder
                        .build_call(printf, &[fmt.into(), out.into()], "print_bool")
                        .ok();
                }
            }
        } else if val.is_pointer_value() {
            // Pointer holding a bool (e.g., from ManualErrorExtract) — convert ptr→i64→i1
            let i64_val = ctx
                .builder
                .build_ptr_to_int(val.into_pointer_value(), ctx.i64_type(), "ptr_to_i64")
                .ok();
            if let Some(i64_val) = i64_val {
                let is_true = ctx
                    .builder
                    .build_int_compare(
                        IntPredicate::NE,
                        i64_val,
                        ctx.i64_type().const_zero(),
                        "is_true",
                    )
                    .ok();
                if let Some(is_true) = is_true {
                    let true_s = ctx.const_string(if newline { "true\n" } else { "true" });
                    let false_s = ctx.const_string(if newline { "false\n" } else { "false" });
                    let out = ctx
                        .builder
                        .build_select(is_true, true_s, false_s, "bool_s")
                        .ok();
                    if let Some(out) = out {
                        let fmt = ctx.const_string("%s");
                        ctx.builder
                            .build_call(printf, &[fmt.into(), out.into()], "print_bool")
                            .ok();
                    }
                }
            }
        }
        return;
    }

    if type_id == builtin::FLOAT {
        if val.is_float_value() {
            let fmt = if newline { "%f\n" } else { "%f" };
            let fmt = ctx.const_string(fmt);
            ctx.builder
                .build_call(printf, &[fmt.into(), val.into()], "print_f")
                .ok();
        } else if val.is_pointer_value() {
            // Pointer holding a float (e.g., from ManualErrorExtract) — convert ptr→i64→f64
            let i64_val = ctx
                .builder
                .build_ptr_to_int(val.into_pointer_value(), ctx.i64_type(), "ptr_to_i64")
                .ok();
            if let Some(i64_val) = i64_val {
                let tmp = ctx.builder.build_alloca(ctx.i64_type(), "f_tmp").ok();
                if let Some(tmp) = tmp {
                    ctx.builder.build_store(tmp, i64_val).ok();
                    let f_ptr = ctx
                        .builder
                        .build_pointer_cast(
                            tmp,
                            ctx.context.ptr_type(inkwell::AddressSpace::default()),
                            "f_ptr",
                        )
                        .ok();
                    if let Some(f_ptr) = f_ptr {
                        let f_val = ctx.builder.build_load(ctx.f64_type(), f_ptr, "f_val").ok();
                        if let Some(f_val) = f_val {
                            let fmt = if newline { "%f\n" } else { "%f" };
                            let fmt = ctx.const_string(fmt);
                            ctx.builder
                                .build_call(printf, &[fmt.into(), f_val.into()], "print_f")
                                .ok();
                        }
                    }
                }
            }
        }
        return;
    }

    if type_id == builtin::INT {
        if val.is_int_value() {
            let fmt = if newline { "%lld\n" } else { "%lld" };
            let fmt = ctx.const_string(fmt);
            let i64v = ctx
                .builder
                .build_int_z_extend_or_bit_cast(val.into_int_value(), ctx.i64_type(), "print_i64")
                .ok();
            if let Some(i64v) = i64v {
                let result = ctx
                    .builder
                    .build_call(printf, &[fmt.into(), i64v.into()], "print_i");
                if std::env::var("DOO_DEBUG").is_ok() {
                    let blk = ctx
                        .builder
                        .get_insert_block()
                        .map(|b| b.get_name().to_string_lossy().to_string());
                    doo_debug!(
                        "CODEGEN",
                        "emit_print_value INT in block {:?}, call result: {:?}",
                        blk,
                        result.is_ok()
                    );
                }
                result.ok();
            } else if std::env::var("DOO_DEBUG").is_ok() {
                doo_debug!("CODEGEN", "emit_print_value INT: i64 extend failed");
            }
        } else if val.is_pointer_value() {
            // Pointer holding an int (e.g., from ManualErrorExtract) — convert ptr→i64
            let i64v = ctx
                .builder
                .build_ptr_to_int(val.into_pointer_value(), ctx.i64_type(), "ptr_to_int")
                .ok();
            if let Some(i64v) = i64v {
                let fmt = if newline { "%lld\n" } else { "%lld" };
                let fmt = ctx.const_string(fmt);
                ctx.builder
                    .build_call(printf, &[fmt.into(), i64v.into()], "print_i")
                    .ok();
            }
        } else if std::env::var("DOO_DEBUG").is_ok() {
            doo_debug!(
                "CODEGEN",
                "emit_print_value INT: val is not int, is {:?}",
                val.get_type()
            );
        }
        return;
    }

    // Handle enum as StructValue (inline { i32, ptr }) - must check BEFORE pointer check
    if val.is_struct_value() {
        if let Some(TypeKind::Enum { name, variants }) = ctx.get_type_kind(type_id) {
            emit_print_enum_value(ctx, printf, val.into_struct_value(), &name, &variants);
            if newline {
                let nl = ctx.const_string("\n");
                ctx.builder
                    .build_call(printf, &[ctx.const_string("%s").into(), nl.into()], "")
                    .ok();
            }
            return;
        }
    }

    if val.is_pointer_value() {
        let ptr = val.into_pointer_value();

        if let Some(kind) = ctx.get_type_kind(type_id) {
            match kind {
                TypeKind::Tuple { elements } => {
                    emit_print_tuple(ctx, printf, ptr, &elements);
                    if newline {
                        let nl = ctx.const_string("\n");
                        ctx.builder
                            .build_call(printf, &[ctx.const_string("%s").into(), nl.into()], "")
                            .ok();
                    }
                    return;
                }
                TypeKind::Struct { name, fields } => {
                    // Extract just name and type for printing (visibility not needed)
                    let field_pairs: Vec<_> =
                        fields.iter().map(|(n, t, _)| (n.clone(), *t)).collect();
                    emit_print_struct(ctx, printf, ptr, &name, &field_pairs);
                    if newline {
                        let nl = ctx.const_string("\n");
                        ctx.builder
                            .build_call(printf, &[ctx.const_string("%s").into(), nl.into()], "")
                            .ok();
                    }
                    return;
                }
                TypeKind::Enum { name, variants } => {
                    emit_print_enum(ctx, printf, ptr, &name, &variants);
                    if newline {
                        let nl = ctx.const_string("\n");
                        ctx.builder
                            .build_call(printf, &[ctx.const_string("%s").into(), nl.into()], "")
                            .ok();
                    }
                    return;
                }
                TypeKind::Array { element } => {
                    emit_print_array(ctx, printf, ptr, element);
                    if newline {
                        let nl = ctx.const_string("\n");
                        ctx.builder
                            .build_call(printf, &[ctx.const_string("%s").into(), nl.into()], "")
                            .ok();
                    }
                    return;
                }
                TypeKind::Map { key, value } => {
                    emit_print_map(ctx, printf, ptr, key, value);
                    if newline {
                        let nl = ctx.const_string("\n");
                        ctx.builder
                            .build_call(printf, &[ctx.const_string("%s").into(), nl.into()], "")
                            .ok();
                    }
                    return;
                }
                _ => {}
            }
        }

        // For unknown pointer types, assume string
        let fmt = if newline { "%s\n" } else { "%s" };
        let fmt = ctx.const_string(fmt);
        ctx.builder
            .build_call(printf, &[fmt.into(), ptr.into()], "print_str")
            .ok();
        return;
    }
}

fn emit_print_tuple<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    tuple_ptr: PointerValue<'ctx>,
    element_types: &[doo_core::types::TypeId],
) {
    let open = ctx.const_string("(");
    let fmt_s = ctx.const_string("%s");
    ctx.builder
        .build_call(printf, &[fmt_s.into(), open.into()], "")
        .ok();

    // Struct/Tuple layout: fields are pointers stored sequentially
    // But codegen might store values directly if primitive?
    // Current Tuple implementation (composites.rs) stores *pointers* to values or values?
    // Usually it delegates to generic struct logic.
    // Assuming pointers or specific types.
    // Wait, Generic CodeGen maps TypeId to LLVM Type.
    // Struct/Tuple are StructType in LLVM.

    // We need LLVM type of the tuple to build GEP.
    // But `val` is just `ptr` (i8* or opaque).
    // We should cast it to the specific struct type.

    // BUT we don't have easy access to the LLVM struct type here without regenerating it.
    // `ctx.get_llvm_type(type_id)` should return it.
    // However, if we don't pass `type_id` of the Tuple itself...
    // `element_types` allows us to reconstruct it?
    // Actually, `emit_print_value` has `type_id`.
    // Let's rely on that? No, I need the inner logic.

    // Simpler approach: offsets.
    // But LLVM structs have padding. GEP is safer.
    // Construct LLVM type for tuple.
    let elem_types: Vec<_> = element_types
        .iter()
        .map(|t| ctx.get_llvm_type(*t).into())
        .collect();
    let tuple_llvm_type = ctx.context.struct_type(&elem_types, false);
    let tuple_typed_ptr = ctx
        .builder
        .build_pointer_cast(
            tuple_ptr,
            tuple_llvm_type.ptr_type(AddressSpace::default()),
            "tuple_cast",
        )
        .ok();

    if let Some(base) = tuple_typed_ptr {
        for (i, &ty) in element_types.iter().enumerate() {
            if i > 0 {
                let comma = ctx.const_string(", ");
                ctx.builder
                    .build_call(printf, &[fmt_s.into(), comma.into()], "")
                    .ok();
            }

            let field_ptr = ctx
                .builder
                .build_struct_gep(tuple_llvm_type, base, i as u32, "field")
                .ok();
            if let Some(fp) = field_ptr {
                let llvm_ty = ctx.get_llvm_type(ty);
                let val = ctx.builder.build_load(llvm_ty, fp, "val").ok();
                if let Some(v) = val {
                    emit_print_value(ctx, printf, ty, v, false, true);
                }
            }
        }
    }

    let close = ctx.const_string(")");
    ctx.builder
        .build_call(printf, &[fmt_s.into(), close.into()], "")
        .ok();
}

fn emit_print_struct<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    struct_ptr: PointerValue<'ctx>,
    name: &str,
    fields: &[(String, doo_core::types::TypeId)],
) {
    let type_name_utf8 = format!("{} {{ ", name);
    let prefix = ctx.const_string(&type_name_utf8);
    let fmt_s = ctx.const_string("%s");
    ctx.builder
        .build_call(printf, &[fmt_s.into(), prefix.into()], "")
        .ok();

    // Use the cached named struct type if available, otherwise create from field types
    // Use get_llvm_type for consistent type mapping (matches JSON.parse, StructCreate, etc.)
    let struct_llvm_type = if let Some(cached) = ctx.lookup_struct_type(name) {
        cached
    } else {
        // Manually create the struct type using get_llvm_type for consistency
        let field_llvm_types: Vec<inkwell::types::BasicTypeEnum> = fields
            .iter()
            .map(|(_, type_id)| ctx.get_llvm_type(*type_id))
            .collect();
        ctx.context.struct_type(&field_llvm_types, false)
    };

    let base = struct_ptr;

    for (i, (fname, fty)) in fields.iter().enumerate() {
        if i > 0 {
            let comma = ctx.const_string(", ");
            ctx.builder
                .build_call(printf, &[fmt_s.into(), comma.into()], "")
                .ok();
        }

        // Print field name
        let fname_s = ctx.const_string(&format!("{}: ", fname));
        ctx.builder
            .build_call(printf, &[fmt_s.into(), fname_s.into()], "")
            .ok();

        let field_ptr = ctx
            .builder
            .build_struct_gep(struct_llvm_type, base, i as u32, "field")
            .ok();
        if let Some(fp) = field_ptr {
            // Use get_llvm_type for consistent type mapping
            let llvm_ty = ctx.get_llvm_type(*fty);
            let val = ctx.builder.build_load(llvm_ty, fp, "val").ok();
            if let Some(v) = val {
                emit_print_value(ctx, printf, *fty, v, false, true);
            }
        }
    }

    let close = ctx.const_string(" }");
    ctx.builder
        .build_call(printf, &[fmt_s.into(), close.into()], "")
        .ok();
}

fn emit_print_enum<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    enum_ptr: PointerValue<'ctx>,
    name: &str,
    variants: &[(String, Option<doo_core::types::TypeId>)],
) {
    // Enum layout: { i32 tag (at offset 0), ptr payload (at offset 8) }
    let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
    let i32_type = ctx.context.i32_type();

    // Get tag using raw byte offset (more reliable than struct GEP for mixed allocations)
    let tag_ptr = ctx
        .builder
        .build_pointer_cast(
            enum_ptr,
            i32_type.ptr_type(AddressSpace::default()),
            "tag_ptr",
        )
        .ok();

    let tag_val = if let Some(tp) = tag_ptr {
        ctx.builder
            .build_load(i32_type, tp, "tag")
            .ok()
            .map(|v| v.into_int_value())
    } else {
        None
    };

    let Some(tag) = tag_val else {
        return;
    };

    // Emit switch or if-chain to print correct variant
    // For simplicity here, we'll iterate variants and generate runtime check
    // Optimization: Use a switch statement block structure, but `emit_print_value` is recursive helper inside a block.
    // Generating complex control flow inside this helper is hard because it returns () and appends to current block.
    // We can do it!

    let current_fn = ctx
        .builder
        .get_insert_block()
        .unwrap()
        .get_parent()
        .unwrap();
    let merge_bb = ctx.context.append_basic_block(current_fn, "print_enum_end");
    let default_bb = ctx
        .context
        .append_basic_block(current_fn, "print_enum_default");

    // Generate switch
    let mut cases = Vec::with_capacity(variants.len());
    let mut target_bbs = Vec::with_capacity(variants.len());

    for (i, _) in variants.iter().enumerate() {
        let bb = ctx
            .context
            .append_basic_block(current_fn, &format!("print_enum_var_{}", i));
        cases.push((ctx.context.i32_type().const_int(i as u64, false), bb));
        target_bbs.push(bb);
    }

    ctx.builder.build_switch(tag, default_bb, &cases).ok();

    // Default (Should technically be unreachable if valid enum)
    ctx.builder.position_at_end(default_bb);
    let unk = ctx.const_string(&format!("{}::Unknown", name));
    let fmt_s = ctx.const_string("%s");
    ctx.builder
        .build_call(printf, &[fmt_s.into(), unk.into()], "")
        .ok();
    ctx.builder.build_unconditional_branch(merge_bb).ok();

    // Variants
    for (i, (var_name, payload_ty)) in variants.iter().enumerate() {
        let bb = target_bbs[i];
        ctx.builder.position_at_end(bb);

        // Print Variant Name
        let prefix = format!("{}::", name);
        let prefix_s = ctx.const_string(&prefix);
        ctx.builder
            .build_call(printf, &[fmt_s.into(), prefix_s.into()], "")
            .ok();

        let vname_s = ctx.const_string(var_name);
        ctx.builder
            .build_call(printf, &[fmt_s.into(), vname_s.into()], "")
            .ok();

        if let Some(pty) = payload_ty {
            let open = ctx.const_string("(");
            ctx.builder
                .build_call(printf, &[fmt_s.into(), open.into()], "")
                .ok();

            // Get the payload pointer at offset 8 (after tag + padding)
            let payload_ptr_field = unsafe {
                ctx.builder
                    .build_gep(
                        ctx.context.i8_type(),
                        enum_ptr,
                        &[ctx.context.i64_type().const_int(8, false)],
                        "payload_ptr_field",
                    )
                    .ok()
            };

            if let Some(ppf) = payload_ptr_field {
                // Cast to ptr* to load the stored pointer
                let ppf_typed = ctx
                    .builder
                    .build_pointer_cast(
                        ppf,
                        ptr_type.ptr_type(AddressSpace::default()),
                        "ppf_typed",
                    )
                    .ok();

                let payload_ptr = ppf_typed.and_then(|pt| {
                    ctx.builder
                        .build_load(ptr_type, pt, "payload_ptr")
                        .ok()
                        .map(|v| v.into_pointer_value())
                });

                if let Some(pp) = payload_ptr {
                    // For pointer types (Str, Array, Map, etc.), the payload_ptr IS the value
                    // For value types (Int, Float, Bool), payload_ptr points TO the value
                    let llvm_pty = ctx.get_llvm_type(*pty);

                    if llvm_pty.is_pointer_type() {
                        // Pointer type: the payload IS the value (string ptr, array ptr, etc.)
                        emit_print_value(ctx, printf, *pty, pp.into(), false, true);
                    } else {
                        // Value type: load the actual value from the payload pointer
                        let val = ctx.builder.build_load(llvm_pty, pp, "pval").ok();
                        if let Some(v) = val {
                            emit_print_value(ctx, printf, *pty, v, false, true);
                        }
                    }
                }
            }

            let close = ctx.const_string(")");
            ctx.builder
                .build_call(printf, &[fmt_s.into(), close.into()], "")
                .ok();
        }

        ctx.builder.build_unconditional_branch(merge_bb).ok();
    }

    ctx.builder.position_at_end(merge_bb);
}

/// Print an enum from a StructValue (inline enum) - extracts tag and payload directly without boxing
fn emit_print_enum_value<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    enum_val: inkwell::values::StructValue<'ctx>,
    name: &str,
    variants: &[(String, Option<doo_core::types::TypeId>)],
) {
    // Extract tag from struct value (field 0)
    let tag = match ctx.builder.build_extract_value(enum_val, 0, "tag") {
        Ok(v) => v.into_int_value(),
        Err(_) => return,
    };

    // Extract payload pointer from struct value (field 1)
    let payload_ptr = match ctx.builder.build_extract_value(enum_val, 1, "payload_ptr") {
        Ok(v) => v.into_pointer_value(),
        Err(_) => return,
    };

    let current_fn = ctx
        .builder
        .get_insert_block()
        .unwrap()
        .get_parent()
        .unwrap();
    let merge_bb = ctx.context.append_basic_block(current_fn, "print_enum_end");
    let default_bb = ctx
        .context
        .append_basic_block(current_fn, "print_enum_default");

    let mut cases = Vec::with_capacity(variants.len());
    let mut target_bbs = Vec::with_capacity(variants.len());

    for (i, _) in variants.iter().enumerate() {
        let bb = ctx
            .context
            .append_basic_block(current_fn, &format!("print_enum_var_{}", i));
        cases.push((ctx.context.i32_type().const_int(i as u64, false), bb));
        target_bbs.push(bb);
    }

    ctx.builder.build_switch(tag, default_bb, &cases).ok();

    // Default
    ctx.builder.position_at_end(default_bb);
    let unk = ctx.const_string(&format!("{}::Unknown", name));
    let fmt_s = ctx.const_string("%s");
    ctx.builder
        .build_call(printf, &[fmt_s.into(), unk.into()], "")
        .ok();
    ctx.builder.build_unconditional_branch(merge_bb).ok();

    // Variants
    for (i, (var_name, payload_ty)) in variants.iter().enumerate() {
        let bb = target_bbs[i];
        ctx.builder.position_at_end(bb);

        let fmt_s = ctx.const_string("%s");
        let prefix = format!("{}::", name);
        let prefix_s = ctx.const_string(&prefix);
        ctx.builder
            .build_call(printf, &[fmt_s.into(), prefix_s.into()], "")
            .ok();

        let vname_s = ctx.const_string(var_name);
        ctx.builder
            .build_call(printf, &[fmt_s.into(), vname_s.into()], "")
            .ok();

        if let Some(pty) = payload_ty {
            let open = ctx.const_string("(");
            ctx.builder
                .build_call(printf, &[fmt_s.into(), open.into()], "")
                .ok();

            // For pointer types, payload_ptr IS the value
            // For value types, load from payload_ptr
            let llvm_pty = ctx.get_llvm_type(*pty);

            if llvm_pty.is_pointer_type() {
                emit_print_value(ctx, printf, *pty, payload_ptr.into(), false, true);
            } else {
                let val = ctx.builder.build_load(llvm_pty, payload_ptr, "pval").ok();
                if let Some(v) = val {
                    emit_print_value(ctx, printf, *pty, v, false, true);
                }
            }

            let close = ctx.const_string(")");
            ctx.builder
                .build_call(printf, &[fmt_s.into(), close.into()], "")
                .ok();
        }

        ctx.builder.build_unconditional_branch(merge_bb).ok();
    }

    ctx.builder.position_at_end(merge_bb);
}

fn emit_print_array<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    array_ptr: PointerValue<'ctx>,
    elem_type: doo_core::types::TypeId,
) {
    let fmt = ctx.const_string("%s");

    // Handle null array pointer (print "nil" instead of crashing)
    let is_null = ctx.builder.build_is_null(array_ptr, "arr_is_null").ok();
    if let Some(is_null) = is_null {
        let current_fn = match ctx.builder.get_insert_block().and_then(|b| b.get_parent()) {
            Some(f) => f,
            None => return,
        };

        let print_nil_bb = ctx.context.append_basic_block(current_fn, "print_arr_nil");
        let print_arr_bb = ctx
            .context
            .append_basic_block(current_fn, "print_arr_content");
        let merge_bb = ctx.context.append_basic_block(current_fn, "print_arr_done");

        ctx.builder
            .build_conditional_branch(is_null, print_nil_bb, print_arr_bb)
            .ok();

        // Print "nil" for null arrays
        ctx.builder.position_at_end(print_nil_bb);
        let nil_str = ctx.const_string("nil");
        ctx.builder
            .build_call(printf, &[fmt.into(), nil_str.into()], "print_nil")
            .ok();
        ctx.builder.build_unconditional_branch(merge_bb).ok();

        // Print actual array contents
        ctx.builder.position_at_end(print_arr_bb);
        emit_print_array_contents(ctx, printf, array_ptr, elem_type, merge_bb);

        ctx.builder.position_at_end(merge_bb);
    } else {
        // Fallback: just print array contents without null check
        let current_fn = match ctx.builder.get_insert_block().and_then(|b| b.get_parent()) {
            Some(f) => f,
            None => return,
        };
        let merge_bb = ctx.context.append_basic_block(current_fn, "print_arr_done");
        emit_print_array_contents(ctx, printf, array_ptr, elem_type, merge_bb);
        ctx.builder.position_at_end(merge_bb);
    }
}

/// Internal helper to print array contents (assumes array_ptr is not null)
fn emit_print_array_contents<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    array_ptr: PointerValue<'ctx>,
    elem_type: doo_core::types::TypeId,
    merge_bb: inkwell::basic_block::BasicBlock<'ctx>,
) {
    let open = ctx.const_string("[");
    let fmt = ctx.const_string("%s");
    ctx.builder
        .build_call(printf, &[fmt.into(), open.into()], "print_arr_open")
        .ok();

    let Some(len_i32) = load_len_i32(ctx, array_ptr) else {
        let close = ctx.const_string("]");
        ctx.builder
            .build_call(printf, &[fmt.into(), close.into()], "print_arr_close")
            .ok();
        ctx.builder.build_unconditional_branch(merge_bb).ok();
        return;
    };
    let len_i64 = ctx
        .builder
        .build_int_z_extend(len_i32, ctx.i64_type(), "len_i64")
        .ok();
    let Some(len_i64) = len_i64 else {
        let close = ctx.const_string("]");
        ctx.builder
            .build_call(printf, &[fmt.into(), close.into()], "print_arr_close")
            .ok();
        ctx.builder.build_unconditional_branch(merge_bb).ok();
        return;
    };

    let elem_llvm = ctx.get_llvm_type(elem_type);
    let elem_ptr_ty = elem_llvm.ptr_type(AddressSpace::default());
    let base = ctx
        .builder
        .build_pointer_cast(array_ptr, elem_ptr_ty, "arr_data_cast")
        .ok();
    let Some(base) = base else {
        let close = ctx.const_string("]");
        ctx.builder
            .build_call(printf, &[fmt.into(), close.into()], "print_arr_close")
            .ok();
        ctx.builder.build_unconditional_branch(merge_bb).ok();
        return;
    };

    let current_fn = match ctx.builder.get_insert_block().and_then(|b| b.get_parent()) {
        Some(f) => f,
        None => return,
    };

    let loop_bb = ctx.context.append_basic_block(current_fn, "print_arr_loop");
    let body_bb = ctx.context.append_basic_block(current_fn, "print_arr_body");
    let inc_bb = ctx.context.append_basic_block(current_fn, "print_arr_inc");
    let end_bb = ctx.context.append_basic_block(current_fn, "print_arr_end");

    let idx_alloca = ctx.builder.build_alloca(ctx.i64_type(), "idx").ok();
    let Some(idx_alloca) = idx_alloca else {
        return;
    };
    ctx.builder
        .build_store(idx_alloca, ctx.i64_type().const_zero())
        .ok();

    ctx.builder.build_unconditional_branch(loop_bb).ok();

    ctx.builder.position_at_end(loop_bb);
    let idx = ctx
        .builder
        .build_load(ctx.i64_type(), idx_alloca, "idx")
        .ok()
        .map(|v| v.into_int_value());
    let Some(idx) = idx else {
        return;
    };
    let cond = ctx
        .builder
        .build_int_compare(IntPredicate::ULT, idx, len_i64, "cond")
        .ok();
    let Some(cond) = cond else {
        return;
    };
    ctx.builder
        .build_conditional_branch(cond, body_bb, end_bb)
        .ok();

    ctx.builder.position_at_end(body_bb);
    let need_comma = ctx
        .builder
        .build_int_compare(
            IntPredicate::UGT,
            idx,
            ctx.i64_type().const_zero(),
            "need_comma",
        )
        .ok();
    if let Some(need_comma) = need_comma {
        let comma_bb = ctx
            .context
            .append_basic_block(current_fn, "print_arr_comma");
        let after_comma_bb = ctx
            .context
            .append_basic_block(current_fn, "print_arr_after_comma");
        ctx.builder
            .build_conditional_branch(need_comma, comma_bb, after_comma_bb)
            .ok();

        ctx.builder.position_at_end(comma_bb);
        let comma = ctx.const_string(", ");
        ctx.builder
            .build_call(printf, &[fmt.into(), comma.into()], "print_comma")
            .ok();
        ctx.builder.build_unconditional_branch(after_comma_bb).ok();

        ctx.builder.position_at_end(after_comma_bb);
    }

    let elem_ptr = unsafe { ctx.builder.build_gep(elem_llvm, base, &[idx], "elem_ptr") }.ok();
    if let Some(elem_ptr) = elem_ptr {
        let elem_val = ctx.builder.build_load(elem_llvm, elem_ptr, "elem").ok();
        if let Some(elem_val) = elem_val {
            emit_print_value(ctx, printf, elem_type, elem_val, false, true);
        }
    }
    ctx.builder.build_unconditional_branch(inc_bb).ok();

    ctx.builder.position_at_end(inc_bb);
    let next = ctx
        .builder
        .build_int_add(idx, ctx.i64_type().const_int(1, false), "next")
        .ok();
    if let Some(next) = next {
        ctx.builder.build_store(idx_alloca, next).ok();
    }
    ctx.builder.build_unconditional_branch(loop_bb).ok();

    ctx.builder.position_at_end(end_bb);
    let close = ctx.const_string("]");
    ctx.builder
        .build_call(printf, &[fmt.into(), close.into()], "print_arr_close")
        .ok();
    ctx.builder.build_unconditional_branch(merge_bb).ok();
}

fn emit_print_map<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    map_ptr: PointerValue<'ctx>,
    key_type: doo_core::types::TypeId,
    val_type: doo_core::types::TypeId,
) {
    let fmt = ctx.const_string("%s");

    // Handle null map pointer (print "nil" instead of crashing)
    let is_null = ctx.builder.build_is_null(map_ptr, "map_is_null").ok();
    if let Some(is_null) = is_null {
        let current_fn = match ctx.builder.get_insert_block().and_then(|b| b.get_parent()) {
            Some(f) => f,
            None => return,
        };

        let print_nil_bb = ctx.context.append_basic_block(current_fn, "print_map_nil");
        let print_map_bb = ctx
            .context
            .append_basic_block(current_fn, "print_map_content");
        let merge_bb = ctx.context.append_basic_block(current_fn, "print_map_done");

        ctx.builder
            .build_conditional_branch(is_null, print_nil_bb, print_map_bb)
            .ok();

        // Print "nil" for null maps
        ctx.builder.position_at_end(print_nil_bb);
        let nil_str = ctx.const_string("nil");
        ctx.builder
            .build_call(printf, &[fmt.into(), nil_str.into()], "print_nil")
            .ok();
        ctx.builder.build_unconditional_branch(merge_bb).ok();

        // Print actual map contents
        ctx.builder.position_at_end(print_map_bb);
        emit_print_map_contents(ctx, printf, map_ptr, key_type, val_type, merge_bb);

        ctx.builder.position_at_end(merge_bb);
    } else {
        // Fallback: just print map contents without null check
        let current_fn = match ctx.builder.get_insert_block().and_then(|b| b.get_parent()) {
            Some(f) => f,
            None => return,
        };
        let merge_bb = ctx.context.append_basic_block(current_fn, "print_map_done");
        emit_print_map_contents(ctx, printf, map_ptr, key_type, val_type, merge_bb);
        ctx.builder.position_at_end(merge_bb);
    }
}

/// Internal helper to print map contents (assumes map_ptr is not null)
fn emit_print_map_contents<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    map_ptr: PointerValue<'ctx>,
    key_type: doo_core::types::TypeId,
    val_type: doo_core::types::TypeId,
    merge_bb: inkwell::basic_block::BasicBlock<'ctx>,
) {
    let open = ctx.const_string("{");
    let fmt = ctx.const_string("%s");
    ctx.builder
        .build_call(printf, &[fmt.into(), open.into()], "print_map_open")
        .ok();

    let Some(len_i32) = load_len_i32(ctx, map_ptr) else {
        let close = ctx.const_string("}");
        ctx.builder
            .build_call(printf, &[fmt.into(), close.into()], "print_map_close")
            .ok();
        ctx.builder.build_unconditional_branch(merge_bb).ok();
        return;
    };
    let len_i64 = ctx
        .builder
        .build_int_z_extend(len_i32, ctx.i64_type(), "len_i64")
        .ok();
    let Some(len_i64) = len_i64 else {
        let close = ctx.const_string("}");
        ctx.builder
            .build_call(printf, &[fmt.into(), close.into()], "print_map_close")
            .ok();
        ctx.builder.build_unconditional_branch(merge_bb).ok();
        return;
    };

    let key_llvm = ctx.get_llvm_type(key_type);
    let val_llvm = ctx.get_llvm_type(val_type);
    let pair_ty = ctx
        .context
        .struct_type(&[key_llvm.into(), val_llvm.into()], false);
    let pair_ptr_ty = pair_ty.ptr_type(AddressSpace::default());
    let base = ctx
        .builder
        .build_pointer_cast(map_ptr, pair_ptr_ty, "map_data_cast")
        .ok();
    let Some(base) = base else {
        let close = ctx.const_string("}");
        ctx.builder
            .build_call(printf, &[fmt.into(), close.into()], "print_map_close")
            .ok();
        ctx.builder.build_unconditional_branch(merge_bb).ok();
        return;
    };

    let current_fn = match ctx.builder.get_insert_block().and_then(|b| b.get_parent()) {
        Some(f) => f,
        None => return,
    };

    let loop_bb = ctx.context.append_basic_block(current_fn, "print_map_loop");
    let body_bb = ctx.context.append_basic_block(current_fn, "print_map_body");
    let inc_bb = ctx.context.append_basic_block(current_fn, "print_map_inc");
    let end_bb = ctx.context.append_basic_block(current_fn, "print_map_end");

    let idx_alloca = ctx.builder.build_alloca(ctx.i64_type(), "idx").ok();
    let Some(idx_alloca) = idx_alloca else {
        return;
    };
    ctx.builder
        .build_store(idx_alloca, ctx.i64_type().const_zero())
        .ok();
    ctx.builder.build_unconditional_branch(loop_bb).ok();

    ctx.builder.position_at_end(loop_bb);
    let idx = ctx
        .builder
        .build_load(ctx.i64_type(), idx_alloca, "idx")
        .ok()
        .map(|v| v.into_int_value());
    let Some(idx) = idx else {
        return;
    };
    let cond = ctx
        .builder
        .build_int_compare(IntPredicate::ULT, idx, len_i64, "cond")
        .ok();
    let Some(cond) = cond else {
        return;
    };
    ctx.builder
        .build_conditional_branch(cond, body_bb, end_bb)
        .ok();

    ctx.builder.position_at_end(body_bb);
    let need_comma = ctx
        .builder
        .build_int_compare(
            IntPredicate::UGT,
            idx,
            ctx.i64_type().const_zero(),
            "need_comma",
        )
        .ok();
    if let Some(need_comma) = need_comma {
        let comma_bb = ctx
            .context
            .append_basic_block(current_fn, "print_map_comma");
        let after_comma_bb = ctx
            .context
            .append_basic_block(current_fn, "print_map_after_comma");
        ctx.builder
            .build_conditional_branch(need_comma, comma_bb, after_comma_bb)
            .ok();

        ctx.builder.position_at_end(comma_bb);
        let comma = ctx.const_string(", ");
        ctx.builder
            .build_call(printf, &[fmt.into(), comma.into()], "print_comma")
            .ok();
        ctx.builder.build_unconditional_branch(after_comma_bb).ok();

        ctx.builder.position_at_end(after_comma_bb);
    }

    let pair_ptr = unsafe { ctx.builder.build_gep(pair_ty, base, &[idx], "pair_ptr") }.ok();
    if let Some(pair_ptr) = pair_ptr {
        let kptr = ctx
            .builder
            .build_struct_gep(pair_ty, pair_ptr, 0, "kptr")
            .ok();
        let vptr = ctx
            .builder
            .build_struct_gep(pair_ty, pair_ptr, 1, "vptr")
            .ok();
        if let (Some(kptr), Some(vptr)) = (kptr, vptr) {
            let k = ctx.builder.build_load(key_llvm, kptr, "k").ok();
            let v = ctx.builder.build_load(val_llvm, vptr, "v").ok();
            if let (Some(k), Some(v)) = (k, v) {
                emit_print_value(ctx, printf, key_type, k, false, true);
                let sep = ctx.const_string(": ");
                ctx.builder
                    .build_call(printf, &[fmt.into(), sep.into()], "print_sep")
                    .ok();
                emit_print_value(ctx, printf, val_type, v, false, true);
            }
        }
    }
    ctx.builder.build_unconditional_branch(inc_bb).ok();

    ctx.builder.position_at_end(inc_bb);
    let next = ctx
        .builder
        .build_int_add(idx, ctx.i64_type().const_int(1, false), "next")
        .ok();
    if let Some(next) = next {
        ctx.builder.build_store(idx_alloca, next).ok();
    }
    ctx.builder.build_unconditional_branch(loop_bb).ok();

    ctx.builder.position_at_end(end_bb);
    let close = ctx.const_string("}");
    ctx.builder
        .build_call(printf, &[fmt.into(), close.into()], "print_map_close")
        .ok();
    ctx.builder.build_unconditional_branch(merge_bb).ok();
}

// ============================================================================
// FFI Call Implementation
// ============================================================================

/// FFI function signature: (param_types, return_type, is_variadic)
/// - param_types: slice of ("ptr" | "i64" | "i32" | "f64" | "void")
/// - return_type: "ptr" | "i64" | "i32" | "f64" | "void"
/// - is_variadic: whether function accepts variable arguments
type FfiSignature = (&'static [&'static str], &'static str, bool);

/// Get FFI function signature for known functions.
/// Returns (param_types, return_type, is_variadic).
fn get_ffi_signature(symbol: &str) -> Option<FfiSignature> {
    // Use match for compile-time known signatures
    match symbol {
        // Standard C Library
        ffi_names::MALLOC => Some((&["i64"], "ptr", false)),
        ffi_names::FREE => Some((&["ptr"], "void", false)),
        ffi_names::REALLOC => Some((&["ptr", "i64"], "ptr", false)),
        ffi_names::STRLEN => Some((&["ptr"], "i64", false)),
        ffi_names::STRCMP => Some((&["ptr", "ptr"], "i32", false)),
        ffi_names::STRCPY => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::STRCAT => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::MEMCPY => Some((&["ptr", "ptr", "i64"], "ptr", false)),
        ffi_names::MEMSET => Some((&["ptr", "i32", "i64"], "ptr", false)),
        ffi_names::PRINTF => Some((&["ptr"], "i32", true)), // variadic
        ffi_names::SNPRINTF => Some((&["ptr", "i64", "ptr"], "i32", true)),
        ffi_names::PUTS => Some((&["ptr"], "i32", false)),
        ffi_names::PUTCHAR => Some((&["i32"], "i32", false)),

        // Doo Runtime
        ffi_names::DOO_ALLOC => Some((&["i64"], "ptr", false)),
        ffi_names::DOO_FREE => Some((&["ptr"], "void", false)),
        ffi_names::DOO_REALLOC => Some((&["ptr", "i64"], "ptr", false)),

        // JSON FFI
        ffi_names::DOO_JSON_WRITER_NEW => Some((&[], "ptr", false)),
        ffi_names::DOO_JSON_WRITER_FREE => Some((&["ptr"], "void", false)),
        ffi_names::DOO_JSON_WRITER_FINISH => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_JSON_WRITE_START_OBJECT => Some((&["ptr"], "void", false)),
        ffi_names::DOO_JSON_WRITE_END_OBJECT => Some((&["ptr"], "void", false)),
        ffi_names::DOO_JSON_WRITE_START_ARRAY => Some((&["ptr"], "void", false)),
        ffi_names::DOO_JSON_WRITE_END_ARRAY => Some((&["ptr"], "void", false)),
        ffi_names::DOO_JSON_WRITE_COMMA => Some((&["ptr"], "void", false)),
        ffi_names::DOO_JSON_WRITE_COLON => Some((&["ptr"], "void", false)),
        ffi_names::DOO_JSON_WRITE_KEY => Some((&["ptr", "ptr"], "void", false)),
        ffi_names::DOO_JSON_WRITE_KEY_INT => Some((&["ptr", "i64"], "void", false)),
        ffi_names::DOO_JSON_WRITE_KEY_FLOAT => Some((&["ptr", "f64"], "void", false)),
        ffi_names::DOO_JSON_WRITE_KEY_BOOL => Some((&["ptr", "i1"], "void", false)),
        ffi_names::DOO_JSON_WRITE_INT => Some((&["ptr", "i64"], "void", false)),
        ffi_names::DOO_JSON_WRITE_FLOAT => Some((&["ptr", "f64"], "void", false)),
        ffi_names::DOO_JSON_WRITE_BOOL => Some((&["ptr", "i32"], "void", false)),
        ffi_names::DOO_JSON_WRITE_STRING => Some((&["ptr", "ptr"], "void", false)),
        ffi_names::DOO_JSON_WRITE_NULL => Some((&["ptr"], "void", false)),
        ffi_names::DOO_JSON_PARSE => Some((&["ptr"], "ptr", false)),

        // File FFI
        ffi_names::DOO_FILE_READ => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_FILE_WRITE => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_FILE_APPEND => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_FILE_DELETE => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_FILE_EXISTS => Some((&["ptr"], "i32", false)),
        ffi_names::DOO_FILE_METADATA => Some((&["ptr"], "ptr", false)),

        // HTTP FFI
        ffi_names::DOO_HTTP_SERVER_NEW => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_SERVER_LISTEN => Some((&["ptr"], "ptr", false)),
        "doo_http_listen" => Some((&["ptr"], "ptr", false)),
        // Function pointer versions (handler is function pointer, not string)
        "doo_http_get_fn" => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        "doo_http_post_fn" => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        "doo_http_put_fn" => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        "doo_http_delete_fn" => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        "doo_http_patch_fn" => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        // String-based versions (legacy)
        "doo_http_get" => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        "doo_http_post" => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        "doo_http_put" => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        "doo_http_delete" => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        "doo_http_patch" => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        "doo_http_use" => Some((&["ptr", "ptr"], "ptr", false)),
        "doo_http_group" => Some((&["ptr", "ptr"], "ptr", false)),
        "doo_http_cors_custom" => Some((&["ptr", "ptr"], "ptr", false)),
        "doo_http_ratelimit_custom" => Some((&["ptr", "ptr"], "ptr", false)),
        "doo_http_get_with_middleware" => Some((&["ptr", "ptr", "ptr", "ptr"], "ptr", false)),
        "doo_http_post_with_middleware" => Some((&["ptr", "ptr", "ptr", "ptr"], "ptr", false)),
        "doo_http_put_with_middleware" => Some((&["ptr", "ptr", "ptr", "ptr"], "ptr", false)),
        "doo_http_delete_with_middleware" => Some((&["ptr", "ptr", "ptr", "ptr"], "ptr", false)),
        "doo_http_patch_with_middleware" => Some((&["ptr", "ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_REGISTER_ROUTE => Some((&["ptr", "ptr", "ptr", "ptr"], "void", false)),
        ffi_names::DOO_HTTP_REQ_GET_HEADER => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_REQ_GET_BODY => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_REQ_GET_PARAM => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_REQ_GET_QUERY => Some((&["ptr", "ptr"], "ptr", false)),
        "doo_http_req_query" => Some((&["ptr", "ptr"], "ptr", false)),
        "doo_http_req_param" => Some((&["ptr", "ptr"], "ptr", false)),
        "doo_http_req_header" => Some((&["ptr", "ptr"], "ptr", false)),
        "doo_http_next_call" => Some((&["ptr"], "ptr", false)),
        "doo_http_auth" => Some((&["ptr", "ptr", "ptr", "ptr", "ptr"], "ptr", false)),
        "doo_http_crud" => Some((&["ptr", "ptr", "ptr", "ptr"], "ptr", false)),
        "doo_http_parse_json" => Some((&["ptr"], "ptr", false)),
        "doo_http_to_json" => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_RES_SET_STATUS => Some((&["ptr", "i32"], "void", false)),
        ffi_names::DOO_HTTP_RES_SET_HEADER => Some((&["ptr", "ptr", "ptr"], "void", false)),
        ffi_names::DOO_HTTP_RES_SET_BODY => Some((&["ptr", "ptr"], "void", false)),
        ffi_names::DOO_HTTP_RES_JSON => Some((&["ptr", "ptr"], "void", false)),

        // Database FFI
        ffi_names::DOO_DB_POSTGRES => Some((&["ptr"], "ptr", false)),
        // These return *mut SimpleResult (pointer to heap-allocated result) for Windows ABI compatibility
        "doo_db_connect_postgres" => Some((&[], "ptr", false)),
        "doo_db_get_global" => Some((&[], "ptr", false)),
        "doo_db_raw" => Some((&["ptr", "ptr"], "ptr", false)),
        "doo_db_raw_param" => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        "doo_db_result_free" => Some((&["ptr"], "void", false)),
        "doo_db_free_string" => Some((&["ptr"], "void", false)),
        ffi_names::DOO_DB_FIND => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_FIND_ALL => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_INSERT => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_UPDATE => Some((&["ptr", "ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_DELETE => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_RAW => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_QUERY => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_EXISTS => Some((&["ptr", "ptr", "ptr"], "i32", false)),
        ffi_names::DOO_DB_RESULT_FREE => Some((&["ptr"], "void", false)),

        // Auth FFI
        ffi_names::DOO_AUTH_HASH_PASSWORD => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_AUTH_VERIFY_PASSWORD => Some((&["ptr", "ptr"], "i32", false)),
        ffi_names::DOO_AUTH_SIGN_TOKEN => Some((&["ptr", "ptr", "i64"], "ptr", false)),
        ffi_names::DOO_AUTH_VERIFY_TOKEN => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_AUTH_FREE_RESULT => Some((&["ptr"], "void", false)),
        "doo_auth_sign" => Some((&["ptr", "ptr", "i64"], "ptr", false)),
        "doo_auth_verify" => Some((&["ptr"], "ptr", false)),
        "doo_auth_free_string" => Some((&["ptr"], "void", false)),
        "doo_http_jwt" => Some((&[], "ptr", false)),

        // String FFI
        ffi_names::DOO_STRING_LEN_UTF8 => Some((&["ptr"], "i64", false)),
        ffi_names::DOO_STRING_CHAR_AT_UTF8 => Some((&["ptr", "i64"], "ptr", false)),
        ffi_names::DOO_STRING_REVERSE_UTF8 => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_STRING_SUBSTRING_UTF8 => Some((&["ptr", "i64", "i64"], "ptr", false)),
        ffi_names::DOO_STRING_REPLACE => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_STRING_TRIM => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_STRING_TRIM_START => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_STRING_TRIM_END => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_STRING_SPLIT => Some((&["ptr", "ptr"], "ptr", false)),

        // Math FFI
        ffi_names::FABS => Some((&["f64"], "f64", false)),
        ffi_names::FLOOR => Some((&["f64"], "f64", false)),
        ffi_names::CEIL => Some((&["f64"], "f64", false)),
        ffi_names::ROUND => Some((&["f64"], "f64", false)),
        ffi_names::SQRT => Some((&["f64"], "f64", false)),

        // Unknown - use default signature
        _ => None,
    }
}

/// Convert FFI type string to LLVM type.
fn ffi_type_to_llvm<'ctx>(
    ctx: &CodegenContext<'ctx>,
    type_str: &str,
) -> Option<BasicTypeEnum<'ctx>> {
    match type_str {
        "ptr" => Some(ctx.context.ptr_type(AddressSpace::default()).into()),
        "i64" => Some(ctx.i64_type().into()),
        "i32" => Some(ctx.i32_type().into()),
        "f64" => Some(ctx.f64_type().into()),
        "void" => None, // void is not a BasicType
        // SimpleResult: { i64 tag, i64 value } - returned by value for Result types
        // Using i64 for both fields ensures proper Windows x64 ABI compatibility.
        // On Windows x64, a struct of exactly 2x i64 (16 bytes) is returned via RAX:RDX registers.
        // This avoids sret (hidden pointer) issues that occur with { i32, ptr } layouts.
        "simple_result" => {
            let struct_ty = ctx
                .context
                .struct_type(&[ctx.i64_type().into(), ctx.i64_type().into()], false);
            Some(struct_ty.into())
        }
        _ => Some(ctx.i64_type().into()), // default to i64
    }
}

/// Declare an FFI function with proper signature and external linkage.
fn declare_ffi_function<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    symbol: &str,
    arg_count: usize,
) -> FunctionValue<'ctx> {
    // Check if already declared
    if let Some(func) = ctx.get_function(symbol) {
        return func;
    }

    let ptr_ty = ctx.context.ptr_type(AddressSpace::default());

    // Get known signature or build default
    let (param_types_vec, return_type, is_variadic) =
        if let Some((param_strs, ret_str, variadic)) = get_ffi_signature(symbol) {
            // Known function: use precise signature
            let params: Vec<BasicTypeEnum> = param_strs
                .iter()
                .filter_map(|s| ffi_type_to_llvm(ctx, s))
                .collect();

            let ret = ffi_type_to_llvm(ctx, ret_str);
            (params, ret, variadic)
        } else {
            // Unknown function: infer from argument count
            // Default: ptr params, ptr return
            let params: Vec<BasicTypeEnum> = (0..arg_count).map(|_| ptr_ty.into()).collect();
            (params, Some(ptr_ty.into()), false)
        };

    // Build function type
    let param_meta: Vec<inkwell::types::BasicMetadataTypeEnum> =
        param_types_vec.iter().map(|t| (*t).into()).collect();

    let fn_type = match return_type {
        Some(ret) => ret.fn_type(&param_meta, is_variadic),
        None => ctx.context.void_type().fn_type(&param_meta, is_variadic),
    };

    // Declare with external linkage for FFI
    let func = ctx
        .module
        .add_function(symbol, fn_type, Some(Linkage::External));

    // Cache the function
    // Note: function_cache is private, so we rely on module.get_function
    func
}

/// Convert a Doo value to FFI-compatible value if needed.
fn convert_to_ffi_arg<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
    expected_type: Option<&str>,
) -> inkwell::values::BasicMetadataValueEnum<'ctx> {
    match expected_type {
        Some("i32") => {
            // Convert i64 to i32 if needed
            if val.is_int_value() {
                let int_val = val.into_int_value();
                if int_val.get_type().get_bit_width() == 64 {
                    let truncated = ctx
                        .builder
                        .build_int_truncate(int_val, ctx.i32_type(), "i64_to_i32")
                        .unwrap();
                    return truncated.into();
                }
            }
            val.into()
        }
        Some("f64") => {
            // Ensure float type
            if val.is_int_value() {
                let int_val = val.into_int_value();
                let float_val = ctx
                    .builder
                    .build_signed_int_to_float(int_val, ctx.f64_type(), "int_to_f64")
                    .unwrap();
                return float_val.into();
            }
            val.into()
        }
        Some("ptr") => {
            // Convert non-pointer types to string pointers
            if val.is_int_value() {
                // Convert i64 to string using sprintf
                // Allocate buffer for max int64 string: "-9223372036854775808" (20 chars + null)
                let i8_type = ctx.context.i8_type();
                let i64_type = ctx.i64_type();
                let ptr_type = ctx.ptr_type();

                // Allocate 24 bytes for safety
                let buffer = ctx
                    .builder
                    .build_array_alloca(i8_type, i64_type.const_int(24, false), "int_to_str_buf")
                    .unwrap();

                // Get or declare sprintf
                let sprintf = ctx.module.get_function("sprintf").unwrap_or_else(|| {
                    let i32_type = ctx.i32_type();
                    let fn_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], true);
                    ctx.module.add_function("sprintf", fn_type, None)
                });

                // Format string: "%lld"
                let fmt = ctx.const_string("%lld");

                // Call sprintf(buffer, "%lld", value)
                let int_val = val.into_int_value();
                ctx.builder
                    .build_call(
                        sprintf,
                        &[buffer.into(), fmt.into(), int_val.into()],
                        "sprintf_int",
                    )
                    .ok();

                return buffer.into();
            } else if val.is_float_value() {
                // Convert f64 to string using sprintf
                let i8_type = ctx.context.i8_type();
                let i64_type = ctx.i64_type();
                let ptr_type = ctx.ptr_type();

                // Allocate 32 bytes for float string
                let buffer = ctx
                    .builder
                    .build_array_alloca(i8_type, i64_type.const_int(32, false), "float_to_str_buf")
                    .unwrap();

                // Get or declare sprintf
                let sprintf = ctx.module.get_function("sprintf").unwrap_or_else(|| {
                    let i32_type = ctx.i32_type();
                    let fn_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], true);
                    ctx.module.add_function("sprintf", fn_type, None)
                });

                // Format string: "%g" (compact float format)
                let fmt = ctx.const_string("%g");

                // Call sprintf(buffer, "%g", value)
                let float_val = val.into_float_value();
                ctx.builder
                    .build_call(
                        sprintf,
                        &[buffer.into(), fmt.into(), float_val.into()],
                        "sprintf_float",
                    )
                    .ok();

                return buffer.into();
            } else if val.is_struct_value() {
                // Struct value (e.g., enum { i32, ptr }) needs to be boxed to pointer
                // This handles single enum values passed to FFI functions like doo_db_raw_param
                let struct_val = val.into_struct_value();
                let alloca = ctx
                    .builder
                    .build_alloca(struct_val.get_type(), "enum_box")
                    .unwrap();
                ctx.builder.build_store(alloca, struct_val).ok();
                return alloca.into();
            }
            // Already a pointer - pass through
            val.into()
        }
        _ => val.into(),
    }
}

/// Try to convert an enum operand to a JSON string for doo_db_raw_param.
/// Returns Some(pointer_value) if the operand is a known enum, None otherwise.
fn try_convert_enum_to_json_string<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    operand: &MirOperand,
) -> Option<PointerValue<'ctx>> {
    // Get the temp/local name to look up enum type
    let var_name = match operand {
        MirOperand::Temp(name) | MirOperand::Local(name) => name.as_str(),
        _ => return None,
    };

    // Check if this temp/local is a known enum type
    let enum_name = ctx.temp_struct_types.get(var_name)?.clone();

    // Look up enum type in registry to get variants
    let type_id = ctx.type_registry.lookup(&enum_name)?;
    let type_info = ctx.type_registry.get(type_id)?;

    let variants: Vec<(String, u32)> = match &type_info.kind {
        doo_core::types::TypeKind::Enum { variants, .. } => variants
            .iter()
            .enumerate()
            .map(|(i, (name, _))| (name.clone(), i as u32))
            .collect(),
        _ => return None,
    };

    // Get the enum value
    let enum_val = operand_to_value(ctx, operand)?;
    let struct_val = if enum_val.is_struct_value() {
        enum_val.into_struct_value()
    } else {
        return None;
    };

    // Extract tag from enum struct
    let tag = ctx
        .builder
        .build_extract_value(struct_val, 0, "enum_tag_for_json")
        .ok()?
        .into_int_value();

    // Generate switch-case to convert tag -> JSON string
    let current_block = ctx.builder.get_insert_block()?;
    let target_fn = current_block.get_parent()?;
    let merge_block = ctx.context.append_basic_block(target_fn, "enum_json_merge");
    let default_block = ctx
        .context
        .append_basic_block(target_fn, "enum_json_default");

    // Build default block with unknown string
    ctx.builder.position_at_end(default_block);
    let unknown_str = ctx.const_string("[\"Unknown\"]");
    ctx.builder.build_unconditional_branch(merge_block).ok();

    // Build case blocks for each variant
    let ptr_type = ctx.ptr_type();
    let mut incoming_vals: Vec<(BasicValueEnum<'ctx>, inkwell::basic_block::BasicBlock<'ctx>)> =
        Vec::new();
    let mut cases: Vec<(
        inkwell::values::IntValue<'ctx>,
        inkwell::basic_block::BasicBlock<'ctx>,
    )> = Vec::new();

    incoming_vals.push((unknown_str.into(), default_block));

    for (variant_name, variant_idx) in &variants {
        let case_block = ctx
            .context
            .append_basic_block(target_fn, &format!("enum_case_{}", variant_name));
        ctx.builder.position_at_end(case_block);

        // Create JSON array string: ["VariantName"]
        let json_str = format!("[\"{}\"]", variant_name);
        let str_ptr = ctx.const_string(&json_str);
        ctx.builder.build_unconditional_branch(merge_block).ok();

        cases.push((
            ctx.context.i32_type().const_int(*variant_idx as u64, false),
            case_block,
        ));
        incoming_vals.push((str_ptr.into(), case_block));
    }

    // Build switch in original block
    ctx.builder.position_at_end(current_block);
    ctx.builder.build_switch(tag, default_block, &cases).ok();

    // Build phi in merge block
    ctx.builder.position_at_end(merge_block);
    let phi = ctx.builder.build_phi(ptr_type, "enum_json_str").ok()?;

    let incoming_refs: Vec<(
        &dyn inkwell::values::BasicValue<'ctx>,
        inkwell::basic_block::BasicBlock<'ctx>,
    )> = incoming_vals
        .iter()
        .map(|(v, b)| (v as &dyn inkwell::values::BasicValue<'ctx>, *b))
        .collect();
    phi.add_incoming(&incoming_refs);

    Some(phi.as_basic_value().into_pointer_value())
}

/// Try to convert an array of enums to a JSON string for doo_db_raw_param.
/// Returns Some(pointer_value) if the operand is an array of enums, None otherwise.
/// Handles both homogeneous enum arrays (all same type) and mixed enum arrays.
/// Also handles EMPTY arrays by returning "[]".
fn try_convert_enum_array_to_json_string<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    operand: &MirOperand,
) -> Option<PointerValue<'ctx>> {
    // Get the temp/local name to look up array element type
    let var_name = match operand {
        MirOperand::Temp(name) | MirOperand::Local(name) => name.as_str(),
        _ => return None,
    };

    // Check if this temp/local is a known array with element type
    let elem_type_id = ctx.array_element_types.get(var_name)?.clone();

    // IMPORTANT: Check for EMPTY arrays FIRST before generating any LLVM code
    // Empty arrays are tracked in array_element_types but NOT in array_element_temps
    // We must handle them explicitly to return "[]" JSON string
    let has_element_temps = ctx.array_element_temps.contains_key(var_name);
    if !has_element_temps {
        // This is an empty array - return "[]" directly
        if std::env::var("DOO_DEBUG").is_ok() {
            doo_debug!(
                "CODEGEN",
                "try_convert_enum_array_to_json_string: empty array {} -> \"[]\"",
                var_name
            );
        }
        return Some(ctx.const_string("[]"));
    }

    // Look up element type in registry to check if it's an enum
    let type_info = ctx.type_registry.get(elem_type_id);

    // Try homogeneous enum array first
    if let Some(info) = &type_info {
        if let doo_core::types::TypeKind::Enum { name, variants, .. } = &info.kind {
            let variant_names: Vec<String> =
                variants.iter().map(|(vname, _)| vname.clone()).collect();

            if std::env::var("DOO_DEBUG").is_ok() {
                doo_debug!(
                    "CODEGEN",
                    "Converting homogeneous enum array {} with variants: {:?}",
                    name,
                    variant_names
                );
            }

            // Get the array pointer
            let array_val = operand_to_value(ctx, operand)?;
            let array_ptr = if array_val.is_pointer_value() {
                array_val.into_pointer_value()
            } else {
                return None;
            };

            // Create variant names string (comma-separated)
            let variants_str = variant_names.join(",");
            let variants_ptr = ctx.const_string(&variants_str);

            // Enum stride is 16 bytes: { i32 tag, ptr payload } = 4 + 8 = 12, padded to 16
            let stride = ctx.i32_type().const_int(16, false);

            // Declare doo_db_serialize_enum_array if not already declared
            let serialize_fn = ctx
                .module
                .get_function("doo_db_serialize_enum_array")
                .unwrap_or_else(|| {
                    let ptr_type = ctx.ptr_type();
                    let i32_type = ctx.i32_type();
                    let fn_type = ptr_type
                        .fn_type(&[ptr_type.into(), ptr_type.into(), i32_type.into()], false);
                    ctx.module
                        .add_function("doo_db_serialize_enum_array", fn_type, None)
                });

            // Call doo_db_serialize_enum_array(array_ptr, variants, stride)
            let result = ctx
                .builder
                .build_call(
                    serialize_fn,
                    &[array_ptr.into(), variants_ptr.into(), stride.into()],
                    "enum_array_json",
                )
                .ok()?
                .try_as_basic_value()
                .basic()?;

            return Some(result.into_pointer_value());
        }
    }

    // Fallback: try mixed enum array via element temps
    try_convert_mixed_enum_array_to_json_string(ctx, var_name)
}

/// Convert a mixed-type enum array to JSON string by checking individual element temps.
fn try_convert_mixed_enum_array_to_json_string<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    array_var_name: &str,
) -> Option<PointerValue<'ctx>> {
    // Get element temps for this array
    let element_temps = ctx.array_element_temps.get(array_var_name)?.clone();

    if element_temps.is_empty() {
        return None;
    }

    // Collect enum info for each element
    let mut enum_infos: Vec<(String, Vec<(String, u32)>)> = Vec::new();
    for temp_name in &element_temps {
        if let Some(enum_name) = ctx.temp_struct_types.get(temp_name) {
            let type_id = ctx.type_registry.lookup(enum_name)?;
            let type_info = ctx.type_registry.get(type_id)?;

            if let doo_core::types::TypeKind::Enum { variants, .. } = &type_info.kind {
                let variant_list: Vec<(String, u32)> = variants
                    .iter()
                    .enumerate()
                    .map(|(i, (name, _))| (name.clone(), i as u32))
                    .collect();
                enum_infos.push((enum_name.clone(), variant_list));
            } else {
                return None; // Not an enum element
            }
        } else {
            return None; // Element type not tracked
        }
    }

    if std::env::var("DOO_DEBUG").is_ok() {
        doo_debug!(
            "CODEGEN",
            "Converting mixed enum array with {} elements",
            enum_infos.len()
        );
    }

    // Generate code to build JSON array string at runtime
    // We'll create: ["variant1", "variant2", ...]

    let ptr_type = ctx.ptr_type();
    let i64_type = ctx.i64_type();
    let i8_type = ctx.context.i8_type();

    // Allocate buffer for JSON string (generous size)
    let buffer_size = i64_type.const_int(256, false);
    let buffer = ctx
        .builder
        .build_array_alloca(i8_type, buffer_size, "mixed_json_buf")
        .ok()?;

    // Get sprintf
    let sprintf = ctx.module.get_function("sprintf").unwrap_or_else(|| {
        let i32_type = ctx.i32_type();
        let fn_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], true);
        ctx.module.add_function("sprintf", fn_type, None)
    });

    // Get strlen
    let strlen = ctx.module.get_function("strlen").unwrap_or_else(|| {
        let fn_type = i64_type.fn_type(&[ptr_type.into()], false);
        ctx.module.add_function("strlen", fn_type, None)
    });

    // Start with "["
    let open_bracket = ctx.const_string("[");
    ctx.builder
        .build_call(sprintf, &[buffer.into(), open_bracket.into()], "")
        .ok();

    // For each element, generate switch-case to append "variant"
    for (elem_idx, (temp_name, (enum_name, variants))) in
        element_temps.iter().zip(enum_infos.iter()).enumerate()
    {
        // Get the current buffer position
        let current_len = ctx
            .builder
            .build_call(strlen, &[buffer.into()], "cur_len")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();
        let write_pos = unsafe {
            ctx.builder
                .build_gep(i8_type, buffer, &[current_len], "write_pos")
        }
        .ok()?;

        // Add comma if not first element
        if elem_idx > 0 {
            let comma_fmt = ctx.const_string(",");
            ctx.builder
                .build_call(sprintf, &[write_pos.into(), comma_fmt.into()], "")
                .ok();

            // Update position
            let current_len = ctx
                .builder
                .build_call(strlen, &[buffer.into()], "cur_len2")
                .ok()?
                .try_as_basic_value()
                .basic()?
                .into_int_value();
            let write_pos = unsafe {
                ctx.builder
                    .build_gep(i8_type, buffer, &[current_len], "write_pos2")
            }
            .ok()?;
        }

        // Get the enum value from temps
        let enum_val = ctx.get_temp(temp_name)?;
        let struct_val = if enum_val.is_struct_value() {
            enum_val.into_struct_value()
        } else {
            continue;
        };

        // Extract tag
        let tag = ctx
            .builder
            .build_extract_value(struct_val, 0, "mixed_tag")
            .ok()?
            .into_int_value();

        // Get current position for writing
        let current_len = ctx
            .builder
            .build_call(strlen, &[buffer.into()], "cur_len3")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();
        let write_pos = unsafe {
            ctx.builder
                .build_gep(i8_type, buffer, &[current_len], "write_pos3")
        }
        .ok()?;

        // Generate switch for this element's variants
        let current_block = ctx.builder.get_insert_block()?;
        let target_fn = current_block.get_parent()?;
        let merge_block = ctx
            .context
            .append_basic_block(target_fn, &format!("mixed_merge_{}", elem_idx));
        let default_block = ctx
            .context
            .append_basic_block(target_fn, &format!("mixed_default_{}", elem_idx));

        // Default: write "Unknown"
        ctx.builder.position_at_end(default_block);
        let unknown_fmt = ctx.const_string("\"Unknown\"");
        ctx.builder
            .build_call(sprintf, &[write_pos.into(), unknown_fmt.into()], "")
            .ok();
        ctx.builder.build_unconditional_branch(merge_block).ok();

        // Cases for each variant
        let mut cases = Vec::new();
        for (variant_name, variant_idx) in variants {
            let case_block = ctx.context.append_basic_block(
                target_fn,
                &format!("mixed_case_{}_{}", elem_idx, variant_name),
            );
            ctx.builder.position_at_end(case_block);

            let variant_fmt = ctx.const_string(&format!("\"{}\"", variant_name));
            ctx.builder
                .build_call(sprintf, &[write_pos.into(), variant_fmt.into()], "")
                .ok();
            ctx.builder.build_unconditional_branch(merge_block).ok();

            cases.push((
                ctx.context.i32_type().const_int(*variant_idx as u64, false),
                case_block,
            ));
        }

        // Build switch
        ctx.builder.position_at_end(current_block);
        ctx.builder.build_switch(tag, default_block, &cases).ok();

        // Continue from merge block
        ctx.builder.position_at_end(merge_block);
    }

    // Append "]"
    let current_len = ctx
        .builder
        .build_call(strlen, &[buffer.into()], "final_len")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_int_value();
    let write_pos = unsafe {
        ctx.builder
            .build_gep(i8_type, buffer, &[current_len], "final_pos")
    }
    .ok()?;
    let close_bracket = ctx.const_string("]");
    ctx.builder
        .build_call(sprintf, &[write_pos.into(), close_bracket.into()], "")
        .ok();

    Some(buffer)
}

/// Emit an FFI call with proper type handling.
fn emit_ffi_call<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    dest: Option<&str>,
    symbol: &str,
    args: &[MirOperand],
) -> Option<BasicValueEnum<'ctx>> {
    if std::env::var("DOO_DEBUG").is_ok() {
        doo_debug!(
            "CODEGEN",
            "FfiCall: {} with {} args -> {:?}",
            symbol,
            args.len(),
            dest
        );
    }

    // Declare FFI function if not already declared
    let func = declare_ffi_function(ctx, symbol, args.len());

    // Get expected param types from signature (for conversion)
    let expected_types: Vec<Option<&str>> =
        if let Some((param_strs, _, _)) = get_ffi_signature(symbol) {
            param_strs.iter().map(|s| Some(*s)).collect()
        } else {
            args.iter().map(|_| None).collect()
        };

    // Special handling for auth/crud: register struct/enum metadata before calling
    // This is needed so the FFI can validate incoming data at runtime
    if symbol == "doo_http_auth" || symbol == "doo_http_crud" {
        emit_struct_metadata_registration_for_auth_crud(ctx, symbol, args);
    }

    // Special handling for *_with_middleware: register user-defined middleware functions
    // The middleware names are passed as comma-separated string, we need to register each one
    // IMPORTANT: Skip built-in middlewares (jwt, cors, etc.) as they have native handlers in the runtime
    if symbol.ends_with("_with_middleware") && args.len() >= 4 {
        // arg[2] is the middleware names string (e.g., "AuthMiddleware,AdminMiddleware")
        if let MirOperand::Const(MirConst::Str(middleware_str)) = &args[2] {
            // Split by comma and register each middleware function
            for mw_name in middleware_str.split(',').map(|s| s.trim()) {
                if !mw_name.is_empty() {
                    // Skip built-in middlewares - they register themselves in the runtime
                    if ffi_names::is_builtin_middleware(mw_name) {
                        if std::env::var("DOO_DEBUG").is_ok() {
                            doo_debug!(
                                "CODEGEN",
                                "Skipping built-in middleware registration: {}",
                                mw_name
                            );
                        }
                        continue;
                    }

                    // Generate wrapper for user-defined middleware and register it
                    let wrapper = get_or_generate_handler_wrapper(ctx, mw_name, symbol);

                    // Call doo_http_register_middleware(name, fn_ptr)
                    let register_fn = ctx
                        .module
                        .get_function("doo_http_register_middleware")
                        .unwrap_or_else(|| {
                            let ptr_type = ctx.ptr_type();
                            let fn_type = ctx
                                .context
                                .void_type()
                                .fn_type(&[ptr_type.into(), ptr_type.into()], false);
                            ctx.module
                                .add_function("doo_http_register_middleware", fn_type, None)
                        });

                    let mw_name_str = ctx.const_string(mw_name);
                    let _ = ctx.builder.build_call(
                        register_fn,
                        &[
                            mw_name_str.into(),
                            wrapper.as_global_value().as_pointer_value().into(),
                        ],
                        "register_mw",
                    );

                    if std::env::var("DOO_DEBUG").is_ok() {
                        doo_debug!(
                            "CODEGEN",
                            "Registered user middleware: {} -> {}",
                            mw_name,
                            wrapper.get_name().to_string_lossy()
                        );
                    }
                }
            }
        }
    }

    // Extract route context for handler wrapper generation
    // For HTTP route registrations, we need to know:
    // - Route path pattern (args[1]) to extract path param names
    // - Middleware names (args[2] for *_with_middleware) to detect JWT
    // - HTTP method (from symbol name)
    let route_context = extract_route_context(symbol, args);

    // Convert arguments - with automatic wrapper generation for FuncRef
    let mut arg_vals: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::with_capacity(args.len());
    for (i, a) in args.iter().enumerate() {
        // Special handling for FuncRef - generate wrapper if needed
        if let MirOperand::FuncRef(func_name) = a {
            let wrapper = get_or_generate_handler_wrapper_with_context(
                ctx,
                func_name,
                symbol,
                &route_context,
            );

            // If this is an HTTP route registration, register handler metadata
            // Check for doo_http_get_fn, doo_http_post_fn, etc. AND *_with_middleware variants
            let is_route_registration = symbol.starts_with("doo_http_")
                && (symbol.ends_with("_fn") || symbol.ends_with("_with_middleware"));
            if is_route_registration {
                emit_handler_metadata_registration(ctx, func_name, &wrapper);
            }

            arg_vals.push(wrapper.as_global_value().as_pointer_value().into());
            continue;
        }

        // Special handling for doo_db_raw_param: convert enum/array params (index 2) to JSON string
        if symbol == "doo_db_raw_param" && i == 2 {
            // Check for empty array literal first - pass "[]" directly
            // Empty arrays are tracked in array_element_types but NOT in array_element_temps
            // (because they have no element temps to track)
            if let MirOperand::Temp(name) = a {
                let has_elem_type = ctx.array_element_types.contains_key(name.as_str());
                let has_elem_temps = ctx.array_element_temps.contains_key(name.as_str());

                if std::env::var("DOO_DEBUG").is_ok() {
                    doo_debug!(
                        "CODEGEN",
                        "doo_db_raw_param arg[2]: temp={}, has_elem_type={}, has_elem_temps={}",
                        name,
                        has_elem_type,
                        has_elem_temps
                    );
                }

                // If it's tracked as an array (has element type) but has no element temps,
                // it's an empty array - pass "[]" directly
                if has_elem_type && !has_elem_temps {
                    if std::env::var("DOO_DEBUG").is_ok() {
                        doo_debug!("CODEGEN", "Converting empty array {} to JSON \"[]\"", name);
                    }
                    let empty_json = ctx.const_string("[]");
                    arg_vals.push(empty_json.into());
                    continue;
                }
            } else if std::env::var("DOO_DEBUG").is_ok() {
                doo_debug!("CODEGEN", "doo_db_raw_param arg[2] is not a Temp: {:?}", a);
            }

            // Try single enum conversion first
            if let Some(converted) = try_convert_enum_to_json_string(ctx, a) {
                arg_vals.push(converted.into());
                continue;
            }
            // Try array of enums conversion
            if let Some(converted) = try_convert_enum_array_to_json_string(ctx, a) {
                arg_vals.push(converted.into());
                continue;
            }
        }

        if let Some(val) = operand_to_value(ctx, a) {
            let expected = expected_types.get(i).copied().flatten();
            arg_vals.push(convert_to_ffi_arg(ctx, val, expected));
        }
    }

    // Build call
    let call_site = ctx.builder.build_call(func, &arg_vals, "ffi_call").ok()?;

    // Handle return value
    if let Some(dest_name) = dest {
        if let Some(ret_val) = call_site.try_as_basic_value().basic() {
            ctx.set_temp(dest_name, ret_val);
            return Some(ret_val);
        }
    }

    // For void functions, return None
    call_site.try_as_basic_value().basic()
}

/// Extract route context from FFI call arguments.
/// For HTTP route registrations like doo_http_get_fn(server, path, handler),
/// this extracts the route path pattern and middleware information.
fn extract_route_context(symbol: &str, args: &[MirOperand]) -> RouteContext {
    let mut ctx = RouteContext::default();

    // Only process HTTP route registrations
    if !symbol.starts_with("doo_http_") {
        return ctx;
    }

    // Extract HTTP method from symbol name
    ctx.http_method = if symbol.contains("_get") {
        Some("GET".to_string())
    } else if symbol.contains("_post") {
        Some("POST".to_string())
    } else if symbol.contains("_put") {
        Some("PUT".to_string())
    } else if symbol.contains("_delete") {
        Some("DELETE".to_string())
    } else if symbol.contains("_patch") {
        Some("PATCH".to_string())
    } else {
        None
    };

    // For route registrations: args[1] is the path pattern
    // doo_http_get_fn(server, path, handler)
    // doo_http_get_with_middleware(server, path, middleware, handler)
    if args.len() >= 2 {
        if let MirOperand::Const(MirConst::Str(path)) = &args[1] {
            ctx.route_path = Some(path.clone());
        }
    }

    // For middleware variants: args[2] is the middleware names
    if symbol.ends_with("_with_middleware") && args.len() >= 3 {
        if let MirOperand::Const(MirConst::Str(middleware_str)) = &args[2] {
            ctx.middleware_names = middleware_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }

    ctx
}

/// Generate or retrieve a wrapper function that adapts a user handler to FFI signature.
///
/// This is the COMPILER MAGIC that allows any handler signature to work with FFI.
///
/// FFI expects: extern "C" fn(*const DooRequest) -> *mut DooResult
/// User might have: fn() -> Str, fn(Request) -> Response, etc.
///
/// The wrapper:
/// 1. Has the FFI-expected signature
/// 2. Calls the user's function with appropriate arguments
/// 3. Wraps the return value in DooResult format
fn get_or_generate_handler_wrapper<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    user_func_name: &str,
    ffi_symbol: &str,
) -> FunctionValue<'ctx> {
    // Delegate to context-aware version with empty context
    get_or_generate_handler_wrapper_with_context(
        ctx,
        user_func_name,
        ffi_symbol,
        &RouteContext::default(),
    )
}

/// Generate or retrieve a wrapper function with route context.
/// This version knows about the route pattern and middleware, allowing correct
/// parameter extraction from path params, JWT claims, etc.
fn get_or_generate_handler_wrapper_with_context<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    user_func_name: &str,
    ffi_symbol: &str,
    route_context: &RouteContext,
) -> FunctionValue<'ctx> {
    let wrapper_name = format!("__ffi_wrapper_{}", user_func_name);

    // Check if wrapper already exists
    if let Some(existing) = ctx.get_function(&wrapper_name) {
        return existing;
    }

    let debug = std::env::var("DOO_DEBUG").is_ok();
    if debug {
        doo_debug!(
            "CODEGEN",
            "Generating FFI wrapper for {} (used by {})",
            user_func_name,
            ffi_symbol
        );
    }

    // Check if this is an FFI function that needs to be called via its external symbol
    let ffi_symbol_info = ctx
        .get_ffi_symbol(user_func_name)
        .map(|(_, sym)| sym.to_string());

    // Get the user's function (or the FFI external function)
    let user_func = if let Some(ref ext_symbol) = ffi_symbol_info {
        // FFI function: get or declare the external symbol
        ctx.module.get_function(ext_symbol).unwrap_or_else(|| {
            // Declare the external function with the expected signature
            let ptr_type = ctx.ptr_type();
            // For simple FFI functions like jwt() -> Str, we use a simple signature
            let fn_type = ptr_type.fn_type(&[], false);
            ctx.module.add_function(
                ext_symbol,
                fn_type,
                Some(inkwell::module::Linkage::External),
            )
        })
    } else {
        // User-defined function: get it directly
        match ctx.get_function(user_func_name) {
            Some(f) => f,
            None => {
                // Function not found - create a dummy wrapper that returns null
                doo_debug!(
                    "CODEGEN",
                    "Warning: Function {} not found for wrapper generation",
                    user_func_name
                );
                return create_dummy_wrapper(ctx, &wrapper_name);
            }
        }
    };

    // Check if return type is a struct (not a primitive)
    let return_type_id = ctx.get_function_return_type(user_func_name);
    let return_type_name = return_type_id.and_then(|tid| {
        ctx.get_type_kind(tid).map(|tk| match tk {
            TypeKind::Str => "Str".to_string(),
            TypeKind::Int => "Int".to_string(),
            TypeKind::Float => "Float".to_string(),
            TypeKind::Bool => "Bool".to_string(),
            TypeKind::Void => "Void".to_string(),
            TypeKind::Struct { name, .. } => name.clone(),
            TypeKind::Enum { name, .. } => name.clone(),
            TypeKind::Array { .. } => "Array".to_string(),
            _ => "Unknown".to_string(),
        })
    });

    let returns_struct = return_type_name.as_ref().map_or(false, |name: &String| {
        !matches!(
            name.as_str(),
            "Str" | "Int" | "Float" | "Bool" | "Void" | "Array" | "Unknown"
        )
    });

    // Analyze user function signature
    let user_fn_type = user_func.get_type();
    let user_param_count = user_fn_type.count_param_types();
    let user_return_type = user_fn_type.get_return_type();

    // Check if this is a middleware function (2 params with second being "Next" type)
    // For FFI functions, we need to check the original function's param types, not the FFI wrapper
    let param_type_ids = ctx.get_function_param_types(user_func_name);
    let all_param_types: Vec<doo_core::types::TypeId> = param_type_ids
        .map(|types| types.to_vec())
        .unwrap_or_default();

    let is_middleware = user_param_count == 2 && all_param_types.len() == 2 && {
        // Check if second param is "Next" type
        all_param_types
            .get(1)
            .map_or(false, |tid| match ctx.get_type_kind(*tid) {
                Some(doo_core::types::TypeKind::Struct { name, .. }) => {
                    name == "Next" || name == "DooNext"
                }
                _ => false,
            })
    };

    // For FFI functions, check if it's a middleware based on the ffi_symbol being passed to
    // (e.g., doo_http_get_with_middleware means this is used as middleware)
    // BUT only if the FFI function actually has the middleware signature (2 params)
    let is_ffi_middleware = ffi_symbol_info.is_some()
        && ffi_symbol.ends_with("_with_middleware")
        && user_param_count == 2;

    if debug {
        doo_debug!("CODEGEN", "User function {} has {} params, returns {:?}, return_type_name={:?}, returns_struct={}, is_middleware={}, is_ffi={}",
            user_func_name, user_param_count, user_return_type, return_type_name, returns_struct, is_middleware, ffi_symbol_info.is_some());
    }

    // Create wrapper function with FFI signature:
    // - Handlers: fn(ptr) -> ptr
    // - Middleware (true middleware with 2 params): fn(ptr, fn_ptr) -> ptr (request + next function)
    let ptr_type = ctx.ptr_type();
    let wrapper_fn_type = if is_middleware || is_ffi_middleware {
        ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false)
    } else {
        ptr_type.fn_type(&[ptr_type.into()], false)
    };
    let wrapper_fn = ctx
        .module
        .add_function(&wrapper_name, wrapper_fn_type, None);

    // Save current position
    let current_block = ctx.builder.get_insert_block();

    // Create wrapper body
    let entry = ctx.context.append_basic_block(wrapper_fn, "entry");
    ctx.builder.position_at_end(entry);

    // Get the request parameter
    let request_ptr = wrapper_fn.get_nth_param(0).unwrap().into_pointer_value();

    // Types we'll need
    let i32_type = ctx.i32_type();
    let i64_type = ctx.i64_type();
    let i8_type = ctx.context.i8_type();

    // Allocate result on heap (we'll need this for both success and error paths)
    let result_struct_type = ctx
        .context
        .struct_type(&[i32_type.into(), ptr_type.into(), i8_type.into()], false);

    let malloc_fn = ctx.module.get_function("malloc").unwrap_or_else(|| {
        let fn_type = ptr_type.fn_type(&[i64_type.into()], false);
        ctx.module.add_function("malloc", fn_type, None)
    });

    // Call the user's function (or FFI function via external symbol)
    // For FFI functions, use their actual parameter count, not the middleware signature
    let user_result = if ffi_symbol_info.is_some() {
        // FFI function: call with the function's actual signature
        // For functions like jwt() that take no params, call with no args
        if user_param_count == 0 {
            ctx.builder
                .build_call(user_func, &[], "user_call")
                .ok()
                .and_then(|cs| cs.try_as_basic_value().basic())
        } else if user_param_count == 1 {
            // FFI function with 1 param (e.g., request)
            ctx.builder
                .build_call(user_func, &[request_ptr.into()], "user_call")
                .ok()
                .and_then(|cs| cs.try_as_basic_value().basic())
        } else if user_param_count == 2 && (is_middleware || is_ffi_middleware) {
            // FFI middleware with 2 params (request, next)
            let next_fn_ptr = wrapper_fn.get_nth_param(1).unwrap().into_pointer_value();
            ctx.builder
                .build_call(
                    user_func,
                    &[request_ptr.into(), next_fn_ptr.into()],
                    "user_call",
                )
                .ok()
                .and_then(|cs| cs.try_as_basic_value().basic())
        } else {
            // Fallback: try with request only
            ctx.builder
                .build_call(user_func, &[request_ptr.into()], "user_call")
                .ok()
                .and_then(|cs| cs.try_as_basic_value().basic())
        }
    } else if is_middleware {
        // Middleware function: fn(Request, Next) -> Response
        // Get the next function pointer from wrapper's second param
        let next_fn_ptr = wrapper_fn.get_nth_param(1).unwrap().into_pointer_value();

        // Call user's middleware with both request and next
        ctx.builder
            .build_call(
                user_func,
                &[request_ptr.into(), next_fn_ptr.into()],
                "user_call",
            )
            .ok()
            .and_then(|cs| cs.try_as_basic_value().basic())
    } else if user_param_count == 0 {
        // Simple handler: fn() -> Str - no validation needed
        ctx.builder
            .build_call(user_func, &[], "user_call")
            .ok()
            .and_then(|cs| cs.try_as_basic_value().basic())
    } else {
        // Handler with struct parameter: validate request body first
        // Get or declare doohttp_populate_struct_from_request
        let populate_fn = ctx
            .module
            .get_function("doohttp_populate_struct_from_request")
            .unwrap_or_else(|| {
                let fn_type = i32_type.fn_type(
                    &[
                        ptr_type.into(),
                        ptr_type.into(),
                        i32_type.into(),
                        ptr_type.into(),
                    ],
                    false,
                );
                ctx.module
                    .add_function("doohttp_populate_struct_from_request", fn_type, None)
            });

        // Get handler name as C string for the validation call
        let handler_name_str = ctx.const_string(user_func_name);

        // Call populate_struct_from_request to validate the body
        // Arguments: request_ptr, struct_ptr (null - we just want validation), source_type (0=body), handler_name
        let validate_result = ctx
            .builder
            .build_call(
                populate_fn,
                &[
                    request_ptr.into(),
                    ptr_type.const_null().into(), // struct_ptr - null since we just want validation
                    i32_type.const_int(0, false).into(), // source_type = 0 (body)
                    handler_name_str.into(),
                ],
                "validate_result",
            )
            .ok()
            .and_then(|cs| cs.try_as_basic_value().basic())
            .map(|v| v.into_int_value())
            .unwrap_or_else(|| i32_type.const_int(0, false));

        // Check if validation failed (non-zero = error)
        let validation_failed = ctx
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                validate_result,
                i32_type.const_zero(),
                "validation_failed",
            )
            .ok();

        if let Some(validation_failed) = validation_failed {
            // Create error and success blocks
            let parent = ctx
                .builder
                .get_insert_block()
                .unwrap()
                .get_parent()
                .unwrap();
            let error_block = ctx.context.append_basic_block(parent, "validation_error");
            let success_block = ctx.context.append_basic_block(parent, "validation_success");

            ctx.builder
                .build_conditional_branch(validation_failed, error_block, success_block)
                .ok();

            // Error block: return RFC 7807 error from last_error
            ctx.builder.position_at_end(error_block);

            // Get error status and JSON
            let get_status_fn = ctx
                .module
                .get_function("doohttp_last_error_status")
                .unwrap_or_else(|| {
                    let fn_type = i32_type.fn_type(&[], false);
                    ctx.module
                        .add_function("doohttp_last_error_status", fn_type, None)
                });

            let get_json_fn = ctx
                .module
                .get_function("doohttp_last_error_json")
                .unwrap_or_else(|| {
                    let fn_type = ptr_type.fn_type(&[], false);
                    ctx.module
                        .add_function("doohttp_last_error_json", fn_type, None)
                });

            let error_status = ctx
                .builder
                .build_call(get_status_fn, &[], "error_status")
                .ok()
                .and_then(|cs| cs.try_as_basic_value().basic())
                .map(|v| v.into_int_value())
                .unwrap_or_else(|| i32_type.const_int(400, false));

            let error_json = ctx
                .builder
                .build_call(get_json_fn, &[], "error_json")
                .ok()
                .and_then(|cs| cs.try_as_basic_value().basic())
                .map(|v| v.into_pointer_value())
                .unwrap_or_else(|| ptr_type.const_null());

            // Build error response struct { status, body, content_type }
            let error_response_type = ctx
                .context
                .struct_type(&[i32_type.into(), ptr_type.into(), ptr_type.into()], false);
            let error_response_size = i64_type.const_int(24, false);
            let error_response_ptr = ctx
                .builder
                .build_call(malloc_fn, &[error_response_size.into()], "error_response")
                .ok()
                .and_then(|cs| cs.try_as_basic_value().basic())
                .map(|v| v.into_pointer_value())
                .unwrap_or_else(|| ptr_type.const_null());

            // Set status
            if let Ok(status_ptr) = ctx.builder.build_struct_gep(
                error_response_type,
                error_response_ptr,
                0,
                "status_ptr",
            ) {
                let _ = ctx.builder.build_store(status_ptr, error_status);
            }
            // Set body
            if let Ok(body_ptr) =
                ctx.builder
                    .build_struct_gep(error_response_type, error_response_ptr, 1, "body_ptr")
            {
                let _ = ctx.builder.build_store(body_ptr, error_json);
            }
            // Set content_type (application/json)
            let json_content_type = ctx.const_string("application/json");
            if let Ok(ct_ptr) =
                ctx.builder
                    .build_struct_gep(error_response_type, error_response_ptr, 2, "ct_ptr")
            {
                let _ = ctx.builder.build_store(ct_ptr, json_content_type);
            }

            // Build DooResult for error: { tag=1, value=error_response, owner=1 }
            let result_size = i64_type.const_int(24, false);
            let error_result_ptr = ctx
                .builder
                .build_call(malloc_fn, &[result_size.into()], "error_result")
                .ok()
                .and_then(|cs| cs.try_as_basic_value().basic())
                .map(|v| v.into_pointer_value())
                .unwrap_or_else(|| ptr_type.const_null());

            if let Ok(tag_ptr) = ctx.builder.build_struct_gep(
                result_struct_type,
                error_result_ptr,
                0,
                "error_tag_ptr",
            ) {
                let _ = ctx
                    .builder
                    .build_store(tag_ptr, i32_type.const_int(1, false)); // tag = 1 (error)
            }
            if let Ok(value_ptr) = ctx.builder.build_struct_gep(
                result_struct_type,
                error_result_ptr,
                1,
                "error_value_ptr",
            ) {
                let _ = ctx.builder.build_store(value_ptr, error_response_ptr);
            }
            if let Ok(owner_ptr) = ctx.builder.build_struct_gep(
                result_struct_type,
                error_result_ptr,
                2,
                "error_owner_ptr",
            ) {
                let _ = ctx
                    .builder
                    .build_store(owner_ptr, i8_type.const_int(1, false)); // owner = 1 (FFI)
            }

            let _ = ctx.builder.build_return(Some(&error_result_ptr));

            // Success block: call the user's function
            ctx.builder.position_at_end(success_block);
        }

        // Get ALL parameter types of the user function
        let param_type_ids = ctx.get_function_param_types(user_func_name);
        let all_param_types: Vec<doo_core::types::TypeId> = param_type_ids
            .map(|types| types.to_vec())
            .unwrap_or_default();
        let first_param_type = all_param_types.first().copied();

        // Check if the first parameter is a special "Request" type that receives raw pointer
        let is_raw_request = first_param_type.map_or(false, |tid| match ctx.get_type_kind(tid) {
            Some(doo_core::types::TypeKind::Struct { name, .. }) => {
                name == "Request" || name == "DooRequest"
            }
            _ => false,
        });

        if is_raw_request || first_param_type.is_none() {
            // User function expects raw request pointer
            ctx.builder
                .build_call(user_func, &[request_ptr.into()], "user_call")
                .ok()
                .and_then(|cs| cs.try_as_basic_value().basic())
        } else {
            // User function expects parsed struct(s) - handle single and multi-param cases
            // DooRequest layout: { *method, *path, *body, *headers, *params, *query, *user_id }
            let doo_request_type = ctx.context.struct_type(
                &[
                    ptr_type.into(), // 0: method
                    ptr_type.into(), // 1: path
                    ptr_type.into(), // 2: body
                    ptr_type.into(), // 3: headers
                    ptr_type.into(), // 4: params (path params as JSON)
                    ptr_type.into(), // 5: query
                    ptr_type.into(), // 6: user_id (JWT claims)
                ],
                false,
            );

            // Helper to load a field from request by index
            let load_request_field =
                |ctx: &mut CodegenContext<'ctx>, index: u32, name: &str| -> PointerValue<'ctx> {
                    ctx.builder
                        .build_struct_gep(
                            doo_request_type,
                            request_ptr,
                            index,
                            &format!("{}_field_ptr", name),
                        )
                        .ok()
                        .and_then(|gep| ctx.builder.build_load(ptr_type, gep, name).ok())
                        .map(|v| v.into_pointer_value())
                        .unwrap_or_else(|| ptr_type.const_null())
                };

            // Build arguments for the user function call
            let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();
            let param_count = all_param_types.len();

            // Get path param names from route context
            let path_param_names = route_context.path_param_names();
            let has_jwt_middleware = route_context.has_jwt_middleware();

            if debug {
                doo_debug!(
                    "CODEGEN",
                    "Handler {} with {} params, path_params={:?}, jwt={}",
                    user_func_name,
                    param_count,
                    path_param_names,
                    has_jwt_middleware
                );
            }

            // Get or declare doo_json_get_field for extracting specific fields from params JSON
            let json_get_field_fn = ctx
                .module
                .get_function(ffi_names::DOO_JSON_GET_FIELD)
                .unwrap_or_else(|| {
                    let fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                    ctx.module
                        .add_function(ffi_names::DOO_JSON_GET_FIELD, fn_type, None)
                });

            for (idx, param_type) in all_param_types.iter().enumerate() {
                // Determine the correct source field for this parameter:
                // 1. JWT middleware + single Int param -> user_id (index 6)
                // 2. Path param match -> params (index 4), then extract specific field
                // 3. Otherwise -> body (index 2)

                let is_int_param = ctx
                    .get_type_kind(*param_type)
                    .map(|k| matches!(k, TypeKind::Int))
                    .unwrap_or(false);

                let source_ptr = if has_jwt_middleware && param_count == 1 && is_int_param {
                    // JWT handler with single Int param - get from user_id field
                    if debug {
                        doo_debug!("CODEGEN", "Param {} from user_id (JWT)", idx);
                    }
                    load_request_field(ctx, 6, "user_id")
                } else if idx < path_param_names.len() {
                    // This param corresponds to a path parameter - need to extract the specific field
                    let path_param_name = path_param_names.get(idx).cloned().unwrap_or_default();
                    if debug {
                        doo_debug!(
                            "CODEGEN",
                            "Param {} from params (path param: {})",
                            idx,
                            path_param_name
                        );
                    }
                    let params_json = load_request_field(ctx, 4, "params");

                    // Extract the specific field from the params JSON object
                    // params is like {"authorId": "1"}, we need to extract "authorId" value
                    let field_name_str = ctx.const_string(&path_param_name);
                    let field_value = ctx
                        .builder
                        .build_call(
                            json_get_field_fn,
                            &[params_json.into(), field_name_str.into()],
                            "field_json",
                        )
                        .ok()
                        .and_then(|cs| cs.try_as_basic_value().basic())
                        .map(|v| v.into_pointer_value())
                        .unwrap_or_else(|| ptr_type.const_null());

                    field_value
                } else {
                    // Default: use body
                    if debug {
                        doo_debug!("CODEGEN", "Param {} from body", idx);
                    }
                    load_request_field(ctx, 2, "body_json")
                };

                let parsed = JsonBuiltins::emit_parse(ctx, source_ptr.into(), Some(*param_type));

                if let Some(val) = parsed {
                    call_args.push(val.into());
                } else {
                    // Fallback: pass null pointer for this param
                    if debug {
                        doo_debug!(
                            "CODEGEN",
                            "Warning: Failed to parse param {} for {}",
                            idx,
                            user_func_name
                        );
                    }
                    call_args.push(ptr_type.const_null().into());
                }
            }

            // Call user function with all parsed arguments
            if call_args.len() == param_count {
                ctx.builder
                    .build_call(user_func, &call_args, "user_call")
                    .ok()
                    .and_then(|cs| cs.try_as_basic_value().basic())
            } else {
                // Fallback to passing request_ptr if parsing fails
                if debug {
                    doo_debug!(
                        "CODEGEN",
                        "Warning: Param count mismatch for {}, expected {} got {}",
                        user_func_name,
                        param_count,
                        call_args.len()
                    );
                }
                None
            }
        }
    };

    // For middleware, we need to wrap the result in DooResult format
    // Middleware can return either:
    // - Response directly (ptr) -> wrap in DooResult { tag=0, value=response }
    // - Result<Response, Error> ({ i32, ptr }) -> extract and rewrap in DooResult
    if is_middleware {
        // Check if the LLVM return type is a struct { i32, ptr } which indicates Result type
        // This is more reliable than checking return_type_name since that might not capture Result wrapper
        let user_returns_result_struct = user_return_type
            .map(|rt| {
                if let inkwell::types::BasicTypeEnum::StructType(st) = rt {
                    // Result<T, E> is represented as { i32 tag, ptr value }
                    st.count_fields() == 2
                } else {
                    false
                }
            })
            .unwrap_or(false);

        // Get the error type info for this middleware function (if it returns Result)
        let error_type_id = ctx.get_function_error_type(user_func_name);
        let error_type_name = error_type_id.and_then(|tid| {
            ctx.get_type_kind(tid).map(|tk| match tk {
                TypeKind::Enum { name, .. } => name.clone(),
                _ => String::new(),
            })
        });

        if debug {
            doo_debug!(
                "CODEGEN",
                "Middleware {} user_returns_result_struct={} error_type={:?}",
                user_func_name,
                user_returns_result_struct,
                error_type_name
            );
        }

        // Allocate DooResult on heap
        let result_size = i64_type.const_int(24, false);
        let doo_result_ptr = ctx
            .builder
            .build_call(malloc_fn, &[result_size.into()], "doo_result")
            .ok()
            .and_then(|cs| cs.try_as_basic_value().basic())
            .map(|v| v.into_pointer_value())
            .unwrap_or_else(|| ptr_type.const_null());

        if let Some(val) = user_result {
            if debug {
                doo_debug!(
                    "CODEGEN",
                    "Middleware {} val.is_struct_value()={}, user_returns_result_struct={}",
                    user_func_name,
                    val.is_struct_value(),
                    user_returns_result_struct
                );
            }

            // If the function returns a struct type { i64, i64 } (SimpleResult), we need to extract values
            // Try to convert to struct value if the return type indicates it's a result struct
            if user_returns_result_struct && error_type_name.is_some() {
                // The call returns { i64, i64 } directly as a struct value
                // We need to extract the tag and value from it
                if let Ok(user_result_struct) = val.try_into() {
                    let user_result_struct: inkwell::values::StructValue = user_result_struct;

                    // Extract i64 tag
                    let tag = ctx
                        .builder
                        .build_extract_value(user_result_struct, 0, "result_tag")
                        .map(|v| v.into_int_value())
                        .unwrap_or_else(|_| i64_type.const_int(0, false));

                    // Extract i64 value and convert to pointer
                    let value_i64 = ctx
                        .builder
                        .build_extract_value(user_result_struct, 1, "result_value_i64")
                        .map(|v| v.into_int_value())
                        .unwrap_or_else(|_| i64_type.const_zero());
                    let value = ctx
                        .builder
                        .build_int_to_ptr(value_i64, ptr_type, "result_value")
                        .unwrap_or_else(|_| ptr_type.const_null());

                    // Create blocks for Ok and Err paths
                    let parent = ctx
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let ok_block = ctx.context.append_basic_block(parent, "middleware_ok");
                    let err_block = ctx.context.append_basic_block(parent, "middleware_err");
                    let merge_block = ctx.context.append_basic_block(parent, "middleware_merge");

                    // Branch based on tag (0 = Ok, non-zero = Err) - use i64 constant
                    let is_err = ctx
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::NE,
                            tag,
                            i64_type.const_zero(),
                            "is_err",
                        )
                        .unwrap();
                    ctx.builder
                        .build_conditional_branch(is_err, err_block, ok_block)
                        .ok();

                    // OK PATH: Extract Body field from Response struct
                    // Response struct layout: { i64 Status, ptr Body, ptr ContentType }
                    ctx.builder.position_at_end(ok_block);
                    let response_struct_type = ctx
                        .context
                        .struct_type(&[i64_type.into(), ptr_type.into(), ptr_type.into()], false);

                    // Load the Body field (index 1) from the Response struct pointer
                    let body_field_ptr = ctx
                        .builder
                        .build_struct_gep(response_struct_type, value, 1, "body_field_ptr")
                        .ok()
                        .and_then(|gep| ctx.builder.build_load(ptr_type, gep, "response_body").ok())
                        .map(|v| v.into_pointer_value())
                        .unwrap_or_else(|| ptr_type.const_null());

                    // Store in DooResult with tag=0 (Ok)
                    // The value should be the JSON body string from the Response.Body field
                    if let Ok(tag_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        0,
                        "ok_tag_ptr",
                    ) {
                        let _ = ctx.builder.build_store(tag_ptr, i32_type.const_zero());
                    }
                    if let Ok(value_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        1,
                        "ok_value_ptr",
                    ) {
                        let _ = ctx.builder.build_store(value_ptr, body_field_ptr);
                    }
                    if let Ok(owner_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        2,
                        "ok_owner_ptr",
                    ) {
                        let _ = ctx
                            .builder
                            .build_store(owner_ptr, i8_type.const_int(1, false));
                    }
                    ctx.builder.build_unconditional_branch(merge_block).ok();

                    // ERROR PATH: Map error enum variant to HTTP status and build error response
                    ctx.builder.position_at_end(err_block);

                    // Get error enum name and variant index from the value pointer
                    // The value pointer points to a struct { i32 variant_index, ptr payload }
                    let error_struct_type = ctx
                        .context
                        .struct_type(&[i32_type.into(), ptr_type.into()], false);

                    let variant_index = ctx
                        .builder
                        .build_struct_gep(error_struct_type, value, 0, "variant_idx_ptr")
                        .ok()
                        .and_then(|gep| ctx.builder.build_load(i32_type, gep, "variant_idx").ok())
                        .map(|v| v.into_int_value())
                        .unwrap_or_else(|| i32_type.const_zero());

                    // Get enum metadata to get variant names
                    let enum_name_str = error_type_name.as_ref().unwrap();
                    let variant_names = if let Some(TypeKind::Enum { variants, .. }) =
                        error_type_id.and_then(|tid| ctx.get_type_kind(tid))
                    {
                        variants
                            .iter()
                            .map(|(name, _)| name.clone())
                            .collect::<Vec<_>>()
                    } else {
                        vec!["Unknown".to_string()]
                    };

                    // Build variant name lookup using a simple select/switch approach
                    // For enums with 2 variants (like AuthError), just use a select instruction
                    // For larger enums, we'd build a proper switch, but for now this is simpler
                    let variant_name_ptr = if variant_names.len() == 2 {
                        // Simple case: use select for binary choice
                        // select i1 (variant_idx == 0), ptr "Unauthorized", ptr "Forbidden"
                        let is_zero = ctx
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                variant_index,
                                i32_type.const_zero(),
                                "is_variant_0",
                            )
                            .unwrap();
                        let name0 = ctx.const_string(&variant_names[0]);
                        let name1 = ctx.const_string(&variant_names[1]);
                        ctx.builder
                            .build_select(is_zero, name0, name1, "variant_name")
                            .ok()
                            .map(|v| v.into_pointer_value())
                            .unwrap_or_else(|| ptr_type.const_null())
                    } else {
                        // Fallback: just use first variant name (should build proper switch for 3+ variants)
                        ctx.const_string(
                            &variant_names
                                .get(0)
                                .cloned()
                                .unwrap_or_else(|| "Unknown".to_string()),
                        )
                    };

                    // Call doohttp_error_variant_to_status to get HTTP status code
                    let error_mapping_fn = ctx
                        .module
                        .get_function("doohttp_error_variant_to_status")
                        .unwrap_or_else(|| {
                            let fn_type = i32_type.fn_type(
                                &[ptr_type.into(), ptr_type.into(), i32_type.into()],
                                false,
                            );
                            ctx.module.add_function(
                                "doohttp_error_variant_to_status",
                                fn_type,
                                None,
                            )
                        });

                    let enum_name_cstr = ctx.const_string(enum_name_str);

                    let http_status = ctx
                        .builder
                        .build_call(
                            error_mapping_fn,
                            &[
                                enum_name_cstr.into(),
                                variant_name_ptr.into(),
                                variant_index.into(),
                            ],
                            "http_status",
                        )
                        .ok()
                        .and_then(|cs| cs.try_as_basic_value().basic())
                        .map(|v| v.into_int_value())
                        .unwrap_or_else(|| i32_type.const_int(500, false));

                    // Get the doohttp_build_rfc7807_error function to create proper error JSON
                    let build_error_fn = ctx
                        .module
                        .get_function("doohttp_build_rfc7807_error")
                        .unwrap_or_else(|| {
                            let fn_type =
                                ptr_type.fn_type(&[i32_type.into(), ptr_type.into()], false);
                            ctx.module
                                .add_function("doohttp_build_rfc7807_error", fn_type, None)
                        });

                    // Build RFC 7807 error JSON using the helper
                    let error_json_ptr = ctx
                        .builder
                        .build_call(
                            build_error_fn,
                            &[http_status.into(), variant_name_ptr.into()],
                            "error_json",
                        )
                        .ok()
                        .and_then(|cs| cs.try_as_basic_value().basic())
                        .map(|v| v.into_pointer_value())
                        .unwrap_or_else(|| ptr_type.const_null());

                    // Build error response struct { i32 status, ptr body, ptr content_type }
                    let error_response_type = ctx
                        .context
                        .struct_type(&[i32_type.into(), ptr_type.into(), ptr_type.into()], false);
                    let error_response_size = i64_type.const_int(24, false);
                    let error_response_ptr = ctx
                        .builder
                        .build_call(malloc_fn, &[error_response_size.into()], "error_response")
                        .ok()
                        .and_then(|cs| cs.try_as_basic_value().basic())
                        .map(|v| v.into_pointer_value())
                        .unwrap_or_else(|| ptr_type.const_null());

                    if let Ok(status_ptr) = ctx.builder.build_struct_gep(
                        error_response_type,
                        error_response_ptr,
                        0,
                        "err_status_ptr",
                    ) {
                        let _ = ctx.builder.build_store(status_ptr, http_status);
                    }
                    if let Ok(body_ptr) = ctx.builder.build_struct_gep(
                        error_response_type,
                        error_response_ptr,
                        1,
                        "err_body_ptr",
                    ) {
                        let _ = ctx.builder.build_store(body_ptr, error_json_ptr);
                    }
                    let json_content_type = ctx.const_string("application/json");
                    if let Ok(ct_ptr) = ctx.builder.build_struct_gep(
                        error_response_type,
                        error_response_ptr,
                        2,
                        "err_ct_ptr",
                    ) {
                        let _ = ctx.builder.build_store(ct_ptr, json_content_type);
                    }

                    // Store in DooResult with tag=1 (Err)
                    if let Ok(tag_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        0,
                        "err_tag_ptr",
                    ) {
                        let _ = ctx
                            .builder
                            .build_store(tag_ptr, i32_type.const_int(1, false));
                    }
                    if let Ok(value_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        1,
                        "err_value_ptr",
                    ) {
                        let _ = ctx.builder.build_store(value_ptr, error_response_ptr);
                    }
                    if let Ok(owner_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        2,
                        "err_owner_ptr",
                    ) {
                        let _ = ctx
                            .builder
                            .build_store(owner_ptr, i8_type.const_int(1, false));
                    }
                    ctx.builder.build_unconditional_branch(merge_block).ok();

                    // MERGE: Return the DooResult
                    ctx.builder.position_at_end(merge_block);
                } else {
                    // Fallback: treat as pointer
                    if debug {
                        doo_debug!(
                            "CODEGEN",
                            "Warning: Failed to extract struct value for {}",
                            user_func_name
                        );
                    }
                    let response_ptr = if val.is_pointer_value() {
                        val.into_pointer_value()
                    } else {
                        ptr_type.const_null()
                    };

                    // Check if return type is Response - if so, extract Body field
                    let should_extract_body = return_type_name.as_deref() == Some("Response");

                    let result_value = if should_extract_body {
                        // Extract Body field (index 1) from Response struct
                        let response_struct_type = ctx.context.struct_type(
                            &[i64_type.into(), ptr_type.into(), ptr_type.into()],
                            false,
                        );

                        ctx.builder
                            .build_struct_gep(
                                response_struct_type,
                                response_ptr,
                                1,
                                "body_field_ptr",
                            )
                            .ok()
                            .and_then(|gep| {
                                ctx.builder.build_load(ptr_type, gep, "response_body").ok()
                            })
                            .map(|v| v.into_pointer_value())
                            .unwrap_or_else(|| ptr_type.const_null())
                    } else {
                        // Serialize the Response struct to JSON
                        let serialize_fn = ctx
                            .module
                            .get_function("doohttp_serialize_struct_to_json")
                            .unwrap_or_else(|| {
                                let fn_type =
                                    ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                                ctx.module.add_function(
                                    "doohttp_serialize_struct_to_json",
                                    fn_type,
                                    None,
                                )
                            });

                        let handler_name_str = ctx
                            .builder
                            .build_global_string_ptr(user_func_name, "middleware_name_fallback")
                            .map(|g| g.as_pointer_value())
                            .unwrap_or_else(|_| ptr_type.const_null());

                        ctx.builder
                            .build_call(
                                serialize_fn,
                                &[response_ptr.into(), handler_name_str.into()],
                                "serialized_fallback",
                            )
                            .ok()
                            .and_then(|cs| cs.try_as_basic_value().basic())
                            .map(|v| v.into_pointer_value())
                            .unwrap_or_else(|| ptr_type.const_null())
                    };

                    if let Ok(tag_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        0,
                        "tag_ptr",
                    ) {
                        let _ = ctx
                            .builder
                            .build_store(tag_ptr, i32_type.const_int(0, false));
                    }
                    if let Ok(value_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        1,
                        "value_ptr",
                    ) {
                        let _ = ctx.builder.build_store(value_ptr, result_value);
                    }
                    if let Ok(owner_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        2,
                        "owner_ptr",
                    ) {
                        let _ = ctx
                            .builder
                            .build_store(owner_ptr, i8_type.const_int(1, false));
                    }
                }
            } else {
                // User returned Response directly (pointer) - extract Body field
                // Response struct layout: { i64 Status, ptr Body, ptr ContentType }
                let response_ptr = if val.is_pointer_value() {
                    val.into_pointer_value()
                } else {
                    ptr_type.const_null()
                };

                // Check if return type name is "Response" - if so, extract Body field
                // Otherwise serialize as before (for other return types)
                let should_extract_body = return_type_name.as_deref() == Some("Response");

                let result_value = if should_extract_body {
                    // Extract Body field (index 1) from Response struct { i64, ptr, ptr }
                    let response_struct_type = ctx
                        .context
                        .struct_type(&[i64_type.into(), ptr_type.into(), ptr_type.into()], false);

                    ctx.builder
                        .build_struct_gep(response_struct_type, response_ptr, 1, "body_field_ptr")
                        .ok()
                        .and_then(|gep| ctx.builder.build_load(ptr_type, gep, "response_body").ok())
                        .map(|v| v.into_pointer_value())
                        .unwrap_or_else(|| ptr_type.const_null())
                } else {
                    // Serialize the struct to JSON (for non-Response return types)
                    let serialize_fn = ctx
                        .module
                        .get_function("doohttp_serialize_struct_to_json")
                        .unwrap_or_else(|| {
                            let fn_type =
                                ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                            ctx.module.add_function(
                                "doohttp_serialize_struct_to_json",
                                fn_type,
                                None,
                            )
                        });

                    let handler_name_str = ctx
                        .builder
                        .build_global_string_ptr(user_func_name, "middleware_name_direct")
                        .map(|g| g.as_pointer_value())
                        .unwrap_or_else(|_| ptr_type.const_null());

                    ctx.builder
                        .build_call(
                            serialize_fn,
                            &[response_ptr.into(), handler_name_str.into()],
                            "serialized_direct",
                        )
                        .ok()
                        .and_then(|cs| cs.try_as_basic_value().basic())
                        .map(|v| v.into_pointer_value())
                        .unwrap_or_else(|| ptr_type.const_null())
                };

                // Store in DooResult with tag=0 (Ok)
                if let Ok(tag_ptr) =
                    ctx.builder
                        .build_struct_gep(result_struct_type, doo_result_ptr, 0, "tag_ptr")
                {
                    let _ = ctx
                        .builder
                        .build_store(tag_ptr, i32_type.const_int(0, false));
                }
                if let Ok(value_ptr) =
                    ctx.builder
                        .build_struct_gep(result_struct_type, doo_result_ptr, 1, "value_ptr")
                {
                    let _ = ctx.builder.build_store(value_ptr, result_value);
                }
                if let Ok(owner_ptr) =
                    ctx.builder
                        .build_struct_gep(result_struct_type, doo_result_ptr, 2, "owner_ptr")
                {
                    let _ = ctx
                        .builder
                        .build_store(owner_ptr, i8_type.const_int(1, false));
                }
            }
        } else {
            // No result - return error
            if let Ok(tag_ptr) =
                ctx.builder
                    .build_struct_gep(result_struct_type, doo_result_ptr, 0, "tag_ptr")
            {
                let _ = ctx
                    .builder
                    .build_store(tag_ptr, i32_type.const_int(1, false));
            }
            if let Ok(value_ptr) =
                ctx.builder
                    .build_struct_gep(result_struct_type, doo_result_ptr, 1, "value_ptr")
            {
                let _ = ctx.builder.build_store(value_ptr, ptr_type.const_null());
            }
            if let Ok(owner_ptr) =
                ctx.builder
                    .build_struct_gep(result_struct_type, doo_result_ptr, 2, "owner_ptr")
            {
                let _ = ctx
                    .builder
                    .build_store(owner_ptr, i8_type.const_int(1, false));
            }
        }

        let _ = ctx.builder.build_return(Some(&doo_result_ptr));

        // Restore position
        if let Some(block) = current_block {
            ctx.builder.position_at_end(block);
        }

        return wrapper_fn;
    }

    // Check if handler returns a Result type (has error type in signature)
    // This handles non-middleware handlers like GetFeed that return [Post] ! DatabaseError
    let handler_error_type_id = ctx.get_function_error_type(user_func_name);
    let handler_returns_result = handler_error_type_id.is_some()
        && user_return_type.map_or(false, |rt| {
            if let inkwell::types::BasicTypeEnum::StructType(st) = rt {
                st.count_fields() == 2 // Result<T, E> is { i64 tag, i64 value }
            } else {
                false
            }
        });

    if handler_returns_result {
        // Handler returns Result<T, Error> - need to extract Ok value or handle Err
        // Allocate DooResult on heap
        let result_size = i64_type.const_int(24, false);
        let doo_result_ptr = ctx
            .builder
            .build_call(malloc_fn, &[result_size.into()], "doo_result")
            .ok()
            .and_then(|cs| cs.try_as_basic_value().basic())
            .map(|v| v.into_pointer_value())
            .unwrap_or_else(|| ptr_type.const_null());

        if let Some(val) = user_result {
            if let Ok(user_result_struct) = val.try_into() {
                let user_result_struct: inkwell::values::StructValue = user_result_struct;

                // Extract i64 tag (0 = Ok, 1 = Err)
                let tag = ctx
                    .builder
                    .build_extract_value(user_result_struct, 0, "result_tag")
                    .map(|v| v.into_int_value())
                    .unwrap_or_else(|_| i64_type.const_int(0, false));

                // Extract i64 value and convert to pointer
                let value_i64 = ctx
                    .builder
                    .build_extract_value(user_result_struct, 1, "result_value_i64")
                    .map(|v| v.into_int_value())
                    .unwrap_or_else(|_| i64_type.const_zero());
                let value = ctx
                    .builder
                    .build_int_to_ptr(value_i64, ptr_type, "result_value")
                    .unwrap_or_else(|_| ptr_type.const_null());

                // Create blocks for Ok and Err paths
                let parent = ctx
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();
                let ok_block = ctx.context.append_basic_block(parent, "handler_ok");
                let err_block = ctx.context.append_basic_block(parent, "handler_err");

                // Branch based on tag (0 = Ok, non-zero = Err)
                let is_err = ctx
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        tag,
                        i64_type.const_zero(),
                        "is_err",
                    )
                    .unwrap();
                ctx.builder
                    .build_conditional_branch(is_err, err_block, ok_block)
                    .ok();

                // OK PATH: Serialize the result to JSON
                ctx.builder.position_at_end(ok_block);

                // For array results [T], we need to serialize the array
                // The value pointer points to the array data
                // Call doohttp_serialize_struct_to_json or similar
                let serialize_fn = ctx
                    .module
                    .get_function("doohttp_serialize_struct_to_json")
                    .unwrap_or_else(|| {
                        let fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                        ctx.module
                            .add_function("doohttp_serialize_struct_to_json", fn_type, None)
                    });

                let handler_name_str = ctx
                    .builder
                    .build_global_string_ptr(user_func_name, "handler_name_for_serialize")
                    .map(|g| g.as_pointer_value())
                    .unwrap_or_else(|_| ptr_type.const_null());

                let json_ptr = ctx
                    .builder
                    .build_call(
                        serialize_fn,
                        &[value.into(), handler_name_str.into()],
                        "serialized_json",
                    )
                    .ok()
                    .and_then(|cs| cs.try_as_basic_value().basic())
                    .map(|v| v.into_pointer_value())
                    .unwrap_or_else(|| ptr_type.const_null());

                // Store in DooResult with tag=0 (Ok)
                if let Ok(tag_ptr) = ctx.builder.build_struct_gep(
                    result_struct_type,
                    doo_result_ptr,
                    0,
                    "ok_tag_ptr",
                ) {
                    let _ = ctx.builder.build_store(tag_ptr, i32_type.const_zero());
                }
                if let Ok(value_ptr) = ctx.builder.build_struct_gep(
                    result_struct_type,
                    doo_result_ptr,
                    1,
                    "ok_value_ptr",
                ) {
                    let _ = ctx.builder.build_store(value_ptr, json_ptr);
                }
                if let Ok(owner_ptr) = ctx.builder.build_struct_gep(
                    result_struct_type,
                    doo_result_ptr,
                    2,
                    "ok_owner_ptr",
                ) {
                    let _ = ctx
                        .builder
                        .build_store(owner_ptr, i8_type.const_int(1, false));
                }
                let _ = ctx.builder.build_return(Some(&doo_result_ptr));

                // ERROR PATH: Build error response with actual error message
                ctx.builder.position_at_end(err_block);

                // Call doohttp_format_error_as_json to format the error message from the result
                // The 'value' variable contains the error message pointer
                let format_error_fn = ctx
                    .module
                    .get_function("doohttp_format_error_as_json")
                    .unwrap_or_else(|| {
                        let fn_type = ptr_type.fn_type(&[ptr_type.into()], false);
                        ctx.module
                            .add_function("doohttp_format_error_as_json", fn_type, None)
                    });

                let error_json_str = ctx
                    .builder
                    .build_call(format_error_fn, &[value.into()], "formatted_error_json")
                    .ok()
                    .and_then(|cs| cs.try_as_basic_value().basic())
                    .map(|v| v.into_pointer_value())
                    .unwrap_or_else(|| ctx.const_string("{\"error\": \"Internal server error\"}"));

                // Build error response struct { status=500, body, content_type }
                let error_response_type = ctx
                    .context
                    .struct_type(&[i32_type.into(), ptr_type.into(), ptr_type.into()], false);
                let error_response_size = i64_type.const_int(24, false);
                let error_response_ptr = ctx
                    .builder
                    .build_call(malloc_fn, &[error_response_size.into()], "error_response")
                    .ok()
                    .and_then(|cs| cs.try_as_basic_value().basic())
                    .map(|v| v.into_pointer_value())
                    .unwrap_or_else(|| ptr_type.const_null());

                // Set status = 500
                if let Ok(status_ptr) = ctx.builder.build_struct_gep(
                    error_response_type,
                    error_response_ptr,
                    0,
                    "status_ptr",
                ) {
                    let _ = ctx
                        .builder
                        .build_store(status_ptr, i32_type.const_int(500, false));
                }
                // Set body
                if let Ok(body_ptr) = ctx.builder.build_struct_gep(
                    error_response_type,
                    error_response_ptr,
                    1,
                    "body_ptr",
                ) {
                    let _ = ctx.builder.build_store(body_ptr, error_json_str);
                }
                // Set content_type
                let json_content_type = ctx.const_string("application/json");
                if let Ok(ct_ptr) = ctx.builder.build_struct_gep(
                    error_response_type,
                    error_response_ptr,
                    2,
                    "ct_ptr",
                ) {
                    let _ = ctx.builder.build_store(ct_ptr, json_content_type);
                }

                // Build DooResult for error: { tag=1, value=error_response, owner=1 }
                if let Ok(tag_ptr) = ctx.builder.build_struct_gep(
                    result_struct_type,
                    doo_result_ptr,
                    0,
                    "error_tag_ptr",
                ) {
                    let _ = ctx
                        .builder
                        .build_store(tag_ptr, i32_type.const_int(1, false));
                }
                if let Ok(value_ptr) = ctx.builder.build_struct_gep(
                    result_struct_type,
                    doo_result_ptr,
                    1,
                    "error_value_ptr",
                ) {
                    let _ = ctx.builder.build_store(value_ptr, error_response_ptr);
                }
                if let Ok(owner_ptr) = ctx.builder.build_struct_gep(
                    result_struct_type,
                    doo_result_ptr,
                    2,
                    "error_owner_ptr",
                ) {
                    let _ = ctx
                        .builder
                        .build_store(owner_ptr, i8_type.const_int(1, false));
                }
                let _ = ctx.builder.build_return(Some(&doo_result_ptr));

                // Restore position
                if let Some(block) = current_block {
                    ctx.builder.position_at_end(block);
                }

                return wrapper_fn;
            }
        }

        // Fallback if we couldn't extract struct - return null result
        if let Ok(tag_ptr) =
            ctx.builder
                .build_struct_gep(result_struct_type, doo_result_ptr, 0, "tag_ptr")
        {
            let _ = ctx
                .builder
                .build_store(tag_ptr, i32_type.const_int(1, false));
        }
        if let Ok(value_ptr) =
            ctx.builder
                .build_struct_gep(result_struct_type, doo_result_ptr, 1, "value_ptr")
        {
            let _ = ctx.builder.build_store(value_ptr, ptr_type.const_null());
        }
        if let Ok(owner_ptr) =
            ctx.builder
                .build_struct_gep(result_struct_type, doo_result_ptr, 2, "owner_ptr")
        {
            let _ = ctx
                .builder
                .build_store(owner_ptr, i8_type.const_int(1, false));
        }
        let _ = ctx.builder.build_return(Some(&doo_result_ptr));

        // Restore position
        if let Some(block) = current_block {
            ctx.builder.position_at_end(block);
        }

        return wrapper_fn;
    }

    // If user returns a struct, serialize it to JSON
    let final_result = if returns_struct {
        if let Some(val) = user_result {
            if val.is_pointer_value() {
                let struct_ptr = val.into_pointer_value();

                // Get or declare doohttp_serialize_struct_to_json
                let serialize_fn = ctx
                    .module
                    .get_function("doohttp_serialize_struct_to_json")
                    .unwrap_or_else(|| {
                        let fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                        ctx.module
                            .add_function("doohttp_serialize_struct_to_json", fn_type, None)
                    });

                // Create handler name string
                let handler_name_str = ctx
                    .builder
                    .build_global_string_ptr(user_func_name, "handler_name_for_serialize")
                    .map(|g| g.as_pointer_value())
                    .unwrap_or_else(|_| ptr_type.const_null());

                // Call serialization function
                let json_ptr = ctx
                    .builder
                    .build_call(
                        serialize_fn,
                        &[struct_ptr.into(), handler_name_str.into()],
                        "serialized_json",
                    )
                    .ok()
                    .and_then(|cs| cs.try_as_basic_value().basic())
                    .map(|v| v.into_pointer_value())
                    .unwrap_or_else(|| ptr_type.const_null());

                Some(json_ptr.into())
            } else {
                user_result
            }
        } else {
            user_result
        }
    } else {
        user_result
    };

    // Build success result
    let result_size = i64_type.const_int(
        result_struct_type
            .size_of()
            .unwrap()
            .get_zero_extended_constant()
            .unwrap_or(24),
        false,
    );
    let result_ptr = ctx
        .builder
        .build_call(malloc_fn, &[result_size.into()], "result_alloc")
        .ok()
        .and_then(|cs| cs.try_as_basic_value().basic())
        .map(|v| v.into_pointer_value())
        .unwrap_or_else(|| ptr_type.const_null());

    // Set tag = 0 (Ok)
    let tag_ptr = ctx
        .builder
        .build_struct_gep(result_struct_type, result_ptr, 0, "tag_ptr")
        .ok();
    if let Some(tag_ptr) = tag_ptr {
        let _ = ctx
            .builder
            .build_store(tag_ptr, ctx.i32_type().const_int(0, false));
    }

    // Set value = user result (as pointer)
    let value_ptr = ctx
        .builder
        .build_struct_gep(result_struct_type, result_ptr, 1, "value_ptr")
        .ok();
    if let Some(value_ptr) = value_ptr {
        let result_as_ptr = match final_result {
            Some(val) if val.is_pointer_value() => val.into_pointer_value(),
            Some(val) if val.is_int_value() => {
                // Convert int to pointer (for status codes, etc.)
                ctx.builder
                    .build_int_to_ptr(val.into_int_value(), ptr_type, "int_to_ptr")
                    .unwrap_or_else(|_| ptr_type.const_null())
            }
            _ => ptr_type.const_null(),
        };
        let _ = ctx.builder.build_store(value_ptr, result_as_ptr);
    }

    // Set owner = 1 (FFI owns)
    let owner_ptr = ctx
        .builder
        .build_struct_gep(result_struct_type, result_ptr, 2, "owner_ptr")
        .ok();
    if let Some(owner_ptr) = owner_ptr {
        let _ = ctx
            .builder
            .build_store(owner_ptr, ctx.context.i8_type().const_int(1, false));
    }

    // Return the result pointer
    let _ = ctx.builder.build_return(Some(&result_ptr));

    // Restore position
    if let Some(block) = current_block {
        ctx.builder.position_at_end(block);
    }

    wrapper_fn
}

/// Create a dummy wrapper that returns null (for error cases)
fn create_dummy_wrapper<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    wrapper_name: &str,
) -> FunctionValue<'ctx> {
    let ptr_type = ctx.ptr_type();
    let wrapper_fn_type = ptr_type.fn_type(&[ptr_type.into()], false);
    let wrapper_fn = ctx.module.add_function(wrapper_name, wrapper_fn_type, None);

    let entry = ctx.context.append_basic_block(wrapper_fn, "entry");
    ctx.builder.position_at_end(entry);
    let _ = ctx.builder.build_return(Some(&ptr_type.const_null()));

    wrapper_fn
}

// ============================================================================
// Result/Error Handling Helpers
// ============================================================================

/// Convert a value to a pointer representation for storing in Result payload.
/// - Pointers: pass through as-is
/// - Integers: use inttoptr
/// - Floats: bitcast to i64, then inttoptr
fn value_to_ptr<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
) -> Option<PointerValue<'ctx>> {
    if val.is_pointer_value() {
        // Already a pointer (string, array, map, struct)
        Some(val.into_pointer_value())
    } else if val.is_int_value() {
        // Cast integer to pointer using inttoptr
        let int_val = val.into_int_value();
        let int_64 = if int_val.get_type().get_bit_width() == 64 {
            int_val
        } else {
            ctx.builder
                .build_int_z_extend(int_val, ctx.i64_type(), "ext")
                .ok()?
        };
        ctx.builder
            .build_int_to_ptr(int_64, ctx.ptr_type(), "int_as_ptr")
            .ok()
    } else if val.is_float_value() {
        // Bitcast float to i64 then to pointer
        let float_val = val.into_float_value();
        let alloca = ctx.builder.build_alloca(ctx.f64_type(), "f_tmp").ok()?;
        ctx.builder.build_store(alloca, float_val).ok()?;
        let i64_ptr = ctx
            .builder
            .build_pointer_cast(alloca, ctx.ptr_type(), "i64_ptr")
            .ok()?;
        let i64_val = ctx
            .builder
            .build_load(ctx.i64_type(), i64_ptr, "f_as_i64")
            .ok()?
            .into_int_value();
        ctx.builder
            .build_int_to_ptr(i64_val, ctx.ptr_type(), "float_as_ptr")
            .ok()
    } else if val.is_struct_value() {
        // Heap-allocate struct and return pointer
        let struct_val = val.into_struct_value();
        let struct_type = struct_val.get_type();
        let heap_ptr = ctx.builder.build_malloc(struct_type, "struct_heap").ok()?;
        ctx.builder.build_store(heap_ptr, struct_val).ok()?;
        Some(heap_ptr)
    } else {
        // Fallback: use null pointer
        Some(ctx.ptr_type().const_null())
    }
}

/// Load a Result struct from a value that may be a pointer or struct.
fn load_result_struct<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    result_val: BasicValueEnum<'ctx>,
) -> Option<inkwell::values::StructValue<'ctx>> {
    // Result struct layout: { i64 tag, i64 value }
    // Using i64 for both fields for consistent ABI with FFI SimpleResult
    let result_struct_type = ctx
        .context
        .struct_type(&[ctx.i64_type().into(), ctx.i64_type().into()], false);

    if result_val.is_pointer_value() && !result_val.is_struct_value() {
        // Load from pointer
        let result_ptr = result_val.into_pointer_value();
        ctx.builder
            .build_load(result_struct_type, result_ptr, "result_struct_load")
            .ok()?
            .try_into()
            .ok()
    } else if result_val.is_struct_value() {
        // Already a struct value
        Some(result_val.into_struct_value())
    } else {
        // Not a Result - return None
        None
    }
}

/// Emit panic code: print message and exit(1).
fn emit_panic<'ctx>(ctx: &mut CodegenContext<'ctx>, message: &str) -> Option<()> {
    // Get or declare printf
    let printf_type = ctx.i32_type().fn_type(&[ctx.ptr_type().into()], true);
    let printf = ctx
        .module
        .get_function("printf")
        .unwrap_or_else(|| ctx.module.add_function("printf", printf_type, None));

    // Print panic message
    let panic_fmt = ctx.const_string("panic: %s\n");
    let panic_msg = ctx.const_string(message);
    ctx.builder
        .build_call(printf, &[panic_fmt.into(), panic_msg.into()], "print_panic")
        .ok()?;

    // Get or declare exit
    let exit_type = ctx
        .context
        .void_type()
        .fn_type(&[ctx.i32_type().into()], false);
    let exit_fn = ctx
        .module
        .get_function("exit")
        .unwrap_or_else(|| ctx.module.add_function("exit", exit_type, None));

    // Exit with code 1
    let exit_code = ctx.i32_type().const_int(1, false);
    ctx.builder
        .build_call(exit_fn, &[exit_code.into()], "exit_on_panic")
        .ok()?;

    ctx.builder.build_unreachable().ok()?;
    Some(())
}
/// Emit a call to doo_http_register_handler_with_metadata to register handler metadata.
/// This is called when an HTTP route is registered, allowing the FFI to validate
/// request bodies against the expected struct types.
fn emit_handler_metadata_registration<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    handler_name: &str,
    wrapper_fn: &FunctionValue<'ctx>,
) {
    let debug = std::env::var("DOO_DEBUG").is_ok();

    // Build metadata JSON from function parameter types
    let metadata_json = build_handler_metadata_json(ctx, handler_name);

    if debug {
        doo_debug!(
            "CODEGEN",
            "Registering handler metadata for {}: {}",
            handler_name,
            metadata_json
        );
    }

    // Get or declare doo_http_register_handler_with_metadata
    let void_type = ctx.context.void_type();
    let ptr_type = ctx.ptr_type();
    let fn_type = void_type.fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false);

    let register_fn = ctx
        .module
        .get_function("doo_http_register_handler_with_metadata")
        .unwrap_or_else(|| {
            ctx.module
                .add_function("doo_http_register_handler_with_metadata", fn_type, None)
        });

    // Create string constants for handler name and metadata
    let handler_name_ptr = ctx.const_string(handler_name);
    let metadata_ptr = ctx.const_string(&metadata_json);

    // Get wrapper function pointer
    let wrapper_ptr = wrapper_fn.as_global_value().as_pointer_value();

    // Call doo_http_register_handler_with_metadata(name, wrapper_ptr, metadata_json)
    let _ = ctx.builder.build_call(
        register_fn,
        &[
            handler_name_ptr.into(),
            wrapper_ptr.into(),
            metadata_ptr.into(),
        ],
        "register_handler_meta",
    );
}

/// Emit struct/enum metadata registration for auth/crud calls.
/// When app.auth() or app.crud() is called with a struct type, we need to register
/// the struct's field layout and any enum types it references so the FFI can
/// validate incoming requests at runtime.
fn emit_struct_metadata_registration_for_auth_crud<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    symbol: &str,
    args: &[MirOperand],
) {
    let debug = std::env::var("DOO_DEBUG").is_ok();

    // For auth: args are [server, signup_path, login_path, struct_name, db]
    // For crud: args are [server, base_path, struct_name, db]
    let struct_name_arg_idx = if symbol == "doo_http_auth" { 3 } else { 2 };

    let struct_name = match args.get(struct_name_arg_idx) {
        Some(MirOperand::Const(MirConst::Str(name))) => name.clone(),
        _ => return, // Not a constant string, can't determine at compile time
    };

    if debug {
        doo_debug!(
            "CODEGEN",
            "Registering struct metadata for {}: {}",
            symbol,
            struct_name
        );
    }

    // Look up the struct in the type registry
    let struct_type_id = match ctx.type_registry.lookup(&struct_name) {
        Some(id) => id,
        None => return,
    };

    // Build struct metadata JSON
    let struct_metadata = build_struct_metadata_json(ctx, struct_type_id);

    // Get or declare doo_http_register_struct_metadata
    let void_type = ctx.context.void_type();
    let ptr_type = ctx.ptr_type();
    let fn_type = void_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);

    let register_fn = ctx
        .module
        .get_function("doo_http_register_struct_metadata")
        .unwrap_or_else(|| {
            ctx.module
                .add_function("doo_http_register_struct_metadata", fn_type, None)
        });

    // Create string constants
    let struct_name_ptr = ctx.const_string(&struct_name);
    let metadata_ptr = ctx.const_string(&struct_metadata);

    // Call doo_http_register_struct_metadata(name, metadata_json)
    let _ = ctx.builder.build_call(
        register_fn,
        &[struct_name_ptr.into(), metadata_ptr.into()],
        "register_struct_meta",
    );

    // Collect field type IDs first to avoid borrow conflict
    let field_type_ids: Vec<doo_core::types::TypeId> = ctx
        .type_registry
        .get(struct_type_id)
        .and_then(|info| {
            if let TypeKind::Struct { fields, .. } = &info.kind {
                Some(fields.iter().map(|(_, tid, _)| *tid).collect())
            } else {
                None
            }
        })
        .unwrap_or_default();

    // Register any enum types referenced by this struct
    for field_type_id in field_type_ids {
        emit_enum_metadata_if_needed(ctx, field_type_id);
    }
}

/// Emit enum metadata registration if the type is an enum.
fn emit_enum_metadata_if_needed<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    type_id: doo_core::types::TypeId,
) {
    let type_info = match ctx.type_registry.get(type_id) {
        Some(info) => info,
        None => return,
    };

    if let TypeKind::Enum { name, variants, .. } = &type_info.kind {
        let debug = std::env::var("DOO_DEBUG").is_ok();

        if debug {
            doo_debug!("CODEGEN", "Registering enum metadata: {}", name);
        }

        // Build variants JSON array
        let variant_names: Vec<&str> = variants.iter().map(|(name, _)| name.as_str()).collect();
        let variants_json =
            serde_json::to_string(&variant_names).unwrap_or_else(|_| "[]".to_string());

        // Get or declare doo_http_register_enum_metadata
        let void_type = ctx.context.void_type();
        let ptr_type = ctx.ptr_type();
        let fn_type = void_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);

        let register_fn = ctx
            .module
            .get_function("doo_http_register_enum_metadata")
            .unwrap_or_else(|| {
                ctx.module
                    .add_function("doo_http_register_enum_metadata", fn_type, None)
            });

        // Create string constants
        let enum_name_ptr = ctx.const_string(name);
        let variants_ptr = ctx.const_string(&variants_json);

        // Call doo_http_register_enum_metadata(name, variants_json)
        let _ = ctx.builder.build_call(
            register_fn,
            &[enum_name_ptr.into(), variants_ptr.into()],
            "register_enum_meta",
        );
    }
}

/// Build struct metadata JSON for a given type ID.
fn build_struct_metadata_json<'ctx>(
    ctx: &CodegenContext<'ctx>,
    type_id: doo_core::types::TypeId,
) -> String {
    let type_info = match ctx.type_registry.get(type_id) {
        Some(info) => info,
        None => return "{}".to_string(),
    };

    if let TypeKind::Struct { fields, .. } = &type_info.kind {
        let field_list: Vec<serde_json::Value> = fields
            .iter()
            .map(|(name, field_type_id, _is_public)| {
                let type_name = type_id_to_string_inner(&ctx.type_registry, *field_type_id);

                // Decorators are not stored in TypeKind::Struct - they're in the AST/HIR
                // For now, we'll include just name and type; decorators would require
                // extending the type registry or looking them up from AST metadata
                serde_json::json!({
                    "name": name,
                    "type": type_name,
                    "decorators": []
                })
            })
            .collect();

        serde_json::to_string(&serde_json::json!({
            "fields": field_list
        }))
        .unwrap_or_else(|_| "{}".to_string())
    } else {
        "{}".to_string()
    }
}

/// Build metadata JSON string for a handler function.
/// Format: {"param_count":N,"param_types":["TypeName"],"struct_layouts":{...}}
fn build_handler_metadata_json<'ctx>(ctx: &CodegenContext<'ctx>, func_name: &str) -> String {
    use doo_core::types::{TypeId, TypeKind, TypeRegistry};
    use std::collections::HashMap;

    // Get function parameter types
    let param_type_ids = ctx.get_function_param_types(func_name);
    let param_count = param_type_ids.map(|v| v.len()).unwrap_or(0);

    let mut param_types: Vec<String> = Vec::new();
    let mut struct_layouts: HashMap<String, serde_json::Value> = HashMap::new();
    let mut enum_variants: HashMap<String, Vec<String>> = HashMap::new();

    // Helper to collect enums referenced by a type
    fn collect_enums_from_type(
        registry: &TypeRegistry,
        type_id: TypeId,
        enum_variants: &mut HashMap<String, Vec<String>>,
    ) {
        if let Some(type_info) = registry.get(type_id) {
            match &type_info.kind {
                TypeKind::Enum { name, variants, .. } => {
                    if !enum_variants.contains_key(name) {
                        // Extract just the variant names from (String, Option<TypeId>)
                        let variant_names: Vec<String> = variants
                            .iter()
                            .map(|(variant_name, _)| variant_name.clone())
                            .collect();
                        enum_variants.insert(name.clone(), variant_names);
                    }
                }
                TypeKind::Array { element } => {
                    collect_enums_from_type(registry, *element, enum_variants);
                }
                TypeKind::Optional { inner } => {
                    collect_enums_from_type(registry, *inner, enum_variants);
                }
                TypeKind::Map { key, value } => {
                    collect_enums_from_type(registry, *key, enum_variants);
                    collect_enums_from_type(registry, *value, enum_variants);
                }
                _ => {}
            }
        }
    }

    // Helper to collect struct layouts recursively
    fn collect_struct_layout(
        registry: &TypeRegistry,
        type_id: TypeId,
        struct_layouts: &mut HashMap<String, serde_json::Value>,
        enum_variants: &mut HashMap<String, Vec<String>>,
    ) {
        if let Some(type_info) = registry.get(type_id) {
            match &type_info.kind {
                TypeKind::Struct { name, fields, .. } => {
                    // Skip if already collected
                    if struct_layouts.contains_key(name) {
                        return;
                    }

                    let mut field_list: Vec<serde_json::Value> = Vec::new();
                    let mut current_offset: u64 = 0;

                    for (field_name, field_type_id, _) in fields {
                        let field_type_name = type_id_to_string_inner(registry, *field_type_id);

                        // Calculate field size and alignment based on LLVM type mapping
                        // CRITICAL: Must match get_llvm_type() in context.rs
                        // - Int -> i64 (8 bytes), NOT i32
                        // - Float -> f64 (8 bytes)
                        // - Bool -> i1 (1 byte, but aligned to 8 for struct fields)
                        // - Str/ptr -> ptr (8 bytes on 64-bit)
                        let (field_size, field_align) =
                            match registry.get(*field_type_id).map(|t| &t.kind) {
                                Some(TypeKind::Int) => (8u64, 8u64),       // i64
                                Some(TypeKind::Float) => (8, 8),           // f64
                                Some(TypeKind::Bool) => (1, 1), // i1 (but struct packs with padding)
                                Some(TypeKind::Str) => (8, 8),  // pointer
                                Some(TypeKind::Array { .. }) => (8, 8), // pointer to array
                                Some(TypeKind::Map { .. }) => (8, 8), // pointer to map
                                Some(TypeKind::Struct { .. }) => (8, 8), // pointer to struct
                                Some(TypeKind::Optional { .. }) => (8, 8), // pointer
                                Some(TypeKind::Enum { .. }) => (16, 8), // { i32, ptr } = 16 bytes
                                _ => (8, 8),                    // default to pointer size
                            };

                        // Align current offset to field's alignment
                        if field_align > 0 && current_offset % field_align != 0 {
                            current_offset = ((current_offset / field_align) + 1) * field_align;
                        }

                        field_list.push(serde_json::json!({
                            "name": field_name,
                            "type": field_type_name,
                            "offset": current_offset
                        }));

                        // Move offset past this field
                        current_offset += field_size;

                        // Recursively collect nested structs and enums
                        collect_struct_layout(
                            registry,
                            *field_type_id,
                            struct_layouts,
                            enum_variants,
                        );
                        collect_enums_from_type(registry, *field_type_id, enum_variants);
                    }
                    struct_layouts.insert(
                        name.clone(),
                        serde_json::json!({
                            "fields": field_list
                        }),
                    );
                }
                TypeKind::Array { element } => {
                    collect_struct_layout(registry, *element, struct_layouts, enum_variants);
                }
                TypeKind::Optional { inner } => {
                    collect_struct_layout(registry, *inner, struct_layouts, enum_variants);
                }
                TypeKind::Map { key, value } => {
                    collect_struct_layout(registry, *key, struct_layouts, enum_variants);
                    collect_struct_layout(registry, *value, struct_layouts, enum_variants);
                }
                _ => {}
            }
        }
    }

    if let Some(type_ids) = param_type_ids {
        for type_id in type_ids {
            // Recursively collect all struct layouts and enum variants
            collect_struct_layout(
                &ctx.type_registry,
                *type_id,
                &mut struct_layouts,
                &mut enum_variants,
            );

            // Get type name from type registry
            if let Some(type_info) = ctx.type_registry.get(*type_id) {
                let type_name = match &type_info.kind {
                    TypeKind::Struct { name, .. } => name.clone(),
                    _ => type_id_to_string_inner(&ctx.type_registry, *type_id),
                };
                param_types.push(type_name);
            }
        }
    }

    // Get return type and include it in metadata
    let mut return_type = "Void".to_string();
    if let Some(ret_type_id) = ctx.get_function_return_type(func_name) {
        // Collect struct layouts for return type
        collect_struct_layout(
            &ctx.type_registry,
            ret_type_id,
            &mut struct_layouts,
            &mut enum_variants,
        );
        return_type = type_id_to_string_inner(&ctx.type_registry, ret_type_id);
    }

    // Build JSON
    let metadata = serde_json::json!({
        "param_count": param_count,
        "param_types": param_types,
        "return_type": return_type,
        "struct_layouts": struct_layouts,
        "enum_variants": enum_variants
    });

    metadata.to_string()
}

/// Convert TypeId to a string representation for metadata, resolving nested types
fn type_id_to_string_inner(
    registry: &doo_core::types::TypeRegistry,
    type_id: doo_core::types::TypeId,
) -> String {
    use doo_core::types::TypeKind;

    if let Some(type_info) = registry.get(type_id) {
        match &type_info.kind {
            TypeKind::Int => "Int".to_string(),
            TypeKind::Float => "Float".to_string(),
            TypeKind::Bool => "Bool".to_string(),
            TypeKind::Str => "Str".to_string(),
            TypeKind::Void => "Void".to_string(),
            TypeKind::Array { element } => {
                let elem_str = type_id_to_string_inner(registry, *element);
                format!("[{}]", elem_str)
            }
            TypeKind::Optional { inner } => {
                let inner_str = type_id_to_string_inner(registry, *inner);
                format!("Optional({})", inner_str)
            }
            TypeKind::Struct { name, .. } => name.clone(),
            TypeKind::Enum { name, .. } => name.clone(),
            TypeKind::Function { .. } => "Function".to_string(),
            TypeKind::Map { key, value } => {
                let key_str = type_id_to_string_inner(registry, *key);
                let value_str = type_id_to_string_inner(registry, *value);
                format!("Map<{},{}>", key_str, value_str)
            }
            TypeKind::Result { ok, err } => {
                let ok_str = type_id_to_string_inner(registry, *ok);
                let err_str = type_id_to_string_inner(registry, *err);
                format!("Result<{},{}>", ok_str, err_str)
            }
            TypeKind::Tuple { elements } => {
                let elem_strs: Vec<String> = elements
                    .iter()
                    .map(|e| type_id_to_string_inner(registry, *e))
                    .collect();
                format!("({})", elem_strs.join(","))
            }
            TypeKind::TypeRef { name } => name.clone(),
            TypeKind::Any => "Any".to_string(),
            TypeKind::Error => "Error".to_string(),
        }
    } else {
        "Unknown".to_string()
    }
}
