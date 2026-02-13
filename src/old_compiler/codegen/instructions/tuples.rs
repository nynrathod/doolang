use inkwell::context::Context;
use inkwell::types::{BasicTypeEnum, StructType};
use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;
use std::collections::HashMap;

/// Helper for managing tuple struct types for multi-value returns
pub struct TupleStructManager<'ctx> {
    context: &'ctx Context,
    cache: HashMap<String, StructType<'ctx>>,
}

impl<'ctx> TupleStructManager<'ctx> {
    /// Create a new tuple struct manager
    pub fn new(context: &'ctx Context) -> Self {
        TupleStructManager {
            context,
            cache: HashMap::new(),
        }
    }

    /// Get or create a tuple struct type for the given tuple type string
    /// Format: "Tuple(Int,Str,Float)" -> struct { i32, ptr, f64 }
    pub fn get_or_create_tuple_struct(&mut self, tuple_type_str: &str) -> StructType<'ctx> {
        // Check cache first
        if let Some(cached_type) = self.cache.get(tuple_type_str) {
            return *cached_type;
        }

        // Extract types from Tuple(Type1,Type2,...)
        let inner = tuple_type_str
            .strip_prefix("Tuple(")
            .and_then(|s| s.strip_suffix(")"))
            .unwrap_or("");

        let types: Vec<&str> = inner.split(',').collect();
        let mut llvm_types: Vec<BasicTypeEnum> = Vec::new();

        for type_str in types {
            let llvm_type = self.map_type_to_llvm(type_str);
            llvm_types.push(llvm_type);
        }

        // Create unique struct name based on tuple type
        let struct_name = format!(
            "tuple_{}",
            tuple_type_str
                .replace("Tuple(", "")
                .replace(")", "")
                .replace(",", "_")
        );

        let struct_type = self.context.opaque_struct_type(&struct_name);
        struct_type.set_body(&llvm_types, false);

        // Cache the type
        self.cache.insert(tuple_type_str.to_string(), struct_type);

        struct_type
    }

    /// Map a simple type string to LLVM type
    fn map_type_to_llvm(&self, type_str: &str) -> BasicTypeEnum<'ctx> {
        let trimmed = type_str.trim();
        if trimmed.contains("Str") {
            self.context.ptr_type(AddressSpace::default()).into()
        } else if trimmed.contains("Array") {
            self.context.ptr_type(AddressSpace::default()).into()
        } else if trimmed.contains("Map") {
            self.context.ptr_type(AddressSpace::default()).into()
        } else if trimmed.contains("Float") {
            self.context.f64_type().into()
        } else if trimmed.contains("Bool") {
            self.context.bool_type().into()
        } else {
            self.context.i32_type().into()
        }
    }
}
