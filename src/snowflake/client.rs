use std::sync::Arc;
use std::time::{Duration, Instant};

use color_eyre::Result;
use tokio::process::Command;
use tokio::sync::Semaphore;
use tracing::{debug, error, warn};

use super::connection::ConnectionConfig;
use super::parser::{self, Column, ColumnsByTable, Database, QueryResult, Schema, Table};

const MAX_CONCURRENT: usize = 8;
const QUERY_TIMEOUT: Duration = Duration::from_secs(30);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct SnowClient {
    connection: ConnectionConfig,
    semaphore: Arc<Semaphore>,
}

impl SnowClient {
    pub fn new(connection: ConnectionConfig) -> Self {
        Self {
            connection,
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT)),
        }
    }

    async fn run_snow(&self, query: &str, timeout: Duration) -> Result<String> {
        let _permit = self.semaphore.acquire().await?;

        let mut args = self.connection.base_args();
        args.push("-q".to_string());
        args.push(query.to_string());

        debug!("Running: snow {}", args.join(" "));

        let output = tokio::time::timeout(timeout, Command::new("snow").args(&args).output())
            .await
            .map_err(|_| color_eyre::eyre::eyre!("Query timed out after {:?}", timeout))?
            .map_err(|e| color_eyre::eyre::eyre!("Failed to execute snow CLI: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("snow CLI error: {}", stderr);
            return Err(color_eyre::eyre::eyre!("snow CLI failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(stdout)
    }

    pub async fn execute_query(&self, sql: &str) -> Result<QueryResult> {
        let start = Instant::now();
        let output = self.run_snow(sql, QUERY_TIMEOUT).await?;
        let elapsed = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        parser::parse_query_result(&output, elapsed)
    }

    pub async fn list_databases(&self) -> Result<Vec<Database>> {
        let output = self.run_snow("SHOW DATABASES", QUERY_TIMEOUT).await?;
        parser::parse_databases(&output)
    }

    pub async fn list_columns(&self, db: &str, schema: &str, table: &str) -> Result<Vec<Column>> {
        let query = format!("DESCRIBE TABLE \"{db}\".\"{schema}\".\"{table}\"");
        let output = self.run_snow(&query, QUERY_TIMEOUT).await?;
        parser::parse_columns(&output)
    }

    /// Refresh catalog data for a database, returning schemas, tables, columns,
    /// and columns grouped by "SCHEMA.TABLE" key for the editor completion plugin.
    pub async fn refresh_catalog_data_with_columns(
        &self,
        db: &str,
    ) -> Result<(Vec<Schema>, Vec<Table>, Vec<Column>, ColumnsByTable)> {
        let query = format!(
            r#"SELECT
    c.TABLE_CATALOG AS database_name,
    c.TABLE_SCHEMA AS schema_name,
    t.TABLE_TYPE,
    c.TABLE_NAME,
    c.COLUMN_NAME,
    c.ORDINAL_POSITION,
    c.DATA_TYPE,
    c.IS_NULLABLE,
    t.ROW_COUNT,
    t.BYTES,
    t.CREATED,
    t.CLUSTERING_KEY,
    t.AUTO_CLUSTERING_ON,
    t.COMMENT,
    t.TABLE_OWNER
FROM "{db}".INFORMATION_SCHEMA.COLUMNS c
JOIN "{db}".INFORMATION_SCHEMA.TABLES t
    ON c.TABLE_SCHEMA = t.TABLE_SCHEMA
    AND c.TABLE_NAME = t.TABLE_NAME
ORDER BY c.TABLE_SCHEMA, c.TABLE_NAME, c.ORDINAL_POSITION"#
        );
        let output = self.run_snow(&query, QUERY_TIMEOUT).await?;
        parser::parse_catalog_data_with_columns(&output, db)
    }

    pub async fn heartbeat(&self) -> Result<bool> {
        match self.run_snow("SELECT 1", HEARTBEAT_TIMEOUT).await {
            Ok(_) => Ok(true),
            Err(e) => {
                warn!("Heartbeat failed: {}", e);
                Ok(false)
            }
        }
    }
}
