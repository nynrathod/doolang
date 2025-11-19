/// This module implements reference counting (RC) runtime support for memory management.
/// It provides functions for incrementing and decrementing reference counts,
/// and for declaring or retrieving standard memory functions (malloc, free, memcpy).
/// All logic is designed to work with LLVM IR via the inkwell library.
use crate::codegen::core::CodeGen;
use inkwell::values::FunctionValue;
use inkwell::AddressSpace;

/// Implements RC runtime logic for the CodeGen context.
/// All methods here are used to generate LLVM IR for reference counting and memory operations.
impl<'ctx> CodeGen<'ctx> {
    /// Initializes the RC runtime by creating the incref and decref functions.
    /// These functions are stored in the CodeGen context for later use.
    pub fn init_rc_runtime(&mut self) {
        self.incref_fn = Some(self.create_incref_function());
        self.decref_fn = Some(self.create_decref_function());
    }

    /// Creates the LLVM function for incrementing the reference count (incref).
    /// This function takes a pointer to the RC header and increments its count.
    /// Returns the LLVM FunctionValue for later use.
    fn create_incref_function(&self) -> FunctionValue<'ctx> {
        // Define the function signature: void(i8*)
        let i8_ptr = self.context.ptr_type(AddressSpace::default());
        let void_type = self.context.void_type();
        let fn_type = void_type.fn_type(&[i8_ptr.into()], false);

        // Add the function to the module
        let function = self.module.add_function("__incref", fn_type, None);
        let entry = self.context.append_basic_block(function, "entry");

        // Position builder at the entry block
        self.builder.position_at_end(entry);

        // Get the RC header pointer from the function parameter
        let rc_ptr = function.get_nth_param(0).unwrap().into_pointer_value();

        // Cast the RC header pointer to i32* (reference count is stored as i32)
        let i32_ptr_type = self.context.ptr_type(AddressSpace::default());
        let rc_ptr_typed = self
            .builder
            .build_pointer_cast(rc_ptr, i32_ptr_type, "rc_ptr")
            .unwrap();

        // Load the current reference count
        let rc = self
            .builder
            .build_load(self.context.i32_type(), rc_ptr_typed, "rc")
            .unwrap()
            .into_int_value();

        // Increment the reference count by 1
        let new_rc = self
            .builder
            .build_int_add(rc, self.context.i32_type().const_int(1, false), "new_rc")
            .unwrap();

        // Store the new reference count back to memory
        self.builder.build_store(rc_ptr_typed, new_rc).unwrap();
        // Return void
        self.builder.build_return(None).unwrap();

        function
    }

    /// Creates the LLVM function for decrementing the reference count (decref).
    /// If the reference count reaches zero, memory is freed.
    /// Includes safety checks to prevent dereferencing invalid pointers (e.g., global constants).
    /// Returns the LLVM FunctionValue for later use.
    fn create_decref_function(&self) -> FunctionValue<'ctx> {
        // Define the function signature: void(i8*)
        let i8_ptr = self.context.ptr_type(AddressSpace::default());
        let void_type = self.context.void_type();
        let fn_type = void_type.fn_type(&[i8_ptr.into()], false);

        // Add the function to the module
        let function = self.module.add_function("__decref", fn_type, None);
        let entry = self.context.append_basic_block(function, "entry");
        let check_validity = self.context.append_basic_block(function, "check_validity");
        let free_block = self.context.append_basic_block(function, "free");
        let exit_block = self.context.append_basic_block(function, "exit");

        // Position builder at the entry block
        self.builder.position_at_end(entry);

        // Get the RC header pointer from the function parameter
        let rc_ptr = function.get_nth_param(0).unwrap().into_pointer_value();

        // SAFETY CHECK: Verify RC header pointer is not null
        // (null pointers indicate invalid/global data)
        let is_null = self.builder.build_is_null(rc_ptr, "is_null").unwrap();

        self.builder
            .build_conditional_branch(is_null, exit_block, check_validity)
            .unwrap();

        // Check validity block: validate RC count is reasonable
        self.builder.position_at_end(check_validity);

        // Cast the RC header pointer to i32* (reference count is stored as i32)
        let i32_ptr_type = self.context.ptr_type(AddressSpace::default());
        let rc_ptr_typed = self
            .builder
            .build_pointer_cast(rc_ptr, i32_ptr_type, "rc_ptr")
            .unwrap();

        // Load the current reference count
        let rc = self
            .builder
            .build_load(self.context.i32_type(), rc_ptr_typed, "rc")
            .unwrap()
            .into_int_value();

        // SAFETY CHECK: RC count should be positive and less than a reasonable max
        // (e.g., 1-1000000). If not, this is likely a global constant pointer, skip it.
        let is_positive = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SGT,
                rc,
                self.context.i32_type().const_int(0, false),
                "is_positive",
            )
            .unwrap();

        let is_reasonable = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SLT,
                rc,
                self.context.i32_type().const_int(1000000, false),
                "is_reasonable",
            )
            .unwrap();

        let is_valid_rc = self
            .builder
            .build_and(is_positive, is_reasonable, "is_valid_rc")
            .unwrap();

        // Branch to decrement logic only if RC is valid
        let do_decrement = self.context.append_basic_block(function, "do_decrement");
        self.builder
            .build_conditional_branch(is_valid_rc, do_decrement, exit_block)
            .unwrap();

        // Do decrement block
        self.builder.position_at_end(do_decrement);

        // Decrement the reference count by 1
        let new_rc = self
            .builder
            .build_int_sub(rc, self.context.i32_type().const_int(1, false), "new_rc")
            .unwrap();

        // Store the new reference count back to memory
        self.builder.build_store(rc_ptr_typed, new_rc).unwrap();

        // Check if the reference count is zero (should free memory)
        let should_free = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                new_rc,
                self.context.i32_type().const_int(0, false),
                "should_free",
            )
            .unwrap();

        // Branch to free or exit block based on the comparison
        self.builder
            .build_conditional_branch(should_free, free_block, exit_block)
            .unwrap();

        // Free block: call free() on the RC header pointer
        self.builder.position_at_end(free_block);
        let free_fn = self.get_or_declare_free();
        self.builder
            .build_call(free_fn, &[rc_ptr.into()], "")
            .unwrap();
        self.builder.build_unconditional_branch(exit_block).unwrap();

        // Exit block: return void
        self.builder.position_at_end(exit_block);
        self.builder.build_return(None).unwrap();

        function
    }

    /// Retrieves the LLVM function for freeing memory (free).
    /// If not already declared, declares it in the module.
    /// Returns the LLVM FunctionValue for free.
    fn get_or_declare_free(&self) -> FunctionValue<'ctx> {
        // Check if the function is already declared
        if let Some(func) = self.module.get_function("free") {
            return func;
        }

        // Declare the function: void(i8*)
        let i8_ptr = self.context.ptr_type(AddressSpace::default());
        let void_type = self.context.void_type();
        let fn_type = void_type.fn_type(&[i8_ptr.into()], false);

        self.module.add_function("free", fn_type, None)
    }

    /// Retrieves the LLVM function for allocating memory (malloc).
    /// If not already declared, declares it in the module.
    /// Returns the LLVM FunctionValue for malloc.
    pub fn get_or_declare_malloc(&self) -> FunctionValue<'ctx> {
        // Check if the function is already declared
        if let Some(func) = self.module.get_function("malloc") {
            return func;
        }

        // Declare the function: i8*(i64)
        let i64_type = self.context.i64_type();
        let i8_ptr = self.context.ptr_type(AddressSpace::default());
        let fn_type = i8_ptr.fn_type(&[i64_type.into()], false);

        self.module.add_function("malloc", fn_type, None)
    }

    /// Retrieves the LLVM function for copying memory (memcpy).
    /// If not already declared, declares it in the module.
    /// Returns the LLVM FunctionValue for memcpy.
    pub fn get_or_declare_memcpy(&self) -> FunctionValue<'ctx> {
        // Check if the function is already declared
        if let Some(func) = self.module.get_function("llvm.memcpy.p0.p0.i64") {
            return func;
        }
        // Declare the function: void(i8*, i8*, i64, i1)
        let i8_ptr = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let i1_type = self.context.bool_type();

        let fn_type = self.context.void_type().fn_type(
            &[
                i8_ptr.into(),
                i8_ptr.into(),
                i64_type.into(),
                i1_type.into(),
            ],
            false,
        );
        self.module
            .add_function("llvm.memcpy.p0.p0.i64", fn_type, None)
    }

    /// Emits code to increment the reference count for a variable.
    /// Looks up the symbol, loads its pointer, computes the RC header,
    /// and calls the incref function.
    pub fn emit_incref(&self, var_name: &str) {
        if let Some(symbol) = self.symbols.get(var_name) {
            // Load the value from the symbol
            let loaded_value = self
                .builder
                .build_load(symbol.ty, symbol.ptr, "loaded")
                .unwrap();

            // Only do RC for pointer types (strings, arrays, maps)
            // Skip integers, booleans, and other non-pointer types
            if !loaded_value.is_pointer_value() {
                return;
            }

            let data_ptr = loaded_value.into_pointer_value();

            // Compute the RC header pointer by subtracting 8 bytes
            let rc_header = unsafe {
                self.builder.build_in_bounds_gep(
                    self.context.i8_type(),
                    data_ptr,
                    &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                    "rc_header",
                )
            }
            .unwrap();

            // Call the incref function with the RC header pointer
            let incref = self.incref_fn.unwrap();
            self.builder
                .build_call(incref, &[rc_header.into()], "")
                .unwrap();
        }
    }

    /// Emits code to decrement the reference count for a variable.
    /// Looks up the symbol, loads its pointer, computes the RC header,
    /// and calls the decref function.
    pub fn emit_decref(&self, var_name: &str) {
        if let Some(symbol) = self.symbols.get(var_name) {
            // Load the value from the symbol
            let loaded_value = self
                .builder
                .build_load(symbol.ty, symbol.ptr, "loaded")
                .unwrap();

            // Only do RC for pointer types (strings, arrays, maps)
            // Skip integers, booleans, and other non-pointer types
            if !loaded_value.is_pointer_value() {
                return;
            }

            let data_ptr = loaded_value.into_pointer_value();

            // Check if the pointer is null before computing RC header
            let is_null = self.builder.build_is_null(data_ptr, "is_null").unwrap();

            let current_block = self.builder.get_insert_block().unwrap();
            let func = current_block.get_parent().unwrap();
            let decref_block = self.context.append_basic_block(func, "do_decref");
            let continue_block = self.context.append_basic_block(func, "after_decref");

            self.builder
                .build_conditional_branch(is_null, continue_block, decref_block)
                .unwrap();

            // In decref_block, compute RC header and call decref
            self.builder.position_at_end(decref_block);

            // Compute the RC header pointer by subtracting 8 bytes
            let rc_header = unsafe {
                self.builder.build_in_bounds_gep(
                    self.context.i8_type(),
                    data_ptr,
                    &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                    "rc_header",
                )
            }
            .unwrap();

            // Call the decref function with the RC header pointer
            let decref = self.decref_fn.unwrap();
            self.builder
                .build_call(decref, &[rc_header.into()], "")
                .unwrap();

            self.builder
                .build_unconditional_branch(continue_block)
                .unwrap();

            // Continue with the rest of the code
            self.builder.position_at_end(continue_block);
        }
    }
}
