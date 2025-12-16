//! Database codegen module - FFI-only design
//! Compiler extracts metadata and calls FFI functions with metadata JSON
//! NO handler generation, NO SQL generation - all logic in FFI

use crate::codegen::core::CodeGen;
use inkwell::values::BasicValueEnum;

impl<'ctx> CodeGen<'ctx> {
    /// Generate auth routes for signup and login
    /// app.auth("/signup", "/login", User, db)
    /// Calls: doo_http_auth(server, signup_path, login_path, struct_name, metadata_json)
    pub fn generate_auth_routes(
        &mut self,
        dest: &str,
        object: &str,
        args: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.len() != 4 {
            panic!(
                "auth() requires 4 arguments: signupPath, loginPath, structName, db. Got {}",
                args.len()
            );
        }

        // Extract arguments
        let signup_path = &args[0];
        let login_path = &args[1];
        let struct_name_arg = &args[2];
        let _db_var = &args[3];

        // Extract struct name from argument
        let struct_name = self.extract_string_literal(struct_name_arg);

        // Get struct metadata
        let struct_metadata = self.struct_metadata.get(&struct_name).cloned();
        let decorators = self
            .struct_field_decorators
            .get(&struct_name)
            .cloned()
            .unwrap_or_default();

        if struct_metadata.is_none() {
            panic!("Struct '{}' not found for auth()", struct_name);
        }

        let metadata = struct_metadata.unwrap();

        // Build metadata JSON
        let metadata_json = self.build_metadata_json(&struct_name, &metadata, &decorators);

        // Call FFI: doo_http_auth(server, signup_path, login_path, struct_name, metadata_json)
        self.generate_auth_ffi_call(
            object,
            signup_path,
            login_path,
            &struct_name,
            &metadata_json,
        );

        // Return None - the result is not actually used and trying to create a Result
        // struct causes segfaults. The FFI side effect (registering routes) is what matters.
        None
    }

    /// Generate CRUD routes for a resource
    /// app.crud("/products", Product, db)
    /// Calls: doo_http_crud(server, base_path, struct_name, metadata_json)
    pub fn generate_crud_routes(
        &mut self,
        dest: &str,
        object: &str,
        args: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.len() != 3 {
            panic!(
                "crud() requires 3 arguments: basePath, structName, db. Got {}",
                args.len()
            );
        }

        // Extract arguments
        let base_path = &args[0];
        let struct_name_arg = &args[1];
        let _db_var = &args[2];

        // Extract struct name
        let struct_name = self.extract_string_literal(struct_name_arg);

        // Get struct metadata
        let struct_metadata = self.struct_metadata.get(&struct_name).cloned();
        let decorators = self
            .struct_field_decorators
            .get(&struct_name)
            .cloned()
            .unwrap_or_default();

        if struct_metadata.is_none() {
            panic!("Struct '{}' not found for crud()", struct_name);
        }

        let metadata = struct_metadata.unwrap();

        // Extract actual path string for noAuth detection
        let base_path_str = self.extract_string_literal(base_path);

        // Build metadata JSON with noAuth flag based on path
        let metadata_json = self.build_metadata_json_with_path(&struct_name, &metadata, &decorators, &base_path_str);

        // Call FFI: doo_http_crud(server, base_path, struct_name, metadata_json)
        self.generate_crud_ffi_call(object, base_path, &struct_name, &metadata_json);

        // Return None - the result is not actually used and trying to create a Result
        // struct causes segfaults. The FFI side effect (registering routes) is what matters.
        None
    }

    /// Extract string literal from argument
    fn extract_string_literal(&self, arg: &str) -> String {
        // Check temp_strings first
        if let Some(s) = self.temp_strings.get(arg) {
            return s.clone();
        }

        // If it's a plain string without %, return as-is
        if !arg.starts_with('%') {
            return arg.to_string();
        }

        // Fallback
        arg.to_string()
    }

    /// Build metadata JSON for struct
    fn build_metadata_json(
        &self,
        struct_name: &str,
        metadata: &crate::codegen::core::StructMetadata,
        decorators: &std::collections::HashMap<String, Vec<(String, Vec<String>)>>,
    ) -> String {
        let mut fields = Vec::new();

        for (i, field_name) in metadata.field_names.iter().enumerate() {
            let field_type = &metadata.field_types[i];
            let field_decorators = decorators.get(field_name).cloned().unwrap_or_default();

            let mut decorator_array = Vec::new();
            for (dec_name, dec_args) in field_decorators {
                let args_json: Vec<String> = dec_args
                    .iter()
                    .map(|arg| format!("\"{}\"", arg.replace("\"", "\\\"")))
                    .collect();
                decorator_array.push(format!(
                    "{{\"name\":\"{}\",\"args\":[{}]}}",
                    dec_name,
                    args_json.join(",")
                ));
            }

            fields.push(format!(
                "{{\"name\":\"{}\",\"type\":\"{}\",\"decorators\":[{}]}}",
                field_name,
                field_type,
                decorator_array.join(",")
            ));
        }

        format!("{{\"fields\":[{}]}}", fields.join(","))
    }

    /// Build metadata JSON with noAuth flag for CRUD routes
    fn build_metadata_json_with_path(
        &self,
        struct_name: &str,
        metadata: &crate::codegen::core::StructMetadata,
        decorators: &std::collections::HashMap<String, Vec<(String, Vec<String>)>>,
        base_path: &str,
    ) -> String {
        let mut fields = Vec::new();

        for (i, field_name) in metadata.field_names.iter().enumerate() {
            let field_type = &metadata.field_types[i];
            let field_decorators = decorators.get(field_name).cloned().unwrap_or_default();

            let mut decorator_array = Vec::new();
            for (dec_name, dec_args) in field_decorators {
                let args_json: Vec<String> = dec_args
                    .iter()
                    .map(|arg| format!("\"{}\"", arg.replace("\"", "\\\"")))
                    .collect();
                decorator_array.push(format!(
                    "{{\"name\":\"{}\",\"args\":[{}]}}",
                    dec_name,
                    args_json.join(",")
                ));
            }

            fields.push(format!(
                "{{\"name\":\"{}\",\"type\":\"{}\",\"decorators\":[{}]}}",
                field_name,
                field_type,
                decorator_array.join(",")
            ));
        }

        // Check if path contains "/public/" to determine noAuth
        let no_auth = base_path.contains("/public/");
        
        format!("{{\"fields\":[{}],\"noAuth\":{}}}", fields.join(","), no_auth)
    }

    /// Generate FFI call to doo_http_auth
    fn generate_auth_ffi_call(
        &mut self,
        server_object: &str,
        signup_path: &str,
        login_path: &str,
        struct_name: &str,
        metadata_json: &str,
    ) {
        use inkwell::AddressSpace;

        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Declare doo_http_auth_impl FFI function
        let auth_fn = self
            .module
            .get_function("doo_http_auth_impl")
            .unwrap_or_else(|| {
                let fn_type = ptr_type.fn_type(
                    &[
                        ptr_type.into(), // server
                        ptr_type.into(), // signup_path
                        ptr_type.into(), // login_path
                        ptr_type.into(), // struct_name
                        ptr_type.into(), // metadata_json
                    ],
                    false,
                );
                self.module
                    .add_function("doo_http_auth_impl", fn_type, None)
            });

        // Get server pointer
        let server_ptr = self.resolve_value(server_object).into_pointer_value();

        // Resolve signup_path and login_path
        let signup_path_val = self.resolve_value(signup_path);
        let login_path_val = self.resolve_value(login_path);

        let signup_path_ptr = if signup_path_val.is_pointer_value() {
            signup_path_val.into_pointer_value()
        } else {
            self.builder
                .build_global_string_ptr(signup_path, "auth_signup_path")
                .unwrap()
                .as_pointer_value()
        };

        let login_path_ptr = if login_path_val.is_pointer_value() {
            login_path_val.into_pointer_value()
        } else {
            self.builder
                .build_global_string_ptr(login_path, "auth_login_path")
                .unwrap()
                .as_pointer_value()
        };

        // Create string constants
        let struct_name_str = self
            .builder
            .build_global_string_ptr(struct_name, "auth_struct_name")
            .unwrap();
        let metadata_str = self
            .builder
            .build_global_string_ptr(metadata_json, "auth_metadata")
            .unwrap();

        // Call FFI - ignore result (memory leak but avoids crash)
        self.builder
            .build_call(
                auth_fn,
                &[
                    server_ptr.into(),
                    signup_path_ptr.into(),
                    login_path_ptr.into(),
                    struct_name_str.as_pointer_value().into(),
                    metadata_str.as_pointer_value().into(),
                ],
                "auth_call",
            )
            .unwrap();
    }

    /// Generate FFI call to doo_http_crud
    fn generate_crud_ffi_call(
        &mut self,
        server_object: &str,
        base_path: &str,
        struct_name: &str,
        metadata_json: &str,
    ) {
        use inkwell::AddressSpace;

        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Declare doo_http_crud_impl FFI function
        let crud_fn = self
            .module
            .get_function("doo_http_crud_impl")
            .unwrap_or_else(|| {
                let fn_type = ptr_type.fn_type(
                    &[
                        ptr_type.into(), // server
                        ptr_type.into(), // base_path
                        ptr_type.into(), // struct_name
                        ptr_type.into(), // metadata_json
                    ],
                    false,
                );
                self.module
                    .add_function("doo_http_crud_impl", fn_type, None)
            });

        // Get server pointer
        let server_ptr = self.resolve_value(server_object).into_pointer_value();

        // Resolve base_path
        let base_path_val = self.resolve_value(base_path);
        let base_path_ptr = if base_path_val.is_pointer_value() {
            base_path_val.into_pointer_value()
        } else {
            self.builder
                .build_global_string_ptr(base_path, "crud_base_path")
                .unwrap()
                .as_pointer_value()
        };

        // Create string constants
        let struct_name_str = self
            .builder
            .build_global_string_ptr(struct_name, "crud_struct_name")
            .unwrap();
        let metadata_str = self
            .builder
            .build_global_string_ptr(metadata_json, "crud_metadata")
            .unwrap();

        // Call FFI - ignore result (memory leak but avoids crash)
        self.builder
            .build_call(
                crud_fn,
                &[
                    server_ptr.into(),
                    base_path_ptr.into(),
                    struct_name_str.as_pointer_value().into(),
                    metadata_str.as_pointer_value().into(),
                ],
                "crud_call",
            )
            .unwrap();
    }
}
