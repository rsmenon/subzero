use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use color_eyre::Result;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

use crate::snowflake::parser::{Column, Database, Schema, Table};

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct CatalogCache {
    pub databases: Vec<Database>,
    pub schemas: Vec<Schema>,
    pub tables: Vec<Table>,
    #[serde(default)]
    pub columns_by_table: HashMap<String, Vec<Column>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueryHistoryEntry {
    pub sql: String,
    pub executed_at: chrono::DateTime<chrono::Utc>,
    pub row_count: Option<usize>,
    pub elapsed_ms: Option<u64>,
    pub error: Option<String>,
}

use crate::components::query::chart::config::ChartTab;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedQuery {
    pub id: usize,
    pub title: String,
    pub sql: String,
    pub saved_at: chrono::DateTime<chrono::Utc>,
    pub last_run_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub charts: Vec<ChartTab>,
}

pub struct CacheManager {
    cache_dir: PathBuf,
    catalog: Mutex<CatalogCache>,
    /// Last time catalog was written to disk (for debouncing column updates)
    last_disk_write: Mutex<Option<std::time::Instant>>,
    /// Sanitized connection name ("_default" for None)
    active_conn_key: Mutex<String>,
    /// Incremented on each `switch_connection`; used as staleness guard for async tasks
    connection_generation: AtomicU64,
}

/// Returns the sanitized connection key for a given connection name.
/// `None` maps to `"_default"`, otherwise uses the name as-is.
#[allow(clippy::ref_option)]
pub fn conn_key(name: &Option<String>) -> String {
    match name {
        Some(n) => n.clone(),
        None => "_default".to_string(),
    }
}

impl CacheManager {
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(cache_dir: PathBuf, initial_conn: Option<String>) -> Self {
        std::fs::create_dir_all(&cache_dir).ok();
        let key = conn_key(&initial_conn);

        // One-time migration: if catalog_<key>.bin doesn't exist but catalog.bin does,
        // rename catalog.bin -> catalog_<key>.bin
        let new_path = cache_dir.join(format!("catalog_{key}.bin"));
        let old_path = cache_dir.join("catalog.bin");
        if !new_path.exists() && old_path.exists() {
            if let Err(e) = std::fs::rename(&old_path, &new_path) {
                warn!(
                    "Failed to migrate catalog.bin -> catalog_{}.bin: {}",
                    key, e
                );
            } else {
                info!("Migrated catalog.bin -> catalog_{}.bin", key);
            }
        }

        Self {
            cache_dir,
            catalog: Mutex::new(CatalogCache::default()),
            last_disk_write: Mutex::new(None),
            active_conn_key: Mutex::new(key),
            connection_generation: AtomicU64::new(0),
        }
    }

    /// Returns the current connection generation counter.
    pub fn generation(&self) -> u64 {
        self.connection_generation.load(Ordering::SeqCst)
    }

    /// Returns the current active connection key.
    pub fn active_conn_key(&self) -> String {
        self.active_conn_key
            .lock()
            .expect("active_conn_key Mutex poisoned")
            .clone()
    }

    /// Switch to a new connection. Bumps the generation counter, clears in-memory catalog,
    /// attempts to load from disk for the new connection, and rewrites nvim JSON.
    /// Returns (`new_generation`, cached catalog if any).
    #[allow(clippy::ref_option)]
    pub fn switch_connection(&self, name: &Option<String>) -> (u64, Option<CatalogCache>) {
        let key = conn_key(name);
        let new_gen = self.connection_generation.fetch_add(1, Ordering::SeqCst) + 1;

        // Update active key
        {
            let mut active = self
                .active_conn_key
                .lock()
                .expect("active_conn_key Mutex poisoned");
            active.clone_from(&key);
        }

        // Clear in-memory catalog
        {
            let mut mem = self.catalog.lock().expect("CatalogCache Mutex poisoned");
            *mem = CatalogCache::default();
        }

        // Try loading from disk for the new connection
        let cached = self.load_catalog();

        // Rewrite nvim JSON (will write empty or the loaded catalog)
        {
            let mem = self.catalog.lock().expect("CatalogCache Mutex poisoned");
            Self::write_catalog_json_inner(&mem);
        }

        (new_gen, cached)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.cache_dir.join(name)
    }

    pub fn save_catalog(&self, catalog: &CatalogCache, save_conn_key: &str) -> Result<()> {
        let data = bincode::serialize(catalog)?;
        let filename = format!("catalog_{save_conn_key}.bin");
        std::fs::write(self.path(&filename), data)?;
        // Only update in-memory state if conn_key matches active key (stale-safe)
        let active = self.active_conn_key();
        if save_conn_key == active {
            let mut mem = self.catalog.lock().expect("CatalogCache Mutex poisoned");
            mem.databases.clone_from(&catalog.databases);
            mem.schemas.clone_from(&catalog.schemas);
            mem.tables.clone_from(&catalog.tables);
            mem.columns_by_table.clone_from(&catalog.columns_by_table);
            mem.updated_at = catalog.updated_at;
        }
        debug!("Saved catalog cache to {}", filename);
        Ok(())
    }

    pub fn load_catalog(&self) -> Option<CatalogCache> {
        let key = self.active_conn_key();
        let filename = format!("catalog_{key}.bin");
        let data = std::fs::read(self.path(&filename)).ok()?;
        match bincode::deserialize::<CatalogCache>(&data) {
            Ok(catalog) => {
                // Populate in-memory state
                let mut mem = self.catalog.lock().expect("CatalogCache Mutex poisoned");
                mem.databases.clone_from(&catalog.databases);
                mem.schemas.clone_from(&catalog.schemas);
                mem.tables.clone_from(&catalog.tables);
                mem.columns_by_table.clone_from(&catalog.columns_by_table);
                mem.updated_at = catalog.updated_at;
                Some(catalog)
            }
            Err(e) => {
                warn!("Failed to deserialize catalog cache: {}", e);
                None
            }
        }
    }

    /// Look up cached columns for a table. Key format: "DB.SCHEMA.TABLE"
    pub fn get_columns(&self, key: &str) -> Option<Vec<Column>> {
        let mem = self.catalog.lock().expect("CatalogCache Mutex poisoned");
        mem.columns_by_table.get(key).cloned()
    }

    /// Update columns for a table and persist to disk + nvim JSON. Key format: "DB.SCHEMA.TABLE"
    /// Disk writes are debounced: at most once every 2 seconds to avoid I/O on every column load.
    pub fn update_columns(&self, key: &str, columns: Vec<Column>) {
        let mut mem = self.catalog.lock().expect("CatalogCache Mutex poisoned");
        mem.columns_by_table.insert(key.to_string(), columns);
        mem.updated_at = Some(chrono::Utc::now());

        // Always update nvim JSON (cheap in-memory operation)
        Self::write_catalog_json_inner(&mem);

        // Debounce disk writes: only serialize + write if >=2s since last write
        let should_write = {
            let last = self
                .last_disk_write
                .lock()
                .expect("last_disk_write Mutex poisoned");
            last.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(2))
        };

        if should_write {
            let data = bincode::serialize(&*mem).ok();
            drop(mem);
            if let Some(data) = data {
                let key = self.active_conn_key();
                let filename = format!("catalog_{key}.bin");
                if let Err(e) = std::fs::write(self.path(&filename), data) {
                    warn!("Failed to persist catalog cache after column update: {}", e);
                } else {
                    debug!("Updated column cache for {}", key);
                    let mut last = self
                        .last_disk_write
                        .lock()
                        .expect("last_disk_write Mutex poisoned");
                    *last = Some(std::time::Instant::now());
                }
            }
        } else {
            debug!("Debounced disk write for column update: {}", key);
        }
    }

    /// Build the catalog JSON string for RPC push or file write.
    /// Returns None if catalog is empty.
    pub fn build_catalog_json(&self) -> Option<String> {
        let mem = self.catalog.lock().expect("CatalogCache Mutex poisoned");
        Self::build_catalog_json_string(&mem)
    }

    /// Write the in-memory catalog as JSON for the embedded nvim completion plugin.
    /// Path: /tmp/sz-catalog-<pid>.json
    pub fn write_catalog_json(&self) {
        let mem = self.catalog.lock().expect("CatalogCache Mutex poisoned");
        Self::write_catalog_json_inner(&mem);
    }

    /// Build the catalog JSON string from in-memory state.
    #[allow(clippy::items_after_statements)]
    fn build_catalog_json_string(catalog: &CatalogCache) -> Option<String> {
        if catalog.databases.is_empty() {
            return None;
        }

        let db_names: Vec<&str> = catalog.databases.iter().map(|d| d.name.as_str()).collect();

        let mut schema_map: HashMap<&str, Vec<&str>> = HashMap::new();
        for s in &catalog.schemas {
            schema_map
                .entry(s.database.as_str())
                .or_default()
                .push(s.name.as_str());
        }

        #[derive(serde::Serialize)]
        struct TableEntry<'a> {
            name: &'a str,
            kind: &'a str,
        }

        let mut table_map: HashMap<String, Vec<TableEntry>> = HashMap::new();
        for t in &catalog.tables {
            let key = format!("{}.{}", t.database, t.schema);
            table_map.entry(key).or_default().push(TableEntry {
                name: &t.name,
                kind: &t.kind,
            });
        }

        #[derive(serde::Serialize)]
        struct ColumnEntry<'a> {
            name: &'a str,
            #[serde(rename = "type")]
            data_type: &'a str,
            nullable: bool,
        }

        let mut col_map: HashMap<&str, Vec<ColumnEntry>> = HashMap::new();
        for (key, cols) in &catalog.columns_by_table {
            let entries: Vec<ColumnEntry> = cols
                .iter()
                .map(|c| ColumnEntry {
                    name: &c.name,
                    data_type: &c.data_type,
                    nullable: c.nullable,
                })
                .collect();
            col_map.insert(key.as_str(), entries);
        }

        #[derive(serde::Serialize)]
        struct CatalogJson<'a> {
            databases: Vec<&'a str>,
            schemas: HashMap<&'a str, Vec<&'a str>>,
            tables: HashMap<String, Vec<TableEntry<'a>>>,
            columns: HashMap<&'a str, Vec<ColumnEntry<'a>>>,
        }

        let json_data = CatalogJson {
            databases: db_names,
            schemas: schema_map,
            tables: table_map,
            columns: col_map,
        };

        serde_json::to_string_pretty(&json_data).ok()
    }

    fn write_catalog_json_inner(catalog: &CatalogCache) {
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("sz-catalog-{pid}.json"));

        match Self::build_catalog_json_string(catalog) {
            Some(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    error!("Failed to write catalog JSON to {:?}: {}", path, e);
                } else {
                    info!("Wrote catalog JSON to {:?}", path);
                }
            }
            None => {
                debug!("No catalog data to write as JSON");
            }
        }
    }

    pub fn save_query_history(&self, history: &[QueryHistoryEntry]) -> Result<()> {
        let data = bincode::serialize(history)?;
        std::fs::write(self.path("query_history.bin"), data)?;
        debug!("Saved query history ({} entries)", history.len());
        Ok(())
    }

    pub fn load_query_history(&self) -> Vec<QueryHistoryEntry> {
        let Ok(data) = std::fs::read(self.path("query_history.bin")) else {
            return Vec::new();
        };
        bincode::deserialize(&data).unwrap_or_default()
    }

    pub fn save_saved_queries(&self, queries: &[SavedQuery]) -> Result<()> {
        let data = bincode::serialize(queries)?;
        std::fs::write(self.path("saved_queries.bin"), data)?;
        debug!("Saved {} saved queries", queries.len());
        Ok(())
    }

    pub fn load_saved_queries(&self) -> Vec<SavedQuery> {
        let Ok(data) = std::fs::read(self.path("saved_queries.bin")) else {
            return Vec::new();
        };
        bincode::deserialize(&data).unwrap_or_default()
    }
}
