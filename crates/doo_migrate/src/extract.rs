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
pub fn extract_schema(path: &Path) -> Result<DatabaseSchema, String> {
    let input_path = resolve_input(path)?;
    let source = fs::read_to_string(&input_path)
        .map_err(|e| format!("Failed to read {}: {}", input_path.display(), e))?;

    let project_root = input_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Parse
    let mut parser = Parser::new(&source, 0);
    let program = parser
        .parse_program()
        .map_err(|e| format!("Parse error: {:?}", e))?;

    // Resolve imports
    let mut program = program;
    let mut loader = doo_driver_loader::ModuleLoader::new();
    let import_resolution =
        doo_driver_loader::resolve_imports(&program, &mut loader, &project_root)?;
    doo_analysis::loader::merge_imports(&mut program, import_resolution);

    // AST transforms
    doo_analysis::transform::transform_route_groups(&mut program);
    doo_analysis::transform::transform_inline_closures(&mut program);

    // Lower to HIR
    let mut type_registry = TypeRegistry::new();
    let mut lowerer = Lower::new();
    let hir = lowerer.lower_program_typed(&program, &mut type_registry);

    // Walk HIR and extract schema
    extract_from_hir(&hir, &type_registry)
}

/// Walk HIR items and extract table/enum definitions.
fn extract_from_hir(
    hir: &doo_hir::HirProgram,
    type_registry: &TypeRegistry,
) -> Result<DatabaseSchema, String> {
    let mut schema = DatabaseSchema::default();
    let mut enum_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut referenced_enums: HashSet<String> = HashSet::new();

    // First pass: collect all enums
    for item in &hir.items {
        if let HirItem::Enum(e) = item {
            let variants: Vec<String> = e.variants.iter().map(|v| v.name.clone()).collect();
            enum_map.insert(e.name.clone(), variants);
        }
    }

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
            let table_name = custom_table_name
                .unwrap_or_else(|| format!("{}s", s.name.to_lowercase()));

            let mut columns = Vec::new();
            let mut primary_key_cols = Vec::new();
            let mut unique_constraints = Vec::new();
            let mut foreign_keys = Vec::new();
            let mut indexes = Vec::new();

            for field in &s.fields {
                let col_name = to_snake_case(&field.name);
                let is_primary = field.decorators.iter().any(|d| d.name == "primary");
                let is_auto = field.decorators.iter().any(|d| {
                    d.name == "auto" || d.name == "autoIncrement"
                });
                let is_unique = field.decorators.iter().any(|d| d.name == "unique");
                let is_index = field.decorators.iter().any(|d| d.name == "index");
                let is_hashed = field.decorators.iter().any(|d| d.name == "hash");
                let is_optional = field.is_optional
                    || field.decorators.iter().any(|d| d.name == "optional");

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
                            let ref_table = format!("{}s", ref_struct.to_lowercase());
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
        return if is_auto { SqlType::Serial } else { SqlType::Integer };
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
                resolve_sql_type(Some(*inner), type_registry, enum_map, referenced_enums, is_auto)
            }
            _ => SqlType::Text,
        }
    } else {
        SqlType::Text
    }
}

/// Resolve input path to a main.doo file.
///
/// TODO: This duplicates `doo_driver::compile::resolve_input_path()` which has
/// BFS scan, multi-project detection, and DOO_ENTRY support. Cannot reuse due
/// to circular dependency (doo_driver depends on doo_migrate).
/// Consider extracting to `doo_core` or a shared crate.
fn resolve_input(path: &Path) -> Result<PathBuf, String> {
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
    // Try path/main.doo, then path/src/main.doo
    let main = path.join("main.doo");
    if main.exists() {
        return Ok(main);
    }
    let src_main = path.join("src").join("main.doo");
    if src_main.exists() {
        return Ok(src_main);
    }
    // Try to find any .doo file
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("doo") {
                return Ok(p);
            }
        }
    }
    Err(format!(
        "No .doo files found in {}",
        path.display()
    ))
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

        Ok(ImportResolution {
            items,
            errors,
        })
    }
}
