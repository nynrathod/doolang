use crate::limits::CODEGEN_MAX_DEPTH;
use inkwell::{
    builder::Builder,
    context::Context,
    module::Module,
    passes::PassManager,
    types::BasicTypeEnum,
    values::{BasicValueEnum, FunctionValue, PointerValue},
};
use std::collections::HashMap;

/// Represents a variable allocated on the stack or in global memory.
/// Stores the variable's pointer and its LLVM type.
#[derive(Debug)]
pub struct Symbol<'ctx> {
    pub ptr: PointerValue<'ctx>,
    pub ty: BasicTypeEnum<'ctx>,
}

/// Metadata for tracking array information
#[derive(Debug, Clone)]
pub struct ArrayMetadata {
    pub length: usize,
    pub element_type: String, // "Int", "Str", etc.
    pub contains_strings: bool,
}

/// Metadata for tracking map information
#[derive(Debug, Clone)]
pub struct MapMetadata {
    pub length: usize,
    pub key_type: String,
    pub value_type: String,
    pub key_is_string: bool,
    pub value_is_string: bool,
    pub key_needs_rc: bool,
    pub value_needs_rc: bool,
}

/// Loop type enumeration
#[derive(Debug, Clone, PartialEq)]
pub enum LoopType {
    Range,
    Array {
        item_var: String,
        array_var: String,
    },
    Map {
        key_var: String,
        value_var: String,
        map_var: String,
    },
    Infinite,
}

/// Loop context for tracking nested loops
#[derive(Debug, Clone)]
pub struct LoopContext {
    pub exit_block: String,
    pub continue_block: String,
    pub loop_vars: Vec<String>,
    pub loop_type: Option<LoopType>,
}

/// The main context structure for generating LLVM Intermediate Representation (IR).
/// It holds all the necessary LLVM components and symbol tables.
pub struct CodeGen<'ctx> {
    pub context: &'ctx Context,
    pub module: Module<'ctx>, // The container for all generated code (globals, functions, types)
    pub builder: Builder<'ctx>, // The tool used to insert instructions into blocks
    pub fpm: PassManager<FunctionValue<'ctx>>, // Function Pass Manager for optimization (e.g., dead code elimination)
    pub symbols: HashMap<String, Symbol<'ctx>>, // Symbol table for local variables (maps names to stack pointers)
    pub temp_values: HashMap<String, BasicValueEnum<'ctx>>, // Stores temporary constant values (used for building complex constants)
    pub globals: Vec<crate::mir::mir::MirInstr>, // List of Intermediate Representation instructions for global definitions
    pub temp_strings: HashMap<String, String>, // Stores original Rust string values (used during string concatenation/definition)
    pub strings_to_concat: std::collections::HashSet<String>, // Tracks strings that need concatenation logic

    // NEW: RC runtime functions
    pub incref_fn: Option<FunctionValue<'ctx>>,
    pub decref_fn: Option<FunctionValue<'ctx>>,

    pub heap_strings: std::collections::HashSet<String>,

    pub heap_arrays: std::collections::HashSet<String>,
    pub heap_maps: std::collections::HashSet<String>,

    pub composite_strings: HashMap<String, Vec<String>>,
    pub composite_string_ptrs: HashMap<String, Vec<BasicValueEnum<'ctx>>>,

    pub array_metadata: HashMap<String, ArrayMetadata>,
    pub map_metadata: HashMap<String, MapMetadata>,
    pub loop_stack: Vec<LoopContext>,
    pub loop_local_vars: std::collections::HashSet<String>, // Track variables allocated inside loop bodies (must not be cleaned up at function level)
    pub arrayget_sources: HashMap<String, String>, // Maps ArrayGet result names to their source array names
    pub current_function_params: Vec<(String, Option<String>)>, // Track current function parameters (name, type) for RC on return
    pub function_return_types: HashMap<String, String>, // Track function return types for proper RC handling on call results
    pub functions_returning_heap: std::collections::HashSet<String>, // Track functions that return heap-allocated values

    pub boolean_temps: std::collections::HashSet<String>, // Track temporary variables from boolean-returning methods
    pub variable_types: HashMap<String, String>, // Track variable types for typeOf function

    pub declared_functions: std::collections::HashSet<String>,
    pub external_modules: HashMap<String, Vec<String>>,
    pub function_aliases: HashMap<String, String>, // Maps alias names to original function names
    pub recursion_depth: usize, // Track recursion depth to prevent stack overflow
}

impl<'ctx> CodeGen<'ctx> {
    /// Check if we've exceeded maximum recursion depth
    pub fn check_depth(&self) -> Result<(), String> {
        if self.recursion_depth >= CODEGEN_MAX_DEPTH {
            return Err("Expression too deeply nested (recursion limit exceeded)".to_string());
        }
        Ok(())
    }
    /// Creates a new CodeGen instance, initializing LLVM structures.
    pub fn new(module_name: &str, context: &'ctx Context) -> Self {
        let module = context.create_module(module_name);
        let builder = context.create_builder();
        let fpm: PassManager<FunctionValue> = PassManager::create(&module);

        Self {
            context,
            module,
            builder,
            fpm,
            symbols: HashMap::new(),
            temp_values: HashMap::new(),
            globals: Vec::new(),
            temp_strings: HashMap::new(),
            strings_to_concat: std::collections::HashSet::new(),

            incref_fn: None,
            decref_fn: None,

            heap_strings: std::collections::HashSet::new(),
            heap_arrays: std::collections::HashSet::new(),
            heap_maps: std::collections::HashSet::new(),

            composite_strings: HashMap::new(),
            composite_string_ptrs: HashMap::new(),

            array_metadata: HashMap::new(),
            map_metadata: HashMap::new(),
            loop_stack: Vec::new(),
            loop_local_vars: std::collections::HashSet::new(),
            arrayget_sources: HashMap::new(),
            current_function_params: Vec::new(),
            function_return_types: HashMap::new(),
            functions_returning_heap: std::collections::HashSet::new(),

            boolean_temps: std::collections::HashSet::new(),
            variable_types: HashMap::new(),

            declared_functions: std::collections::HashSet::new(),
            external_modules: HashMap::new(),
            function_aliases: HashMap::new(),
            recursion_depth: 0,
        }
    }

    /// Prints the final generated LLVM IR to standard error (stderr).
    pub fn dump(&self) {
        self.module.print_to_stderr();
    }

    /// Enter a new loop context
    pub fn enter_loop(&mut self, exit_block: String, continue_block: String) {
        self.enter_loop_with_type(exit_block, continue_block, None);
    }

    /// Enter a new loop context with type information
    pub fn enter_loop_with_type(
        &mut self,
        exit_block: String,
        continue_block: String,
        loop_type: Option<LoopType>,
    ) {
        self.loop_stack.push(LoopContext {
            exit_block,
            continue_block,
            loop_vars: Vec::new(),
            loop_type,
        });
    }

    /// Exit current loop context and return it
    pub fn exit_loop(&mut self) -> Option<LoopContext> {
        self.loop_stack.pop()
    }

    /// Add a variable to current loop's cleanup list
    pub fn add_loop_var(&mut self, var: String) {
        if let Some(ctx) = self.loop_stack.last_mut() {
            ctx.loop_vars.push(var);
        }
    }

    /// Check if a variable is a loop iteration variable in any active loop
    pub fn is_loop_var(&self, var: &str) -> bool {
        let var_base = var.trim_start_matches('%').trim_end_matches("_array");
        for loop_ctx in &self.loop_stack {
            for loop_var in &loop_ctx.loop_vars {
                let loop_var_base = loop_var.trim_start_matches('%').trim_end_matches("_array");
                if loop_var_base == var_base {
                    return true;
                }
            }
        }
        false
    }
}
