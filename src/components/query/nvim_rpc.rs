#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_possible_wrap)]

use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::time::timeout;

use async_trait::async_trait;
use color_eyre::Result;
use color_eyre::eyre::eyre;
use nvim_rs::Neovim;
use rmpv::Value;
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{info, warn};

use crate::action::Action;
use crate::event::AppEvent;

type Writer = Compat<tokio::io::WriteHalf<UnixStream>>;

/// Handler for nvim RPC notifications (`buf_attach` callbacks and sz commands).
#[derive(Clone)]
pub struct NvimHandler {
    /// Sender for buffer content updates from `buf_attach`.
    buf_tx: tokio::sync::watch::Sender<String>,
    /// Sender for app actions triggered from within nvim (e.g. :q, :w).
    action_tx: mpsc::UnboundedSender<AppEvent>,
}

#[async_trait]
impl nvim_rs::Handler for NvimHandler {
    type Writer = Writer;

    async fn handle_notify(&self, name: String, args: Vec<Value>, _neovim: Neovim<Self::Writer>) {
        match name.as_str() {
            "nvim_buf_lines_event" => {
                self.handle_buf_lines_event(&args);
            }
            "nvim_buf_detach_event" => {
                self.buf_tx.send_modify(std::string::String::clear);
            }
            // sz_cmd notifications sent from init.lua command intercepts
            "sz_cmd" => {
                if let Some(cmd) = args.first().and_then(|v| v.as_str()) {
                    match cmd {
                        "new_query" => {
                            let _ = self.action_tx.send(AppEvent::Action(Action::NewQuery));
                        }
                        "save_query" => {
                            let _ = self
                                .action_tx
                                .send(AppEvent::Action(Action::SaveQueryToHistory));
                        }
                        "save_and_new" => {
                            // Save first, then new — order preserved via the unbounded channel
                            let _ = self
                                .action_tx
                                .send(AppEvent::Action(Action::SaveQueryToHistory));
                            let _ = self.action_tx.send(AppEvent::Action(Action::NewQuery));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

impl NvimHandler {
    fn handle_buf_lines_event(&self, args: &[Value]) {
        // args: [buf, changedtick, firstline, lastline, linedata, more]
        if args.len() < 5 {
            return;
        }
        let firstline = args[2].as_u64().unwrap_or(0) as usize;
        let lastline = args[3].as_i64().unwrap_or(-1);
        let new_lines: Vec<String> = args[4]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        self.buf_tx.send_modify(|current| {
            let mut lines: Vec<String> = if current.is_empty() {
                Vec::new()
            } else {
                current.lines().map(String::from).collect()
            };

            if lastline < 0 {
                // Initial full buffer load (send_buffer=true)
                lines = new_lines;
            } else {
                let lastline = lastline as usize;
                // Incremental update: replace lines[firstline..lastline] with new_lines
                let end = lastline.min(lines.len());
                lines.splice(firstline..end, new_lines);
            }

            *current = lines.join("\n");
        });
    }
}

/// Sync-accessible parts of the RPC state, readable from the main/render thread
/// without needing async. These are populated by the RPC connection and kept alive
/// independently of the async Neovim handle.
pub struct NvimRpcSync {
    /// Receiver for the live buffer mirror, updated by `buf_attach` notifications.
    buf_mirror: tokio::sync::watch::Receiver<String>,
    /// Cached nvim mode string (e.g. "n", "i", "v"), updated periodically.
    cached_mode: Arc<StdMutex<Option<String>>>,
}

impl NvimRpcSync {
    /// Get the current buffer contents from the live mirror.
    /// Returns None if `buf_attach` hasn't fired yet or mirror is empty.
    pub fn get_mirror(&self) -> Option<String> {
        let val = self.buf_mirror.borrow().clone();
        if val.is_empty() { None } else { Some(val) }
    }

    /// Get the cached mode string, if available.
    pub fn get_cached_mode(&self) -> Option<String> {
        self.cached_mode.lock().ok()?.clone()
    }
}

/// Async RPC client wrapping a persistent connection to the embedded nvim instance.
/// Lives inside an `Arc<tokio::sync::Mutex>` so async tasks can access it.
pub struct NvimRpc {
    nvim: Neovim<Writer>,
    /// Shared reference to the cached mode, same Arc as `NvimRpcSync`.
    cached_mode: Arc<StdMutex<Option<String>>>,
}

impl NvimRpc {
    /// Set buffer contents via RPC. Times out after 2 seconds.
    pub async fn set_buffer_lines(&self, content: &str) -> Result<()> {
        let rpc_timeout = Duration::from_secs(2);
        let buf = match timeout(rpc_timeout, self.nvim.get_current_buf()).await {
            Ok(Ok(buf)) => buf,
            Ok(Err(e)) => return Err(eyre!("{}", e)),
            Err(_) => return Err(eyre!("RPC get_current_buf timed out")),
        };
        let lines: Vec<String> = content.lines().map(std::string::ToString::to_string).collect();
        match timeout(rpc_timeout, buf.set_lines(0, -1, false, lines)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(eyre!("{}", e)),
            Err(_) => Err(eyre!("RPC set_lines timed out")),
        }
    }

    /// Get current nvim mode via RPC. Times out after 2 seconds.
    pub async fn get_mode(&self) -> Result<String> {
        let mode_info = match timeout(Duration::from_secs(2), self.nvim.get_mode()).await {
            Ok(Ok(info)) => info,
            Ok(Err(e)) => return Err(eyre!("{}", e)),
            Err(_) => return Err(eyre!("RPC get_mode timed out")),
        };
        // get_mode returns Vec<(Value, Value)>
        for (key, val) in &mode_info {
            if key.as_str() == Some("mode")
                && let Some(m) = val.as_str()
            {
                return Ok(m.to_string());
            }
        }
        Ok("n".to_string())
    }

    /// Update the cached mode via an async RPC call.
    /// Timeout is handled inside `get_mode()`.
    pub async fn sync_mode(&self) {
        match self.get_mode().await {
            Ok(mode) => {
                if let Ok(mut cached) = self.cached_mode.lock() {
                    *cached = Some(mode);
                }
            }
            Err(e) => {
                warn!("sync_mode failed: {e}");
                if let Ok(mut cached) = self.cached_mode.lock() {
                    *cached = None;
                }
            }
        }
    }

    /// Push catalog data directly into nvim's Lua runtime via RPC. Times out after 2 seconds.
    pub async fn push_catalog(&self, catalog_json: &str) -> Result<()> {
        let lua_code = r#"
            local json_str = ...
            local ok, data = pcall(vim.json.decode, json_str)
            if not ok or type(data) ~= "table" then
                return
            end
            local catalog = require("sz.catalog")
            catalog.databases = data.databases or {}
            catalog.schemas = data.schemas or {}
            catalog.tables = data.tables or {}
            catalog.columns = data.columns or {}
            catalog._rebuild_lookups()
            catalog.generation = catalog.generation + 1
            for _, cb in ipairs(catalog._on_reload_callbacks) do cb() end
        "#;
        match timeout(
            Duration::from_secs(2),
            self.nvim.exec_lua(lua_code, vec![Value::from(catalog_json)]),
        )
        .await
        {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(eyre!("push_catalog failed: {}", e)),
            Err(_) => Err(eyre!("push_catalog timed out")),
        }
    }

    /// Store the RPC channel ID in a nvim global so init.lua can use it for rpcnotify.
    /// Must be called after connection is established. Times out after 2 seconds.
    /// Returns an optional warning message (e.g. if channel_id resolved to 0).
    pub async fn setup_integration(&self) -> Result<Option<String>> {
        let rpc_timeout = Duration::from_secs(2);
        let api_info = match timeout(rpc_timeout, self.nvim.get_api_info()).await {
            Ok(Ok(info)) => info,
            Ok(Err(e)) => return Err(eyre!("{}", e)),
            Err(_) => return Err(eyre!("RPC get_api_info timed out")),
        };
        let channel_id: u64 = api_info.first().and_then(nvim_rs::Value::as_u64).unwrap_or(0);
        let warning = if channel_id == 0 {
            let msg = "nvim channel ID is 0 -- command intercepts (:w/:q) will not function";
            warn!("{msg}");
            Some(msg.to_string())
        } else {
            None
        };
        match timeout(
            rpc_timeout,
            self.nvim
                .exec_lua("vim.g.sz_rpc_channel = ...", vec![Value::from(channel_id)]),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(eyre!("setup_integration failed: {}", e)),
            Err(_) => return Err(eyre!("setup_integration exec_lua timed out")),
        }
        info!("sz RPC channel ID registered with nvim: {}", channel_id);
        Ok(warning)
    }

    /// Attach to the current buffer to receive live line change notifications.
    /// Times out after 2 seconds.
    async fn attach_buffer(&self) -> Result<()> {
        let rpc_timeout = Duration::from_secs(2);
        let buf = match timeout(rpc_timeout, self.nvim.get_current_buf()).await {
            Ok(Ok(buf)) => buf,
            Ok(Err(e)) => return Err(eyre!("{}", e)),
            Err(_) => return Err(eyre!("RPC get_current_buf timed out during attach")),
        };
        // send_buffer=true -> nvim sends full contents immediately as the first notification
        match timeout(rpc_timeout, buf.attach(true, vec![])).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(eyre!("buf_attach failed: {}", e)),
            Err(_) => return Err(eyre!("buf_attach timed out")),
        }
        info!("nvim buf_attach succeeded");
        Ok(())
    }
}

/// Connect to nvim's Unix domain socket with retry/backoff.
async fn connect_with_retry(socket_path: &Path, max_attempts: u32) -> Result<UnixStream> {
    for attempt in 0..max_attempts {
        match UnixStream::connect(socket_path).await {
            Ok(stream) => return Ok(stream),
            Err(e) if attempt < max_attempts - 1 => {
                let delay_ms = 50 * (u64::from(attempt) + 1);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                if attempt > 3 {
                    warn!(
                        "nvim RPC connect attempt {}/{} failed: {}",
                        attempt + 1,
                        max_attempts,
                        e
                    );
                }
            }
            Err(e) => {
                return Err(eyre!(
                    "Failed to connect to nvim socket after {} attempts: {}",
                    max_attempts,
                    e
                ));
            }
        }
    }
    Err(eyre!("Exhausted all connection attempts"))
}

/// Result of a successful RPC connection. Contains both the async handle
/// (for spawned tasks) and the sync accessors (for the main/render thread).
/// After connection, `take_sync_state()` moves the sync state to `NvimEditor`
/// while the async `rpc` handle remains for spawned tasks to use.
pub struct NvimRpcConnection {
    /// Async handle — lives here for async tasks to call via the shared slot
    pub rpc: NvimRpc,
    /// Sync accessors — taken once by `NvimEditor` after connection
    sync_state: Option<NvimRpcSync>,
    /// Warning message from setup (e.g. channel_id == 0)
    pub setup_warning: Option<String>,
}

impl NvimRpcConnection {
    /// Take the sync state out for storage on `NvimEditor`.
    /// Can only be called once; subsequent calls return None.
    pub fn take_sync_state(&mut self) -> Option<NvimRpcSync> {
        self.sync_state.take()
    }
}

/// Establish the RPC connection and attach to the buffer.
/// Returns both the async RPC handle and sync state accessors.
pub async fn connect(
    socket_path: &Path,
    action_tx: mpsc::UnboundedSender<AppEvent>,
) -> Result<NvimRpcConnection> {
    let stream = connect_with_retry(socket_path, 20).await?;

    let (reader, writer) = tokio::io::split(stream);
    let reader = reader.compat();
    let writer = writer.compat_write();

    let (buf_tx, buf_rx) = tokio::sync::watch::channel(String::new());
    let handler = NvimHandler { buf_tx, action_tx };

    let (nvim, io_task) = Neovim::<Writer>::new(reader, writer, handler);

    // Spawn the IO handler in the background — it processes RPC messages
    tokio::spawn(async move {
        if let Err(e) = io_task.await {
            // The IO task ending is expected when nvim exits
            info!("nvim RPC IO task ended: {:?}", e);
        }
    });

    let cached_mode = Arc::new(StdMutex::new(None));

    let rpc = NvimRpc {
        nvim,
        cached_mode: Arc::clone(&cached_mode),
    };

    // Attach to the buffer to start receiving live updates
    rpc.attach_buffer().await?;

    // Register our channel ID with nvim so init.lua command intercepts can call back
    let setup_warning = rpc.setup_integration().await?;

    // Initial mode sync
    rpc.sync_mode().await;

    let sync_state = NvimRpcSync {
        buf_mirror: buf_rx,
        cached_mode,
    };

    info!("nvim RPC connected and buf_attach active");
    Ok(NvimRpcConnection {
        rpc,
        sync_state: Some(sync_state),
        setup_warning,
    })
}
