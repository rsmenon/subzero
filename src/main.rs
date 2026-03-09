#![allow(clippy::too_many_lines)]

mod action;
mod app;
mod cache;
mod components;
mod config;
mod event;
mod snowflake;
mod theme;
mod tui;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use color_eyre::Result;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::components::query::nvim_rpc::NvimRpcConnection;

use crate::action::Action;
use crate::app::App;
use crate::cache::CacheManager;
use crate::cache::store::{QueryHistoryEntry, SavedQuery};
use crate::components::Component;
use crate::config::AppConfig;
use crate::event::{AppEvent, EventHandler};
use crate::snowflake::client::SnowClient;
use crate::snowflake::connection::ConnectionConfig;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    // Load configuration
    let config = AppConfig::load()?;

    // Set up tracing to file
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config.log_path)?;

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(log_file)
        .with_ansi(false)
        .init();

    info!("sz starting up");

    // Load theme (must happen before any rendering)
    let theme_path = config.app_dir.join("theme.yaml");
    if !theme_path.exists() {
        let default_config = theme::ThemeConfig::default();
        theme::save(&default_config, &theme_path).ok();
    }
    theme::load(&theme_path);

    // Initialize terminal
    let mut terminal = tui::init()?;

    // Create app
    let mut app = App::new();

    // Create event handler
    let mut events = EventHandler::new();

    // Set up Snowflake client
    let conn_config = ConnectionConfig::from_app_config(&config);
    let snow_client = Arc::new(RwLock::new(SnowClient::new(conn_config)));

    // Set up cache manager
    let cache = Arc::new(CacheManager::new(
        config.cache_dir.clone(),
        config.connection_name.clone(),
    ));

    // Shared slot for passing the NvimRpc handle from the async connect task to the main thread.
    // Created here, shared with the connect task, consumed when NvimRpcConnected arrives.
    // Shared slot for passing the NvimRpc connection from async tasks.
    // After NvimRpcConnected, sync_state is moved to the editor and only the async handle remains.
    let nvim_rpc_slot: Arc<tokio::sync::Mutex<Option<NvimRpcConnection>>> =
        Arc::new(tokio::sync::Mutex::new(None));

    // Wire up the nvim editor's action channel
    app.query.set_action_tx(events.action_tx.clone());

    // Load query history into the query pane
    let history_entries = cache.load_query_history();
    app.query.set_history(history_entries);

    // Load saved queries
    let saved = cache.load_saved_queries();
    app.saved_queries_pane.set_queries(saved);

    // Initialize settings pane with config data
    {
        let connections = config
            .snowflake
            .as_ref()
            .map(|s| s.connections.clone())
            .unwrap_or_default();
        let default_conn = config.connection_name.clone();
        app.settings
            .init_from_config(connections, default_conn, config.cache_dir.clone());
    }

    // Start heartbeat background task
    let heartbeat_tx = events.action_tx.clone();
    let heartbeat_client = Arc::clone(&snow_client);
    tokio::spawn(async move {
        // Initial heartbeat
        let client_snapshot = heartbeat_client
            .read()
            .expect("SnowClient RwLock poisoned")
            .clone();
        let result = client_snapshot.heartbeat().await.unwrap_or(false);
        let _ = heartbeat_tx.send(AppEvent::Action(Action::HeartbeatResult(result)));

        // Periodic heartbeat every 30 seconds
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.tick().await; // skip first immediate tick
        loop {
            interval.tick().await;
            let client_snapshot = heartbeat_client
                .read()
                .expect("SnowClient RwLock poisoned")
                .clone();
            let result = client_snapshot.heartbeat().await.unwrap_or(false);
            if heartbeat_tx
                .send(AppEvent::Action(Action::HeartbeatResult(result)))
                .is_err()
            {
                break;
            }
        }
    });

    // Load catalog from cache on startup (never auto-refresh from Snowflake)
    {
        let catalog_tx = events.action_tx.clone();
        let catalog_cache = Arc::clone(&cache);

        if let Some(cached) = catalog_cache.load_catalog() {
            catalog_cache.write_catalog_json();
            let _ = catalog_tx.send(AppEvent::Action(Action::CatalogLoaded(
                Arc::new(cached.databases),
                Arc::new(cached.schemas),
                Arc::new(cached.tables),
                cached.updated_at,
            )));
        }
    }

    // Initial render
    terminal.draw(|frame| app.render(frame))?;

    // Main event loop
    while app.running {
        let event = events.next().await?;

        match event {
            AppEvent::Key(key) => {
                if let Some(action) = app.handle_key(key)? {
                    // Process action chain
                    let mut current = Some(action);
                    while let Some(act) = current {
                        current = handle_action(
                            &mut app,
                            act,
                            &snow_client,
                            &events,
                            &cache,
                            &config,
                            &mut terminal,
                            &nvim_rpc_slot,
                        )?;
                    }
                }
                // Render after key events
                terminal.draw(|frame| app.render(frame))?;
            }
            AppEvent::Action(action) => {
                let mut current = Some(action);
                while let Some(act) = current {
                    current = handle_action(
                        &mut app,
                        act,
                        &snow_client,
                        &events,
                        &cache,
                        &config,
                        &mut terminal,
                        &nvim_rpc_slot,
                    )?;
                }
                terminal.draw(|frame| app.render(frame))?;
            }
            AppEvent::Resize => {
                terminal.draw(|frame| app.render(frame))?;
            }
            // Tick and Mouse events are not yet handled
            AppEvent::Tick | AppEvent::Mouse => {}
        }
    }

    // Clean shutdown
    info!("sz shutting down");

    // Remove catalog JSON temp file
    let catalog_json_path =
        std::env::temp_dir().join(format!("sz-catalog-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&catalog_json_path);

    tui::restore()?;

    Ok(())
}

/// Optimized catalog refresh: `list_databases` first, then `refresh_catalog_data` per DB in parallel
#[allow(clippy::too_many_lines)]
async fn spawn_optimized_catalog_refresh(
    catalog_tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
    catalog_client: SnowClient,
    catalog_cache: Arc<CacheManager>,
    list_db_task_id: usize,
    conn_key: String,
    expected_gen: u64,
) {
    match catalog_client.list_databases().await {
        Ok(dbs) => {
            // Type alias for per-db results: (db_name, schemas, tables, columns_by_table)
            type DbResult = (
                String,
                Vec<crate::snowflake::Schema>,
                Vec<crate::snowflake::Table>,
                crate::snowflake::parser::ColumnsByTable,
            );

            let _ = catalog_tx.send(AppEvent::Action(Action::CompleteTask(list_db_task_id)));
            // Only send progressive UI updates if still the active connection
            if catalog_cache.generation() == expected_gen {
                let _ = catalog_tx.send(AppEvent::Action(Action::DatabasesLoaded(dbs.clone())));
            }

            // Create all per-database tasks upfront (Pending status)
            for db in &dbs {
                let _ = catalog_tx.send(AppEvent::Action(Action::CreateTask {
                    source: "Catalog Refresh".to_string(),
                    name: format!("Refresh data for catalog = {}", db.name),
                }));
            }

            // Determine parallelism: 2.5x CPU cores, capped at db count
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let max_parallel = {
                let num_cpus = std::thread::available_parallelism()
                    .map(std::num::NonZero::get)
                    .unwrap_or(4);
                ((num_cpus as f64 * 2.5) as usize).max(1)
            };
            let semaphore = Arc::new(tokio::sync::Semaphore::new(max_parallel));

            // Spawn all DB refreshes in parallel, gated by the semaphore
            let mut handles = Vec::with_capacity(dbs.len());
            for db in &dbs {
                let db_name = db.name.clone();
                let client = catalog_client.clone();
                let tx = catalog_tx.clone();
                let sem = Arc::clone(&semaphore);
                let cache_ref = Arc::clone(&catalog_cache);

                let handle = tokio::spawn(async move {
                    let _permit = sem.acquire().await;
                    match client.refresh_catalog_data_with_columns(&db_name).await {
                        Ok((schemas, tables, _columns, columns_by_table)) => {
                            // Only send progressive UI update if still the active connection
                            if cache_ref.generation() == expected_gen {
                                let schemas_arc = Arc::new(schemas.clone());
                                let tables_arc = Arc::new(tables.clone());
                                let _ = tx.send(AppEvent::Action(Action::DatabaseCatalogLoaded {
                                    db: db_name.clone(),
                                    schemas: schemas_arc,
                                    tables: tables_arc,
                                }));
                            }
                            let result: DbResult = (db_name, schemas, tables, columns_by_table);
                            Ok(result)
                        }
                        Err(e) => {
                            error!("Failed to refresh catalog data for {}: {}", db_name, e);
                            Err(e)
                        }
                    }
                });
                handles.push(handle);
            }

            // Collect all results
            let mut all_schemas = Vec::new();
            let mut all_tables = Vec::new();
            // Map of "DB.SCHEMA.TABLE" -> Vec<Column> for the catalog JSON export
            let mut all_columns_by_table: HashMap<String, Vec<crate::snowflake::Column>> =
                HashMap::new();

            for handle in handles {
                if let Ok(Ok((db_name, schemas, tables, columns_by_table))) = handle.await {
                    // Merge columns_by_table with DB prefix
                    for (schema_table, cols) in columns_by_table {
                        let key = format!("{db_name}.{schema_table}");
                        all_columns_by_table.insert(key, cols);
                    }
                    all_schemas.extend(schemas);
                    all_tables.extend(tables);
                }
            }

            // Send final ObjectsLoaded with all data
            let catalog = crate::cache::store::CatalogCache {
                databases: dbs.clone(),
                schemas: all_schemas.clone(),
                tables: all_tables.clone(),
                columns_by_table: all_columns_by_table.clone(),
                updated_at: Some(chrono::Utc::now()),
            };
            // Always save to disk under the correct connection key
            let _ = catalog_cache.save_catalog(&catalog, &conn_key);

            // Only send UI updates if this refresh is still for the active connection
            if catalog_cache.generation() == expected_gen {
                // Write catalog JSON for the embedded nvim editor's completion plugin
                catalog_cache.write_catalog_json();

                // Also push via RPC if connected (the action handler will check)
                if let Some(json) = catalog_cache.build_catalog_json() {
                    let _ = catalog_tx.send(AppEvent::Action(Action::PushCatalogToNvim(json)));
                }

                let _ = catalog_tx.send(AppEvent::Action(Action::ObjectsLoaded(
                    all_schemas,
                    all_tables,
                )));
                let _ = catalog_tx.send(AppEvent::Action(Action::CachedColumnsRefreshed));
            } else {
                info!(
                    "Catalog refresh for '{}' completed but connection switched (gen {} != {}); skipping UI updates",
                    conn_key, expected_gen, catalog_cache.generation()
                );
            }
        }
        Err(e) => {
            let _ = catalog_tx.send(AppEvent::Action(Action::CompleteTask(list_db_task_id)));
            let _ = catalog_tx.send(AppEvent::Action(Action::CatalogError(e.to_string())));
        }
    }
}

/// Handle actions that need async/terminal access (`ExecuteQuery`, `LaunchExternalEditor`).
/// Returns the next action in the chain, or None.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn handle_action(
    app: &mut App,
    action: Action,
    snow_client: &Arc<RwLock<SnowClient>>,
    events: &EventHandler,
    cache: &Arc<CacheManager>,
    config: &AppConfig,
    terminal: &mut tui::Tui,
    nvim_rpc_slot: &Arc<tokio::sync::Mutex<Option<NvimRpcConnection>>>,
) -> Result<Option<Action>> {
    match action {
        Action::ExecuteQuery(ref sql) => {
            // Create background task for query execution
            let task_id = app.task_manager.create_task(
                "Query Execution",
                &format!("Execute: {}", &sql.chars().take(50).collect::<String>()),
            );
            app.task_manager.start_task(task_id);

            // Tell query pane to show loading state
            app.query.update(Action::ExecuteQuery(sql.clone()))?;
            terminal.draw(|frame| app.render(frame))?;

            // Update last_run_at for saved query if loaded from one
            if let Some(saved_id) = app.loaded_saved_query_id {
                if let Some(q) = app
                    .saved_queries_pane
                    .queries
                    .iter_mut()
                    .find(|q| q.id == saved_id)
                {
                    q.last_run_at = Some(chrono::Utc::now());
                }
                let _ = cache.save_saved_queries(&app.saved_queries_pane.queries);
            }

            // Spawn async query execution
            let query_tx = events.action_tx.clone();
            let client = snow_client
                .read()
                .expect("SnowClient RwLock poisoned")
                .clone();
            let sql_owned = sql.clone();
            let cache_clone = Arc::clone(cache);
            tokio::spawn(async move {
                match client.execute_query(&sql_owned).await {
                    Ok(result) => {
                        // Save to history
                        let entry = QueryHistoryEntry {
                            sql: sql_owned,
                            executed_at: chrono::Utc::now(),
                            row_count: Some(result.row_count),
                            elapsed_ms: Some(result.elapsed_ms),
                            error: None,
                        };
                        save_history_entry(&cache_clone, entry);
                        let _ = query_tx.send(AppEvent::Action(Action::CompleteTask(task_id)));
                        let _ = query_tx
                            .send(AppEvent::Action(Action::QueryResultData(Arc::new(result))));
                        // Update history in query pane
                        let updated = cache_clone.load_query_history();
                        let _ = query_tx.send(AppEvent::Action(Action::SetQueryHistory(updated)));
                    }
                    Err(e) => {
                        let err_msg = e.to_string();
                        let entry = QueryHistoryEntry {
                            sql: sql_owned,
                            executed_at: chrono::Utc::now(),
                            row_count: None,
                            elapsed_ms: None,
                            error: Some(err_msg.clone()),
                        };
                        save_history_entry(&cache_clone, entry);
                        let _ = query_tx.send(AppEvent::Action(Action::CompleteTask(task_id)));
                        let _ = query_tx.send(AppEvent::Action(Action::QueryError(err_msg)));
                        let updated = cache_clone.load_query_history();
                        let _ = query_tx.send(AppEvent::Action(Action::SetQueryHistory(updated)));
                    }
                }
            });
            Ok(None)
        }
        Action::RefreshCatalog => {
            // Set loading state with progress tracking
            app.explore.tree.set_loading();

            // Create the "Get all catalogs" task synchronously so we have the ID
            let list_db_task_id = app
                .task_manager
                .create_task("Catalog Refresh", "Get all catalogs");
            app.task_manager.start_task(list_db_task_id);

            // Update refresh counts
            let running = app.task_manager.running_count("Catalog Refresh");
            let total = app.task_manager.pending_or_running_count("Catalog Refresh");
            app.explore.tree.refresh_running = running;
            app.explore.tree.refresh_total = total;
            app.settings.refresh_running = running;
            app.settings.refresh_total = total;

            terminal.draw(|frame| app.render(frame))?;

            let catalog_tx = events.action_tx.clone();
            let catalog_client = snow_client
                .read()
                .expect("SnowClient RwLock poisoned")
                .clone();
            let catalog_cache = Arc::clone(cache);
            let refresh_conn_key = cache.active_conn_key();
            let refresh_gen = cache.generation();

            tokio::spawn(async move {
                spawn_optimized_catalog_refresh(
                    catalog_tx,
                    catalog_client,
                    catalog_cache,
                    list_db_task_id,
                    refresh_conn_key,
                    refresh_gen,
                )
                .await;
            });
            Ok(None)
        }
        Action::LoadColumns {
            ref db,
            ref schema,
            ref table,
        } => {
            let col_tx = events.action_tx.clone();
            let col_client = snow_client
                .read()
                .expect("SnowClient RwLock poisoned")
                .clone();
            let col_cache = Arc::clone(cache);
            let db = db.clone();
            let schema = schema.clone();
            let table = table.clone();

            // Set loading state directly (do NOT dispatch LoadColumns back through
            // app.dispatch — that would re-enter handle_action and infinite-loop).
            app.explore.detail.set_breadcrumb(&db, &schema, &table);
            app.explore
                .detail
                .set_table_meta(app.explore.tree.find_table(&db, &schema, &table));
            app.explore.detail.set_columns_loading();
            terminal.draw(|frame| app.render(frame))?;

            tokio::spawn(async move {
                // Try catalog cache first
                let cache_key = format!("{db}.{schema}.{table}");
                if let Some(columns) = col_cache.get_columns(&cache_key) {
                    let _ = col_tx.send(AppEvent::Action(Action::ColumnsLoaded {
                        db,
                        schema,
                        table,
                        columns,
                    }));
                    return;
                }
                match col_client.list_columns(&db, &schema, &table).await {
                    Ok(columns) => {
                        col_cache.update_columns(&cache_key, columns.clone());
                        let _ = col_tx.send(AppEvent::Action(Action::ColumnsLoaded {
                            db,
                            schema,
                            table,
                            columns,
                        }));
                    }
                    Err(e) => {
                        let _ = col_tx.send(AppEvent::Action(Action::ColumnsError {
                            db,
                            schema,
                            table,
                            error: e.to_string(),
                        }));
                    }
                }
            });
            Ok(None)
        }
        Action::RefreshColumns {
            ref db,
            ref schema,
            ref table,
        } => {
            let task_id = app.task_manager.create_task(
                "Column Refresh",
                &format!("Refresh columns for {db}.{schema}.{table}"),
            );
            app.task_manager.start_task(task_id);

            let col_tx = events.action_tx.clone();
            let col_client = snow_client
                .read()
                .expect("SnowClient RwLock poisoned")
                .clone();
            let col_cache = Arc::clone(cache);
            let db = db.clone();
            let schema = schema.clone();
            let table = table.clone();

            app.explore.detail.set_breadcrumb(&db, &schema, &table);
            app.explore
                .detail
                .set_table_meta(app.explore.tree.find_table(&db, &schema, &table));
            app.explore.detail.set_columns_loading();
            terminal.draw(|frame| app.render(frame))?;

            tokio::spawn(async move {
                let cache_key = format!("{db}.{schema}.{table}");
                match col_client.list_columns(&db, &schema, &table).await {
                    Ok(columns) => {
                        col_cache.update_columns(&cache_key, columns.clone());
                        let _ = col_tx.send(AppEvent::Action(Action::CompleteTask(task_id)));
                        let _ = col_tx.send(AppEvent::Action(Action::ColumnsLoaded {
                            db,
                            schema,
                            table,
                            columns,
                        }));
                    }
                    Err(e) => {
                        let _ = col_tx.send(AppEvent::Action(Action::CompleteTask(task_id)));
                        let _ = col_tx.send(AppEvent::Action(Action::ColumnsError {
                            db,
                            schema,
                            table,
                            error: e.to_string(),
                        }));
                    }
                }
            });
            Ok(None)
        }
        Action::LoadPreview {
            ref db,
            ref schema,
            ref table,
        } => {
            let preview_tx = events.action_tx.clone();
            let preview_client = snow_client
                .read()
                .expect("SnowClient RwLock poisoned")
                .clone();
            let db = db.clone();
            let schema = schema.clone();
            let table = table.clone();

            // Set loading state directly (do NOT dispatch LoadPreview back through
            // app.dispatch — that would re-enter handle_action and infinite-loop).
            app.explore.detail.set_breadcrumb(&db, &schema, &table);
            app.explore
                .detail
                .set_table_meta(app.explore.tree.find_table(&db, &schema, &table));
            app.explore.detail.set_preview_loading();
            terminal.draw(|frame| app.render(frame))?;

            tokio::spawn(async move {
                let sql = format!(
                    "SELECT * FROM \"{db}\".\"{schema}\".\"{table}\" LIMIT 100"
                );
                match preview_client.execute_query(&sql).await {
                    Ok(result) => {
                        let _ = preview_tx.send(AppEvent::Action(Action::PreviewLoaded {
                            db,
                            schema,
                            table,
                            result,
                        }));
                    }
                    Err(e) => {
                        let _ = preview_tx.send(AppEvent::Action(Action::PreviewError {
                            db,
                            schema,
                            table,
                            error: e.to_string(),
                        }));
                    }
                }
            });
            Ok(None)
        }
        Action::ApplyOverrides {
            ref connection_name,
            ..
        } => {
            // Build a new ConnectionConfig with the selected connection name
            let new_conn = ConnectionConfig {
                connection_name: connection_name.clone(),
            };
            let new_client = SnowClient::new(new_conn);
            // Actually switch the connection by replacing the inner client
            *snow_client.write().expect("SnowClient RwLock poisoned") = new_client;

            // Switch catalog cache to the new connection
            let (_new_gen, cached_catalog) = cache.switch_connection(connection_name);
            app.explore.tree.clear();
            app.explore.detail = crate::components::explore::detail::DetailPanel::new();
            app.explore.catalog_loaded = false;

            if let Some(catalog) = cached_catalog {
                app.explore.catalog_loaded = true;
                let _ = events.action_tx.send(AppEvent::Action(Action::CatalogLoaded(
                    Arc::new(catalog.databases),
                    Arc::new(catalog.schemas),
                    Arc::new(catalog.tables),
                    catalog.updated_at,
                )));
            }

            // Push catalog (possibly empty) to nvim
            if let Some(json) = cache.build_catalog_json() {
                let _ = events.action_tx.send(AppEvent::Action(Action::PushCatalogToNvim(json)));
            } else {
                let _ = events
                    .action_tx
                    .send(AppEvent::Action(Action::PushCatalogToNvim("{}".into())));
            }

            // Run heartbeat against the new connection
            let heartbeat_tx = events.action_tx.clone();
            let heartbeat_snapshot = snow_client
                .read()
                .expect("SnowClient RwLock poisoned")
                .clone();
            tokio::spawn(async move {
                let result = heartbeat_snapshot.heartbeat().await.unwrap_or(false);
                let _ = heartbeat_tx.send(AppEvent::Action(Action::HeartbeatResult(result)));
            });
            // Auto-clear settings status after 3 seconds
            let clear_tx = events.action_tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(3)).await;
                let _ = clear_tx.send(AppEvent::Action(Action::ClearStatus));
            });
            Ok(Some(Action::Render))
        }
        Action::ClearCache => {
            // Remove cache files but preserve user data (saved queries, query history)
            let preserve = ["saved_queries.bin", "query_history.bin"];
            if let Ok(entries) = std::fs::read_dir(&config.cache_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if !preserve.iter().any(|p| *p == name_str.as_ref()) {
                        std::fs::remove_file(entry.path()).ok();
                    }
                }
            }
            app.settings.update(Action::CacheClearDone)?;
            // Auto-clear settings status after 3 seconds
            let clear_tx = events.action_tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(3)).await;
                let _ = clear_tx.send(AppEvent::Action(Action::ClearStatus));
            });
            Ok(Some(Action::Render))
        }
        Action::StatusMessage(ref _msg) => {
            // Dispatch to the component to display
            let result = app.dispatch(action)?;
            // Spawn a timer to auto-clear the status after 3 seconds
            let clear_tx = events.action_tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(3)).await;
                let _ = clear_tx.send(AppEvent::Action(Action::ClearStatus));
            });
            Ok(result)
        }
        Action::CachedColumnsRefreshed => {
            let result = app.dispatch(Action::CachedColumnsRefreshed);
            // BUG-5: Spawn a 2-second timer to trigger re-render so AllDone auto-disappears
            let render_tx = events.action_tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let _ = render_tx.send(AppEvent::Action(Action::Render));
            });
            result
        }
        Action::SaveQueryToHistory => {
            let sql = app.query.get_sql();
            if sql.is_empty() {
                return Ok(Some(Action::StatusMessage(
                    "Nothing to save — editor is empty".to_string(),
                )));
            }

            let charts = app.query.get_chart_configs();

            // Check if we have a loaded saved query
            if let Some(saved_id) = app.loaded_saved_query_id {
                // Overwrite the existing saved query
                if let Some(q) = app
                    .saved_queries_pane
                    .queries
                    .iter_mut()
                    .find(|q| q.id == saved_id)
                {
                    q.sql = sql;
                    q.saved_at = chrono::Utc::now();
                    q.charts = charts;
                }
            } else {
                // Save as new
                let next_id = app
                    .saved_queries_pane
                    .queries
                    .iter()
                    .map(|q| q.id)
                    .max()
                    .unwrap_or(0)
                    + 1;
                let count = app.saved_queries_pane.queries.len() + 1;
                app.saved_queries_pane.queries.push(SavedQuery {
                    id: next_id,
                    title: format!("Query {count}"),
                    sql,
                    saved_at: chrono::Utc::now(),
                    last_run_at: None,
                    charts,
                });
                app.loaded_saved_query_id = Some(next_id);
            }
            let _ = cache.save_saved_queries(&app.saved_queries_pane.queries);
            Ok(Some(Action::StatusMessage("Query saved".to_string())))
        }
        Action::PersistSavedQueries => {
            let _ = cache.save_saved_queries(&app.saved_queries_pane.queries);
            Ok(Some(Action::Render))
        }
        Action::DatabaseCatalogLoaded { ref db, .. } => {
            // Complete the matching catalog refresh task for this db
            let task_name = format!("Refresh data for catalog = {db}");
            if let Some(task) = app
                .task_manager
                .find_running_task("Catalog Refresh", &task_name)
            {
                app.task_manager.complete_task(task);
            }
            // Update the tree with per-database data
            let result = app.dispatch(action)?;
            // Update refresh counts
            let running = app.task_manager.running_count("Catalog Refresh");
            let total = app.task_manager.pending_or_running_count("Catalog Refresh");
            app.explore.tree.refresh_running = running;
            app.explore.tree.refresh_total = total;
            app.settings.refresh_running = running;
            app.settings.refresh_total = total;
            Ok(result)
        }
        Action::NvimReady => {
            // Nvim just spawned — start background task to connect RPC
            let socket_path = app.query.editor_mut().socket_path().clone();
            let rpc_tx = events.action_tx.clone();
            let rpc_slot = Arc::clone(nvim_rpc_slot);
            tokio::spawn(async move {
                match crate::components::query::nvim_rpc::connect(&socket_path, rpc_tx.clone()).await {
                    Ok(conn) => {
                        info!("nvim RPC connection established");
                        // Store the async RPC handle in the shared slot
                        *rpc_slot.lock().await = Some(conn);
                        let _ = rpc_tx.send(AppEvent::Action(Action::NvimRpcConnected));
                    }
                    Err(e) => {
                        warn!("Failed to connect nvim RPC: {}", e);
                        let _ = rpc_tx.send(AppEvent::Action(Action::StatusMessage(
                            "RPC connection failed; using fallback".into(),
                        )));
                    }
                }
            });
            // Also dispatch to query pane for the NvimReady notification
            app.dispatch(Action::NvimReady)
        }
        Action::NvimRpcConnected => {
            // Retrieve the sync state from the shared slot.
            // The async NvimRpc handle stays in the slot for future async tasks.
            // try_lock() is safe here: the connect task drops the lock guard before
            // sending NvimRpcConnected (see spawn above), so the lock is always free
            // by the time we reach this handler.
            if let Ok(mut slot) = nvim_rpc_slot.try_lock() {
                if let Some(ref mut conn) = *slot {
                    // Surface channel_id == 0 warning to the user
                    if let Some(ref warning) = conn.setup_warning {
                        let _ = events.action_tx.send(AppEvent::Action(
                            Action::StatusMessage(warning.clone()),
                        ));
                    }
                    if let Some(sync_state) = conn.take_sync_state() {
                        app.query.editor_mut().set_rpc_sync(sync_state);
                    }
                }
            } // slot dropped here

            // If we have catalog data, push it via RPC now
            if app.query.editor_mut().has_rpc() {
                if let Some(json) = cache.build_catalog_json() {
                    let rpc_slot = Arc::clone(nvim_rpc_slot);
                    tokio::spawn(async move {
                        let slot = rpc_slot.lock().await;
                        if let Some(ref conn) = *slot {
                            if let Err(e) = conn.rpc.push_catalog(&json).await {
                                warn!("Initial catalog push failed: {}", e);
                            } else {
                                info!("Initial catalog pushed via RPC");
                            }
                        }
                    });
                }

                // Start periodic mode sync task (every 500ms)
                let rpc_slot_mode = Arc::clone(nvim_rpc_slot);
                let mode_tx = events.action_tx.clone();
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(Duration::from_millis(500));
                    loop {
                        interval.tick().await;
                        let slot = rpc_slot_mode.lock().await;
                        if let Some(ref conn) = *slot {
                            conn.rpc.sync_mode().await;
                        } else {
                            break; // RPC gone, stop syncing
                        }
                        drop(slot);
                        // Trigger a render so mode changes are reflected
                        if mode_tx.send(AppEvent::Action(Action::Render)).is_err() {
                            break;
                        }
                    }
                });
            }

            app.dispatch(Action::NvimRpcConnected)
        }
        Action::SetEditorContent(ref content) => {
            // Set editor content via RPC
            let rpc_slot = Arc::clone(nvim_rpc_slot);
            let content = content.clone();
            tokio::spawn(async move {
                let slot = rpc_slot.lock().await;
                if let Some(ref conn) = *slot
                    && let Err(e) = conn.rpc.set_buffer_lines(&content).await
                {
                    warn!("SetEditorContent via RPC failed: {}", e);
                }
            });
            Ok(Some(Action::Render))
        }
        Action::PushCatalogToNvim(ref json) => {
            let rpc_slot = Arc::clone(nvim_rpc_slot);
            let json = json.clone();
            tokio::spawn(async move {
                let slot = rpc_slot.lock().await;
                if let Some(ref conn) = *slot {
                    if let Err(e) = conn.rpc.push_catalog(&json).await {
                        warn!("PushCatalogToNvim failed: {}", e);
                    } else {
                        info!("Catalog pushed to nvim via RPC");
                    }
                }
            });
            Ok(None)
        }
        Action::NvimExited(_) => {
            // Clear the RPC slot so the periodic mode sync task terminates
            // on its next iteration (it breaks when the slot is None).
            // Use try_lock to avoid blocking the event loop; if the lock is held
            // by a short-lived async task, the mode sync task will see the editor's
            // exited state and stop on its own.
            if let Ok(mut slot) = nvim_rpc_slot.try_lock() {
                *slot = None;
            } else {
                // Slot is held by an async task; spawn a background clear.
                let rpc_slot = Arc::clone(nvim_rpc_slot);
                tokio::spawn(async move {
                    *rpc_slot.lock().await = None;
                });
            }
            // Reset editor state so nvim auto-restarts on next render, then start a
            // new query. This handles :q! and unexpected crashes gracefully — :q and
            // :wq are intercepted via cabbrev in init.lua so nvim never actually exits.
            app.dispatch(action)?;
            Ok(Some(Action::NewQuery))
        }
        Action::SaveTheme(ref theme_config) => {
            let theme_path = config.app_dir.join("theme.yaml");
            match theme::save(theme_config, &theme_path) {
                Ok(()) => {
                    if let Err(e) = theme::apply(theme_config) {
                        return Ok(Some(Action::StatusMessage(format!(
                            "Theme apply failed: {e}"
                        ))));
                    }
                    Ok(Some(Action::ThemeSaved))
                }
                Err(e) => Ok(Some(Action::StatusMessage(format!(
                    "Failed to write theme.yaml: {e}"
                )))),
            }
        }
        Action::ResetTheme => {
            let theme_path = config.app_dir.join("theme.yaml");
            let defaults = theme::ThemeConfig::default();
            let _ = theme::save(&defaults, &theme_path);
            let _ = theme::apply(&defaults);
            Ok(Some(Action::ThemeSaved))
        }
        // All other actions go through normal dispatch
        other => app.dispatch(other),
    }
}

fn save_history_entry(cache: &CacheManager, entry: QueryHistoryEntry) {
    let mut history = cache.load_query_history();
    history.insert(0, entry);
    // Keep last 1000 entries
    history.truncate(1000);
    if let Err(e) = cache.save_query_history(&history) {
        error!("Failed to save query history: {}", e);
    }
}
