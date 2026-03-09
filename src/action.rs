use std::sync::Arc;

use crate::app::Mode;
use crate::cache::store::QueryHistoryEntry;
use crate::snowflake::parser::{Column, Database, QueryResult, Schema, Table};

#[derive(Debug, Clone)]
pub enum Action {
    SwitchMode(Mode),
    Quit,
    ShowQuitDialog,
    HideQuitDialog,
    HeartbeatResult(bool),
    CatalogLoaded(
        Arc<Vec<Database>>,
        Arc<Vec<Schema>>,
        Arc<Vec<Table>>,
        Option<chrono::DateTime<chrono::Utc>>,
    ),
    ColumnsLoaded {
        db: String,
        schema: String,
        table: String,
        columns: Vec<Column>,
    },
    PreviewLoaded {
        db: String,
        schema: String,
        table: String,
        result: QueryResult,
    },
    ExecuteQuery(String),
    QueryResultData(Arc<QueryResult>),
    QueryError(String),
    ShowQueryHistory,
    LoadQueryFromHistory(String),
    ExportResults,
    StatusMessage(String),
    ClearStatus,
    SetQueryHistory(Vec<QueryHistoryEntry>),
    RefreshCatalog,
    LoadColumns {
        db: String,
        schema: String,
        table: String,
    },
    RefreshColumns {
        db: String,
        schema: String,
        table: String,
    },
    LoadPreview {
        db: String,
        schema: String,
        table: String,
    },
    // Lazy loading: databases loaded first, then objects (tables/views) in background
    DatabasesLoaded(Vec<Database>),
    ObjectsLoaded(Vec<Schema>, Vec<Table>),
    CachedColumnsRefreshed,
    CatalogError(String),
    ColumnsError {
        db: String,
        schema: String,
        table: String,
        error: String,
    },
    PreviewError {
        db: String,
        schema: String,
        table: String,
        error: String,
    },
    ApplyOverrides {
        connection_name: Option<String>,
    },
    ClearCache,
    CacheClearDone,
    // Background tasks
    CreateTask {
        source: String,
        name: String,
    },
    CompleteTask(usize),
    ToggleBackgroundTasks,
    // Saved queries
    SaveQueryToHistory,
    ToggleSavedQueries,
    LoadSavedQuery {
        id: usize,
        sql: String,
    },
    PersistSavedQueries,
    // Per-database catalog refresh
    DatabaseCatalogLoaded {
        db: String,
        schemas: Arc<Vec<Schema>>,
        tables: Arc<Vec<Table>>,
    },
    // Neovim embed lifecycle
    NvimReady,
    NvimExited(String),
    NvimRpcConnected,
    SetEditorContent(String),
    PushCatalogToNvim(String),

    NewQuery,
    Render,
    None,

    // Theme
    SaveTheme(Box<crate::theme::ThemeConfig>),
    ResetTheme,
    ThemeSaved,
}
