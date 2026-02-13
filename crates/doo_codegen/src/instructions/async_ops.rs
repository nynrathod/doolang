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
                    ctx.set_temp(dest, result_ptr);
                    Some(result_ptr)
                } else {
                    None
                }
            }

            // ================================================================
            // Spawn: spawn a function as an async task
            // doo_spawn(fn_ptr: extern "C" fn() -> *mut DooResult) -> *mut TaskHandle
            // ================================================================
            MirInstrKind::Spawn { dest, func } => {
                // Get the function value for the closure/function to spawn
                let func_val = ctx.get_function(func)?;
                let func_ptr = func_val.as_global_value().as_pointer_value();

                let spawn_fn = get_or_declare_doo_spawn(ctx);
                let call_site = ctx
                    .builder
                    .build_call(spawn_fn, &[func_ptr.into()], "spawn_handle")
                    .ok()?;

                if let Some(handle_ptr) = call_site.try_as_basic_value().basic() {
                    ctx.set_temp(dest, handle_ptr);
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
                    ctx.set_temp(dest, scope_ptr);
                    Some(scope_ptr)
                } else {
                    None
                }
            }

            // ================================================================
            // ScopeSpawn: spawn a task within a scope
            // doo_scope_spawn(scope: *mut ScopeHandle, fn_ptr: extern "C" fn() -> *mut DooResult)
            // ================================================================
            MirInstrKind::ScopeSpawn { scope, func } => {
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
                let func_val = ctx.get_function(func)?;
                let func_ptr = func_val.as_global_value().as_pointer_value();

                let scope_spawn_fn = get_or_declare_doo_scope_spawn(ctx);
                let _ = ctx
                    .builder
                    .build_call(
                        scope_spawn_fn,
                        &[scope_ptr.into(), func_ptr.into()],
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
                    ctx.set_temp(dest, result_ptr);
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
    const NAME: &str = "doo_task_await";
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let fn_ty = ptr_ty.fn_type(&[ptr_ty.into()], false);
    ctx.module.add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_spawn(fn_ptr: ptr) -> ptr`
fn get_or_declare_doo_spawn<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = "doo_spawn";
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let fn_ty = ptr_ty.fn_type(&[ptr_ty.into()], false);
    ctx.module.add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_scope_create() -> ptr`
fn get_or_declare_doo_scope_create<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = "doo_scope_create";
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let fn_ty = ptr_ty.fn_type(&[], false);
    ctx.module.add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_scope_spawn(scope: ptr, fn_ptr: ptr)` — void return
fn get_or_declare_doo_scope_spawn<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = "doo_scope_spawn";
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let void_ty = ctx.context.void_type();
    let fn_ty = void_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
    ctx.module.add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_scope_wait(scope: ptr) -> ptr`
fn get_or_declare_doo_scope_wait<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = "doo_scope_wait";
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let fn_ty = ptr_ty.fn_type(&[ptr_ty.into()], false);
    ctx.module.add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_runtime_init() -> i32`
pub fn get_or_declare_doo_runtime_init<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = "doo_runtime_init";
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let i32_ty = ctx.i32_type();
    let fn_ty = i32_ty.fn_type(&[], false);
    ctx.module.add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_runtime_block_on(fn_ptr: ptr) -> ptr`
pub fn get_or_declare_doo_runtime_block_on<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = "doo_runtime_block_on";
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let fn_ty = ptr_ty.fn_type(&[ptr_ty.into()], false);
    ctx.module.add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_sleep(ms: i64) -> ptr`
pub fn get_or_declare_doo_sleep<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = "doo_sleep";
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let i64_ty = ctx.i64_type();
    let fn_ty = ptr_ty.fn_type(&[i64_ty.into()], false);
    ctx.module.add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_sleep_async(ms: i64) -> ptr`
pub fn get_or_declare_doo_sleep_async<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = "doo_sleep_async";
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let i64_ty = ctx.i64_type();
    let fn_ty = ptr_ty.fn_type(&[i64_ty.into()], false);
    ctx.module.add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_timeout(ms: i64, fn_ptr: ptr) -> ptr`
pub fn get_or_declare_doo_timeout<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = "doo_timeout";
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let i64_ty = ctx.i64_type();
    let fn_ty = ptr_ty.fn_type(&[i64_ty.into(), ptr_ty.into()], false);
    ctx.module.add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_spawn_detach(fn_ptr: ptr)` — void return
pub fn get_or_declare_doo_spawn_detach<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = "doo_spawn_detach";
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let void_ty = ctx.context.void_type();
    let fn_ty = void_ty.fn_type(&[ptr_ty.into()], false);
    ctx.module.add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_task_cancel(handle: ptr)` — void return
pub fn get_or_declare_doo_task_cancel<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = "doo_task_cancel";
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let void_ty = ctx.context.void_type();
    let fn_ty = void_ty.fn_type(&[ptr_ty.into()], false);
    ctx.module.add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_task_free(handle: ptr)` — void return
pub fn get_or_declare_doo_task_free<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = "doo_task_free";
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let void_ty = ctx.context.void_type();
    let fn_ty = void_ty.fn_type(&[ptr_ty.into()], false);
    ctx.module.add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_scope_free(scope: ptr)` — void return
pub fn get_or_declare_doo_scope_free<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = "doo_scope_free";
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let void_ty = ctx.context.void_type();
    let fn_ty = void_ty.fn_type(&[ptr_ty.into()], false);
    ctx.module.add_function(NAME, fn_ty, Some(Linkage::External))
}

/// `doo_spawn_blocking(fn_ptr: ptr) -> ptr` — for CPU-heavy work
pub fn get_or_declare_doo_spawn_blocking<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = "doo_spawn_blocking";
    if let Some(f) = ctx.module.get_function(NAME) {
        return f;
    }
    let ptr_ty = ctx.ptr_type();
    let fn_ty = ptr_ty.fn_type(&[ptr_ty.into()], false);
    ctx.module.add_function(NAME, fn_ty, Some(Linkage::External))
}
