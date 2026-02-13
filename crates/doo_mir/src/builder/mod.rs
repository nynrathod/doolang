//! MIR Builder
//!
//! Converts HIR to MIR with lowering of high-level constructs.

pub mod expr;
pub mod pattern;
pub mod stmt;

use doo_analysis::{Decision, OwnershipResults};
use doo_core::doo_debug;
use doo_core::types::{builtin, TypeId as CoreTypeId, TypeKind, TypeRegistry};
use doo_core::Span as CoreSpan;
use doo_hir::{
    ConstValue, HirBinOp, HirExpr, HirExprKind, HirFunction, HirItem, HirMatchPattern, HirProgram,
    HirStmt, HirStmtKind, HirUnaryOp,
};

use rustc_hash::FxHashMap;

use crate::types::*;

/// FFI function information extracted from @extern decorator.
///
/// SINGLE SOURCE OF TRUTH for FFI linkage.
#[derive(Debug, Clone)]
pub struct FfiFunctionInfo {
    /// The FFI library name (e.g., "doo_http")
    pub library: String,
    /// The FFI symbol name (e.g., "doo_http_server_new")
    pub symbol: String,
}

/// Derive FFI symbol from library and function name.
///
/// Examples:
/// - ("doo_http", "Server.new") -> "doo_http_server_new"
/// - ("doo_http", "Server.get") -> "doo_http_server_get"
/// - ("doo_db", "Query.exec") -> "doo_db_query_exec"
fn derive_ffi_symbol(library: &str, func_name: &str) -> String {
    // Split function name by '.' for methods
    let parts: Vec<&str> = func_name.split('.').collect();

    if parts.len() == 2 {
        // Method: Server.get -> {library}_server_get
        let type_name = parts[0].to_lowercase();
        let method_name = parts[1].to_lowercase();
        format!("{}_{}", library, format!("{}_{}", type_name, method_name))
    } else {
        // Plain function: myFunc -> {library}_myfunc
        format!("{}_{}", library, func_name.to_lowercase())
    }
}

/// HIR to MIR builder.
pub struct MirBuilder<'a> {
    /// Current function being built.
    pub(crate) current_func: Option<MirFunction>,
    /// Current block index.
    pub(crate) current_block: usize,
    /// Temporary counter for unique names.
    pub(crate) temp_counter: usize,
    /// Block counter for unique labels.
    pub(crate) block_counter: usize,

    pub(crate) type_registry: &'a TypeRegistry,
    pub(crate) container_kinds: FxHashMap<String, ContainerKind>,

    /// Stack of break target labels for loop control flow.
    pub(crate) break_targets: Vec<String>,
    /// Stack of continue target labels for loop control flow.
    pub(crate) continue_targets: Vec<String>,

    /// Ownership analysis results (decisions for each variable use).
    pub ownership_results: Option<OwnershipResults>,

    /// Temporary variable types for type propagation.
    pub(crate) temp_types: FxHashMap<String, CoreTypeId>,

    /// Function return types for type propagation during Call expression building.
    pub(crate) function_return_types: FxHashMap<String, CoreTypeId>,

    /// Function error types to detect Result-returning functions.
    /// Key: function name, Value: (ok_type, err_type)
    pub(crate) function_result_types: FxHashMap<String, (CoreTypeId, CoreTypeId)>,

    /// Function parameter types for type propagation during Call argument building.
    /// Key: function name, Value: list of parameter types
    pub(crate) function_param_types: FxHashMap<String, Vec<CoreTypeId>>,

    /// Counter for generating unique closure function names.
    pub(crate) closure_counter: usize,

    /// Pending closure functions to be added to the program.
    /// Each entry is (func_name, params, body_expr).
    pub(crate) pending_closures: Vec<(String, Vec<(String, Option<CoreTypeId>)>, Box<HirExpr>)>,

    /// Closure return types for type propagation.
    /// Key: closure function name, Value: return type
    pub(crate) closure_return_types: FxHashMap<String, CoreTypeId>,

    /// FFI function registry: maps function name to FFI info (library, symbol).
    /// Used to emit FfiCall instead of Call for FFI functions.
    pub(crate) ffi_functions: FxHashMap<String, FfiFunctionInfo>,

    /// Function name aliases: maps simple import names to mangled names.
    /// e.g., "Postgres" -> "_method_Database_Postgres"
    /// This allows calling imported associated functions by their simple name.
    pub(crate) function_aliases: FxHashMap<String, String>,

    /// Stack of active scope variable names.
    /// When non-empty, `go { ... }` emits ScopeSpawn instead of Spawn.
    pub(crate) scope_stack: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerKind {
    Array,
    Map,
}

impl<'a> MirBuilder<'a> {
    /// Create a new MIR builder.
    pub fn new(type_registry: &'a TypeRegistry) -> Self {
        Self {
            current_func: None,
            current_block: 0,
            temp_counter: 0,
            block_counter: 0,
            type_registry,
            container_kinds: FxHashMap::default(),
            break_targets: Vec::new(),
            continue_targets: Vec::new(),
            ownership_results: None,
            temp_types: FxHashMap::default(),
            function_return_types: FxHashMap::default(),
            function_result_types: FxHashMap::default(),
            function_param_types: FxHashMap::default(),
            closure_counter: 0,
            pending_closures: Vec::new(),
            closure_return_types: FxHashMap::default(),
            ffi_functions: FxHashMap::default(),
            function_aliases: FxHashMap::default(),
            scope_stack: Vec::new(),
        }
    }

    /// Create a new MIR builder with ownership results.
    pub fn with_ownership(
        type_registry: &'a TypeRegistry,
        ownership_results: OwnershipResults,
    ) -> Self {
        Self {
            current_func: None,
            current_block: 0,
            temp_counter: 0,
            block_counter: 0,
            type_registry,
            container_kinds: FxHashMap::default(),
            break_targets: Vec::new(),
            continue_targets: Vec::new(),
            ownership_results: Some(ownership_results),
            temp_types: FxHashMap::default(),
            function_return_types: FxHashMap::default(),
            function_result_types: FxHashMap::default(),
            function_param_types: FxHashMap::default(),
            closure_counter: 0,
            pending_closures: Vec::new(),
            closure_return_types: FxHashMap::default(),
            ffi_functions: FxHashMap::default(),
            function_aliases: FxHashMap::default(),
            scope_stack: Vec::new(),
        }
    }

    /// Set ownership results after creation.
    pub fn set_ownership_results(&mut self, results: OwnershipResults) {
        self.ownership_results = Some(results);
    }

    /// Get the ownership decision for a variable at a specific span.
    pub(crate) fn get_ownership_decision(&self, name: &str, span: CoreSpan) -> Option<Decision> {
        self.ownership_results.as_ref()?.get_decision(name, span)
    }

    /// Build MIR from HIR program.
    pub fn build(&mut self, hir: &HirProgram) -> MirProgram {
        let mut program = MirProgram::new();

        // First pass: collect all function return types, parameter types, and FFI info
        for item in &hir.items {
            if let HirItem::Function(f) = item {
                // For functions with error types, track them separately
                // This includes both `-> T ! E` (return_type + error_type)
                // and `-> ! E` (error_type only, meaning void return with possible error)
                if let Some(error_type) = f.error_type {
                    // Use VOID as the ok type when no return type is specified
                    let return_type = f.return_type.unwrap_or(builtin::VOID);
                    // Store the Result type components
                    self.function_result_types
                        .insert(f.name.clone(), (return_type, error_type));
                    if std::env::var("DOO_DEBUG").is_ok() {
                        doo_debug!(
                            "MIR",
                            "Registered Result function: {} (ok={:?}, err={:?})",
                            f.name,
                            return_type,
                            error_type
                        );
                    }
                    // Also store the return type for the temp type (will be the ok value for unwrapping)
                    self.function_return_types
                        .insert(f.name.clone(), return_type);
                } else if let Some(return_type) = f.return_type {
                    self.function_return_types
                        .insert(f.name.clone(), return_type);
                } else {
                    // Void functions with no error type — still register so is_function_name() works
                    // (needed when void functions are passed as FuncRef, e.g. WS event handlers)
                    self.function_return_types
                        .insert(f.name.clone(), builtin::VOID);
                }

                // Collect parameter types for type-aware argument building
                let param_types: Vec<CoreTypeId> =
                    f.params.iter().filter_map(|p| p.type_id).collect();
                if !param_types.is_empty() {
                    self.function_param_types
                        .insert(f.name.clone(), param_types);
                }

                // Extract FFI info from @extern decorator (SINGLE SOURCE OF TRUTH)
                if let Some(ffi_info) = self.extract_ffi_info(&f.decorators, &f.name) {
                    if std::env::var("DOO_DEBUG").is_ok() {
                        doo_debug!(
                            "MIR",
                            "Registered FFI function: {} -> lib={} sym={}",
                            f.name,
                            ffi_info.library,
                            ffi_info.symbol
                        );
                    }
                    self.ffi_functions.insert(f.name.clone(), ffi_info);
                }

                // Create alias from simple name to mangled name for imported associated functions
                // e.g., "_method_Database_Postgres" -> alias "Postgres" -> "_method_Database_Postgres"
                if f.name.starts_with("_method_") {
                    // Extract simple name: "_method_TypeName_MethodName" -> "MethodName"
                    if let Some(pos) = f.name[8..].find('_') {
                        let simple_name = &f.name[8 + pos + 1..];
                        if !simple_name.is_empty() {
                            self.function_aliases
                                .insert(simple_name.to_string(), f.name.clone());
                            if std::env::var("DOO_DEBUG").is_ok() {
                                doo_debug!(
                                    "MIR",
                                    "Registered function alias: {} -> {}",
                                    simple_name,
                                    f.name
                                );
                            }
                        }
                    }
                }
            }
        }

        for item in &hir.items {
            match item {
                HirItem::Function(f) => {
                    let mir_func = self.build_function(f);
                    program.functions.push(mir_func);
                }
                HirItem::Struct(s) => {
                    let mir_struct = StructDef {
                        name: s.name.clone(),
                        fields: s
                            .fields
                            .iter()
                            .map(|f| FieldDef {
                                name: f.name.clone(),
                                type_id: f.type_id.unwrap_or(builtin::ANY),
                                optional: f.is_optional,
                                decorators: Vec::new(),
                                default_value: None,
                            })
                            .collect(),
                        decorators: Vec::new(),
                    };
                    program.structs.insert(s.name.clone(), mir_struct);
                }
                HirItem::Enum(e) => {
                    let mir_enum = EnumDef {
                        name: e.name.clone(),
                        variants: e
                            .variants
                            .iter()
                            .enumerate()
                            .map(|(i, v)| VariantDef {
                                name: v.name.clone(),
                                index: i as u32,
                                payload_type: v.payload,
                            })
                            .collect(),
                    };
                    program.enums.insert(e.name.clone(), mir_enum);
                }
                HirItem::Import(_) => {
                    // Imports handled elsewhere
                }
            }
        }

        // Generate MIR functions for all pending closures
        while let Some((closure_name, params, body)) = self.pending_closures.pop() {
            let closure_func = self.build_closure_function(&closure_name, &params, &body);
            program.functions.push(closure_func);
        }

        program
    }

    /// Build a MIR function for a closure.
    fn build_closure_function(
        &mut self,
        name: &str,
        params: &[(String, Option<CoreTypeId>)],
        body: &HirExpr,
    ) -> MirFunction {
        // Save current state
        let saved_func = self.current_func.take();
        let saved_block = self.current_block;
        let saved_temp = self.temp_counter;
        let saved_label = self.block_counter;
        let saved_temp_types = std::mem::take(&mut self.temp_types);
        let saved_container_kinds = std::mem::take(&mut self.container_kinds);

        // Reset counters for closure
        self.temp_counter = 0;
        self.block_counter = 0;

        // Create new closure function
        let mut func = MirFunction::new(name.to_string());
        func.is_closure = true; // Mark as closure for special codegen handling

        // Keep original param types for proper MIR body codegen
        // The LLVM signature will use i64 calling convention, but codegen
        // will handle the conversion
        func.params = params
            .iter()
            .map(|(pname, ptype)| ParamDef {
                name: pname.clone(),
                type_id: ptype.unwrap_or(builtin::INT),
            })
            .collect();
        // Don't set return_type yet - we'll infer it from the body expression

        // Create entry block
        func.blocks.push(MirBlock::new("entry".to_string()));
        self.current_func = Some(func);
        self.current_block = 0;

        // Register parameters as locals with their original types
        for (pname, ptype) in params {
            if let Some(f) = &mut self.current_func {
                f.locals.push(LocalDef {
                    name: pname.clone(),
                    type_id: ptype.unwrap_or(builtin::INT),
                    mutable: false,
                });
            }
        }

        // Build the body expression
        let result = self.build_expr(body);

        // Infer actual return type from the result expression
        let mut return_type = self.infer_operand_type(&result);

        // If the result type is ANY, it might be because the closure body has an explicit
        // `return` statement (which sets the terminator). Check for an existing Return terminator
        // and infer the type from its operands.
        if return_type == builtin::ANY {
            if let Some(f) = &self.current_func {
                // Check all blocks for Return terminators (for closures with explicit returns)
                for block in &f.blocks {
                    if let MirTerminator::Return { values } = &block.terminator {
                        if let Some(first_val) = values.first() {
                            let inferred = self.infer_operand_type(first_val);
                            if inferred != builtin::ANY {
                                return_type = inferred;
                                break;
                            }
                        }
                    }
                }
            }
        }

        // If still ANY, use the HIR body's type_id as a fallback.
        // The HIR lowering may have set the body type via apply_closure_signature,
        // but the MIR operand-level inference lost it (e.g., BinOp on closure params
        // where param types were set but the expr.type_id wasn't propagated).
        if return_type == builtin::ANY {
            if let Some(body_type) = body.type_id {
                if body_type != builtin::ANY {
                    return_type = body_type;
                }
            }
        }

        // If STILL ANY, infer from param types and the MIR instructions.
        // For closures like (a, b) => a * b, if params are Float, result is Float.
        if return_type == builtin::ANY {
            if let Some(f) = &self.current_func {
                // Check if any param is Float - arithmetic on Floats returns Float
                let has_float_param = f.params.iter().any(|p| p.type_id == builtin::FLOAT);
                if has_float_param {
                    // Scan instructions for arithmetic ops that would produce Float results
                    for block in &f.blocks {
                        for instr in &block.instructions {
                            if let crate::MirInstrKind::BinaryOp { op, .. } = &instr.kind {
                                match op {
                                    crate::BinaryOp::Add
                                    | crate::BinaryOp::Sub
                                    | crate::BinaryOp::Mul
                                    | crate::BinaryOp::Div
                                    | crate::BinaryOp::Mod => {
                                        return_type = builtin::FLOAT;
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                        }
                        if return_type != builtin::ANY {
                            break;
                        }
                    }
                }
            }
        }

        // Update function's return type with the actual type
        if let Some(f) = &mut self.current_func {
            f.return_type = Some(return_type);
        }

        // Add return statement only if no explicit return was already set
        // (e.g., block closures with `return x;` will have set the terminator already)
        self.set_terminator_if_none(MirTerminator::Return {
            values: vec![result],
        });

        // Extract the built function
        let closure_func = self.current_func.take().unwrap();

        // Restore previous state
        self.current_func = saved_func;
        self.current_block = saved_block;
        self.temp_counter = saved_temp;
        self.block_counter = saved_label;
        self.temp_types = saved_temp_types;
        self.container_kinds = saved_container_kinds;

        closure_func
    }

    /// Build MIR for a function.
    fn build_function(&mut self, hir: &HirFunction) -> MirFunction {
        self.temp_counter = 0;
        self.block_counter = 0;
        self.container_kinds.clear();

        let mut func = MirFunction::new(hir.name.clone());
        func.params = hir
            .params
            .iter()
            .map(|p| ParamDef {
                name: p.name.clone(),
                type_id: p.type_id.unwrap_or(builtin::ANY),
            })
            .collect();
        func.return_type = hir.return_type;
        func.error_type = hir.error_type;
        func.is_async = hir.is_async;

        // Set FFI linkage info if this is an FFI function
        if let Some(ffi_info) = self.ffi_functions.get(&hir.name) {
            func.ffi = Some(FfiLinkage {
                library: ffi_info.library.clone(),
                symbol: Some(ffi_info.symbol.clone()),
            });
        }

        // Create entry block
        func.blocks.push(MirBlock::new("entry".to_string()));
        self.current_func = Some(func);
        self.current_block = 0;

        // Register parameters as locals
        for param in &hir.params {
            if let Some(f) = &mut self.current_func {
                f.locals.push(LocalDef {
                    name: param.name.clone(),
                    type_id: param.type_id.unwrap_or(builtin::ANY),
                    mutable: false,
                });
            }
        }

        // Build statements, treating the last expression statement specially for implicit returns
        // in functions that have a return type
        let has_return_type = hir.return_type.is_some();
        let stmt_count = hir.body.len();

        for (idx, stmt) in hir.body.iter().enumerate() {
            let is_last_stmt = idx + 1 == stmt_count;

            // Check if this is the last statement and it's a bare expression
            // that should be implicitly returned
            if is_last_stmt && has_return_type {
                if let HirStmtKind::Expr(expr) = &stmt.kind {
                    // This is the last expression in a function with a return type
                    // - treat it as an implicit return (like Rust does)
                    let result = self.build_expr(expr);
                    self.set_terminator_if_none(MirTerminator::Return {
                        values: vec![result],
                    });
                    continue;
                }
            }

            self.build_stmt(stmt);
        }

        // Ensure function has a terminator
        // For void functions and functions with `-> ! E` (void + error type),
        // we need to add implicit return when control reaches the end
        // Check if we need to add implicit return for `-> ! E` functions
        let needs_void_error_return = if let Some(f) = &self.current_func {
            if let Some(block) = f.blocks.get(self.current_block) {
                matches!(block.terminator, MirTerminator::Unreachable)
                    && f.return_type.is_none()
                    && f.error_type.is_some()
            } else {
                false
            }
        } else {
            false
        };

        // Generate temp BEFORE borrowing block mutably (to avoid borrow conflict)
        let ok_dest = if needs_void_error_return {
            Some(self.new_temp())
        } else {
            None
        };

        if let Some(f) = &mut self.current_func {
            if let Some(block) = f.blocks.get_mut(self.current_block) {
                if matches!(block.terminator, MirTerminator::Unreachable) {
                    if f.return_type.is_none() && f.error_type.is_none() {
                        // Pure void function: just return
                        block.terminator = MirTerminator::Return { values: Vec::new() };
                    } else if let Some(dest) = ok_dest {
                        // `-> ! E` function: return Ok(void) wrapped as Result
                        let void_val = MirOperand::Const(MirConst::Int(0)); // void placeholder
                        block.instructions.push(MirInstr::new(MirInstrKind::WrapOk {
                            dest: dest.clone(),
                            value: void_val,
                        }));
                        block.terminator = MirTerminator::Return {
                            values: vec![MirOperand::Temp(dest)],
                        };
                    }
                    // Non-void functions should have explicit returns; unreachable blocks stay unreachable
                }
            }
        }

        self.current_func.take().unwrap()
    }

    pub fn build_stmt(&mut self, stmt: &HirStmt) {
        stmt::build_stmt(self, stmt);
    }

    pub fn build_expr(&mut self, expr: &HirExpr) -> MirOperand {
        expr::build_expr(self, expr)
    }

    pub fn build_expr_with_expected_type(
        &mut self,
        expr: &HirExpr,
        expected_type: Option<CoreTypeId>,
    ) -> MirOperand {
        expr::build_expr_with_expected_type(self, expr, expected_type)
    }

    pub fn build_match_condition(
        &mut self,
        scrutinees: &[MirOperand],
        pattern: &HirMatchPattern,
        span: Span,
    ) -> MirOperand {
        pattern::build_match_condition(self, scrutinees, pattern, span)
    }

    // ========================================================================
    // Helpers
    // ========================================================================

    pub(crate) fn convert_span(&self, core_span: CoreSpan) -> Span {
        Span {
            start: core_span.start,
            end: core_span.end,
        }
    }

    pub(crate) fn emit(&mut self, kind: MirInstrKind, span: Span) {
        if let Some(f) = &mut self.current_func {
            if let Some(block) = f.blocks.get_mut(self.current_block) {
                block.instructions.push(MirInstr { kind, span });
            }
        }
    }

    pub(crate) fn set_terminator(&mut self, term: MirTerminator) {
        if let Some(f) = &mut self.current_func {
            if let Some(block) = f.blocks.get_mut(self.current_block) {
                block.terminator = term;
            }
        }
    }

    /// Check if the current block already has a terminator set (not Unreachable).
    pub(crate) fn current_block_has_terminator(&self) -> bool {
        if let Some(f) = &self.current_func {
            if let Some(block) = f.blocks.get(self.current_block) {
                return !matches!(block.terminator, MirTerminator::Unreachable);
            }
        }
        false
    }

    /// Set terminator only if one hasn't been set yet.
    pub(crate) fn set_terminator_if_none(&mut self, term: MirTerminator) {
        if !self.current_block_has_terminator() {
            self.set_terminator(term);
        }
    }

    pub(crate) fn add_block(&mut self, label: &str) {
        if let Some(f) = &mut self.current_func {
            f.blocks.push(MirBlock::new(label.to_string()));
            self.current_block = f.blocks.len() - 1;
        }
    }

    pub(crate) fn new_temp(&mut self) -> String {
        let name = format!("_t{}", self.temp_counter);
        self.temp_counter += 1;
        name
    }

    /// Add a temporary variable to func.locals so codegen can access its type.
    /// This is needed for temps that hold intermediate values (like if-expr results)
    /// which need proper LLVM types during alloca creation.
    pub(crate) fn add_temp_local(&mut self, name: &str, type_id: CoreTypeId) {
        if let Some(f) = &mut self.current_func {
            // Only add if not already present
            if !f.locals.iter().any(|l| l.name == name) {
                f.locals.push(LocalDef {
                    name: name.to_string(),
                    type_id,
                    mutable: false,
                });
            }
        }
    }

    pub(crate) fn new_block_label(&mut self, prefix: &str) -> String {
        let label = format!("{}_{}", prefix, self.block_counter);
        self.block_counter += 1;
        label
    }

    pub(crate) fn expr_to_name(&self, expr: &HirExpr) -> String {
        match &expr.kind {
            HirExprKind::Local { name } => name.clone(),
            HirExprKind::Global { name } => name.clone(),
            _ => "__expr".to_string(),
        }
    }

    pub(crate) fn const_to_mir(&self, cv: &ConstValue) -> MirConst {
        match cv {
            ConstValue::Int(v) => MirConst::Int(*v),
            ConstValue::Float(v) => MirConst::Float(*v),
            ConstValue::Bool(v) => MirConst::Bool(*v),
            ConstValue::Str(v) => MirConst::Str(v.clone()),
            ConstValue::Nil => MirConst::Nil,
        }
    }

    pub(crate) fn binop_to_mir(&self, op: HirBinOp) -> BinaryOp {
        match op {
            HirBinOp::Add => BinaryOp::Add,
            HirBinOp::Sub => BinaryOp::Sub,
            HirBinOp::Mul => BinaryOp::Mul,
            HirBinOp::Div => BinaryOp::Div,
            HirBinOp::Mod => BinaryOp::Mod,
            HirBinOp::Eq => BinaryOp::Eq,
            HirBinOp::NotEq => BinaryOp::Ne,
            HirBinOp::Lt => BinaryOp::Lt,
            HirBinOp::Gt => BinaryOp::Gt,
            HirBinOp::LtEq => BinaryOp::Le,
            HirBinOp::GtEq => BinaryOp::Ge,
            HirBinOp::In => BinaryOp::Eq,
            HirBinOp::And => BinaryOp::And,
            HirBinOp::Or => BinaryOp::Or,
            HirBinOp::BitAnd => BinaryOp::And,
            HirBinOp::BitOr => BinaryOp::Or,
        }
    }

    pub(crate) fn infer_container_kind(&self, expr: &HirExpr) -> Option<ContainerKind> {
        match &expr.kind {
            HirExprKind::Array(_) => Some(ContainerKind::Array),
            HirExprKind::Map(_) => Some(ContainerKind::Map),
            HirExprKind::Move(inner) | HirExprKind::Clone(inner) => {
                self.infer_container_kind(inner)
            }
            HirExprKind::Borrow { expr: inner, .. } => self.infer_container_kind(inner),
            HirExprKind::Local { name } => {
                // First check the container_kinds cache
                if let Some(kind) = self.container_kinds.get(name).copied() {
                    return Some(kind);
                }
                // Fallback: look up the local variable's type from the function
                if let Some(type_id) = self.get_local_type(name) {
                    return self.container_kind_from_type_id(type_id);
                }
                // Final fallback: check expr.type_id
                expr.type_id
                    .and_then(|tid| self.container_kind_from_type_id(tid))
            }
            _ => expr
                .type_id
                .and_then(|tid| self.container_kind_from_type_id(tid)),
        }
    }

    pub(crate) fn container_kind_from_expr(&self, expr: &HirExpr) -> Option<ContainerKind> {
        self.infer_container_kind(expr)
    }

    fn container_kind_from_type_id(&self, type_id: CoreTypeId) -> Option<ContainerKind> {
        self.type_registry
            .get(type_id)
            .and_then(|info| match info.kind {
                TypeKind::Array { .. } => Some(ContainerKind::Array),
                TypeKind::Map { .. } => Some(ContainerKind::Map),
                _ => None,
            })
    }

    pub(crate) fn array_elem_type_from_type_id(&self, type_id: CoreTypeId) -> Option<CoreTypeId> {
        self.type_registry
            .get(type_id)
            .and_then(|info| match &info.kind {
                TypeKind::Array { element } => Some(*element),
                _ => None,
            })
    }

    pub(crate) fn map_types_from_type_id(
        &self,
        type_id: CoreTypeId,
    ) -> Option<(CoreTypeId, CoreTypeId)> {
        self.type_registry
            .get(type_id)
            .and_then(|info| match info.kind {
                TypeKind::Map { key, value } => Some((key, value)),
                _ => None,
            })
    }

    /// Get the type of a struct field.
    pub(crate) fn struct_field_type(
        &self,
        struct_type: CoreTypeId,
        field_name: &str,
    ) -> Option<CoreTypeId> {
        self.type_registry
            .get(struct_type)
            .and_then(|info| match &info.kind {
                TypeKind::Struct { fields, .. } => fields
                    .iter()
                    .find(|(name, _, _)| name == field_name)
                    .map(|(_, t, _)| *t),
                _ => None,
            })
    }

    /// Get the payload type of an enum variant.
    /// Used during match expression lowering to register payload bindings with correct types.
    pub(crate) fn get_enum_variant_payload_type(
        &self,
        enum_name: &str,
        variant_name: &str,
    ) -> Option<CoreTypeId> {
        // Look up the enum type in the registry
        let type_id = self.type_registry.lookup(enum_name)?;
        let type_info = self.type_registry.get(type_id)?;

        // Extract the variant's payload type from the enum definition
        if let TypeKind::Enum { variants, .. } = &type_info.kind {
            for (vname, payload_type) in variants {
                if vname == variant_name {
                    return *payload_type;
                }
            }
        }
        None
    }

    /// Get the element types if the given type is a Tuple.
    /// Returns None if the type is not a tuple.
    /// Used to get individual element types for enum variant tuple payloads.
    pub(crate) fn get_tuple_element_types(&self, type_id: CoreTypeId) -> Option<Vec<CoreTypeId>> {
        let type_info = self.type_registry.get(type_id)?;
        if let TypeKind::Tuple { elements } = &type_info.kind {
            Some(elements.clone())
        } else {
            None
        }
    }

    /// Look up the type of a local variable by name.
    pub(crate) fn get_local_type(&self, name: &str) -> Option<CoreTypeId> {
        self.current_func
            .as_ref()
            .and_then(|f| f.locals.iter().find(|l| l.name == name).map(|l| l.type_id))
    }

    /// Recursively infer the type of an HIR expression.
    /// Handles Local, Field, and expressions with type_id set.
    pub(crate) fn infer_hir_expr_type(&self, expr: &HirExpr) -> Option<CoreTypeId> {
        // First check if type_id is already set
        if let Some(tid) = expr.type_id {
            return Some(tid);
        }

        // Otherwise, infer based on expression kind
        match &expr.kind {
            HirExprKind::Local { name } => self.get_local_type(name),
            HirExprKind::Field { object, field } => {
                // Recursively get the object's type, then look up the field type
                let obj_type = self.infer_hir_expr_type(object)?;
                self.struct_field_type(obj_type, field)
            }
            _ => None,
        }
    }

    /// Get the return type of the current function being built.
    pub(crate) fn get_current_function_return_type(&self) -> Option<CoreTypeId> {
        self.current_func.as_ref().and_then(|f| f.return_type)
    }

    /// Get the error type of the current function being built.
    pub(crate) fn get_current_function_error_type(&self) -> Option<CoreTypeId> {
        self.current_func.as_ref().and_then(|f| f.error_type)
    }

    /// Get the return type of a function by name.
    /// Resolves aliases before lookup.
    pub(crate) fn get_function_return_type(&self, name: &str) -> Option<CoreTypeId> {
        // First try direct lookup
        if let Some(tid) = self.function_return_types.get(name).copied() {
            return Some(tid);
        }
        // Then try through alias
        if let Some(resolved) = self.function_aliases.get(name) {
            return self.function_return_types.get(resolved).copied();
        }
        None
    }

    /// Check if a name refers to a known function (not a variable).
    /// Resolves aliases before lookup.
    pub(crate) fn is_function_name(&self, name: &str) -> bool {
        let resolved = self.resolve_function_name(name);
        self.function_return_types.contains_key(&resolved)
            || self.ffi_functions.contains_key(&resolved)
    }

    /// Check if a name refers to a registered type (struct or enum).
    /// Returns true if the name is found in the type registry as a struct or enum type.
    pub(crate) fn is_type_name(&self, name: &str) -> bool {
        if let Some(type_id) = self.type_registry.lookup(name) {
            if let Some(info) = self.type_registry.get(type_id) {
                return matches!(info.kind, TypeKind::Struct { .. } | TypeKind::Enum { .. });
            }
        }
        false
    }

    /// Get the return type for a builtin method call based on receiver type and method name.
    /// This is the SINGLE SOURCE OF TRUTH lookup using doo_core::methods.
    pub(crate) fn get_builtin_method_return_type(
        &self,
        receiver_type: CoreTypeId,
        method: &str,
    ) -> Option<CoreTypeId> {
        // Use the extended version with no closure info
        self.get_builtin_method_return_type_with_closure(receiver_type, method, None)
    }

    /// Get the return type for a builtin method call, with optional closure argument type.
    /// This handles generic return types like [U] (from map) where U is closure's return.
    /// SINGLE SOURCE OF TRUTH using doo_core::methods.
    pub(crate) fn get_builtin_method_return_type_with_closure(
        &self,
        receiver_type: CoreTypeId,
        method: &str,
        closure_type: Option<CoreTypeId>,
    ) -> Option<CoreTypeId> {
        use doo_core::methods::get_method;

        // Get the type name for lookup
        let type_name: &str = match self.type_registry.get(receiver_type).map(|info| &info.kind) {
            Some(TypeKind::Str) => "Str",
            Some(TypeKind::Int) => "Int",
            Some(TypeKind::Float) => "Float",
            Some(TypeKind::Bool) => "Bool",
            Some(TypeKind::Array { .. }) => "[T]",
            Some(TypeKind::Map { .. }) => "{K:V}",
            _ => return None,
        };

        // Look up the method definition
        let method_def = get_method(type_name, method)?;

        // Convert return type string to TypeId
        match method_def.return_type {
            "Int" => Some(builtin::INT),
            "Bool" => Some(builtin::BOOL),
            "Str" => Some(builtin::STR),
            "Float" => Some(builtin::FLOAT),
            "Void" => Some(builtin::VOID),
            // For generic types like T, [T], [U], U, etc.
            "T" => {
                // Element type of array or value type of map
                if let Some(info) = self.type_registry.get(receiver_type) {
                    match &info.kind {
                        TypeKind::Array { element } => Some(*element),
                        TypeKind::Map { value, .. } => Some(*value),
                        _ => Some(builtin::ANY),
                    }
                } else {
                    Some(builtin::ANY)
                }
            }
            "[T]" => {
                // Same array type as receiver
                if let Some(info) = self.type_registry.get(receiver_type) {
                    if let TypeKind::Array { element: _ } = &info.kind {
                        return Some(receiver_type); // Same type for slice
                    }
                }
                Some(builtin::ANY)
            }
            // [U] - Array of closure return type (e.g., map returns [U])
            "[U]" => {
                // Get U from closure's function return type
                if let Some(closure_tid) = closure_type {
                    if let Some(info) = self.type_registry.get(closure_tid) {
                        if let TypeKind::Function { returns, .. } = &info.kind {
                            // Look up the array type [U] by name
                            // Array type names are formatted as "[ElementName]"
                            if let Some(elem_info) = self.type_registry.get(*returns) {
                                let array_name = format!("[{}]", elem_info.name);
                                if let Some(array_tid) = self.type_registry.lookup(&array_name) {
                                    return Some(array_tid);
                                }
                            }
                            // If U is same as receiver element type, return receiver type
                            if let Some(recv_info) = self.type_registry.get(receiver_type) {
                                if let TypeKind::Array { element } = &recv_info.kind {
                                    if *element == *returns {
                                        return Some(receiver_type);
                                    }
                                }
                            }
                        }
                    }
                }
                // Fallback: same array type as receiver (e.g., identity map)
                if let Some(info) = self.type_registry.get(receiver_type) {
                    if let TypeKind::Array { .. } = &info.kind {
                        return Some(receiver_type);
                    }
                }
                Some(builtin::ANY)
            }
            // U - Closure return type (e.g., reduce returns U)
            "U" => {
                // Get U from closure's function return type
                if let Some(closure_tid) = closure_type {
                    if let Some(info) = self.type_registry.get(closure_tid) {
                        if let TypeKind::Function { returns, .. } = &info.kind {
                            return Some(*returns);
                        }
                    }
                }
                Some(builtin::ANY)
            }
            "[K]" => {
                // Array of keys from a map
                if let Some(info) = self.type_registry.get(receiver_type) {
                    if let TypeKind::Map { key, .. } = &info.kind {
                        return Some(*key);
                    }
                }
                Some(builtin::ANY)
            }
            "[V]" => {
                // Array of values from a map
                if let Some(info) = self.type_registry.get(receiver_type) {
                    if let TypeKind::Map { value, .. } = &info.kind {
                        return Some(*value);
                    }
                }
                Some(builtin::ANY)
            }
            _ => Some(builtin::ANY),
        }
    }

    /// Get the parameter types of a function by name.
    /// Resolves aliases before lookup.
    pub(crate) fn get_function_param_types(&self, name: &str) -> Option<&Vec<CoreTypeId>> {
        // First try direct lookup
        if let Some(types) = self.function_param_types.get(name) {
            return Some(types);
        }
        // Then try through alias
        if let Some(resolved) = self.function_aliases.get(name) {
            return self.function_param_types.get(resolved);
        }
        None
    }

    /// Set the type of a temporary variable.
    pub(crate) fn set_temp_type(&mut self, name: &str, type_id: CoreTypeId) {
        self.temp_types.insert(name.to_string(), type_id);
    }

    /// Get the type of a temporary variable.
    pub(crate) fn get_temp_type(&self, name: &str) -> Option<CoreTypeId> {
        self.temp_types.get(name).copied()
    }

    /// Infer type from a MirOperand.
    pub(crate) fn infer_operand_type(&self, operand: &MirOperand) -> CoreTypeId {
        use crate::MirConst;
        match operand {
            MirOperand::Const(c) => match c {
                MirConst::Int(_) => builtin::INT,
                MirConst::Float(_) => builtin::FLOAT,
                MirConst::Bool(_) => builtin::BOOL,
                MirConst::Str(_) => builtin::STR,
                MirConst::Nil => builtin::ANY,
            },
            MirOperand::Temp(name) => {
                // Check if we have a recorded type for this temp
                self.get_temp_type(name).unwrap_or(builtin::ANY)
            }
            MirOperand::Local(name) => {
                // Check temp_types FIRST to handle shadowed bindings correctly.
                // When a match binding shadows a local with a different type,
                // we register the binding's type in temp_types, which takes precedence.
                self.get_temp_type(name)
                    .or_else(|| self.get_local_type(name))
                    .unwrap_or(builtin::ANY)
            }
            MirOperand::Global(_) => builtin::ANY,
            MirOperand::FuncRef(_) => builtin::ANY, // Function pointers are opaque
        }
    }

    pub(crate) fn unaryop_to_mir(&self, op: HirUnaryOp) -> UnaryOp {
        match op {
            HirUnaryOp::Neg => UnaryOp::Neg,
            HirUnaryOp::Not => UnaryOp::Not,
        }
    }

    /// Extract FFI info from function decorators.
    ///
    /// NEW DESIGN (single decorator):
    /// - @extern("library", "symbol") - explicit library and symbol
    /// - @extern("library") - symbol auto-derived from function name
    ///
    /// This is the SINGLE SOURCE OF TRUTH for FFI function detection.
    fn extract_ffi_info(
        &self,
        decorators: &[doo_hir::HirDecorator],
        func_name: &str,
    ) -> Option<FfiFunctionInfo> {
        for dec in decorators {
            if dec.name == "extern" {
                // Extract string arguments from @extern decorator
                let args: Vec<_> = dec
                    .args
                    .iter()
                    .filter_map(|arg| {
                        if let HirExprKind::Const(ConstValue::Str(s)) = &arg.kind {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .collect();

                match args.len() {
                    2 => {
                        // @extern("library", "symbol") - explicit
                        return Some(FfiFunctionInfo {
                            library: args[0].clone(),
                            symbol: args[1].clone(),
                        });
                    }
                    1 => {
                        // @extern("library") - auto-derive symbol from function name
                        // Mangle: Server.get -> doo_http_server_get (using library prefix)
                        let lib = &args[0];
                        let symbol = derive_ffi_symbol(lib, func_name);
                        return Some(FfiFunctionInfo {
                            library: lib.clone(),
                            symbol,
                        });
                    }
                    _ => {}
                }
            }
        }
        None
    }

    /// Resolve a function name through aliases.
    /// Returns the canonical (mangled) function name if an alias exists,
    /// or the original name otherwise.
    pub(crate) fn resolve_function_name(&self, name: &str) -> String {
        self.function_aliases
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    /// Check if a function is an FFI function and get its info.
    /// Resolves aliases before lookup.
    pub(crate) fn get_ffi_info(&self, func_name: &str) -> Option<&FfiFunctionInfo> {
        // First try direct lookup
        if let Some(info) = self.ffi_functions.get(func_name) {
            return Some(info);
        }
        // Then try through alias
        if let Some(resolved) = self.function_aliases.get(func_name) {
            return self.ffi_functions.get(resolved);
        }
        None
    }
}
