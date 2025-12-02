use crate::codegen::core::CodeGen;
use inkwell::values::FunctionValue;
use inkwell::AddressSpace;

impl<'ctx> CodeGen<'ctx> {
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
