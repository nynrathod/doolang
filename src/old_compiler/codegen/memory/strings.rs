use crate::codegen::core::CodeGen;
use inkwell::values::FunctionValue;
use inkwell::values::PointerValue;
use inkwell::AddressSpace;

impl<'ctx> CodeGen<'ctx> {
    pub fn clone_ffi_string_to_rc(&mut self, src: PointerValue<'ctx>) -> PointerValue<'ctx> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let null_ptr = ptr_type.const_null();

        let is_null = self.builder.build_is_null(src, "ffi_str_is_null").unwrap();
        let current_block = self.builder.get_insert_block().unwrap();
        let func = current_block.get_parent().unwrap();
        let null_block = self.context.append_basic_block(func, "ffi_str_null");
        let nonnull_block = self.context.append_basic_block(func, "ffi_str_nonnull");
        let merge_block = self.context.append_basic_block(func, "ffi_str_merge");

        self.builder
            .build_conditional_branch(is_null, null_block, nonnull_block)
            .unwrap();

        self.builder.position_at_end(null_block);
        self.builder
            .build_unconditional_branch(merge_block)
            .unwrap();

        self.builder.position_at_end(nonnull_block);

        let strlen_fn = self.get_or_declare_strlen();
        let len_i64 = self
            .builder
            .build_call(strlen_fn, &[src.into()], "ffi_strlen")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();

        let len_plus_null = self
            .builder
            .build_int_add(
                len_i64,
                self.context.i64_type().const_int(1, false),
                "ffi_len_plus_null",
            )
            .unwrap();

        let total_size = self
            .builder
            .build_int_add(
                len_plus_null,
                self.context.i64_type().const_int(8, false),
                "ffi_total_size",
            )
            .unwrap();

        let malloc_fn = self.get_or_declare_malloc();
        let heap_ptr = self
            .builder
            .build_call(malloc_fn, &[total_size.into()], "ffi_rc_heap")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        let rc_ptr = self
            .builder
            .build_pointer_cast(heap_ptr, ptr_type, "ffi_rc_ptr")
            .unwrap();
        self.builder
            .build_store(rc_ptr, self.context.i32_type().const_int(1, false))
            .unwrap();

        let len_ptr = unsafe {
            self.builder.build_gep(
                self.context.i8_type(),
                heap_ptr,
                &[self.context.i32_type().const_int(4, false)],
                "ffi_len_ptr",
            )
        }
        .unwrap();
        let len_ptr_cast = self
            .builder
            .build_pointer_cast(len_ptr, ptr_type, "ffi_len_ptr_cast")
            .unwrap();
        let len_i32 = self
            .builder
            .build_int_truncate(len_i64, self.context.i32_type(), "ffi_len_i32")
            .unwrap();
        self.builder.build_store(len_ptr_cast, len_i32).unwrap();

        let data_ptr = unsafe {
            self.builder.build_gep(
                self.context.i8_type(),
                heap_ptr,
                &[self.context.i32_type().const_int(8, false)],
                "ffi_data_ptr",
            )
        }
        .unwrap();

        let memcpy_fn = self.get_or_declare_memcpy();
        self.builder
            .build_call(
                memcpy_fn,
                &[
                    data_ptr.into(),
                    src.into(),
                    len_plus_null.into(),
                    self.context.bool_type().const_zero().into(),
                ],
                "",
            )
            .unwrap();

        // DO NOT free the source FFI string here!
        // For strings from DB FFI (extracted directly from DooResult), they are allocated
        // with the runtime allocator and marked owner=LLVM, so they're RC-managed.
        // Freeing here causes use-after-free when the HTTP layer tries to read the string.
        // let free_fn = self.module.get_function("dooruntime_free_string").unwrap_or_else(|| {
        //     let fn_type = self.context.void_type().fn_type(&[ptr_type.into()], false);
        //     self.module
        //         .add_function("dooruntime_free_string", fn_type, None)
        // });
        // self.builder.build_call(free_fn, &[src.into()], "").unwrap();

        self.builder
            .build_unconditional_branch(merge_block)
            .unwrap();

        self.builder.position_at_end(merge_block);
        let phi = self.builder.build_phi(ptr_type, "ffi_str_phi").unwrap();
        phi.add_incoming(&[(&null_ptr, null_block), (&data_ptr, nonnull_block)]);

        phi.as_basic_value().into_pointer_value()
    }

    pub fn generate_string_concat(
        &mut self,
        name: &str,
        left: &str,
        right: &str,
    ) -> Option<inkwell::values::BasicValueEnum<'ctx>> {
        let left_ptr = self.resolve_value(left).into_pointer_value();
        let right_ptr = self.resolve_value(right).into_pointer_value();

        let strlen_fn = self.get_or_declare_strlen();

        let left_len = self
            .builder
            .build_call(strlen_fn, &[left_ptr.into()], "left_len")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();

        let right_len = self
            .builder
            .build_call(strlen_fn, &[right_ptr.into()], "right_len")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();

        let total_len = self
            .builder
            .build_int_add(left_len, right_len, "partial_len")
            .unwrap();
        let total_len_plus_null = self
            .builder
            .build_int_add(
                total_len,
                self.context.i64_type().const_int(1, false),
                "len_with_null",
            )
            .unwrap();
        let total_size = self
            .builder
            .build_int_add(
                total_len_plus_null,
                self.context.i64_type().const_int(8, false),
                "total_size",
            )
            .unwrap();

        let malloc_fn = self.get_or_declare_malloc();
        let heap_ptr = self
            .builder
            .build_call(malloc_fn, &[total_size.into()], "concat_heap")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        let rc_ptr = self
            .builder
            .build_pointer_cast(
                heap_ptr,
                self.context.ptr_type(inkwell::AddressSpace::default()),
                "rc_ptr",
            )
            .unwrap();
        self.builder
            .build_store(rc_ptr, self.context.i32_type().const_int(1, false))
            .unwrap();

        let data_ptr = unsafe {
            self.builder.build_gep(
                self.context.i8_type(),
                heap_ptr,
                &[self.context.i32_type().const_int(8, false)],
                "data_ptr",
            )
        }
        .unwrap();

        let memcpy_fn = self.get_or_declare_memcpy();
        self.builder
            .build_call(
                memcpy_fn,
                &[
                    data_ptr.into(),
                    left_ptr.into(),
                    left_len.into(),
                    self.context.bool_type().const_zero().into(),
                ],
                "",
            )
            .unwrap();

        // Cast left_len to i32 for GEP index
        let left_len_i32 = self
            .builder
            .build_int_cast(left_len, self.context.i32_type(), "left_len_i32")
            .unwrap();

        let right_dest = unsafe {
            self.builder.build_gep(
                self.context.i8_type(),
                data_ptr,
                &[left_len_i32],
                "right_dest",
            )
        }
        .unwrap();

        self.builder
            .build_call(
                memcpy_fn,
                &[
                    right_dest.into(),
                    right_ptr.into(),
                    right_len.into(),
                    self.context.bool_type().const_zero().into(),
                ],
                "",
            )
            .unwrap();

        let null_pos = unsafe {
            self.builder.build_gep(
                self.context.i8_type(),
                data_ptr,
                &[self
                    .builder
                    .build_int_cast(total_len, self.context.i32_type(), "total_len_i32")
                    .unwrap()],
                "null_pos",
            )
        }
        .unwrap();
        self.builder
            .build_store(null_pos, self.context.i8_type().const_zero())
            .unwrap();

        self.temp_values.insert(name.to_string(), data_ptr.into());
        self.heap_strings.insert(name.to_string());

        Some(data_ptr.into())
    }

    pub fn get_or_declare_strlen(&self) -> FunctionValue<'ctx> {
        if let Some(func) = self.module.get_function("strlen") {
            return func;
        }

        // Declare strlen: size_t strlen(const char *s)
        let i8_ptr = self.context.ptr_type(AddressSpace::default());
        let size_t = self.context.i64_type(); // Using i64 for size_t
        let fn_type = size_t.fn_type(&[i8_ptr.into()], false);

        self.module.add_function("strlen", fn_type, None)
    }
}
