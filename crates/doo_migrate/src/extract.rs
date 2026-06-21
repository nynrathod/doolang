//! Schema Extractor — .doo source → DatabaseSchema
//!
//! Parses all .doo files in a project, lowers to HIR, and walks the HIR items
//! to extract `@table` structs, implicit tables (from app.auth/app.crud),
//! and referenced enums into a `DatabaseSchema`.
//!
//! Uses the exact same compiler frontend as `doo build` — zero duplication.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::{env, fs};

use doo_core::string::to_snake_case;
use doo_core::types::TypeRegistry;
use doo_frontend::Parser;
use doo_hir::{ConstValue, HirExpr, HirExprKind, HirItem, HirStmtKind, Lower};

use crate::schema::*;

// ============================================================================
// Implicit Table Constants — Single Source of Truth
// ============================================================================
// These are the ONLY place where Server method names and derived table names
// are defined for migration discovery. The FFI runtime uses matching names
// (doo_http_auth, doo_http_crud) but the migration system discovers tables
// by scanning HIR method calls, not by calling the FFI.

/// Server method name for auth endpoint registration.
const SERVER_METHOD_AUTH: &str = "auth";
/// Server method name for CRUD endpoint registration.
const SERVER_METHOD_CRUD: &str = "crud";
/// Default table name for auth-backed structs.
/// Matches the FFI convention: app.auth() always creates a "users" table.
const DEFAULT_AUTH_TABLE: &str = "users";

/// Extract a DatabaseSchema from the project.
///
/// Always uses entry-point resolution: starts from the specified file or
/// main.doo and follows the full import chain. Never scans unrelated .doo
/// files in the directory — the import graph is the single source of truth.
pub fn extract_schema(path: &Path) -> Result<DatabaseSchema, String> {
    let project_root = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    };

    // Single strategy: entry-point resolution via import chain.
    // main.doo (or specified file) is the truth — only its imports matter.
    try_extract_from_entry(path, &project_root)
}

/// Strategy 1: Resolve a proper entry point (main.doo or src/main.doo),
/// parse it, resolve its full import chain, lower to HIR, extract schema.
fn try_extract_from_entry(path: &Path, project_root: &Path) -> Result<DatabaseSchema, String> {
    let entry = resolve_entry_point(path)?;
    let source = fs::read_to_string(&entry)
        .map_err(|e| format!("Failed to read {}: {}", entry.display(), e))?;

    // Parse
    let mut parser = Parser::new(&source, 0);
    let mut program = parser
        .parse_program()
        .map_err(|e| format!("Parse error: {:?}", e))?;

    // Resolve imports
    let mut loader = doo_driver_loader::ModuleLoader::new();
    let import_resolution =
        doo_driver_loader::resolve_imports(&program, &mut loader, project_root)?;
    doo_analysis::loader::merge_imports(&mut program, import_resolution);

    // AST transforms
    doo_analysis::transform::transform_route_groups(&mut program);
    doo_analysis::transform::transform_inline_closures(&mut program);

    // Lower to HIR
    let mut type_registry = TypeRegistry::new();
    let mut lowerer = Lower::new();
    let hir = lowerer.lower_program_typed(&program, &mut type_registry);

    extract_from_hir(&hir, &type_registry)
}

/// Strategy 2: Recursively discover ALL .doo files, parse each one,
/// No hardcoded filenames beyond the standard project convention (main.doo).
fn resolve_entry_point(path: &Path) -> Result<PathBuf, String> {
    // Check DOO_ENTRY override
    if let Ok(entry) = std::env::var(doo_core::constants::env_vars::DOO_ENTRY) {
        let entry_path = PathBuf::from(&entry);
        if entry_path.exists() {
            return Ok(entry_path);
        }
    }

    if path.is_file() {
        return Ok(path.to_path_buf());
    }

    // Try main.doo, then src/main.doo (standard project conventions)
    let main = path.join("main.doo");
    if main.exists() {
        return Ok(main);
    }
    let src_main = path.join("src").join("main.doo");
    if src_main.exists() {
        return Ok(src_main);
    }

    Err(format!(
        "No main.doo or src/main.doo found in {}",
        path.display()
    ))
}
/// Discover tables that exist implicitly through app.auth() and app.crud() calls.
///
/// These structs do NOT have a @table decorator, but the Server creates tables
/// for them at runtime. The migration system must know about them to generate DDL.
///
/// Returns a map of struct_name → table_name for implicit tables.
fn discover_implicit_tables(
    hir: &doo_hir::HirProgram,
    all_structs: &HashMap<String, (&doo_hir::HirStruct, bool)>,
) -> HashMap<String, String> {
    let mut implicit: HashMap<String, String> = HashMap::new();

    for item in &hir.items {
        if let HirItem::Function(f) = item {
            for stmt in &f.body {
                collect_method_calls(stmt, all_structs, &mut implicit);
            }
        }
    }

    implicit
}

/// Recursively walk HIR statements/expressions looking for app.auth() and app.crud()
/// method calls. When found, extract the struct name arg and derive the table name.
fn collect_method_calls(
    stmt: &doo_hir::HirStmt,
    all_structs: &HashMap<String, (&doo_hir::HirStruct, bool)>,
    implicit: &mut HashMap<String, String>,
) {
    match &stmt.kind {
        HirStmtKind::Expr(expr) | HirStmtKind::Let { value: expr, .. } => {
            walk_expr_for_implicit_tables(expr, all_structs, implicit);
        }
        HirStmtKind::Assign { target, value } => {
            walk_expr_for_implicit_tables(target, all_structs, implicit);
            walk_expr_for_implicit_tables(value, all_structs, implicit);
        }
        HirStmtKind::Return(exprs) => {
            for e in exprs {
                walk_expr_for_implicit_tables(e, all_structs, implicit);
            }
        }
        HirStmtKind::If {
            condition,
            then_block,
            else_block,
        } => {
            walk_expr_for_implicit_tables(condition, all_structs, implicit);
            for s in then_block {
                collect_method_calls(s, all_structs, implicit);
            }
            if let Some(block) = else_block {
                for s in block {
                    collect_method_calls(s, all_structs, implicit);
                }
            }
        }
        HirStmtKind::While {
            condition, body, ..
        } => {
            walk_expr_for_implicit_tables(condition, all_structs, implicit);
            for s in body {
                collect_method_calls(s, all_structs, implicit);
            }
        }
        HirStmtKind::ManualErrorExtract { expr, .. } => {
            walk_expr_for_implicit_tables(expr, all_structs, implicit);
        }
        _ => {}
    }
}

/// Recursively walk a HIR expression tree looking for MethodCall patterns
/// that match app.auth() or app.crud(). When found, extract the struct name
/// argument and register the implicit table.
fn walk_expr_for_implicit_tables(
    expr: &HirExpr,
    all_structs: &HashMap<String, (&doo_hir::HirStruct, bool)>,
    implicit: &mut HashMap<String, String>,
) {
    match &expr.kind {
        HirExprKind::MethodCall {
            receiver: _,
            method,
            args,
        } => {
            // Check for app.auth() or app.crud()
            if method == SERVER_METHOD_AUTH {
                // app.auth(signupPath, loginPath, StructName, db)
                // Find the struct name argument — it's a Global or Local referencing a known struct
                if let Some(struct_name) = find_struct_arg_in_method_args(args, all_structs) {
                    // Auth always uses "users" as the table name (matches FFI convention)
                    implicit
                        .entry(struct_name)
                        .or_insert_with(|| DEFAULT_AUTH_TABLE.to_string());
                }
            } else if method == SERVER_METHOD_CRUD {
                // app.crud(basePath, StructName, db)
                // Find the struct name argument and derive table name from the path
                if let Some(struct_name) = find_struct_arg_in_method_args(args, all_structs) {
                    let table_name = extract_crud_table_name(args);
                    implicit.entry(struct_name).or_insert_with(|| table_name);
                }
            }

            // Recurse into nested expressions
            for arg in args {
                walk_expr_for_implicit_tables(arg, all_structs, implicit);
            }
        }
        HirExprKind::Call { func, args } => {
            walk_expr_for_implicit_tables(func, all_structs, implicit);
            for arg in args {
                walk_expr_for_implicit_tables(arg, all_structs, implicit);
            }
        }
        HirExprKind::Field { object, .. } => {
            walk_expr_for_implicit_tables(object, all_structs, implicit);
        }
        HirExprKind::Index { object, index } => {
            walk_expr_for_implicit_tables(object, all_structs, implicit);
            walk_expr_for_implicit_tables(index, all_structs, implicit);
        }
        HirExprKind::BinOp { lhs, rhs, .. } => {
            walk_expr_for_implicit_tables(lhs, all_structs, implicit);
            walk_expr_for_implicit_tables(rhs, all_structs, implicit);
        }
        HirExprKind::UnaryOp { operand, .. } => {
            walk_expr_for_implicit_tables(operand, all_structs, implicit);
        }
        HirExprKind::Struct { fields, .. } => {
            for (_, field_expr) in fields {
                walk_expr_for_implicit_tables(field_expr, all_structs, implicit);
            }
        }
        HirExprKind::Block { stmts, expr } => {
            for s in stmts {
                collect_method_calls(s, all_structs, implicit);
            }
            if let Some(e) = expr {
                walk_expr_for_implicit_tables(e, all_structs, implicit);
            }
        }
        HirExprKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
            walk_expr_for_implicit_tables(condition, all_structs, implicit);
            walk_expr_for_implicit_tables(then_expr, all_structs, implicit);
            if let Some(e) = else_expr {
                walk_expr_for_implicit_tables(e, all_structs, implicit);
            }
        }
        HirExprKind::Match { values, arms } => {
            for v in values {
                walk_expr_for_implicit_tables(v, all_structs, implicit);
            }
            for arm in arms {
                walk_expr_for_implicit_tables(&arm.body, all_structs, implicit);
                if let Some(guard) = &arm.guard {
                    walk_expr_for_implicit_tables(guard, all_structs, implicit);
                }
            }
        }
        HirExprKind::Array(elems) | HirExprKind::Tuple(elems) => {
            for e in elems {
                walk_expr_for_implicit_tables(e, all_structs, implicit);
            }
        }
        HirExprKind::Map(entries) => {
            for (k, v) in entries {
                walk_expr_for_implicit_tables(k, all_structs, implicit);
                walk_expr_for_implicit_tables(v, all_structs, implicit);
            }
        }
        HirExprKind::Ok(inner) | HirExprKind::Err(inner) | HirExprKind::Try(inner) => {
            walk_expr_for_implicit_tables(inner, all_structs, implicit);
        }
        HirExprKind::UnwrapOrPanic {
            expr: inner,
            message,
        } => {
            walk_expr_for_implicit_tables(inner, all_structs, implicit);
            walk_expr_for_implicit_tables(message, all_structs, implicit);
        }
        HirExprKind::Range { start, end, .. } => {
            walk_expr_for_implicit_tables(start, all_structs, implicit);
            walk_expr_for_implicit_tables(end, all_structs, implicit);
        }
        // Route block — recurse into each route expression
        HirExprKind::RouteBlock { routes } => {
            for r in routes {
                walk_expr_for_implicit_tables(r, all_structs, implicit);
            }
        }
        // Ownership annotations — unwrap and recurse
        HirExprKind::Move(inner) | HirExprKind::Clone(inner) | HirExprKind::Await(inner) => {
            walk_expr_for_implicit_tables(inner, all_structs, implicit);
        }
        HirExprKind::Borrow { expr: inner, .. } => {
            walk_expr_for_implicit_tables(inner, all_structs, implicit);
        }
        // Closure — recurse into body
        HirExprKind::Closure { body, .. } => {
            walk_expr_for_implicit_tables(body, all_structs, implicit);
        }
        // Cast — recurse into value (to_type is a TypeId, not an expression)
        HirExprKind::Cast { value, .. } => {
            walk_expr_for_implicit_tables(value, all_structs, implicit);
        }
        // Concurrency — recurse into body
        HirExprKind::Spawn { body } => {
            walk_expr_for_implicit_tables(body, all_structs, implicit);
        }
        // Scope block — recurse into statements
        HirExprKind::ScopeBlock { stmts } => {
            for s in stmts {
                collect_method_calls(s, all_structs, implicit);
            }
        }
        // Leaf nodes — no recursion needed
        HirExprKind::Const(_)
        | HirExprKind::Local { .. }
        | HirExprKind::Global { .. }
        | HirExprKind::EnumVariant { .. }
        | HirExprKind::Spread(_) => {}
    }
}

/// Find a struct name argument in a method call's argument list.
/// Returns the struct name if any argument is a Global/Local referencing a known struct.
fn find_struct_arg_in_method_args(
    args: &[HirExpr],
    all_structs: &HashMap<String, (&doo_hir::HirStruct, bool)>,
) -> Option<String> {
    for arg in args {
        match &arg.kind {
            HirExprKind::Global { name } | HirExprKind::Local { name } => {
                if all_structs.contains_key(name.as_str()) {
                    return Some(name.clone());
                }
            }
            HirExprKind::Const(ConstValue::Str(name)) => {
                if all_structs.contains_key(name.as_str()) {
                    return Some(name.clone());
                }
            }
            _ => {}
        }
    }
    None
}

/// Extract the table name from a CRUD path argument.
/// app.crud("/products", ...) → "products"
/// app.crud("/api/v1/todos", ...) → "todos"
fn extract_crud_table_name(args: &[HirExpr]) -> String {
    // The first argument of app.crud() is the base path
    if let Some(first_arg) = args.first() {
        if let HirExprKind::Const(ConstValue::Str(path)) = &first_arg.kind {
            // Extract the last segment of the path
            let segments: Vec<&str> = path
                .trim_start_matches('/')
                .split('/')
                .filter(|s| {
                    // Skip common API prefixes and numeric IDs
                    !s.is_empty()
                        && !s.eq_ignore_ascii_case("api")
                        && !s.eq_ignore_ascii_case("v1")
                        && !s.eq_ignore_ascii_case("v2")
                        && s.parse::<i64>().is_err()
                })
                .collect();
            if let Some(last) = segments.last() {
                return last.to_string();
            }
        }
    }
    // Fallback: use the path as-is (minus leading slash)
    if let Some(first_arg) = args.first() {
        if let HirExprKind::Const(ConstValue::Str(path)) = &first_arg.kind {
            return path.trim_start_matches('/').to_string();
        }
    }
    "unknown".to_string()
}

/// Build a TableDef from a HIR struct (shared by @table and implicit table paths).
fn build_table_def_from_struct(
    s: &doo_hir::HirStruct,
    table_name: &str,
    type_registry: &TypeRegistry,
    all_structs: &HashMap<String, (&doo_hir::HirStruct, bool)>,
    table_struct_names: &HashSet<String>,
    enum_map: &HashMap<String, Vec<String>>,
    referenced_enums: &mut HashSet<String>,
) -> TableDef {
    let auto_timestamp = s.decorators.iter().any(|d| d.name == "autoTimestamp");

    let mut columns = Vec::new();
    let mut primary_key_cols = Vec::new();
    let mut unique_constraints = Vec::new();
    let mut foreign_keys = Vec::new();
    let mut indexes = Vec::new();
    let mut table_struct_refs: Vec<String> = Vec::new();
    let mut transitive_enum_refs: Vec<String> = Vec::new();

    for field in &s.fields {
        let col_name = to_snake_case(&field.name);
        let is_primary = field.decorators.iter().any(|d| d.name == "primary");
        let is_auto = field
            .decorators
            .iter()
            .any(|d| d.name == "auto" || d.name == "autoIncrement");
        let is_unique = field.decorators.iter().any(|d| d.name == "unique");
        let is_index = field.decorators.iter().any(|d| d.name == "index");
        let is_hashed = field.decorators.iter().any(|d| d.name == "hash");
        let is_optional = field.is_optional || field.decorators.iter().any(|d| d.name == "optional")
                // @internal fields are never in request body, so INSERTs omit them → must be nullable
                || field.decorators.iter().any(|d| d.name == "internal");

        let mut field_transitive_enums: Vec<String> = Vec::new();
        check_non_table_struct_field(
            field.type_id,
            type_registry,
            all_structs,
            table_struct_names,
            enum_map,
            referenced_enums,
            &mut table_struct_refs,
            &mut field_transitive_enums,
        );
        for enum_name in &field_transitive_enums {
            if !transitive_enum_refs.contains(enum_name) {
                transitive_enum_refs.push(enum_name.clone());
            }
        }

        let sql_type = resolve_sql_type(
            field.type_id,
            type_registry,
            enum_map,
            referenced_enums,
            is_auto,
        );

        let default = field.decorators.iter().find_map(|d| {
            if d.name == "default" {
                d.args.first().and_then(|a| match &a.kind {
                    HirExprKind::Const(ConstValue::Int(n)) => Some(DefaultValue::Integer(*n)),
                    HirExprKind::Const(ConstValue::Float(f)) => Some(DefaultValue::Float(*f)),
                    HirExprKind::Const(ConstValue::Bool(b)) => Some(DefaultValue::Boolean(*b)),
                    HirExprKind::Const(ConstValue::Str(s)) => Some(DefaultValue::String(s.clone())),
                    _ => None,
                })
            } else {
                None
            }
        });

        if is_primary {
            primary_key_cols.push(col_name.clone());
        }

        if is_unique {
            unique_constraints.push(UniqueConstraintDef {
                name: format!("{}_{}_key", table_name, col_name),
                columns: vec![col_name.clone()],
            });
        }

        if is_index {
            indexes.push(IndexDef {
                name: format!("idx_{}_{}", table_name, col_name),
                columns: vec![col_name.clone()],
                unique: false,
            });
        }

        for dec in &field.decorators {
            if dec.name == "foreign" {
                if let Some(arg) = dec.args.first() {
                    let ref_struct = match &arg.kind {
                        HirExprKind::Const(ConstValue::Str(s)) => s.clone(),
                        HirExprKind::Local { name } => name.clone(),
                        HirExprKind::Global { name } => name.clone(),
                        _ => continue,
                    };
                    let ref_table = resolve_table_name(&ref_struct, all_structs);
                    foreign_keys.push(ForeignKeyDef {
                        name: format!("{}_{}_fkey", table_name, col_name),
                        columns: vec![col_name.clone()],
                        ref_table,
                        ref_columns: vec!["id".to_string()],
                        on_delete: ForeignKeyAction::Cascade,
                        on_update: ForeignKeyAction::NoAction,
                    });
                }
            }
        }

        columns.push(ColumnDef {
            name: col_name,
            field_name: field.name.clone(),
            sql_type,
            nullable: is_optional,
            default,
            is_auto,
            is_primary,
            is_unique,
            is_index,
            is_hashed,
        });
    }

    // Add autoTimestamp columns
    if auto_timestamp {
        let has_created_at = columns.iter().any(|c| c.name == "created_at");
        let has_updated_at = columns.iter().any(|c| c.name == "updated_at");

        if !has_created_at {
            columns.push(ColumnDef {
                name: "created_at".to_string(),
                field_name: "createdAt".to_string(),
                sql_type: SqlType::TimestampTz,
                nullable: false,
                default: Some(DefaultValue::Expression("NOW()".to_string())),
                is_auto: false,
                is_primary: false,
                is_unique: false,
                is_index: false,
                is_hashed: false,
            });
        }
        if !has_updated_at {
            columns.push(ColumnDef {
                name: "updated_at".to_string(),
                field_name: "updatedAt".to_string(),
                sql_type: SqlType::TimestampTz,
                nullable: false,
                default: Some(DefaultValue::Expression("NOW()".to_string())),
                is_auto: false,
                is_primary: false,
                is_unique: false,
                is_index: false,
                is_hashed: false,
            });
        }
    }

    let primary_key = if !primary_key_cols.is_empty() {
        Some(PrimaryKeyDef {
            name: format!("{}_pkey", table_name),
            columns: primary_key_cols,
        })
    } else {
        None
    };

    table_struct_refs.sort();
    table_struct_refs.dedup();
    transitive_enum_refs.sort();
    transitive_enum_refs.dedup();

    TableDef {
        name: table_name.to_string(),
        struct_name: s.name.clone(),
        columns,
        primary_key,
        unique_constraints,
        check_constraints: Vec::new(),
        foreign_keys,
        indexes,
        auto_timestamp,
        struct_refs: table_struct_refs,
        transitive_enum_refs,
    }
}

fn extract_from_hir(
    hir: &doo_hir::HirProgram,
    type_registry: &TypeRegistry,
) -> Result<DatabaseSchema, String> {
    let mut schema = DatabaseSchema::default();
    let mut enum_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut referenced_enums: HashSet<String> = HashSet::new();

    // First pass: collect all enums AND all structs (both @table and non-table)
    // all_structs: struct_name -> (HirStruct ref, is_table)
    let mut all_structs: HashMap<String, (&doo_hir::HirStruct, bool)> = HashMap::new();

    for item in &hir.items {
        match item {
            HirItem::Enum(e) => {
                let variants: Vec<String> = e.variants.iter().map(|v| v.name.clone()).collect();
                enum_map.insert(e.name.clone(), variants);
            }
            HirItem::Struct(s) => {
                let has_table = s.decorators.iter().any(|d| d.name == "table");
                all_structs.insert(s.name.clone(), (s, has_table));
            }
            _ => {}
        }
    }

    // Discover implicit tables from app.auth() and app.crud() calls.
    // These are structs without @table that still create database tables at runtime.
    let implicit_tables = discover_implicit_tables(hir, &all_structs);

    // Pre-compute which struct names are tables (either @table or implicit).
    // This is the SINGLE SOURCE OF TRUTH for "is this struct a database table?"
    let mut table_struct_names: HashSet<String> = all_structs
        .iter()
        .filter(|(_, (_, is_table))| *is_table)
        .map(|(name, _)| name.clone())
        .collect();
    // Add implicit table structs
    for struct_name in implicit_tables.keys() {
        table_struct_names.insert(struct_name.clone());
    }

    // Second pass: extract @table structs
    for item in &hir.items {
        if let HirItem::Struct(s) = item {
            let has_table = s.decorators.iter().any(|d| d.name == "table");
            if !has_table {
                continue;
            }

            // Extract custom table name from @table("name")
            let custom_table_name = s.decorators.iter().find_map(|d| {
                if d.name == "table" {
                    d.args.first().and_then(|a| {
                        if let HirExprKind::Const(ConstValue::Str(t)) = &a.kind {
                            Some(t.clone())
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            });

            let table_name =
                custom_table_name.unwrap_or_else(|| format!("{}s", s.name.to_lowercase()));

            let table_def = build_table_def_from_struct(
                s,
                &table_name,
                type_registry,
                &all_structs,
                &table_struct_names,
                &enum_map,
                &mut referenced_enums,
            );
            schema.tables.push(table_def);
        }
    }

    // Third pass: process implicit table structs (app.auth/app.crud)
    for (struct_name, table_name) in &implicit_tables {
        // Skip if this struct was already processed as a @table struct
        if schema.tables.iter().any(|t| t.struct_name == *struct_name) {
            continue;
        }

        if let Some((hir_struct, _)) = all_structs.get(struct_name) {
            let table_def = build_table_def_from_struct(
                hir_struct,
                table_name,
                type_registry,
                &all_structs,
                &table_struct_names,
                &enum_map,
                &mut referenced_enums,
            );
            schema.tables.push(table_def);
        }
    }

    // Create enum types ONLY for enums that are referenced by table columns
    // (or transitively through non-table struct fields used by table columns).
    // Enums defined in .doo source but not used in any table column do NOT
    // need PostgreSQL enum types — they're just in-code types with no DB presence.
    for (enum_name, variants) in &enum_map {
        if referenced_enums.contains(enum_name) {
            schema.enums.push(EnumTypeDef {
                name: to_snake_case(enum_name),
                enum_name: enum_name.clone(),
                variants: variants.clone(),
            });
        }
    }

    Ok(schema)
}

/// Resolve Doo TypeId to PostgreSQL SqlType.
fn resolve_sql_type(
    type_id: Option<doo_core::types::TypeId>,
    type_registry: &TypeRegistry,
    enum_map: &HashMap<String, Vec<String>>,
    referenced_enums: &mut HashSet<String>,
    is_auto: bool,
) -> SqlType {
    let Some(tid) = type_id else {
        return SqlType::Text; // Unknown type defaults to TEXT
    };

    use doo_core::types::{builtin, TypeKind};

    if tid == builtin::INT {
        return if is_auto {
            SqlType::Serial
        } else {
            SqlType::Integer
        };
    }
    if tid == builtin::FLOAT {
        return SqlType::DoublePrecision;
    }
    if tid == builtin::BOOL {
        return SqlType::Boolean;
    }
    if tid == builtin::STR {
        return SqlType::Text;
    }

    // Check type registry for struct/enum types
    if let Some(type_info) = type_registry.get(tid) {
        match &type_info.kind {
            TypeKind::Struct { name, .. } => {
                // If the struct name matches an enum, treat it as an enum type
                if enum_map.contains_key(name.as_str()) {
                    referenced_enums.insert(name.to_string());
                    return SqlType::Enum(to_snake_case(name));
                }
                SqlType::Jsonb // Nested struct → store as JSONB
            }
            TypeKind::Enum { name, .. } => {
                // Enum types — map to PostgreSQL ENUM
                if enum_map.contains_key(name.as_str()) {
                    referenced_enums.insert(name.to_string());
                    return SqlType::Enum(to_snake_case(name));
                }
                SqlType::Text
            }
            TypeKind::Array { .. } => SqlType::Jsonb,
            TypeKind::Map { .. } => SqlType::Jsonb,
            TypeKind::Optional { inner } => {
                // Recurse for optional inner type
                resolve_sql_type(
                    Some(*inner),
                    type_registry,
                    enum_map,
                    referenced_enums,
                    is_auto,
                )
            }
            _ => SqlType::Text,
        }
    } else {
        SqlType::Text
    }
}

/// Recursively resolve ALL transitive type dependencies (enums, non-table structs,
/// arrays of structs/enums, maps, optionals, tuples) from any TypeId.
///
/// This is the single source of truth for dependency discovery across the type graph.
/// Handles EVERY possible type shape that could appear in a @table struct field:
///
/// - Enum → marks as referenced enum and transitive_enum_ref
/// - Non-table Struct → records struct_ref, then recurses into its fields
/// - Array<X> → recurses into element type X
/// - Map<K,V> → recurses into key K and value V
/// - Optional<X> → recurses into inner type X
/// - Tuple<elements> → recurses into each element
/// - Result<ok, err> → recurses into both ok and err
/// - @table Struct → skipped (handled by FK dependency resolution separately)
/// - Builtins (Int, Str, Bool, Float) → skipped
///
/// `visited_structs` prevents infinite recursion on circular struct references.
fn resolve_transitive_deps(
    type_id: Option<doo_core::types::TypeId>,
    type_registry: &TypeRegistry,
    all_structs: &HashMap<String, (&doo_hir::HirStruct, bool)>,
    table_struct_names: &HashSet<String>,
    enum_map: &HashMap<String, Vec<String>>,
    referenced_enums: &mut HashSet<String>,
    struct_refs: &mut Vec<String>,
    transitive_enum_refs: &mut Vec<String>,
    visited_structs: &mut HashSet<String>,
) {
    use doo_core::types::TypeKind;

    let Some(tid) = type_id else { return };
    let Some(type_info) = type_registry.get(tid) else {
        return;
    };

    match &type_info.kind {
        // ── Enum ────────────────────────────────────────────────────────
        TypeKind::Enum { name, .. } => {
            if enum_map.contains_key(name.as_str()) {
                let sql_name = to_snake_case(name);
                if !transitive_enum_refs.contains(&sql_name) {
                    referenced_enums.insert(name.clone());
                    transitive_enum_refs.push(sql_name);
                }
            }
        }

        // ── Struct (could be @table struct, non-table struct, or enum) ──
        // IMPORTANT: In Doo's type registry, enums may be represented as
        // TypeKind::Struct (not TypeKind::Enum). Both arms must handle enums.
        TypeKind::Struct { name, fields, .. } => {
            // Check if this is actually an enum type (represented as Struct
            // in the type registry). If so, mark it as referenced.
            if enum_map.contains_key(name.as_str()) {
                let sql_name = to_snake_case(name);
                if !transitive_enum_refs.contains(&sql_name) {
                    referenced_enums.insert(name.clone());
                    transitive_enum_refs.push(sql_name);
                }
                return;
            }
            // Skip @table structs (they generate their own migration changes)
            if table_struct_names.contains(name) {
                return;
            }
            // Prevent infinite recursion on circular struct refs
            if visited_structs.contains(name) {
                return;
            }
            visited_structs.insert(name.clone());

            // Record this non-table struct reference
            if !struct_refs.contains(name) {
                struct_refs.push(name.clone());
            }

            // Look up the HirStruct for decorator info (like @foreign)
            let hir_fields: Vec<&doo_hir::HirField> = all_structs
                .get(name)
                .map(|(s, _)| s.fields.iter().collect())
                .unwrap_or_default();

            // Build a map of field name -> HirField for decorator lookups
            let hir_field_map: HashMap<&str, &doo_hir::HirField> =
                hir_fields.iter().map(|f| (f.name.as_str(), *f)).collect();

            for (field_name, field_tid, _) in fields {
                // Check if this field has a @foreign decorator → FK dependency
                if let Some(hir_field) = hir_field_map.get(field_name.as_str()) {
                    for dec in &hir_field.decorators {
                        if dec.name == "foreign" {
                            if let Some(arg) = dec.args.first() {
                                let ref_struct = match &arg.kind {
                                    HirExprKind::Const(ConstValue::Str(s)) => s.clone(),
                                    HirExprKind::Local { name } => name.clone(),
                                    HirExprKind::Global { name } => name.clone(),
                                    _ => continue,
                                };
                                // If the referenced struct is a @table struct,
                                // record its table name as a dependency
                                if table_struct_names.contains(&ref_struct) {
                                    let ref_table = resolve_table_name(&ref_struct, all_structs);
                                    if !struct_refs.contains(&ref_table) {
                                        struct_refs.push(ref_table);
                                    }
                                }
                            }
                        }
                    }
                }

                // Recursively resolve transitive deps from this field's type
                resolve_transitive_deps(
                    Some(*field_tid),
                    type_registry,
                    all_structs,
                    table_struct_names,
                    enum_map,
                    referenced_enums,
                    struct_refs,
                    transitive_enum_refs,
                    visited_structs,
                );
            }
        }

        // ── Array<X> → check element type ───────────────────────────────
        TypeKind::Array { element } => {
            resolve_transitive_deps(
                Some(*element),
                type_registry,
                all_structs,
                table_struct_names,
                enum_map,
                referenced_enums,
                struct_refs,
                transitive_enum_refs,
                visited_structs,
            );
        }

        // ── Map<K,V> → check both key and value types ───────────────────
        TypeKind::Map { key, value } => {
            resolve_transitive_deps(
                Some(*key),
                type_registry,
                all_structs,
                table_struct_names,
                enum_map,
                referenced_enums,
                struct_refs,
                transitive_enum_refs,
                visited_structs,
            );
            resolve_transitive_deps(
                Some(*value),
                type_registry,
                all_structs,
                table_struct_names,
                enum_map,
                referenced_enums,
                struct_refs,
                transitive_enum_refs,
                visited_structs,
            );
        }

        // ── Optional<X> → check inner type ──────────────────────────────
        TypeKind::Optional { inner } => {
            resolve_transitive_deps(
                Some(*inner),
                type_registry,
                all_structs,
                table_struct_names,
                enum_map,
                referenced_enums,
                struct_refs,
                transitive_enum_refs,
                visited_structs,
            );
        }

        // ── Result<ok, err> → check both types ──────────────────────────
        TypeKind::Result { ok, err } => {
            resolve_transitive_deps(
                Some(*ok),
                type_registry,
                all_structs,
                table_struct_names,
                enum_map,
                referenced_enums,
                struct_refs,
                transitive_enum_refs,
                visited_structs,
            );
            resolve_transitive_deps(
                Some(*err),
                type_registry,
                all_structs,
                table_struct_names,
                enum_map,
                referenced_enums,
                struct_refs,
                transitive_enum_refs,
                visited_structs,
            );
        }

        // ── Tuple<elements> → check each element type ───────────────────
        TypeKind::Tuple { elements } => {
            for elem_tid in elements {
                resolve_transitive_deps(
                    Some(*elem_tid),
                    type_registry,
                    all_structs,
                    table_struct_names,
                    enum_map,
                    referenced_enums,
                    struct_refs,
                    transitive_enum_refs,
                    visited_structs,
                );
            }
        }

        // ── Everything else (builtins, functions, interfaces, etc.) → skip ──
        _ => {}
    }
}

/// Resolve the actual table name for a struct, checking @table decorator first.
///
/// If the struct has a custom `@table("name")` decorator, returns that name.
/// Otherwise returns the default `format!("{}s", name.to_lowercase())`.
fn resolve_table_name(
    struct_name: &str,
    all_structs: &HashMap<String, (&doo_hir::HirStruct, bool)>,
) -> String {
    if let Some((hir_struct, _)) = all_structs.get(struct_name) {
        if let Some(custom_name) = hir_struct.decorators.iter().find_map(|d| {
            if d.name == "table" {
                d.args.first().and_then(|a| {
                    if let HirExprKind::Const(ConstValue::Str(s)) = &a.kind {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
            } else {
                None
            }
        }) {
            return custom_name;
        }
    }
    // Default: lowercase + "s"
    format!("{}s", struct_name.to_lowercase())
}

/// Wrapper around `resolve_transitive_deps` for a single @table struct field.
/// Creates the visited_structs set and passes all required context.
fn check_non_table_struct_field(
    type_id: Option<doo_core::types::TypeId>,
    type_registry: &TypeRegistry,
    all_structs: &HashMap<String, (&doo_hir::HirStruct, bool)>,
    table_struct_names: &HashSet<String>,
    enum_map: &HashMap<String, Vec<String>>,
    referenced_enums: &mut HashSet<String>,
    struct_refs: &mut Vec<String>,
    transitive_enum_refs: &mut Vec<String>,
) {
    let mut visited_structs: HashSet<String> = HashSet::new();
    resolve_transitive_deps(
        type_id,
        type_registry,
        all_structs,
        table_struct_names,
        enum_map,
        referenced_enums,
        struct_refs,
        transitive_enum_refs,
        &mut visited_structs,
    );
}

/// Module-local import resolution shim.
///
/// Shared types (`ImportResolution`, `merge_imports`, `resolve_module_path`)
/// come from `doo_analysis::loader` — single source of truth.
///
/// This module contains only the migration-specific `ModuleLoader` and
/// `resolve_imports` which use a simplified discovery strategy (no caching,
/// no full analysis integration) suitable for extracting `@table` structs
/// from imported modules.
mod doo_driver_loader {
    use std::collections::{HashSet, VecDeque};
    use std::path::{Path, PathBuf};
    use std::{env, fs};

    use doo_frontend::ast::{Item, Program};
    use doo_frontend::Parser;

    // Shared from doo_analysis::loader — single source of truth
    use doo_analysis::loader::{resolve_module_path, ImportResolution};

    /// Module loader — discovers and parses imported .doo files.
    pub struct ModuleLoader {
        loaded: HashSet<PathBuf>,
        sources: Vec<(usize, String, String)>,
        file_id_counter: usize,
    }

    impl ModuleLoader {
        pub fn new() -> Self {
            Self {
                loaded: HashSet::new(),
                sources: Vec::new(),
                file_id_counter: 1, // 0 = main file
            }
        }

        #[allow(dead_code)]
        pub fn imported_sources(&self) -> &[(usize, String, String)] {
            &self.sources
        }
    }

    /// Resolve all imports in the program.
    ///
    /// Simplified for migration use: silently skips unresolved imports and
    /// parse errors. Uses `doo_analysis::loader::resolve_module_path` for
    /// file discovery.
    pub fn resolve_imports(
        program: &Program,
        loader: &mut ModuleLoader,
        project_root: &Path,
    ) -> Result<ImportResolution, String> {
        let mut items = Vec::new();
        let mut errors = Vec::new();
        let mut queue: VecDeque<Vec<String>> = VecDeque::new();

        // Collect import paths
        for item in &program.items {
            if let Item::Import(import) = item {
                queue.push_back(import.path.clone());
            }
        }

        // Resolve stdlib path
        let stdlib_path = env::var(doo_core::constants::env_vars::DOO_STDLIB_PATH)
            .map(PathBuf::from)
            .ok()
            .or_else(|| {
                env::current_exe().ok().and_then(|exe| {
                    let mut p = exe.parent()?.to_path_buf();
                    loop {
                        let std_dir = p.join("std");
                        if std_dir.exists() {
                            return Some(std_dir);
                        }
                        p = p.parent()?.to_path_buf();
                    }
                })
            });

        while let Some(path) = queue.pop_front() {
            if path.is_empty() {
                continue;
            }

            let module_file = resolve_module_path(&path, project_root, stdlib_path.as_deref());
            let Some(file_path) = module_file else {
                continue; // Skip unresolved imports silently for migration
            };

            if !loader.loaded.insert(file_path.clone()) {
                continue;
            }

            let source = match fs::read_to_string(&file_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let file_id = loader.file_id_counter;
            loader.file_id_counter += 1;
            let name = file_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            loader.sources.push((file_id, name, source.clone()));

            let mut parser = Parser::new(&source, file_id as u32);
            match parser.parse_program() {
                Ok(parsed) => {
                    // Recursively queue nested imports
                    for item in &parsed.items {
                        if let Item::Import(import) = item {
                            queue.push_back(import.path.clone());
                        }
                    }
                    items.extend(parsed.items);
                }
                Err(_) => {
                    // Parse errors in imports are non-fatal for migration
                }
            }
        }

        Ok(ImportResolution { items, errors })
    }
}
