#![allow(clippy::too_many_lines)]

use color_eyre::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Database {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    pub name: String,
    pub database: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Table {
    pub name: String,
    pub database: String,
    pub schema: String,
    pub kind: String, // TABLE, VIEW, etc.
    pub row_count: Option<i64>,
    pub bytes: Option<i64>,
    pub created: Option<String>,
    pub clustering_key: Option<String>,
    pub auto_clustering: Option<bool>,
    pub comment: Option<String>,
    pub owner: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub row_count: usize,
    pub elapsed_ms: u64,
}

/// Parse `JSON_EXT` output from `snow sql` for SHOW DATABASES
pub fn parse_databases(json: &str) -> Result<Vec<Database>> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    let mut databases = Vec::new();

    if let Some(rows) = value.as_array() {
        for row in rows {
            if let Some(name) = row.get("name").and_then(|n| n.as_str()) {
                databases.push(Database {
                    name: name.to_string(),
                });
            }
        }
    }

    Ok(databases)
}

/// Parse `JSON_EXT` output from `snow sql` for DESCRIBE TABLE
pub fn parse_columns(json: &str) -> Result<Vec<Column>> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    let mut columns = Vec::new();

    if let Some(rows) = value.as_array() {
        for row in rows {
            if let Some(name) = row.get("name").and_then(|n| n.as_str()) {
                columns.push(Column {
                    name: name.to_string(),
                    data_type: row
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("UNKNOWN")
                        .to_string(),
                    nullable: row
                        .get("null?")
                        .and_then(|n| n.as_str())
                        .is_none_or(|n| n == "Y"),
                });
            }
        }
    }

    Ok(columns)
}

/// Columns grouped by "SCHEMA.TABLE" key within a database
pub type ColumnsByTable = std::collections::HashMap<String, Vec<Column>>;

/// Parse catalog data and return columns grouped by "SCHEMA.TABLE" key.
/// Single-pass: parses JSON once, builds all outputs in one iteration.
#[allow(clippy::type_complexity)]
pub fn parse_catalog_data_with_columns(
    json: &str,
    db: &str,
) -> Result<(Vec<Schema>, Vec<Table>, Vec<Column>, ColumnsByTable)> {
    #[derive(Default)]
    struct TableMeta {
        kind: String,
        row_count: Option<i64>,
        bytes: Option<i64>,
        created: Option<String>,
        clustering_key: Option<String>,
        auto_clustering: Option<bool>,
        comment: Option<String>,
        owner: Option<String>,
    }

    let value: serde_json::Value = serde_json::from_str(json)?;
    let empty_vec = vec![];
    let rows = value.as_array().unwrap_or(&empty_vec);

    let mut schemas = std::collections::BTreeSet::new();

    let mut tables_map: std::collections::BTreeMap<(String, String), TableMeta> =
        std::collections::BTreeMap::new();
    let mut columns = Vec::new();
    let mut grouped: ColumnsByTable = std::collections::HashMap::new();

    for row in rows {
        let schema_name = row
            .get("SCHEMA_NAME")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let table_type = row
            .get("TABLE_TYPE")
            .and_then(|v| v.as_str())
            .unwrap_or("TABLE");
        let table_name = row
            .get("TABLE_NAME")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let column_name = row
            .get("COLUMN_NAME")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let data_type = row
            .get("DATA_TYPE")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN");
        let is_nullable = row
            .get("IS_NULLABLE")
            .and_then(|v| v.as_str())
            .is_none_or(|v| v == "YES");

        schemas.insert(schema_name.to_string());

        let kind = if table_type.contains("VIEW") {
            "VIEW".to_string()
        } else {
            "TABLE".to_string()
        };

        tables_map
            .entry((schema_name.to_string(), table_name.to_string()))
            .or_insert_with(|| {
                let row_count = row.get("ROW_COUNT").and_then(|v| {
                    v.as_i64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                });
                let bytes = row.get("BYTES").and_then(|v| {
                    v.as_i64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                });
                let created = row
                    .get("CREATED")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string);
                let clustering_key = row
                    .get("CLUSTERING_KEY")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(std::string::ToString::to_string);
                let auto_clustering = row.get("AUTO_CLUSTERING_ON").and_then(|v| {
                    v.as_bool().or_else(|| {
                        v.as_str().map(|s| {
                            s.eq_ignore_ascii_case("yes")
                                || s.eq_ignore_ascii_case("true")
                                || s == "ON"
                        })
                    })
                });
                let comment = row
                    .get("COMMENT")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(std::string::ToString::to_string);
                let owner = row
                    .get("TABLE_OWNER")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(std::string::ToString::to_string);

                TableMeta {
                    kind,
                    row_count,
                    bytes,
                    created,
                    clustering_key,
                    auto_clustering,
                    comment,
                    owner,
                }
            });

        let col = Column {
            name: column_name.to_string(),
            data_type: data_type.to_string(),
            nullable: is_nullable,
        };

        // Build grouped columns by "SCHEMA.TABLE" key
        let key = format!("{schema_name}.{table_name}");
        grouped.entry(key).or_default().push(col.clone());

        columns.push(col);
    }

    let schema_list: Vec<Schema> = schemas
        .into_iter()
        .map(|name| Schema {
            name,
            database: db.to_string(),
        })
        .collect();

    let table_list: Vec<Table> = tables_map
        .into_iter()
        .map(|((schema, name), meta)| Table {
            name,
            database: db.to_string(),
            schema,
            kind: meta.kind,
            row_count: meta.row_count,
            bytes: meta.bytes,
            created: meta.created,
            clustering_key: meta.clustering_key,
            auto_clustering: meta.auto_clustering,
            comment: meta.comment,
            owner: meta.owner,
        })
        .collect();

    Ok((schema_list, table_list, columns, grouped))
}

/// Parse generic query results from `JSON_EXT` format
pub fn parse_query_result(json: &str, elapsed_ms: u64) -> Result<QueryResult> {
    let value: serde_json::Value = serde_json::from_str(json)?;

    let empty_vec = vec![];
    let rows_data = value.as_array().unwrap_or(&empty_vec);

    // Extract column names from first row
    let columns: Vec<String> = rows_data
        .first()
        .and_then(|row| row.as_object())
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default();

    // Convert rows to vec of vec
    let rows: Vec<Vec<serde_json::Value>> = rows_data
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|col| row.get(col).cloned().unwrap_or(serde_json::Value::Null))
                .collect()
        })
        .collect();

    let row_count = rows.len();

    Ok(QueryResult {
        columns,
        rows,
        row_count,
        elapsed_ms,
    })
}
