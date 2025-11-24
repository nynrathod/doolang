/// Mid-level Intermediate Representation for the language
/// Contains the core data structures used after AST parsing
/// and before LLVM IR generation
use crate::parser::ast::AstNode;

/// Represents a complete MIR program with functions and globals
#[derive(Debug, Clone)]
pub struct MirProgram {
    pub functions: Vec<MirFunction>, // All function definitions
    pub globals: Vec<MirInstr>,      // Global variable initializations
    pub is_main_entry: bool,         // Whether this is the main entry point file (requires main())
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
        value: Option<String>,
    },
    EnumMatch {
        name: String,
        enum_instance: String,
        variant: String,
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

    /// Manual error extraction with ?? operator
    /// Extracts Ok values and error into separate variables
    /// let a, b ?? err = expr;
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
}
