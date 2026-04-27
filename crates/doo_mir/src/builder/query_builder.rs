//! Query Builder — Compile-Time SQL Generation
//!
//! Intercepts fluent query chains like:
//!   `db.find(Task).where({ status: "active" }).limit(10).exec()?`
//!
//! And compiles them to a single `doo_db_raw_param(db, sql, params_json)` FFI call,
//! where `sql` is a compile-time constant and `params_json` is built at runtime.
//!
//! ## Architecture
//! 1. `is_query_terminal` — detects chain terminal (exec/execOne/toSql)
//! 2. `try_lower_query_chain` — entry point called from `build_expr`
//! 3. `extract_chain` — recursively walks HIR chain from terminal to root
//! 4. `build_sql` — generates SQL string with `$1`, `$2`, ... placeholders
//! 5. `emit_query` — emits MIR: build params array, JSON.stringify, FfiCall
//!
//! ## Rules (Single Source of Truth)
//! - All method names come from `doo_core::constants::ffi_names`
//! - All error codes come from `doo_core::errors::codes::ErrorCode`
//! - SQL param numbering: PostgreSQL style `$1`, `$2`, ...
//! - INSERT/UPDATE/DELETE always append `RETURNING *`
//! - `execOne` adds `LIMIT 1` to SELECT queries
//! - `toSql` emits SQL string constant instead of FFI call
//! - update/delete without `.where()` emits E0707 error

use doo_core::constants::ffi_names;
use doo_core::errors::codes::{CompilerError, ErrorCode};
use doo_core::types::builtin;
use doo_core::types::TypeKind;
use doo_core::Span as CoreSpan;
use doo_hir::{ConstValue, HirExpr, HirExprKind};
use std::collections::{HashMap, HashSet};

use crate::sym::sym;
use crate::types::{MirConst, MirInstrKind, MirOperand, MirTerminator, Span as MirSpan};

use super::MirBuilder;

// ============================================================================
// Public API
// ============================================================================

/// Check if a method name is a query chain terminal.
/// Called by `build_expr` to decide whether to attempt chain interception.
#[inline]
pub fn is_query_terminal(method: &str) -> bool {
    ffi_names::is_query_terminal(method)
}

/// Try to lower a complete query chain expression.
///
/// Called when `build_expr` encounters a MethodCall with a terminal method
/// (exec/execOne/toSql). Returns `Some(operand)` if the chain was recognised
/// and lowered; `None` if the call should be handled by normal dispatch.
pub fn try_lower_query_chain(
    builder: &mut MirBuilder,
    expr: &HirExpr,
    span: MirSpan,
) -> Option<MirOperand> {
    // Extract the chain structure from the HIR expression tree.
    let chain = extract_chain(expr)?;

    // Build SQL + collect param HIR expressions.
    let (sql, param_exprs, error) = build_sql(&chain, builder);

    // Surface validation errors — but continue to emit fallback code so the
    // compiler can keep going and find more errors in the same file.
    if let Some(err) = error {
        builder.query_errors.push(err);
        return Some(MirOperand::Const(MirConst::Nil));
    }

    // toSql terminal: return the SQL string as a compile-time constant.
    if chain.terminal == ffi_names::QB_TERMINAL_TO_SQL {
        return Some(MirOperand::Const(MirConst::Str(sql)));
    }

    // Build the db receiver operand (the `db` variable).
    let db_op = build_expr_from_hir(builder, chain.db_expr, span);

    // Auto-migrate the queried model table before executing query terminals.
    // This keeps direct `doo run` query-builder scripts self-contained.
    emit_model_auto_migration(builder, &db_op, chain.model_name, span);

    // Build the params JSON string operand.
    let params_op = build_params_operand(builder, &param_exprs, span);

    // Emit FfiCall to doo_db_raw_param(db, sql_str, params_json).
    // The FFI returns a *mut DooResult — we must unwrap it inline so callers
    // receive the payload (JSON string ptr) directly instead of a DooResult*.
    let ffi_dest = builder.new_temp();
    let sql_op = MirOperand::Const(MirConst::Str(sql));
    builder.set_temp_type(ffi_dest, builtin::STR);
    builder.emit(
        MirInstrKind::FfiCall {
            dest: Some(ffi_dest),
            lib: sym(ffi_names::LIB_DOO_DB),
            symbol: sym(ffi_names::DOO_DB_RAW_PARAM),
            args: vec![db_op, sql_op, params_op],
        },
        span,
    );

    // Emit inline Result-unwrap pattern (equivalent to `?` operator) so that
    // the outer `?` applied by the caller doesn't need to handle this itself.
    // doo_db_raw_param returns *mut DooResult; IsOk/UnwrapOk extract the payload.
    let is_ok_dest = builder.new_temp();
    builder.emit(
        MirInstrKind::IsOk {
            dest: is_ok_dest,
            value: MirOperand::Temp(ffi_dest),
        },
        span,
    );

    let ok_label = builder.new_block_label("qb_ok");
    let err_label = builder.new_block_label("qb_err");
    let cont_label = builder.new_block_label("qb_cont");

    builder.set_terminator(MirTerminator::Branch {
        cond: MirOperand::Temp(is_ok_dest),
        then_block: ok_label,
        else_block: err_label,
    });

    // Ok path: unwrap payload and jump to continuation.
    builder.add_block(ok_label);
    let unwrapped = builder.new_temp();
    builder.set_temp_type(unwrapped, builtin::STR);
    builder.emit(
        MirInstrKind::UnwrapOk {
            dest: unwrapped,
            value: MirOperand::Temp(ffi_dest),
            expected_type: Some(builtin::STR),
        },
        span,
    );
    builder.set_terminator(MirTerminator::Goto { target: cont_label });

    // Err path: propagate or panic depending on current function signature.
    builder.add_block(err_label);
    if builder.get_current_function_error_type().is_some() {
        let err_dest = builder.new_temp();
        builder.emit(
            MirInstrKind::UnwrapErr {
                dest: err_dest,
                value: MirOperand::Temp(ffi_dest),
            },
            span,
        );
        let wrapped_err = builder.new_temp();
        builder.emit(
            MirInstrKind::WrapErr {
                dest: wrapped_err,
                value: MirOperand::Temp(err_dest),
            },
            span,
        );
        builder.set_terminator(MirTerminator::Return {
            values: vec![MirOperand::Temp(wrapped_err)],
        });
    } else {
        // No error type — emit a panic so the program aborts on DB error.
        let err_dest = builder.new_temp();
        builder.emit(
            MirInstrKind::UnwrapErr {
                dest: err_dest,
                value: MirOperand::Temp(ffi_dest),
            },
            span,
        );
        builder.emit(
            MirInstrKind::Panic {
                message: MirOperand::Temp(err_dest),
            },
            span,
        );
        builder.set_terminator(MirTerminator::Unreachable);
    }

    // Continuation block: value is the unwrapped JSON string pointer.
    builder.add_block(cont_label);

    Some(MirOperand::Temp(unwrapped))
}

/// Emit `CREATE TABLE IF NOT EXISTS` for the model used in this query chain.
///
/// This is intentionally generated from compiler-known struct metadata, so table
/// shape stays in sync with the Doo model definition without hardcoded SQL.
fn emit_model_auto_migration(
    builder: &mut MirBuilder,
    db_op: &MirOperand,
    model_name: &str,
    span: MirSpan,
) {
    let Some(create_sql) = build_create_table_sql(builder, model_name) else {
        return;
    };

    builder.emit(
        MirInstrKind::FfiCall {
            dest: None,
            lib: sym(ffi_names::LIB_DOO_DB),
            symbol: sym(ffi_names::DOO_DB_RAW),
            args: vec![db_op.clone(), MirOperand::Const(MirConst::Str(create_sql))],
        },
        span,
    );
}

/// Build PostgreSQL `CREATE TABLE IF NOT EXISTS ...` SQL from model metadata.
fn build_create_table_sql(builder: &MirBuilder, model_name: &str) -> Option<String> {
    let meta = builder.struct_metas.get(model_name)?;
    let table_name = resolve_table_name(model_name, builder);

    let struct_type_id = builder.type_registry.lookup(model_name)?;
    let struct_fields = match builder.type_registry.get(struct_type_id) {
        Some(info) => match &info.kind {
            TypeKind::Struct { fields, .. } => fields,
            _ => return None,
        },
        None => return None,
    };

    let mut column_defs: Vec<String> = Vec::new();

    for field in &meta.fields {
        let field_type_id = struct_fields.iter().find_map(|(name, type_id, _)| {
            if name == &field.name {
                Some(*type_id)
            } else {
                None
            }
        })?;

        let (sql_type, nullable) = map_type_to_sql(builder, field_type_id);

        let mut col = String::new();
        col.push_str(&field.name);
        col.push(' ');

        if field.is_primary && field.is_auto {
            col.push_str("BIGSERIAL PRIMARY KEY");
        } else {
            col.push_str(&sql_type);
            if field.is_primary {
                col.push_str(" PRIMARY KEY");
            }
            if !nullable {
                col.push_str(" NOT NULL");
            }
        }

        column_defs.push(col);
    }

    if column_defs.is_empty() {
        return None;
    }

    Some(format!(
        "CREATE TABLE IF NOT EXISTS {} ({})",
        table_name,
        column_defs.join(", ")
    ))
}

/// Convert a Doo type to PostgreSQL column type and nullability.
fn map_type_to_sql(builder: &MirBuilder, type_id: doo_core::types::TypeId) -> (String, bool) {
    let mut nullable = false;
    let mut current = type_id;

    loop {
        let Some(info) = builder.type_registry.get(current) else {
            return ("TEXT".to_string(), nullable);
        };
        match &info.kind {
            TypeKind::Optional { inner } => {
                nullable = true;
                current = *inner;
            }
            TypeKind::Bool => return ("BOOLEAN".to_string(), nullable),
            TypeKind::Int => return ("BIGINT".to_string(), nullable),
            TypeKind::Float => return ("DOUBLE PRECISION".to_string(), nullable),
            TypeKind::Str => return ("TEXT".to_string(), nullable),
            _ => return ("TEXT".to_string(), nullable),
        }
    }
}

// ============================================================================
// Chain Extraction
// ============================================================================

/// A single clause extracted from the HIR chain.
#[derive(Debug)]
enum ChainClause<'a> {
    Where(&'a [HirExpr]),
    OrWhere(&'a [HirExpr]),
    WhereNot(&'a [HirExpr]),
    WhereIn(&'a [HirExpr]),
    WhereNull(&'a [HirExpr]),
    WhereNotNull(&'a [HirExpr]),
    WhereBetween(&'a [HirExpr]),
    OrderBy(&'a [HirExpr]),
    Limit(&'a HirExpr),
    Offset(&'a HirExpr),
    Select(&'a [HirExpr]),
    Exclude(&'a [HirExpr]),
    Distinct,
    GroupBy(&'a [HirExpr]),
    Having(&'a [HirExpr]),
    Aggregate(&'a [HirExpr]),
    Set(&'a [HirExpr]),
    Increment(&'a [HirExpr]),
    Decrement(&'a [HirExpr]),
    Returning(&'a [HirExpr]),
}

/// Extracted query chain structure.
struct QueryChain<'a> {
    /// Terminal method: "exec", "execOne", or "toSql".
    terminal: &'a str,
    /// Query operation: "find", "findOne", "count", etc.
    operation: &'a str,
    /// Model name (e.g., "Task").
    model_name: &'a str,
    /// Optional insert/insertMany data argument.
    data_arg: Option<&'a HirExpr>,
    /// The db receiver expression.
    db_expr: &'a HirExpr,
    /// Clauses in chain order (outermost first).
    clauses: Vec<ChainClause<'a>>,
    /// Span from the terminal expression.
    span: CoreSpan,
}

/// Walk the HIR chain from terminal down to entry, collecting clauses.
/// Returns `None` if this is not a recognizable query chain.
fn extract_chain(expr: &HirExpr) -> Option<QueryChain<'_>> {
    // The outermost node must be a MethodCall with a terminal method.
    let (terminal, chain_head, span) = match &expr.kind {
        HirExprKind::MethodCall {
            receiver,
            method,
            args,
        } if ffi_names::is_query_terminal(method) && args.is_empty() => {
            (method.as_str(), receiver.as_ref(), expr.span)
        }
        _ => return None,
    };

    let mut clauses: Vec<ChainClause<'_>> = Vec::new();
    let mut current = chain_head;

    // Walk from the terminal inward, collecting clauses.
    loop {
        match &current.kind {
            HirExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                let m = method.as_str();

                if ffi_names::is_query_entry(m) {
                    // Found the entry point.
                    let model_name = extract_model_name(args)?;
                    let data_arg = if args.len() > 1 { Some(&args[1]) } else { None };
                    return Some(QueryChain {
                        terminal,
                        operation: m,
                        model_name,
                        data_arg,
                        db_expr: receiver.as_ref(),
                        clauses,
                        span,
                    });
                }

                // Collect the clause.
                let clause = match m {
                    ffi_names::QB_CHAIN_WHERE => ChainClause::Where(args),
                    ffi_names::QB_CHAIN_OR_WHERE => ChainClause::OrWhere(args),
                    ffi_names::QB_CHAIN_WHERE_NOT => ChainClause::WhereNot(args),
                    ffi_names::QB_CHAIN_WHERE_IN => ChainClause::WhereIn(args),
                    ffi_names::QB_CHAIN_WHERE_NULL => ChainClause::WhereNull(args),
                    ffi_names::QB_CHAIN_WHERE_NOT_NULL => ChainClause::WhereNotNull(args),
                    ffi_names::QB_CHAIN_WHERE_BETWEEN => ChainClause::WhereBetween(args),
                    ffi_names::QB_CHAIN_ORDER_BY => ChainClause::OrderBy(args),
                    ffi_names::QB_CHAIN_LIMIT => {
                        let arg = args.first()?;
                        ChainClause::Limit(arg)
                    }
                    ffi_names::QB_CHAIN_OFFSET => {
                        let arg = args.first()?;
                        ChainClause::Offset(arg)
                    }
                    ffi_names::QB_CHAIN_SELECT => ChainClause::Select(args),
                    ffi_names::QB_CHAIN_EXCLUDE => ChainClause::Exclude(args),
                    ffi_names::QB_CHAIN_DISTINCT => ChainClause::Distinct,
                    ffi_names::QB_CHAIN_GROUP_BY => ChainClause::GroupBy(args),
                    ffi_names::QB_CHAIN_HAVING => ChainClause::Having(args),
                    ffi_names::QB_CHAIN_AGGREGATE => ChainClause::Aggregate(args),
                    ffi_names::QB_CHAIN_SET => ChainClause::Set(args),
                    ffi_names::QB_CHAIN_INCREMENT => ChainClause::Increment(args),
                    ffi_names::QB_CHAIN_DECREMENT => ChainClause::Decrement(args),
                    ffi_names::QB_CHAIN_RETURNING => ChainClause::Returning(args),
                    _ => return None, // Unknown method — not a QB chain
                };

                clauses.push(clause);
                current = receiver.as_ref();
            }
            _ => return None,
        }
    }
}

/// Extract the model name from the entry method args.
/// E.g., `find(Task)` → args[0] is `Local("Task")` → "Task".
fn extract_model_name(args: &[HirExpr]) -> Option<&str> {
    args.first().and_then(|a| {
        if let HirExprKind::Local { name } = &a.kind {
            Some(name.as_str())
        } else {
            None
        }
    })
}

// ============================================================================
// SQL Generation
// ============================================================================

/// SQL builder state — maintains param index counter and collects param exprs.
struct SqlState<'a> {
    /// Next `$N` param index (1-based).
    param_idx: usize,
    /// Param HIR expressions in `$N` order.
    param_exprs: Vec<&'a HirExpr>,
}

impl<'a> SqlState<'a> {
    fn new() -> Self {
        Self {
            param_idx: 1,
            param_exprs: Vec::new(),
        }
    }

    /// Register a param expression, returning its `$N` placeholder.
    fn push_param(&mut self, expr: &'a HirExpr) -> String {
        let placeholder = format!("${}", self.param_idx);
        self.param_idx += 1;
        self.param_exprs.push(expr);
        placeholder
    }
}

/// Build the SQL string and collect param expressions.
///
/// Returns `(sql, param_exprs, Option<error>)`.
fn build_sql<'a>(
    chain: &'a QueryChain<'a>,
    builder: &MirBuilder,
) -> (String, Vec<&'a HirExpr>, Option<CompilerError>) {
    // Resolve table name from struct metadata.
    let table_name = resolve_table_name(chain.model_name, builder);

    // Validate: model must exist (as a struct in the program).
    if !builder.struct_metas.contains_key(chain.model_name) {
        let err = CompilerError::new(
            ErrorCode::QueryBuilderUnknownModel,
            format!(
                "query builder: unknown model '{}' — no struct with that name found",
                chain.model_name
            ),
            chain.span,
        );
        return (String::new(), Vec::new(), Some(err));
    }

    // Validate referenced fields and operator/type semantics against the model struct.
    if let Some(meta) = builder.struct_metas.get(chain.model_name) {
        let model_fields: HashSet<&str> = meta.fields.iter().map(|f| f.name.as_str()).collect();
        let mut model_field_types: HashMap<&str, doo_core::types::TypeId> = HashMap::new();
        if let Some(struct_type_id) = builder.type_registry.lookup(chain.model_name) {
            if let Some(info) = builder.type_registry.get(struct_type_id) {
                if let TypeKind::Struct { fields, .. } = &info.kind {
                    for (name, tid, _) in fields {
                        model_field_types.insert(name.as_str(), *tid);
                    }
                }
            }
        }

        if let Some(err) = validate_plan_fields(chain, &model_fields) {
            return (String::new(), Vec::new(), Some(err));
        }
        if let Some(err) = validate_plan_types(chain, &model_field_types, builder) {
            return (String::new(), Vec::new(), Some(err));
        }
    }

    let mut state = SqlState::new();

    let sql = match chain.operation {
        ffi_names::QB_ENTRY_FIND | ffi_names::QB_ENTRY_FIND_ONE => build_select_sql(
            chain,
            &table_name,
            &mut state,
            chain.terminal == ffi_names::QB_TERMINAL_EXEC_ONE
                || chain.operation == ffi_names::QB_ENTRY_FIND_ONE,
        ),
        ffi_names::QB_ENTRY_COUNT => build_count_sql(chain, &table_name, &mut state),
        ffi_names::QB_ENTRY_INSERT => {
            build_insert_sql(chain, &table_name, &mut state, builder, false)
        }
        ffi_names::QB_ENTRY_INSERT_MANY => {
            build_insert_sql(chain, &table_name, &mut state, builder, true)
        }
        ffi_names::QB_ENTRY_UPDATE => build_update_sql(chain, &table_name, &mut state, builder),
        ffi_names::QB_ENTRY_DELETE => build_delete_sql(chain, &table_name, &mut state),
        _ => String::from("SELECT 1"),
    };

    // Validate: update/delete must have a WHERE clause.
    if matches!(
        chain.operation,
        ffi_names::QB_ENTRY_UPDATE | ffi_names::QB_ENTRY_DELETE
    ) {
        let has_filter = chain.clauses.iter().any(|c| {
            matches!(
                c,
                ChainClause::Where(_)
                    | ChainClause::OrWhere(_)
                    | ChainClause::WhereNot(_)
                    | ChainClause::WhereIn(_)
                    | ChainClause::WhereNull(_)
                    | ChainClause::WhereNotNull(_)
                    | ChainClause::WhereBetween(_)
            )
        });
        if !has_filter {
            let err = CompilerError::new(
                ErrorCode::QueryBuilderMissingWhere,
                format!(
                    "query builder: '{}' requires at least one filter clause (.where/.whereIn/.whereBetween/...) to prevent accidental bulk operations",
                    chain.operation
                ),
                chain.span,
            );
            return (sql, state.param_exprs, Some(err));
        }
    }

    (sql, state.param_exprs, None)
}

/// Validate field references in query clauses against model fields.
fn validate_plan_fields(
    chain: &QueryChain<'_>,
    model_fields: &HashSet<&str>,
) -> Option<CompilerError> {
    for clause in &chain.clauses {
        match clause {
            ChainClause::Where(args) | ChainClause::OrWhere(args) | ChainClause::WhereNot(args) => {
                let Some(obj) = args.first() else { continue };
                if let HirExprKind::Struct { fields, .. } = &obj.kind {
                    for (field_name, _) in fields {
                        if !model_fields.contains(field_name.as_str()) {
                            return Some(CompilerError::new(
                                ErrorCode::QueryBuilderUnknownField,
                                format!(
                                    "query builder: unknown field '{}' on model '{}'",
                                    field_name, chain.model_name
                                ),
                                chain.span,
                            ));
                        }
                    }
                }
            }
            ChainClause::WhereIn(args) | ChainClause::WhereBetween(args) => {
                let Some(first) = args.first() else { continue };
                if let Some(field_name) = extract_string_lit(first) {
                    if !model_fields.contains(field_name) {
                        return Some(CompilerError::new(
                            ErrorCode::QueryBuilderUnknownField,
                            format!(
                                "query builder: unknown field '{}' on model '{}'",
                                field_name, chain.model_name
                            ),
                            chain.span,
                        ));
                    }
                }
            }
            ChainClause::WhereNull(args) | ChainClause::WhereNotNull(args) => {
                for arg in args.iter() {
                    if let Some(field_name) = extract_string_lit(arg) {
                        if !model_fields.contains(field_name) {
                            return Some(CompilerError::new(
                                ErrorCode::QueryBuilderUnknownField,
                                format!(
                                    "query builder: unknown field '{}' on model '{}'",
                                    field_name, chain.model_name
                                ),
                                chain.span,
                            ));
                        }
                    } else if let HirExprKind::Array(items) = &arg.kind {
                        for item in items {
                            if let Some(field_name) = extract_string_lit(item) {
                                if !model_fields.contains(field_name) {
                                    return Some(CompilerError::new(
                                        ErrorCode::QueryBuilderUnknownField,
                                        format!(
                                            "query builder: unknown field '{}' on model '{}'",
                                            field_name, chain.model_name
                                        ),
                                        chain.span,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            ChainClause::OrderBy(args) | ChainClause::Set(args) => {
                let Some(obj) = args.first() else { continue };
                if let HirExprKind::Struct { fields, .. } = &obj.kind {
                    for (field_name, _) in fields {
                        if !model_fields.contains(field_name.as_str()) {
                            return Some(CompilerError::new(
                                ErrorCode::QueryBuilderUnknownField,
                                format!(
                                    "query builder: unknown field '{}' on model '{}'",
                                    field_name, chain.model_name
                                ),
                                chain.span,
                            ));
                        }
                    }
                }
            }
            ChainClause::Select(args)
            | ChainClause::Exclude(args)
            | ChainClause::GroupBy(args)
            | ChainClause::Returning(args) => {
                let fields = extract_field_names(args);
                for field_name in fields {
                    if field_name != "*" && !model_fields.contains(field_name) {
                        return Some(CompilerError::new(
                            ErrorCode::QueryBuilderUnknownField,
                            format!(
                                "query builder: unknown field '{}' on model '{}'",
                                field_name, chain.model_name
                            ),
                            chain.span,
                        ));
                    }
                }
            }
            ChainClause::Increment(args) | ChainClause::Decrement(args) => {
                let Some(first) = args.first() else { continue };
                if let Some(field_name) = extract_string_lit(first) {
                    if !model_fields.contains(field_name) {
                        return Some(CompilerError::new(
                            ErrorCode::QueryBuilderUnknownField,
                            format!(
                                "query builder: unknown field '{}' on model '{}'",
                                field_name, chain.model_name
                            ),
                            chain.span,
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn validate_plan_types(
    chain: &QueryChain<'_>,
    model_field_types: &HashMap<&str, doo_core::types::TypeId>,
    builder: &MirBuilder,
) -> Option<CompilerError> {
    for clause in &chain.clauses {
        match clause {
            ChainClause::Where(args) | ChainClause::OrWhere(args) | ChainClause::WhereNot(args) => {
                let Some(obj) = args.first() else { continue };
                if let HirExprKind::Struct { fields, .. } = &obj.kind {
                    for (field_name, val_expr) in fields {
                        let Some(field_tid) = model_field_types.get(field_name.as_str()) else {
                            continue;
                        };
                        if let Some(err) = validate_condition_value_type(
                            chain, field_name, *field_tid, val_expr, builder,
                        ) {
                            return Some(err);
                        }
                    }
                }
            }
            ChainClause::WhereIn(args) => {
                if args.len() >= 2 {
                    if let Some(field_name) = extract_string_lit(&args[0]) {
                        if let Some(field_tid) = model_field_types.get(field_name) {
                            if let Some(err) = validate_list_compat(
                                chain, field_name, *field_tid, &args[1], builder,
                            ) {
                                return Some(err);
                            }
                        }
                    }
                }
            }
            ChainClause::WhereBetween(args) => {
                if args.len() >= 3 {
                    if let Some(field_name) = extract_string_lit(&args[0]) {
                        if let Some(field_tid) = model_field_types.get(field_name) {
                            if !is_numeric_type(*field_tid, builder) {
                                return Some(qb_invalid_chain(
                                    chain,
                                    format!("query builder: whereBetween requires numeric field '{}', got {}", field_name, type_name_of(*field_tid, builder)),
                                ));
                            }
                            if let Some(err) =
                                validate_numeric_expr(chain, field_name, &args[1], builder)
                            {
                                return Some(err);
                            }
                            if let Some(err) =
                                validate_numeric_expr(chain, field_name, &args[2], builder)
                            {
                                return Some(err);
                            }
                        }
                    }
                }
            }
            ChainClause::Increment(args) | ChainClause::Decrement(args) => {
                if args.len() >= 2 {
                    if let Some(field_name) = extract_string_lit(&args[0]) {
                        if let Some(field_tid) = model_field_types.get(field_name) {
                            if !is_numeric_type(*field_tid, builder) {
                                return Some(qb_invalid_chain(
                                    chain,
                                    format!("query builder: numeric mutation requires numeric field '{}', got {}", field_name, type_name_of(*field_tid, builder)),
                                ));
                            }
                            if let Some(err) =
                                validate_numeric_expr(chain, field_name, &args[1], builder)
                            {
                                return Some(err);
                            }
                        }
                    }
                }
            }
            ChainClause::Aggregate(args) => {
                let Some(obj) = args.first() else { continue };
                if let HirExprKind::Struct { fields, .. } = &obj.kind {
                    for (_alias, fn_expr) in fields {
                        if let HirExprKind::Call {
                            func,
                            args: fn_args,
                        } = &fn_expr.kind
                        {
                            let HirExprKind::Local { name: agg_fn } = &func.kind else {
                                continue;
                            };
                            let is_supported =
                                matches!(agg_fn.as_str(), "Count" | "Sum" | "Avg" | "Min" | "Max");
                            if !is_supported {
                                return Some(qb_invalid_chain(
                                    chain,
                                    format!(
                                        "query builder: unsupported aggregate function '{}'",
                                        agg_fn
                                    ),
                                ));
                            }

                            let target_col = fn_args
                                .first()
                                .and_then(|a| extract_string_lit(a))
                                .unwrap_or("*");
                            if target_col != "*" {
                                let Some(field_tid) = model_field_types.get(target_col) else {
                                    continue;
                                };
                                if matches!(agg_fn.as_str(), "Sum" | "Avg")
                                    && !is_numeric_type(*field_tid, builder)
                                {
                                    return Some(qb_invalid_chain(
                                        chain,
                                        format!(
                                            "query builder: {} requires numeric field '{}', got {}",
                                            agg_fn,
                                            target_col,
                                            type_name_of(*field_tid, builder)
                                        ),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn validate_condition_value_type(
    chain: &QueryChain<'_>,
    field_name: &str,
    field_tid: doo_core::types::TypeId,
    val_expr: &HirExpr,
    builder: &MirBuilder,
) -> Option<CompilerError> {
    match &val_expr.kind {
        HirExprKind::Local { name } => {
            if matches!(
                name.as_str(),
                ffi_names::QB_OP_IS_NULL | ffi_names::QB_OP_IS_NOT_NULL
            ) {
                return None;
            }
            None
        }
        HirExprKind::Call { func, args } => {
            if let HirExprKind::Local { name: op } = &func.kind {
                match op.as_str() {
                    ffi_names::QB_OP_GT
                    | ffi_names::QB_OP_GTE
                    | ffi_names::QB_OP_LT
                    | ffi_names::QB_OP_LTE => {
                        if !is_numeric_type(field_tid, builder) {
                            return Some(qb_invalid_chain(
                                chain,
                                format!(
                                    "query builder: '{}' requires numeric field '{}'",
                                    op, field_name
                                ),
                            ));
                        }
                        if let Some(a) = args.first() {
                            return validate_numeric_expr(chain, field_name, a, builder);
                        }
                    }
                    ffi_names::QB_OP_LIKE | ffi_names::QB_OP_ILIKE => {
                        if !is_string_type(field_tid, builder) {
                            return Some(qb_invalid_chain(
                                chain,
                                format!(
                                    "query builder: '{}' requires string field '{}'",
                                    op, field_name
                                ),
                            ));
                        }
                        if let Some(a) = args.first() {
                            return validate_string_expr(chain, field_name, a, builder);
                        }
                    }
                    ffi_names::QB_OP_IN | ffi_names::QB_OP_NOT_IN => {
                        if let Some(a) = args.first() {
                            return validate_list_compat(chain, field_name, field_tid, a, builder);
                        }
                    }
                    ffi_names::QB_OP_BETWEEN => {
                        if !is_numeric_type(field_tid, builder) {
                            return Some(qb_invalid_chain(
                                chain,
                                format!(
                                    "query builder: Between requires numeric field '{}'",
                                    field_name
                                ),
                            ));
                        }
                        if args.len() >= 2 {
                            if let Some(err) =
                                validate_numeric_expr(chain, field_name, &args[0], builder)
                            {
                                return Some(err);
                            }
                            if let Some(err) =
                                validate_numeric_expr(chain, field_name, &args[1], builder)
                            {
                                return Some(err);
                            }
                        }
                    }
                    _ => {
                        if let Some(et) = expr_primitive_type(val_expr, builder) {
                            let ft = normalize_optional(field_tid, builder);
                            if !primitive_compatible(ft, et) {
                                return Some(qb_invalid_chain(
                                    chain,
                                    format!("query builder: type mismatch on '{}': expected {}, found {}", field_name, type_name_of(field_tid, builder), type_name_of(et, builder)),
                                ));
                            }
                        }
                    }
                }
            }
            None
        }
        _ => {
            if let Some(et) = expr_primitive_type(val_expr, builder) {
                let ft = normalize_optional(field_tid, builder);
                if !primitive_compatible(ft, et) {
                    return Some(qb_invalid_chain(
                        chain,
                        format!(
                            "query builder: type mismatch on '{}': expected {}, found {}",
                            field_name,
                            type_name_of(field_tid, builder),
                            type_name_of(et, builder)
                        ),
                    ));
                }
            }
            None
        }
    }
}

fn validate_list_compat(
    chain: &QueryChain<'_>,
    field_name: &str,
    field_tid: doo_core::types::TypeId,
    arg_expr: &HirExpr,
    builder: &MirBuilder,
) -> Option<CompilerError> {
    if let HirExprKind::Array(items) = &arg_expr.kind {
        let field_t = normalize_optional(field_tid, builder);
        for item in items {
            if let Some(item_t) = expr_primitive_type(item, builder) {
                if !primitive_compatible(field_t, item_t) {
                    return Some(qb_invalid_chain(
                        chain,
                        format!(
                            "query builder: list item type mismatch on '{}': expected {}, found {}",
                            field_name,
                            type_name_of(field_tid, builder),
                            type_name_of(item_t, builder)
                        ),
                    ));
                }
            }
        }
    }
    None
}

fn validate_numeric_expr(
    chain: &QueryChain<'_>,
    field_name: &str,
    expr: &HirExpr,
    builder: &MirBuilder,
) -> Option<CompilerError> {
    if let Some(tid) = expr_primitive_type(expr, builder) {
        if !is_numeric_type(tid, builder) {
            return Some(qb_invalid_chain(
                chain,
                format!(
                    "query builder: numeric value required for '{}', found {}",
                    field_name,
                    type_name_of(tid, builder)
                ),
            ));
        }
    }
    None
}

fn validate_string_expr(
    chain: &QueryChain<'_>,
    field_name: &str,
    expr: &HirExpr,
    builder: &MirBuilder,
) -> Option<CompilerError> {
    if let Some(tid) = expr_primitive_type(expr, builder) {
        if !is_string_type(tid, builder) {
            return Some(qb_invalid_chain(
                chain,
                format!(
                    "query builder: string value required for '{}', found {}",
                    field_name,
                    type_name_of(tid, builder)
                ),
            ));
        }
    }
    None
}

fn expr_primitive_type(expr: &HirExpr, builder: &MirBuilder) -> Option<doo_core::types::TypeId> {
    if let Some(tid) = expr.type_id {
        let normalized = normalize_optional(tid, builder);
        if is_primitive(normalized, builder) {
            return Some(normalized);
        }
    }
    match &expr.kind {
        HirExprKind::Const(ConstValue::Int(_)) => Some(builtin::INT),
        HirExprKind::Const(ConstValue::Float(_)) => Some(builtin::FLOAT),
        HirExprKind::Const(ConstValue::Bool(_)) => Some(builtin::BOOL),
        HirExprKind::Const(ConstValue::Str(_)) => Some(builtin::STR),
        _ => None,
    }
}

fn normalize_optional(
    tid: doo_core::types::TypeId,
    builder: &MirBuilder,
) -> doo_core::types::TypeId {
    let mut cur = tid;
    loop {
        let Some(info) = builder.type_registry.get(cur) else {
            return cur;
        };
        match &info.kind {
            TypeKind::Optional { inner } => cur = *inner,
            _ => return cur,
        }
    }
}

fn is_primitive(tid: doo_core::types::TypeId, builder: &MirBuilder) -> bool {
    let Some(info) = builder.type_registry.get(tid) else {
        return false;
    };
    matches!(
        info.kind,
        TypeKind::Int | TypeKind::Float | TypeKind::Bool | TypeKind::Str
    )
}

fn is_numeric_type(tid: doo_core::types::TypeId, builder: &MirBuilder) -> bool {
    let n = normalize_optional(tid, builder);
    let Some(info) = builder.type_registry.get(n) else {
        return false;
    };
    matches!(info.kind, TypeKind::Int | TypeKind::Float)
}

fn is_string_type(tid: doo_core::types::TypeId, builder: &MirBuilder) -> bool {
    let n = normalize_optional(tid, builder);
    let Some(info) = builder.type_registry.get(n) else {
        return false;
    };
    matches!(info.kind, TypeKind::Str)
}

fn primitive_compatible(expected: doo_core::types::TypeId, found: doo_core::types::TypeId) -> bool {
    expected == found
}

fn type_name_of(tid: doo_core::types::TypeId, builder: &MirBuilder) -> String {
    builder
        .type_registry
        .get(tid)
        .map(|i| i.name.clone())
        .unwrap_or_else(|| "Unknown".to_string())
}

fn qb_invalid_chain(chain: &QueryChain<'_>, message: String) -> CompilerError {
    CompilerError::new(ErrorCode::QueryBuilderInvalidChain, message, chain.span)
}

// ---- SELECT ----------------------------------------------------------------

fn build_select_sql<'a>(
    chain: &'a QueryChain<'a>,
    table: &str,
    state: &mut SqlState<'a>,
    limit_one: bool,
) -> String {
    let mut sql = String::new();

    // SELECT [DISTINCT] [cols|aggregate]
    let distinct = chain
        .clauses
        .iter()
        .any(|c| matches!(c, ChainClause::Distinct));
    let select_cols = chain.clauses.iter().find_map(|c| {
        if let ChainClause::Select(args) = c {
            Some(*args)
        } else {
            None
        }
    });
    let exclude_cols: Vec<&str> = chain
        .clauses
        .iter()
        .find_map(|c| {
            if let ChainClause::Exclude(args) = c {
                Some(*args)
            } else {
                None
            }
        })
        .map(|args| extract_field_names(args))
        .unwrap_or_default();
    let aggregate = chain.clauses.iter().find_map(|c| {
        if let ChainClause::Aggregate(args) = c {
            Some(*args)
        } else {
            None
        }
    });

    if let Some(agg_args) = aggregate {
        // .aggregate({ count: Count("*"), sum: Sum("amount") })
        let agg_cols = build_aggregate_cols(agg_args);
        sql.push_str("SELECT ");
        sql.push_str(&agg_cols);
    } else if let Some(cols) = select_cols {
        // .select(["col1", "col2"])
        let names = extract_string_list(cols);
        let cols_str = if names.is_empty() {
            "*".to_string()
        } else {
            names.join(", ")
        };
        sql.push_str(if distinct {
            "SELECT DISTINCT "
        } else {
            "SELECT "
        });
        sql.push_str(&cols_str);
    } else if !exclude_cols.is_empty() {
        // .exclude(["col1"]) — we can't easily do this in SQL without knowing all cols,
        // so we fall back to SELECT * and note this is best-effort.
        sql.push_str(if distinct {
            "SELECT DISTINCT *"
        } else {
            "SELECT *"
        });
    } else {
        sql.push_str(if distinct {
            "SELECT DISTINCT *"
        } else {
            "SELECT *"
        });
    }

    sql.push_str(" FROM ");
    sql.push_str(table);

    // WHERE
    append_where_clauses(chain, state, &mut sql);

    // GROUP BY
    if let Some(group_args) = chain.clauses.iter().find_map(|c| {
        if let ChainClause::GroupBy(a) = c {
            Some(*a)
        } else {
            None
        }
    }) {
        let cols = extract_string_list(group_args);
        if !cols.is_empty() {
            sql.push_str(" GROUP BY ");
            sql.push_str(&cols.join(", "));
        }
    }

    // HAVING
    if let Some(having_args) = chain.clauses.iter().find_map(|c| {
        if let ChainClause::Having(a) = c {
            Some(*a)
        } else {
            None
        }
    }) {
        if !having_args.is_empty() {
            sql.push_str(" HAVING ");
            let conds = build_conditions(having_args, state, "AND");
            sql.push_str(&conds);
        }
    }

    // ORDER BY
    if let Some(order_args) = chain.clauses.iter().find_map(|c| {
        if let ChainClause::OrderBy(a) = c {
            Some(*a)
        } else {
            None
        }
    }) {
        let order_str = build_order_by(order_args);
        if !order_str.is_empty() {
            sql.push_str(" ORDER BY ");
            sql.push_str(&order_str);
        }
    }

    // LIMIT (execOne forces LIMIT 1)
    if limit_one {
        sql.push_str(" LIMIT 1");
    } else if let Some(limit_expr) = chain.clauses.iter().find_map(|c| {
        if let ChainClause::Limit(e) = c {
            Some(*e)
        } else {
            None
        }
    }) {
        match &limit_expr.kind {
            HirExprKind::Const(ConstValue::Int(n)) => {
                sql.push_str(&format!(" LIMIT {}", n));
            }
            _ => {
                let ph = state.push_param(limit_expr);
                sql.push_str(&format!(" LIMIT {}", ph));
            }
        }
    }

    // OFFSET
    if let Some(offset_expr) = chain.clauses.iter().find_map(|c| {
        if let ChainClause::Offset(e) = c {
            Some(*e)
        } else {
            None
        }
    }) {
        match &offset_expr.kind {
            HirExprKind::Const(ConstValue::Int(n)) => {
                sql.push_str(&format!(" OFFSET {}", n));
            }
            _ => {
                let ph = state.push_param(offset_expr);
                sql.push_str(&format!(" OFFSET {}", ph));
            }
        }
    }

    sql
}

// ---- COUNT -----------------------------------------------------------------

fn build_count_sql<'a>(chain: &'a QueryChain<'a>, table: &str, state: &mut SqlState<'a>) -> String {
    let mut sql = format!("SELECT COUNT(*) FROM {}", table);
    append_where_clauses(chain, state, &mut sql);
    sql
}

// ---- INSERT ----------------------------------------------------------------

fn build_insert_sql<'a>(
    chain: &'a QueryChain<'a>,
    table: &str,
    state: &mut SqlState<'a>,
    builder: &MirBuilder,
    many: bool,
) -> String {
    // Get non-auto fields from struct metadata.
    let meta = builder.struct_metas.get(chain.model_name);
    let insert_fields: Vec<&str> = if let Some(m) = meta {
        m.fields
            .iter()
            .filter(|f| !f.is_auto)
            .map(|f| f.name.as_str())
            .collect()
    } else {
        Vec::new()
    };

    if insert_fields.is_empty() {
        return format!("INSERT INTO {} DEFAULT VALUES RETURNING *", table);
    }

    let cols = insert_fields.join(", ");

    // Build VALUES from data_arg (object literal or array of object literals).
    let rows = if many {
        build_insert_many_values(chain.data_arg, &insert_fields, state)
    } else {
        build_insert_one_values(chain.data_arg, &insert_fields, state)
    };

    // Check RETURNING clause override.
    let returning = build_returning_clause(chain);
    format!(
        "INSERT INTO {} ({}) VALUES {} {}",
        table, cols, rows, returning
    )
}

fn build_insert_one_values<'a>(
    data_arg: Option<&'a HirExpr>,
    fields: &[&str],
    state: &mut SqlState<'a>,
) -> String {
    let Some(data) = data_arg else {
        // No data — generate placeholders for all fields.
        let placeholders: Vec<String> = fields
            .iter()
            .map(|_| {
                let p = format!("${}", state.param_idx);
                state.param_idx += 1;
                p
            })
            .collect();
        return format!("({})", placeholders.join(", "));
    };

    match &data.kind {
        HirExprKind::Struct {
            fields: field_exprs,
            ..
        } => {
            // Object literal: { title: "foo", status: "bar" }
            // Use field order from the struct definition (insert_fields) for ordering.
            let mut placeholders: Vec<String> = Vec::new();
            for &field_name in fields {
                let matching = field_exprs.iter().find(|(k, _)| k == field_name);
                if let Some((_, val_expr)) = matching {
                    placeholders.push(state.push_param(val_expr));
                } else {
                    // Field not in data — use NULL.
                    placeholders.push("NULL".to_string());
                }
            }
            format!("({})", placeholders.join(", "))
        }
        _ => {
            // Single non-object arg — generate one placeholder.
            let ph = state.push_param(data);
            format!("({})", ph)
        }
    }
}

fn build_insert_many_values<'a>(
    data_arg: Option<&'a HirExpr>,
    fields: &[&str],
    state: &mut SqlState<'a>,
) -> String {
    let Some(data) = data_arg else {
        return format!(
            "({})",
            fields
                .iter()
                .map(|_| {
                    let p = format!("${}", state.param_idx);
                    state.param_idx += 1;
                    p
                })
                .collect::<Vec<_>>()
                .join(", ")
        );
    };

    if let HirExprKind::Array(rows) = &data.kind {
        let row_strs: Vec<String> = rows
            .iter()
            .map(|row| build_insert_one_values(Some(row), fields, state))
            .collect();
        row_strs.join(", ")
    } else {
        build_insert_one_values(Some(data), fields, state)
    }
}

// ---- UPDATE ----------------------------------------------------------------

fn build_update_sql<'a>(
    chain: &'a QueryChain<'a>,
    table: &str,
    state: &mut SqlState<'a>,
    _builder: &MirBuilder,
) -> String {
    let mut sql = format!("UPDATE {} SET ", table);

    // SET clause — collect values FIRST (they appear before WHERE in SQL).
    let set_str = if let Some(set_args) = chain.clauses.iter().find_map(|c| {
        if let ChainClause::Set(a) = c {
            Some(*a)
        } else {
            None
        }
    }) {
        build_set_clause(set_args, state)
    } else {
        String::new()
    };

    // Increment/decrement assignments.
    let mut extra_sets: Vec<String> = Vec::new();
    for clause in &chain.clauses {
        match clause {
            ChainClause::Increment(args) => {
                if let Some(inc_str) = build_increment_clause(args, state, "+") {
                    extra_sets.push(inc_str);
                }
            }
            ChainClause::Decrement(args) => {
                if let Some(dec_str) = build_increment_clause(args, state, "-") {
                    extra_sets.push(dec_str);
                }
            }
            _ => {}
        }
    }

    let mut all_sets = Vec::new();
    if !set_str.is_empty() {
        all_sets.push(set_str);
    }
    all_sets.extend(extra_sets);
    sql.push_str(&all_sets.join(", "));

    // WHERE clause.
    append_where_clauses(chain, state, &mut sql);

    // RETURNING.
    let returning = build_returning_clause(chain);
    sql.push(' ');
    sql.push_str(&returning);
    sql
}

// ---- DELETE ----------------------------------------------------------------

fn build_delete_sql<'a>(
    chain: &'a QueryChain<'a>,
    table: &str,
    state: &mut SqlState<'a>,
) -> String {
    let mut sql = format!("DELETE FROM {}", table);
    append_where_clauses(chain, state, &mut sql);
    let returning = build_returning_clause(chain);
    sql.push(' ');
    sql.push_str(&returning);
    sql
}

// ============================================================================
// WHERE Clause Building
// ============================================================================

/// Append all WHERE-related clauses (WHERE, OR WHERE, WHERE NOT) to the SQL.
fn append_where_clauses<'a>(chain: &'a QueryChain<'a>, state: &mut SqlState<'a>, sql: &mut String) {
    // Collect all WHERE fragments in chain order.
    let mut where_parts: Vec<String> = Vec::new();

    for clause in &chain.clauses {
        match clause {
            ChainClause::Where(args) => {
                if !args.is_empty() {
                    let cond = build_conditions(args, state, "AND");
                    if !cond.is_empty() {
                        where_parts.push(cond);
                    }
                }
            }
            ChainClause::OrWhere(args) => {
                if !args.is_empty() && !where_parts.is_empty() {
                    let cond = build_conditions(args, state, "AND");
                    if !cond.is_empty() {
                        // Wrap previous parts in parens and add OR.
                        let prev = format!("({})", where_parts.join(" AND "));
                        where_parts = vec![format!("{} OR ({})", prev, cond)];
                    }
                }
            }
            ChainClause::WhereNot(args) => {
                if !args.is_empty() {
                    let cond = build_conditions(args, state, "AND");
                    if !cond.is_empty() {
                        where_parts.push(format!("NOT ({})", cond));
                    }
                }
            }
            ChainClause::WhereIn(args) => {
                if args.len() >= 2 {
                    let field = extract_string_lit(&args[0]).unwrap_or("id");
                    let cond = build_in_condition_expr(field, &args[1], state, false);
                    if !cond.is_empty() {
                        where_parts.push(cond);
                    }
                }
            }
            ChainClause::WhereNull(args) => {
                for a in args.iter() {
                    if let Some(field) = extract_string_lit(a) {
                        where_parts.push(format!("{} IS NULL", field));
                    }
                }
            }
            ChainClause::WhereNotNull(args) => {
                for a in args.iter() {
                    if let Some(field) = extract_string_lit(a) {
                        where_parts.push(format!("{} IS NOT NULL", field));
                    }
                }
            }
            ChainClause::WhereBetween(args) => {
                if args.len() >= 3 {
                    let field = extract_string_lit(&args[0]).unwrap_or("id");
                    let ph1 = state.push_param(&args[1]);
                    let ph2 = state.push_param(&args[2]);
                    where_parts.push(format!("{} BETWEEN {} AND {}", field, ph1, ph2));
                }
            }
            _ => {}
        }
    }

    if !where_parts.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_parts.join(" AND "));
    }
}

/// Build conditions from a struct literal `{ field: value, field2: Op(value) }`.
fn build_conditions<'a>(args: &'a [HirExpr], state: &mut SqlState<'a>, joiner: &str) -> String {
    let Some(obj) = args.first() else {
        return String::new();
    };

    let fields = match &obj.kind {
        HirExprKind::Struct { fields, .. } => fields,
        _ => return String::new(),
    };

    let parts: Vec<String> = fields
        .iter()
        .map(|(field_name, val_expr)| build_condition_expr(field_name, val_expr, state))
        .collect();

    parts.join(&format!(" {} ", joiner))
}

/// Build a single condition `field OP $N` or `field IS NULL` etc.
fn build_condition_expr<'a>(
    field: &str,
    val_expr: &'a HirExpr,
    state: &mut SqlState<'a>,
) -> String {
    match &val_expr.kind {
        // Plain identifier: Gt, Lt, IsNull, IsNotNull, Asc, Desc
        HirExprKind::Local { name } => {
            match name.as_str() {
                ffi_names::QB_OP_IS_NULL => format!("{} IS NULL", field),
                ffi_names::QB_OP_IS_NOT_NULL => format!("{} IS NOT NULL", field),
                _ => format!("{} = {}", field, name), // fallback
            }
        }

        // Operator call: Gt(25), Like("foo%"), In([1,2]), Between(1, 10)
        HirExprKind::Call { func, args } => {
            if let HirExprKind::Local { name: op } = &func.kind {
                match op.as_str() {
                    ffi_names::QB_OP_GT => args
                        .first()
                        .map(|a| format!("{} > {}", field, state.push_param(a)))
                        .unwrap_or_default(),
                    ffi_names::QB_OP_GTE => args
                        .first()
                        .map(|a| format!("{} >= {}", field, state.push_param(a)))
                        .unwrap_or_default(),
                    ffi_names::QB_OP_LT => args
                        .first()
                        .map(|a| format!("{} < {}", field, state.push_param(a)))
                        .unwrap_or_default(),
                    ffi_names::QB_OP_LTE => args
                        .first()
                        .map(|a| format!("{} <= {}", field, state.push_param(a)))
                        .unwrap_or_default(),
                    ffi_names::QB_OP_NE => args
                        .first()
                        .map(|a| format!("{} != {}", field, state.push_param(a)))
                        .unwrap_or_default(),
                    ffi_names::QB_OP_LIKE => args
                        .first()
                        .map(|a| format!("{} LIKE {}", field, state.push_param(a)))
                        .unwrap_or_default(),
                    ffi_names::QB_OP_ILIKE => args
                        .first()
                        .map(|a| format!("{} ILIKE {}", field, state.push_param(a)))
                        .unwrap_or_default(),
                    ffi_names::QB_OP_IN => args
                        .first()
                        .map(|a| build_in_condition_expr(field, a, state, false))
                        .unwrap_or_default(),
                    ffi_names::QB_OP_NOT_IN => args
                        .first()
                        .map(|a| build_in_condition_expr(field, a, state, true))
                        .unwrap_or_default(),
                    ffi_names::QB_OP_BETWEEN => {
                        if args.len() >= 2 {
                            let ph1 = state.push_param(&args[0]);
                            let ph2 = state.push_param(&args[1]);
                            format!("{} BETWEEN {} AND {}", field, ph1, ph2)
                        } else {
                            String::new()
                        }
                    }
                    ffi_names::QB_OP_IS_NULL => format!("{} IS NULL", field),
                    ffi_names::QB_OP_IS_NOT_NULL => format!("{} IS NOT NULL", field),
                    _ => {
                        // Unknown operator — treat as equality with the call result as param.
                        format!("{} = {}", field, state.push_param(val_expr))
                    }
                }
            } else {
                let ph = state.push_param(val_expr);
                format!("{} = {}", field, ph)
            }
        }

        // Literal or local variable — equality.
        _ => {
            let ph = state.push_param(val_expr);
            format!("{} = {}", field, ph)
        }
    }
}

/// Build IN / NOT IN condition.
///
/// For literal arrays, expands to `field IN ($1, $2, ...)` so each value is a
/// scalar param (driver-friendly, no array binding edge cases).
/// For non-literal args, falls back to ANY/ALL with a single array param.
fn build_in_condition_expr<'a>(
    field: &str,
    arg_expr: &'a HirExpr,
    state: &mut SqlState<'a>,
    negate: bool,
) -> String {
    if let HirExprKind::Array(items) = &arg_expr.kind {
        if items.is_empty() {
            return if negate {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            };
        }

        let placeholders: Vec<String> = items.iter().map(|item| state.push_param(item)).collect();
        let op = if negate { "NOT IN" } else { "IN" };
        return format!("{} {} ({})", field, op, placeholders.join(", "));
    }

    let ph = state.push_param(arg_expr);
    if negate {
        format!("{} NOT IN ({})", field, ph)
    } else {
        format!("{} IN ({})", field, ph)
    }
}

// ============================================================================
// SET / INCREMENT / ORDER BY / AGGREGATE / RETURNING
// ============================================================================

fn build_set_clause<'a>(args: &'a [HirExpr], state: &mut SqlState<'a>) -> String {
    let Some(obj) = args.first() else {
        return String::new();
    };
    match &obj.kind {
        HirExprKind::Struct { fields, .. } => fields
            .iter()
            .map(|(name, val)| {
                let ph = state.push_param(val);
                format!("{} = {}", name, ph)
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    }
}

fn build_increment_clause<'a>(
    args: &'a [HirExpr],
    state: &mut SqlState<'a>,
    op: &str,
) -> Option<String> {
    // increment(field, amount) or increment({field: amount})
    if args.len() >= 2 {
        let field = extract_string_lit(&args[0])?;
        let ph = state.push_param(&args[1]);
        Some(format!("{} = {} {} {}", field, field, op, ph))
    } else if let Some(obj) = args.first() {
        if let HirExprKind::Struct { fields, .. } = &obj.kind {
            let parts: Vec<String> = fields
                .iter()
                .map(|(name, val)| {
                    let ph = state.push_param(val);
                    format!("{} = {} {} {}", name, name, op, ph)
                })
                .collect();
            Some(parts.join(", "))
        } else {
            None
        }
    } else {
        None
    }
}

fn build_order_by(args: &[HirExpr]) -> String {
    let Some(obj) = args.first() else {
        return String::new();
    };
    match &obj.kind {
        HirExprKind::Struct { fields, .. } => fields
            .iter()
            .map(|(col, dir_expr)| {
                let dir = match &dir_expr.kind {
                    HirExprKind::Local { name } if name == ffi_names::QB_SORT_DESC => "DESC",
                    _ => "ASC",
                };
                format!("{} {}", col, dir)
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    }
}

fn build_aggregate_cols(args: &[HirExpr]) -> String {
    let Some(obj) = args.first() else {
        return "*".to_string();
    };
    match &obj.kind {
        HirExprKind::Struct { fields, .. } => fields
            .iter()
            .map(|(alias, fn_expr)| {
                let fn_str = match &fn_expr.kind {
                    HirExprKind::Call {
                        func,
                        args: fn_args,
                    } => {
                        if let HirExprKind::Local { name: fn_name } = &func.kind {
                            let col = fn_args
                                .first()
                                .and_then(|a| extract_string_lit(a))
                                .unwrap_or("*");
                            format!("{}({}) AS {}", fn_name.to_uppercase(), col, alias)
                        } else {
                            alias.clone()
                        }
                    }
                    _ => alias.clone(),
                };
                fn_str
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => "*".to_string(),
    }
}

fn build_returning_clause(chain: &QueryChain<'_>) -> String {
    if let Some(ret_args) = chain.clauses.iter().find_map(|c| {
        if let ChainClause::Returning(a) = c {
            Some(*a)
        } else {
            None
        }
    }) {
        // returning(["id", "name"]) or returning("*")
        if let Some(first) = ret_args.first() {
            if let HirExprKind::Array(items) = &first.kind {
                let cols = extract_string_list(items);
                return format!("RETURNING {}", cols.join(", "));
            }
            if let Some(col) = extract_string_lit(first) {
                return format!("RETURNING {}", col);
            }
        }
    }
    "RETURNING *".to_string()
}

// ============================================================================
// Helpers
// ============================================================================

/// Resolve the SQL table name for a struct.
/// Uses `@table("name")` override if present, otherwise `lowercase(name) + "s"`.
fn resolve_table_name(model_name: &str, builder: &MirBuilder) -> String {
    if let Some(meta) = builder.struct_metas.get(model_name) {
        if let Some(table) = &meta.table_name {
            return table.clone();
        }
    }
    // Default: lowercase struct name + "s" (simple pluralization).
    format!("{}s", model_name.to_lowercase())
}

/// Extract string literal value from a HirExpr.
fn extract_string_lit(expr: &HirExpr) -> Option<&str> {
    if let HirExprKind::Const(ConstValue::Str(s)) = &expr.kind {
        Some(s.as_str())
    } else {
        None
    }
}

/// Extract a list of string literals from HIR exprs (e.g., select(["col1", "col2"])).
fn extract_string_list(args: &[HirExpr]) -> Vec<&str> {
    // Handle both: select("col1", "col2") and select(["col1", "col2"])
    if args.len() == 1 {
        if let HirExprKind::Array(items) = &args[0].kind {
            return items.iter().filter_map(extract_string_lit).collect();
        }
    }
    args.iter().filter_map(extract_string_lit).collect()
}

/// Extract field names from string literal expressions.
fn extract_field_names(args: &[HirExpr]) -> Vec<&str> {
    extract_string_list(args)
}

// ============================================================================
// MIR Emission
// ============================================================================

/// Build a MirOperand by evaluating a HIR expression.
/// This is a thin wrapper around `build_expr` to keep the call site clean.
fn build_expr_from_hir(builder: &mut MirBuilder, expr: &HirExpr, _span: MirSpan) -> MirOperand {
    super::expr::build_expr(builder, expr)
}

/// Build the params JSON string operand.
///
/// For all-literal params: serializes to a compile-time JSON array string.
/// For runtime params: emits ArrayCreate + JSON.stringify MethodCall.
fn build_params_operand<'a>(
    builder: &mut MirBuilder,
    param_exprs: &[&'a HirExpr],
    span: MirSpan,
) -> MirOperand {
    if param_exprs.is_empty() {
        // No params — pass empty JSON array string.
        return MirOperand::Const(MirConst::Str("[]".to_string()));
    }

    // Try to build compile-time JSON string if all params are literals.
    if let Some(json) = try_build_literal_params_json(param_exprs) {
        return MirOperand::Const(MirConst::Str(json));
    }

    // Runtime path: build array + JSON.stringify.
    // 1. Evaluate each param expression.
    let param_ops: Vec<MirOperand> = param_exprs
        .iter()
        .map(|e| super::expr::build_expr(builder, e))
        .collect();

    // 2. Emit ArrayCreate.
    let arr_dest = builder.new_temp();
    builder.emit(
        MirInstrKind::ArrayCreate {
            dest: arr_dest,
            elements: param_ops,
            elem_type: builtin::ANY,
        },
        span,
    );

    // 3. Emit JSON.stringify(array) → string.
    let json_dest = builder.new_temp();
    builder.set_temp_type(json_dest, builtin::STR);
    builder.emit(
        MirInstrKind::MethodCall {
            dest: Some(json_dest),
            receiver: MirOperand::Global(sym("JSON")),
            receiver_type: builtin::ANY,
            method: sym("stringify"),
            args: vec![MirOperand::Temp(arr_dest)],
            arg_types: vec![builtin::ANY],
            return_type: Some(builtin::STR),
        },
        span,
    );

    MirOperand::Temp(json_dest)
}

/// Try to serialize all params as a compile-time JSON array string.
/// Returns `None` if any param is a runtime value.
fn try_build_literal_params_json(param_exprs: &[&HirExpr]) -> Option<String> {
    let mut json = String::from("[");
    for (i, expr) in param_exprs.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        let value_json = literal_expr_to_json(expr)?;
        json.push_str(&value_json);
    }
    json.push(']');
    Some(json)
}

/// Convert a literal HIR expression to JSON text.
/// Supports constants and nested array literals.
fn literal_expr_to_json(expr: &HirExpr) -> Option<String> {
    match &expr.kind {
        HirExprKind::Const(cv) => match cv {
            ConstValue::Str(s) => {
                let mut out = String::from("\"");
                for ch in s.chars() {
                    match ch {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\r' => out.push_str("\\r"),
                        '\t' => out.push_str("\\t"),
                        c => out.push(c),
                    }
                }
                out.push('"');
                Some(out)
            }
            ConstValue::Int(n) => Some(n.to_string()),
            ConstValue::Float(f) => Some(format!("{}", f)),
            ConstValue::Bool(b) => Some(if *b { "true" } else { "false" }.to_string()),
            ConstValue::Nil => Some("null".to_string()),
        },
        HirExprKind::Array(items) => {
            let mut out = String::from("[");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let item_json = literal_expr_to_json(item)?;
                out.push_str(&item_json);
            }
            out.push(']');
            Some(out)
        }
        _ => None,
    }
}
