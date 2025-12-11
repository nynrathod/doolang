use crate::codegen::core::helpers::parse_tuple_types;
use crate::codegen::core::CodeGen;
use crate::mir::mir::{CodegenBlock, MirBlock, MirFunction, MirInstr, MirProgram, MirTerminator};
use inkwell::module::Linkage;
use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum};
use inkwell::values::{BasicValueEnum, FunctionValue};
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

        // Copy enum_table, enum_variant_order, and struct_table from MirProgram for type metadata access
        self.enum_table = program.enum_table.clone();
        self.enum_variant_order = program.enum_variant_order.clone();
        self.struct_table = program.struct_table.clone();

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
                // Store metadata for this struct type
                let metadata = crate::codegen::core::context::StructMetadata {
                    field_names: field_names.clone(),
                    field_types: field_types.clone(),
                };
                self.struct_metadata.insert(struct_name.clone(), metadata);

                // Create the canonical LLVM struct type
                let llvm_field_types: Vec<BasicTypeEnum> = field_types
                    .iter()
                    .map(|type_str| self.type_string_to_llvm_type(type_str))
                    .collect();

                let struct_type = self.context.struct_type(&llvm_field_types, false);
                self.canonical_struct_types
                    .insert(struct_name.clone(), struct_type);
            }
        }

        // WORKAROUND: Manually add FileError and FileMetadata struct metadata since imported structs
        // are not included in the MIR globals. This should be fixed by propagating
        // struct declarations from imported modules.
        if !self.struct_metadata.contains_key("FileError") {
            let metadata = crate::codegen::core::context::StructMetadata {
                field_names: vec!["Message".to_string()],
                field_types: vec!["Str".to_string()],
            };
            self.struct_metadata
                .insert("FileError".to_string(), metadata);

            // Also create the canonical LLVM struct type
            let llvm_field_types = vec![self
                .context
                .ptr_type(inkwell::AddressSpace::default())
                .into()];
            let struct_type = self.context.struct_type(&llvm_field_types, false);
            self.canonical_struct_types
                .insert("FileError".to_string(), struct_type);
        }

        if !self.struct_metadata.contains_key("FileMetadata") {
            let metadata = crate::codegen::core::context::StructMetadata {
                field_names: vec![
                    "isFile".to_string(),
                    "isDir".to_string(),
                    "isSymlink".to_string(),
                    "size".to_string(),
                    "readonly".to_string(),
                    "created".to_string(),
                    "modified".to_string(),
                    "accessed".to_string(),
                ],
                field_types: vec![
                    "Bool".to_string(),
                    "Bool".to_string(),
                    "Bool".to_string(),
                    "Int".to_string(),
                    "Bool".to_string(),
                    "Int".to_string(),
                    "Int".to_string(),
                    "Int".to_string(),
                ],
            };
            self.struct_metadata
                .insert("FileMetadata".to_string(), metadata);

            // Also create the canonical LLVM struct type
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
        }

        // WORKAROUND: Manually add HTTP stdlib structs since imported structs
        // are not included in the MIR globals. This should be fixed by propagating
        // struct declarations from imported modules.

        // HttpError struct
        if !self.struct_metadata.contains_key("HttpError") {
            let metadata = crate::codegen::core::context::StructMetadata {
                field_names: vec!["Status".to_string(), "Message".to_string()],
                field_types: vec!["Int".to_string(), "Str".to_string()],
            };
            self.struct_metadata
                .insert("HttpError".to_string(), metadata);
            let llvm_field_types = vec![
                self.context.i32_type().into(),
                self.context
                    .ptr_type(inkwell::AddressSpace::default())
                    .into(),
            ];
            let struct_type = self.context.struct_type(&llvm_field_types, false);
            self.canonical_struct_types
                .insert("HttpError".to_string(), struct_type);
        }

        // Request struct
        if !self.struct_metadata.contains_key("Request") {
            let metadata = crate::codegen::core::context::StructMetadata {
                field_names: vec![
                    "Method".to_string(),
                    "Path".to_string(),
                    "Body".to_string(),
                    "ContentType".to_string(),
                ],
                field_types: vec![
                    "Str".to_string(),
                    "Str".to_string(),
                    "Str".to_string(),
                    "Str".to_string(),
                ],
            };
            self.struct_metadata.insert("Request".to_string(), metadata);
            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
            let llvm_field_types = vec![
                ptr_type.into(),
                ptr_type.into(),
                ptr_type.into(),
                ptr_type.into(),
            ];
            let struct_type = self.context.struct_type(&llvm_field_types, false);
            self.canonical_struct_types
                .insert("Request".to_string(), struct_type);
        }

        // Response struct
        if !self.struct_metadata.contains_key("Response") {
            let metadata = crate::codegen::core::context::StructMetadata {
                field_names: vec![
                    "Status".to_string(),
                    "Body".to_string(),
                    "ContentType".to_string(),
                ],
                field_types: vec!["Int".to_string(), "Str".to_string(), "Str".to_string()],
            };
            self.struct_metadata
                .insert("Response".to_string(), metadata);
            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
            let llvm_field_types = vec![
                self.context.i32_type().into(),
                ptr_type.into(),
                ptr_type.into(),
            ];
            let struct_type = self.context.struct_type(&llvm_field_types, false);
            self.canonical_struct_types
                .insert("Response".to_string(), struct_type);
        }

        // Server struct
        if !self.struct_metadata.contains_key("Server") {
            let metadata = crate::codegen::core::context::StructMetadata {
                field_names: vec!["Port".to_string(), "Host".to_string()],
                field_types: vec!["Int".to_string(), "Str".to_string()],
            };
            self.struct_metadata.insert("Server".to_string(), metadata);
            let llvm_field_types = vec![
                self.context.i32_type().into(),
                self.context
                    .ptr_type(inkwell::AddressSpace::default())
                    .into(),
            ];
            let struct_type = self.context.struct_type(&llvm_field_types, false);
            self.canonical_struct_types
                .insert("Server".to_string(), struct_type);
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

        // --- HTTP HANDLER DETECTION AND WRAPPERS ---
        // Check if Server struct is used (indicates HTTP application)
        let uses_http = program.struct_table.contains_key("Server")
            || program.functions.iter().any(|f| {
                f.blocks.iter().any(|b| {
                    b.instrs.iter().any(|i| {
                        if let crate::mir::mir::MirInstr::MethodCall { method, .. } = i {
                            ["post", "get", "put", "delete", "patch", "start", "group"]
                                .contains(&method.as_str())
                        } else {
                            false
                        }
                    })
                })
            });

        if uses_http {
            // Generate wrappers for all potential HTTP handlers
            self.generate_http_handler_wrappers(&program.functions);
        }

        // --- FUNCTION GENERATION ---
        // Generate LLVM IR for all user-defined functions and apply optimizations.
        for func in &program.functions {
            let llvm_func = self.generate_function(func);
            // Apply registered optimization passes (like O1, O2, O3) to the generated function.
            self.fpm.run_on(&llvm_func);
        }

        // --- MAIN ENTRY POINT ---
        // For non-main-entry files (imported modules), generate a default main if needed
        if !program.is_main_entry && self.module.get_function("main").is_none() {
            self.generate_default_main();
        }
    }

    /// Generate HTTP handler wrapper functions
    /// Wraps user handler functions to match FFI signature: fn(*mut DooRequest) -> *mut DooResult
    fn generate_http_handler_wrappers(&mut self, functions: &[MirFunction]) {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();

        // Declare global variable to store current DooRequest pointer for enum error handling
        // This allows enum error handling code to extract the request path
        if self
            .module
            .get_global("__doo_current_request_ptr")
            .is_none()
        {
            let global_request_ptr = self.module.add_global(
                ptr_type,
                Some(AddressSpace::default()),
                "__doo_current_request_ptr",
            );
            global_request_ptr.set_linkage(Linkage::Internal);
            global_request_ptr.set_initializer(&ptr_type.const_null());
        }

        // Declare FFI helper functions
        self.declare_http_ffi_helpers();

        for func in functions {
            // Skip FFI functions, main, and functions without return types
            if func.ffi_lib.is_some() || func.name == "main" || func.return_type.is_none() {
                continue;
            }

            // Only wrap functions that match HTTP handler/middleware signature patterns
            // HTTP handlers: 0-2 parameters, have return type
            // Middleware: 2 parameters (Request, Next)
            let param_count = func.params.len();
            if param_count > 2 {
                continue;
            }

            // Skip HTTP helper/utility functions by name pattern
            let handler_name = &func.name;
            if handler_name.ends_with("Response")
                || handler_name.starts_with("Status")
                || handler_name.contains("::")  // Skip methods (Type::method)
                || handler_name == "parseJson"
                || handler_name == "toJson"
                || handler_name.ends_with("Rfc7807")  // Skip RFC 7807 helper functions
                || handler_name.starts_with("ErrorRfc7807")
            // Skip RFC 7807 error builders
            {
                continue;
            }

            // Check if this is a middleware function (takes Request and Next)
            let is_middleware = if param_count == 2 {
                if let Some(param_types) = self.function_param_types.get(handler_name) {
                    param_types.len() == 2
                        && param_types[0] == "Request"
                        && param_types[1] == "Next"
                } else {
                    false
                }
            } else {
                false
            };

            if is_middleware {
                // Middleware functions need wrappers to convert to pointer
                self.http_middleware_to_register.push(handler_name.clone());

                let wrapper_name = format!("{}_http_wrapper", handler_name);

                // Check if wrapper already exists
                if self.module.get_function(&wrapper_name).is_some() {
                    continue;
                }

                // Get the original middleware function
                let original_middleware = match self.module.get_function(handler_name) {
                    Some(f) => f,
                    None => continue,
                };

                // Create wrapper function with FFI signature: fn(*mut Request, *mut Next) -> *mut DooResult
                let wrapper_fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                let wrapper_fn = self
                    .module
                    .add_function(&wrapper_name, wrapper_fn_type, None);

                let entry_block = self.context.append_basic_block(wrapper_fn, "entry");
                self.builder.position_at_end(entry_block);

                // Get the request and next parameters
                let request_param = wrapper_fn.get_nth_param(0).unwrap().into_pointer_value();
                let next_param = wrapper_fn.get_nth_param(1).unwrap().into_pointer_value();

                // Call the original middleware
                let middleware_result = self
                    .builder
                    .build_call(
                        original_middleware,
                        &[request_param.into(), next_param.into()],
                        "middleware_result",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();

                // Check if middleware has error type (returns Result) or not (returns Response)
                let has_error_type = func.error_type.is_some();

                let malloc_fn = self.module.get_function("malloc").unwrap();

                if has_error_type {
                    // Middleware returns Result struct { i32 tag, ptr value } by value
                    // Allocate space for Result struct on heap and copy it there
                    let result_size = self.context.i64_type().const_int(16, false); // sizeof(Result) = 16 bytes
                    let result_ptr = self
                        .builder
                        .build_call(malloc_fn, &[result_size.into()], "result_malloc")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    // Store the Result struct to the allocated memory
                    self.builder
                        .build_store(result_ptr, middleware_result)
                        .unwrap();

                    // Return the pointer
                    self.builder.build_return(Some(&result_ptr)).unwrap();
                } else {
                    // Middleware returns Response pointer (no error type)
                    // middleware_result is already a pointer to Response
                    // Just wrap it in Ok Result: { tag: 0, value: response_ptr }
                    let response_ptr = middleware_result.into_pointer_value();

                    // Create Result struct: { tag: 0, value: response_ptr }
                    let result_struct_type = self
                        .context
                        .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false);

                    let result_size = self.context.i64_type().const_int(16, false);
                    let result_ptr = self
                        .builder
                        .build_call(malloc_fn, &[result_size.into()], "result_malloc")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    // Set tag = 0 (Ok)
                    let tag_ptr = self
                        .builder
                        .build_struct_gep(result_struct_type, result_ptr, 0, "tag_ptr")
                        .unwrap();
                    self.builder
                        .build_store(tag_ptr, self.context.i32_type().const_int(0, false))
                        .unwrap();

                    // Set value = response_ptr (already a pointer, not a struct)
                    let value_ptr = self
                        .builder
                        .build_struct_gep(result_struct_type, result_ptr, 1, "value_ptr")
                        .unwrap();
                    self.builder.build_store(value_ptr, response_ptr).unwrap();

                    // Return the Result pointer
                    self.builder.build_return(Some(&result_ptr)).unwrap();
                }

                continue;
            }

            // Track this handler for registration
            self.http_handlers_to_register.push(handler_name.clone());

            let wrapper_name = format!("{}_http_wrapper", handler_name);

            // Check if wrapper already exists
            if self.module.get_function(&wrapper_name).is_some() {
                continue;
            }

            // Get the original handler function
            let original_handler = match self.module.get_function(handler_name) {
                Some(f) => f,
                None => continue,
            };

            // Create wrapper function with FFI signature: fn(*mut DooRequest) -> *mut DooResult
            let wrapper_fn_type = ptr_type.fn_type(&[ptr_type.into()], false);
            let wrapper_fn = self
                .module
                .add_function(&wrapper_name, wrapper_fn_type, None);

            let entry_block = self.context.append_basic_block(wrapper_fn, "entry");
            self.builder.position_at_end(entry_block);

            // Get the request parameter
            let request_param = wrapper_fn.get_nth_param(0).unwrap().into_pointer_value();

            // Store request pointer in global variable for enum error handling
            let global_request_ptr = self.module.get_global("__doo_current_request_ptr").unwrap();
            self.builder
                .build_store(global_request_ptr.as_pointer_value(), request_param)
                .unwrap();

            // Call the original handler based on its signature
            let handler_result = if original_handler.count_params() == 0 {
                // Handler takes no parameters: fn() -> ReturnType
                self.builder
                    .build_call(original_handler, &[], "handler_result")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
            } else if original_handler.count_params() == 1 {
                // Handler takes 1 parameter - could be path parameter (Int/Float/Bool) or body parameter (struct)
                let llvm_param_type = original_handler.get_type().get_param_types()[0];

                // Get the parameter type name from MIR
                let param_type_str =
                    if let Some(param_types) = self.function_param_types.get(handler_name) {
                        if !param_types.is_empty() {
                            param_types[0].clone()
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };

                // Check if this is Request type (inject Request object)
                let is_request = param_type_str == "Request";

                // Check if this is a primitive type (path parameter) or struct (body parameter)
                let is_primitive = param_type_str == "Int"
                    || param_type_str == "I32"
                    || param_type_str == "I64"
                    || param_type_str == "Float"
                    || param_type_str == "F32"
                    || param_type_str == "F64"
                    || param_type_str == "Bool"
                    || param_type_str == "Str";

                if is_request {
                    // Request injection - pass DooRequest pointer directly as-is
                    // The Request struct layout matches the first 4 fields of DooRequest:
                    // DooRequest: { method, path, body, content_type, params, query, headers }
                    // Request:    { Method, Path, Body, ContentType }
                    // Field access works because the memory layout is compatible
                    // Method calls (query/param/header) are FFI that receive the pointer
                    self.builder
                        .build_call(original_handler, &[request_param.into()], "handler_result")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                } else if is_primitive {
                    // Path/query parameter - extract from request params
                    // For now, we'll extract the first path parameter (common pattern: /users/:id)

                    // For path parameters, we use a convention: if handler has 1 param with name "id", extract ":id"
                    // Otherwise extract the first parameter name
                    let param_name_str = if let Some(param_name) = func.params.first() {
                        param_name.clone()
                    } else {
                        "id".to_string()
                    };

                    let param_name_ptr = self
                        .builder
                        .build_global_string_ptr(&param_name_str, "param_name")
                        .unwrap()
                        .as_pointer_value();

                    // Helpers to check last RFC7807 error set by FFI extraction
                    let last_error_status_fn =
                        if let Some(f) = self.module.get_function("doohttp_last_error_status") {
                            f
                        } else {
                            let fn_type = i32_type.fn_type(&[], false);
                            self.module
                                .add_function("doohttp_last_error_status", fn_type, None)
                        };
                    let last_error_json_fn =
                        if let Some(f) = self.module.get_function("doohttp_last_error_json") {
                            f
                        } else {
                            let fn_type = ptr_type.fn_type(&[], false);
                            self.module
                                .add_function("doohttp_last_error_json", fn_type, None)
                        };

                    // Extract parameter based on type
                    let param_value: inkwell::values::BasicValueEnum = if param_type_str == "Str" {
                        // For string parameters, use doo_http_req_param
                        let extract_str_fn = if let Some(f) =
                            self.module.get_function("doo_http_req_param")
                        {
                            f
                        } else {
                            // fn doo_http_req_param(request: *const DooRequest, key: *const c_char) -> *const c_char
                            let extract_fn_type =
                                ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                            self.module
                                .add_function("doo_http_req_param", extract_fn_type, None)
                        };

                        let param_value_basic = self
                            .builder
                            .build_call(
                                extract_str_fn,
                                &[request_param.into(), param_name_ptr.into()],
                                "param_value",
                            )
                            .unwrap()
                            .try_as_basic_value()
                            .left()
                            .unwrap();

                        // Check for RFC7807 error from FFI
                        let status_val = self
                            .builder
                            .build_call(last_error_status_fn, &[], "param_err_status")
                            .unwrap()
                            .try_as_basic_value()
                            .left()
                            .unwrap()
                            .into_int_value();
                        let has_error = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::NE,
                                status_val,
                                i32_type.const_int(0, false),
                                "has_param_error",
                            )
                            .unwrap();
                        let current_fn = self
                            .builder
                            .get_insert_block()
                            .unwrap()
                            .get_parent()
                            .unwrap();
                        let err_block = self
                            .context
                            .append_basic_block(current_fn, "param_error_block");
                        let ok_block = self
                            .context
                            .append_basic_block(current_fn, "param_ok_block");
                        self.builder
                            .build_conditional_branch(has_error, err_block, ok_block)
                            .unwrap();

                        // Error block: return DooResult error
                        self.builder.position_at_end(err_block);
                        let err_json_ptr = self
                            .builder
                            .build_call(last_error_json_fn, &[], "param_error_json")
                            .unwrap()
                            .try_as_basic_value()
                            .left()
                            .unwrap()
                            .into_pointer_value();
                        let http_error_type = self
                            .context
                            .struct_type(&[i32_type.into(), ptr_type.into()], false);
                        let malloc_fn = self.module.get_function("malloc").unwrap();
                        let http_error_ptr = self
                            .builder
                            .build_call(
                                malloc_fn,
                                &[i64_type.const_int(16, false).into()],
                                "param_http_error_malloc",
                            )
                            .unwrap()
                            .try_as_basic_value()
                            .left()
                            .unwrap()
                            .into_pointer_value();
                        let status_ptr = self
                            .builder
                            .build_struct_gep(http_error_type, http_error_ptr, 0, "status_ptr")
                            .unwrap();
                        self.builder.build_store(status_ptr, status_val).unwrap();
                        let msg_ptr = self
                            .builder
                            .build_struct_gep(http_error_type, http_error_ptr, 1, "msg_ptr")
                            .unwrap();
                        self.builder.build_store(msg_ptr, err_json_ptr).unwrap();

                        let result_type = self
                            .context
                            .struct_type(&[i32_type.into(), ptr_type.into()], false);
                        let result_ptr = self
                            .builder
                            .build_call(
                                malloc_fn,
                                &[i64_type.const_int(16, false).into()],
                                "param_result_malloc",
                            )
                            .unwrap()
                            .try_as_basic_value()
                            .left()
                            .unwrap()
                            .into_pointer_value();
                        let tag_ptr = self
                            .builder
                            .build_struct_gep(result_type, result_ptr, 0, "tag_ptr")
                            .unwrap();
                        self.builder
                            .build_store(tag_ptr, i32_type.const_int(1, false))
                            .unwrap();
                        let val_ptr = self
                            .builder
                            .build_struct_gep(result_type, result_ptr, 1, "val_ptr")
                            .unwrap();
                        self.builder.build_store(val_ptr, http_error_ptr).unwrap();
                        self.builder.build_return(Some(&result_ptr)).unwrap();

                        // Continue OK block
                        self.builder.position_at_end(ok_block);

                        param_value_basic
                    } else {
                        // For numeric types, use doohttp_extract_param_int
                        let extract_fn = if let Some(f) =
                            self.module.get_function("doohttp_extract_param_int")
                        {
                            f
                        } else {
                            // fn doohttp_extract_param_int(request: *const DooRequest, param_name: *const c_char) -> i64
                            let extract_fn_type =
                                i64_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                            self.module.add_function(
                                "doohttp_extract_param_int",
                                extract_fn_type,
                                None,
                            )
                        };

                        // Extract parameter as i64 (covers Int, can be cast for other types)
                        let param_value_i64 = self
                            .builder
                            .build_call(
                                extract_fn,
                                &[request_param.into(), param_name_ptr.into()],
                                "param_value",
                            )
                            .unwrap()
                            .try_as_basic_value()
                            .left()
                            .unwrap()
                            .into_int_value();

                        // Check FFI error flag
                        let status_val = self
                            .builder
                            .build_call(last_error_status_fn, &[], "param_err_status_int")
                            .unwrap()
                            .try_as_basic_value()
                            .left()
                            .unwrap()
                            .into_int_value();
                        let has_error = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::NE,
                                status_val,
                                i32_type.const_int(0, false),
                                "has_param_error_int",
                            )
                            .unwrap();
                        let current_fn = self
                            .builder
                            .get_insert_block()
                            .unwrap()
                            .get_parent()
                            .unwrap();
                        let err_block = self
                            .context
                            .append_basic_block(current_fn, "param_error_block_int");
                        let ok_block = self
                            .context
                            .append_basic_block(current_fn, "param_ok_block_int");
                        self.builder
                            .build_conditional_branch(has_error, err_block, ok_block)
                            .unwrap();

                        self.builder.position_at_end(err_block);
                        let err_json_ptr = self
                            .builder
                            .build_call(last_error_json_fn, &[], "param_error_json_int")
                            .unwrap()
                            .try_as_basic_value()
                            .left()
                            .unwrap()
                            .into_pointer_value();
                        let http_error_type = self
                            .context
                            .struct_type(&[i32_type.into(), ptr_type.into()], false);
                        let malloc_fn = self.module.get_function("malloc").unwrap();
                        let http_error_ptr = self
                            .builder
                            .build_call(
                                malloc_fn,
                                &[i64_type.const_int(16, false).into()],
                                "param_http_error_malloc_int",
                            )
                            .unwrap()
                            .try_as_basic_value()
                            .left()
                            .unwrap()
                            .into_pointer_value();
                        let status_ptr = self
                            .builder
                            .build_struct_gep(http_error_type, http_error_ptr, 0, "status_ptr_int")
                            .unwrap();
                        self.builder.build_store(status_ptr, status_val).unwrap();
                        let msg_ptr = self
                            .builder
                            .build_struct_gep(http_error_type, http_error_ptr, 1, "msg_ptr_int")
                            .unwrap();
                        self.builder.build_store(msg_ptr, err_json_ptr).unwrap();

                        let result_type = self
                            .context
                            .struct_type(&[i32_type.into(), ptr_type.into()], false);
                        let result_ptr = self
                            .builder
                            .build_call(
                                malloc_fn,
                                &[i64_type.const_int(16, false).into()],
                                "param_result_malloc_int",
                            )
                            .unwrap()
                            .try_as_basic_value()
                            .left()
                            .unwrap()
                            .into_pointer_value();
                        let tag_ptr = self
                            .builder
                            .build_struct_gep(result_type, result_ptr, 0, "tag_ptr_int")
                            .unwrap();
                        self.builder
                            .build_store(tag_ptr, i32_type.const_int(1, false))
                            .unwrap();
                        let val_ptr = self
                            .builder
                            .build_struct_gep(result_type, result_ptr, 1, "val_ptr_int")
                            .unwrap();
                        self.builder.build_store(val_ptr, http_error_ptr).unwrap();
                        self.builder.build_return(Some(&result_ptr)).unwrap();

                        self.builder.position_at_end(ok_block);

                        // Convert to the expected type
                        if param_type_str == "Int" || param_type_str == "I32" {
                            // Truncate i64 to i32
                            self.builder
                                .build_int_truncate(param_value_i64, i32_type, "param_i32")
                                .unwrap()
                                .into()
                        } else if param_type_str == "I64" {
                            param_value_i64.into()
                        } else if param_type_str == "Float" || param_type_str == "F32" {
                            // Convert i64 to f32
                            self.builder
                                .build_signed_int_to_float(
                                    param_value_i64,
                                    self.context.f32_type(),
                                    "param_f32",
                                )
                                .unwrap()
                                .into()
                        } else if param_type_str == "F64" {
                            // Convert i64 to f64
                            self.builder
                                .build_signed_int_to_float(
                                    param_value_i64,
                                    self.context.f64_type(),
                                    "param_f64",
                                )
                                .unwrap()
                                .into()
                        } else if param_type_str == "Bool" {
                            // Convert i64 to i1 (bool)
                            let bool_val = self
                                .builder
                                .build_int_compare(
                                    inkwell::IntPredicate::NE,
                                    param_value_i64,
                                    i64_type.const_int(0, false),
                                    "param_bool",
                                )
                                .unwrap();
                            // Convert i1 to i32 for Doo's bool representation
                            self.builder
                                .build_int_z_extend(bool_val, i32_type, "bool_i32")
                                .unwrap()
                                .into()
                        } else {
                            // Default: use as-is
                            param_value_i64.into()
                        }
                    };

                    self.builder
                        .build_call(original_handler, &[param_value.into()], "handler_result")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                } else {
                    // Struct parameter - could be query params or body params
                    // Convention: if struct name contains "Params" or "Query", treat as query params
                    // Otherwise, parse from request body
                    let is_query_param = param_type_str.contains("Params")
                        || param_type_str.contains("Query")
                        || param_type_str.contains("Search");

                    let malloc_fn = self.module.get_function("malloc").unwrap();
                    let struct_size = i64_type.const_int(128, false);
                    let struct_ptr = self
                        .builder
                        .build_call(malloc_fn, &[struct_size.into()], "param_struct_ptr")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    // Zero out the memory
                    let memset_fn = if let Some(f) = self.module.get_function("memset") {
                        f
                    } else {
                        let memset_type = ptr_type
                            .fn_type(&[ptr_type.into(), i32_type.into(), i64_type.into()], false);
                        self.module.add_function("memset", memset_type, None)
                    };
                    let zero = i32_type.const_int(0, false);
                    self.builder
                        .build_call(
                            memset_fn,
                            &[struct_ptr.into(), zero.into(), struct_size.into()],
                            "",
                        )
                        .unwrap();

                    if is_query_param {
                        // Parse query parameters from URL into struct
                        // Pass the entire request pointer to parse_query_into_struct
                        if !param_type_str.is_empty() {
                            self.parse_query_into_struct(
                                request_param,
                                struct_ptr,
                                &param_type_str,
                            );

                            // After parsing query, check for RFC7807 error set by FFI
                            let last_error_status_fn = if let Some(f) =
                                self.module.get_function("doohttp_last_error_status")
                            {
                                f
                            } else {
                                let fn_type = i32_type.fn_type(&[], false);
                                self.module
                                    .add_function("doohttp_last_error_status", fn_type, None)
                            };
                            let last_error_json_fn = if let Some(f) =
                                self.module.get_function("doohttp_last_error_json")
                            {
                                f
                            } else {
                                let fn_type = ptr_type.fn_type(&[], false);
                                self.module
                                    .add_function("doohttp_last_error_json", fn_type, None)
                            };

                            let status_val = self
                                .builder
                                .build_call(last_error_status_fn, &[], "query_err_status")
                                .unwrap()
                                .try_as_basic_value()
                                .left()
                                .unwrap()
                                .into_int_value();
                            let has_error = self
                                .builder
                                .build_int_compare(
                                    inkwell::IntPredicate::NE,
                                    status_val,
                                    i32_type.const_int(0, false),
                                    "has_query_error",
                                )
                                .unwrap();

                            let current_fn = self
                                .builder
                                .get_insert_block()
                                .unwrap()
                                .get_parent()
                                .unwrap();
                            let err_block = self
                                .context
                                .append_basic_block(current_fn, "query_parse_error");
                            let cont_block = self
                                .context
                                .append_basic_block(current_fn, "query_parse_ok");
                            self.builder
                                .build_conditional_branch(has_error, err_block, cont_block)
                                .unwrap();

                            // Error block: build DooResult error from last_error_json
                            self.builder.position_at_end(err_block);
                            let error_json_ptr = self
                                .builder
                                .build_call(last_error_json_fn, &[], "query_error_json")
                                .unwrap()
                                .try_as_basic_value()
                                .left()
                                .unwrap()
                                .into_pointer_value();

                            let http_error_type = self
                                .context
                                .struct_type(&[i32_type.into(), ptr_type.into()], false);
                            let malloc_fn = self.module.get_function("malloc").unwrap();
                            let http_error_ptr = self
                                .builder
                                .build_call(
                                    malloc_fn,
                                    &[i64_type.const_int(16, false).into()],
                                    "query_http_error_malloc",
                                )
                                .unwrap()
                                .try_as_basic_value()
                                .left()
                                .unwrap()
                                .into_pointer_value();

                            let status_ptr = self
                                .builder
                                .build_struct_gep(http_error_type, http_error_ptr, 0, "status_ptr")
                                .unwrap();
                            self.builder.build_store(status_ptr, status_val).unwrap();

                            let message_ptr = self
                                .builder
                                .build_struct_gep(http_error_type, http_error_ptr, 1, "msg_ptr")
                                .unwrap();
                            self.builder
                                .build_store(message_ptr, error_json_ptr)
                                .unwrap();

                            let result_type = self
                                .context
                                .struct_type(&[i32_type.into(), ptr_type.into()], false);
                            let result_ptr = self
                                .builder
                                .build_call(
                                    malloc_fn,
                                    &[i64_type.const_int(16, false).into()],
                                    "query_result_malloc",
                                )
                                .unwrap()
                                .try_as_basic_value()
                                .left()
                                .unwrap()
                                .into_pointer_value();
                            let tag_ptr = self
                                .builder
                                .build_struct_gep(result_type, result_ptr, 0, "tag_ptr")
                                .unwrap();
                            self.builder
                                .build_store(tag_ptr, i32_type.const_int(1, false))
                                .unwrap();
                            let val_ptr = self
                                .builder
                                .build_struct_gep(result_type, result_ptr, 1, "val_ptr")
                                .unwrap();
                            self.builder.build_store(val_ptr, http_error_ptr).unwrap();
                            self.builder.build_return(Some(&result_ptr)).unwrap();

                            // Continue block
                            self.builder.position_at_end(cont_block);
                        }
                    } else {
                        // Parse JSON body into struct
                        // Get the request body field from DooRequest
                        // DooRequest layout: { method, path, body, content_type, params, query, headers }
                        // body is at offset 16 (8 bytes for method ptr + 8 bytes for path ptr)
                        let body_field_ptr = unsafe {
                            self.builder
                                .build_gep(
                                    self.context.i8_type(),
                                    request_param,
                                    &[i32_type.const_int(16, false)],
                                    "body_field_ptr",
                                )
                                .unwrap()
                        };
                        let body_field_ptr_typed = self
                            .builder
                            .build_pointer_cast(
                                body_field_ptr,
                                ptr_type.ptr_type(AddressSpace::default()),
                                "body_field_typed",
                            )
                            .unwrap();
                        let body_str_ptr = self
                            .builder
                            .build_load(ptr_type, body_field_ptr_typed, "body_str")
                            .unwrap()
                            .into_pointer_value();

                        if !param_type_str.is_empty() {
                            // Parse JSON body into struct
                            self.parse_json_into_struct(body_str_ptr, struct_ptr, &param_type_str);
                        }
                    }

                    self.builder
                        .build_call(original_handler, &[struct_ptr.into()], "handler_result")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                }
            } else if original_handler.count_params() == 2 {
                // Handler takes 2 parameters: could be:
                // 1) fn(id: Int, data: SomeStruct) -> ReturnType (primitive path param + body struct)
                // 2) fn(path: PathStruct, data: BodyStruct) -> ReturnType (path struct + body struct)
                let llvm_param1_type = original_handler.get_type().get_param_types()[0];
                let llvm_param2_type = original_handler.get_type().get_param_types()[1];

                // Get parameter type names from MIR
                let param1_type_str =
                    if let Some(param_types) = self.function_param_types.get(handler_name) {
                        if !param_types.is_empty() {
                            param_types[0].clone()
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };

                let param2_type_str =
                    if let Some(param_types) = self.function_param_types.get(handler_name) {
                        if param_types.len() > 1 {
                            param_types[1].clone()
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };

                // Check if first parameter is primitive or struct
                let param1_is_primitive = param1_type_str == "Int"
                    || param1_type_str == "I32"
                    || param1_type_str == "I64"
                    || param1_type_str == "Float"
                    || param1_type_str == "F32"
                    || param1_type_str == "F64"
                    || param1_type_str == "Bool"
                    || param1_type_str == "Str";

                // First parameter - extract path parameter
                let param1_value: inkwell::values::BasicValueEnum = if param1_is_primitive {
                    // Extract as primitive
                    let extract_fn = if let Some(f) =
                        self.module.get_function("doohttp_extract_param_int")
                    {
                        f
                    } else {
                        let extract_fn_type =
                            i64_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                        self.module
                            .add_function("doohttp_extract_param_int", extract_fn_type, None)
                    };

                    let param1_name_str = if let Some(param_name) = func.params.first() {
                        param_name.clone()
                    } else {
                        "id".to_string()
                    };

                    let param1_name_ptr = self
                        .builder
                        .build_global_string_ptr(&param1_name_str, "param1_name")
                        .unwrap()
                        .as_pointer_value();

                    let param1_value_i64 = self
                        .builder
                        .build_call(
                            extract_fn,
                            &[request_param.into(), param1_name_ptr.into()],
                            "param1_value",
                        )
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_int_value();

                    // Convert to i32 if needed
                    if param1_type_str == "Int" || param1_type_str == "I32" {
                        self.builder
                            .build_int_truncate(param1_value_i64, i32_type, "param1_i32")
                            .unwrap()
                            .into()
                    } else {
                        param1_value_i64.into()
                    }
                } else {
                    // First parameter is a struct (path parameters)
                    let malloc_fn = self.module.get_function("malloc").unwrap();
                    let param1_struct_size = i64_type.const_int(128, false);
                    let param1_struct_ptr = self
                        .builder
                        .build_call(malloc_fn, &[param1_struct_size.into()], "param1_struct_ptr")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    let memset_fn = if let Some(f) = self.module.get_function("memset") {
                        f
                    } else {
                        let memset_type = ptr_type
                            .fn_type(&[ptr_type.into(), i32_type.into(), i64_type.into()], false);
                        self.module.add_function("memset", memset_type, None)
                    };
                    let zero = i32_type.const_int(0, false);
                    self.builder
                        .build_call(
                            memset_fn,
                            &[
                                param1_struct_ptr.into(),
                                zero.into(),
                                param1_struct_size.into(),
                            ],
                            "",
                        )
                        .unwrap();

                    // Parse path parameters into struct
                    if !param1_type_str.is_empty() {
                        self.parse_query_into_struct(
                            request_param,
                            param1_struct_ptr,
                            &param1_type_str,
                        );
                    }

                    param1_struct_ptr.into()
                };

                // Second parameter - parse JSON body into struct
                let malloc_fn = self.module.get_function("malloc").unwrap();
                let struct_size = i64_type.const_int(128, false);
                let struct_ptr = self
                    .builder
                    .build_call(malloc_fn, &[struct_size.into()], "param2_struct_ptr")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                let memset_fn = if let Some(f) = self.module.get_function("memset") {
                    f
                } else {
                    let memset_type = ptr_type
                        .fn_type(&[ptr_type.into(), i32_type.into(), i64_type.into()], false);
                    self.module.add_function("memset", memset_type, None)
                };
                let zero = i32_type.const_int(0, false);
                self.builder
                    .build_call(
                        memset_fn,
                        &[struct_ptr.into(), zero.into(), struct_size.into()],
                        "",
                    )
                    .unwrap();

                // Get request body
                let body_field_ptr = unsafe {
                    self.builder
                        .build_gep(
                            self.context.i8_type(),
                            request_param,
                            &[i32_type.const_int(16, false)],
                            "body_field_ptr",
                        )
                        .unwrap()
                };
                let body_field_ptr_typed = self
                    .builder
                    .build_pointer_cast(
                        body_field_ptr,
                        ptr_type.ptr_type(AddressSpace::default()),
                        "body_field_typed",
                    )
                    .unwrap();
                let body_str_ptr = self
                    .builder
                    .build_load(ptr_type, body_field_ptr_typed, "body_str")
                    .unwrap()
                    .into_pointer_value();

                if !param2_type_str.is_empty() {
                    self.parse_json_into_struct(body_str_ptr, struct_ptr, &param2_type_str);
                }

                self.builder
                    .build_call(
                        original_handler,
                        &[param1_value.into(), struct_ptr.into()],
                        "handler_result",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
            } else {
                // Should not reach here due to filter above
                continue;
            };

            // Wrap the result in DooResponse and DooResult
            let response_ptr = if let Some(result_value) = handler_result {
                self.wrap_handler_result_in_response(
                    result_value,
                    func.return_type.as_ref().unwrap(),
                    func.error_type.as_deref(),
                )
            } else {
                // Void return - create empty response
                self.create_empty_response()
            };

            // Wrap response in DooResult (success case: tag=0, value=response_ptr)
            let doo_result = self.wrap_response_in_result(response_ptr);
            self.builder.build_return(Some(&doo_result)).unwrap();
        }
    }

    fn declare_http_ffi_helpers(&mut self) {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();

        // Declare malloc
        if self.module.get_function("malloc").is_none() {
            let malloc_type = ptr_type.fn_type(&[i64_type.into()], false);
            self.module.add_function("malloc", malloc_type, None);
        }

        // Declare sprintf
        if self.module.get_function("sprintf").is_none() {
            let sprintf_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], true);
            self.module.add_function("sprintf", sprintf_type, None);
        }

        // Declare memcpy
        if self.module.get_function("memcpy").is_none() {
            let memcpy_type =
                ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into()], false);
            self.module.add_function("memcpy", memcpy_type, None);
        }

        // Declare memset
        if self.module.get_function("memset").is_none() {
            let memset_type =
                ptr_type.fn_type(&[ptr_type.into(), i32_type.into(), i64_type.into()], false);
            self.module.add_function("memset", memset_type, None);
        }

        // Declare doohttp_error_to_status: (error_type: *const i8, variant: *const i8) -> i32
        if self
            .module
            .get_function("doohttp_error_to_status")
            .is_none()
        {
            let error_to_status_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
            self.module
                .add_function("doohttp_error_to_status", error_to_status_type, None);
        }

        // Declare doohttp_error_message: (error_type: *const i8, variant: *const i8) -> *const i8
        if self.module.get_function("doohttp_error_message").is_none() {
            let error_message_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
            self.module
                .add_function("doohttp_error_message", error_message_type, None);
        }

        // Declare doohttp_error_rfc7807: (status: i32, detail: *const i8, instance: *const i8) -> *const i8
        if self.module.get_function("doohttp_error_rfc7807").is_none() {
            let rfc7807_type =
                ptr_type.fn_type(&[i32_type.into(), ptr_type.into(), ptr_type.into()], false);
            self.module
                .add_function("doohttp_error_rfc7807", rfc7807_type, None);
        }

        // Declare doohttp_get_request_path: (request: *const DooRequest) -> *const i8
        // Extracts the path field from DooRequest struct
        if self
            .module
            .get_function("doohttp_get_request_path")
            .is_none()
        {
            let get_path_type = ptr_type.fn_type(&[ptr_type.into()], false);
            self.module
                .add_function("doohttp_get_request_path", get_path_type, None);
        }

        // Declare doohttp_error_rfc7807_auto_instance: (status: i32, detail: *const i8) -> *const i8
        // This version automatically uses thread-local request path as instance
        if self
            .module
            .get_function("doohttp_error_rfc7807_auto_instance")
            .is_none()
        {
            let rfc7807_auto_type = ptr_type.fn_type(&[i32_type.into(), ptr_type.into()], false);
            self.module.add_function(
                "doohttp_error_rfc7807_auto_instance",
                rfc7807_auto_type,
                None,
            );
        }

        // Declare doohttp_error_rfc7807_with_method: (status: i32, detail: *const i8, instance: *const i8, method: *const i8) -> *const i8
        if self
            .module
            .get_function("doohttp_error_rfc7807_with_method")
            .is_none()
        {
            let rfc7807_method_type = ptr_type.fn_type(
                &[
                    i32_type.into(),
                    ptr_type.into(),
                    ptr_type.into(),
                    ptr_type.into(),
                ],
                false,
            );
            self.module.add_function(
                "doohttp_error_rfc7807_with_method",
                rfc7807_method_type,
                None,
            );
        }

        // Declare doohttp_error_rfc7807_validation: (detail: *const i8, instance: *const i8, fields_json: *const i8) -> *const i8
        if self
            .module
            .get_function("doohttp_error_rfc7807_validation")
            .is_none()
        {
            let rfc7807_validation_type =
                ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false);
            self.module.add_function(
                "doohttp_error_rfc7807_validation",
                rfc7807_validation_type,
                None,
            );
        }

        // Declare doohttp_error_rfc7807_method_not_allowed: (detail: *const i8, instance: *const i8, allowed_methods: *const i8) -> *const i8
        if self
            .module
            .get_function("doohttp_error_rfc7807_method_not_allowed")
            .is_none()
        {
            let rfc7807_method_not_allowed_type =
                ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false);
            self.module.add_function(
                "doohttp_error_rfc7807_method_not_allowed",
                rfc7807_method_not_allowed_type,
                None,
            );
        }
    }

    /// Wrap handler result in DooResponse struct
    fn wrap_handler_result_in_response(
        &mut self,
        result: inkwell::values::BasicValueEnum<'ctx>,
        return_type: &str,
        error_type: Option<&str>,
    ) -> inkwell::values::PointerValue<'ctx> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();

        // Check if this is a Result type (has error_type) - handle this FIRST
        if error_type.is_some() {
            // Result type: struct { i32 tag, ptr value }
            // tag = 0 means Ok, tag = 1 means Err

            let result_struct = result.into_struct_value();

            // Extract tag (first field)
            let tag = self
                .builder
                .build_extract_value(result_struct, 0, "result_tag")
                .unwrap()
                .into_int_value();

            // Extract value pointer (second field)
            let value_ptr = self
                .builder
                .build_extract_value(result_struct, 1, "result_value_ptr")
                .unwrap()
                .into_pointer_value();

            // Check if Ok (tag == 0) or Err (tag == 1)
            let is_ok = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::EQ,
                    tag,
                    i32_type.const_int(0, false),
                    "is_ok",
                )
                .unwrap();

            // Create two blocks: one for Ok, one for Err
            let current_fn = self
                .builder
                .get_insert_block()
                .unwrap()
                .get_parent()
                .unwrap();
            let ok_block = self.context.append_basic_block(current_fn, "result_ok");
            let err_block = self.context.append_basic_block(current_fn, "result_err");
            let merge_block = self.context.append_basic_block(current_fn, "result_merge");

            self.builder
                .build_conditional_branch(is_ok, ok_block, err_block)
                .unwrap();

            // OK case: handle based on return type
            self.builder.position_at_end(ok_block);
            let ok_response = if return_type == "Response" {
                // Response ! Error: value_ptr points to Response struct, extract it directly
                self.extract_response_struct_to_doo_response(value_ptr)
            } else {
                // Other types: serialize the value with 200 status
                let ok_value: inkwell::values::BasicValueEnum = value_ptr.into();
                self.wrap_value_in_response_with_status(ok_value, return_type, 200)
            };
            self.builder
                .build_unconditional_branch(merge_block)
                .unwrap();

            // ERR case: determine error status code and message based on error enum type
            self.builder.position_at_end(err_block);
            let (error_status, error_msg) =
                self.determine_error_status_and_message(error_type.unwrap(), value_ptr);
            let err_response = self.create_error_response_with_status(error_status, error_msg);

            // Get the actual block we ended up in after error handling
            let err_exit_block = self.builder.get_insert_block().unwrap();

            self.builder
                .build_unconditional_branch(merge_block)
                .unwrap();

            // Merge: phi node to select the correct response
            self.builder.position_at_end(merge_block);
            let phi = self.builder.build_phi(ptr_type, "response_ptr").unwrap();
            phi.add_incoming(&[(&ok_response, ok_block), (&err_response, err_exit_block)]);

            return phi.as_basic_value().into_pointer_value();
        }

        // Check if return type is Response struct - handle it directly (manual Response handling)
        if return_type == "Response" {
            // Response struct: { Status: Int, Body: Str, ContentType: Str }
            // Extract fields and create DooResponse
            let response_struct_ptr = result.into_pointer_value();
            return self.extract_response_struct_to_doo_response(response_struct_ptr);
        }

        // Non-Result type: wrap directly with 200 status
        self.wrap_value_in_response_with_status(result, return_type, 200)
    }

    /// Extract Response struct and convert to DooResponse
    fn extract_response_struct_to_doo_response(
        &mut self,
        response_ptr: inkwell::values::PointerValue<'ctx>,
    ) -> inkwell::values::PointerValue<'ctx> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let malloc_fn = self.module.get_function("malloc").unwrap();

        // Allocate DooResponse struct
        let response_size = i64_type.const_int(24, false);
        let doo_response_ptr = self
            .builder
            .build_call(malloc_fn, &[response_size.into()], "doo_response_ptr")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Get Response struct type
        let response_struct_type = *self.canonical_struct_types.get("Response").unwrap();

        // Extract Status field (Int - field 0)
        let status_field_ptr = unsafe {
            self.builder
                .build_struct_gep(response_struct_type, response_ptr, 0, "status_ptr")
                .unwrap()
        };
        let status_value = self
            .builder
            .build_load(i32_type, status_field_ptr, "status")
            .unwrap()
            .into_int_value();

        // Extract Body field (Str - field 1)
        let body_field_ptr = unsafe {
            self.builder
                .build_struct_gep(response_struct_type, response_ptr, 1, "body_ptr")
                .unwrap()
        };
        let body_value = self
            .builder
            .build_load(ptr_type, body_field_ptr, "body")
            .unwrap()
            .into_pointer_value();

        // Extract ContentType field (Str - field 2)
        let content_type_field_ptr = unsafe {
            self.builder
                .build_struct_gep(response_struct_type, response_ptr, 2, "content_type_ptr")
                .unwrap()
        };
        let content_type_value = self
            .builder
            .build_load(ptr_type, content_type_field_ptr, "content_type")
            .unwrap()
            .into_pointer_value();

        // Store into DooResponse
        // Status field
        let doo_status_ptr = self
            .builder
            .build_pointer_cast(
                doo_response_ptr,
                i32_type.ptr_type(AddressSpace::default()),
                "doo_status_ptr",
            )
            .unwrap();
        self.builder
            .build_store(doo_status_ptr, status_value)
            .unwrap();

        // Body field (offset 8)
        let doo_body_field_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    doo_response_ptr,
                    &[i32_type.const_int(8, false)],
                    "doo_body_ptr",
                )
                .unwrap()
        };
        let doo_body_ptr_typed = self
            .builder
            .build_pointer_cast(
                doo_body_field_ptr,
                ptr_type.ptr_type(AddressSpace::default()),
                "doo_body_typed",
            )
            .unwrap();
        self.builder
            .build_store(doo_body_ptr_typed, body_value)
            .unwrap();

        // ContentType field (offset 16)
        let doo_content_type_field_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    doo_response_ptr,
                    &[i32_type.const_int(16, false)],
                    "doo_content_type_ptr",
                )
                .unwrap()
        };
        let doo_content_type_ptr_typed = self
            .builder
            .build_pointer_cast(
                doo_content_type_field_ptr,
                ptr_type.ptr_type(AddressSpace::default()),
                "doo_content_type_typed",
            )
            .unwrap();
        self.builder
            .build_store(doo_content_type_ptr_typed, content_type_value)
            .unwrap();

        doo_response_ptr
    }

    /// Determine error status code based on error enum variant
    /// Returns (status_code, error_message_ptr) to avoid creating branches inside
    fn determine_error_status_and_message(
        &mut self,
        error_type: &str,
        error_value_ptr: inkwell::values::PointerValue<'ctx>,
    ) -> (
        inkwell::values::IntValue<'ctx>,
        inkwell::values::PointerValue<'ctx>,
    ) {
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Debug: Print error type being processed
        eprintln!("[CODEGEN DEBUG] Processing error type: {}", error_type);

        // Extract the enum variant tag (first field of the enum struct)
        let tag = self
            .builder
            .build_load(i32_type, error_value_ptr, "error_tag")
            .unwrap()
            .into_int_value();

        // Look up the variant name from enum metadata
        if let Some(enum_variants) = self.enum_variant_order.get(error_type) {
            eprintln!(
                "[CODEGEN DEBUG] Found {} variants for {}",
                enum_variants.len(),
                error_type
            );
            for (idx, (name, _)) in enum_variants.iter().enumerate() {
                eprintln!("[CODEGEN DEBUG]   Variant {}: {}", idx, name);
            }

            // Declare FFI functions
            let doohttp_error_to_status_fn =
                if let Some(f) = self.module.get_function("doohttp_error_to_status") {
                    f
                } else {
                    let fn_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                    self.module
                        .add_function("doohttp_error_to_status", fn_type, None)
                };
            let doohttp_error_message_fn =
                if let Some(f) = self.module.get_function("doohttp_error_message") {
                    f
                } else {
                    let fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                    self.module
                        .add_function("doohttp_error_message", fn_type, None)
                };

            let error_type_str = self
                .builder
                .build_global_string_ptr(error_type, "error_type_str")
                .unwrap()
                .as_pointer_value();

            // Create blocks for switch
            let current_fn = self
                .builder
                .get_insert_block()
                .unwrap()
                .get_parent()
                .unwrap();
            let default_block = self.context.append_basic_block(current_fn, "err_default");
            let merge_block = self.context.append_basic_block(current_fn, "err_merge");

            // Create case blocks and collect them
            let mut switch_cases = vec![];
            let mut case_info = vec![];

            for (idx, (variant_name, _)) in enum_variants.iter().enumerate() {
                let case_block = self
                    .context
                    .append_basic_block(current_fn, &format!("err_case_{}", variant_name));
                switch_cases.push((i32_type.const_int(idx as u64, false), case_block));
                case_info.push((case_block, variant_name.as_str()));
            }

            eprintln!(
                "[CODEGEN DEBUG] Building switch with {} cases",
                switch_cases.len()
            );

            // Build switch
            self.builder
                .build_switch(tag, default_block, &switch_cases)
                .unwrap();

            // Build case blocks
            let mut phi_status_vals = vec![];
            let mut phi_msg_vals = vec![];

            for (case_block, variant_name) in case_info {
                eprintln!(
                    "[CODEGEN DEBUG] Building case for variant: {}",
                    variant_name
                );
                self.builder.position_at_end(case_block);

                let variant_str = self
                    .builder
                    .build_global_string_ptr(variant_name, &format!("var_{}", variant_name))
                    .unwrap()
                    .as_pointer_value();

                // Call FFI to get status code
                let status = self
                    .builder
                    .build_call(
                        doohttp_error_to_status_fn,
                        &[error_type_str.into(), variant_str.into()],
                        "err_status",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                // Call FFI to get message
                let message = self
                    .builder
                    .build_call(
                        doohttp_error_message_fn,
                        &[error_type_str.into(), variant_str.into()],
                        "err_msg",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                phi_status_vals.push((status, case_block));
                phi_msg_vals.push((message, case_block));

                self.builder
                    .build_unconditional_branch(merge_block)
                    .unwrap();
            }

            // Default block
            eprintln!("[CODEGEN DEBUG] Building default error block");
            self.builder.position_at_end(default_block);
            let def_status = i32_type.const_int(500, false);
            let def_msg = self
                .builder
                .build_global_string_ptr("Internal server error", "def_msg")
                .unwrap()
                .as_pointer_value();
            phi_status_vals.push((def_status, default_block));
            phi_msg_vals.push((def_msg, default_block));
            self.builder
                .build_unconditional_branch(merge_block)
                .unwrap();

            // Merge block with phi nodes
            eprintln!(
                "[CODEGEN DEBUG] Building merge block with {} phi entries",
                phi_status_vals.len()
            );
            self.builder.position_at_end(merge_block);
            let status_phi = self.builder.build_phi(i32_type, "status_phi").unwrap();
            let msg_phi = self.builder.build_phi(ptr_type, "msg_phi").unwrap();

            for (val, block) in phi_status_vals {
                status_phi.add_incoming(&[(&val, block)]);
            }
            for (val, block) in phi_msg_vals {
                msg_phi.add_incoming(&[(&val, block)]);
            }

            eprintln!("[CODEGEN DEBUG] Error handling complete, returning phi values");
            return (
                status_phi.as_basic_value().into_int_value(),
                msg_phi.as_basic_value().into_pointer_value(),
            );
        }

        // Fallback: no metadata
        eprintln!(
            "[CODEGEN DEBUG] No enum metadata found for {}, using fallback",
            error_type
        );
        let status = i32_type.const_int(500, false);
        let msg = self
            .builder
            .build_global_string_ptr("Internal server error", "fallback_msg")
            .unwrap()
            .as_pointer_value();
        (status, msg)
    }

    /// Create error response with specific status code and message (no internal branches)
    /// Uses RFC 7807 format via FFI
    fn create_error_response_with_status(
        &mut self,
        status_code: inkwell::values::IntValue<'ctx>,
        error_msg_ptr: inkwell::values::PointerValue<'ctx>,
    ) -> inkwell::values::PointerValue<'ctx> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();

        let response_size = i64_type.const_int(24, false);
        let malloc_fn = self.module.get_function("malloc").unwrap();
        let response_ptr = self
            .builder
            .build_call(malloc_fn, &[response_size.into()], "error_response_ptr")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Set status code
        let status_field_ptr = self
            .builder
            .build_pointer_cast(
                response_ptr,
                i32_type.ptr_type(AddressSpace::default()),
                "status_ptr",
            )
            .unwrap();
        self.builder
            .build_store(status_field_ptr, status_code)
            .unwrap();

        // Use RFC 7807 format via FFI
        // Declare doohttp_error_rfc7807 function
        let doohttp_error_rfc7807_fn =
            if let Some(f) = self.module.get_function("doohttp_error_rfc7807") {
                f
            } else {
                let fn_type =
                    ptr_type.fn_type(&[i32_type.into(), ptr_type.into(), ptr_type.into()], false);
                self.module
                    .add_function("doohttp_error_rfc7807", fn_type, None)
            };

        // Extract instance path from current request pointer stored in global
        let global_request_ptr = self
            .module
            .get_global("__doo_current_request_ptr")
            .expect("__doo_current_request_ptr global not found");

        let request_ptr = self
            .builder
            .build_load(
                ptr_type,
                global_request_ptr.as_pointer_value(),
                "current_request_ptr",
            )
            .unwrap()
            .into_pointer_value();

        // Call doohttp_get_request_path to extract path from DooRequest
        let get_path_fn = self
            .module
            .get_function("doohttp_get_request_path")
            .expect("doohttp_get_request_path FFI function not found");

        let instance_path = self
            .builder
            .build_call(get_path_fn, &[request_ptr.into()], "request_path")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Call doohttp_error_rfc7807(status, detail, instance)
        let rfc7807_json = self
            .builder
            .build_call(
                doohttp_error_rfc7807_fn,
                &[
                    status_code.into(),
                    error_msg_ptr.into(),
                    instance_path.into(),
                ],
                "rfc7807_json",
            )
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Body field (offset 8)
        let body_field_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    response_ptr,
                    &[i32_type.const_int(8, false)],
                    "body_field_ptr",
                )
                .unwrap()
        };
        let body_field_ptr_typed = self
            .builder
            .build_pointer_cast(
                body_field_ptr,
                ptr_type.ptr_type(AddressSpace::default()),
                "body_field_ptr_typed",
            )
            .unwrap();
        self.builder
            .build_store(body_field_ptr_typed, rfc7807_json)
            .unwrap();

        // Content-Type field (offset 16)
        let content_type_str = self
            .builder
            .build_global_string_ptr("application/json", "content_type_json_err")
            .unwrap()
            .as_pointer_value();
        let content_type_field_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    response_ptr,
                    &[i32_type.const_int(16, false)],
                    "content_type_field_ptr",
                )
                .unwrap()
        };
        let content_type_field_ptr_typed = self
            .builder
            .build_pointer_cast(
                content_type_field_ptr,
                ptr_type.ptr_type(AddressSpace::default()),
                "content_type_field_ptr_typed",
            )
            .unwrap();
        self.builder
            .build_store(content_type_field_ptr_typed, content_type_str)
            .unwrap();

        response_ptr
    }

    /// Wrap a plain value in DooResponse struct with specific status code
    fn wrap_value_in_response_with_status(
        &mut self,
        result: inkwell::values::BasicValueEnum<'ctx>,
        return_type: &str,
        status_code: u32,
    ) -> inkwell::values::PointerValue<'ctx> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();

        // Allocate DooResponse struct: { i32 status, *const c_char body, *const c_char content_type }
        let response_size = i64_type.const_int(24, false); // 4 bytes status + 8 bytes body ptr + 8 bytes content_type ptr + padding
        let malloc_fn = self.module.get_function("malloc").unwrap();
        let response_ptr = self
            .builder
            .build_call(malloc_fn, &[response_size.into()], "response_ptr")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Cast to i32* for status field
        let status_field_ptr = self
            .builder
            .build_pointer_cast(
                response_ptr,
                i32_type.ptr_type(AddressSpace::default()),
                "status_field_ptr",
            )
            .unwrap();

        // Set status to provided status code
        let status_value = i32_type.const_int(status_code as u64, false);
        self.builder
            .build_store(status_field_ptr, status_value)
            .unwrap();

        // Convert result to JSON string
        let body_json_str = self.convert_value_to_json_string(result, return_type);

        // Get pointer to body field (offset 8 from start, after i32 status + padding)
        let body_field_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    response_ptr,
                    &[i32_type.const_int(8, false)],
                    "body_field_ptr",
                )
                .unwrap()
        };
        let body_field_ptr_typed = self
            .builder
            .build_pointer_cast(
                body_field_ptr,
                ptr_type.ptr_type(AddressSpace::default()),
                "body_field_ptr_typed",
            )
            .unwrap();
        self.builder
            .build_store(body_field_ptr_typed, body_json_str)
            .unwrap();

        // Set content-type to "application/json"
        let content_type_str = self
            .builder
            .build_global_string_ptr("application/json", "content_type_json")
            .unwrap()
            .as_pointer_value();
        let content_type_field_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    response_ptr,
                    &[i32_type.const_int(16, false)],
                    "content_type_field_ptr",
                )
                .unwrap()
        };
        let content_type_field_ptr_typed = self
            .builder
            .build_pointer_cast(
                content_type_field_ptr,
                ptr_type.ptr_type(AddressSpace::default()),
                "content_type_field_ptr_typed",
            )
            .unwrap();
        self.builder
            .build_store(content_type_field_ptr_typed, content_type_str)
            .unwrap();

        response_ptr
    }

    /// Create an empty DooResponse (for Void returns)
    fn create_empty_response(&mut self) -> inkwell::values::PointerValue<'ctx> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();

        let response_size = i64_type.const_int(24, false);
        let malloc_fn = self.module.get_function("malloc").unwrap();
        let response_ptr = self
            .builder
            .build_call(malloc_fn, &[response_size.into()], "empty_response_ptr")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Status 200
        let status_field_ptr = self
            .builder
            .build_pointer_cast(
                response_ptr,
                i32_type.ptr_type(AddressSpace::default()),
                "status_ptr",
            )
            .unwrap();
        self.builder
            .build_store(status_field_ptr, i32_type.const_int(200, false))
            .unwrap();

        // Empty body
        let empty_body = self
            .builder
            .build_global_string_ptr("", "empty_body")
            .unwrap()
            .as_pointer_value();
        let body_field_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    response_ptr,
                    &[i32_type.const_int(8, false)],
                    "body_ptr",
                )
                .unwrap()
        };
        let body_field_ptr_typed = self
            .builder
            .build_pointer_cast(
                body_field_ptr,
                ptr_type.ptr_type(AddressSpace::default()),
                "body_ptr_typed",
            )
            .unwrap();
        self.builder
            .build_store(body_field_ptr_typed, empty_body)
            .unwrap();

        // Content-type
        let content_type = self
            .builder
            .build_global_string_ptr("application/json", "ct")
            .unwrap()
            .as_pointer_value();
        let ct_field_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    response_ptr,
                    &[i32_type.const_int(16, false)],
                    "ct_ptr",
                )
                .unwrap()
        };
        let ct_field_ptr_typed = self
            .builder
            .build_pointer_cast(
                ct_field_ptr,
                ptr_type.ptr_type(AddressSpace::default()),
                "ct_ptr_typed",
            )
            .unwrap();
        self.builder
            .build_store(ct_field_ptr_typed, content_type)
            .unwrap();

        response_ptr
    }

    /// Wrap DooResponse* in DooResult* (success case)
    fn wrap_response_in_result(
        &mut self,
        response_ptr: inkwell::values::PointerValue<'ctx>,
    ) -> inkwell::values::PointerValue<'ctx> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();

        // Allocate DooResult struct: { i32 tag, *mut c_void value }
        let result_size = i64_type.const_int(16, false); // 4 bytes tag + padding + 8 bytes ptr
        let malloc_fn = self.module.get_function("malloc").unwrap();
        let result_ptr = self
            .builder
            .build_call(malloc_fn, &[result_size.into()], "result_ptr")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Set tag to 0 (Ok)
        let tag_field_ptr = self
            .builder
            .build_pointer_cast(
                result_ptr,
                i32_type.ptr_type(AddressSpace::default()),
                "tag_ptr",
            )
            .unwrap();
        self.builder
            .build_store(tag_field_ptr, i32_type.const_int(0, false))
            .unwrap();

        // Set value to response_ptr
        let value_field_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    result_ptr,
                    &[i32_type.const_int(8, false)],
                    "value_ptr",
                )
                .unwrap()
        };
        let value_field_ptr_typed = self
            .builder
            .build_pointer_cast(
                value_field_ptr,
                ptr_type.ptr_type(AddressSpace::default()),
                "value_ptr_typed",
            )
            .unwrap();
        self.builder
            .build_store(value_field_ptr_typed, response_ptr)
            .unwrap();

        result_ptr
    }

    /// Convert a value to JSON string representation
    fn convert_value_to_json_string(
        &mut self,
        value: inkwell::values::BasicValueEnum<'ctx>,
        type_str: &str,
    ) -> inkwell::values::PointerValue<'ctx> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let malloc_fn = self.module.get_function("malloc").unwrap();
        let sprintf_fn = self.module.get_function("sprintf").unwrap();

        // Allocate buffer for JSON string (256 bytes should be enough for simple types)
        let buffer_size = i64_type.const_int(256, false);
        let json_buffer = self
            .builder
            .build_call(malloc_fn, &[buffer_size.into()], "json_buffer")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Check if value is a struct pointer first (most common case for HTTP handlers)
        if value.is_pointer_value() && self.struct_metadata.contains_key(type_str) {
            // Struct: serialize to JSON object
            self.serialize_struct_to_json(value.into_pointer_value(), type_str, json_buffer);
            return json_buffer;
        }

        // Handle different types based on value type and type_str
        if value.is_int_value() && (type_str == "Int" || type_str == "I32" || type_str == "I64") {
            // Integer: format as "%d"
            let format_str = self
                .builder
                .build_global_string_ptr("%d", "int_fmt")
                .unwrap()
                .as_pointer_value();
            let int_val = value.into_int_value();
            self.builder
                .build_call(
                    sprintf_fn,
                    &[json_buffer.into(), format_str.into(), int_val.into()],
                    "",
                )
                .unwrap();
        } else if value.is_float_value()
            && (type_str == "Float" || type_str == "F32" || type_str == "F64")
        {
            // Float: format as "%f"
            let format_str = self
                .builder
                .build_global_string_ptr("%f", "float_fmt")
                .unwrap()
                .as_pointer_value();
            let float_val = value.into_float_value();
            self.builder
                .build_call(
                    sprintf_fn,
                    &[json_buffer.into(), format_str.into(), float_val.into()],
                    "",
                )
                .unwrap();
        } else if value.is_int_value() && type_str == "Bool" {
            // Boolean: "true" or "false"
            let bool_val = value.into_int_value();
            let true_str = self
                .builder
                .build_global_string_ptr("true", "true_str")
                .unwrap()
                .as_pointer_value();
            let false_str = self
                .builder
                .build_global_string_ptr("false", "false_str")
                .unwrap()
                .as_pointer_value();
            let is_true = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    bool_val,
                    i32_type.const_int(0, false),
                    "is_true",
                )
                .unwrap();
            let result_str = self
                .builder
                .build_select(is_true, true_str, false_str, "bool_str")
                .unwrap();
            let format_str = self
                .builder
                .build_global_string_ptr("%s", "str_fmt")
                .unwrap()
                .as_pointer_value();
            self.builder
                .build_call(
                    sprintf_fn,
                    &[json_buffer.into(), format_str.into(), result_str.into()],
                    "",
                )
                .unwrap();
        } else if value.is_pointer_value() && (type_str == "Str" || type_str.contains("String")) {
            // String: wrap in quotes "\"value\""
            let format_str = self
                .builder
                .build_global_string_ptr("\"%s\"", "str_quoted_fmt")
                .unwrap()
                .as_pointer_value();
            let str_val = value.into_pointer_value();
            self.builder
                .build_call(
                    sprintf_fn,
                    &[json_buffer.into(), format_str.into(), str_val.into()],
                    "",
                )
                .unwrap();
        } else if value.is_pointer_value() {
            // Pointer value but not a known struct - try to serialize as struct anyway
            self.serialize_struct_to_json(value.into_pointer_value(), type_str, json_buffer);
        } else {
            // Unknown type: return "null"
            let null_str = self
                .builder
                .build_global_string_ptr("null", "null_str")
                .unwrap()
                .as_pointer_value();
            let format_str = self
                .builder
                .build_global_string_ptr("%s", "str_fmt")
                .unwrap()
                .as_pointer_value();
            self.builder
                .build_call(
                    sprintf_fn,
                    &[json_buffer.into(), format_str.into(), null_str.into()],
                    "",
                )
                .unwrap();
        }

        json_buffer
    }

    /// Serialize a struct to JSON string
    fn serialize_struct_to_json(
        &mut self,
        struct_ptr: inkwell::values::PointerValue<'ctx>,
        struct_type: &str,
        json_buffer: inkwell::values::PointerValue<'ctx>,
    ) {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let sprintf_fn = self.module.get_function("sprintf").unwrap();

        // Get or declare strcat
        let strcat_fn = if let Some(func) = self.module.get_function("strcat") {
            func
        } else {
            let strcat_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
            self.module.add_function("strcat", strcat_type, None)
        };

        // Get struct metadata
        if let Some(metadata) = self.struct_metadata.get(struct_type) {
            // Start with opening brace
            let open_brace = self
                .builder
                .build_global_string_ptr("{", "json_open")
                .unwrap()
                .as_pointer_value();
            let format_str = self
                .builder
                .build_global_string_ptr("%s", "str_fmt")
                .unwrap()
                .as_pointer_value();
            self.builder
                .build_call(
                    sprintf_fn,
                    &[json_buffer.into(), format_str.into(), open_brace.into()],
                    "",
                )
                .unwrap();

            // Iterate through fields
            for (field_idx, field_name) in metadata.field_names.iter().enumerate() {
                let field_type = &metadata.field_types[field_idx];

                // Add comma separator if not first field
                if field_idx > 0 {
                    let comma = self
                        .builder
                        .build_global_string_ptr(",", "comma")
                        .unwrap()
                        .as_pointer_value();
                    self.builder
                        .build_call(strcat_fn, &[json_buffer.into(), comma.into()], "")
                        .unwrap();
                }

                // Add field name: "fieldName":
                let field_name_json = format!("\"{}\":", field_name);
                let field_name_str = self
                    .builder
                    .build_global_string_ptr(&field_name_json, &format!("field_{}", field_name))
                    .unwrap()
                    .as_pointer_value();
                self.builder
                    .build_call(strcat_fn, &[json_buffer.into(), field_name_str.into()], "")
                    .unwrap();

                // Get field value from struct
                let struct_llvm_type = *self.canonical_struct_types.get(struct_type).unwrap();
                let field_ptr = unsafe {
                    self.builder
                        .build_struct_gep(
                            struct_llvm_type,
                            struct_ptr,
                            field_idx as u32,
                            &format!("field_{}_ptr", field_name),
                        )
                        .unwrap()
                };

                // Load field value and convert to JSON
                let field_llvm_type = self.type_string_to_llvm_type(field_type);
                let field_value = self
                    .builder
                    .build_load(
                        field_llvm_type,
                        field_ptr,
                        &format!("field_{}_val", field_name),
                    )
                    .unwrap();

                // Create temporary buffer for field value
                let temp_buffer = self
                    .builder
                    .build_call(
                        self.module.get_function("malloc").unwrap(),
                        &[i64_type.const_int(128, false).into()],
                        "temp_buf",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                // Serialize field value based on type
                if field_type == "Int" || field_type == "I32" {
                    let int_fmt = self
                        .builder
                        .build_global_string_ptr("%d", "int_fmt")
                        .unwrap()
                        .as_pointer_value();
                    self.builder
                        .build_call(
                            sprintf_fn,
                            &[
                                temp_buffer.into(),
                                int_fmt.into(),
                                field_value.into_int_value().into(),
                            ],
                            "",
                        )
                        .unwrap();
                } else if field_type == "Str" || field_type.contains("String") {
                    // String field: wrap in quotes
                    let str_fmt = self
                        .builder
                        .build_global_string_ptr("\"%s\"", "str_quoted")
                        .unwrap()
                        .as_pointer_value();
                    self.builder
                        .build_call(
                            sprintf_fn,
                            &[
                                temp_buffer.into(),
                                str_fmt.into(),
                                field_value.into_pointer_value().into(),
                            ],
                            "",
                        )
                        .unwrap();
                } else if field_type == "Bool" {
                    // Boolean: true or false
                    let bool_val = field_value.into_int_value();
                    let true_str = self
                        .builder
                        .build_global_string_ptr("true", "true")
                        .unwrap()
                        .as_pointer_value();
                    let false_str = self
                        .builder
                        .build_global_string_ptr("false", "false")
                        .unwrap()
                        .as_pointer_value();
                    let is_true = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::NE,
                            bool_val,
                            i32_type.const_int(0, false),
                            "is_true",
                        )
                        .unwrap();
                    let bool_str = self
                        .builder
                        .build_select(is_true, true_str, false_str, "bool_str")
                        .unwrap();
                    let fmt = self
                        .builder
                        .build_global_string_ptr("%s", "fmt")
                        .unwrap()
                        .as_pointer_value();
                    self.builder
                        .build_call(
                            sprintf_fn,
                            &[temp_buffer.into(), fmt.into(), bool_str.into()],
                            "",
                        )
                        .unwrap();
                } else {
                    // Default: null for unknown types
                    let null_str = self
                        .builder
                        .build_global_string_ptr("null", "null")
                        .unwrap()
                        .as_pointer_value();
                    let fmt = self
                        .builder
                        .build_global_string_ptr("%s", "fmt")
                        .unwrap()
                        .as_pointer_value();
                    self.builder
                        .build_call(
                            sprintf_fn,
                            &[temp_buffer.into(), fmt.into(), null_str.into()],
                            "",
                        )
                        .unwrap();
                }

                // Append field value to JSON buffer
                self.builder
                    .build_call(strcat_fn, &[json_buffer.into(), temp_buffer.into()], "")
                    .unwrap();
            }

            // Close JSON object
            let close_brace = self
                .builder
                .build_global_string_ptr("}", "json_close")
                .unwrap()
                .as_pointer_value();
            self.builder
                .build_call(strcat_fn, &[json_buffer.into(), close_brace.into()], "")
                .unwrap();
        } else {
            // Struct metadata not found, return empty object
            let empty_obj = self
                .builder
                .build_global_string_ptr("{}", "empty_obj")
                .unwrap()
                .as_pointer_value();
            let fmt = self
                .builder
                .build_global_string_ptr("%s", "fmt")
                .unwrap()
                .as_pointer_value();
            self.builder
                .build_call(
                    sprintf_fn,
                    &[json_buffer.into(), fmt.into(), empty_obj.into()],
                    "",
                )
                .unwrap();
        }
    }

    /// Build validator spec string from struct field decorators and types
    /// Format per field: "fieldName:Type:validator1|validator2"
    /// Examples:
    ///   Email:Str:email
    ///   Age:Int:min18
    ///   Note:Str:
    fn build_validator_spec(&self, struct_type: &str) -> String {
        let mut specs = Vec::new();

        // Get struct metadata
        if let Some(metadata) = self.struct_metadata.get(struct_type) {
            // Check if we have decorator info stored
            if let Some(field_decorators) = self.struct_field_decorators.get(struct_type) {
                for (idx, field_name) in metadata.field_names.iter().enumerate() {
                    let field_type = metadata.field_types.get(idx).cloned().unwrap_or_default();
                    if let Some(decorators) = field_decorators.get(field_name) {
                        if !decorators.is_empty() {
                            let decorator_strs: Vec<String> = decorators
                                .iter()
                                .map(|(name, args)| {
                                    if args.is_empty() {
                                        name.clone()
                                    } else {
                                        // Handle decorators with arguments
                                        match name.as_str() {
                                            "min" | "max" => {
                                                format!("{}{}", name, args[0])
                                            }
                                            "enum" => {
                                                format!("enum:{}", args.join("|"))
                                            }
                                            "pattern" => {
                                                format!("pattern:{}", args[0])
                                            }
                                            _ => name.clone(),
                                        }
                                    }
                                })
                                .collect();

                            let field_spec = format!(
                                "{}:{}:{}",
                                field_name,
                                field_type,
                                decorator_strs.join("|")
                            );
                            specs.push(field_spec);
                        } else {
                            // No decorators, still include type for required/missing checks
                            specs.push(format!("{}:{}:", field_name, field_type));
                        }
                    } else {
                        // No decorator metadata, still include type
                        specs.push(format!("{}:{}:", field_name, field_type));
                    }
                }
            }
        }

        specs.join(";")
    }

    fn parse_json_into_struct(
        &mut self,
        json_str_ptr: inkwell::values::PointerValue<'ctx>,
        struct_ptr: inkwell::values::PointerValue<'ctx>,
        struct_type: &str,
    ) {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i32_type = self.context.i32_type();

        // Build validator spec from struct field decorators
        let validator_spec = self.build_validator_spec(struct_type);

        // Declare doohttp_parse_json_struct FFI function
        // fn doohttp_parse_json_struct(body: *const c_char, struct_name: *const c_char, validator_spec: *const c_char) -> *mut c_void
        let parse_json_struct_fn =
            if let Some(f) = self.module.get_function("doohttp_parse_json_struct") {
                f
            } else {
                let fn_type =
                    ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false);
                self.module
                    .add_function("doohttp_parse_json_struct", fn_type, None)
            };

        // Create struct name string
        let struct_name_ptr = self
            .builder
            .build_global_string_ptr(struct_type, "struct_name")
            .unwrap()
            .as_pointer_value();

        // Create validator spec string
        let validator_spec_ptr = self
            .builder
            .build_global_string_ptr(&validator_spec, "validator_spec")
            .unwrap()
            .as_pointer_value();

        // Call doohttp_parse_json_struct
        // This function:
        // 1. Parses JSON and validates structure (400 if malformed, missing fields, wrong types, unknown fields)
        // 2. Validates decorators (@email, @min, @max, etc.) (422 if validation fails)
        // 3. Returns parsed JSON string ptr on success, NULL on error
        // 4. Sets last error via set_last_error() for automatic RFC 7807 response
        let parsed_result = self
            .builder
            .build_call(
                parse_json_struct_fn,
                &[
                    json_str_ptr.into(),
                    struct_name_ptr.into(),
                    validator_spec_ptr.into(),
                ],
                "parsed_json",
            )
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Check if parsing succeeded (non-NULL result)
        let parse_success = self
            .builder
            .build_is_not_null(parsed_result, "parse_success")
            .unwrap();

        let success_block = self.context.append_basic_block(
            self.builder
                .get_insert_block()
                .unwrap()
                .get_parent()
                .unwrap(),
            "parse_success",
        );
        let error_block = self.context.append_basic_block(
            self.builder
                .get_insert_block()
                .unwrap()
                .get_parent()
                .unwrap(),
            "parse_error",
        );

        self.builder
            .build_conditional_branch(parse_success, success_block, error_block)
            .unwrap();

        // Error block: Return error response
        self.builder.position_at_end(error_block);

        // Get last error status and JSON from FFI
        let last_error_status_fn =
            if let Some(f) = self.module.get_function("doohttp_last_error_status") {
                f
            } else {
                let fn_type = i32_type.fn_type(&[], false);
                self.module
                    .add_function("doohttp_last_error_status", fn_type, None)
            };

        let last_error_json_fn =
            if let Some(f) = self.module.get_function("doohttp_last_error_json") {
                f
            } else {
                let fn_type = ptr_type.fn_type(&[], false);
                self.module
                    .add_function("doohttp_last_error_json", fn_type, None)
            };

        let error_status = self
            .builder
            .build_call(last_error_status_fn, &[], "error_status")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();

        let error_json = self
            .builder
            .build_call(last_error_json_fn, &[], "error_json")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Create DooResult error struct { i32 tag=1, ptr value=DooHttpError }
        // DooHttpError: { i32 status, *const i8 message }
        let http_error_type = self
            .context
            .struct_type(&[i32_type.into(), ptr_type.into()], false);

        // Allocate DooHttpError on heap
        let malloc_fn = self.module.get_function("malloc").unwrap();
        let error_size = self.context.i64_type().const_int(16, false);
        let http_error_ptr = self
            .builder
            .build_call(malloc_fn, &[error_size.into()], "http_error_malloc")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Store status
        let status_ptr = self
            .builder
            .build_struct_gep(http_error_type, http_error_ptr, 0, "status_ptr")
            .unwrap();
        self.builder.build_store(status_ptr, error_status).unwrap();

        // Store message
        let message_ptr = self
            .builder
            .build_struct_gep(http_error_type, http_error_ptr, 1, "message_ptr")
            .unwrap();
        self.builder.build_store(message_ptr, error_json).unwrap();

        // Create DooResult error: { i32 tag=1, ptr value=http_error_ptr }
        let result_type = self
            .context
            .struct_type(&[i32_type.into(), ptr_type.into()], false);
        let result_size = self.context.i64_type().const_int(16, false);
        let result_ptr = self
            .builder
            .build_call(malloc_fn, &[result_size.into()], "result_malloc")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Set tag=1 (error)
        let tag_ptr = self
            .builder
            .build_struct_gep(result_type, result_ptr, 0, "tag_ptr")
            .unwrap();
        self.builder
            .build_store(tag_ptr, i32_type.const_int(1, false))
            .unwrap();

        // Set value=http_error_ptr
        let value_ptr = self
            .builder
            .build_struct_gep(result_type, result_ptr, 1, "value_ptr")
            .unwrap();
        self.builder.build_store(value_ptr, http_error_ptr).unwrap();

        // Return the error result
        self.builder.build_return(Some(&result_ptr)).unwrap();

        // Success block: Continue with manual parsing (for now, until we have full struct deserialization)
        self.builder.position_at_end(success_block);

        // Get struct metadata for manual field parsing
        if let Some(metadata) = self.struct_metadata.get(struct_type).cloned() {
            // Declare JSON parsing helper functions
            let strstr_fn = if let Some(f) = self.module.get_function("strstr") {
                f
            } else {
                let fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                self.module.add_function("strstr", fn_type, None)
            };
            let strchr_fn = if let Some(f) = self.module.get_function("strchr") {
                f
            } else {
                let fn_type = ptr_type.fn_type(&[ptr_type.into(), i32_type.into()], false);
                self.module.add_function("strchr", fn_type, None)
            };
            let atoi_fn = if let Some(f) = self.module.get_function("atoi") {
                f
            } else {
                let fn_type = i32_type.fn_type(&[ptr_type.into()], false);
                self.module.add_function("atoi", fn_type, None)
            };

            // For each field in the struct, try to extract it from JSON
            for (field_idx, field_name) in metadata.field_names.iter().enumerate() {
                let field_type = &metadata.field_types[field_idx];

                // Create search pattern: "fieldName": (without trailing quote for flexibility)
                let search_pattern = format!("\"{}\":", field_name);
                let pattern_str = self
                    .builder
                    .build_global_string_ptr(&search_pattern, &format!("pattern_{}", field_name))
                    .unwrap()
                    .as_pointer_value();

                // Find the field in JSON: strstr(json, "fieldName":")
                let field_start = self
                    .builder
                    .build_call(
                        strstr_fn,
                        &[json_str_ptr.into(), pattern_str.into()],
                        "field_start",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                // Check if field was found (not null)
                let field_found = self
                    .builder
                    .build_is_not_null(field_start, "field_found")
                    .unwrap();

                let then_block = self.context.append_basic_block(
                    self.builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap(),
                    &format!("parse_field_{}", field_name),
                );
                let continue_block = self.context.append_basic_block(
                    self.builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap(),
                    &format!("continue_{}", field_name),
                );

                self.builder
                    .build_conditional_branch(field_found, then_block, continue_block)
                    .unwrap();

                self.builder.position_at_end(then_block);

                // Skip past the pattern to get to the value
                let pattern_len = search_pattern.len() as u64;
                let after_colon = unsafe {
                    self.builder
                        .build_gep(
                            self.context.i8_type(),
                            field_start,
                            &[i32_type.const_int(pattern_len, false)],
                            "after_colon",
                        )
                        .unwrap()
                };

                // Skip whitespace after colon
                // For now, just skip one space if present
                let first_char = self
                    .builder
                    .build_load(self.context.i8_type(), after_colon, "first_char")
                    .unwrap()
                    .into_int_value();
                let is_space = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        first_char,
                        self.context.i8_type().const_int(' ' as u64, false),
                        "is_space",
                    )
                    .unwrap();
                let skip_amount = self
                    .builder
                    .build_select(
                        is_space,
                        i32_type.const_int(1, false),
                        i32_type.const_int(0, false),
                        "skip_amount",
                    )
                    .unwrap()
                    .into_int_value();
                let value_start_raw = unsafe {
                    self.builder
                        .build_gep(
                            self.context.i8_type(),
                            after_colon,
                            &[skip_amount],
                            "value_start_raw",
                        )
                        .unwrap()
                };

                // Check if value starts with quote (string) or not (number/bool)
                let first_value_char = self
                    .builder
                    .build_load(self.context.i8_type(), value_start_raw, "first_value_char")
                    .unwrap()
                    .into_int_value();
                let is_quoted = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        first_value_char,
                        self.context.i8_type().const_int('"' as u64, false),
                        "is_quoted",
                    )
                    .unwrap();

                // If quoted, skip the opening quote
                let value_start_adjusted = self
                    .builder
                    .build_select(
                        is_quoted,
                        unsafe {
                            self.builder
                                .build_gep(
                                    self.context.i8_type(),
                                    value_start_raw,
                                    &[i32_type.const_int(1, false)],
                                    "skip_quote",
                                )
                                .unwrap()
                        },
                        value_start_raw,
                        "value_start",
                    )
                    .unwrap()
                    .into_pointer_value();
                let value_start = value_start_adjusted;

                // Get field pointer in struct
                let struct_llvm_type = *self.canonical_struct_types.get(struct_type).unwrap();
                let field_ptr = unsafe {
                    self.builder
                        .build_struct_gep(
                            struct_llvm_type,
                            struct_ptr,
                            field_idx as u32,
                            &format!("field_{}_ptr", field_name),
                        )
                        .unwrap()
                };

                // Parse based on field type
                if field_type == "Str" || field_type.contains("String") {
                    // For strings, find closing quote or comma/brace
                    // Try to find closing quote first
                    let quote_char = i32_type.const_int('"' as u64, false);
                    let value_end_quote = self
                        .builder
                        .build_call(
                            strchr_fn,
                            &[value_start.into(), quote_char.into()],
                            "value_end_quote",
                        )
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    // Also try comma (for non-last fields) and closing brace (for last field)
                    let comma_char = i32_type.const_int(',' as u64, false);
                    let value_end_comma = self
                        .builder
                        .build_call(
                            strchr_fn,
                            &[value_start.into(), comma_char.into()],
                            "value_end_comma",
                        )
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    let brace_char = i32_type.const_int('}' as u64, false);
                    let value_end_brace = self
                        .builder
                        .build_call(
                            strchr_fn,
                            &[value_start.into(), brace_char.into()],
                            "value_end_brace",
                        )
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    // Use quote if found and valid, otherwise use the earlier of comma/brace
                    let quote_is_null = self
                        .builder
                        .build_is_null(value_end_quote, "quote_is_null")
                        .unwrap();

                    // Choose between comma and brace (whichever comes first)
                    let comma_int = self
                        .builder
                        .build_ptr_to_int(value_end_comma, self.context.i64_type(), "comma_int")
                        .unwrap();
                    let brace_int = self
                        .builder
                        .build_ptr_to_int(value_end_brace, self.context.i64_type(), "brace_int")
                        .unwrap();
                    let comma_is_null = self
                        .builder
                        .build_is_null(value_end_comma, "comma_is_null")
                        .unwrap();
                    let use_brace = self
                        .builder
                        .build_or(
                            comma_is_null,
                            self.builder
                                .build_int_compare(
                                    inkwell::IntPredicate::ULT,
                                    brace_int,
                                    comma_int,
                                    "brace_before_comma",
                                )
                                .unwrap(),
                            "use_brace",
                        )
                        .unwrap();
                    let value_end_fallback = self
                        .builder
                        .build_select(
                            use_brace,
                            value_end_brace,
                            value_end_comma,
                            "value_end_fallback",
                        )
                        .unwrap()
                        .into_pointer_value();

                    let value_end = self
                        .builder
                        .build_select(
                            quote_is_null,
                            value_end_fallback,
                            value_end_quote,
                            "value_end",
                        )
                        .unwrap()
                        .into_pointer_value();

                    // Calculate length
                    let value_end_int = self
                        .builder
                        .build_ptr_to_int(value_end, self.context.i64_type(), "end_int")
                        .unwrap();
                    let value_start_int = self
                        .builder
                        .build_ptr_to_int(value_start, self.context.i64_type(), "start_int")
                        .unwrap();
                    let str_len = self
                        .builder
                        .build_int_sub(value_end_int, value_start_int, "str_len")
                        .unwrap();

                    // Allocate and copy string
                    let malloc_fn = self.module.get_function("malloc").unwrap();
                    let str_len_plus_one = self
                        .builder
                        .build_int_add(
                            str_len,
                            self.context.i64_type().const_int(1, false),
                            "len_plus_one",
                        )
                        .unwrap();
                    let str_buffer = self
                        .builder
                        .build_call(malloc_fn, &[str_len_plus_one.into()], "str_buffer")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    // Copy string using memcpy
                    let memcpy_fn = self.module.get_function("memcpy").unwrap();
                    self.builder
                        .build_call(
                            memcpy_fn,
                            &[str_buffer.into(), value_start.into(), str_len.into()],
                            "",
                        )
                        .unwrap();

                    // Null terminate
                    let null_pos = unsafe {
                        self.builder
                            .build_gep(self.context.i8_type(), str_buffer, &[str_len], "null_pos")
                            .unwrap()
                    };
                    self.builder
                        .build_store(null_pos, self.context.i8_type().const_int(0, false))
                        .unwrap();

                    // Store in struct
                    self.builder.build_store(field_ptr, str_buffer).unwrap();
                } else if field_type == "Int" || field_type == "I32" {
                    // Parse integer using atoi
                    let int_value = self
                        .builder
                        .build_call(atoi_fn, &[value_start.into()], "int_value")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_int_value();

                    self.builder.build_store(field_ptr, int_value).unwrap();
                }

                self.builder
                    .build_unconditional_branch(continue_block)
                    .unwrap();
                self.builder.position_at_end(continue_block);
            }
        }
    }

    /// Parse query parameters from request into a struct
    fn parse_query_into_struct(
        &mut self,
        request_ptr: inkwell::values::PointerValue<'ctx>,
        struct_ptr: inkwell::values::PointerValue<'ctx>,
        struct_type: &str,
    ) {
        // Get struct metadata
        if let Some(metadata) = self.struct_metadata.get(struct_type).cloned() {
            let ptr_type = self.context.ptr_type(AddressSpace::default());
            let i32_type = self.context.i32_type();
            let i64_type = self.context.i64_type();

            // Declare query helper function to get values from request
            let doohttp_extract_query_typed_fn =
                if let Some(f) = self.module.get_function("doohttp_extract_query_typed") {
                    f
                } else {
                    // fn doohttp_extract_query_typed(request: *const DooRequest, name: *const c_char, ty: *const c_char) -> *const c_char
                    let fn_type = ptr_type
                        .fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false);
                    self.module
                        .add_function("doohttp_extract_query_typed", fn_type, None)
                };

            let atoi_fn = if let Some(f) = self.module.get_function("atoi") {
                f
            } else {
                let fn_type = i32_type.fn_type(&[ptr_type.into()], false);
                self.module.add_function("atoi", fn_type, None)
            };
            let atof_fn = if let Some(f) = self.module.get_function("atof") {
                f
            } else {
                let fn_type = self.context.f64_type().fn_type(&[ptr_type.into()], false);
                self.module.add_function("atof", fn_type, None)
            };
            let strcmp_fn = if let Some(f) = self.module.get_function("strcmp") {
                f
            } else {
                let fn_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                self.module.add_function("strcmp", fn_type, None)
            };

            // Helpers to read last RFC7807 error after each extraction
            let last_error_status_fn =
                if let Some(f) = self.module.get_function("doohttp_last_error_status") {
                    f
                } else {
                    let fn_type = i32_type.fn_type(&[], false);
                    self.module
                        .add_function("doohttp_last_error_status", fn_type, None)
                };
            let last_error_json_fn =
                if let Some(f) = self.module.get_function("doohttp_last_error_json") {
                    f
                } else {
                    let fn_type = ptr_type.fn_type(&[], false);
                    self.module
                        .add_function("doohttp_last_error_json", fn_type, None)
                };

            // For each field in the struct, extract it from query params
            for (field_idx, field_name) in metadata.field_names.iter().enumerate() {
                let field_type = &metadata.field_types[field_idx];

                // Get the field name as a C string
                let field_name_ptr = self
                    .builder
                    .build_global_string_ptr(field_name, &format!("query_key_{}", field_name))
                    .unwrap()
                    .as_pointer_value();

                // Get the field type as a C string
                let field_type_ptr = self
                    .builder
                    .build_global_string_ptr(field_type, &format!("query_type_{}", field_name))
                    .unwrap()
                    .as_pointer_value();

                // Call typed extractor to get the value from request (sets RFC7807 on errors)
                let value_str_ptr = self
                    .builder
                    .build_call(
                        doohttp_extract_query_typed_fn,
                        &[
                            request_ptr.into(),
                            field_name_ptr.into(),
                            field_type_ptr.into(),
                        ],
                        &format!("query_val_{}", field_name),
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                // If extractor set an error, skip storing; wrapper will handle last_error_status
                let status_val = self
                    .builder
                    .build_call(last_error_status_fn, &[], "query_field_err_status")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();
                let has_error = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        status_val,
                        i32_type.const_int(0, false),
                        "query_field_has_error",
                    )
                    .unwrap();
                let current_fn = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();
                let err_block = self
                    .context
                    .append_basic_block(current_fn, &format!("query_field_err_{}", field_name));
                let cont_block = self
                    .context
                    .append_basic_block(current_fn, &format!("query_field_cont_{}", field_name));
                self.builder
                    .build_conditional_branch(has_error, err_block, cont_block)
                    .unwrap();

                // Error: propagate error by returning Result Err in wrapper (handled after this function)
                self.builder.position_at_end(err_block);
                // Just branch to end, wrapper will see last_error_status
                self.builder.build_unconditional_branch(cont_block).unwrap();

                self.builder.position_at_end(cont_block);

                // Get field pointer in struct
                let struct_llvm_type = *self.canonical_struct_types.get(struct_type).unwrap();
                let field_ptr = unsafe {
                    self.builder
                        .build_struct_gep(
                            struct_llvm_type,
                            struct_ptr,
                            field_idx as u32,
                            &format!("field_{}_ptr", field_name),
                        )
                        .unwrap()
                };

                // Check if value is null (not provided)
                let is_null = self
                    .builder
                    .build_is_null(value_str_ptr, "is_null")
                    .unwrap();

                let current_fn = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();
                let set_block = self
                    .context
                    .append_basic_block(current_fn, &format!("set_{}", field_name));
                let skip_block = self
                    .context
                    .append_basic_block(current_fn, &format!("skip_{}", field_name));

                self.builder
                    .build_conditional_branch(is_null, skip_block, set_block)
                    .unwrap();

                // Set block: convert and store the value
                self.builder.position_at_end(set_block);

                // Parse based on field type
                if field_type == "Int" || field_type == "I32" {
                    let int_val = self
                        .builder
                        .build_call(atoi_fn, &[value_str_ptr.into()], "int_val")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_int_value();
                    self.builder.build_store(field_ptr, int_val).unwrap();
                } else if field_type == "Str" || field_type.contains("String") {
                    // Store the string pointer directly
                    self.builder.build_store(field_ptr, value_str_ptr).unwrap();
                } else if field_type == "Bool" {
                    // Check if string is "true" or "1"
                    let true_str = self
                        .builder
                        .build_global_string_ptr("true", "true_str")
                        .unwrap()
                        .as_pointer_value();
                    let cmp_result = self
                        .builder
                        .build_call(strcmp_fn, &[value_str_ptr.into(), true_str.into()], "cmp")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_int_value();
                    let is_true = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            cmp_result,
                            i32_type.const_int(0, false),
                            "is_true",
                        )
                        .unwrap();
                    let bool_val = self
                        .builder
                        .build_int_z_extend(is_true, i32_type, "bool_val")
                        .unwrap();
                    self.builder.build_store(field_ptr, bool_val).unwrap();
                } else if field_type == "Float" || field_type == "F64" || field_type == "F32" {
                    let float_val = self
                        .builder
                        .build_call(atof_fn, &[value_str_ptr.into()], "float_val")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_float_value();
                    if field_type == "F32" {
                        let f32_type = self.context.f32_type();
                        let f32_val = self
                            .builder
                            .build_float_trunc(float_val, f32_type, "float32")
                            .unwrap();
                        self.builder.build_store(field_ptr, f32_val).unwrap();
                    } else {
                        let f64_type = self.context.f64_type();
                        if let Err(_) = self.builder.build_store(field_ptr, float_val) {
                            let casted = self
                                .builder
                                .build_float_ext(float_val, f64_type, "float64_ext")
                                .unwrap();
                            self.builder.build_store(field_ptr, casted).unwrap();
                        }
                    }
                }

                self.builder.build_unconditional_branch(skip_block).unwrap();

                // Skip block: continue to next field
                self.builder.position_at_end(skip_block);
            }
        }
    }

    /// Generate calls to register HTTP handlers at runtime
    /// Called at the start of main() to register all handler functions
    fn generate_http_handler_registration_calls(&mut self) {
        // Get or declare doo_http_register_handler function
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let void_type = self.context.void_type();

        // Declare doo_http_register_handler if not already declared
        let register_fn = self
            .module
            .get_function("doo_http_register_handler")
            .unwrap_or_else(|| {
                // Signature: void doo_http_register_handler(const char* name, DooHandlerFn handler)
                // DooHandlerFn = fn(*mut DooRequest) -> *mut DooResult
                let request_ptr_type = ptr_type;
                let result_ptr_type = ptr_type;
                let handler_fn_type = result_ptr_type.fn_type(&[request_ptr_type.into()], false);
                let handler_ptr_type = handler_fn_type.ptr_type(AddressSpace::default());

                let fn_type = void_type.fn_type(&[ptr_type.into(), handler_ptr_type.into()], false);
                self.module
                    .add_function("doo_http_register_handler", fn_type, None)
            });

        // Declare doo_http_register_middleware if not already declared
        let register_middleware_fn = self
            .module
            .get_function("doo_http_register_middleware")
            .unwrap_or_else(|| {
                // Signature: void doo_http_register_middleware(const char* name, DooMiddlewareFn middleware)
                // DooMiddlewareFn = fn(*mut DooRequest, *mut DooNext) -> *mut DooResult
                let request_ptr_type = ptr_type;
                let next_ptr_type = ptr_type;
                let result_ptr_type = ptr_type;
                let middleware_fn_type = result_ptr_type
                    .fn_type(&[request_ptr_type.into(), next_ptr_type.into()], false);
                let middleware_ptr_type = middleware_fn_type.ptr_type(AddressSpace::default());

                let fn_type =
                    void_type.fn_type(&[ptr_type.into(), middleware_ptr_type.into()], false);
                self.module
                    .add_function("doo_http_register_middleware", fn_type, None)
            });

        // Generate registration call for each handler
        for handler_name in &self.http_handlers_to_register.clone() {
            let handler_wrapper_name = format!("{}_http_wrapper", handler_name);

            // Check if this has a handler wrapper (regular handlers)
            if let Some(wrapper_func) = self.module.get_function(&handler_wrapper_name) {
                // Register as regular handler
                let name_str = format!("{}\0", handler_name);
                let name_global = self
                    .builder
                    .build_global_string_ptr(&name_str, &format!("handler_name_{}", handler_name))
                    .unwrap();
                let name_ptr = name_global.as_pointer_value();

                let wrapper_ptr = wrapper_func.as_global_value().as_pointer_value();

                // Call doo_http_register_handler(name, wrapper)
                self.builder
                    .build_call(register_fn, &[name_ptr.into(), wrapper_ptr.into()], "")
                    .unwrap();
            }
        }

        // Generate registration call for each middleware
        for middleware_name in &self.http_middleware_to_register.clone() {
            let middleware_wrapper_name = format!("{}_http_wrapper", middleware_name);

            // Get the middleware wrapper
            if let Some(wrapper_func) = self.module.get_function(&middleware_wrapper_name) {
                // Register as middleware
                let name_str = format!("{}\0", middleware_name);
                let name_global = self
                    .builder
                    .build_global_string_ptr(
                        &name_str,
                        &format!("middleware_name_{}", middleware_name),
                    )
                    .unwrap();
                let name_ptr = name_global.as_pointer_value();

                let wrapper_ptr = wrapper_func.as_global_value().as_pointer_value();

                // Call doo_http_register_middleware(name, wrapper)
                self.builder
                    .build_call(
                        register_middleware_fn,
                        &[name_ptr.into(), wrapper_ptr.into()],
                        "",
                    )
                    .unwrap();
            }
        }
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

        // If this is main(), inject HTTP handler registration at the start
        if func.name == "main" && !self.http_handlers_to_register.is_empty() {
            self.generate_http_handler_registration_calls();
        }

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
                            let metadata = crate::codegen::core::context::StructMetadata {
                                field_names: field_names.clone(),
                                field_types: field_types.clone(),
                            };
                            self.struct_metadata.insert(struct_name.clone(), metadata);

                            // Create the canonical LLVM struct type
                            let llvm_field_types: Vec<BasicTypeEnum> = field_types
                                .iter()
                                .map(|type_str| self.type_string_to_llvm_type(type_str))
                                .collect();

                            let struct_type = self.context.struct_type(&llvm_field_types, false);
                            self.canonical_struct_types
                                .insert(struct_name.clone(), struct_type);
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
