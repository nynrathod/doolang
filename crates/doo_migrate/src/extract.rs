//! Schema Extractor — .doo source → DatabaseSchema
//!
//! Parses all .doo files in a project, lowers to HIR, and walks the HIR items
//! to extract `@table` structs and referenced enums into a `DatabaseSchema`.
//!
//! Uses the exact same compiler frontend as `doo build` — zero duplication.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::{env, fs};

use doo_core::string::to_snake_case;
use doo_core::types::TypeRegistry;
use doo_frontend::Parser;
use doo_hir::{ConstValue, HirExprKind, HirItem, Lower};

use crate::schema::*;

/// Extract a DatabaseSchema from all .doo files under the given path.
///
/// Uses a two-strategy approach (like TypeORM's entity discovery):
///
/// **Strategy 1 — Entry-point resolution**: Finds `main.doo` (or `src/main.doo`)
/// and follows the full import chain via the compiler's import resolution.
/// This handles well-structured projects with proper dependency chains.
///
/// **Strategy 2 — Universal scan** (fallback): Recursively discovers ALL `.doo`
/// files in the project, parses each one, collects every `@table` struct and
/// enum from everywhere, and builds a combined schema. No hardcoded filenames.
/// Works for any project layout — temp dirs, partial projects, scattered structs.
pub fn extract_schema(path: &Path) -> Result<DatabaseSchema, String> {
    let project_root = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    };

    // Strategy 1: Try proper entry point with full import resolution
    if let Ok(schema) = try_extract_from_entry(path, &project_root) {
        if !schema.tables.is_empty() {
            return Ok(schema);
        }
    }

    // Strategy 2: Scan ALL .doo files — find @table structs everywhere
    extract_from_all_files(&project_root)
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
/// collect every @table struct and enum, build a combined schema.
/// This is the universal fallback — no hardcoded filenames, no assumptions
/// about project structure. Works for any directory with .doo files.
fn extract_from_all_files(project_root: &Path) -> Result<DatabaseSchema, String> {
    let doo_files = discover_doo_files(project_root)?;

    if doo_files.is_empty() {
        return Err(format!("No .doo files found in {}", project_root.display()));
    }

    let mut all_items: Vec<doo_frontend::ast::Item> = Vec::new();

    for (file_id, file_path) in doo_files.iter().enumerate() {
        let source = fs::read_to_string(file_path)
            .map_err(|e| format!("Failed to read {}: {}", file_path.display(), e))?;

        let mut parser = Parser::new(&source, file_id as u32);
        let program = match parser.parse_program() {
            Ok(p) => p,
            Err(_) => continue, // Skip unparseable files — not all .doo files need @table
        };

        all_items.extend(program.items);
    }

    if all_items.is_empty() {
        return Ok(DatabaseSchema::default());
    }

    // Build a combined program from ALL collected items (structs, enums, imports, etc.)
    let combined_program = doo_frontend::ast::Program::new(all_items, doo_core::Span::empty());

    // Attempt import resolution on the combined program (best-effort).
    // In the fallback path, not all imports may be resolvable — that's OK.
    // We just need the @table struct and enum definitions, which are self-contained.
    let mut program = combined_program;
    let mut loader = doo_driver_loader::ModuleLoader::new();
    if let Ok(import_resolution) =
        doo_driver_loader::resolve_imports(&program, &mut loader, project_root)
    {
        doo_analysis::loader::merge_imports(&mut program, import_resolution);
    }

    // AST transforms
    doo_analysis::transform::transform_route_groups(&mut program);
    doo_analysis::transform::transform_inline_closures(&mut program);

    // Lower to HIR
    let mut type_registry = TypeRegistry::new();
    let mut lowerer = Lower::new();
    let hir = lowerer.lower_program_typed(&program, &mut type_registry);

    extract_from_hir(&hir, &type_registry)
}

/// Recursively discover all .doo files under a directory.
/// Skips hidden dirs, target/, and node_modules/.
fn discover_doo_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    discover_recursive(dir, &mut files)
        .map_err(|e| format!("Failed to scan {}: {}", dir.display(), e))?;
    files.sort(); // Deterministic ordering
    Ok(files)
}

fn discover_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip hidden dirs, build artifacts, and dependency dirs
            if dir_name.starts_with('.') || dir_name == "target" || dir_name == "node_modules" {
                continue;
            }
            discover_recursive(&path, files)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("doo") {
            files.push(path);
        }
    }
    Ok(())
}

/// Resolve a path to a canonical entry point (.doo file or directory → main.doo).
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
fn extract_from_hir(
    hir: &doo_hir::HirProgram,
    type_registry: &TypeRegistry,
) -> Result<DatabaseSchema, String> {
    let mut schema = DatabaseSchema::default();
    let mut enum_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut referenced_enums: HashSet<String> = HashSet::new();

    // First pass: collect all enums AND all structs (both @table and non-table)
    // all_structs: struct_name -> (is_table, HirStruct ref)
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

    // Pre-compute which struct names are @table (for dependency lookups)
    let table_struct_names: HashSet<String> = all_structs
        .iter()
        .filter(|(_, (_, is_table))| *is_table)
        .map(|(name, _)| name.clone())
        .collect();

    // Second pass: extract @table structs
    for item in &hir.items {
        if let HirItem::Struct(s) = item {
            // Check if this struct has a @table decorator
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

            // Check for @autoTimestamp
            let auto_timestamp = s.decorators.iter().any(|d| d.name == "autoTimestamp");

            // Resolve table name: @table("custom") or lowercase(name) + "s"
            let table_name =
                custom_table_name.unwrap_or_else(|| format!("{}s", s.name.to_lowercase()));

            let mut columns = Vec::new();
            let mut primary_key_cols = Vec::new();
            let mut unique_constraints = Vec::new();
            let mut foreign_keys = Vec::new();
            let mut indexes = Vec::new();
            // Track non-table structs referenced by this table's fields
            let mut table_struct_refs: Vec<String> = Vec::new();
            // Track transitive enum refs found through non-table struct fields
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
                let is_optional =
                    field.is_optional || field.decorators.iter().any(|d| d.name == "optional");

                // Check if this field's type is a non-table struct reference
                // If so, we need to recursively resolve its fields' enum/FK dependencies
                let mut field_transitive_enums: Vec<String> = Vec::new();
                check_non_table_struct_field(
                    field.type_id,
                    type_registry,
                    &all_structs,
                    &table_struct_names,
                    &enum_map,
                    &mut referenced_enums,
                    &mut table_struct_refs,
                    &mut field_transitive_enums,
                );
                // Collect transitive enum refs from this field
                for enum_name in &field_transitive_enums {
                    if !transitive_enum_refs.contains(enum_name) {
                        transitive_enum_refs.push(enum_name.clone());
                    }
                }

                // Resolve SQL type from Doo type
                let sql_type = resolve_sql_type(
                    field.type_id,
                    type_registry,
                    &enum_map,
                    &mut referenced_enums,
                    is_auto,
                );

                // Extract default value from @default(value)
                let default = field.decorators.iter().find_map(|d| {
                    if d.name == "default" {
                        d.args.first().and_then(|a| match &a.kind {
                            HirExprKind::Const(ConstValue::Int(n)) => {
                                Some(DefaultValue::Integer(*n))
                            }
                            HirExprKind::Const(ConstValue::Float(f)) => {
                                Some(DefaultValue::Float(*f))
                            }
                            HirExprKind::Const(ConstValue::Bool(b)) => {
                                Some(DefaultValue::Boolean(*b))
                            }
                            HirExprKind::Const(ConstValue::Str(s)) => {
                                Some(DefaultValue::String(s.clone()))
                            }
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

                // Extract @foreign(StructName) → foreign key
                for dec in &field.decorators {
                    if dec.name == "foreign" {
                        if let Some(arg) = dec.args.first() {
                            let ref_struct = match &arg.kind {
                                HirExprKind::Const(ConstValue::Str(s)) => s.clone(),
                                HirExprKind::Local { name } => name.clone(),
                                HirExprKind::Global { name } => name.clone(),
                                _ => continue,
                            };
                            let ref_table = resolve_table_name(&ref_struct, &all_structs);
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

            // Deduplicate struct refs and transitive enum refs
            table_struct_refs.sort();
            table_struct_refs.dedup();
            transitive_enum_refs.sort();
            transitive_enum_refs.dedup();

            schema.tables.push(TableDef {
                name: table_name,
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
            });
        }
    }

    // Create enum types for referenced enums
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
            if path.len() < 2 {
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
