//! Async Operations Instruction Handler
//!
//! Handles all async MIR instructions by emitting LLVM IR calls
//! to the `doo_ffi_runtime` FFI functions:
//!
//! - `Await`       → `doo_task_await(handle) -> *mut DooResult`
//! - `Spawn`       → `doo_spawn(fn_ptr) -> *mut TaskHandle`
//! - `ScopeCreate` → `doo_scope_create() -> *mut ScopeHandle`
//! - `ScopeSpawn`  → `doo_scope_spawn(scope, fn_ptr)`
//! - `ScopeWait`   → `doo_scope_wait(scope) -> *mut DooResult`
//!
//! All handles use pure ownership (Box::into_raw / Box::from_raw).
//! No Rc/Arc — matches Doo's auto-ownership model.

use super::InstructionHandler;
use crate::context::CodegenContext;
use crate::utils::operand_to_value;
use doo_core::constants::ffi_names;
use doo_mir::sym::resolve;
use doo_mir::{MirInstr, MirInstrKind};
use inkwell::module::Linkage;
use inkwell::values::BasicValueEnum;

/// Async operations instruction handler.
pub struct AsyncOpsHandler;

impl<'ctx> InstructionHandler<'ctx> for AsyncOpsHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(
            &instr.kind,
            MirInstrKind::Sleep { .. }
                | MirInstrKind::Await { .. }
                | MirInstrKind::Spawn { .. }
                | MirInstrKind::ScopeCreate { .. }
                | MirInstrKind::ScopeSpawn { .. }
                | MirInstrKind::ScopeWait { .. }
        )
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            // ================================================================
            // Sleep: doo_sleep(ms: i64) -> *mut DooResult (blocking)
            // ================================================================
            MirInstrKind::Sleep { ms } => {
                let ms_val = operand_to_value(ctx, ms)?;
                let ms_int = if ms_val.is_int_value() {
                    ms_val.into_int_value()
                } else {
                    return None;
                };

                let sleep_fn = get_or_declare_doo_sleep(ctx);
                let _call_site = ctx
                    .builder
                    .build_call(sleep_fn, &[ms_int.into()], "sleep_result")
                    .ok()?;
                // Sleep returns *mut DooResult but we discard it (void semantics)
                None
            }

            // ================================================================
            // Await: consume a TaskHandle, get the result
            // doo_task_await(handle: *mut TaskHandle) -> *mut DooResult
            // ================================================================
            MirInstrKind::Await { dest, handle } => {
                let handle_val = operand_to_value(ctx, handle)?;

                // Ensure handle is a pointer (TaskHandle is opaque ptr)
                let handle_ptr = if handle_val.is_pointer_value() {
                    handle_val.into_pointer_value()
                } else if handle_val.is_int_value() {
                    // Convert i64 to pointer if needed
                    ctx.builder
                        .build_int_to_ptr(
                            handle_val.into_int_value(),
                            ctx.ptr_type(),
                            "handle_to_ptr",
                        )
                        .ok()?
                } else {
                    return None;
                };

                let await_fn = get_or_declare_doo_task_await(ctx);
                let call_site = ctx
                    .builder
                    .build_call(await_fn, &[handle_ptr.into()], "await_result")
                    .ok()?;

                if let Some(result_ptr) = call_site.try_as_basic_value().basic() {
                    ctx.set_temp(&resolve(*dest), result_ptr);
                    Some(result_ptr)
                } else {
                    None
                }
            }

            // ================================================================
            // Spawn: spawn a function as an async task
            // doo_spawn(fn_ptr, env_ptr) -> *mut TaskHandle
            // ================================================================
            MirInstrKind::Spawn {
                dest,
                func,
                captures,
            } => {
                // Get the function value for the closure/function to spawn
                let func_val = ctx.get_function(&resolve(*func))?;
                let func_ptr = func_val.as_global_value().as_pointer_value();

                // Pack captures into env struct (or null if none)
                let env_ptr = build_env_pack(ctx, captures)?;

                let spawn_fn = get_or_declare_doo_spawn(ctx);
                let call_site = ctx
                    .builder
                    .build_call(spawn_fn, &[func_ptr.into(), env_ptr.into()], "spawn_handle")
                    .ok()?;

                if let Some(handle_ptr) = call_site.try_as_basic_value().basic() {
                    ctx.set_temp(&resolve(*dest), handle_ptr);
                    Some(handle_ptr)
                } else {
                    None
                }
            }

            // ================================================================
            // ScopeCreate: create an empty scope handle
            // doo_scope_create() -> *mut ScopeHandle
            // ================================================================
            MirInstrKind::ScopeCreate { dest } => {
                let scope_create_fn = get_or_declare_doo_scope_create(ctx);
                let call_site = ctx
                    .builder
                    .build_call(scope_create_fn, &[], "scope_handle")
                    .ok()?;

                if let Some(scope_ptr) = call_site.try_as_basic_value().basic() {
                    ctx.set_temp(&resolve(*dest), scope_ptr);
                    Some(scope_ptr)
                } else {
                    None
                }
            }

            // ================================================================
            // ScopeSpawn: spawn a task within a scope
            // doo_scope_spawn(scope, fn_ptr, env_ptr)
            // ================================================================
            MirInstrKind::ScopeSpawn {
                scope,
                func,
                captures,
            } => {
                let scope_val = operand_to_value(ctx, scope)?;

                // Ensure scope is a pointer
                let scope_ptr = if scope_val.is_pointer_value() {
                    scope_val.into_pointer_value()
                } else if scope_val.is_int_value() {
                    ctx.builder
                        .build_int_to_ptr(
                            scope_val.into_int_value(),
                            ctx.ptr_type(),
                            "scope_to_ptr",
                        )
                        .ok()?
                } else {
                    return None;
                };

                // Get the function value for the closure to spawn in scope
                let func_val = ctx.get_function(&resolve(*func))?;
                let func_ptr = func_val.as_global_value().as_pointer_value();

                // Pack captures into env struct (or null if none)
                let env_ptr = build_env_pack(ctx, captures)?;

                let scope_spawn_fn = get_or_declare_doo_scope_spawn(ctx);
                let _ = ctx
                    .builder
                    .build_call(
                        scope_spawn_fn,
                        &[scope_ptr.into(), func_ptr.into(), env_ptr.into()],
                        "scope_spawn",
                    )
                    .ok()?;

                // ScopeSpawn returns void — no value to set
                None
            }

            // ================================================================
            // ScopeWait: wait for all scope tasks to complete
            // doo_scope_wait(scope: *mut ScopeHandle) -> *mut DooResult
            // ================================================================
            MirInstrKind::ScopeWait { dest, scope } => {
                let scope_val = operand_to_value(ctx, scope)?;

                // Ensure scope is a pointer
                let scope_ptr = if scope_val.is_pointer_value() {
                    scope_val.into_pointer_value()
                } else if scope_val.is_int_value() {
                    ctx.builder
                        .build_int_to_ptr(
                            scope_val.into_int_value(),
                            ctx.ptr_type(),
                            "scope_to_ptr",
                        )
                        .ok()?
                } else {
                    return None;
                };

                let scope_wait_fn = get_or_declare_doo_scope_wait(ctx);
                let call_site = ctx
                    .builder
                    .build_call(scope_wait_fn, &[scope_ptr.into()], "scope_wait_result")
                    .ok()?;

                if let Some(result_ptr) = call_site.try_as_basic_value().basic() {
                    ctx.set_temp(&resolve(*dest), result_ptr);
                    Some(result_ptr)
                } else {
                    None
                }
            }

            _ => None,
        }
    }
}

// ============================================================================
// FFI Function Declaration Helpers
// ============================================================================
// Each function is declared once and cached in the LLVM module.
// All use `extern "C"` calling convention matching the Rust FFI signatures.

/// `doo_task_await(handle: ptr) -> ptr`
fn get_or_declare_doo_task_await<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = ffi_names::DOO_TASK_AWAIT;
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let fn_ty = ptr_ty.fn_type(&[ptr_ty.into()], false);
    ctx.module
        .add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_spawn(fn_ptr: ptr, env: ptr) -> ptr`
fn get_or_declare_doo_spawn<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = ffi_names::DOO_SPAWN;
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let fn_ty = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
    ctx.module
        .add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_scope_create() -> ptr`
fn get_or_declare_doo_scope_create<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = ffi_names::DOO_SCOPE_CREATE;
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let fn_ty = ptr_ty.fn_type(&[], false);
    ctx.module
        .add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_scope_spawn(scope: ptr, fn_ptr: ptr, env: ptr)` — void return
fn get_or_declare_doo_scope_spawn<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = ffi_names::DOO_SCOPE_SPAWN;
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let void_ty = ctx.context.void_type();
    let fn_ty = void_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), ptr_ty.into()], false);
    ctx.module
        .add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_scope_wait(scope: ptr) -> ptr`
fn get_or_declare_doo_scope_wait<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = ffi_names::DOO_SCOPE_WAIT;
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let fn_ty = ptr_ty.fn_type(&[ptr_ty.into()], false);
    ctx.module
        .add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_runtime_init() -> i32`
pub fn get_or_declare_doo_runtime_init<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = ffi_names::DOO_RUNTIME_INIT;
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let i32_ty = ctx.i32_type();
    let fn_ty = i32_ty.fn_type(&[], false);
    ctx.module
        .add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_runtime_block_on(fn_ptr: ptr) -> ptr`
pub fn get_or_declare_doo_runtime_block_on<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = ffi_names::DOO_RUNTIME_BLOCK_ON;
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let fn_ty = ptr_ty.fn_type(&[ptr_ty.into()], false);
    ctx.module
        .add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_sleep(ms: i64) -> ptr`
pub fn get_or_declare_doo_sleep<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = ffi_names::DOO_SLEEP;
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let i64_ty = ctx.i64_type();
    let fn_ty = ptr_ty.fn_type(&[i64_ty.into()], false);
    ctx.module
        .add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_sleep_async(ms: i64) -> ptr`
pub fn get_or_declare_doo_sleep_async<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = ffi_names::DOO_SLEEP_ASYNC;
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let i64_ty = ctx.i64_type();
    let fn_ty = ptr_ty.fn_type(&[i64_ty.into()], false);
    ctx.module
        .add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_timeout(ms: i64, fn_ptr: ptr) -> ptr`
pub fn get_or_declare_doo_timeout<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = ffi_names::DOO_TIMEOUT;
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let i64_ty = ctx.i64_type();
    let fn_ty = ptr_ty.fn_type(&[i64_ty.into(), ptr_ty.into()], false);
    ctx.module
        .add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_spawn_detach(fn_ptr: ptr, env: ptr)` — void return
pub fn get_or_declare_doo_spawn_detach<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = ffi_names::DOO_SPAWN_DETACH;
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let void_ty = ctx.context.void_type();
    let fn_ty = void_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
    ctx.module
        .add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_task_cancel(handle: ptr)` — void return
pub fn get_or_declare_doo_task_cancel<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = ffi_names::DOO_TASK_CANCEL;
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let void_ty = ctx.context.void_type();
    let fn_ty = void_ty.fn_type(&[ptr_ty.into()], false);
    ctx.module
        .add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_task_free(handle: ptr)` — void return
pub fn get_or_declare_doo_task_free<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = ffi_names::DOO_TASK_FREE;
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let void_ty = ctx.context.void_type();
    let fn_ty = void_ty.fn_type(&[ptr_ty.into()], false);
    ctx.module
        .add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_scope_free(scope: ptr)` — void return
pub fn get_or_declare_doo_scope_free<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = ffi_names::DOO_SCOPE_FREE;
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let void_ty = ctx.context.void_type();
    let fn_ty = void_ty.fn_type(&[ptr_ty.into()], false);
    ctx.module
        .add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_spawn_blocking(fn_ptr: ptr) -> ptr` — for CPU-heavy work
pub fn get_or_declare_doo_spawn_blocking<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = ffi_names::DOO_SPAWN_BLOCKING;
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let fn_ty = ptr_ty.fn_type(&[ptr_ty.into()], false);
    ctx.module
        .add_function(NAME, fn_ty, Some(Linkage::External))
}

// ============================================================================
// Capture Environment Packing/Unpacking
// ============================================================================

/// Pack captured variable values into a heap-allocated env struct.
/// Returns a pointer to the env (or null if no captures).
///
/// Layout: `[i64 × N]` — each slot stores a POINTER (ptrtoint'd) to the
/// outer function's alloca for that captured variable. The spawn function
/// uses these pointers directly, so writes propagate back to the parent scope.
/// The env struct itself is freed by the spawn function after unpacking.
fn build_env_pack<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    captures: &[doo_mir::MirOperand],
) -> Option<inkwell::values::PointerValue<'ctx>> {
    if captures.is_empty() {
        // No captures — pass null env pointer
        return Some(ctx.ptr_type().const_null());
    }

    let i64_ty = ctx.i64_type();
    let count = captures.len() as u64;

    // malloc(count * 8) — one i64 slot per captured variable
    let size = i64_ty.const_int(count * 8, false);
    let malloc_fn = get_or_declare_malloc(ctx);
    let env_raw = ctx
        .builder
        .build_call(malloc_fn, &[size.into()], "env_malloc")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_pointer_value();

    // Store POINTER to each captured variable's alloca (reference capture)
    for (i, cap) in captures.iter().enumerate() {
        let cap_name = match cap {
            doo_mir::MirOperand::Local(n) | doo_mir::MirOperand::Temp(n) => resolve(*n),
            _ => continue,
        };

        // Get the alloca pointer for this variable in the OUTER scope
        let alloca_ptr = match ctx.get_local(&cap_name) {
            Some(ptr) => ptr,
            None => continue,
        };

        // Convert alloca pointer to i64 for uniform storage
        let as_i64 = ctx
            .builder
            .build_ptr_to_int(alloca_ptr, i64_ty, &format!("cap_ref_{}", i))
            .ok()?;

        // GEP to the i-th i64 slot and store
        let field_ptr = unsafe {
            ctx.builder
                .build_gep(
                    i64_ty,
                    env_raw,
                    &[i64_ty.const_int(i as u64, false)],
                    &format!("env_field_{}", i),
                )
                .ok()?
        };
        ctx.builder.build_store(field_ptr, as_i64).ok()?;
    }

    Some(env_raw)
}

/// Emit env unpack code at the start of a spawn function.
/// Loads POINTERS to outer allocas from the env struct, then replaces
/// the spawn function's local allocas with the outer pointers.
/// This implements reference capture — the spawn function directly
/// reads/writes the parent scope's variables.
/// Frees the env struct after unpacking (it only holds pointers, not values).
pub fn emit_env_unpack<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    captures: &[String],
    llvm_func: inkwell::values::FunctionValue<'ctx>,
) {
    if captures.is_empty() {
        return;
    }

    let i64_ty = ctx.i64_type();
    let ptr_ty = ctx.ptr_type();

    // env_ptr is always param 0 for closure/spawn functions
    let env_ptr = llvm_func.get_nth_param(0).unwrap().into_pointer_value();

    // Load each captured alloca pointer from the env struct
    for (i, cap_name) in captures.iter().enumerate() {
        let field_ptr = unsafe {
            ctx.builder
                .build_gep(
                    i64_ty,
                    env_ptr,
                    &[i64_ty.const_int(i as u64, false)],
                    &format!("env_load_{}", i),
                )
                .ok()
        };
        if let Some(field_ptr) = field_ptr {
            if let Ok(loaded_i64) =
                ctx.builder
                    .build_load(i64_ty, field_ptr, &format!("cap_raw_{}", cap_name))
            {
                // Convert i64 back to pointer (this is the outer alloca pointer)
                if let Ok(outer_alloca) = ctx.builder.build_int_to_ptr(
                    loaded_i64.into_int_value(),
                    ptr_ty,
                    &format!("cap_ptr_{}", cap_name),
                ) {
                    // Get the type of the local alloca we're replacing
                    let local_ty = ctx.get_local_type(cap_name).unwrap_or(i64_ty.into());
                    // Replace the spawn function's local alloca with the outer pointer
                    ctx.replace_local_ptr(cap_name, outer_alloca, local_ty);
                }
            }
        }
    }

    // Free the env struct — it only held pointer values, not the actual data
    let free_fn = get_or_declare_free(ctx);
    let _ = ctx
        .builder
        .build_call(free_fn, &[env_ptr.into()], "env_free");
}

/// `malloc(size: i64) -> ptr`
fn get_or_declare_malloc<'ctx>(ctx: &CodegenContext<'ctx>) -> inkwell::values::FunctionValue<'ctx> {
    if let Some(f) = ctx.module.get_function(ffi_names::MALLOC) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let i64_ty = ctx.context.i64_type();
    let fn_ty = ptr_ty.fn_type(&[i64_ty.into()], false);
    ctx.module
        .add_function(ffi_names::MALLOC, fn_ty, Some(Linkage::External))
}

/// `free(ptr: ptr)` — void return
fn get_or_declare_free<'ctx>(ctx: &CodegenContext<'ctx>) -> inkwell::values::FunctionValue<'ctx> {
    if let Some(f) = ctx.module.get_function(ffi_names::FREE) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let void_ty = ctx.context.void_type();
    let fn_ty = void_ty.fn_type(&[ptr_ty.into()], false);
    ctx.module
        .add_function(ffi_names::FREE, fn_ty, Some(Linkage::External))
}
