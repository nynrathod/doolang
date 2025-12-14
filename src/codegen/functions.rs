use crate::codegen::core::context::FieldLayout;
use crate::codegen::core::helpers::parse_tuple_types;
use crate::codegen::core::CodeGen;
use crate::mir::mir::{CodegenBlock, MirBlock, MirFunction, MirInstr, MirProgram, MirTerminator};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue};
use inkwell::AddressSpace;
use std::collections::HashMap;

impl<'ctx> CodeGen<'ctx> {
    /// The main entry point for code generation. Processes the entire MIR program.
    /// This function orchestrates the translation of the MIR (Mid-level Intermediate Representation)
    /// into LLVM IR, handling global variables, functions, and the main entry point.
    /// It also initializes reference counting runtime and applies optimization passes.
    pub fn generate_program(&mut self, program: &MirProgram) {
        // Initialize RC runtime FIRST to ensure reference counting functions are available.
        self.init_rc_runtime();
        // Declare builtin string conversion functions.
        self.declare_builtin_functions();

        // Store the global instructions for later use (e.g., initialization).
        self.globals = program.globals.clone();

        // Copy enum_table, enum_variant_order, struct_table, and struct_field_decorators from MirProgram for type metadata access
        self.enum_table = program.enum_table.clone();
        self.enum_variant_order = program.enum_variant_order.clone();
        self.struct_table = program.struct_table.clone();
        self.struct_field_decorators = program.struct_field_decorators.clone();

        // --- PRE-PROCESSING ---
        // CRITICAL: Scan for struct declarations FIRST to populate metadata and create canonical types
        // This MUST happen BEFORE predeclaring functions so that struct parameter types are recognized
        // Pre-scan for enum declarations to register types
        for instr in &program.globals {
            if let MirInstr::EnumDecl {
                enum_name,
                variants,
            } = instr
            {
                // Register enum in enum_table (variant -> payload type)
                let mut variant_map = std::collections::HashMap::new();
                // Register enum variants (variant -> tag)
                let mut variant_list = Vec::new();

                for (idx, variant) in variants.iter().enumerate() {
                    variant_map.insert(variant.name.clone(), variant.payload.clone());
                    variant_list.push((variant.name.clone(), idx as u32));
                }
                self.enum_table.insert(enum_name.clone(), variant_map);
                self.enum_variants.insert(enum_name.clone(), variant_list);
            }
        }

        for instr in &program.globals {
            if let MirInstr::StructDecl {
                struct_name,
                field_names,
                field_types,
            } = instr
            {
                // Create the canonical LLVM struct type first
                let llvm_field_types: Vec<BasicTypeEnum> = field_types
                    .iter()
                    .map(|type_str| self.type_string_to_llvm_type(type_str))
                    .collect();

                let struct_type = self.context.struct_type(&llvm_field_types, false);
                self.canonical_struct_types
                    .insert(struct_name.clone(), struct_type);

                // Compute exact layout from LLVM
                let (field_layouts, total_size, total_align) =
                    self.compute_struct_layout(struct_type, field_names, field_types);

                // Store metadata with layout info
                let metadata = crate::codegen::core::context::StructMetadata {
                    field_names: field_names.clone(),
                    field_types: field_types.clone(),
                    field_layouts,
                    total_size,
                    total_align,
                };
                self.struct_metadata.insert(struct_name.clone(), metadata);
            }
        }

        // WORKAROUND: Manually add FileError and FileMetadata struct metadata since imported structs
        // are not included in the MIR globals. This should be fixed by propagating
        // struct declarations from imported modules.
        if !self.struct_metadata.contains_key("FileError") {
            // Create the canonical LLVM struct type
            let llvm_field_types = vec![self
                .context
                .ptr_type(inkwell::AddressSpace::default())
                .into()];
            let struct_type = self.context.struct_type(&llvm_field_types, false);
            self.canonical_struct_types
                .insert("FileError".to_string(), struct_type);

            // Compute layout
            let field_names = vec!["Message".to_string()];
            let field_types = vec!["Str".to_string()];
            let (field_layouts, total_size, total_align) =
                self.compute_struct_layout(struct_type, &field_names, &field_types);

            let metadata = crate::codegen::core::context::StructMetadata {
                field_names,
                field_types,
                field_layouts,
                total_size,
                total_align,
            };
            self.struct_metadata
                .insert("FileError".to_string(), metadata);
        }

        if !self.struct_metadata.contains_key("FileMetadata") {
            // Create the canonical LLVM struct type
            // Bool fields are i32 (0 or 1), Int fields are i32, i64 for size/timestamps
            let llvm_field_types = vec![
                self.context.i32_type().into(), // isFile (Bool as i32)
                self.context.i32_type().into(), // isDir (Bool as i32)
                self.context.i32_type().into(), // isSymlink (Bool as i32)
                self.context.i64_type().into(), // size (i64 from FFI)
                self.context.i32_type().into(), // readonly (Bool as i32)
                self.context.i64_type().into(), // created (i64 timestamp)
                self.context.i64_type().into(), // modified (i64 timestamp)
                self.context.i64_type().into(), // accessed (i64 timestamp)
            ];
            let struct_type = self.context.struct_type(&llvm_field_types, false);
            self.canonical_struct_types
                .insert("FileMetadata".to_string(), struct_type);

            // Compute layout
            let field_names = vec![
                "isFile".to_string(),
                "isDir".to_string(),
                "isSymlink".to_string(),
                "size".to_string(),
                "readonly".to_string(),
                "created".to_string(),
                "modified".to_string(),
                "accessed".to_string(),
            ];
            let field_types = vec![
                "Bool".to_string(),
                "Bool".to_string(),
                "Bool".to_string(),
                "Int".to_string(),
                "Bool".to_string(),
                "Int".to_string(),
                "Int".to_string(),
                "Int".to_string(),
            ];
            let (field_layouts, total_size, total_align) =
                self.compute_struct_layout(struct_type, &field_names, &field_types);

            let metadata = crate::codegen::core::context::StructMetadata {
                field_names,
                field_types,
                field_layouts,
                total_size,
                total_align,
            };
            self.struct_metadata
                .insert("FileMetadata".to_string(), metadata);
        }

        // Pre-scan and declare all functions for forward references
        // This allows functions to call each other regardless of definition order
        for func in &program.functions {
            self.predeclare_function(func);
        }

        // Pre-scan all functions to populate return types and mark those returning heap-allocated types
        // This ensures both function_return_types and functions_returning_heap are populated
        // BEFORE we generate function bodies, so that function calls can properly detect
        // which functions return heap values and what their return types are
        for func in &program.functions {
            if let Some(ref ret_type_str) = func.return_type {
                // Store the return type for all functions under Doo name
                self.function_return_types
                    .insert(func.name.clone(), ret_type_str.clone());

                // For FFI functions, also store under the FFI symbol name
                // This ensures lookup works when we resolve File::Exists -> doo_file_exists
                if let Some(ref ffi_symbol) = func.ffi_symbol {
                    self.function_return_types
                        .insert(ffi_symbol.clone(), ret_type_str.clone());
                }

                // Mark functions that return heap-allocated types
                if ret_type_str.contains("Array")
                    || ret_type_str.contains("Map")
                    || ret_type_str.contains("Str")
                {
                    self.functions_returning_heap.insert(func.name.clone());
                    if let Some(ref ffi_symbol) = func.ffi_symbol {
                        self.functions_returning_heap.insert(ffi_symbol.clone());
                    }
                }
            }

            // Store error type if function can fail
            if let Some(ref err_type_str) = func.error_type {
                self.function_error_types
                    .insert(func.name.clone(), err_type_str.clone());
                // Also store under FFI symbol name
                if let Some(ref ffi_symbol) = func.ffi_symbol {
                    self.function_error_types
                        .insert(ffi_symbol.clone(), err_type_str.clone());
                }
            }
        }

        // Scan all global instructions to identify strings involved in concatenation.
        // This helps optimize string handling and memory management.
        for instr in &program.globals {
            if let MirInstr::StringConcat { left, right, .. } = instr {
                self.strings_to_concat.insert(left.clone());
                self.strings_to_concat.insert(right.clone());
            }
        }

        // --- GLOBAL GENERATION ---
        // Generate LLVM IR for all global variables and constants.
        for g in &program.globals {
            self.generate_global(g);
        }

        // --- FUNCTION GENERATION ---
        // Generate LLVM IR for all user-defined functions and apply optimizations.
        for func in &program.functions {
            let llvm_func = self.generate_function(func);
            // Apply registered optimization passes (like O1, O2, O3) to the generated function.
            self.fpm.run_on(&llvm_func);
        }

        // --- MIDDLEWARE REGISTRATION ---
        // Register middleware functions with HTTP FFI
        // Middleware signature: fn(req: Request, next: Next) -> Response [! ErrorType]
        for func in &program.functions {
            // Check if this is a middleware function
            if func.params.len() == 2 && func.param_types.len() == 2 {
                let param1 = func.param_types[0].as_ref();
                let param2 = func.param_types[1].as_ref();
                let return_type = func.return_type.as_ref();

                // Middleware has signature: (Request, Next) -> Response
                if param1.map(|s| s.as_str()) == Some("Request")
                    && param2.map(|s| s.as_str()) == Some("Next")
                    && return_type.map(|s| s.as_str()) == Some("Response")
                {
                    self.register_middleware_function(&func.name, func.error_type.as_deref());
                }
            }
        }

        // --- MAIN ENTRY POINT ---
        // For non-main-entry files (imported modules), generate a default main if needed
        if !program.is_main_entry && self.module.get_function("main").is_none() {
            self.generate_default_main();
        }
    }

    /// Register a middleware function with the HTTP FFI
    fn register_middleware_function(&mut self, middleware_name: &str, error_type: Option<&str>) {
        use inkwell::AddressSpace;

        // Get the middleware function
        let middleware_fn = match self.module.get_function(middleware_name) {
            Some(f) => f,
            None => return,
        };

        // Generate wrapper for the middleware
        let wrapper_name =
            self.generate_middleware_wrapper(middleware_name, &middleware_fn, error_type);
        if wrapper_name.is_none() {
            return;
        }
        let wrapper_name = wrapper_name.unwrap();

        // Get the wrapper function
        let wrapper_fn = match self.module.get_function(&wrapper_name) {
            Some(f) => f,
            None => return,
        };

        // Declare the FFI registration function if not present
        let register_fn = if let Some(f) = self.module.get_function("doo_http_register_middleware")
        {
            f
        } else {
            let void_type = self.context.void_type();
            let ptr_type = self.context.ptr_type(AddressSpace::default());
            let fn_type = void_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
            self.module
                .add_function("doo_http_register_middleware", fn_type, None)
        };

        // Get the entry block of main (or create one if it doesn't exist)
        let main_fn = if let Some(f) = self.module.get_function("main") {
            f
        } else {
            // If main doesn't exist yet, we'll register during handler generation
            return;
        };

        // Find the first instruction in main to insert before it
        if let Some(entry_block) = main_fn.get_first_basic_block() {
            if let Some(first_instr) = entry_block.get_first_instruction() {
                self.builder.position_before(&first_instr);
            } else {
                self.builder.position_at_end(entry_block);
            }
        } else {
            return;
        }

        // Create C string for middleware name
        let name_global = self
            .builder
            .build_global_string_ptr(middleware_name, "middleware_name")
            .unwrap();
        let name_cstr = name_global.as_pointer_value();

        // Get wrapper function pointer and cast to generic pointer
        let wrapper_fn_ptr = wrapper_fn.as_global_value().as_pointer_value();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let generic_ptr = self
            .builder
            .build_pointer_cast(wrapper_fn_ptr, ptr_type, "middleware_wrapper_cast")
            .unwrap();

        // Call doo_http_register_middleware(name, function_ptr)
        self.builder
            .build_call(
                register_fn,
                &[name_cstr.into(), generic_ptr.into()],
                "register_middleware",
            )
            .unwrap();
    }

    /// Emit a debug print statement (eprintln in generated code)
    fn emit_debug_print(&self, message: &str) {
        let printf_fn_type = self.context.i32_type().fn_type(
            &[self
                .context
                .ptr_type(inkwell::AddressSpace::default())
                .into()],
            true,
        );
        let printf_fn = if let Some(f) = self.module.get_function("printf") {
            f
        } else {
            self.module.add_function("printf", printf_fn_type, None)
        };

        let format_str = self
            .builder
            .build_global_string_ptr(&format!("{}\n", message), "debug_str")
            .unwrap();
        self.builder
            .build_call(
                printf_fn,
                &[format_str.as_pointer_value().into()],
                "debug_print",
            )
            .unwrap();
    }

    /// Emit a debug print for a pointer value
    fn emit_debug_print_ptr(&self, message: &str, ptr: inkwell::values::PointerValue<'ctx>) {
        let printf_fn_type = self.context.i32_type().fn_type(
            &[self
                .context
                .ptr_type(inkwell::AddressSpace::default())
                .into()],
            true,
        );
        let printf_fn = if let Some(f) = self.module.get_function("printf") {
            f
        } else {
            self.module.add_function("printf", printf_fn_type, None)
        };

        let i64_type = self.context.i64_type();
        let ptr_as_int = self
            .builder
            .build_ptr_to_int(ptr, i64_type, "ptr_as_int")
            .unwrap();

        let format_str = self
            .builder
            .build_global_string_ptr(&format!("{}: %p\n", message), "debug_ptr_str")
            .unwrap();
        self.builder
            .build_call(
                printf_fn,
                &[format_str.as_pointer_value().into(), ptr_as_int.into()],
                "debug_print_ptr",
            )
            .unwrap();
    }

    /// Emit a debug print for an integer value
    fn emit_debug_print_int(&self, message: &str, value: inkwell::values::IntValue<'ctx>) {
        let printf_fn_type = self.context.i32_type().fn_type(
            &[self
                .context
                .ptr_type(inkwell::AddressSpace::default())
                .into()],
            true,
        );
        let printf_fn = if let Some(f) = self.module.get_function("printf") {
            f
        } else {
            self.module.add_function("printf", printf_fn_type, None)
        };

        let format_str = self
            .builder
            .build_global_string_ptr(&format!("{}: %d\n", message), "debug_int_str")
            .unwrap();
        self.builder
            .build_call(
                printf_fn,
                &[format_str.as_pointer_value().into(), value.into()],
                "debug_print_int",
            )
            .unwrap();
    }

    /// Create a simple DooHttpError struct with status and message
    fn create_simple_http_error(
        &self,
        status: u16,
        message: &str,
    ) -> inkwell::values::PointerValue<'ctx> {
        use inkwell::AddressSpace;
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i32_type = self.context.i32_type();

        let error_http_alloc = self
            .builder
            .build_malloc(
                self.context
                    .struct_type(&[i32_type.into(), ptr_type.into()], false),
                "error_http",
            )
            .unwrap();

        let error_status_ptr = self
            .builder
            .build_struct_gep(
                self.context
                    .struct_type(&[i32_type.into(), ptr_type.into()], false),
                error_http_alloc,
                0,
                "error_status_ptr",
            )
            .unwrap();
        self.builder
            .build_store(error_status_ptr, i32_type.const_int(status as u64, false))
            .unwrap();

        let error_msg = self
            .builder
            .build_global_string_ptr(message, "error_msg")
            .unwrap()
            .as_pointer_value();
        let error_msg_ptr = self
            .builder
            .build_struct_gep(
                self.context
                    .struct_type(&[i32_type.into(), ptr_type.into()], false),
                error_http_alloc,
                1,
                "error_msg_ptr",
            )
            .unwrap();
        self.builder.build_store(error_msg_ptr, error_msg).unwrap();

        error_http_alloc
    }

    /// Generate a wrapper for middleware function that converts FFI types to Doo structs
    /// Middleware signature: extern "C" fn(*mut DooRequest, *mut DooNext) -> *mut DooResult
    fn generate_middleware_wrapper(
        &mut self,
        middleware_name: &str,
        middleware_fn: &inkwell::values::FunctionValue<'ctx>,
        error_type: Option<&str>,
    ) -> Option<String> {
        use inkwell::AddressSpace;

        let wrapper_name = format!("{}_middleware_wrapper", middleware_name);

        // Check if wrapper already exists
        if self.module.get_function(&wrapper_name).is_some() {
            return Some(wrapper_name);
        }

        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i32_type = self.context.i32_type();

        // Create wrapper function: extern "C" fn(*mut DooRequest, *mut DooNext) -> *mut DooResult
        let wrapper_fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
        let wrapper_fn = self
            .module
            .add_function(&wrapper_name, wrapper_fn_type, None);

        let entry_bb = self.context.append_basic_block(wrapper_fn, "entry");
        let saved_block = self.builder.get_insert_block();
        self.builder.position_at_end(entry_bb);

        // Get parameters
        let request_ptr = wrapper_fn.get_nth_param(0).unwrap().into_pointer_value();
        let next_ptr = wrapper_fn.get_nth_param(1).unwrap().into_pointer_value();

        // Get the actual parameter types from the middleware function
        let middleware_fn_type = middleware_fn.get_type();
        let param_types = middleware_fn_type.get_param_types();

        if param_types.len() != 2 {
            eprintln!(
                "Error: Middleware function {} has wrong number of parameters",
                middleware_name
            );
            if let Some(block) = saved_block {
                self.builder.position_at_end(block);
            }
            return None;
        }

        // Check if parameters are pointers or structs
        let request_param_type = param_types[0];
        let next_param_type = param_types[1];

        // CRITICAL FIX: Pass the original DooRequest pointer directly
        // DooRequest and Request have the same memory layout for the first 4 fields
        // FFI methods like Request.header() expect to receive DooRequest* so they can access
        // the extra fields (params, query, headers) at offsets 4, 5, 6
        // Simply cast the DooRequest* to Request* - they're compatible
        let request_arg: BasicMetadataValueEnum = if request_param_type.is_pointer_type() {
            // Function expects pointer - pass DooRequest pointer directly (cast as Request*)
            request_ptr.into()
        } else {
            // Function expects struct by value - load the first 4 fields from DooRequest
            // This creates a Request struct by value from DooRequest fields
            let method_ptr = self
                .builder
                .build_load(ptr_type, request_ptr, "method_ptr")
                .unwrap()
                .into_pointer_value();

            let path_field_ptr = unsafe {
                self.builder
                    .build_gep(
                        ptr_type,
                        request_ptr,
                        &[i32_type.const_int(1, false)],
                        "path_field_ptr",
                    )
                    .unwrap()
            };
            let path_ptr = self
                .builder
                .build_load(ptr_type, path_field_ptr, "path_ptr")
                .unwrap()
                .into_pointer_value();

            let body_field_ptr = unsafe {
                self.builder
                    .build_gep(
                        ptr_type,
                        request_ptr,
                        &[i32_type.const_int(2, false)],
                        "body_field_ptr",
                    )
                    .unwrap()
            };
            let body_ptr = self
                .builder
                .build_load(ptr_type, body_field_ptr, "body_ptr")
                .unwrap()
                .into_pointer_value();

            let content_type_field_ptr = unsafe {
                self.builder
                    .build_gep(
                        ptr_type,
                        request_ptr,
                        &[i32_type.const_int(3, false)],
                        "content_type_field_ptr",
                    )
                    .unwrap()
            };
            let content_type_ptr = self
                .builder
                .build_load(ptr_type, content_type_field_ptr, "content_type_ptr")
                .unwrap()
                .into_pointer_value();

            // Build Request struct on stack (allocate and store fields)
            let request_struct_type = self.get_struct_type("Request");
            let request_alloca = self
                .builder
                .build_alloca(request_struct_type, "request_alloca")
                .unwrap();

            // Store fields in the allocated struct
            let method_field_ptr = self
                .builder
                .build_struct_gep(request_struct_type, request_alloca, 0, "method_field")
                .unwrap();
            self.builder
                .build_store(method_field_ptr, method_ptr)
                .unwrap();

            let path_field_ptr_gep = self
                .builder
                .build_struct_gep(request_struct_type, request_alloca, 1, "path_field")
                .unwrap();
            self.builder
                .build_store(path_field_ptr_gep, path_ptr)
                .unwrap();

            let body_field_ptr_gep = self
                .builder
                .build_struct_gep(request_struct_type, request_alloca, 2, "body_field")
                .unwrap();
            self.builder
                .build_store(body_field_ptr_gep, body_ptr)
                .unwrap();

            let ct_field_ptr = self
                .builder
                .build_struct_gep(request_struct_type, request_alloca, 3, "ct_field")
                .unwrap();
            self.builder
                .build_store(ct_field_ptr, content_type_ptr)
                .unwrap();

            // Load struct by value
            self.builder
                .build_load(request_struct_type, request_alloca, "request_val")
                .unwrap()
                .into()
        };

        // Build Next struct on stack - contains the DooNext pointer
        // Next has two Int fields (HandlerPtrLow, HandlerPtrHigh) to store a 64-bit pointer
        let next_struct_type = self.get_struct_type("Next");
        let next_alloca = self
            .builder
            .build_alloca(next_struct_type, "next_alloca")
            .unwrap();

        // Cast pointer to i64 first
        let i64_type = self.context.i64_type();
        let next_as_i64 = self
            .builder
            .build_ptr_to_int(next_ptr, i64_type, "next_as_i64")
            .unwrap();

        // Split into low and high 32 bits
        let low_bits = self
            .builder
            .build_int_truncate(next_as_i64, i32_type, "ptr_low")
            .unwrap();

        let high_bits_i64 = self
            .builder
            .build_right_shift(next_as_i64, i64_type.const_int(32, false), false, "shifted")
            .unwrap();
        let high_bits = self
            .builder
            .build_int_truncate(high_bits_i64, i32_type, "ptr_high")
            .unwrap();

        // Store low bits in field 0
        let low_field_ptr = self
            .builder
            .build_struct_gep(next_struct_type, next_alloca, 0, "low_field")
            .unwrap();
        self.builder.build_store(low_field_ptr, low_bits).unwrap();

        // Store high bits in field 1
        let high_field_ptr = self
            .builder
            .build_struct_gep(next_struct_type, next_alloca, 1, "high_field")
            .unwrap();
        self.builder.build_store(high_field_ptr, high_bits).unwrap();

        let next_arg: BasicMetadataValueEnum = if next_param_type.is_pointer_type() {
            // Function expects pointer to Next struct
            next_alloca.into()
        } else {
            // Function expects Next struct by value - load it
            self.builder
                .build_load(next_struct_type, next_alloca, "next_val")
                .unwrap()
                .into()
        };

        // Call the middleware function with appropriate argument types
        let middleware_result = self
            .builder
            .build_call(
                *middleware_fn,
                &[request_arg, next_arg],
                "middleware_result",
            )
            .unwrap()
            .try_as_basic_value()
            .left();

        // The middleware returns a Response struct or Result<Response, Error>
        if let Some(response_value) = middleware_result {
            // Check if this is a pointer to a struct or a direct struct value
            if response_value.is_pointer_value() {
                // Pointer to Response struct - just return it directly as DooResponse
                // Don't try to load it - the pointer IS the response
                let response_ptr = response_value.into_pointer_value();

                // Build DooResult with the response pointer
                let result_alloc = self
                    .builder
                    .build_malloc(
                        self.context
                            .struct_type(&[i32_type.into(), ptr_type.into()], false),
                        "result_alloc",
                    )
                    .unwrap();

                let tag_ptr = self
                    .builder
                    .build_struct_gep(
                        self.context
                            .struct_type(&[i32_type.into(), ptr_type.into()], false),
                        result_alloc,
                        0,
                        "tag_ptr",
                    )
                    .unwrap();
                self.builder
                    .build_store(tag_ptr, i32_type.const_int(0, false))
                    .unwrap();

                let value_ptr = self
                    .builder
                    .build_struct_gep(
                        self.context
                            .struct_type(&[i32_type.into(), ptr_type.into()], false),
                        result_alloc,
                        1,
                        "value_ptr",
                    )
                    .unwrap();
                self.builder.build_store(value_ptr, response_ptr).unwrap();

                self.builder.build_return(Some(&result_alloc)).unwrap();

                // Restore builder position and return
                if let Some(block) = saved_block {
                    self.builder.position_at_end(block);
                }
                return Some(wrapper_name);
            } else if response_value.is_struct_value() {
                let struct_val = response_value.into_struct_value();
                let struct_type = struct_val.get_type();

                // Check if it's a Result struct (2 fields) or Response struct (3 fields)
                if struct_type.count_fields() == 2 {
                    // This is a Result struct - same handling as pointer case above
                    let tag = self
                        .builder
                        .build_extract_value(struct_val, 0, "result_tag")
                        .unwrap()
                        .into_int_value();

                    let is_ok = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            tag,
                            i32_type.const_int(0, false),
                            "is_ok",
                        )
                        .unwrap();

                    let ok_block = self.context.append_basic_block(wrapper_fn, "result_ok");
                    let err_block = self.context.append_basic_block(wrapper_fn, "result_err");

                    self.builder
                        .build_conditional_branch(is_ok, ok_block, err_block)
                        .unwrap();

                    // Error block: create DooHttpError using RFC 7807 FFI function
                    self.builder.position_at_end(err_block);

                    // Extract the error enum value (ptr in field 1)
                    let error_enum_ptr = self
                        .builder
                        .build_extract_value(struct_val, 1, "error_enum_ptr")
                        .unwrap()
                        .into_pointer_value();

                    // Cast to enum struct type {i32 tag, ptr payload}
                    let enum_struct_type = self
                        .context
                        .struct_type(&[i32_type.into(), ptr_type.into()], false);

                    // Load the enum struct
                    let enum_val = self
                        .builder
                        .build_load(enum_struct_type, error_enum_ptr, "error_enum")
                        .unwrap()
                        .into_struct_value();

                    // Extract tag (variant index)
                    let error_tag = self
                        .builder
                        .build_extract_value(enum_val, 0, "error_tag")
                        .unwrap()
                        .into_int_value();

                    // Get error enum metadata (name and variants)
                    let (enum_name_str, variant_name_str) = if let Some(error_type_str) = error_type
                    {
                        // Get enum variants to map tag to variant name
                        let variant_name =
                            if let Some(variants) = self.enum_variant_order.get(error_type_str) {
                                // Try to find variant name by tag at runtime
                                // For now, we'll generate a switch/branch for each variant
                                // But first, let's just use the first matching variant name as default
                                variants
                                    .first()
                                    .map(|(name, _)| name.as_str())
                                    .unwrap_or("Unknown")
                            } else {
                                "Unknown"
                            };
                        (error_type_str, variant_name)
                    } else {
                        ("Error", "Unknown")
                    };

                    // Build switch to map tag to variant name at runtime
                    let error_http_alloc = if let Some(error_type_str) = error_type {
                        if let Some(variants) = self.enum_variant_order.get(error_type_str) {
                            // Declare the FFI function for converting enum to RFC 7807
                            let convert_fn_type = ptr_type.fn_type(
                                &[
                                    ptr_type.into(), // enum_name
                                    i32_type.into(), // variant_tag
                                    ptr_type.into(), // variant_name
                                    ptr_type.into(), // instance (request path)
                                ],
                                false,
                            );
                            let convert_fn = if let Some(f) = self
                                .module
                                .get_function("doohttp_middleware_error_to_rfc7807")
                            {
                                f
                            } else {
                                self.module.add_function(
                                    "doohttp_middleware_error_to_rfc7807",
                                    convert_fn_type,
                                    None,
                                )
                            };

                            // Get current request path
                            let get_path_fn_type = ptr_type.fn_type(&[], false);
                            let get_path_fn = if let Some(f) =
                                self.module.get_function("doohttp_get_current_request_path")
                            {
                                f
                            } else {
                                self.module.add_function(
                                    "doohttp_get_current_request_path",
                                    get_path_fn_type,
                                    None,
                                )
                            };
                            let request_path = self
                                .builder
                                .build_call(get_path_fn, &[], "request_path")
                                .unwrap()
                                .try_as_basic_value()
                                .left()
                                .unwrap()
                                .into_pointer_value();

                            // For each variant, we need to check the tag and call the FFI with the right variant name
                            // Create a PHI node to merge results from all branches
                            let merge_block =
                                self.context.append_basic_block(wrapper_fn, "error_merge");
                            let mut variant_blocks = Vec::new();

                            for (idx, (variant_name, _)) in variants.iter().enumerate() {
                                let variant_block = self.context.append_basic_block(
                                    wrapper_fn,
                                    &format!("error_variant_{}", variant_name),
                                );
                                variant_blocks.push((
                                    idx as u64,
                                    variant_block,
                                    variant_name.clone(),
                                ));
                            }

                            // Default block for unknown variants
                            let default_block =
                                self.context.append_basic_block(wrapper_fn, "error_default");

                            // Build switch on error tag
                            self.builder
                                .build_switch(
                                    error_tag,
                                    default_block,
                                    &variant_blocks
                                        .iter()
                                        .map(|(idx, block, _)| {
                                            (i32_type.const_int(*idx, false), *block)
                                        })
                                        .collect::<Vec<_>>(),
                                )
                                .unwrap();

                            // Generate each variant block
                            let enum_name_cstr = self
                                .builder
                                .build_global_string_ptr(enum_name_str, "error_enum_name")
                                .unwrap()
                                .as_pointer_value();

                            let mut phi_incoming = Vec::new();

                            for (_idx, block, variant_name) in &variant_blocks {
                                self.builder.position_at_end(*block);

                                let variant_cstr = self
                                    .builder
                                    .build_global_string_ptr(
                                        variant_name,
                                        &format!("variant_{}", variant_name),
                                    )
                                    .unwrap()
                                    .as_pointer_value();

                                let error_result = self
                                    .builder
                                    .build_call(
                                        convert_fn,
                                        &[
                                            enum_name_cstr.into(),
                                            error_tag.into(),
                                            variant_cstr.into(),
                                            request_path.into(),
                                        ],
                                        "error_http",
                                    )
                                    .unwrap()
                                    .try_as_basic_value()
                                    .left()
                                    .unwrap()
                                    .into_pointer_value();

                                phi_incoming.push((error_result, *block));
                                self.builder
                                    .build_unconditional_branch(merge_block)
                                    .unwrap();
                            }

                            // Default block
                            self.builder.position_at_end(default_block);
                            let default_variant_cstr = self
                                .builder
                                .build_global_string_ptr("Unknown", "variant_unknown")
                                .unwrap()
                                .as_pointer_value();

                            let default_error_result = self
                                .builder
                                .build_call(
                                    convert_fn,
                                    &[
                                        enum_name_cstr.into(),
                                        error_tag.into(),
                                        default_variant_cstr.into(),
                                        request_path.into(),
                                    ],
                                    "error_http_default",
                                )
                                .unwrap()
                                .try_as_basic_value()
                                .left()
                                .unwrap()
                                .into_pointer_value();

                            phi_incoming.push((default_error_result, default_block));
                            self.builder
                                .build_unconditional_branch(merge_block)
                                .unwrap();

                            // Merge block with PHI
                            self.builder.position_at_end(merge_block);
                            let phi = self.builder.build_phi(ptr_type, "error_http_phi").unwrap();
                            for (val, block) in phi_incoming {
                                phi.add_incoming(&[(&val, block)]);
                            }
                            phi.as_basic_value().into_pointer_value()
                        } else {
                            // No variant info, use simple error
                            self.create_simple_http_error(401, "Unauthorized")
                        }
                    } else {
                        // No error type, use simple error
                        self.create_simple_http_error(401, "Unauthorized")
                    };

                    let error_result_alloc = self
                        .builder
                        .build_malloc(
                            self.context
                                .struct_type(&[i32_type.into(), ptr_type.into()], false),
                            "error_result",
                        )
                        .unwrap();

                    let error_tag_ptr = self
                        .builder
                        .build_struct_gep(
                            self.context
                                .struct_type(&[i32_type.into(), ptr_type.into()], false),
                            error_result_alloc,
                            0,
                            "error_tag_ptr",
                        )
                        .unwrap();
                    self.builder
                        .build_store(error_tag_ptr, i32_type.const_int(1, false))
                        .unwrap();

                    let error_value_ptr = self
                        .builder
                        .build_struct_gep(
                            self.context
                                .struct_type(&[i32_type.into(), ptr_type.into()], false),
                            error_result_alloc,
                            1,
                            "error_value_ptr",
                        )
                        .unwrap();
                    self.builder
                        .build_store(error_value_ptr, error_http_alloc)
                        .unwrap();

                    self.builder
                        .build_return(Some(&error_result_alloc))
                        .unwrap();

                    // Ok block: extract response pointer and return it
                    self.builder.position_at_end(ok_block);
                    let response_ptr = self
                        .builder
                        .build_extract_value(struct_val, 1, "response_ptr")
                        .unwrap()
                        .into_pointer_value();

                    // Build DooResult with the response pointer
                    let result_alloc = self
                        .builder
                        .build_malloc(
                            self.context
                                .struct_type(&[i32_type.into(), ptr_type.into()], false),
                            "result_alloc",
                        )
                        .unwrap();

                    let tag_ptr = self
                        .builder
                        .build_struct_gep(
                            self.context
                                .struct_type(&[i32_type.into(), ptr_type.into()], false),
                            result_alloc,
                            0,
                            "tag_ptr",
                        )
                        .unwrap();
                    self.builder
                        .build_store(tag_ptr, i32_type.const_int(0, false))
                        .unwrap();

                    let value_ptr = self
                        .builder
                        .build_struct_gep(
                            self.context
                                .struct_type(&[i32_type.into(), ptr_type.into()], false),
                            result_alloc,
                            1,
                            "value_ptr",
                        )
                        .unwrap();
                    self.builder.build_store(value_ptr, response_ptr).unwrap();

                    self.builder.build_return(Some(&result_alloc)).unwrap();
                } else {
                    // Direct Response struct (3 fields) - allocate on heap and return pointer
                    // Response struct: { Status: Int, Body: Str, ContentType: Str }
                    let response_alloc = self
                        .builder
                        .build_malloc(
                            self.context.struct_type(
                                &[i32_type.into(), ptr_type.into(), ptr_type.into()],
                                false,
                            ),
                            "response_alloc",
                        )
                        .unwrap();

                    // Extract and store Status field
                    let status_val = self
                        .builder
                        .build_extract_value(struct_val, 0, "status_val")
                        .unwrap()
                        .into_int_value();
                    let status_ptr = self
                        .builder
                        .build_struct_gep(
                            self.context.struct_type(
                                &[i32_type.into(), ptr_type.into(), ptr_type.into()],
                                false,
                            ),
                            response_alloc,
                            0,
                            "status_ptr",
                        )
                        .unwrap();
                    self.builder.build_store(status_ptr, status_val).unwrap();

                    // Extract and store Body field
                    let body_val = self
                        .builder
                        .build_extract_value(struct_val, 1, "body_val")
                        .unwrap()
                        .into_pointer_value();
                    let body_ptr = self
                        .builder
                        .build_struct_gep(
                            self.context.struct_type(
                                &[i32_type.into(), ptr_type.into(), ptr_type.into()],
                                false,
                            ),
                            response_alloc,
                            1,
                            "body_ptr",
                        )
                        .unwrap();
                    self.builder.build_store(body_ptr, body_val).unwrap();

                    // Extract and store ContentType field
                    let ct_val = self
                        .builder
                        .build_extract_value(struct_val, 2, "ct_val")
                        .unwrap()
                        .into_pointer_value();
                    let ct_ptr = self
                        .builder
                        .build_struct_gep(
                            self.context.struct_type(
                                &[i32_type.into(), ptr_type.into(), ptr_type.into()],
                                false,
                            ),
                            response_alloc,
                            2,
                            "ct_ptr",
                        )
                        .unwrap();
                    self.builder.build_store(ct_ptr, ct_val).unwrap();

                    // Build DooResult with response pointer
                    let result_alloc = self
                        .builder
                        .build_malloc(
                            self.context
                                .struct_type(&[i32_type.into(), ptr_type.into()], false),
                            "result_alloc",
                        )
                        .unwrap();

                    let tag_ptr = self
                        .builder
                        .build_struct_gep(
                            self.context
                                .struct_type(&[i32_type.into(), ptr_type.into()], false),
                            result_alloc,
                            0,
                            "tag_ptr",
                        )
                        .unwrap();
                    self.builder
                        .build_store(tag_ptr, i32_type.const_int(0, false))
                        .unwrap();

                    let value_ptr = self
                        .builder
                        .build_struct_gep(
                            self.context
                                .struct_type(&[i32_type.into(), ptr_type.into()], false),
                            result_alloc,
                            1,
                            "value_ptr",
                        )
                        .unwrap();
                    self.builder.build_store(value_ptr, response_alloc).unwrap();

                    self.builder.build_return(Some(&result_alloc)).unwrap();
                }
            } else {
                // Not a struct or pointer - this shouldn't happen
                eprintln!(
                    "Warning: Middleware returned unexpected type: {:?}",
                    response_value.get_type()
                );
                let result_alloc = self
                    .builder
                    .build_malloc(
                        self.context
                            .struct_type(&[i32_type.into(), ptr_type.into()], false),
                        "result_alloc",
                    )
                    .unwrap();
                self.builder.build_return(Some(&result_alloc)).unwrap();
                if let Some(block) = saved_block {
                    self.builder.position_at_end(block);
                }
                return Some(wrapper_name);
            }
        } else {
            // No return value - return error
            eprintln!("Warning: Middleware returned no value");
            let result_alloc = self
                .builder
                .build_malloc(
                    self.context
                        .struct_type(&[i32_type.into(), ptr_type.into()], false),
                    "result_alloc",
                )
                .unwrap();

            let tag_ptr = self
                .builder
                .build_struct_gep(
                    self.context
                        .struct_type(&[i32_type.into(), ptr_type.into()], false),
                    result_alloc,
                    0,
                    "tag_ptr",
                )
                .unwrap();
            self.builder
                .build_store(tag_ptr, i32_type.const_zero())
                .unwrap();

            self.builder.build_return(Some(&result_alloc)).unwrap();
        }

        // Restore builder position
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }

        Some(wrapper_name)
    }

    // ADD THIS NEW METHOD:
    fn predeclare_function(&mut self, func: &MirFunction) {
        if self.declared_functions.contains(&func.name) {
            return;
        }

        // Check if this is an FFI function
        let is_ffi = func.ffi_lib.is_some();
        let symbol_name = func.ffi_symbol.as_ref().unwrap_or(&func.name);

        // Store parameter types as strings for JSON.parse conversion
        let param_type_strings: Vec<String> = func
            .param_types
            .iter()
            .map(|type_opt| type_opt.clone().unwrap_or_else(|| "Int".to_string()))
            .collect();
        self.function_param_types
            .insert(func.name.clone(), param_type_strings);

        // Build parameter types
        let param_types: Vec<BasicMetadataTypeEnum> = func
            .param_types
            .iter()
            .map(|type_opt| self.map_type_to_llvm(type_opt))
            .collect();

        // Determine base return type first
        let base_return_type = if func.name == "main" {
            // Force main to be i32 () for C/Clang compatibility
            Some(self.context.i32_type().into())
        } else if let Some(ref ret_type_str) = func.return_type {
            // Check for Void return type - these should be declared as void, not i32
            if ret_type_str == "Void" {
                None
            } else {
                Some(self.get_llvm_return_type(ret_type_str))
            }
        } else {
            None
        };

        // If function has error type, wrap return in Result struct { i32 tag, value }
        let fn_type = if func.name == "main" {
            self.context.i32_type().fn_type(&param_types, false)
        } else if let Some(base_type) = base_return_type {
            if func.error_type.is_some() {
                // CRITICAL: FFI functions with error types return a POINTER to Result struct
                // (not struct by value) to avoid ABI/calling convention issues
                // Non-FFI functions still return Result struct by value
                let ptr_type = self.context.ptr_type(AddressSpace::default());
                let result_struct = self
                    .context
                    .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false);

                if is_ffi {
                    // FFI: return pointer to Result struct
                    ptr_type.fn_type(&param_types, false)
                } else {
                    // Non-FFI: return Result struct by value
                    result_struct.fn_type(&param_types, false)
                }
            } else {
                // Convert BasicTypeEnum to a function type
                match base_type {
                    BasicTypeEnum::IntType(t) => t.fn_type(&param_types, false),
                    BasicTypeEnum::FloatType(t) => t.fn_type(&param_types, false),
                    BasicTypeEnum::PointerType(t) => t.fn_type(&param_types, false),
                    BasicTypeEnum::StructType(t) => t.fn_type(&param_types, false),
                    BasicTypeEnum::ArrayType(t) => t.fn_type(&param_types, false),
                    BasicTypeEnum::VectorType(t) => t.fn_type(&param_types, false),
                    BasicTypeEnum::ScalableVectorType(t) => t.fn_type(&param_types, false),
                }
            }
        } else if func.error_type.is_some() {
            // Void return type but has error type - still needs to return Result struct
            // This handles functions like `fn Validate(x: Int) -> ! Str`
            let ptr_type = self.context.ptr_type(AddressSpace::default());
            let result_struct = self
                .context
                .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false);

            if is_ffi {
                // FFI: return pointer to Result struct (consistent with other FFI error functions)
                ptr_type.fn_type(&param_types, false)
            } else {
                // Non-FFI: return Result struct by value
                result_struct.fn_type(&param_types, false)
            }
        } else {
            self.context.void_type().fn_type(&param_types, false)
        };

        // Declare function (use external symbol name for FFI)
        let llvm_func = self.module.add_function(symbol_name, fn_type, None);

        // For FFI functions, set external linkage
        if is_ffi {
            llvm_func.set_linkage(inkwell::module::Linkage::External);

            // If FFI function has a different symbol name, create alias mapping
            // so that calls using the Doo function name can find the FFI symbol
            if symbol_name != &func.name {
                self.function_aliases
                    .insert(func.name.clone(), symbol_name.clone());
            }
        }

        self.declared_functions.insert(func.name.clone());
    }

    fn get_llvm_return_type(&mut self, ret_type_str: &str) -> BasicTypeEnum<'ctx> {
        if ret_type_str.contains("Void") {
            // Void is not a BasicType, so return i32 as placeholder
            return self.context.i32_type().into();
        }

        let parsed_types = parse_tuple_types(ret_type_str);
        let is_tuple_return = if ret_type_str.starts_with("Tuple(") {
            true
        } else {
            parsed_types.len() > 1
        };

        if is_tuple_return {
            // Multi-value return - create a struct type
            let inner_types = if ret_type_str.starts_with("Tuple(") && ret_type_str.ends_with(')') {
                &ret_type_str[6..ret_type_str.len() - 1]
            } else {
                ret_type_str
            };
            let types = parse_tuple_types(inner_types);
            let mut field_types: Vec<BasicTypeEnum> = Vec::new();

            for type_str in &types {
                let llvm_type = if type_str.contains("String") || type_str.contains("Str") {
                    self.context.ptr_type(AddressSpace::default()).into()
                } else if type_str.contains("Array") || type_str.contains("Map") {
                    self.context.ptr_type(AddressSpace::default()).into()
                } else if type_str.contains("Float") {
                    self.context.f64_type().into()
                } else if type_str.contains("Bool") {
                    // Use i32 for Bool to match internal representation
                    self.context.i32_type().into()
                } else {
                    self.context.i32_type().into()
                };
                field_types.push(llvm_type);
            }

            let struct_type = self.context.struct_type(&field_types, false);
            let tuple_type_str = format!("Tuple({})", ret_type_str);
            self.tuple_struct_types.insert(tuple_type_str, struct_type);
            struct_type.into()
        } else if ret_type_str.contains("String") || ret_type_str.contains("Str") {
            self.context.ptr_type(AddressSpace::default()).into()
        } else if ret_type_str.contains("Array") || ret_type_str.contains("Map") {
            self.context.ptr_type(AddressSpace::default()).into()
        } else if ret_type_str.contains("Float") {
            self.context.f64_type().into()
        } else if ret_type_str.contains("Bool") {
            // Use i32 for Bool to match internal representation
            self.context.i32_type().into()
        } else if ret_type_str.contains("Struct(")
            || self.struct_metadata.contains_key(ret_type_str)
        {
            // Struct return types are pointers to heap-allocated structs
            self.context.ptr_type(AddressSpace::default()).into()
        } else if ret_type_str.starts_with("Enum(") || self.enum_table.contains_key(ret_type_str) {
            // Enum return types are { i32 tag, ptr payload } structs
            let ptr_type = self.context.ptr_type(AddressSpace::default());
            self.context
                .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false)
                .into()
        } else {
            self.context.i32_type().into()
        }
    }

    fn map_type_to_llvm(&self, type_opt: &Option<String>) -> BasicMetadataTypeEnum<'ctx> {
        if let Some(type_str) = type_opt {
            // Check if this is a known struct type (either "Struct(Name)" or just "Name")
            let is_struct =
                type_str.contains("Struct(") || self.struct_metadata.contains_key(type_str);

            // Check if this is an enum type
            let is_enum = type_str.starts_with("Enum(") || self.enum_table.contains_key(type_str);

            // Primitives (Int, Float, Bool) are passed by value
            // Strings, Arrays, Maps, and Structs are passed by pointer
            // Enums are passed as struct { i32 tag, ptr payload } by value
            if type_str == "Int" {
                self.context.i32_type().into()
            } else if type_str == "Float" {
                self.context.f64_type().into()
            } else if type_str == "Bool" {
                // Use i32 for Bool to match internal representation (bool values stored as i32)
                self.context.i32_type().into()
            } else if type_str.contains("String") || type_str.contains("Str") {
                self.context.ptr_type(AddressSpace::default()).into()
            } else if type_str.contains("Array") || type_str.contains("Map") {
                self.context.ptr_type(AddressSpace::default()).into()
            } else if is_enum {
                // Enum is represented as { i32 tag, ptr payload }
                let ptr_type = self.context.ptr_type(AddressSpace::default());
                let enum_struct_type = self
                    .context
                    .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false);
                enum_struct_type.into()
            } else if is_struct {
                // Struct parameters are passed as pointers
                self.context.ptr_type(AddressSpace::default()).into()
            } else {
                self.context.i32_type().into()
            }
        } else {
            self.context.i32_type().into()
        }
    }

    /// Extract the element type from an Array type string
    /// Format: "Array(Int)", "Array(Str)", "Array(StructName)", etc.
    fn extract_array_element_type(type_str: &str) -> &str {
        if type_str.contains("Array(Str)") {
            "Str"
        } else if type_str.contains("Array(Float)") {
            "Float"
        } else if type_str.contains("Array(Bool)") {
            "Bool"
        } else if type_str.contains("Array(Int)") {
            "Int"
        } else if type_str.starts_with("Array(") && type_str.ends_with(")") {
            // Extract the element type for struct arrays like Array(Task)
            let inner = &type_str[6..type_str.len() - 1];
            inner
        } else {
            "Int" // default
        }
    }

    /// Extract key and value types from a Map type string
    /// Format: "Map(key,value) format
    fn extract_map_types(type_str: &str) -> (&str, &str) {
        // Handle Map(key,value) format
        if type_str.contains("Map(Str,Str)") || type_str.contains("Map(String,String)") {
            ("Str", "Str")
        } else if type_str.contains("Map(Str,Int)") || type_str.contains("Map(String,Int)") {
            ("Str", "Int")
        } else if type_str.contains("Map(Str,Float)") || type_str.contains("Map(String,Float)") {
            ("Str", "Float")
        } else if type_str.contains("Map(Str,Bool)") || type_str.contains("Map(String,Bool)") {
            ("Str", "Bool")
        } else if type_str.contains("Map(Int,Str)") || type_str.contains("Map(Int,String)") {
            ("Int", "Str")
        } else if type_str.contains("Map(Int,Int)") {
            ("Int", "Int")
        } else if type_str.contains("Map(Int,Float)") {
            ("Int", "Float")
        } else if type_str.contains("Map(Int,Bool)") {
            ("Int", "Bool")
        } else if type_str.contains("Map(Float,Str)") || type_str.contains("Map(Float,String)") {
            ("Float", "Str")
        } else if type_str.contains("Map(Float,Int)") {
            ("Float", "Int")
        } else if type_str.contains("Map(Float,Float)") {
            ("Float", "Float")
        } else if type_str.contains("Map(Float,Bool)") {
            ("Float", "Bool")
        } else if type_str.contains("Map(Bool,Str)") || type_str.contains("Map(Bool,String)") {
            ("Bool", "Str")
        } else if type_str.contains("Map(Bool,Int)") {
            ("Bool", "Int")
        } else if type_str.contains("Map(Bool,Float)") {
            ("Bool", "Float")
        } else if type_str.contains("Map(Bool,Bool)") {
            ("Bool", "Bool")
        } else {
            ("Int", "Int") // default
        }
    }

    /// Compute exact field layout from LLVM struct type using size_of() and manual alignment
    pub fn compute_struct_layout(
        &self,
        struct_type: inkwell::types::StructType<'ctx>,
        field_names: &[String],
        field_types: &[String],
    ) -> (Vec<FieldLayout>, u64, u64) {
        let mut layouts = Vec::new();
        let mut current_offset: u64 = 0;
        let mut max_align: u64 = 1;

        for (i, (name, type_name)) in field_names.iter().zip(field_types.iter()).enumerate() {
            let field_type = struct_type.get_field_type_at_index(i as u32).unwrap();

            // Get size and alignment for this field type
            // Use a manual size calculation based on type
            let size = match &field_type.as_basic_type_enum() {
                BasicTypeEnum::IntType(int_ty) => {
                    let bit_width = int_ty.get_bit_width();
                    ((bit_width + 7) / 8) as u64 // Convert bits to bytes
                }
                BasicTypeEnum::FloatType(_) => 8, // f64 is 8 bytes
                BasicTypeEnum::PointerType(_) => 8, // Pointers are 8 bytes on 64-bit
                BasicTypeEnum::StructType(_st) => {
                    // For nested structs, try to get size or use default
                    if let Some(size_val) = _st.size_of() {
                        size_val.get_zero_extended_constant().unwrap_or(8)
                    } else {
                        8
                    }
                }
                BasicTypeEnum::ArrayType(_) => 8, // Arrays as pointers
                BasicTypeEnum::VectorType(_) => 16,
                BasicTypeEnum::ScalableVectorType(_) => 16,
            };
            let align = self.get_type_alignment(&field_type.as_basic_type_enum());

            // Align current offset to field alignment
            current_offset = (current_offset + align - 1) & !(align - 1);

            // Track maximum alignment for struct
            max_align = max_align.max(align);

            layouts.push(FieldLayout {
                name: name.clone(),
                type_name: type_name.clone(),
                offset: current_offset,
                size,
                align,
            });

            // Move offset forward by field size
            current_offset += size;
        }

        // Align total size to struct alignment
        let total_size = (current_offset + max_align - 1) & !(max_align - 1);

        (layouts, total_size, max_align)
    }

    /// Get alignment for a type (platform-specific, assuming 64-bit)
    fn get_type_alignment(&self, ty: &inkwell::types::BasicTypeEnum<'ctx>) -> u64 {
        match ty {
            inkwell::types::BasicTypeEnum::IntType(int_ty) => {
                let bit_width = int_ty.get_bit_width();
                match bit_width {
                    1..=8 => 1,
                    9..=16 => 2,
                    17..=32 => 4,
                    _ => 8,
                }
            }
            inkwell::types::BasicTypeEnum::FloatType(_) => 8, // f64 is 8-byte aligned
            inkwell::types::BasicTypeEnum::PointerType(_) => 8, // Pointers are 8-byte aligned on 64-bit
            inkwell::types::BasicTypeEnum::StructType(st) => {
                // For struct types, use max alignment of fields (recursively)
                // For simplicity, assume 8-byte alignment for nested structs
                8
            }
            inkwell::types::BasicTypeEnum::ArrayType(_) => 8,
            inkwell::types::BasicTypeEnum::VectorType(_) => 16,
            inkwell::types::BasicTypeEnum::ScalableVectorType(_) => 16,
        }
    }

    /// Convert a type string to an LLVM BasicTypeEnum
    /// Handles primitives, pointers (Str, Array, Map), and structs
    fn type_string_to_llvm_type(&self, type_str: &str) -> BasicTypeEnum<'ctx> {
        let type_str = type_str.trim();

        if type_str == "Int" || type_str == "i32" {
            self.context.i32_type().into()
        } else if type_str == "Float" || type_str == "f64" {
            self.context.f64_type().into()
        } else if type_str == "Bool" {
            // Use i32 for Bool to match internal representation (all Bools are stored as i32)
            self.context.i32_type().into()
        } else if type_str == "Str" || type_str == "String" {
            self.context.ptr_type(AddressSpace::default()).into()
        } else if type_str.starts_with("Array") {
            self.context.ptr_type(AddressSpace::default()).into()
        } else if type_str.starts_with("Map") {
            self.context.ptr_type(AddressSpace::default()).into()
        } else if type_str.starts_with("Struct(") {
            // Extract struct name from "Struct(Name)"
            let struct_name = type_str
                .trim_start_matches("Struct(")
                .trim_end_matches(")")
                .trim();

            // If we already have a canonical type for this struct, use it
            if let Some(struct_type) = self.canonical_struct_types.get(struct_name) {
                (*struct_type).into()
            } else {
                // Fallback to pointer for forward-declared structs
                self.context.ptr_type(AddressSpace::default()).into()
            }
        } else if self.struct_metadata.contains_key(type_str) {
            // Handle bare struct names (without "Struct()" wrapper) - nested structs
            // For nested structs, we store them as pointers
            self.context.ptr_type(AddressSpace::default()).into()
        } else if self.enum_table.contains_key(type_str) || type_str.starts_with("Enum(") {
            // Handle enums - represented as { i32 tag, ptr payload } struct
            let ptr_type = self.context.ptr_type(AddressSpace::default());
            self.context
                .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false)
                .into()
        } else {
            // Default fallback
            self.context.i32_type().into()
        }
    }

    /// Extract element type from Array return type string
    /// Format: "Array(Int)", "Array(Str)", "Array(User)", etc.
    pub fn extract_array_element_type_from_return(type_str: &str) -> String {
        // Try to parse Array(Type) format
        if let Some(start) = type_str.find("Array(") {
            let inner_start = start + 6; // Length of "Array("
            if let Some(end) = type_str[inner_start..].find(')') {
                let element_type = &type_str[inner_start..inner_start + end];
                return element_type.to_string();
            }
        }
        // Fallback for common types
        if type_str.contains("Array(Str)") {
            "Str".to_string()
        } else if type_str.contains("Array(Float)") {
            "Float".to_string()
        } else if type_str.contains("Array(Bool)") {
            "Bool".to_string()
        } else {
            "Int".to_string() // default
        }
    }

    /// Extract key and value types from a Map return type string
    /// Format: "Map(Str,Int)", "Map(Int,Float)", etc.
    pub fn extract_map_types_from_return(type_str: &str) -> (&str, &str) {
        // Handle Map(key,value) format
        // String key types
        if type_str.contains("Map(Str,Str)") {
            ("Str", "Str")
        } else if type_str.contains("Map(Str,Float)") {
            ("Str", "Float")
        } else if type_str.contains("Map(Str,Bool)") {
            ("Str", "Bool")
        } else if type_str.contains("Map(Str,Int)") {
            ("Str", "Int")
        }
        // Int key types
        else if type_str.contains("Map(Int,Str)") {
            ("Int", "Str")
        } else if type_str.contains("Map(Int,Float)") {
            ("Int", "Float")
        } else if type_str.contains("Map(Int,Bool)") {
            ("Int", "Bool")
        } else if type_str.contains("Map(Int,Int)") {
            ("Int", "Int")
        }
        // Float key types
        else if type_str.contains("Map(Float,Str)") {
            ("Float", "Str")
        } else if type_str.contains("Map(Float,Float)") {
            ("Float", "Float")
        } else if type_str.contains("Map(Float,Bool)") {
            ("Float", "Bool")
        } else if type_str.contains("Map(Float,Int)") {
            ("Float", "Int")
        }
        // Bool key types
        else if type_str.contains("Map(Bool,Str)") {
            ("Bool", "Str")
        } else if type_str.contains("Map(Bool,Float)") {
            ("Bool", "Float")
        } else if type_str.contains("Map(Bool,Bool)") {
            ("Bool", "Bool")
        } else if type_str.contains("Map(Bool,Int)") {
            ("Bool", "Int")
        } else {
            ("Int", "Int") // default
        }
    }

    /// Creates a minimal `main` function (`i32 ()`) that returns 0.
    /// This is a fallback to guarantee the presence of a valid entry point in the generated binary.
    /// Also executes any global-scope runtime statements (like print).
    pub fn generate_default_main(&mut self) {
        let main_type = self.context.i32_type().fn_type(&[], false);
        let main_func = self.module.add_function("main", main_type, None);

        let entry_bb = self.context.append_basic_block(main_func, "entry");
        self.builder.position_at_end(entry_bb);

        // Enable UTF-8 console output on Windows
        // Call SetConsoleOutputCP(65001) to enable UTF-8
        // This is always generated in LLVM IR; on non-Windows platforms it will be a no-op
        let set_console_cp_type = self
            .context
            .i32_type()
            .fn_type(&[self.context.i32_type().into()], false);
        let set_console_cp_fn = self
            .module
            .get_function("SetConsoleOutputCP")
            .unwrap_or_else(|| {
                self.module
                    .add_function("SetConsoleOutputCP", set_console_cp_type, None)
            });

        // 65001 is UTF-8 code page
        let utf8_codepage = self.context.i32_type().const_int(65001, false);
        self.builder
            .build_call(set_console_cp_fn, &[utf8_codepage.into()], "set_utf8")
            .unwrap();

        // Execute any runtime instructions from global scope (like Print, BinaryOp for runtime values)
        // NOTE: Skip ConstString instructions - they are already processed in generate_global
        // and stored as static constants. Processing them here would cause duplicate heap allocations.
        for instr in &self.globals.clone() {
            match instr {
                MirInstr::ConstString { .. } => {
                    // Skip - these are already defined as module-level constants in generate_global
                    // Reprocessing would cause memory leaks from duplicate malloc calls
                }
                MirInstr::Print { .. } => {
                    self.generate_instr(instr);
                }
                MirInstr::BinaryOp(_, _, _, _) => {
                    // Generate runtime binary operations that weren't constant-folded
                    self.generate_instr(instr);
                }
                _ => {
                    // Other instructions are already handled in generate_global
                }
            }
        }

        // Perform cleanup before returning from main
        self.generate_function_exit_cleanup();

        let zero = self.context.i32_type().const_int(0, false);
        // Generates the `ret i32 0` instruction.
        self.builder.build_return(Some(&zero)).unwrap();
    }

    /// Generates the LLVM structure and code for a single MIR function.
    /// Generates LLVM IR for a user-defined function.
    /// This method:
    /// - Defines the function signature (return type and parameter types).
    /// - Creates all basic blocks for control flow.
    /// - Allocates and registers parameters in the symbol table.
    /// - Translates MIR blocks and instructions into LLVM IR.
    /// - Handles block terminators (return, jump, conditional jump).
    /// Returns the LLVM FunctionValue for further manipulation or optimization.
    pub fn generate_function(&mut self, func: &MirFunction) -> FunctionValue<'ctx> {
        // FFI functions are external - don't generate body
        if func.ffi_lib.is_some() {
            let symbol_name = func.ffi_symbol.as_ref().unwrap_or(&func.name);
            return self.module.get_function(symbol_name).expect(&format!(
                "FFI function '{}' should have been predeclared",
                func.name
            ));
        }

        // Reset recursion depth for this function to prevent accumulation across functions
        self.recursion_depth = 0;

        // Set current function context for error handling
        self.current_function_name = Some(func.name.clone());
        self.current_error_type = func.error_type.clone();

        // Clear no_storage_vars to ensure fresh state for this function
        self.no_storage_vars.clear();

        // Pre-scan Call instructions to identify tuple pointer variables from Result returns
        // These should NOT get stack allocations since they're just pointers
        // This MUST happen before cross-block analysis and allocation
        for block in &func.blocks {
            for instr in &block.instrs {
                if let crate::mir::MirInstr::Call {
                    dest, func: callee, ..
                } = instr
                {
                    // Check if this function returns a Result with multi-value Ok
                    if let Some(error_type) = self.function_error_types.get(callee) {
                        if !error_type.is_empty() {
                            // This function can fail (returns Result)
                            if let Some(return_type) = self.function_return_types.get(callee) {
                                // Check if return type is multi-value
                                let is_multi_value =
                                    return_type.contains(',') || return_type.starts_with("Tuple(");

                                if is_multi_value && !dest.is_empty() {
                                    // Mark first dest as no-storage (it's the tuple pointer)
                                    self.no_storage_vars.insert(dest[0].clone());
                                }
                            }
                        }
                    }
                }
            }
        }

        // Clear symbols table to prevent conflicts between functions
        self.symbols.clear();
        self.temp_values.clear();
        self.heap_strings.clear();
        // DO NOT clear heap_arrays and heap_maps!
        // They track return values from function calls within this function's body
        // and need to persist so print can detect them as arrays/maps
        // self.heap_arrays.clear();
        // self.heap_maps.clear();
        // DO NOT clear array_metadata and map_metadata here!
        // They need to persist to track results from function calls within this function
        // self.array_metadata.clear();
        // self.map_metadata.clear();
        self.composite_string_ptrs.clear();
        self.composite_strings.clear();

        // Note: function return types are already stored in the pre-scan phase above
        // No need to store them again here

        // Track function parameters for RC handling on return
        self.current_function_params.clear();
        for (i, param_name) in func.params.iter().enumerate() {
            let param_type = func.param_types.get(i).and_then(|t| t.clone());
            self.current_function_params
                .push((param_name.clone(), param_type));
        }

        // Use the predeclared function (which already has correct signature with Result wrapping if needed)
        let llvm_func = self.module.get_function(&func.name).expect(&format!(
            "Function '{}' should have been predeclared",
            func.name
        ));

        // Create a separate entry block for parameter allocation
        let entry_block = self.context.append_basic_block(llvm_func, "entry");
        self.builder.position_at_end(entry_block);

        // Create all necessary basic blocks within the function (e.g., entry, if.then, loop.body).
        let mut bb_map = HashMap::new();
        for block in &func.blocks {
            let bb = self.context.append_basic_block(llvm_func, &block.label);
            bb_map.insert(block.label.clone(), bb);
        }

        // Clear ALL state before starting a new function
        // Each function must have completely fresh scope with no interference from previous functions
        self.symbols.clear();
        self.array_metadata.clear();
        self.map_metadata.clear();
        self.arrayget_sources.clear();
        self.temp_values.clear();
        self.heap_strings.clear();
        self.heap_arrays.clear();
        self.heap_maps.clear();
        self.composite_string_ptrs.clear();
        self.loop_stack.clear();
        self.loop_local_vars.clear();
        self.struct_field_sources.clear();
        self.struct_instance_types.clear();
        self.variable_types.clear();

        // Allocate space for parameters and store their incoming values in the entry block.
        // This ensures parameters are available as local variables in the function scope.
        for (i, param) in func.params.iter().enumerate() {
            let param_val = llvm_func.get_nth_param(i as u32).unwrap();

            // Get the correct type for this parameter
            let param_type = if let Some(Some(ref type_str)) = func.param_types.get(i) {
                // Check if this is a known struct type (either "Struct(Name)" or just "Name")
                let is_struct =
                    type_str.contains("Struct(") || self.struct_metadata.contains_key(type_str);

                // Check if this is an enum type
                let is_enum =
                    type_str.starts_with("Enum(") || self.enum_table.contains_key(type_str);

                // Map MIR type strings to LLVM types
                // Primitives (Int, Float, Bool) are passed by value
                // Strings, Arrays, Maps, and Structs are passed by pointer
                // Enums are passed as struct { i32 tag, ptr payload } by value
                if type_str == "Int" {
                    self.context.i32_type().into()
                } else if type_str == "Float" {
                    self.context.f64_type().into()
                } else if type_str == "Bool" {
                    // Use i32 for Bool to match parameter type (now i32 not i1)
                    self.context.i32_type().into()
                } else if type_str.contains("String") || type_str.contains("Str") {
                    self.context.ptr_type(AddressSpace::default()).into()
                } else if type_str.contains("Array") {
                    self.context.ptr_type(AddressSpace::default()).into()
                } else if type_str.contains("Map") {
                    self.context.ptr_type(AddressSpace::default()).into()
                } else if is_enum {
                    // Enum is represented as { i32 tag, ptr payload }
                    let ptr_type = self.context.ptr_type(AddressSpace::default());
                    self.context
                        .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false)
                        .into()
                } else if is_struct {
                    // Struct parameters are passed as pointers
                    self.context.ptr_type(AddressSpace::default()).into()
                } else {
                    self.context.i32_type().into()
                }
            } else {
                self.context.i32_type().into()
            };

            let alloca = self
                .builder
                .build_alloca(param_type, param)
                .expect("Failed to allocate function parameter");

            // Store the incoming parameter value into the allocated space
            self.builder
                .build_store(alloca, param_val)
                .expect("Failed to store parameter value");

            // Register the parameter in the symbol table for future lookups.
            self.symbols.insert(
                param.clone(),
                crate::codegen::Symbol {
                    ptr: alloca,
                    ty: param_type,
                },
            );

            // Track parameter type and struct instance metadata for downstream codegen
            if let Some(Some(ref type_str)) = func.param_types.get(i) {
                self.variable_types.insert(param.clone(), type_str.clone());

                // If this is a struct parameter, record the struct name for field accesses
                let is_struct =
                    type_str.contains("Struct(") || self.struct_metadata.contains_key(type_str);
                if is_struct {
                    let struct_name = if type_str.starts_with("Struct(") && type_str.ends_with(")")
                    {
                        type_str
                            .trim_start_matches("Struct(")
                            .trim_end_matches(')')
                            .to_string()
                    } else {
                        type_str.clone()
                    };
                    self.struct_instance_types
                        .insert(param.clone(), struct_name.clone());
                    self.struct_instance_types
                        .insert(format!("%{}", param), struct_name);
                }
            }

            // Create metadata for array and map parameters
            // This is crucial for imported functions to work with arrays/maps
            if let Some(Some(ref type_str)) = func.param_types.get(i) {
                // Check if this is a known struct type (either "Struct(Name)" or just "Name")
                let is_struct =
                    type_str.contains("Struct(") || self.struct_metadata.contains_key(type_str);

                if type_str.contains("Array") {
                    // Extract element type from Array(Type) format
                    let element_type = Self::extract_array_element_type(type_str);

                    let contains_strings = element_type == "Str";

                    // We don't know the actual length, so we need to extract it from the array pointer
                    // For now, we'll mark it as having unknown length (0) and rely on builtin methods
                    // The actual length will be computed when needed via array.len()
                    self.array_metadata.insert(
                        param.clone(),
                        crate::codegen::ArrayMetadata {
                            length: 0, // Unknown at function entry - will use runtime length checks
                            element_type: element_type.to_string(),
                            contains_strings,
                        },
                    );

                    // Also store the parameter value so it can be resolved
                    self.temp_values.insert(param.clone(), param_val);
                } else if type_str.contains("Map") {
                    // Extract key and value types from Map(Key,Value) format
                    let (key_type, value_type) = Self::extract_map_types(type_str);

                    let key_is_string = key_type == "Str";
                    let value_is_string = value_type == "Str";

                    self.map_metadata.insert(
                        param.clone(),
                        crate::codegen::MapMetadata {
                            length: 0, // Unknown at function entry
                            key_type: key_type.to_string(),
                            value_type: value_type.to_string(),
                            key_is_string,
                            value_is_string,
                            key_needs_rc: key_is_string,
                            value_needs_rc: value_is_string,
                        },
                    );

                    // Also store the parameter value so it can be resolved
                    self.temp_values.insert(param.clone(), param_val);
                } else if type_str == "Int" || type_str == "Float" || type_str == "Bool" {
                    // Store primitive parameters in temp_values for direct resolution
                    self.temp_values.insert(param.clone(), param_val);
                    // Track variable types for print formatting
                    self.variable_types.insert(param.clone(), type_str.clone());
                    // Mark Bool parameters for proper "true"/"false" printing
                    if type_str == "Bool" {
                        self.boolean_temps.insert(param.clone());
                    }
                } else if type_str.starts_with("Enum(") || self.enum_table.contains_key(type_str) {
                    // Enum parameter - store in temp_values and track type
                    self.temp_values.insert(param.clone(), param_val);

                    // Normalize the type string to "Enum(Name)" format
                    let normalized_type = if type_str.starts_with("Enum(") {
                        type_str.clone()
                    } else {
                        format!("Enum({})", type_str)
                    };
                    self.variable_types.insert(param.clone(), normalized_type);
                } else if is_struct {
                    // Extract the actual struct name from type string
                    let struct_name = if type_str.starts_with("Struct(") && type_str.ends_with(")")
                    {
                        // Extract name from "Struct(Name)" format
                        &type_str[7..type_str.len() - 1]
                    } else {
                        // It's already just the struct name
                        type_str.as_str()
                    };

                    // Normalize the type string to "Struct(Name)" format
                    let normalized_type = format!("Struct({})", struct_name);

                    // Store struct type metadata and temp value for struct parameters
                    self.variable_types.insert(param.clone(), normalized_type);
                    self.temp_values.insert(param.clone(), param_val);

                    // Track struct instance type for method lookup (with multiple name variations)
                    self.struct_instance_types
                        .insert(param.clone(), struct_name.to_string());
                    if param.starts_with('%') {
                        self.struct_instance_types.insert(
                            param.trim_start_matches('%').to_string(),
                            struct_name.to_string(),
                        );
                    }
                    if !param.starts_with('%') {
                        self.struct_instance_types
                            .insert(format!("%{}", param), struct_name.to_string());
                    }

                    // Don't track structs in heap_arrays - that's only for actual arrays
                    // Structs have their own tracking via struct_instance_types
                }
            }
        }

        // Pre-allocate variables that are used across multiple blocks
        // This is necessary for proper SSA form and cross-block variable access
        use std::collections::HashSet;
        let mut defined_vars: HashMap<String, HashSet<String>> = HashMap::new(); // block -> vars defined
        let mut used_vars: HashMap<String, HashSet<String>> = HashMap::new(); // block -> vars used

        // Scan all blocks to find variable definitions and uses
        for block in &func.blocks {
            let mut block_defs = HashSet::new();
            let mut block_uses = HashSet::new();

            for instr in &block.instrs {
                match instr {
                    crate::mir::MirInstr::Assign { name, value, .. } => {
                        block_defs.insert(name.clone());
                        // Track ALL variable uses, including temps starting with %
                        // This is critical for detecting cross-block temps from ternary/conditional expressions
                        if !value.parse::<i32>().is_ok() && value != "true" && value != "false" {
                            block_uses.insert(value.clone());
                        }
                    }
                    crate::mir::MirInstr::BinaryOp(_, _, left, right) => {
                        // Track ALL variable uses, including temps starting with %
                        if !left.parse::<i32>().is_ok() && left != "true" && left != "false" {
                            block_uses.insert(left.clone());
                        }
                        if !right.parse::<i32>().is_ok() && right != "true" && right != "false" {
                            block_uses.insert(right.clone());
                        }
                    }
                    crate::mir::MirInstr::ArrayLen { array, .. } => {
                        // Track ALL variable uses, including temps
                        block_uses.insert(array.clone());
                    }
                    crate::mir::MirInstr::ArrayGet { array, index, .. } => {
                        // Track ALL variable uses, including temps
                        block_uses.insert(array.clone());
                        block_uses.insert(index.clone());
                    }
                    // TupleGet defines a variable from a tuple element
                    // This is critical for match arm payload bindings
                    crate::mir::MirInstr::TupleGet { name, tuple, .. } => {
                        block_defs.insert(name.clone());
                        block_uses.insert(tuple.clone());
                    }
                    // EnumGetPayload defines a variable from enum payload
                    crate::mir::MirInstr::EnumGetPayload {
                        name, enum_value, ..
                    } => {
                        block_defs.insert(name.clone());
                        block_uses.insert(enum_value.clone());
                    }
                    // ConstInt/ConstString/ConstBool/ConstFloat define variables
                    crate::mir::MirInstr::ConstInt { name, .. } => {
                        block_defs.insert(name.clone());
                    }
                    crate::mir::MirInstr::ConstFloat { name, .. } => {
                        block_defs.insert(name.clone());
                    }
                    crate::mir::MirInstr::ConstBool { name, .. } => {
                        block_defs.insert(name.clone());
                    }
                    crate::mir::MirInstr::ConstString { name, .. } => {
                        block_defs.insert(name.clone());
                    }
                    // Call defines destination variables
                    crate::mir::MirInstr::Call { dest, args, .. } => {
                        for d in dest {
                            block_defs.insert(d.clone());
                        }
                        for arg in args {
                            if !arg.parse::<i32>().is_ok() && arg != "true" && arg != "false" {
                                block_uses.insert(arg.clone());
                            }
                        }
                    }
                    // StructInit defines a variable
                    crate::mir::MirInstr::StructInit { name, fields, .. } => {
                        block_defs.insert(name.clone());
                        for (_, val) in fields {
                            if !val.parse::<i32>().is_ok() && val != "true" && val != "false" {
                                block_uses.insert(val.clone());
                            }
                        }
                    }
                    // StructGet defines a variable and uses the struct instance
                    crate::mir::MirInstr::StructGet {
                        name,
                        struct_instance,
                        ..
                    } => {
                        block_defs.insert(name.clone());
                        block_uses.insert(struct_instance.clone());
                    }
                    // StructSet uses the struct instance and value
                    crate::mir::MirInstr::StructSet {
                        struct_instance,
                        value,
                        ..
                    } => {
                        block_uses.insert(struct_instance.clone());
                        if !value.parse::<i32>().is_ok() && value != "true" && value != "false" {
                            block_uses.insert(value.clone());
                        }
                    }
                    // EnumInit defines a variable
                    crate::mir::MirInstr::EnumInit { name, value, .. } => {
                        block_defs.insert(name.clone());
                        if let Some(v) = value {
                            if !v.parse::<i32>().is_ok() && v != "true" && v != "false" {
                                block_uses.insert(v.clone());
                            }
                        }
                    }
                    // EnumGetTag defines a variable
                    crate::mir::MirInstr::EnumGetTag {
                        name, enum_value, ..
                    } => {
                        block_defs.insert(name.clone());
                        block_uses.insert(enum_value.clone());
                    }
                    // Print uses variables
                    crate::mir::MirInstr::Print { values } => {
                        for val in values {
                            if !val.parse::<i32>().is_ok() && val != "true" && val != "false" {
                                block_uses.insert(val.clone());
                            }
                        }
                    }
                    // StringConcat defines a variable and uses operands
                    crate::mir::MirInstr::StringConcat { name, left, right } => {
                        block_defs.insert(name.clone());
                        block_uses.insert(left.clone());
                        block_uses.insert(right.clone());
                    }
                    // Cast defines a variable and uses the source
                    crate::mir::MirInstr::Cast { name, value, .. } => {
                        block_defs.insert(name.clone());
                        if !value.parse::<i32>().is_ok() && value != "true" && value != "false" {
                            block_uses.insert(value.clone());
                        }
                    }
                    // TryPropagate defines a variable and uses the result
                    crate::mir::MirInstr::TryPropagate { name, result, .. } => {
                        block_defs.insert(name.clone());
                        if !result.parse::<i32>().is_ok() && result != "true" && result != "false" {
                            block_uses.insert(result.clone());
                        }
                    }
                    _ => {}
                }
            }

            // Check terminator for variable uses
            if let Some(term) = &block.terminator {
                match term {
                    crate::mir::MirInstr::CondJump { cond, .. } => {
                        // Track ALL variable uses, including temps starting with %
                        if !cond.parse::<i32>().is_ok() && cond != "true" && cond != "false" {
                            block_uses.insert(cond.clone());
                        }
                    }
                    _ => {}
                }
            }

            defined_vars.insert(block.label.clone(), block_defs);
            used_vars.insert(block.label.clone(), block_uses);
        }

        // Find variables that are defined in one block and used in another
        let mut cross_block_vars = HashSet::new();
        for (use_block, uses) in &used_vars {
            for var in uses {
                // Check if this variable is defined in a different block
                let mut defined_elsewhere = false;
                for (def_block, defs) in &defined_vars {
                    if def_block != use_block && defs.contains(var) {
                        defined_elsewhere = true;
                        break;
                    }
                }
                if defined_elsewhere {
                    cross_block_vars.insert(var.clone());
                }
                // Also check: if var starts with % and is NOT defined in ANY block,
                // it's a temp from nested expression that needs allocation
                if var.starts_with('%') && !defined_vars.values().any(|defs| defs.contains(var)) {
                    // This temp is used but never defined - likely from nested match
                    cross_block_vars.insert(var.clone());
                }
            }
        }

        // Determine variable types by scanning instructions that define them
        let mut var_types: HashMap<String, BasicTypeEnum<'ctx>> = HashMap::new();
        // Track which function produced each tuple (for TupleExtract type resolution)
        let mut tuple_sources: HashMap<String, String> = HashMap::new();
        // Track which function call produced each temp result (for TryPropagate to trace back)
        let mut call_sources: HashMap<String, String> = HashMap::new();
        // Track tuple element types from EnumGetPayload (for TupleGet type resolution)
        let mut tuple_element_types: HashMap<String, Vec<BasicTypeEnum<'ctx>>> = HashMap::new();

        // First pass: process all instructions except Assign
        for block in &func.blocks {
            for instr in &block.instrs {
                match instr {
                    // StructDecl - register struct metadata and canonical type for local structs
                    crate::mir::MirInstr::StructDecl {
                        struct_name,
                        field_names,
                        field_types,
                    } => {
                        // Only register if not already registered (e.g., from globals)
                        if !self.struct_metadata.contains_key(struct_name) {
                            // Create the canonical LLVM struct type
                            let llvm_field_types: Vec<BasicTypeEnum> = field_types
                                .iter()
                                .map(|type_str| self.type_string_to_llvm_type(type_str))
                                .collect();

                            let struct_type = self.context.struct_type(&llvm_field_types, false);
                            self.canonical_struct_types
                                .insert(struct_name.clone(), struct_type);

                            // Compute layout
                            let (field_layouts, total_size, total_align) =
                                self.compute_struct_layout(struct_type, field_names, field_types);

                            let metadata = crate::codegen::core::context::StructMetadata {
                                field_names: field_names.clone(),
                                field_types: field_types.clone(),
                                field_layouts,
                                total_size,
                                total_align,
                            };
                            self.struct_metadata.insert(struct_name.clone(), metadata);
                        }
                    }
                    // Arrays are always pointers
                    crate::mir::MirInstr::Array { name, .. } => {
                        var_types.insert(
                            name.clone(),
                            self.context.ptr_type(AddressSpace::default()).into(),
                        );
                    }
                    // Maps are always pointers
                    crate::mir::MirInstr::Map { name, .. } => {
                        var_types.insert(
                            name.clone(),
                            self.context.ptr_type(AddressSpace::default()).into(),
                        );
                    }
                    // ArraySlice results are always pointers (heap-allocated arrays)
                    crate::mir::MirInstr::ArraySlice { name, .. } => {
                        var_types.insert(
                            name.clone(),
                            self.context.ptr_type(AddressSpace::default()).into(),
                        );
                    }
                    // ArrayGet results - check if array contains structs (pointers) or primitives
                    crate::mir::MirInstr::ArrayGet { name, array, .. } => {
                        // Check if the array contains struct elements by looking at array metadata
                        // or by checking if the array name suggests it's a struct array
                        // For now, default to pointer type for struct arrays
                        // This will be refined at codegen time based on actual array metadata
                        if let Some(arr_type) = var_types.get(array) {
                            if arr_type.is_pointer_type() {
                                // Array of pointers - element is also a pointer (struct)
                                var_types.insert(
                                    name.clone(),
                                    self.context.ptr_type(AddressSpace::default()).into(),
                                );
                            } else {
                                // Array of primitives - element is the primitive type
                                var_types.insert(name.clone(), *arr_type);
                            }
                        } else {
                            // Default to pointer for struct arrays (most common case in loops)
                            var_types.insert(
                                name.clone(),
                                self.context.ptr_type(AddressSpace::default()).into(),
                            );
                        }

                        // CRITICAL: Track struct instance type for ArrayGet result
                        // This is needed for method calls on loop iteration variables
                        // Look up the array's element type from array_metadata
                        if let Some(arr_meta) = self.array_metadata.get(array) {
                            let elem_type = &arr_meta.element_type;
                            // Check if element type is a struct
                            if self.struct_metadata.contains_key(elem_type) {
                                self.struct_instance_types
                                    .insert(name.clone(), elem_type.clone());
                            }
                        }
                    }
                    // Strings are always pointers
                    crate::mir::MirInstr::ConstString { name, .. } => {
                        var_types.insert(
                            name.clone(),
                            self.context.ptr_type(AddressSpace::default()).into(),
                        );
                    }
                    // Variables with "_array" or "_map" suffix are pointers
                    // BUT: exclude index variables (ending with __index)
                    crate::mir::MirInstr::Assign { name, value, .. } => {
                        // CRITICAL: Propagate struct_instance_types from value to name
                        // This ensures loop iteration variables have correct struct type tracking
                        if let Some(struct_type) = self.struct_instance_types.get(value).cloned() {
                            self.struct_instance_types.insert(name.clone(), struct_type);
                        }

                        // CRITICAL: Propagate array_metadata from value to name
                        // This ensures loop array copies (task_array = tasks) have correct element type info
                        if let Some(arr_meta) = self.array_metadata.get(value).cloned() {
                            self.array_metadata.insert(name.clone(), arr_meta);
                        }

                        // Index variables are always i32
                        if name.ends_with("__index") || name.ends_with("_end") {
                            var_types.insert(name.clone(), self.context.i32_type().into());
                        } else if name.ends_with("_array")
                            || name.ends_with("_map")
                            || name.ends_with("item_array")
                            || name.ends_with("_ptr")
                        {
                            // Only mark as pointer if it's NOT an index variable
                            var_types.insert(
                                name.clone(),
                                self.context.ptr_type(AddressSpace::default()).into(),
                            );
                        }
                        // If assigned from a known type, preserve that type
                        // BUT: not if this is an index variable
                        else if !name.ends_with("__index") && !name.ends_with("_end") {
                            if let Some(val_type) = var_types.get(value) {
                                var_types.insert(name.clone(), *val_type);
                            }
                        }
                    }
                    // ArrayLen results are i32
                    crate::mir::MirInstr::ArrayLen { name, .. } => {
                        var_types.insert(name.clone(), self.context.i32_type().into());
                    }
                    // MapLen results are i32
                    crate::mir::MirInstr::MapLen { name, .. } => {
                        var_types.insert(name.clone(), self.context.i32_type().into());
                    }
                    // Integer constants are i32
                    crate::mir::MirInstr::ConstInt { name, .. } => {
                        var_types.insert(name.clone(), self.context.i32_type().into());
                    }
                    // Float constants are f64
                    crate::mir::MirInstr::ConstFloat { name, .. } => {
                        var_types.insert(name.clone(), self.context.f64_type().into());
                    }
                    // Boolean constants are i32
                    crate::mir::MirInstr::ConstBool { name, .. } => {
                        var_types.insert(name.clone(), self.context.i32_type().into());
                    }
                    // EnumInit - enums are represented as { i32, ptr } structs
                    crate::mir::MirInstr::EnumInit { name, .. } => {
                        let ptr_type = self.context.ptr_type(AddressSpace::default());
                        let enum_struct_type = self
                            .context
                            .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false);
                        var_types.insert(name.clone(), enum_struct_type.into());
                    }
                    // TupleCreate - tuples are stored as pointers to heap-allocated structs
                    crate::mir::MirInstr::TupleCreate { name, .. } => {
                        var_types.insert(
                            name.clone(),
                            self.context.ptr_type(AddressSpace::default()).into(),
                        );
                    }
                    // StructInit - structs are stored as pointers (heap-allocated)
                    crate::mir::MirInstr::StructInit { name, .. } => {
                        var_types.insert(
                            name.clone(),
                            self.context.ptr_type(AddressSpace::default()).into(),
                        );
                    }
                    // StructGet - field access, type depends on field
                    crate::mir::MirInstr::StructGet {
                        name,
                        struct_instance,
                        field,
                        ..
                    } => {
                        // Try to determine the actual field type from struct metadata
                        let field_type = if let Some(struct_type_str) =
                            self.variable_types.get(struct_instance)
                        {
                            // Extract struct name from type string
                            let struct_name = if struct_type_str.starts_with("Struct(")
                                && struct_type_str.ends_with(")")
                            {
                                &struct_type_str[7..struct_type_str.len() - 1]
                            } else if !struct_type_str.is_empty()
                                && struct_type_str != "Unknown"
                                && !struct_type_str.starts_with("Array")
                                && !struct_type_str.starts_with("Map")
                                && !struct_type_str.starts_with("Int")
                                && !struct_type_str.starts_with("Float")
                                && !struct_type_str.starts_with("Bool")
                                && !struct_type_str.starts_with("Str")
                            {
                                struct_type_str.as_str()
                            } else {
                                ""
                            };

                            // Look up field type from struct metadata
                            if let Some(metadata) = self.struct_metadata.get(struct_name) {
                                if let Some(field_index) =
                                    metadata.field_names.iter().position(|f| f == field)
                                {
                                    if let Some(field_type_name) =
                                        metadata.field_types.get(field_index)
                                    {
                                        // Determine LLVM type based on field type name
                                        if field_type_name == "Str" || field_type_name == "String" {
                                            self.context.ptr_type(AddressSpace::default()).into()
                                        } else if field_type_name.starts_with("Array(") {
                                            // CRITICAL: Also populate array_metadata for array fields
                                            // This ensures methods like push() work on struct array fields
                                            let element_type =
                                                &field_type_name[6..field_type_name.len() - 1];
                                            let contains_strings = element_type == "Str";
                                            self.array_metadata.insert(
                                                name.clone(),
                                                crate::codegen::ArrayMetadata {
                                                    length: 0,
                                                    element_type: element_type.to_string(),
                                                    contains_strings,
                                                },
                                            );
                                            // Also track as heap array
                                            self.heap_arrays.insert(name.clone());
                                            self.context.ptr_type(AddressSpace::default()).into()
                                        } else if field_type_name.starts_with("Map(") {
                                            self.context.ptr_type(AddressSpace::default()).into()
                                        } else if self.struct_metadata.contains_key(field_type_name)
                                        {
                                            // Nested struct - pointer type
                                            self.context.ptr_type(AddressSpace::default()).into()
                                        } else if self.enum_table.contains_key(field_type_name)
                                            || field_type_name.starts_with("Enum(")
                                        {
                                            // Enum type - represented as { i32 tag, ptr payload } struct
                                            let ptr_type =
                                                self.context.ptr_type(AddressSpace::default());
                                            self.context
                                                .struct_type(
                                                    &[
                                                        self.context.i32_type().into(),
                                                        ptr_type.into(),
                                                    ],
                                                    false,
                                                )
                                                .into()
                                        } else if field_type_name == "Float" {
                                            self.context.f64_type().into()
                                        } else if field_type_name == "Bool" {
                                            self.context.i32_type().into()
                                        } else {
                                            // Default to i32 for Int and unknown types
                                            self.context.i32_type().into()
                                        }
                                    } else {
                                        self.context.i32_type().into()
                                    }
                                } else {
                                    self.context.i32_type().into()
                                }
                            } else {
                                self.context.i32_type().into()
                            }
                        } else {
                            // No type info available, default to i32
                            self.context.i32_type().into()
                        };

                        var_types.insert(name.clone(), field_type);
                    }
                    // StringConcat - result is a string pointer
                    crate::mir::MirInstr::StringConcat { name, .. } => {
                        var_types.insert(
                            name.clone(),
                            self.context.ptr_type(AddressSpace::default()).into(),
                        );
                    }
                    // TupleExtract - determine type from tuple element
                    crate::mir::MirInstr::TupleExtract {
                        name,
                        source,
                        index,
                    } => {
                        // Look up the function that produced this tuple via tuple_sources
                        if let Some(func_name) = tuple_sources.get(source) {
                            if let Some(return_type_str) = self.function_return_types.get(func_name)
                            {
                                // Strip Tuple() wrapper if present
                                let inner = if return_type_str.starts_with("Tuple(")
                                    && return_type_str.ends_with(')')
                                {
                                    &return_type_str[6..return_type_str.len() - 1]
                                } else {
                                    return_type_str.as_str()
                                };
                                let types = parse_tuple_types(inner);
                                if let Some(type_str) = types.get(*index) {
                                    let llvm_type = if type_str.starts_with('[')
                                        || type_str.starts_with("Array")
                                    {
                                        // Array type
                                        self.context.ptr_type(AddressSpace::default()).into()
                                    } else if type_str.starts_with('{')
                                        || type_str.starts_with("Map")
                                    {
                                        // Map type
                                        self.context.ptr_type(AddressSpace::default()).into()
                                    } else if type_str.contains("Str")
                                        || type_str.contains("String")
                                    {
                                        // String type
                                        self.context.ptr_type(AddressSpace::default()).into()
                                    } else if type_str.contains("Float") {
                                        // Float type
                                        self.context.f64_type().into()
                                    } else if type_str.contains("Bool") {
                                        // Bool type
                                        self.context.i32_type().into()
                                    } else {
                                        // Int or default
                                        self.context.i32_type().into()
                                    };
                                    var_types.insert(name.clone(), llvm_type);
                                }
                            }
                        }
                    }
                    // Call - determine type from function return type
                    crate::mir::MirInstr::Call { dest, func, .. } => {
                        if let Some(return_type_str) = self.function_return_types.get(func) {
                            if dest.len() == 1 {
                                let dest_name = &dest[0];
                                // Track which function produced this result (for TryPropagate)
                                call_sources.insert(dest_name.clone(), func.clone());
                                // Check if this is a tuple return by parsing the type
                                // Strip Tuple() wrapper if present
                                let inner = if return_type_str.starts_with("Tuple(")
                                    && return_type_str.ends_with(')')
                                {
                                    &return_type_str[6..return_type_str.len() - 1]
                                } else {
                                    return_type_str.as_str()
                                };
                                let types = crate::codegen::core::helpers::parse_tuple_types(inner);
                                if types.len() > 1 {
                                    // This is a tuple return - track it so TupleExtract can find types
                                    tuple_sources.insert(dest_name.clone(), func.clone());
                                } else {
                                    // Single return value
                                    if return_type_str.contains("Array")
                                        || return_type_str.contains("Map")
                                        || return_type_str.contains("Str")
                                    {
                                        var_types.insert(
                                            dest_name.clone(),
                                            self.context.ptr_type(AddressSpace::default()).into(),
                                        );
                                    } else if return_type_str.contains("Float") {
                                        var_types.insert(
                                            dest_name.clone(),
                                            self.context.f64_type().into(),
                                        );
                                    } else if return_type_str.contains("Bool") {
                                        var_types.insert(
                                            dest_name.clone(),
                                            self.context.i32_type().into(),
                                        );
                                    } else {
                                        var_types.insert(
                                            dest_name.clone(),
                                            self.context.i32_type().into(),
                                        );
                                    }

                                    // CRITICAL: Track struct_instance_types for struct return types
                                    // This ensures method calls on function results work correctly
                                    // e.g., let store = CreateStore(); store.Add(...);
                                    let struct_name = if return_type_str.starts_with("Struct(")
                                        && return_type_str.ends_with(")")
                                    {
                                        Some(&return_type_str[7..return_type_str.len() - 1])
                                    } else if self.struct_metadata.contains_key(return_type_str) {
                                        Some(return_type_str.as_str())
                                    } else {
                                        None
                                    };

                                    if let Some(sname) = struct_name {
                                        self.struct_instance_types
                                            .insert(dest_name.clone(), sname.to_string());
                                        // Also set var_type to pointer for struct types
                                        var_types.insert(
                                            dest_name.clone(),
                                            self.context.ptr_type(AddressSpace::default()).into(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    // MethodCall - determine type from method (especially JSON.stringify/parse)
                    crate::mir::MirInstr::MethodCall {
                        dest,
                        object,
                        method,
                        ..
                    } => {
                        // JSON methods return strings (pointers)
                        if object == "JSON" {
                            if method == "stringify" || method == "parse" {
                                var_types.insert(
                                    dest.clone(),
                                    self.context.ptr_type(AddressSpace::default()).into(),
                                );
                            }
                        }
                        // String methods that return strings
                        else if method == "trim"
                            || method == "toLowerCase"
                            || method == "toUpperCase"
                            || method == "toUpper"
                            || method == "toLower"
                            || method == "replace"
                            || method == "substring"
                            || method == "slice"
                            || method == "split"
                            || method == "join"
                            || method == "concat"
                            || method == "repeat"
                            || method == "padStart"
                            || method == "padEnd"
                            || method == "charAt"
                            || method == "reverse"
                        {
                            var_types.insert(
                                dest.clone(),
                                self.context.ptr_type(AddressSpace::default()).into(),
                            );
                        }
                        // String methods that return integers
                        else if method == "length"
                            || method == "indexOf"
                            || method == "lastIndexOf"
                        {
                            var_types.insert(dest.clone(), self.context.i32_type().into());
                        }
                        // Int.toString returns string
                        else if method == "toString" {
                            var_types.insert(
                                dest.clone(),
                                self.context.ptr_type(AddressSpace::default()).into(),
                            );
                        }
                        // Array methods that return arrays (pointers)
                        else if method == "map" || method == "filter" {
                            var_types.insert(
                                dest.clone(),
                                self.context.ptr_type(AddressSpace::default()).into(),
                            );
                        }
                        // Array reduce returns the accumulator type (default to i32)
                        else if method == "reduce" {
                            var_types.insert(dest.clone(), self.context.i32_type().into());
                        }
                        // String startsWith/endsWith/contains/includes return bool (i32)
                        else if method == "startsWith"
                            || method == "endsWith"
                            || method == "contains"
                            || method == "includes"
                        {
                            var_types.insert(dest.clone(), self.context.i32_type().into());
                        }
                        // String len returns i32
                        else if method == "len" {
                            var_types.insert(dest.clone(), self.context.i32_type().into());
                        }
                        // Check if this is a user-defined struct method
                        // Look up the struct type and method return type from function_return_types
                        else {
                            // Try to find struct type for the object from multiple sources:
                            // 1. struct_instance_types (populated during earlier passes)
                            // 2. variable_types (may have type info)
                            // 3. Scan function_return_types for methods matching this object's possible types
                            let struct_type = self
                                .struct_instance_types
                                .get(object)
                                .cloned()
                                .or_else(|| {
                                    // Check variable_types
                                    self.variable_types.get(object).and_then(|vt| {
                                        if vt.starts_with("Struct(") && vt.ends_with(")") {
                                            Some(vt[7..vt.len() - 1].to_string())
                                        } else if self.struct_metadata.contains_key(vt) {
                                            Some(vt.clone())
                                        } else {
                                            None
                                        }
                                    })
                                })
                                .or_else(|| {
                                    // Scan function_return_types for any Type::method that matches
                                    // This handles cases where struct_instance_types isn't populated yet
                                    for func_name in self.function_return_types.keys() {
                                        if func_name.ends_with(&format!("::{}", method)) {
                                            // Extract the struct name from "StructName::method"
                                            if let Some(struct_name) =
                                                func_name.strip_suffix(&format!("::{}", method))
                                            {
                                                // Verify this is actually a struct type
                                                if self.struct_metadata.contains_key(struct_name) {
                                                    return Some(struct_name.to_string());
                                                }
                                            }
                                        }
                                    }
                                    None
                                });

                            if let Some(struct_name) = struct_type {
                                // Build the method name as "StructName::MethodName"
                                let full_method_name = format!("{}::{}", struct_name, method);
                                if let Some(return_type_str) =
                                    self.function_return_types.get(&full_method_name)
                                {
                                    // Determine the LLVM type from the return type string
                                    let llvm_type = match return_type_str.as_str() {
                                        "Str" | "String" => {
                                            self.context.ptr_type(AddressSpace::default()).into()
                                        }
                                        "Int" => self.context.i32_type().into(),
                                        "Float" => self.context.f64_type().into(),
                                        "Bool" => self.context.i32_type().into(),
                                        t if t.starts_with("Array(") => {
                                            self.context.ptr_type(AddressSpace::default()).into()
                                        }
                                        t if t.starts_with("Map(") => {
                                            self.context.ptr_type(AddressSpace::default()).into()
                                        }
                                        t if t.starts_with("Struct(") => {
                                            self.context.ptr_type(AddressSpace::default()).into()
                                        }
                                        _ => {
                                            // Check if it's a struct type reference
                                            if self.struct_metadata.contains_key(return_type_str) {
                                                self.context
                                                    .ptr_type(AddressSpace::default())
                                                    .into()
                                            } else {
                                                // Default to i32 for unknown types
                                                self.context.i32_type().into()
                                            }
                                        }
                                    };
                                    var_types.insert(dest.clone(), llvm_type);
                                }
                            }
                        }
                    }
                    // Binary operations - determine type from op string
                    crate::mir::MirInstr::BinaryOp(op, name, ..) => {
                        // Op format is "operator:type" e.g. "add:float", "mul:int", "ge:int"
                        // Comparison and logical operators ALWAYS return Bool (stored as i32)
                        // regardless of operand type
                        let is_comparison = op.starts_with("eq:")
                            || op.starts_with("ne:")
                            || op.starts_with("lt:")
                            || op.starts_with("le:")
                            || op.starts_with("gt:")
                            || op.starts_with("ge:")
                            || op.starts_with("and:")
                            || op.starts_with("or:");

                        let op_type = if is_comparison {
                            // Comparisons and logical ops return Bool (i32)
                            self.context.i32_type().into()
                        } else if op.contains(":float") {
                            self.context.f64_type().into()
                        } else {
                            // Default to i32 for int operations
                            self.context.i32_type().into()
                        };
                        var_types.insert(name.clone(), op_type);
                    }
                    // Cast operations - determine type from target_type
                    crate::mir::MirInstr::Cast {
                        name, target_type, ..
                    } => {
                        let cast_type = match target_type.as_str() {
                            "Float" => self.context.f64_type().into(),
                            "String" => self.context.ptr_type(AddressSpace::default()).into(),
                            "Bool" => self.context.i32_type().into(),
                            _ => self.context.i32_type().into(),
                        };
                        var_types.insert(name.clone(), cast_type);
                    }
                    // TupleGet - determine type from tuple_element_types tracked from EnumGetPayload
                    // This is critical for match arm payload bindings that use tuple elements
                    crate::mir::MirInstr::TupleGet { name, tuple, index } => {
                        // Look up the tuple element types from the source tuple
                        if let Some(elem_types) = tuple_element_types.get(tuple) {
                            if let Some(elem_type) = elem_types.get(*index) {
                                var_types.insert(name.clone(), *elem_type);
                            } else {
                                // Index out of bounds - default to i32
                                var_types.insert(name.clone(), self.context.i32_type().into());
                            }
                        } else {
                            // No tuple element types found - default to i32
                            var_types.insert(name.clone(), self.context.i32_type().into());
                        }
                    }
                    // EnumGetPayload - determine type from payload_type
                    crate::mir::MirInstr::EnumGetPayload {
                        name, payload_type, ..
                    } => {
                        let payload_llvm_type: BasicTypeEnum = if let Some(ref ptype) = payload_type
                        {
                            match ptype {
                                crate::parser::ast::TypeNode::Int => self.context.i32_type().into(),
                                crate::parser::ast::TypeNode::Float => {
                                    self.context.f64_type().into()
                                }
                                crate::parser::ast::TypeNode::Bool => {
                                    // Bool is stored as i32 in symbols
                                    self.context.i32_type().into()
                                }
                                crate::parser::ast::TypeNode::String => {
                                    self.context.ptr_type(AddressSpace::default()).into()
                                }
                                crate::parser::ast::TypeNode::Array(_) => {
                                    self.context.ptr_type(AddressSpace::default()).into()
                                }
                                crate::parser::ast::TypeNode::Map(_, _) => {
                                    self.context.ptr_type(AddressSpace::default()).into()
                                }
                                crate::parser::ast::TypeNode::Tuple(_) => {
                                    self.context.ptr_type(AddressSpace::default()).into()
                                }
                                crate::parser::ast::TypeNode::TypeRef(_) => {
                                    // Nested enum - stored as struct pointer
                                    self.context.ptr_type(AddressSpace::default()).into()
                                }
                                crate::parser::ast::TypeNode::Enum(_, _) => {
                                    self.context.ptr_type(AddressSpace::default()).into()
                                }
                                _ => self.context.i32_type().into(),
                            }
                        } else {
                            // No payload type info - default to i32
                            self.context.i32_type().into()
                        };
                        var_types.insert(name.clone(), payload_llvm_type);

                        // If this is a tuple payload, also track the element types for TupleGet
                        if let Some(crate::parser::ast::TypeNode::Tuple(types)) = payload_type {
                            let elem_types: Vec<BasicTypeEnum<'ctx>> = types
                                .iter()
                                .map(|t| match t {
                                    crate::parser::ast::TypeNode::Int => {
                                        self.context.i32_type().into()
                                    }
                                    crate::parser::ast::TypeNode::Float => {
                                        self.context.f64_type().into()
                                    }
                                    crate::parser::ast::TypeNode::Bool => {
                                        // Bool is stored as i32 in symbols
                                        self.context.i32_type().into()
                                    }
                                    crate::parser::ast::TypeNode::String => {
                                        self.context.ptr_type(AddressSpace::default()).into()
                                    }
                                    crate::parser::ast::TypeNode::Array(_) => {
                                        self.context.ptr_type(AddressSpace::default()).into()
                                    }
                                    crate::parser::ast::TypeNode::Map(_, _) => {
                                        self.context.ptr_type(AddressSpace::default()).into()
                                    }
                                    _ => self.context.i32_type().into(),
                                })
                                .collect();
                            tuple_element_types.insert(name.clone(), elem_types);
                        }
                    }
                    // TryPropagate - the result is a Result struct { i32, ptr }
                    // The unwrapped value type depends on the Ok type of the Result
                    crate::mir::MirInstr::TryPropagate { name, result, .. } => {
                        // The TryPropagate instruction produces the unwrapped Ok value
                        // Try to trace back to the function that produced this result
                        if let Some(func_name) = call_sources.get(result) {
                            // Found the source function - check its return type
                            if let Some(return_type_str) = self.function_return_types.get(func_name)
                            {
                                // Strip Tuple() wrapper if present
                                let inner = if return_type_str.starts_with("Tuple(")
                                    && return_type_str.ends_with(')')
                                {
                                    &return_type_str[6..return_type_str.len() - 1]
                                } else {
                                    return_type_str.as_str()
                                };
                                let types = crate::codegen::core::helpers::parse_tuple_types(inner);
                                if types.len() > 1 {
                                    // Multi-value return - TryPropagate produces a tuple pointer
                                    // Track this so TupleExtract can resolve element types
                                    tuple_sources.insert(name.clone(), func_name.clone());
                                    var_types.insert(
                                        name.clone(),
                                        self.context.ptr_type(AddressSpace::default()).into(),
                                    );
                                } else if types.len() == 1 {
                                    // Single value return
                                    let type_str = &types[0];
                                    if type_str.contains("Str") || type_str.contains("String") {
                                        var_types.insert(
                                            name.clone(),
                                            self.context.ptr_type(AddressSpace::default()).into(),
                                        );
                                    } else if type_str.contains("Float") {
                                        var_types
                                            .insert(name.clone(), self.context.f64_type().into());
                                    } else if type_str.contains("Array") || type_str.contains("Map")
                                    {
                                        var_types.insert(
                                            name.clone(),
                                            self.context.ptr_type(AddressSpace::default()).into(),
                                        );
                                    } else {
                                        var_types
                                            .insert(name.clone(), self.context.i32_type().into());
                                    }
                                } else {
                                    var_types.insert(name.clone(), self.context.i32_type().into());
                                }
                            } else {
                                var_types.insert(name.clone(), self.context.i32_type().into());
                            }
                        } else if let Some(result_type) = var_types.get(result) {
                            // If result is a Result struct, the unwrapped value could be various types
                            // For now, propagate the type or default to i32
                            var_types.insert(name.clone(), *result_type);
                        } else {
                            // Default to i32 for simple Ok values
                            var_types.insert(name.clone(), self.context.i32_type().into());
                        }
                    }
                    // Skip Assign in first pass
                    crate::mir::MirInstr::Assign { .. } => {}
                    _ => {}
                }
            }
        }

        // Second pass: process Assign instructions to propagate types
        for block in &func.blocks {
            for instr in &block.instrs {
                if let crate::mir::MirInstr::Assign { name, value, .. } = instr {
                    // If the source value has a known type, propagate it to the destination
                    if let Some(&source_type) = var_types.get(value) {
                        var_types.insert(name.clone(), source_type);
                    }
                }
            }
        }

        // Allocate stack space for cross-block variables with correct types
        // Store cross-block vars in CodeGen so Assign doesn't remove their symbols
        for var in &cross_block_vars {
            self.cross_block_vars.insert(var.clone());
        }

        for var in &cross_block_vars {
            if !self.symbols.contains_key(var) {
                // Skip function parameters - they are already initialized from incoming values
                if func.params.contains(var) {
                    continue;
                }

                // Skip variables marked as no-storage (e.g., tuple pointers from Result unwrapping)
                if self.no_storage_vars.contains(var) {
                    continue;
                }

                // Determine the correct type for this variable
                let var_type = var_types.get(var).copied().unwrap_or_else(|| {
                    // Index and end variables are always i32
                    if var.ends_with("__index")
                        || var.ends_with("_end")
                        || var == "i"
                        || var == "counter"
                    {
                        self.context.i32_type().into()
                    }
                    // Default heuristic: if name suggests array/map/string, use ptr, otherwise i32
                    else if var.ends_with("_array")
                        || var.ends_with("_map")
                        || var.ends_with("item_array")
                        || var.ends_with("_ptr")
                    {
                        self.context.ptr_type(AddressSpace::default()).into()
                    } else {
                        self.context.i32_type().into()
                    }
                });

                let alloca = self
                    .builder
                    .build_alloca(var_type, var)
                    .expect("Failed to allocate cross-block variable");

                // ALWAYS initialize loop variables immediately to prevent uninitialized value errors
                // Loop index variables (__index, _end) must start at 0
                // Other variables use appropriate defaults (0 for int, null for ptr)
                let default_val: BasicValueEnum = if var_type.is_pointer_type() {
                    self.context
                        .ptr_type(AddressSpace::default())
                        .const_null()
                        .into()
                } else if var_type.is_int_type() {
                    self.context.i32_type().const_int(0, false).into()
                } else {
                    self.context.i32_type().const_int(0, false).into()
                };

                self.builder.build_store(alloca, default_val).unwrap();

                self.symbols.insert(
                    var.clone(),
                    crate::codegen::Symbol {
                        ptr: alloca,
                        ty: var_type,
                    },
                );
            }
        }

        // After ALL allocations in entry block, jump to first MIR block
        if let Some(first_mir_block) = func.blocks.first() {
            if let Some(first_bb) = bb_map.get(&first_mir_block.label) {
                self.builder.build_unconditional_branch(*first_bb).unwrap();
            }
        }

        // Convert MIR block terminators to a unified structure for easier handling.
        // This simplifies codegen for control flow instructions.
        let _: Vec<CodegenBlock> = func
            .blocks
            .iter()
            // The map operation transforms the block structure to handle terminators uniformly.
            .map(|b| CodegenBlock {
                label: &b.label,
                instrs: &b.instrs,
                // Pattern match to extract the inner values from the Instruction enum.
                terminator: match &b.terminator {
                    Some(MirInstr::Return { values }) => Some(MirTerminator::Return {
                        values: values.clone(),
                    }),
                    Some(MirInstr::Jump { label: target }) => Some(MirTerminator::Jump {
                        target: target.clone(),
                    }),
                    Some(MirInstr::CondJump {
                        cond,
                        then_block,
                        else_block,
                    }) => Some(MirTerminator::CondJump {
                        cond: cond.clone(),
                        then_block: then_block.clone(),
                        else_block: else_block.clone(),
                    }),
                    _ => None,
                },
            })
            .collect();

        // Generate instructions and terminators for all blocks.
        for block in &func.blocks {
            self.generate_block_with_loops(block, llvm_func, &bb_map);
        }

        llvm_func
    }

    /// Generate cleanup for all RC variables at function exit
    /// This ensures variables in conditional blocks are properly cleaned up
    fn generate_function_exit_cleanup(&mut self) {
        // OWNERSHIP MODEL:
        // Only cleanup heap objects that:
        // 1. Have a symbol (alloca in entry block) - these are guaranteed valid across all blocks
        // 2. Are not loop-local (loop vars are cleaned in loop exit)
        // 3. Are not compiler temporaries (temps like %123, data_ptr45, etc.)
        //
        // We NEVER cleanup:
        // - Temporary GEP results (only exist in one block)
        // - Values in temp_values that have no corresponding symbol
        // - Block-local SSA values

        // Helper to check if a name is a compiler temporary
        let is_compiler_temp = |name: &str| -> bool {
            name.starts_with('%')
                || name.starts_with("data_ptr")
                || name.starts_with("heap_")
                || name.starts_with("rc_")
                || name.starts_with("concat_")
                || name.starts_with("temp_")
                || name.contains("_ptr")
                || name.contains("_header")
                || name.contains("_val")
                || name.contains("elem_")
                || name.contains("pair_")
                || name.contains("key_")
                || name.contains("loaded")
        };

        // Collect heap strings from symbols (user variables only)
        let mut heap_strings: Vec<String> = self
            .symbols
            .keys()
            .filter(|name| {
                !self.loop_local_vars.contains(*name)
                    && !is_compiler_temp(name)
                    && self.heap_strings.contains(*name)
            })
            .cloned()
            .collect();
        heap_strings.reverse();

        // Collect heap arrays from symbols
        let mut heap_arrays: Vec<String> = self
            .symbols
            .keys()
            .filter(|name| {
                !self.loop_local_vars.contains(*name)
                    && !is_compiler_temp(name)
                    && self.heap_arrays.contains(*name)
            })
            .cloned()
            .collect();
        heap_arrays.reverse();

        // Collect heap maps from symbols
        let mut heap_maps: Vec<String> = self
            .symbols
            .keys()
            .filter(|name| {
                !self.loop_local_vars.contains(*name)
                    && !is_compiler_temp(name)
                    && self.heap_maps.contains(*name)
            })
            .cloned()
            .collect();
        heap_maps.reverse();

        // Cleanup composite strings in arrays/maps
        // SAFETY: Only cleanup strings from composites whose parent variable is a valid symbol
        for (var_name, str_ptrs) in &self.composite_string_ptrs {
            // Skip if parent variable doesn't exist in symbols
            if !self.symbols.contains_key(var_name) {
                continue;
            }
            // Skip loop-local and compiler temps
            if self.loop_local_vars.contains(var_name) || is_compiler_temp(var_name) {
                continue;
            }

            // Safe to cleanup: parent is a valid symbol
            for str_ptr in str_ptrs {
                let data_ptr = str_ptr.into_pointer_value();
                let rc_header = unsafe {
                    self.builder.build_in_bounds_gep(
                        self.context.i8_type(),
                        data_ptr,
                        &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                        "rc_header",
                    )
                }
                .unwrap();

                let decref = self.decref_fn.unwrap();
                self.builder
                    .build_call(decref, &[rc_header.into()], "")
                    .unwrap();
            }
        }

        // Cleanup arrays (RC-managed)
        for var_name in heap_arrays {
            // Skip aliases to struct fields; the struct owns their lifetime and pushes may have updated the field pointer
            if self.struct_field_sources.contains_key(&var_name) {
                continue;
            }
            self.emit_decref(&var_name);
        }

        // Cleanup sliced arrays (malloc'd without RC header - use free directly)
        let mut slice_arrays: Vec<String> = self
            .slice_arrays
            .iter()
            .filter(|name| {
                !self.loop_local_vars.contains(*name)
                    && !is_compiler_temp(name)
                    && self.symbols.contains_key(*name)
            })
            .cloned()
            .collect();
        slice_arrays.reverse();

        let free_fn = self.module.get_function("free").unwrap_or_else(|| {
            let fn_type = self.context.void_type().fn_type(
                &[self
                    .context
                    .ptr_type(inkwell::AddressSpace::default())
                    .into()],
                false,
            );
            self.module.add_function("free", fn_type, None)
        });

        for var_name in slice_arrays {
            if let Some(sym) = self.symbols.get(&var_name) {
                let ptr_val = self
                    .builder
                    .build_load(
                        self.context.ptr_type(inkwell::AddressSpace::default()),
                        sym.ptr,
                        &format!("{}_load", var_name),
                    )
                    .unwrap();

                // Check if pointer is not null before freeing
                let null_ptr = self
                    .context
                    .ptr_type(inkwell::AddressSpace::default())
                    .const_null();
                let is_not_null = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        ptr_val.into_pointer_value(),
                        null_ptr,
                        "is_not_null",
                    )
                    .unwrap();

                // Only free if not null
                let current_fn = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();
                let free_bb = self.context.append_basic_block(current_fn, "free_slice");
                let skip_bb = self.context.append_basic_block(current_fn, "skip_free");
                let merge_bb = self.context.append_basic_block(current_fn, "merge_free");

                self.builder
                    .build_conditional_branch(is_not_null, free_bb, skip_bb)
                    .unwrap();

                self.builder.position_at_end(free_bb);
                self.builder
                    .build_call(free_fn, &[ptr_val.into()], &format!("{}_free", var_name))
                    .unwrap();
                self.builder.build_unconditional_branch(merge_bb).unwrap();

                self.builder.position_at_end(skip_bb);
                self.builder.build_unconditional_branch(merge_bb).unwrap();

                self.builder.position_at_end(merge_bb);
            }
        }

        // Cleanup maps
        for var_name in heap_maps {
            self.emit_decref(&var_name);
        }

        // Cleanup strings
        for var_name in heap_strings {
            self.emit_decref(&var_name);
        }

        // NOTE: We intentionally DO NOT cleanup temporary heap strings that only exist in temp_values.
        // These are SSA values created inside conditional blocks (if/else, match arms) and using them
        // at function exit would cause LLVM verification errors ("Instruction does not dominate all uses!")
        // because the cleanup code is in a block that doesn't dominate the conditional blocks where
        // the temps were created.
        //
        // Heap strings that need cleanup MUST be stored into symbols (allocas) at creation time.
        // Only symbols can be safely accessed at function exit because allocas are in the entry block.
        //
        // The proper fix for leaking temps would be to:
        // 1. Store concat results into allocas when inside conditional contexts, OR
        // 2. Track which temps were created in which blocks and only cleanup those that dominate exit
        //
        // For now, we accept the minor leak for intermediate concat results in conditionals.
        // The strings will be freed when the process exits.
    }

    /// Generates LLVM IR for a single MIR block.
    /// This method:
    /// - Handles loop markers and loop setup instructions.
    /// - Processes instructions for assignments, operations, and memory management.
    /// - Manages reference counting for heap-allocated variables.
    /// - Handles block terminators and loop continuation logic.
    /// It ensures correct control flow and memory cleanup for loops and regular blocks.
    pub fn generate_block(
        &mut self,
        block: &MirBlock,
        func: FunctionValue<'ctx>,
        bb_map: &HashMap<String, inkwell::basic_block::BasicBlock<'ctx>>,
    ) {
        let bb = bb_map.get(&block.label).unwrap();
        self.builder.position_at_end(*bb);

        // Track if this is a loop body and what kind
        let mut loop_increment_var: Option<String> = None;
        let mut loop_cond_block: Option<String> = None;
        let mut is_range_loop = false;
        let mut is_array_loop = false;
        let mut is_map_loop = false;
        let mut array_name: Option<String> = None;
        let mut index_var: Option<String> = None;
        let mut item_var: Option<String> = None;
        let mut map_name: Option<String> = None;
        let mut key_var: Option<String> = None;
        let mut val_var: Option<String> = None;

        // Scan for loop markers to identify loop context and variables.
        for instr in &block.instrs {
            match instr {
                MirInstr::LoopBodyMarker {
                    var, cond_block, ..
                } => {
                    is_range_loop = true;
                    loop_increment_var = Some(var.clone());
                    loop_cond_block = Some(cond_block.clone());
                }
                MirInstr::ArrayLoopMarker {
                    array,
                    index,
                    item,
                    cond_block,
                } => {
                    is_array_loop = true;
                    array_name = Some(array.clone());
                    index_var = Some(index.clone());
                    item_var = Some(item.clone());
                    loop_cond_block = Some(cond_block.clone());
                }
                MirInstr::MapLoopMarker {
                    map,
                    index,
                    key,
                    value,
                    cond_block,
                } => {
                    is_map_loop = true;
                    map_name = Some(map.clone());
                    index_var = Some(index.clone());
                    key_var = Some(key.clone());
                    val_var = Some(value.clone());
                    loop_cond_block = Some(cond_block.clone());
                }
                _ => {}
            }
        }

        // Process instructions in the block.
        for instr in &block.instrs {
            match instr {
                // Skip marker instructions (used only for loop context).
                MirInstr::LoopBodyMarker { .. }
                | MirInstr::ArrayLoopMarker { .. }
                | MirInstr::MapLoopMarker { .. } => continue,

                // Handle loop setup instructions.
                MirInstr::ForRange { .. }
                | MirInstr::ForArray { .. }
                | MirInstr::ForMap { .. }
                | MirInstr::ForInfinite { .. } => {
                    self.generate_for_loop(instr, bb_map);
                }

                // Handle break/continue with cleanup of loop variables.
                MirInstr::Break { .. } | MirInstr::Continue { .. } => {
                    // Clean up loop variables before jumping.
                    if is_array_loop && item_var.is_some() {
                        let item = item_var.as_ref().unwrap();
                        if self.heap_strings.contains(item) {
                            self.emit_decref(item);
                        }
                    }
                    if is_map_loop {
                        if let Some(key) = &key_var {
                            if self.heap_strings.contains(key) {
                                self.emit_decref(key);
                            }
                        }
                        if let Some(val) = &val_var {
                            if self.heap_strings.contains(val) {
                                self.emit_decref(val);
                            }
                        }
                    }

                    self.generate_for_loop(instr, bb_map);
                    return; // These terminate the block
                }

                // Handle array element and map pair loading.
                MirInstr::LoadArrayElement { .. } | MirInstr::LoadMapPair { .. } => {
                    self.generate_instr(instr);
                }

                // Regular instructions (assignments, operations, etc.).
                _ => {
                    self.generate_instr(instr);
                }
            }
        }

        // After all instructions, handle loop continuation logic.
        if is_range_loop {
            // Range loop: increment variable and jump to condition.
            if let (Some(var), Some(cond_block)) = (loop_increment_var, loop_cond_block) {
                let cond_bb = bb_map.get(&cond_block).expect("Condition block not found");
                self.generate_loop_increment_and_branch(&var, *cond_bb);
                return; // Don't process terminator
            }
        } else if is_array_loop {
            // Array loop: decref item (if string), increment index, jump to condition.
            if let (Some(item), Some(index), Some(cond_block)) =
                (item_var, index_var, loop_cond_block)
            {
                // Decref the item if it's a string (was incref'd when loaded).
                if self.heap_strings.contains(&item) {
                    self.emit_decref(&item);
                }

                // Increment index.
                if let Some(symbol) = self.symbols.get(&index) {
                    let current = self
                        .builder
                        .build_load(self.context.i32_type(), symbol.ptr, "current_idx")
                        .unwrap()
                        .into_int_value();

                    let one = self.context.i32_type().const_int(1, false);
                    let incremented = self
                        .builder
                        .build_int_add(current, one, "incremented_idx")
                        .unwrap();

                    self.builder.build_store(symbol.ptr, incremented).unwrap();
                }

                // Jump back to condition.
                let cond_bb = bb_map.get(&cond_block).expect("Condition block not found");
                self.builder.build_unconditional_branch(*cond_bb).unwrap();
                return;
            }
        } else if is_map_loop {
            // Map loop: decref key and value (if strings), increment index, jump to condition.
            if let (Some(key), Some(val), Some(index), Some(cond_block)) =
                (key_var, val_var, index_var, loop_cond_block)
            {
                // Decref key if string.
                if self.heap_strings.contains(&key) {
                    self.emit_decref(&key);
                }

                // Decref value if string.
                if self.heap_strings.contains(&val) {
                    self.emit_decref(&val);
                }

                // Increment index.
                if let Some(symbol) = self.symbols.get(&index) {
                    let current = self
                        .builder
                        .build_load(self.context.i32_type(), symbol.ptr, "current_idx")
                        .unwrap()
                        .into_int_value();

                    let one = self.context.i32_type().const_int(1, false);
                    let incremented = self
                        .builder
                        .build_int_add(current, one, "incremented_idx")
                        .unwrap();

                    self.builder.build_store(symbol.ptr, incremented).unwrap();
                }

                // Jump back to condition.
                let cond_bb = bb_map.get(&cond_block).expect("Condition block not found");
                self.builder.build_unconditional_branch(*cond_bb).unwrap();
                return;
            }
        }

        // Handle regular block terminator (return, jump, cond jump).
        if let Some(instr) = &block.terminator {
            let term = match instr {
                MirInstr::Return { values } => MirTerminator::Return {
                    values: values.clone(),
                },
                MirInstr::Jump { label: target } => MirTerminator::Jump {
                    target: target.clone(),
                },
                MirInstr::CondJump {
                    cond,
                    then_block,
                    else_block,
                } => MirTerminator::CondJump {
                    cond: cond.clone(),
                    then_block: then_block.clone(),
                    else_block: else_block.clone(),
                },
                _ => return,
            };
            self.generate_terminator(&term, func, bb_map);
        }
    }

    /// Generates the final instruction of a basic block (the control flow transfer).
    /// Generates LLVM IR for block terminators (return, jump, conditional jump).
    /// This method:
    /// - Handles memory cleanup for heap-allocated variables (strings, arrays, maps) on return.
    /// - Emits LLVM IR for unconditional and conditional branches.
    /// - Ensures correct control flow and resource management at block boundaries.
    pub fn generate_terminator(
        &mut self,
        term: &MirTerminator,
        func: FunctionValue<'ctx>,
        _bb_map: &HashMap<String, inkwell::basic_block::BasicBlock<'ctx>>,
    ) {
        match term {
            // Handles function return.
            // In functions.rs, MirTerminator::Return
            MirTerminator::Return { values } => {
                // SAFE COMPOSITE CLEANUP: Only decref strings from valid symbols
                // We must NOT try to decref temporary GEP results that were created in other blocks

                // 1. Cleanup composite strings - but ONLY for variables that exist in symbols
                for (var_name, str_ptrs) in &self.composite_string_ptrs {
                    // CRITICAL SAFETY CHECKS:
                    // - Variable must exist in symbols (has an alloca in entry block)
                    // - Variable must not be loop-local (loop vars are cleaned elsewhere)
                    // - Variable must not be a compiler temporary
                    if !self.symbols.contains_key(var_name) {
                        continue;
                    }
                    if self.loop_local_vars.contains(var_name) {
                        continue;
                    }
                    if var_name.starts_with('%')
                        || var_name.starts_with("data_ptr")
                        || var_name.starts_with("temp_")
                        || var_name.contains("_ptr")
                        || var_name.contains("elem_")
                    {
                        continue;
                    }

                    // Now it's safe to decref the string pointers in this composite
                    for str_ptr in str_ptrs {
                        let data_ptr = str_ptr.into_pointer_value();
                        let rc_header = unsafe {
                            self.builder.build_in_bounds_gep(
                                self.context.i8_type(),
                                data_ptr,
                                &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                                "rc_header",
                            )
                        }
                        .unwrap();

                        let decref = self.decref_fn.unwrap();
                        self.builder
                            .build_call(decref, &[rc_header.into()], "")
                            .unwrap();
                    }
                }

                // 2. Cleanup composite strings tracked via composite_strings map

                // Determine what value is being returned (if any) to exclude it from cleanup
                let return_value_name = if !values.is_empty() {
                    Some(values[0].as_str())
                } else {
                    None
                };

                // Helper to detect compiler temps (do not decref them here)
                let is_compiler_temp = |name: &str| {
                    name.starts_with('%')
                        || name.starts_with("data_ptr")
                        || name.starts_with("temp_")
                        || name.contains("_ptr")
                        || name.contains("elem_")
                };

                // 2. Free arrays (exclude return value)
                let mut heap_array_vars: Vec<String> = self
                    .symbols
                    .keys()
                    .filter(|name| {
                        self.heap_arrays.contains(*name)
                            && return_value_name.map_or(true, |ret| ret != *name)
                            && !self.struct_field_sources.contains_key(*name)
                            && !is_compiler_temp(name)
                    })
                    .cloned()
                    .collect();
                heap_array_vars.reverse();

                for var_name in heap_array_vars {
                    if self.struct_field_sources.contains_key(&var_name) {
                        continue;
                    }
                    if is_compiler_temp(&var_name) {
                        continue;
                    }
                    self.emit_decref(&var_name);
                }

                // 3. Free maps (exclude return value)
                let mut heap_map_vars: Vec<String> = self
                    .symbols
                    .keys()
                    .filter(|name| {
                        self.heap_maps.contains(*name)
                            && return_value_name.map_or(true, |ret| ret != *name)
                    })
                    .cloned()
                    .collect();
                heap_map_vars.reverse();

                for var_name in heap_map_vars {
                    self.emit_decref(&var_name);
                }

                // 4. Free simple strings from symbols (exclude return value)
                let mut heap_str_vars: Vec<String> = self
                    .symbols
                    .keys()
                    .filter(|name| {
                        self.heap_strings.contains(*name)
                            && return_value_name.map_or(true, |ret| ret != *name)
                    })
                    .cloned()
                    .collect();
                heap_str_vars.reverse();

                for var_name in heap_str_vars {
                    self.emit_decref(&var_name);
                }

                // NOTE: We intentionally DO NOT cleanup temporary heap strings that only exist in temp_values.
                // These are SSA values created inside conditional blocks (if/else, match arms) and using them
                // at function exit would cause LLVM verification errors ("Instruction does not dominate all uses!")
                // because the cleanup code is in a block that doesn't dominate the conditional blocks where
                // the temps were created.
                //
                // Heap strings that need cleanup MUST be stored into symbols (allocas) at creation time.
                // Only symbols can be safely accessed at function exit because allocas are in the entry block.

                if values.is_empty() {
                    // Check if this is the main function - it must return i32 0
                    let fn_name = func.get_name().to_str().unwrap();
                    if fn_name == "main" {
                        let zero = self.context.i32_type().const_int(0, false);
                        self.builder.build_return(Some(&zero)).unwrap();
                    } else if self.current_error_type.is_some() {
                        // Void return with error type - wrap in Ok Result with null pointer
                        let ptr_type = self.context.ptr_type(AddressSpace::default());
                        let result_struct_type = self
                            .context
                            .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false);

                        let ok_tag = self.context.i32_type().const_int(0, false); // 0 = Ok
                        let null_ptr = ptr_type.const_null();

                        let result_alloca = self
                            .builder
                            .build_alloca(result_struct_type, "void_ok_result")
                            .unwrap();

                        // Set tag field
                        let tag_ptr = self
                            .builder
                            .build_struct_gep(result_struct_type, result_alloca, 0, "tag_ptr")
                            .unwrap();
                        self.builder.build_store(tag_ptr, ok_tag).unwrap();

                        // Set value field (null for Void)
                        let value_ptr = self
                            .builder
                            .build_struct_gep(result_struct_type, result_alloca, 1, "value_ptr")
                            .unwrap();
                        self.builder.build_store(value_ptr, null_ptr).unwrap();

                        // Load and return the Result struct
                        let result_val = self
                            .builder
                            .build_load(result_struct_type, result_alloca, "void_ok_result_val")
                            .unwrap();
                        self.builder.build_return(Some(&result_val)).unwrap();
                    } else {
                        // Void return - no value, no error type
                        self.builder.build_return(None).unwrap();
                    }
                } else if values.len() == 1 {
                    // Single return value
                    let return_value_name = &values[0];

                    // Track if this function returns a heap-allocated value
                    let fn_name = func.get_name().to_str().unwrap();
                    let is_heap_return = self.heap_strings.contains(return_value_name)
                        || self.heap_arrays.contains(return_value_name)
                        || self.heap_maps.contains(return_value_name);

                    if is_heap_return {
                        self.functions_returning_heap.insert(fn_name.to_string());
                    }

                    let mut val = self.resolve_value(return_value_name);

                    // Check if we need to convert JSON.parse result (heap string) to expected return type
                    let fn_name = func.get_name().to_str().unwrap();
                    if self.heap_strings.contains(return_value_name) && val.is_pointer_value() {
                        if let Some(return_type_str) = self.function_return_types.get(fn_name) {
                            let json_str_ptr = val.into_pointer_value();
                            if return_type_str == "Int" {
                                // Convert JSON string to Int using atoi
                                let atoi_fn =
                                    self.module.get_function("atoi").unwrap_or_else(|| {
                                        let fn_type = self.context.i32_type().fn_type(
                                            &[self
                                                .context
                                                .ptr_type(AddressSpace::default())
                                                .into()],
                                            false,
                                        );
                                        self.module.add_function("atoi", fn_type, None)
                                    });
                                val = self
                                    .builder
                                    .build_call(atoi_fn, &[json_str_ptr.into()], "parsed_int")
                                    .unwrap()
                                    .try_as_basic_value()
                                    .left()
                                    .unwrap();
                            } else if return_type_str == "Float" {
                                // Convert JSON string to Float using atof
                                let atof_fn =
                                    self.module.get_function("atof").unwrap_or_else(|| {
                                        let fn_type = self.context.f64_type().fn_type(
                                            &[self
                                                .context
                                                .ptr_type(AddressSpace::default())
                                                .into()],
                                            false,
                                        );
                                        self.module.add_function("atof", fn_type, None)
                                    });
                                val = self
                                    .builder
                                    .build_call(atof_fn, &[json_str_ptr.into()], "parsed_float")
                                    .unwrap()
                                    .try_as_basic_value()
                                    .left()
                                    .unwrap();
                            } else if return_type_str == "Bool" {
                                // Convert JSON string to Bool - check first char
                                let first_char = self
                                    .builder
                                    .build_load(self.context.i8_type(), json_str_ptr, "first_char")
                                    .unwrap()
                                    .into_int_value();
                                let is_t = self
                                    .builder
                                    .build_int_compare(
                                        inkwell::IntPredicate::EQ,
                                        first_char,
                                        self.context.i8_type().const_int(b't' as u64, false),
                                        "is_t",
                                    )
                                    .unwrap();
                                let is_1 = self
                                    .builder
                                    .build_int_compare(
                                        inkwell::IntPredicate::EQ,
                                        first_char,
                                        self.context.i8_type().const_int(b'1' as u64, false),
                                        "is_1",
                                    )
                                    .unwrap();
                                let bool_i1 = self.builder.build_or(is_t, is_1, "bool_i1").unwrap();
                                let bool_val = self
                                    .builder
                                    .build_int_z_extend(
                                        bool_i1,
                                        self.context.i32_type(),
                                        "bool_val",
                                    )
                                    .unwrap();
                                val = bool_val.into();
                            }
                        }
                    }

                    // Check if we're returning a locally-created heap value (not a parameter)
                    // Only locally-created heap values have RC headers that we should increment
                    let is_local_heap_value = self.heap_strings.contains(return_value_name)
                        || self.heap_arrays.contains(return_value_name)
                        || self.heap_maps.contains(return_value_name);

                    if is_local_heap_value {
                        let fn_name = func.get_name().to_str().unwrap();
                        self.functions_returning_heap.insert(fn_name.to_string());

                        // Only call incref if this is a locally-created heap value with an RC header
                        // CRITICAL: Must check for null BEFORE computing ptr - 8, otherwise we get
                        // an invalid address like 0xFFFFFFF8 which passes the null check in __incref
                        if val.is_pointer_value() {
                            let ptr = val.into_pointer_value();

                            // Add null check before computing RC header offset
                            let current_fn = self
                                .builder
                                .get_insert_block()
                                .unwrap()
                                .get_parent()
                                .unwrap();
                            let do_incref_block = self
                                .context
                                .append_basic_block(current_fn, "return_do_incref");
                            let skip_incref_block = self
                                .context
                                .append_basic_block(current_fn, "return_skip_incref");

                            let is_null = self
                                .builder
                                .build_is_null(ptr, "return_ptr_is_null")
                                .unwrap();
                            self.builder
                                .build_conditional_branch(
                                    is_null,
                                    skip_incref_block,
                                    do_incref_block,
                                )
                                .unwrap();

                            // Do incref block - pointer is not null
                            self.builder.position_at_end(do_incref_block);
                            let rc_header = unsafe {
                                self.builder.build_in_bounds_gep(
                                    self.context.i8_type(),
                                    ptr,
                                    &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                                    "return_rc_header",
                                )
                            }
                            .unwrap();

                            let incref_fn = self.incref_fn.unwrap();
                            self.builder
                                .build_call(incref_fn, &[rc_header.into()], "")
                                .unwrap();
                            self.builder
                                .build_unconditional_branch(skip_incref_block)
                                .unwrap();

                            // Skip incref block - continue to return
                            self.builder.position_at_end(skip_incref_block);
                        }
                    } else {
                        // If returning an RC-typed parameter (not a locally-created value),
                        // just mark the function as returning heap but DON'T incref
                        // (parameters are owned by caller, not this function)
                        let is_rc_param =
                            self.current_function_params
                                .iter()
                                .any(|(param_name, param_type)| {
                                    if param_name == return_value_name {
                                        if let Some(type_str) = param_type {
                                            return type_str.contains("String")
                                                || type_str.contains("Str")
                                                || type_str.contains("Array")
                                                || type_str.contains("Map");
                                        }
                                    }
                                    false
                                });

                        if is_rc_param {
                            let fn_name = func.get_name().to_str().unwrap();
                            self.functions_returning_heap.insert(fn_name.to_string());
                        }
                    }

                    // If this function returns a Result type (has error_type), wrap the value in Ok Result
                    // UNLESS the value is already a Result struct (from ResultOk/ResultErr)
                    if self.current_error_type.is_some() {
                        // Check if the value is already a Result struct
                        let is_already_result = val.is_struct_value()
                            && self.result_types.contains_key(return_value_name)
                            || self
                                .variable_types
                                .get(return_value_name)
                                .map_or(false, |t| t == "Result");

                        if is_already_result {
                            // Value is already a Result struct from ResultOk/ResultErr - return as-is
                            self.builder.build_return(Some(&val)).unwrap();
                        } else {
                            // Value needs to be wrapped in Result struct
                            // Create Result struct: { i32 tag = 0 (Ok), ptr value }
                            let ptr_type = self.context.ptr_type(AddressSpace::default());
                            let result_struct_type = self.context.struct_type(
                                &[self.context.i32_type().into(), ptr_type.into()],
                                false,
                            );

                            // Convert the value to a pointer for storage in Result
                            let value_as_ptr = if val.is_pointer_value() {
                                // Already a pointer (String, Array, Map, Struct)
                                val.into_pointer_value()
                            } else if val.is_int_value() {
                                // Convert i32 to pointer
                                let int_val = val.into_int_value();
                                let i64_val = self
                                    .builder
                                    .build_int_z_extend(
                                        int_val,
                                        self.context.i64_type(),
                                        "i32_to_i64",
                                    )
                                    .unwrap();
                                self.builder
                                    .build_int_to_ptr(
                                        i64_val,
                                        self.context.ptr_type(AddressSpace::default()),
                                        "i32_to_ptr",
                                    )
                                    .unwrap()
                            } else if val.is_float_value() {
                                // Convert f64 to pointer
                                let float_val = val.into_float_value();
                                let alloca = self
                                    .builder
                                    .build_alloca(self.context.f64_type(), "f64_tmp")
                                    .unwrap();
                                self.builder.build_store(alloca, float_val).unwrap();
                                let i64_ptr = self
                                    .builder
                                    .build_pointer_cast(
                                        alloca,
                                        self.context.ptr_type(AddressSpace::default()),
                                        "f64_ptr_cast",
                                    )
                                    .unwrap();
                                let i64_val = self
                                    .builder
                                    .build_load(self.context.i64_type(), i64_ptr, "f64_as_i64")
                                    .unwrap()
                                    .into_int_value();
                                self.builder
                                    .build_int_to_ptr(
                                        i64_val,
                                        self.context.ptr_type(AddressSpace::default()),
                                        "f64_to_ptr",
                                    )
                                    .unwrap()
                            } else {
                                // Fallback: null pointer
                                ptr_type.const_null()
                            };

                            // Build the Ok Result struct
                            let ok_tag = self.context.i32_type().const_int(0, false); // 0 = Ok
                            let result_alloca = self
                                .builder
                                .build_alloca(result_struct_type, "ok_result")
                                .unwrap();

                            // Set tag field
                            let tag_ptr = self
                                .builder
                                .build_struct_gep(result_struct_type, result_alloca, 0, "tag_ptr")
                                .unwrap();
                            self.builder.build_store(tag_ptr, ok_tag).unwrap();

                            // Set value field
                            let value_ptr = self
                                .builder
                                .build_struct_gep(result_struct_type, result_alloca, 1, "value_ptr")
                                .unwrap();
                            self.builder.build_store(value_ptr, value_as_ptr).unwrap();

                            // Load and return the Result struct
                            let result_val = self
                                .builder
                                .build_load(result_struct_type, result_alloca, "ok_result_val")
                                .unwrap();
                            self.builder.build_return(Some(&result_val)).unwrap();
                        }
                    } else {
                        // Normal return (no error type)
                        // Check if we need to convert Bool type (i32 -> i1)
                        let return_type = func.get_type().get_return_type();
                        let final_val = if let Some(ret_type) = return_type {
                            if ret_type.is_int_type() && val.is_int_value() {
                                let ret_int_type = ret_type.into_int_type();
                                let val_int = val.into_int_value();
                                // Convert i32 to i1 if needed (Bool return type)
                                if ret_int_type.get_bit_width() == 1
                                    && val_int.get_type().get_bit_width() == 32
                                {
                                    let i1_val = self
                                        .builder
                                        .build_int_truncate(val_int, ret_int_type, "bool_trunc")
                                        .unwrap();
                                    i1_val.into()
                                } else {
                                    val
                                }
                            } else {
                                val
                            }
                        } else {
                            val
                        };
                        self.builder.build_return(Some(&final_val)).unwrap();
                    }
                } else {
                    // Multiple return values - build a struct
                    let return_values: Vec<BasicValueEnum> =
                        values.iter().map(|v| self.resolve_value(v)).collect();

                    let types: Vec<BasicTypeEnum> =
                        return_values.iter().map(|v| v.get_type()).collect();

                    let struct_type = self.context.struct_type(&types, false);
                    let struct_alloca =
                        self.builder.build_alloca(struct_type, "ret_tuple").unwrap();

                    for (i, val) in return_values.iter().enumerate() {
                        let field_ptr = self
                            .builder
                            .build_struct_gep(
                                struct_type,
                                struct_alloca,
                                i as u32,
                                &format!("ret_field_{}", i),
                            )
                            .unwrap();
                        self.builder.build_store(field_ptr, *val).unwrap();
                    }

                    let tuple_val = self
                        .builder
                        .build_load(struct_type, struct_alloca, "ret_tuple_val")
                        .unwrap();

                    // If this function returns a Result type (has error_type), wrap the tuple in Ok Result
                    if self.current_error_type.is_some() {
                        // Create Result struct: { i32 tag = 0 (Ok), ptr value }
                        let ptr_type = self.context.ptr_type(AddressSpace::default());
                        let result_struct_type = self
                            .context
                            .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false);

                        // Allocate space for the tuple on heap and store pointer in Result
                        let tuple_heap = self
                            .builder
                            .build_alloca(struct_type, "tuple_heap")
                            .unwrap();
                        self.builder.build_store(tuple_heap, tuple_val).unwrap();

                        let tuple_ptr = self
                            .builder
                            .build_pointer_cast(tuple_heap, ptr_type, "tuple_as_ptr")
                            .unwrap();

                        // Build the Ok Result struct
                        let ok_tag = self.context.i32_type().const_int(0, false); // 0 = Ok
                        let result_alloca = self
                            .builder
                            .build_alloca(result_struct_type, "tuple_ok_result")
                            .unwrap();

                        // Set tag field
                        let tag_ptr = self
                            .builder
                            .build_struct_gep(result_struct_type, result_alloca, 0, "tag_ptr")
                            .unwrap();
                        self.builder.build_store(tag_ptr, ok_tag).unwrap();

                        // Set value field
                        let value_ptr = self
                            .builder
                            .build_struct_gep(result_struct_type, result_alloca, 1, "value_ptr")
                            .unwrap();
                        self.builder.build_store(value_ptr, tuple_ptr).unwrap();

                        // Load and return the Result struct
                        let result_val = self
                            .builder
                            .build_load(result_struct_type, result_alloca, "tuple_ok_result_val")
                            .unwrap();
                        self.builder.build_return(Some(&result_val)).unwrap();
                    } else {
                        // Normal multi-value return (no error type)
                        self.builder.build_return(Some(&tuple_val)).unwrap();
                    }
                }
            }
            // Handles unconditional jump (goto).
            MirTerminator::Jump { target } => {
                if let Some(target_bb) = _bb_map.get(target) {
                    self.builder.build_unconditional_branch(*target_bb).unwrap();
                }
            }
            // Handles conditional jump (if/else).
            MirTerminator::CondJump {
                cond,
                then_block,
                else_block,
            } => {
                let cond_val = self.resolve_value(cond);

                // Check if condition is already i1 (from comparison) or i32 (from bool variable)
                let cond_i1 = if cond_val.is_int_value() {
                    let int_val = cond_val.into_int_value();
                    let int_type = int_val.get_type();

                    if int_type.get_bit_width() == 1 {
                        // Already i1, use directly
                        int_val
                    } else {
                        // i32 boolean, convert to i1
                        self.builder
                            .build_int_compare(
                                inkwell::IntPredicate::NE,
                                int_val,
                                self.context.i32_type().const_zero(),
                                "cond_i1",
                            )
                            .unwrap()
                    }
                } else {
                    debug_assert!(false, "Condition value is not an integer type");
                    self.context.i32_type().const_zero()
                };

                // Emit conditional branch
                if let (Some(then_bb), Some(else_bb)) =
                    (_bb_map.get(then_block), _bb_map.get(else_block))
                {
                    self.builder
                        .build_conditional_branch(cond_i1, *then_bb, *else_bb)
                        .unwrap();
                }
            }
        }
    }

    /// Generates LLVM IR for a block that is part of a loop structure.
    /// This method:
    /// - Handles loop body markers and identifies loop variables and blocks.
    /// - Processes instructions, including loop setup and element loading with reference counting.
    /// - Manages incrementing loop variables and jumping back to condition blocks.
    /// - Handles block terminators for control flow.
    /// It ensures correct loop semantics and memory management for complex loop constructs.
    pub fn generate_block_with_loops(
        &mut self,
        block: &MirBlock,
        func: FunctionValue<'ctx>,
        bb_map: &HashMap<String, inkwell::basic_block::BasicBlock<'ctx>>,
    ) {
        let bb = bb_map.get(&block.label).unwrap();
        self.builder.position_at_end(*bb);

        // Track if this is a loop body block
        let mut is_loop_body = false;
        let mut loop_var = None;
        let mut loop_cond_bb = None;
        let mut loop_increment_bb = None;

        // Check if any instruction marks this as a loop body
        for instr in &block.instrs {
            match instr {
                MirInstr::LoopBodyMarker {
                    var,
                    cond_block,
                    increment_block,
                } => {
                    is_loop_body = true;
                    loop_var = Some(var.clone());
                    loop_cond_bb = bb_map.get(cond_block).copied();
                    loop_increment_bb = bb_map.get(increment_block).copied();
                }
                _ => {}
            }
        }

        // Generate all instructions in the block.
        for instr in &block.instrs {
            // Check for loop-related instructions and handle accordingly.
            match instr {
                MirInstr::ForRange { .. }
                | MirInstr::ForArray { .. }
                | MirInstr::ForMap { .. }
                | MirInstr::ForInfinite { .. } => {
                    self.generate_for_loop(instr, bb_map);
                    // For loops handle their own control flow and position the builder
                    // at a different block (body_bb), so we must return to avoid
                    // adding a terminator to the wrong block
                    return;
                }

                MirInstr::LoadArrayElement { dest, array, index } => {
                    // Load array element with RC handling.
                    let array_ptr = self.resolve_value(array).into_pointer_value();
                    let index_val = self.resolve_value(index).into_int_value();

                    // Determine if elements are strings for RC logic.
                    let is_string =
                        self.heap_strings.contains(array) || self.array_contains_strings(array);

                    let elem_type = self.get_array_element_type(array);
                    let elem_val =
                        self.load_array_element_with_rc(array_ptr, index_val, elem_type, is_string);

                    // Store in destination variable.
                    if let Some(symbol) = self.symbols.get(dest) {
                        self.builder.build_store(symbol.ptr, elem_val).unwrap();
                    }
                }

                MirInstr::LoadMapPair {
                    key_dest,
                    val_dest,
                    map,
                    index,
                } => {
                    let map_ptr = self.resolve_value(map).into_pointer_value();
                    let index_val = self.resolve_value(index).into_int_value();

                    let (key_is_string, val_is_string) = self.map_contains_strings(map);
                    let pair_type = self.get_map_pair_type(map);

                    let (key_val, val_val) = self.load_map_pair_with_rc(
                        map_ptr,
                        index_val,
                        pair_type,
                        key_is_string,
                        val_is_string,
                    );

                    // Store key and value.
                    if let Some(symbol) = self.symbols.get(key_dest) {
                        self.builder.build_store(symbol.ptr, key_val).unwrap();
                    }
                    if let Some(symbol) = self.symbols.get(val_dest) {
                        self.builder.build_store(symbol.ptr, val_val).unwrap();
                    }
                }

                MirInstr::Break { .. } | MirInstr::Continue { .. } => {
                    self.generate_for_loop(instr, bb_map);
                    return; // These terminate the block
                }

                _ => {
                    self.generate_instr(instr);
                }
            }
        }

        // If this is a loop body, handle increment and loop back.
        if is_loop_body {
            if let (Some(var), Some(_), Some(cond_bb)) = (loop_var, loop_increment_bb, loop_cond_bb)
            {
                // Generate increment: var = var + 1
                if let Some(symbol) = self.symbols.get(&var) {
                    let current = self
                        .builder
                        .build_load(self.context.i32_type(), symbol.ptr, "current")
                        .unwrap()
                        .into_int_value();

                    let one = self.context.i32_type().const_int(1, false);
                    let incremented = self
                        .builder
                        .build_int_add(current, one, "incremented")
                        .unwrap();

                    self.builder.build_store(symbol.ptr, incremented).unwrap();
                }

                // Jump back to condition block for next loop iteration.
                self.builder.build_unconditional_branch(cond_bb).unwrap();
                return;
            }
        }

        // Handle terminator if present (return, jump, cond jump).
        if let Some(instr) = &block.terminator {
            let term = match instr {
                MirInstr::Return { values } => crate::mir::mir::MirTerminator::Return {
                    values: values.clone(),
                },
                MirInstr::Jump { label: target } => crate::mir::mir::MirTerminator::Jump {
                    target: target.clone(),
                },
                MirInstr::CondJump {
                    cond,
                    then_block,
                    else_block,
                } => crate::mir::mir::MirTerminator::CondJump {
                    cond: cond.clone(),
                    then_block: then_block.clone(),
                    else_block: else_block.clone(),
                },
                _ => return,
            };
            self.generate_terminator(&term, func, bb_map);
        } else {
            // No terminator - add appropriate return based on function type
            let fn_name = func.get_name().to_str().unwrap();

            // Check if main function needs special handling
            if fn_name == "main" {
                // Main function must return i32 0
                self.generate_function_exit_cleanup();
                let zero = self.context.i32_type().const_int(0, false);
                self.builder.build_return(Some(&zero)).unwrap();
            } else {
                // For non-main functions, check return type
                let fn_type = func.get_type();
                let return_type = fn_type.get_return_type();

                if return_type.is_none() {
                    // Void function - add cleanup and return void
                    self.generate_function_exit_cleanup();
                    self.builder.build_return(None).unwrap();
                } else {
                    // Non-void function without terminator - unreachable
                    self.generate_function_exit_cleanup();
                    self.builder.build_unreachable().unwrap();
                }
            }
        }
    }

    /// Enhanced cleanup for loop exit with RC
    /// Cleans up heap-allocated loop variables when exiting a loop.
    /// This method:
    /// - Decrements reference counts for strings, arrays, and maps.
    /// - Handles cleanup of composite string pointers in arrays and maps.
    /// - Ensures proper memory management and avoids leaks in loop constructs.
    pub fn generate_loop_cleanup(&mut self, loop_vars: &[String]) {
        // When exiting a loop, clean up any heap-allocated loop variables.
        for var in loop_vars {
            if self.heap_strings.contains(var) {
                self.emit_decref(var);
            }
            if self.heap_arrays.contains(var) {
                // Clean up strings in array elements if needed.
                if let Some(str_ptrs) = self.composite_string_ptrs.get(var) {
                    for str_ptr in str_ptrs {
                        let data_ptr = str_ptr.into_pointer_value();
                        let rc_header = unsafe {
                            self.builder.build_in_bounds_gep(
                                self.context.i8_type(),
                                data_ptr,
                                &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                                "rc_header",
                            )
                        }
                        .unwrap();

                        let decref = self.decref_fn.unwrap();
                        self.builder
                            .build_call(decref, &[rc_header.into()], "")
                            .unwrap();
                    }
                }
                self.emit_decref(var);
            }
            if self.heap_maps.contains(var) {
                // Clean up strings in map if needed.
                if let Some(str_names) = self.composite_strings.get(var) {
                    for str_name in str_names {
                        if let Some(val) = self.temp_values.get(str_name) {
                            if val.is_pointer_value() {
                                let data_ptr = val.into_pointer_value();
                                let rc_header = unsafe {
                                    self.builder.build_in_bounds_gep(
                                        self.context.i8_type(),
                                        data_ptr,
                                        &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                                        "rc_header",
                                    )
                                }
                                .unwrap();

                                let decref = self.decref_fn.unwrap();
                                self.builder
                                    .build_call(decref, &[rc_header.into()], "")
                                    .unwrap();
                            }
                        }
                    }
                }
                self.emit_decref(var);
            }
        }
    }
}
