-- sz catalog reader
-- Reads and watches the catalog JSON file written by sz

local M = {}

-- Generation counter: incremented on every load() call
-- Consumers compare against their own snapshot to detect staleness
M.generation = 0

-- Catalog data (populated from JSON)
M.databases = {}
M.schemas = {}   -- db -> list of schema names
M.tables = {}    -- "DB.SCHEMA" -> list of {name, kind}
M.columns = {}   -- "DB.SCHEMA.TABLE" -> list of {name, type, nullable}

-- Flat lookup tables (built from catalog data)
M.all_table_names = {}   -- list of all table/view names (unqualified)
M.all_column_names = {}  -- list of all column names (unqualified)
M.table_to_columns = {}  -- unqualified table name -> list of column entries

local catalog_path = nil
local fs_event = nil
local on_reload_callbacks = {}

-- Expose callbacks list for RPC push (used by nvim_exec_lua)
M._on_reload_callbacks = on_reload_callbacks

--- Get the catalog JSON path from the SZ_CATALOG_PATH env var
local function get_catalog_path()
  if catalog_path then
    return catalog_path
  end
  catalog_path = vim.env.SZ_CATALOG_PATH
  return catalog_path
end

--- Rebuild flat lookup tables from the current catalog data.
--- Called after load() or after RPC push sets M.databases/schemas/tables/columns.
function M._rebuild_lookups()
  M.all_table_names = {}
  M.all_column_names = {}
  M.table_to_columns = {}

  local seen_tables = {}
  local seen_columns = {}

  for _, entries in pairs(M.tables) do
    for _, entry in ipairs(entries) do
      if not seen_tables[entry.name] then
        seen_tables[entry.name] = true
        table.insert(M.all_table_names, entry)
      end
    end
  end

  for qualified_name, cols in pairs(M.columns) do
    -- Extract unqualified table name from "DB.SCHEMA.TABLE"
    local table_name = qualified_name:match("[^.]+$")
    if table_name then
      M.table_to_columns[table_name] = cols
      M.table_to_columns[qualified_name] = cols
    end

    for _, col in ipairs(cols) do
      if not seen_columns[col.name] then
        seen_columns[col.name] = true
        table.insert(M.all_column_names, col)
      end
    end
  end

  table.sort(M.all_table_names, function(a, b) return a.name < b.name end)
  table.sort(M.all_column_names, function(a, b) return a.name < b.name end)
end

--- Parse the catalog JSON file and populate lookup tables
function M.load()
  local path = get_catalog_path()
  if not path or path == "" then
    return
  end

  local f = io.open(path, "r")
  if not f then
    return
  end

  local content = f:read("*a")
  f:close()

  if not content or content == "" then
    return
  end

  local ok, data = pcall(vim.json.decode, content)
  if not ok or type(data) ~= "table" then
    return
  end

  -- Store raw catalog data
  M.databases = data.databases or {}
  M.schemas = data.schemas or {}
  M.tables = data.tables or {}
  M.columns = data.columns or {}

  -- Build flat lookup tables
  M._rebuild_lookups()

  -- Bump generation so consumers know to rebuild their caches
  M.generation = M.generation + 1

  -- Notify subscribers
  for _, cb in ipairs(on_reload_callbacks) do
    cb()
  end
end

--- Register a callback to run after every catalog load/reload
function M.on_reload(cb)
  table.insert(on_reload_callbacks, cb)
end

--- Watch the catalog JSON file for changes using libuv fs_event.
--- Watches the parent directory instead of the file directly so that the
--- watcher fires even when the file doesn't exist yet (fresh install).
function M.watch()
  local path = get_catalog_path()
  if not path or path == "" then
    return
  end

  -- Close existing watcher if any
  if fs_event then
    fs_event:stop()
    fs_event = nil
  end

  fs_event = vim.uv.new_fs_event()
  if not fs_event then
    return
  end

  -- Watch the parent directory and filter for the target filename.
  -- This ensures the watcher fires when the file is first created.
  local dir = path:match("(.*/)")
  local filename = path:match(".*/(.+)$")
  if not dir or not filename then
    -- Fallback: try watching the file directly
    fs_event:start(path, {}, vim.schedule_wrap(function(err, _, _)
      if err then return end
      M.load()
    end))
    return
  end

  fs_event:start(dir, {}, vim.schedule_wrap(function(err, fname, _)
    if err or fname ~= filename then return end
    -- Reload catalog data
    M.load()
  end))
end

--- Stop watching the catalog file
function M.stop()
  if fs_event then
    fs_event:stop()
    fs_event = nil
  end
end

return M
