/// Mid-level Intermediate Representation for the language
/// Contains the core data structures used after AST parsing
/// and before LLVM IR generation
use crate::parser::ast::{AstNode, TypeNode};
use std::collections::{HashMap, HashSet};

/// Represents a complete MIR program with functions and globals
#[derive(Debug, Clone)]
pub struct MirProgram {
    pub functions: Vec<MirFunction>, // All function definitions
    pub globals: Vec<MirInstr>,      // Global variable initializations
    pub is_main_entry: bool,         // Whether this is the main entry point file (requires main())
    pub enum_table: HashMap<String, HashMap<String, Option<TypeNode>>>, // Enum definitions: name -> variant -> payload type
    pub struct_table: HashMap<String, HashMap<String, TypeNode>>, // Struct definitions: name -> field -> type
    pub enum_variant_order: HashMap<String, Vec<(String, Option<TypeNode>)>>, // Ordered enum variants: enum_name -> [(variant_name, payload_type)]
}

impl MirProgram {
    /// Validate all functions to ensure temporaries are defined before use
    pub fn validate(&self) -> Result<(), String> {
        for func in &self.functions {
            if let Err(e) = func.validate() {
                return Err(format!(
                    "MIR validation failed in function '{}': {}",
                    func.name, e
                ));
            }
        }
        Ok(())
    }
}

/// A single function in MIR form
#[derive(Debug, Clone)]
pub struct MirFunction {
    pub name: String, // Function identifier
    pub params: Vec<String>,
    pub param_types: Vec<Option<String>>, // Parameter types (e.g., "Int", "Str", "Array", "Map")
    pub return_type: Option<String>,
    pub error_type: Option<String>, // Error type for functions that can fail
    pub blocks: Vec<MirBlock>,
    pub ffi_lib: Option<String>, // FFI library name from @ffi("libname")
    pub ffi_symbol: Option<String>, // FFI symbol name from @extern("symbol_name")
}

impl MirFunction {
    /// Validate that all temporaries are defined before use
    pub fn validate(&self) -> Result<(), String> {
        let mut defined: HashSet<String> = HashSet::new();

        // Add function parameters as defined
        for param in &self.params {
            defined.insert(param.clone());
        }

        // Process each block in order
        for block in &self.blocks {
            // Process instructions
            for instr in &block.instrs {
                // Check used values are defined
                for used in instr.get_used_values() {
                    if used.starts_with('%') && !defined.contains(&used) {
                        eprintln!("MIR validation error in function '{}':", self.name);
                        eprintln!("Block '{}': Undefined temporary '{}'", block.label, used);
                        eprintln!("Instruction: {:?}", instr);
                        eprintln!("\nDefined temporaries so far: {:?}", defined);
                        eprintln!("\nAll blocks:");
                        for b in &self.blocks {
                            eprintln!("  Block '{}': {} instrs", b.label, b.instrs.len());
                            for (i, inst) in b.instrs.iter().enumerate() {
                                eprintln!("    [{}] {:?}", i, inst);
                            }
                        }
                        return Err(format!(
                            "Undefined temporary '{}' in block '{}'",
                            used, block.label
                        ));
                    }
                }

                // Add defined values
                if let Some(def) = instr.get_defined_value() {
                    defined.insert(def.clone());
                    // MapGetPair also defines {name}_k and {name}_v temporaries
                    if matches!(instr, MirInstr::MapGetPair { .. }) {
                        defined.insert(format!("{}_k", def));
                        defined.insert(format!("{}_v", def));
                    }
                }
            }

            // Check terminator
            if let Some(term) = &block.terminator {
                for used in term.get_used_values() {
                    if used.starts_with('%') && !defined.contains(&used) {
                        return Err(format!(
                            "Undefined temporary '{}' in terminator of block '{}'",
                            used, block.label
                        ));
                    }
                }
            }
        }

        Ok(())
    }
}

/// A basic block - sequence of instructions with single entry/exit
#[derive(Debug, Clone)]
pub struct MirBlock {
    pub label: String,                // Block identifier
    pub instrs: Vec<MirInstr>,        // Sequential instructions
    pub terminator: Option<MirInstr>, // Block terminator (jump/return)
}

pub struct CodegenBlock<'a> {
    pub label: &'a str,
    pub instrs: &'a [MirInstr],
    pub terminator: Option<MirTerminator>, // use real terminator here
}

/// MIR instruction types - covers all operations in the language
#[derive(Debug, Clone)]
pub enum MirInstr {
    // Reference counting operations
    IncRef {
        value: String, // temp/variable to increment
    },
    DecRef {
        value: String, // temp/variable to decrement
    },

    // Basic constants
    ConstInt {
        name: String,
        value: i32,
    },
    ConstFloat {
        name: String,
        value: f64,
    },
    ConstBool {
        name: String,
        value: bool,
    },
    ConstString {
        name: String,
        value: String,
    },

    // Collections
    Array {
        name: String,
        elements: Vec<String>,
        element_type: Option<String>, // "Int", "Float", "Bool", "Str" - helps distinguish [1] from [true]
    },
    Map {
        name: String,
        entries: Vec<(String, String)>,
        key_type: Option<String>,
        value_type: Option<String>,
    },

    // Type casting
    Cast {
        name: String,
        value: String,
        source_type: String, // "Int", "Float", "String", "Bool"
        target_type: String, // "Int", "Float", "String", "Bool"
    },

    // Range operations
    RangeCreate {
        name: String,
        start: String,
        end: String,
        inclusive: bool,
    },

    // Collection operations
    // Get and Set - read and write value
    ArrayLen {
        name: String,
        array: String,
    },
    ArrayGet {
        name: String,
        array: String,
        index: String,
    },
    ArraySlice {
        name: String,
        array: String,
        start: String,
        end: String,
        inclusive: bool,
    },
    ArraySet {
        array: String,
        index: String,
        value: String,
    },
    MapLen {
        name: String,
        map: String,
    },
    MapGet {
        name: String,
        map: String,
        key: String,
    },
    MapGetPair {
        name: String,
        map: String,
        index: String,
    },
    MapSet {
        map: String,
        key: String,
        value: String,
    },
    MapContains {
        name: String,
        map: String,
        key: String,
    },

    // Arithmetic operations
    Add(String, String, String), // (dest, lhs, rhs)
    Sub(String, String, String),
    Mul(String, String, String),
    Div(String, String, String),

    // Generic binary operations (covers arithmetic and comparisons)
    BinaryOp(String, String, String, String), // (op, dest, lhs, rhs)
    StringConcat {
        name: String,
        left: String,
        right: String,
    },

    // Assignment and variable operations
    Assign {
        name: String,
        value: String,
        mutable: bool,
    },
    IncrementDecrement {
        variable: String,
        op: String, // "++", "--"
    },

    // Tuple operations
    TupleCreate {
        name: String,
        elements: Vec<String>,
    },
    TupleExtract {
        name: String,
        source: String,
        index: usize,
    },
    TupleGet {
        name: String,
        tuple: String,
        index: usize,
    },

    // Function related
    Arg {
        name: String,
    },
    Call {
        dest: Vec<String>, // multiple temps for tuple destructuring
        func: String,      // function name
        args: Vec<String>, // arguments (as temp names)
    },
    MethodCall {
        dest: String,
        object: String,
        method: String,
        args: Vec<String>,
    },
    Closure {
        name: String,
        params: Vec<String>,
        param_types: Vec<Option<String>>,
        body_expr: String, // result temp name from evaluating closure body
        body_ast: Option<Box<AstNode>>, // Store AST for proper codegen
        return_type: Option<String>,
        captures: Vec<String>, // captured variables from outer scope
    },
    Return {
        values: Vec<String>,
    },

    // Control flow
    Jump {
        label: String,
    },
    CondJump {
        cond: String,
        then_block: String,
        else_block: String,
    },

    // I/O operations
    Print {
        values: Vec<String>,
    },

    // Struct and enum operations
    StructDecl {
        struct_name: String,
        field_names: Vec<String>,
        field_types: Vec<String>, // Type strings like "Int", "Str", "Struct(Point)", etc.
    },
    StructInit {
        name: String,
        struct_name: String,
        fields: Vec<(String, String)>,
    },
    StructGet {
        name: String,
        struct_instance: String,
        field: String,
    },
    StructSet {
        struct_instance: String,
        field: String,
        value: String,
    },

    EnumInit {
        name: String,
        enum_name: String,
        variant: String,
        variant_index: u32,
        value: Option<String>,
    },
    EnumMatch {
        name: String,
        enum_instance: String,
        variant: String,
    },

    /// Extract the tag from an enum for comparison
    EnumGetTag {
        name: String,       // Destination for the tag value
        enum_value: String, // Source enum value
    },

    /// Extract the payload from an enum variant
    EnumGetPayload {
        name: String,                   // Destination for the payload value
        enum_value: String,             // Source enum value
        enum_name: String,              // Enum type name (e.g., "HttpCode")
        variant: String,                // Variant name (e.g., "Success")
        payload_type: Option<TypeNode>, // Expected payload type
    },

    /// Range-based for loop: for i in 0..10 or for i in 0..=10
    ForRange {
        var: String,        // Loop variable (e.g., "i")
        start: String,      // Start value (temp or literal)
        end: String,        // End value (temp or literal)
        inclusive: bool,    // true for ..=, false for ..
        cond_block: String, // Label of condition check block
        body_block: String, // Label of loop body block
        exit_block: String, // Label of block after loop
    },

    /// Array iteration: for item in arr
    ForArray {
        var: String,        // Item variable name
        array: String,      // Array variable name
        index_var: String,  // Internal index counter variable
        cond_block: String, // Label of condition check block
        body_block: String, // Label of loop body block
        exit_block: String, // Label of block after loop
    },

    /// Map iteration: for (key, value) in map
    ForMap {
        key_var: String,    // Key variable name
        value_var: String,  // Value variable name
        map: String,        // Map variable name
        index_var: String,  // Internal index counter
        cond_block: String, // Label of condition check block
        body_block: String, // Label of loop body block
        exit_block: String, // Label of block after loop
    },

    /// Infinite loop: for { }
    ForInfinite {
        body_block: String, // Label of loop body block
    },

    /// Break statement - exits current loop
    Break {
        target: String, // Exit block label
    },

    /// Continue statement - jumps to next iteration
    Continue {
        target: String, // Condition/increment block label
    },

    /// Marker instruction to indicate a block is a loop body
    /// This helps codegen know to add increment logic
    LoopBodyMarker {
        var: String,             // Variable to increment (for range loops)
        cond_block: String,      // Block to jump back to for condition check
        increment_block: String, // Optional explicit increment block
    },

    /// Load element from array during iteration
    LoadArrayElement {
        dest: String,  // Destination variable
        array: String, // Source array
        index: String, // Index variable
    },

    /// Load key-value pair from map during iteration
    LoadMapPair {
        key_dest: String, // Destination for key
        val_dest: String, // Destination for value
        map: String,      // Source map
        index: String,    // Index variable
    },

    ArrayLoopMarker {
        array: String,
        index: String,
        item: String,
        cond_block: String,
    },

    MapLoopMarker {
        map: String,
        index: String,
        key: String,
        value: String,
        cond_block: String,
    },

    // Error handling instructions
    /// Create an Ok result value
    ResultOk {
        name: String,
        values: Vec<String>, // Success values
    },

    /// Create an Err result value
    ResultErr {
        name: String,
        error: String, // Error value
    },

    /// Check if result is Ok or Err and branch
    ResultCheck {
        result: String,
        is_ok_dest: String, // Temp to store boolean: true if Ok, false if Err
    },

    /// Extract value from Ok result
    ResultUnwrapOk {
        name: String,
        result: String,
    },

    /// Extract error from Err result
    ResultUnwrapErr {
        name: String,
        result: String,
    },

    /// Propagate error if Err, otherwise continue with Ok value
    /// This is the ? operator
    TryPropagate {
        name: String,        // Destination for Ok value
        result: String,      // Result to check
        error_block: String, // Block to jump to if Err
    },

    /// Unwrap Result or panic with message
    /// expr ?? panic("message")
    UnwrapOrPanic {
        name: String,      // Destination for Ok value
        result: String,    // Result to check
        panic_msg: String, // Panic message if Err
    },

    /// Extracts Ok values and error into separate variables
    /// let a, b , err = expr;
    ManualErrorExtract {
        ok_names: Vec<String>, // Names for Ok values (single or tuple)
        error_name: String,    // Name for error variable (or "_" to ignore)
        result: String,        // Result to extract from
    },
}

/// MIR Terminators - special instructions that end a basic block
#[derive(Debug, Clone)]
pub enum MirTerminator {
    /// Return from function
    Return {
        values: Vec<String>, // return values
    },

    /// Unconditional jump to another block
    Jump {
        target: String, // block label
    },

    /// Conditional jump
    CondJump {
        cond: String,       // condition variable/temp
        then_block: String, // jump if true
        else_block: String, // jump if false
    },
}

impl MirInstr {
    pub fn as_string(&self) -> Option<&String> {
        match self {
            MirInstr::ConstString { value, .. } => Some(value),
            _ => None,
        }
    }

    /// Get all values/temporaries used by this instruction
    pub fn get_used_values(&self) -> Vec<String> {
        let mut used = Vec::new();
        match self {
            MirInstr::IncRef { value } | MirInstr::DecRef { value } => {
                used.push(value.clone());
            }
            MirInstr::Assign { value, .. } => {
                used.push(value.clone());
            }
            MirInstr::Call { args, .. } => {
                used.extend(args.clone());
            }
            MirInstr::Return { values } => {
                used.extend(values.clone());
            }
            MirInstr::CondJump { cond, .. } => {
                used.push(cond.clone());
            }
            MirInstr::Print { values } => {
                used.extend(values.clone());
            }
            MirInstr::Add(_, lhs, rhs)
            | MirInstr::Sub(_, lhs, rhs)
            | MirInstr::Mul(_, lhs, rhs)
            | MirInstr::Div(_, lhs, rhs) => {
                used.push(lhs.clone());
                used.push(rhs.clone());
            }
            MirInstr::BinaryOp(_, _, lhs, rhs) => {
                used.push(lhs.clone());
                used.push(rhs.clone());
            }
            MirInstr::StringConcat { left, right, .. } => {
                used.push(left.clone());
                used.push(right.clone());
            }
            MirInstr::IncrementDecrement { variable, .. } => {
                used.push(variable.clone());
            }
            MirInstr::TupleCreate { elements, .. } => {
                used.extend(elements.clone());
            }
            MirInstr::TupleExtract { source, .. } => {
                used.push(source.clone());
            }
            MirInstr::TupleGet { tuple, .. } => {
                used.push(tuple.clone());
            }
            MirInstr::Array { elements, .. } => {
                used.extend(elements.clone());
            }
            MirInstr::ArrayGet { array, index, .. } => {
                used.push(array.clone());
                used.push(index.clone());
            }
            MirInstr::ArraySet {
                array,
                index,
                value,
            } => {
                used.push(array.clone());
                used.push(index.clone());
                used.push(value.clone());
            }
            MirInstr::ArraySlice {
                array, start, end, ..
            } => {
                used.push(array.clone());
                used.push(start.clone());
                used.push(end.clone());
            }
            MirInstr::ArrayLen { array, .. } => {
                used.push(array.clone());
            }
            MirInstr::Map { entries, .. } => {
                for (k, v) in entries {
                    used.push(k.clone());
                    used.push(v.clone());
                }
            }
            MirInstr::MapGet { map, key, .. } => {
                used.push(map.clone());
                used.push(key.clone());
            }
            MirInstr::MapGetPair { map, index, .. } => {
                used.push(map.clone());
                used.push(index.clone());
            }
            MirInstr::MapSet { map, key, value } => {
                used.push(map.clone());
                used.push(key.clone());
                used.push(value.clone());
            }
            MirInstr::MapContains { map, key, .. } => {
                used.push(map.clone());
                used.push(key.clone());
            }
            MirInstr::MapLen { map, .. } => {
                used.push(map.clone());
            }
            MirInstr::StructInit { fields, .. } => {
                for (_, v) in fields {
                    used.push(v.clone());
                }
            }
            MirInstr::StructGet {
                struct_instance, ..
            } => {
                used.push(struct_instance.clone());
            }
            MirInstr::StructSet {
                struct_instance,
                value,
                ..
            } => {
                used.push(struct_instance.clone());
                used.push(value.clone());
            }
            MirInstr::EnumInit { value: Some(v), .. } => {
                used.push(v.clone());
            }
            MirInstr::EnumGetTag { enum_value, .. } => {
                used.push(enum_value.clone());
            }
            MirInstr::EnumGetPayload { enum_value, .. } => {
                used.push(enum_value.clone());
            }
            MirInstr::EnumMatch { enum_instance, .. } => {
                used.push(enum_instance.clone());
            }
            MirInstr::Cast { value, .. } => {
                used.push(value.clone());
            }
            MirInstr::TryPropagate { result, .. } => {
                used.push(result.clone());
            }
            MirInstr::UnwrapOrPanic {
                result, panic_msg, ..
            } => {
                used.push(result.clone());
                used.push(panic_msg.clone());
            }
            MirInstr::RangeCreate { start, end, .. } => {
                used.push(start.clone());
                used.push(end.clone());
            }
            MirInstr::MethodCall { object, args, .. } => {
                used.push(object.clone());
                used.extend(args.clone());
            }
            MirInstr::Closure {
                body_expr,
                captures,
                ..
            } => {
                used.push(body_expr.clone());
                used.extend(captures.clone());
            }
            MirInstr::ResultOk { values, .. } => {
                used.extend(values.clone());
            }
            MirInstr::ResultErr { error, .. } => {
                used.push(error.clone());
            }
            _ => {}
        }
        used
    }

    /// Get the value/temporary defined by this instruction
    pub fn get_defined_value(&self) -> Option<String> {
        match self {
            MirInstr::Assign { name, .. }
            | MirInstr::Arg { name }
            | MirInstr::Add(name, _, _)
            | MirInstr::Sub(name, _, _)
            | MirInstr::Mul(name, _, _)
            | MirInstr::Div(name, _, _)
            | MirInstr::BinaryOp(_, name, _, _)
            | MirInstr::StringConcat { name, .. }
            | MirInstr::TupleCreate { name, .. }
            | MirInstr::TupleExtract { name, .. }
            | MirInstr::TupleGet { name, .. }
            | MirInstr::Array { name, .. }
            | MirInstr::ArrayGet { name, .. }
            | MirInstr::ArraySlice { name, .. }
            | MirInstr::ArrayLen { name, .. }
            | MirInstr::Map { name, .. }
            | MirInstr::MapGet { name, .. }
            | MirInstr::MapGetPair { name, .. }
            | MirInstr::MapContains { name, .. }
            | MirInstr::MapLen { name, .. }
            | MirInstr::StructInit { name, .. }
            | MirInstr::StructGet { name, .. }
            | MirInstr::EnumInit { name, .. }
            | MirInstr::EnumGetTag { name, .. }
            | MirInstr::EnumGetPayload { name, .. }
            | MirInstr::EnumMatch { name, .. }
            | MirInstr::Cast { name, .. }
            | MirInstr::TryPropagate { name, .. }
            | MirInstr::UnwrapOrPanic { name, .. }
            | MirInstr::RangeCreate { name, .. }
            | MirInstr::MethodCall { dest: name, .. }
            | MirInstr::Closure { name, .. }
            | MirInstr::ResultOk { name, .. }
            | MirInstr::ResultErr { name, .. }
            | MirInstr::ConstInt { name, .. }
            | MirInstr::ConstFloat { name, .. }
            | MirInstr::ConstString { name, .. }
            | MirInstr::ConstBool { name, .. } => Some(name.clone()),
            MirInstr::Call { dest, .. } => {
                // Call can define multiple values, return the first one if any
                dest.first().cloned()
            }
            _ => None,
        }
    }
}
