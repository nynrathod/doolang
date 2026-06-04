//! String Builtin Methods - Complete Implementation
//! All 16+ methods from legacy: len, charAt, substring, concat, indexOf, toUpper, toLower,
//! replace, trim, reverse, contains, startsWith, endsWith, repeat, charCode, countSubstr

use crate::context::CodegenContext;
use doo_core::constants::ffi_names;
use doo_core::types::builtin;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

pub struct StringBuiltins;

impl StringBuiltins {
    /// Dispatch string method call
    pub fn dispatch<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        dest: Option<&str>,
        receiver_ptr: PointerValue<'ctx>,
        method: &str,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        // CRITICAL: Null-coerce the receiver before dispatching to any method.
        // String methods call strlen/strcmp/memcpy which have LLVM-inferred
        // nonnull+dereferenceable(1) attributes. A null receiver causes UB that
        // LLVM O3 exploits to mark the entire calling function as noreturn.
        let safe_receiver = crate::utils::null_coerce_str(ctx, receiver_ptr);
        let result = match method {
            "len" => Self::emit_len(ctx, safe_receiver),
            "charAt" => Self::emit_char_at(ctx, safe_receiver, args),
            "substring" => Self::emit_substring(ctx, safe_receiver, args),
            "concat" => Self::emit_concat(ctx, safe_receiver, args),
            "indexOf" => Self::emit_index_of(ctx, safe_receiver, args),
            "toUpper" => Self::emit_case_convert(ctx, safe_receiver, true),
            "toLower" => Self::emit_case_convert(ctx, safe_receiver, false),
            "replace" => Self::emit_replace(ctx, safe_receiver, args),
            "trim" => Self::emit_trim(ctx, safe_receiver),
            "reverse" => Self::emit_reverse(ctx, safe_receiver),
            "contains" => Self::emit_contains(ctx, safe_receiver, args),
            "startsWith" => Self::emit_starts_with(ctx, safe_receiver, args),
            "endsWith" => Self::emit_ends_with(ctx, safe_receiver, args),
            "repeat" => Self::emit_repeat(ctx, safe_receiver, args),
            "charCode" => Self::emit_char_code(ctx, safe_receiver),
            "countSubstr" => Self::emit_count_substr(ctx, safe_receiver, args),
            "split" => Self::emit_split(ctx, safe_receiver, args),
            _ => return None,
        };

        // Register array element type for methods that return arrays
        if method == "split" {
            if let Some(dest_name) = dest {
                ctx.array_element_types
                    .insert(dest_name.to_string(), builtin::STR);
            }
        }

        if let (Some(val), Some(dest_name)) = (result, dest) {
            ctx.set_temp(dest_name, val);
        }

        result
    }

    // =========================================================================
    // len() -> Int
    // =========================================================================
    fn emit_len<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        str_ptr: PointerValue<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        let strlen = get_or_declare_strlen(ctx);
        let len_i64 = ctx
            .builder
            .build_call(strlen, &[str_ptr.into()], "strlen")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();
        // Return i64 directly since Doo Int is i64
        Some(len_i64.into())
    }

    // =========================================================================
    // charAt(index: Int) -> Str
    // =========================================================================
    fn emit_char_at<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        str_ptr: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }
        let index = args[0].into_int_value();
        let malloc = get_or_declare_malloc(ctx);

        // Allocate 2 bytes for [char, null]
        let size = ctx.context.i64_type().const_int(2, false);
        let result_ptr = ctx
            .builder
            .build_call(malloc, &[size.into()], "char_str")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_pointer_value();

        // Get character at index (byte-wise)
        let idx_i64 = ctx
            .builder
            .build_int_z_extend(index, ctx.context.i64_type(), "idx")
            .ok()?;
        let char_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), str_ptr, &[idx_i64], "char_ptr")
                .ok()?
        };
        let char_val = ctx
            .builder
            .build_load(ctx.context.i8_type(), char_ptr, "char")
            .ok()?;

        ctx.builder.build_store(result_ptr, char_val).ok()?;
        let null_ptr = unsafe {
            ctx.builder
                .build_gep(
                    ctx.context.i8_type(),
                    result_ptr,
                    &[ctx.context.i64_type().const_int(1, false)],
                    "null_ptr",
                )
                .ok()?
        };
        ctx.builder
            .build_store(null_ptr, ctx.context.i8_type().const_int(0, false))
            .ok()?;

        Some(result_ptr.into())
    }

    // =========================================================================
    // substring(start: Int, end: Int) -> Str
    // =========================================================================
    fn emit_substring<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        str_ptr: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.len() < 2 {
            return None;
        }
        let start = args[0].into_int_value();
        let end = args[1].into_int_value();
        let malloc = get_or_declare_malloc(ctx);
        let memcpy = get_or_declare_memcpy(ctx);

        // Calculate length
        let len = ctx.builder.build_int_sub(end, start, "len").ok()?;
        let len_i64 = ctx
            .builder
            .build_int_z_extend(len, ctx.context.i64_type(), "len64")
            .ok()?;
        let size = ctx
            .builder
            .build_int_add(len_i64, ctx.context.i64_type().const_int(1, false), "size")
            .ok()?;

        let result_ptr = ctx
            .builder
            .build_call(malloc, &[size.into()], "substr")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_pointer_value();

        // Copy substring
        let start_i64 = ctx
            .builder
            .build_int_z_extend(start, ctx.context.i64_type(), "start64")
            .ok()?;
        let src_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), str_ptr, &[start_i64], "src")
                .ok()?
        };
        ctx.builder
            .build_call(
                memcpy,
                &[result_ptr.into(), src_ptr.into(), len_i64.into()],
                "",
            )
            .ok()?;

        // Null terminate
        let null_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), result_ptr, &[len_i64], "null_ptr")
                .ok()?
        };
        ctx.builder
            .build_store(null_ptr, ctx.context.i8_type().const_int(0, false))
            .ok()?;

        Some(result_ptr.into())
    }

    // =========================================================================
    // concat(other: Str) -> Str
    // =========================================================================
    fn emit_concat<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        str_ptr: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }
        let other_ptr = args[0].into_pointer_value();
        let other_ptr = crate::utils::null_coerce_str(ctx, other_ptr);
        let strlen = get_or_declare_strlen(ctx);
        let malloc = get_or_declare_malloc(ctx);
        let memcpy = get_or_declare_memcpy(ctx);

        // Get lengths
        let len1 = ctx
            .builder
            .build_call(strlen, &[str_ptr.into()], "len1")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();
        let len2 = ctx
            .builder
            .build_call(strlen, &[other_ptr.into()], "len2")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();

        // Allocate len1 + len2 + 1
        let total_len = ctx.builder.build_int_add(len1, len2, "total").ok()?;
        let size = ctx
            .builder
            .build_int_add(
                total_len,
                ctx.context.i64_type().const_int(1, false),
                "size",
            )
            .ok()?;

        let result_ptr = ctx
            .builder
            .build_call(malloc, &[size.into()], "concat")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_pointer_value();

        // Copy first string
        ctx.builder
            .build_call(
                memcpy,
                &[result_ptr.into(), str_ptr.into(), len1.into()],
                "",
            )
            .ok()?;

        // Copy second string
        let dest2 = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), result_ptr, &[len1], "dest2")
                .ok()?
        };
        let len2_plus_null = ctx
            .builder
            .build_int_add(len2, ctx.context.i64_type().const_int(1, false), "len2p1")
            .ok()?;
        ctx.builder
            .build_call(
                memcpy,
                &[dest2.into(), other_ptr.into(), len2_plus_null.into()],
                "",
            )
            .ok()?;

        Some(result_ptr.into())
    }

    // =========================================================================
    // indexOf(needle: Str) -> Int
    // =========================================================================
    fn emit_index_of<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        str_ptr: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }
        let needle_ptr = args[0].into_pointer_value();
        let needle_ptr = crate::utils::null_coerce_str(ctx, needle_ptr);
        let strstr = get_or_declare_strstr(ctx);

        let found_ptr = ctx
            .builder
            .build_call(strstr, &[str_ptr.into(), needle_ptr.into()], "found")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_pointer_value();

        // Calculate index: found_ptr - str_ptr, or -1 if not found
        let is_null = ctx.builder.build_is_null(found_ptr, "is_null").ok()?;

        let str_int = ctx
            .builder
            .build_ptr_to_int(str_ptr, ctx.context.i64_type(), "str_int")
            .ok()?;
        let found_int = ctx
            .builder
            .build_ptr_to_int(found_ptr, ctx.context.i64_type(), "found_int")
            .ok()?;
        let diff = ctx.builder.build_int_sub(found_int, str_int, "diff").ok()?;

        // Return i64 with proper -1 for not found
        let neg_one = ctx.context.i64_type().const_int((-1_i64) as u64, true);
        let result = ctx
            .builder
            .build_select(is_null, neg_one, diff, "indexOf")
            .ok()?;

        Some(result)
    }

    // =========================================================================
    // toUpper() / toLower() -> Str
    // =========================================================================
    fn emit_case_convert<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        str_ptr: PointerValue<'ctx>,
        to_upper: bool,
    ) -> Option<BasicValueEnum<'ctx>> {
        let strlen = get_or_declare_strlen(ctx);
        let malloc = get_or_declare_malloc(ctx);

        let len = ctx
            .builder
            .build_call(strlen, &[str_ptr.into()], "len")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();
        let size = ctx
            .builder
            .build_int_add(len, ctx.context.i64_type().const_int(1, false), "size")
            .ok()?;

        let result_ptr = ctx
            .builder
            .build_call(malloc, &[size.into()], "case_str")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_pointer_value();

        // Generate loop to convert each character
        let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
        let loop_bb = ctx.context.append_basic_block(current_fn, "case_loop");
        let body_bb = ctx.context.append_basic_block(current_fn, "case_body");
        let after_bb = ctx.context.append_basic_block(current_fn, "case_after");

        let idx_alloca = ctx.alloca_in_entry_block(ctx.context.i64_type(), "idx")?;
        ctx.builder
            .build_store(idx_alloca, ctx.context.i64_type().const_zero())
            .ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        // Loop condition
        ctx.builder.position_at_end(loop_bb);
        let idx = ctx
            .builder
            .build_load(ctx.context.i64_type(), idx_alloca, "idx")
            .ok()?
            .into_int_value();
        let cmp = ctx
            .builder
            .build_int_compare(IntPredicate::ULT, idx, len, "cmp")
            .ok()?;
        ctx.builder
            .build_conditional_branch(cmp, body_bb, after_bb)
            .ok()?;

        // Loop body
        ctx.builder.position_at_end(body_bb);
        let src_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), str_ptr, &[idx], "src")
                .ok()?
        };
        let dst_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), result_ptr, &[idx], "dst")
                .ok()?
        };
        let char_val = ctx
            .builder
            .build_load(ctx.context.i8_type(), src_ptr, "char")
            .ok()?
            .into_int_value();

        // Convert case
        let (range_start, range_end, _offset) = if to_upper {
            (97u64, 122u64, -32i64 as u64) // 'a'-'z' -> subtract 32
        } else {
            (65u64, 90u64, 32u64) // 'A'-'Z' -> add 32
        };

        let is_in_range_start = ctx
            .builder
            .build_int_compare(
                IntPredicate::UGE,
                char_val,
                ctx.context.i8_type().const_int(range_start, false),
                "ge",
            )
            .ok()?;
        let is_in_range_end = ctx
            .builder
            .build_int_compare(
                IntPredicate::ULE,
                char_val,
                ctx.context.i8_type().const_int(range_end, false),
                "le",
            )
            .ok()?;
        let is_in_range = ctx
            .builder
            .build_and(is_in_range_start, is_in_range_end, "in_range")
            .ok()?;

        let converted = if to_upper {
            ctx.builder
                .build_int_sub(char_val, ctx.context.i8_type().const_int(32, false), "conv")
                .ok()?
        } else {
            ctx.builder
                .build_int_add(char_val, ctx.context.i8_type().const_int(32, false), "conv")
                .ok()?
        };
        let final_char = ctx
            .builder
            .build_select(is_in_range, converted, char_val, "final")
            .ok()?;
        ctx.builder.build_store(dst_ptr, final_char).ok()?;

        let next_idx = ctx
            .builder
            .build_int_add(idx, ctx.context.i64_type().const_int(1, false), "next")
            .ok()?;
        ctx.builder.build_store(idx_alloca, next_idx).ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        // After loop - null terminate
        ctx.builder.position_at_end(after_bb);
        let null_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), result_ptr, &[len], "null")
                .ok()?
        };
        ctx.builder
            .build_store(null_ptr, ctx.context.i8_type().const_int(0, false))
            .ok()?;

        Some(result_ptr.into())
    }

    // =========================================================================
    // replace(old: Str, new: Str) -> Str (replaces ALL occurrences)
    // Delegates to doo_string_replace_all in the runtime for correct all-occurrence behavior.
    // =========================================================================
    fn emit_replace<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        str_ptr: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.len() < 2 {
            return None;
        }
        if !args[0].is_pointer_value() || !args[1].is_pointer_value() {
            return None;
        }
        let old_ptr = args[0].into_pointer_value();
        let old_ptr = crate::utils::null_coerce_str(ctx, old_ptr);
        let new_ptr = args[1].into_pointer_value();
        let new_ptr = crate::utils::null_coerce_str(ctx, new_ptr);

        let replace_all_fn = get_or_declare_string_replace_all(ctx);

        let result = ctx
            .builder
            .build_call(
                replace_all_fn,
                &[str_ptr.into(), old_ptr.into(), new_ptr.into()],
                "replace_all",
            )
            .ok()?
            .try_as_basic_value()
            .basic()?;

        Some(result)
    }

    // =========================================================================
    // trim() -> Str
    // =========================================================================
    fn emit_trim<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        str_ptr: PointerValue<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        let strlen = get_or_declare_strlen(ctx);
        let malloc = get_or_declare_malloc(ctx);
        let memcpy = get_or_declare_memcpy(ctx);

        let len = ctx
            .builder
            .build_call(strlen, &[str_ptr.into()], "len")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();

        let current_fn = ctx.builder.get_insert_block()?.get_parent()?;

        // start scan
        let start_alloca = ctx.alloca_in_entry_block(ctx.context.i64_type(), "start")?;
        ctx.builder
            .build_store(start_alloca, ctx.context.i64_type().const_zero())
            .ok()?;
        let start_loop = ctx
            .context
            .append_basic_block(current_fn, "trim_start_loop");
        let start_body = ctx
            .context
            .append_basic_block(current_fn, "trim_start_body");
        let start_end = ctx.context.append_basic_block(current_fn, "trim_start_end");
        ctx.builder.build_unconditional_branch(start_loop).ok()?;

        ctx.builder.position_at_end(start_loop);
        let sidx = ctx
            .builder
            .build_load(ctx.context.i64_type(), start_alloca, "sidx")
            .ok()?
            .into_int_value();
        let scond = ctx
            .builder
            .build_int_compare(IntPredicate::ULT, sidx, len, "scond")
            .ok()?;
        ctx.builder
            .build_conditional_branch(scond, start_body, start_end)
            .ok()?;

        ctx.builder.position_at_end(start_body);
        let ch_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), str_ptr, &[sidx], "ch_ptr")
        }
        .ok()?;
        let ch = ctx
            .builder
            .build_load(ctx.context.i8_type(), ch_ptr, "ch")
            .ok()?
            .into_int_value();
        let is_space = Self::build_is_ws(ctx, ch)?;
        let next_s = ctx
            .builder
            .build_int_add(sidx, ctx.context.i64_type().const_int(1, false), "next")
            .ok()?;
        let cont_bb = ctx
            .context
            .append_basic_block(current_fn, "trim_start_cont");
        ctx.builder
            .build_conditional_branch(is_space, cont_bb, start_end)
            .ok()?;

        ctx.builder.position_at_end(cont_bb);
        ctx.builder.build_store(start_alloca, next_s).ok()?;
        ctx.builder.build_unconditional_branch(start_loop).ok()?;

        // end scan
        ctx.builder.position_at_end(start_end);
        let end_alloca = ctx.alloca_in_entry_block(ctx.context.i64_type(), "end")?;
        ctx.builder.build_store(end_alloca, len).ok()?;
        let end_loop = ctx.context.append_basic_block(current_fn, "trim_end_loop");
        let end_body = ctx.context.append_basic_block(current_fn, "trim_end_body");
        let end_end = ctx.context.append_basic_block(current_fn, "trim_end_end");
        ctx.builder.build_unconditional_branch(end_loop).ok()?;

        ctx.builder.position_at_end(end_loop);
        let eidx = ctx
            .builder
            .build_load(ctx.context.i64_type(), end_alloca, "eidx")
            .ok()?
            .into_int_value();
        let start_idx = ctx
            .builder
            .build_load(ctx.context.i64_type(), start_alloca, "start")
            .ok()?
            .into_int_value();
        let has_more = ctx
            .builder
            .build_int_compare(IntPredicate::UGT, eidx, start_idx, "has_more")
            .ok()?;
        ctx.builder
            .build_conditional_branch(has_more, end_body, end_end)
            .ok()?;

        ctx.builder.position_at_end(end_body);
        let eidx_m1 = ctx
            .builder
            .build_int_sub(eidx, ctx.context.i64_type().const_int(1, false), "eidx_m1")
            .ok()?;
        let ch_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), str_ptr, &[eidx_m1], "ch_ptr")
        }
        .ok()?;
        let ch = ctx
            .builder
            .build_load(ctx.context.i8_type(), ch_ptr, "ch")
            .ok()?
            .into_int_value();
        let is_space = Self::build_is_ws(ctx, ch)?;
        let cont_bb = ctx.context.append_basic_block(current_fn, "trim_end_cont");
        ctx.builder
            .build_conditional_branch(is_space, cont_bb, end_end)
            .ok()?;
        ctx.builder.position_at_end(cont_bb);
        ctx.builder.build_store(end_alloca, eidx_m1).ok()?;
        ctx.builder.build_unconditional_branch(end_loop).ok()?;

        // build substring
        ctx.builder.position_at_end(end_end);
        let start_idx = ctx
            .builder
            .build_load(ctx.context.i64_type(), start_alloca, "start")
            .ok()?
            .into_int_value();
        let end_idx = ctx
            .builder
            .build_load(ctx.context.i64_type(), end_alloca, "end")
            .ok()?
            .into_int_value();
        let out_len = ctx
            .builder
            .build_int_sub(end_idx, start_idx, "out_len")
            .ok()?;
        let size = ctx
            .builder
            .build_int_add(out_len, ctx.context.i64_type().const_int(1, false), "size")
            .ok()?;
        let out_ptr = ctx
            .builder
            .build_call(malloc, &[size.into()], "trim_out")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_pointer_value();
        let src_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), str_ptr, &[start_idx], "src")
        }
        .ok()?;
        ctx.builder
            .build_call(
                memcpy,
                &[out_ptr.into(), src_ptr.into(), out_len.into()],
                "",
            )
            .ok()?;
        let nul_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), out_ptr, &[out_len], "nul")
        }
        .ok()?;
        ctx.builder
            .build_store(nul_ptr, ctx.context.i8_type().const_int(0, false))
            .ok()?;
        Some(out_ptr.into())
    }

    fn build_is_ws<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        ch: IntValue<'ctx>,
    ) -> Option<IntValue<'ctx>> {
        let i8 = ctx.context.i8_type();
        let sp = i8.const_int(32, false);
        let tab = i8.const_int(9, false);
        let nl = i8.const_int(10, false);
        let cr = i8.const_int(13, false);

        let is_sp = ctx
            .builder
            .build_int_compare(IntPredicate::EQ, ch, sp, "is_sp")
            .ok()?;
        let is_tab = ctx
            .builder
            .build_int_compare(IntPredicate::EQ, ch, tab, "is_tab")
            .ok()?;
        let is_nl = ctx
            .builder
            .build_int_compare(IntPredicate::EQ, ch, nl, "is_nl")
            .ok()?;
        let is_cr = ctx
            .builder
            .build_int_compare(IntPredicate::EQ, ch, cr, "is_cr")
            .ok()?;
        let t1 = ctx.builder.build_or(is_sp, is_tab, "t1").ok()?;
        let t2 = ctx.builder.build_or(is_nl, is_cr, "t2").ok()?;
        ctx.builder.build_or(t1, t2, "is_ws").ok()
    }

    // =========================================================================
    // reverse() -> Str
    // =========================================================================
    fn emit_reverse<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        str_ptr: PointerValue<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        let strlen = get_or_declare_strlen(ctx);
        let malloc = get_or_declare_malloc(ctx);

        let len = ctx
            .builder
            .build_call(strlen, &[str_ptr.into()], "len")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();
        let size = ctx
            .builder
            .build_int_add(len, ctx.context.i64_type().const_int(1, false), "size")
            .ok()?;

        let result_ptr = ctx
            .builder
            .build_call(malloc, &[size.into()], "rev_str")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_pointer_value();

        // Loop to copy in reverse
        let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
        let loop_bb = ctx.context.append_basic_block(current_fn, "rev_loop");
        let body_bb = ctx.context.append_basic_block(current_fn, "rev_body");
        let after_bb = ctx.context.append_basic_block(current_fn, "rev_after");

        let idx_alloca = ctx.alloca_in_entry_block(ctx.context.i64_type(), "idx")?;
        ctx.builder
            .build_store(idx_alloca, ctx.context.i64_type().const_zero())
            .ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        ctx.builder.position_at_end(loop_bb);
        let idx = ctx
            .builder
            .build_load(ctx.context.i64_type(), idx_alloca, "idx")
            .ok()?
            .into_int_value();
        let cmp = ctx
            .builder
            .build_int_compare(IntPredicate::ULT, idx, len, "cmp")
            .ok()?;
        ctx.builder
            .build_conditional_branch(cmp, body_bb, after_bb)
            .ok()?;

        ctx.builder.position_at_end(body_bb);
        // src = str_ptr[len - 1 - idx], dst = result_ptr[idx]
        let rev_idx = ctx
            .builder
            .build_int_sub(len, ctx.context.i64_type().const_int(1, false), "len_m1")
            .ok()?;
        let rev_idx = ctx.builder.build_int_sub(rev_idx, idx, "rev_idx").ok()?;
        let src_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), str_ptr, &[rev_idx], "src")
                .ok()?
        };
        let dst_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), result_ptr, &[idx], "dst")
                .ok()?
        };
        let char_val = ctx
            .builder
            .build_load(ctx.context.i8_type(), src_ptr, "char")
            .ok()?;
        ctx.builder.build_store(dst_ptr, char_val).ok()?;

        let next_idx = ctx
            .builder
            .build_int_add(idx, ctx.context.i64_type().const_int(1, false), "next")
            .ok()?;
        ctx.builder.build_store(idx_alloca, next_idx).ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        ctx.builder.position_at_end(after_bb);
        let null_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), result_ptr, &[len], "null")
                .ok()?
        };
        ctx.builder
            .build_store(null_ptr, ctx.context.i8_type().const_int(0, false))
            .ok()?;

        Some(result_ptr.into())
    }

    // =========================================================================
    // contains(needle: Str) -> Bool
    // =========================================================================
    fn emit_contains<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        str_ptr: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }
        let needle_ptr = args[0].into_pointer_value();
        let needle_ptr = crate::utils::null_coerce_str(ctx, needle_ptr);
        let strstr = get_or_declare_strstr(ctx);

        let found = ctx
            .builder
            .build_call(strstr, &[str_ptr.into(), needle_ptr.into()], "found")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_pointer_value();

        let is_null = ctx.builder.build_is_null(found, "is_null").ok()?;

        // is_null is true when NOT found, so we negate: contains = !is_null
        let result = ctx.builder.build_not(is_null, "contains").ok()?;

        Some(result.into())
    }

    // =========================================================================
    // startsWith(prefix: Str) -> Bool
    // =========================================================================
    fn emit_starts_with<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        str_ptr: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }
        let prefix_ptr = args[0].into_pointer_value();
        let prefix_ptr = crate::utils::null_coerce_str(ctx, prefix_ptr);
        let strlen = get_or_declare_strlen(ctx);
        let strncmp = get_or_declare_strncmp(ctx);

        let prefix_len = ctx
            .builder
            .build_call(strlen, &[prefix_ptr.into()], "plen")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();

        let cmp = ctx
            .builder
            .build_call(
                strncmp,
                &[str_ptr.into(), prefix_ptr.into(), prefix_len.into()],
                "cmp",
            )
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();

        let is_eq = ctx
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                cmp,
                ctx.context.i32_type().const_int(0, false),
                "eq",
            )
            .ok()?;

        // Return i1 (bool) directly - is_eq is already an i1 from the comparison
        Some(is_eq.into())
    }

    // =========================================================================
    // endsWith(suffix: Str) -> Bool
    // =========================================================================
    fn emit_ends_with<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        str_ptr: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }
        let suffix_ptr = args[0].into_pointer_value();
        let suffix_ptr = crate::utils::null_coerce_str(ctx, suffix_ptr);
        let strlen = get_or_declare_strlen(ctx);
        let strncmp = get_or_declare_strncmp(ctx);

        let str_len = ctx
            .builder
            .build_call(strlen, &[str_ptr.into()], "slen")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();

        let suffix_len = ctx
            .builder
            .build_call(strlen, &[suffix_ptr.into()], "suflen")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();

        let offset = ctx
            .builder
            .build_int_sub(str_len, suffix_len, "offset")
            .ok()?;

        let start_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), str_ptr, &[offset], "start")
                .ok()?
        };

        let cmp = ctx
            .builder
            .build_call(
                strncmp,
                &[start_ptr.into(), suffix_ptr.into(), suffix_len.into()],
                "cmp",
            )
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();

        let is_eq = ctx
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                cmp,
                ctx.context.i32_type().const_int(0, false),
                "eq",
            )
            .ok()?;

        // Return i1 (bool) directly - is_eq is already an i1 from the comparison
        Some(is_eq.into())
    }

    // =========================================================================
    // repeat(n: Int) -> Str
    // =========================================================================
    fn emit_repeat<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        str_ptr: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }
        let n = args[0].into_int_value();
        let strlen = get_or_declare_strlen(ctx);
        let malloc = get_or_declare_malloc(ctx);
        let memcpy = get_or_declare_memcpy(ctx);

        let len = ctx
            .builder
            .build_call(strlen, &[str_ptr.into()], "len")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();

        let n_i64 = ctx
            .builder
            .build_int_z_extend(n, ctx.context.i64_type(), "n64")
            .ok()?;
        let total_len = ctx.builder.build_int_mul(len, n_i64, "total").ok()?;
        let size = ctx
            .builder
            .build_int_add(
                total_len,
                ctx.context.i64_type().const_int(1, false),
                "size",
            )
            .ok()?;

        let result_ptr = ctx
            .builder
            .build_call(malloc, &[size.into()], "repeat")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_pointer_value();

        // Loop to copy n times
        let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
        let loop_bb = ctx.context.append_basic_block(current_fn, "rep_loop");
        let body_bb = ctx.context.append_basic_block(current_fn, "rep_body");
        let after_bb = ctx.context.append_basic_block(current_fn, "rep_after");

        let idx_alloca = ctx.alloca_in_entry_block(ctx.context.i64_type(), "idx")?;
        ctx.builder
            .build_store(idx_alloca, ctx.context.i64_type().const_zero())
            .ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        ctx.builder.position_at_end(loop_bb);
        let idx = ctx
            .builder
            .build_load(ctx.context.i64_type(), idx_alloca, "idx")
            .ok()?
            .into_int_value();
        let cmp = ctx
            .builder
            .build_int_compare(IntPredicate::ULT, idx, n_i64, "cmp")
            .ok()?;
        ctx.builder
            .build_conditional_branch(cmp, body_bb, after_bb)
            .ok()?;

        ctx.builder.position_at_end(body_bb);
        let offset = ctx.builder.build_int_mul(idx, len, "off").ok()?;
        let dst = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), result_ptr, &[offset], "dst")
                .ok()?
        };
        ctx.builder
            .build_call(memcpy, &[dst.into(), str_ptr.into(), len.into()], "")
            .ok()?;

        let next_idx = ctx
            .builder
            .build_int_add(idx, ctx.context.i64_type().const_int(1, false), "next")
            .ok()?;
        ctx.builder.build_store(idx_alloca, next_idx).ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        ctx.builder.position_at_end(after_bb);
        let null_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), result_ptr, &[total_len], "null")
                .ok()?
        };
        ctx.builder
            .build_store(null_ptr, ctx.context.i8_type().const_int(0, false))
            .ok()?;

        Some(result_ptr.into())
    }

    // =========================================================================
    // charCode() -> Int (ASCII code of first character)
    // =========================================================================
    fn emit_char_code<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        str_ptr: PointerValue<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        let char_val = ctx
            .builder
            .build_load(ctx.context.i8_type(), str_ptr, "char")
            .ok()?
            .into_int_value();
        let code = ctx
            .builder
            .build_int_z_extend(char_val, ctx.context.i32_type(), "code")
            .ok()?;
        Some(code.into())
    }

    // =========================================================================
    // countSubstr(needle: Str) -> Int
    // =========================================================================
    fn emit_count_substr<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        str_ptr: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }
        let needle_ptr = args[0].into_pointer_value();
        let needle_ptr = crate::utils::null_coerce_str(ctx, needle_ptr);
        let strstr = get_or_declare_strstr(ctx);
        let strlen = get_or_declare_strlen(ctx);

        let needle_len = ctx
            .builder
            .build_call(strlen, &[needle_ptr.into()], "nlen")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();

        // Loop to count occurrences
        let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
        let loop_bb = ctx.context.append_basic_block(current_fn, "count_loop");
        let body_bb = ctx.context.append_basic_block(current_fn, "count_body");
        let after_bb = ctx.context.append_basic_block(current_fn, "count_after");

        let ptr_alloca = ctx.alloca_in_entry_block(
            ctx.context.i8_type().ptr_type(AddressSpace::default()),
            "ptr",
        )?;
        let count_alloca = ctx.alloca_in_entry_block(ctx.context.i32_type(), "count")?;
        ctx.builder.build_store(ptr_alloca, str_ptr).ok()?;
        ctx.builder
            .build_store(count_alloca, ctx.context.i32_type().const_zero())
            .ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        ctx.builder.position_at_end(loop_bb);
        let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let curr_ptr = ctx
            .builder
            .build_load(ptr_type, ptr_alloca, "curr")
            .ok()?
            .into_pointer_value();
        let found = ctx
            .builder
            .build_call(strstr, &[curr_ptr.into(), needle_ptr.into()], "found")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_pointer_value();
        let is_null = ctx.builder.build_is_null(found, "is_null").ok()?;
        ctx.builder
            .build_conditional_branch(is_null, after_bb, body_bb)
            .ok()?;

        ctx.builder.position_at_end(body_bb);
        let count = ctx
            .builder
            .build_load(ctx.context.i32_type(), count_alloca, "cnt")
            .ok()?
            .into_int_value();
        let new_count = ctx
            .builder
            .build_int_add(count, ctx.context.i32_type().const_int(1, false), "new_cnt")
            .ok()?;
        ctx.builder.build_store(count_alloca, new_count).ok()?;

        // Move pointer past the found occurrence
        let next_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), found, &[needle_len], "next")
                .ok()?
        };
        ctx.builder.build_store(ptr_alloca, next_ptr).ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        ctx.builder.position_at_end(after_bb);
        let final_count = ctx
            .builder
            .build_load(ctx.context.i32_type(), count_alloca, "final")
            .ok()?;

        Some(final_count)
    }

    // =========================================================================
    // split(delimiter: Str) -> [Str]
    // =========================================================================
    fn emit_split<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        str_ptr: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }
        let delim_ptr = args[0].into_pointer_value();
        let split_fn = get_or_declare_string_split(ctx);
        let result = ctx
            .builder
            .build_call(split_fn, &[str_ptr.into(), delim_ptr.into()], "split_arr")
            .ok()?
            .try_as_basic_value()
            .basic()?;
        Some(result)
    }
}

// =============================================================================
// libc Function Declarations
// =============================================================================

fn get_or_declare_strlen<'ctx>(ctx: &CodegenContext<'ctx>) -> inkwell::values::FunctionValue<'ctx> {
    ctx.module
        .get_function(ffi_names::STRLEN)
        .unwrap_or_else(|| {
            let fn_type = ctx.context.i64_type().fn_type(
                &[ctx
                    .context
                    .i8_type()
                    .ptr_type(AddressSpace::default())
                    .into()],
                false,
            );
            ctx.module.add_function(ffi_names::STRLEN, fn_type, None)
        })
}

fn get_or_declare_strstr<'ctx>(ctx: &CodegenContext<'ctx>) -> inkwell::values::FunctionValue<'ctx> {
    ctx.module
        .get_function(ffi_names::STRSTR)
        .unwrap_or_else(|| {
            let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
            let fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
            ctx.module.add_function(ffi_names::STRSTR, fn_type, None)
        })
}

fn get_or_declare_strncmp<'ctx>(
    ctx: &CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    ctx.module
        .get_function(ffi_names::STRNCMP)
        .unwrap_or_else(|| {
            let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
            let fn_type = ctx.context.i32_type().fn_type(
                &[
                    ptr_type.into(),
                    ptr_type.into(),
                    ctx.context.i64_type().into(),
                ],
                false,
            );
            ctx.module.add_function(ffi_names::STRNCMP, fn_type, None)
        })
}

fn get_or_declare_malloc<'ctx>(ctx: &CodegenContext<'ctx>) -> inkwell::values::FunctionValue<'ctx> {
    ctx.module
        .get_function(ffi_names::MALLOC)
        .unwrap_or_else(|| {
            let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
            let fn_type = ptr_type.fn_type(&[ctx.context.i64_type().into()], false);
            ctx.module.add_function(ffi_names::MALLOC, fn_type, None)
        })
}

fn get_or_declare_memcpy<'ctx>(ctx: &CodegenContext<'ctx>) -> inkwell::values::FunctionValue<'ctx> {
    ctx.module
        .get_function(ffi_names::MEMCPY)
        .unwrap_or_else(|| {
            let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
            let fn_type = ptr_type.fn_type(
                &[
                    ptr_type.into(),
                    ptr_type.into(),
                    ctx.context.i64_type().into(),
                ],
                false,
            );
            ctx.module.add_function(ffi_names::MEMCPY, fn_type, None)
        })
}

fn get_or_declare_string_split<'ctx>(
    ctx: &CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    ctx.module
        .get_function(ffi_names::DOO_STRING_SPLIT)
        .unwrap_or_else(|| {
            let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
            // doo_string_split(str: *const c_char, delim: *const c_char) -> *mut u8
            let fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
            ctx.module
                .add_function(ffi_names::DOO_STRING_SPLIT, fn_type, None)
        })
}

fn get_or_declare_string_replace_all<'ctx>(
    ctx: &CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    ctx.module
        .get_function(ffi_names::DOO_STRING_REPLACE_ALL)
        .unwrap_or_else(|| {
            let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
            // doo_string_replace_all(haystack: *const c_char, needle: *const c_char, replacement: *const c_char) -> *const c_char
            let fn_type = ptr_type.fn_type(
                &[ptr_type.into(), ptr_type.into(), ptr_type.into()],
                false,
            );
            ctx.module
                .add_function(ffi_names::DOO_STRING_REPLACE_ALL, fn_type, None)
        })
}
