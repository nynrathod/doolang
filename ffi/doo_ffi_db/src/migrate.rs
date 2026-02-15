//! Database Migrations
//!
//! Auto-migration from struct definitions.
//! Generates CREATE TABLE IF NOT EXISTS + CREATE INDEX statements.

use serde::Deserialize;

/// Column definition.
#[derive(Debug, Clone, Deserialize)]
pub struct ColumnDef {
    pub name: String,
    pub sql_type: String,
    #[serde(default)]
    pub nullable: bool,
    #[serde(default)]
    pub primary_key: bool,
    #[serde(default)]
    pub auto_increment: bool,
    #[serde(default)]
    pub unique: bool,
    pub default: Option<String>,
}

/// Table schema.
#[derive(Debug, Clone, Deserialize)]
pub struct TableSchema {
    pub name: String,
    pub columns: Vec<ColumnDef>,
    pub primary_key: Option<String>,
    #[serde(default)]
    pub foreign_keys: Vec<ForeignKey>,
    #[serde(default)]
    pub indexes: Vec<Index>,
}

/// Foreign key constraint.
#[derive(Debug, Clone, Deserialize)]
pub struct ForeignKey {
    pub column: String,
    pub ref_table: String,
    pub ref_column: String,
}

/// Index definition.
#[derive(Debug, Clone, Deserialize)]
pub struct Index {
    pub name: String,
    pub columns: Vec<String>,
    #[serde(default)]
    pub unique: bool,
}

impl TableSchema {
    /// Generate CREATE TABLE SQL.
    pub fn to_create_sql(&self) -> String {
        let mut sql = format!("CREATE TABLE IF NOT EXISTS {} (\n", self.name);

        let mut col_defs = Vec::new();
        for col in &self.columns {
            let mut def = format!("  {} {}", col.name, col.sql_type);
            if col.primary_key {
                def.push_str(" PRIMARY KEY");
            }
            if col.auto_increment {
                def.push_str(" GENERATED ALWAYS AS IDENTITY");
            }
            if !col.nullable {
                def.push_str(" NOT NULL");
            }
            if col.unique && !col.primary_key {
                def.push_str(" UNIQUE");
            }
            if let Some(default) = &col.default {
                def.push_str(&format!(" DEFAULT {}", default));
            }
            col_defs.push(def);
        }

        // Add foreign key constraints
        for fk in &self.foreign_keys {
            col_defs.push(format!(
                "  FOREIGN KEY ({}) REFERENCES {}({})",
                fk.column, fk.ref_table, fk.ref_column
            ));
        }

        sql.push_str(&col_defs.join(",\n"));
        sql.push_str("\n);\n");

        // Add indexes
        for idx in &self.indexes {
            let unique = if idx.unique { "UNIQUE " } else { "" };
            sql.push_str(&format!(
                "CREATE {}INDEX IF NOT EXISTS {} ON {} ({});\n",
                unique, idx.name, self.name, idx.columns.join(", ")
            ));
        }

        sql
    }
}
