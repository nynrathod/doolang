//! FFI call implementation — signatures, declarations, and call emission.

use super::call_utils::operand_to_value;
use super::call_wrappers::{
    extract_route_context,
    get_or_generate_handler_wrapper,
    get_or_generate_handler_wrapper_with_context,
    get_or_generate_ws_handler_wrapper,
    get_or_generate_ws_event_handler_wrapper,
    get_or_generate_ws_lifecycle_handler_wrapper,
};
use super::call_metadata::{
    emit_handler_metadata_registration,
    emit_struct_metadata_registration_for_auth_crud,
};
use super::RouteContext;
use crate::context::CodegenContext;
use doo_core::constants::ffi_names;
use doo_core::doo_debug;
use doo_core::types::{builtin, TypeKind};
use doo_mir::sym::resolve;
use doo_mir::{MirConst, MirOperand};
use inkwell::module::Linkage;
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue};
use inkwell::AddressSpace;
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
        ffi_names::DOO_FILE_INIT => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_FILE_READ => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_FILE_WRITE => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_FILE_APPEND => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_FILE_DELETE => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_FILE_EXISTS => Some((&["ptr"], "i32", false)),
        ffi_names::DOO_FILE_METADATA => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_FILE_COPY => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_FILE_MOVE => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_FILE_SIZE => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_FILE_READ_LINES => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_FILE_MKDIR => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_FILE_MKDIR_ALL => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_FILE_RMDIR => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_FILE_RMDIR_ALL => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_FILE_LIST_DIR => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_FILE_IS_FILE => Some((&["ptr"], "i32", false)),
        ffi_names::DOO_FILE_IS_DIR => Some((&["ptr"], "i32", false)),
        ffi_names::DOO_FILE_MODIFIED_TIME => Some((&["ptr"], "i64", false)),
        ffi_names::DOO_FILE_FREE_RESULT => Some((&["ptr"], "void", false)),

        // HTTP FFI
        ffi_names::DOO_HTTP_SERVER_NEW => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_SERVER_LISTEN => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_LISTEN => Some((&["ptr"], "ptr", false)),
        // Function pointer versions (handler is function pointer, not string)
        ffi_names::DOO_HTTP_GET_FN => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_POST_FN => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_PUT_FN => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_DELETE_FN => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_PATCH_FN => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        // String-based versions (legacy)
        ffi_names::DOO_HTTP_GET => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_POST => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_PUT => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_DELETE => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_PATCH => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_USE => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_GROUP => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_CORS_CUSTOM => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_RATELIMIT_CUSTOM => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_GET_WITH_MIDDLEWARE => Some((&["ptr", "ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_POST_WITH_MIDDLEWARE => Some((&["ptr", "ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_PUT_WITH_MIDDLEWARE => Some((&["ptr", "ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_DELETE_WITH_MIDDLEWARE => Some((&["ptr", "ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_PATCH_WITH_MIDDLEWARE => Some((&["ptr", "ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_REGISTER_ROUTE => Some((&["ptr", "ptr", "ptr", "ptr"], "void", false)),
        ffi_names::DOO_HTTP_REQ_GET_HEADER => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_REQ_GET_BODY => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_REQ_GET_PARAM => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_REQ_GET_QUERY => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_REQ_QUERY => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_REQ_PARAM => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_REQ_HEADER => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_NEXT_CALL => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_AUTH => Some((&["ptr", "ptr", "ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_CRUD => Some((&["ptr", "ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_PARSE_JSON => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_TO_JSON => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_RES_SET_STATUS => Some((&["ptr", "i32"], "void", false)),
        ffi_names::DOO_HTTP_RES_SET_HEADER => Some((&["ptr", "ptr", "ptr"], "void", false)),
        ffi_names::DOO_HTTP_RES_SET_BODY => Some((&["ptr", "ptr"], "void", false)),
        ffi_names::DOO_HTTP_RES_JSON => Some((&["ptr", "ptr"], "void", false)),

        // Database FFI
        ffi_names::DOO_DB_POSTGRES => Some((&["ptr"], "ptr", false)),
        // These return *mut SimpleResult (pointer to heap-allocated result) for Windows ABI compatibility
        ffi_names::DOO_DB_CONNECT_POSTGRES => Some((&[], "ptr", false)),
        ffi_names::DOO_DB_GET_GLOBAL => Some((&[], "ptr", false)),
        ffi_names::DOO_DB_RAW => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_RAW_PARAM => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_RESULT_FREE => Some((&["ptr"], "void", false)),
        ffi_names::DOO_DB_FREE_STRING => Some((&["ptr"], "void", false)),
        ffi_names::DOO_DB_FIND => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_FIND_ALL => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_INSERT => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_UPDATE => Some((&["ptr", "ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_DELETE => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_QUERY => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_EXISTS => Some((&["ptr", "ptr", "ptr"], "i32", false)),

        // Auth FFI
        ffi_names::DOO_AUTH_HASH_PASSWORD => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_AUTH_VERIFY_PASSWORD => Some((&["ptr", "ptr"], "i32", false)),
        ffi_names::DOO_AUTH_SIGN_TOKEN => Some((&["ptr", "ptr", "i64"], "ptr", false)),
        ffi_names::DOO_AUTH_VERIFY_TOKEN => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_AUTH_FREE_RESULT => Some((&["ptr"], "void", false)),
        ffi_names::DOO_AUTH_SIGN => Some((&["ptr", "ptr", "i64"], "ptr", false)),
        ffi_names::DOO_AUTH_VERIFY => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_AUTH_FREE_STRING => Some((&["ptr"], "void", false)),
        ffi_names::DOO_HTTP_JWT => Some((&[], "ptr", false)),

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

        // WebSocket FFI
        // Route registration: (server_ptr, path, handler_fn_ptr) -> result
        ffi_names::DOO_WS_ROUTE => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_WS_INIT => Some((&[], "void", false)),
        // Server instance methods: (_server, ...) -> result
        ffi_names::DOO_WS_CONFIG => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_WS_SHUTDOWN => Some((&["ptr"], "void", false)),
        ffi_names::DOO_WS_ACTIVE_CONNECTIONS => Some((&["ptr"], "i64", false)),
        ffi_names::DOO_WS_IS_WS_ROUTE => Some((&["ptr", "ptr"], "i64", false)),
        // Server instance getter
        ffi_names::DOO_HTTP_GET_SERVER_INSTANCE => Some((&[], "ptr", false)),
        // Connection operations: (conn_ptr, ...) -> result
        ffi_names::DOO_WS_CONN_ID => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_WS_CONN_EMIT => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_WS_CONN_EMIT_BINARY => Some((&["ptr", "ptr", "i64"], "ptr", false)),
        ffi_names::DOO_WS_CONN_JOIN => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_WS_CONN_LEAVE => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_WS_CONN_CLOSE => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_WS_CONN_IS_CLOSED => Some((&["ptr"], "i64", false)),
        ffi_names::DOO_WS_CONN_ON => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_WS_CONN_ON_CONNECT => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_WS_CONN_ON_DISCONNECT => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_WS_CONN_ON_ERROR => Some((&["ptr", "ptr"], "ptr", false)),
        // Broadcast & room (Server instance methods): (_server, ...) -> result
        ffi_names::DOO_WS_BROADCAST => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_WS_ROOM_EMIT => Some((&["ptr", "ptr", "ptr", "ptr"], "ptr", false)),

        // Process FFI
        ffi_names::DOO_PROCESS_RUN => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_PROCESS_OUTPUT => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_PROCESS_SPAWN => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_PROCESS_KILL => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_PROCESS_STATUS => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_PROCESS_WAIT_OUTPUT => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_PROCESS_IS_RUNNING => Some((&["ptr"], "i64", false)),
        ffi_names::DOO_PROCESS_READ_STDOUT => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_PROCESS_READ_STDERR => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_PROCESS_SHUTDOWN => Some((&[], "void", false)),
        ffi_names::DOO_PROCESS_ACTIVE_COUNT => Some((&[], "i64", false)),

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
pub(super) fn declare_ffi_function<'ctx>(
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
                let int_val = val.into_int_value();
                let ptr_type = ctx.ptr_type();

                // Check if this is a null/nil value (i64 0 from MirConst::Nil)
                // In that case, pass a null pointer directly instead of stringifying
                if let Some(const_val) = int_val.get_zero_extended_constant() {
                    if const_val == 0 {
                        return ptr_type.const_null().into();
                    }
                }

                // Convert i64 to string using sprintf
                // Allocate buffer for max int64 string: "-9223372036854775808" (20 chars + null)
                let i8_type = ctx.context.i8_type();
                let i64_type = ctx.i64_type();

                // Allocate 24 bytes for safety
                let buffer = ctx
                    .builder
                    .build_array_alloca(i8_type, i64_type.const_int(24, false), "int_to_str_buf")
                    .unwrap();

                // Get or declare sprintf
                let sprintf = ctx.module.get_function(ffi_names::SPRINTF).unwrap_or_else(|| {
                    let i32_type = ctx.i32_type();
                    let fn_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], true);
                    ctx.module.add_function(ffi_names::SPRINTF, fn_type, None)
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
                let sprintf = ctx.module.get_function(ffi_names::SPRINTF).unwrap_or_else(|| {
                    let i32_type = ctx.i32_type();
                    let fn_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], true);
                    ctx.module.add_function(ffi_names::SPRINTF, fn_type, None)
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
        MirOperand::Temp(name) | MirOperand::Local(name) => resolve(*name),
        _ => return None,
    };

    // Check if this temp/local is a known enum type
    let enum_name = ctx.temp_struct_types.get(&var_name)?.clone();

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
        MirOperand::Temp(name) | MirOperand::Local(name) => resolve(*name),
        _ => return None,
    };

    // Check if this temp/local is a known array with element type
    let elem_type_id = ctx.array_element_types.get(&var_name)?.clone();

    // IMPORTANT: Check for EMPTY arrays FIRST before generating any LLVM code
    // Empty arrays are tracked in array_element_types but NOT in array_element_temps
    // We must handle them explicitly to return "[]" JSON string
    let has_element_temps = ctx.array_element_temps.contains_key(&var_name);
    if !has_element_temps {
        // This is an empty array - return "[]" directly
        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
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

            if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
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
                .get_function(ffi_names::DOO_DB_SERIALIZE_ENUM_ARRAY)
                .unwrap_or_else(|| {
                    let ptr_type = ctx.ptr_type();
                    let i32_type = ctx.i32_type();
                    let fn_type = ptr_type
                        .fn_type(&[ptr_type.into(), ptr_type.into(), i32_type.into()], false);
                    ctx.module
                        .add_function(ffi_names::DOO_DB_SERIALIZE_ENUM_ARRAY, fn_type, None)
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
    try_convert_mixed_enum_array_to_json_string(ctx, &var_name)
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

    if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
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
    let sprintf = ctx.module.get_function(ffi_names::SPRINTF).unwrap_or_else(|| {
        let i32_type = ctx.i32_type();
        let fn_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], true);
        ctx.module.add_function(ffi_names::SPRINTF, fn_type, None)
    });

    // Get strlen
    let strlen = ctx.module.get_function(ffi_names::STRLEN).unwrap_or_else(|| {
        let fn_type = i64_type.fn_type(&[ptr_type.into()], false);
        ctx.module.add_function(ffi_names::STRLEN, fn_type, None)
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
pub(super) fn emit_ffi_call<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    dest: Option<&str>,
    symbol: &str,
    args: &[MirOperand],
) -> Option<BasicValueEnum<'ctx>> {
    if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
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
    if symbol == ffi_names::DOO_HTTP_AUTH || symbol == ffi_names::DOO_HTTP_CRUD {
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
                        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
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
                        .get_function(ffi_names::DOO_HTTP_REGISTER_MIDDLEWARE)
                        .unwrap_or_else(|| {
                            let ptr_type = ctx.ptr_type();
                            let fn_type = ctx
                                .context
                                .void_type()
                                .fn_type(&[ptr_type.into(), ptr_type.into()], false);
                            ctx.module
                                .add_function(ffi_names::DOO_HTTP_REGISTER_MIDDLEWARE, fn_type, None)
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

                    if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
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
            // ============================================================
            // WebSocket handler wrappers — different from HTTP handlers
            // ============================================================
            if symbol == ffi_names::DOO_WS_ROUTE {
                // WS route handler: fn(*const WsConnection) -> void
                let wrapper = get_or_generate_ws_handler_wrapper(ctx, &resolve(*func_name));
                arg_vals.push(wrapper.as_global_value().as_pointer_value().into());
                continue;
            }
            if symbol == ffi_names::DOO_WS_CONN_ON || symbol == ffi_names::DOO_WS_CONN_ON_ERROR {
                // Event/error handler: fn(*const c_char) -> void
                // User function takes a Str param — direct pointer passthrough
                let wrapper = get_or_generate_ws_event_handler_wrapper(ctx, &resolve(*func_name));
                arg_vals.push(wrapper.as_global_value().as_pointer_value().into());
                continue;
            }
            if symbol == ffi_names::DOO_WS_CONN_ON_CONNECT || symbol == ffi_names::DOO_WS_CONN_ON_DISCONNECT {
                // Lifecycle handler: fn() -> void (no params)
                let wrapper = get_or_generate_ws_lifecycle_handler_wrapper(ctx, &resolve(*func_name));
                arg_vals.push(wrapper.as_global_value().as_pointer_value().into());
                continue;
            }

            // ============================================================
            // HTTP handler wrappers (existing logic)
            // ============================================================
            let func_name_str = resolve(*func_name);
            let wrapper = get_or_generate_handler_wrapper_with_context(
                ctx,
                &func_name_str,
                symbol,
                &route_context,
            );

            // If this is an HTTP route registration, register handler metadata
            // Check for doo_http_get_fn, doo_http_post_fn, etc. AND *_with_middleware variants
            let is_route_registration = symbol.starts_with("doo_http_")
                && (symbol.ends_with("_fn") || symbol.ends_with("_with_middleware"));
            if is_route_registration {
                emit_handler_metadata_registration(ctx, &func_name_str, &wrapper);
            }

            arg_vals.push(wrapper.as_global_value().as_pointer_value().into());
            continue;
        }

        // Special handling for doo_db_raw_param: convert enum/array params (index 2) to JSON string
        if symbol == ffi_names::DOO_DB_RAW_PARAM && i == 2 {
            // Check for empty array literal first - pass "[]" directly
            // Empty arrays are tracked in array_element_types but NOT in array_element_temps
            // (because they have no element temps to track)
            if let MirOperand::Temp(name) = a {
                let name_str = resolve(*name);
                let has_elem_type = ctx.array_element_types.contains_key(&name_str);
                let has_elem_temps = ctx.array_element_temps.contains_key(&name_str);

                if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                    doo_debug!(
                        "CODEGEN",
                        "doo_db_raw_param arg[2]: temp={}, has_elem_type={}, has_elem_temps={}",
                        name_str,
                        has_elem_type,
                        has_elem_temps
                    );
                }

                // If it's tracked as an array (has element type) but has no element temps,
                // it's an empty array - pass "[]" directly
                if has_elem_type && !has_elem_temps {
                    if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                        doo_debug!("CODEGEN", "Converting empty array {} to JSON \"[]\"", name_str);
                    }
                    let empty_json = ctx.const_string("[]");
                    arg_vals.push(empty_json.into());
                    continue;
                }
            } else if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
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
