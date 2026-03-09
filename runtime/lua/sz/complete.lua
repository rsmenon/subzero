-- sz SQL completion source
-- Context-aware omnifunc powered by catalog data with fuzzy subsequence matching

local catalog = require("sz.catalog")
local Fuzzy = require("sz.fuzzy")

local M = {}

-- SQL keywords for top-level completion
local SQL_KEYWORDS = {
  "SELECT", "FROM", "WHERE", "JOIN", "LEFT", "RIGHT", "INNER", "OUTER",
  "FULL", "CROSS", "ON", "AND", "OR", "NOT", "IN", "EXISTS", "BETWEEN",
  "LIKE", "ILIKE", "IS", "NULL", "AS", "ORDER", "BY", "GROUP", "HAVING",
  "LIMIT", "OFFSET", "UNION", "ALL", "INTERSECT", "EXCEPT", "INSERT",
  "INTO", "VALUES", "UPDATE", "SET", "DELETE", "CREATE", "TABLE", "VIEW",
  "ALTER", "DROP", "INDEX", "WITH", "RECURSIVE", "CASE", "WHEN", "THEN",
  "ELSE", "END", "DISTINCT", "ASC", "DESC", "NULLS", "FIRST", "LAST",
  "OVER", "PARTITION", "WINDOW", "ROWS", "RANGE", "UNBOUNDED", "PRECEDING",
  "FOLLOWING", "CURRENT", "ROW", "QUALIFY", "PIVOT", "UNPIVOT", "LATERAL",
  "FLATTEN", "SAMPLE", "TABLESAMPLE", "MERGE", "USING", "MATCHED",
  "GRANT", "REVOKE", "SHOW", "DESCRIBE", "EXPLAIN", "COPY", "PUT", "GET",
  "STAGE", "FILE", "FORMAT", "WAREHOUSE", "DATABASE", "SCHEMA", "ROLE",
  "TRUE", "FALSE",
}

-- Cached fuzzy matchers for large-list contexts
local fuzzy_table_context = nil   -- "table" context: tables + qualified + dbs + schemas
local fuzzy_column_context = nil  -- "column" context: all columns + unqualified tables
local fuzzy_keyword_context = nil -- "keyword" context: keywords + schemas + tables

-- Cached lists for scoped (small) contexts
local cached_schemas_in_db = {}      -- db_name -> list of schema items
local cached_tables_in_schema = {}   -- "DB.SCHEMA" -> list of table items
local cached_columns_for_table = {}  -- table_name -> list of column items

-- Generation tracking to detect stale caches
local last_generation = -1

-- Keyword sets for context detection (used by get_context)
local tbl_keywords = {
  FROM = true, JOIN = true, TABLE = true, INTO = true,
  UPDATE = true, DELETE = true,
}
local col_keywords = {
  SELECT = true, WHERE = true, ON = true, SET = true,
  BY = true, HAVING = true, WHEN = true, THEN = true,
  AND = true, OR = true,
}

--- Filter a small list using fuzzy subsequence matching, sorted by score descending
local function filter_small(candidates, base)
  if not base or base == "" then
    return candidates
  end
  local upper_base = base:upper()
  local matches = {}
  for _, item in ipairs(candidates) do
    local score = Fuzzy.subseq_score(upper_base, item.word:upper())
    if score then
      table.insert(matches, { item = item, score = score })
    end
  end
  table.sort(matches, function(a, b)
    if a.score ~= b.score then return a.score > b.score end
    return #a.item.word < #b.item.word
  end)
  local result = {}
  for _, m in ipairs(matches) do
    table.insert(result, m.item)
  end
  return result
end

--- Rebuild all fuzzy matchers and cached lists from current catalog data
local function rebuild_caches()
  last_generation = catalog.generation

  -- Reset all caches
  cached_schemas_in_db = {}
  cached_tables_in_schema = {}
  cached_columns_for_table = {}

  -- Build component item lists and scoped caches
  local unqualified_table_items = {}
  local qualified_table_items = {}
  local db_items = {}
  local schema_items = {}
  local all_column_items = {}

  -- Database items
  for _, db in ipairs(catalog.databases) do
    local item = { word = db, kind = "database", menu = "DATABASE" }
    table.insert(db_items, item)
  end

  -- Schema items + per-db cache
  for db, schema_list in pairs(catalog.schemas) do
    local db_schemas = {}
    for _, schema in ipairs(schema_list) do
      local qual = db .. "." .. schema
      local schema_item = { word = qual, kind = "schema", menu = "SCHEMA" }
      table.insert(schema_items, schema_item)
      table.insert(db_schemas, { word = schema, kind = "schema", menu = "SCHEMA" })
    end
    cached_schemas_in_db[db] = db_schemas
  end

  -- Table items + per-schema cache
  local seen_tables = {}
  for qual_key, entries in pairs(catalog.tables) do
    local schema_tables = {}
    for _, entry in ipairs(entries) do
      -- Qualified table item
      local qual_item = {
        word = qual_key .. "." .. entry.name,
        kind = entry.kind or "TABLE",
        menu = entry.kind or "",
      }
      table.insert(qualified_table_items, qual_item)

      -- Per-schema cache
      local tbl_item = {
        word = entry.name,
        kind = entry.kind or "TABLE",
        menu = entry.kind or "",
      }
      table.insert(schema_tables, tbl_item)

      -- Unqualified (deduplicated)
      if not seen_tables[entry.name] then
        seen_tables[entry.name] = true
        table.insert(unqualified_table_items, tbl_item)
      end
    end
    cached_tables_in_schema[qual_key] = schema_tables
  end

  -- Column items + per-table cache
  for qualified_name, cols in pairs(catalog.columns) do
    local table_name = qualified_name:match("[^.]+$")
    local col_items = {}
    for _, col in ipairs(cols) do
      local item = { word = col.name, kind = col.type or "", menu = col.type or "" }
      table.insert(col_items, item)
      table.insert(all_column_items, item)
    end
    if table_name then
      cached_columns_for_table[table_name] = col_items
      cached_columns_for_table[qualified_name] = col_items
    end
  end

  -- Build keyword items
  local kw_items = {}
  for _, kw in ipairs(SQL_KEYWORDS) do
    table.insert(kw_items, { word = kw, kind = "keyword", menu = "" })
  end

  -- Build fuzzy matchers for the three main contexts (resets incremental sessions)

  -- "table" context: unqualified tables + qualified tables + databases + schemas
  fuzzy_table_context = Fuzzy.new()
  for _, item in ipairs(unqualified_table_items) do
    fuzzy_table_context:add(item)
  end
  for _, item in ipairs(qualified_table_items) do
    fuzzy_table_context:add(item)
  end
  for _, item in ipairs(db_items) do
    fuzzy_table_context:add(item)
  end
  for _, item in ipairs(schema_items) do
    fuzzy_table_context:add(item)
  end

  -- "column" context: all columns + unqualified tables
  fuzzy_column_context = Fuzzy.new()
  for _, item in ipairs(all_column_items) do
    fuzzy_column_context:add(item)
  end
  for _, item in ipairs(unqualified_table_items) do
    fuzzy_column_context:add(item)
  end

  -- "keyword" context: keywords + schemas + unqualified tables
  fuzzy_keyword_context = Fuzzy.new()
  for _, item in ipairs(kw_items) do
    fuzzy_keyword_context:add(item)
  end
  for _, item in ipairs(schema_items) do
    fuzzy_keyword_context:add(item)
  end
  for _, item in ipairs(unqualified_table_items) do
    fuzzy_keyword_context:add(item)
  end
end

--- Ensure caches are fresh (rebuild if catalog generation has changed)
local function ensure_caches()
  if last_generation ~= catalog.generation then
    rebuild_caches()
  end
end

-- Eagerly rebuild caches when catalog reloads (e.g. from file watcher)
catalog.on_reload(rebuild_caches)

--- Return schemas in a specific database (small list)
local function schemas_in_db(db_name)
  return cached_schemas_in_db[db_name] or {}
end

--- Return tables in a specific schema (small list)
local function tables_in_schema(db_schema)
  return cached_tables_in_schema[db_schema] or {}
end

--- Return columns for a specific table (small list)
local function column_items_for_table(table_name)
  return cached_columns_for_table[table_name]
    or cached_columns_for_table[table_name:upper()]
    or {}
end

--- Determine the completion context from the current line
--- Returns context, qualifier, resolved
---   context: "dot_db" | "dot_schema" | "dot_table" | "dot_column" | "table" | "column" | "keyword"
---   qualifier: the catalog key for dot contexts (uppercase)
---   resolved: true if qualifier was found in the catalog (scoped lookup will work)
---
--- Dot contexts trigger for both trailing dot (DB.|) and mid-word (DB.partial|).
--- When resolved=false, the caller should fall back to broad fuzzy matching.
local function get_context(line, col)
  local before = line:sub(1, col)

  -- Check for dot-qualified identifiers (both trailing dot and mid-word)
  local full_ident = before:match("([%w_.]+)$")
  if full_ident and full_ident:find(".", 1, true) then
    -- Split on last dot: qualifier = everything before, partial = after
    local qualifier = full_ident:match("(.+)%.")
    if qualifier then
      local upper_q = qualifier:upper()
      local _, dots = qualifier:gsub("%.", ".")

      if dots == 0 then
        -- Single segment: DB.| or TABLE.|
        if catalog.schemas[upper_q] then
          return "dot_db", upper_q, true
        end
        -- Could be a table name for column completion
        if cached_columns_for_table[upper_q] or cached_columns_for_table[qualifier] then
          return "dot_column", upper_q, true
        end
        return "dot_column", upper_q, false
      elseif dots == 1 then
        -- Two segments: DB.SCHEMA.| or unknown
        if catalog.tables[upper_q] then
          return "dot_schema", upper_q, true
        end
        return "dot_schema", upper_q, false
      else
        -- Three+ segments: DB.SCHEMA.TABLE.|
        local resolved = cached_columns_for_table[upper_q] ~= nil
        return "dot_table", upper_q, resolved
      end
    end
  end

  local preceding = before:upper():match("(%w+)%s+[%w_.]*$")

  if preceding then
    if tbl_keywords[preceding] then
      return "table", nil
    end
    if col_keywords[preceding] then
      return "column", nil
    end
  end

  -- Fallback: scan backward through the line for the last SQL keyword.
  -- Handles cases like "ORDER BY col1, col2, " where the immediately
  -- preceding word is an identifier, not a keyword.
  local upper_before = before:upper()
  local best_pos = 0
  local best_ctx = nil
  for kw, _ in pairs(tbl_keywords) do
    -- Find all occurrences, keep the rightmost
    local p = 1
    while true do
      local s, e = upper_before:find("%f[%w]" .. kw .. "%f[%W]", p)
      if not s then break end
      if s > best_pos then
        best_pos = s
        best_ctx = "table"
      end
      p = e + 1
    end
  end
  for kw, _ in pairs(col_keywords) do
    local p = 1
    while true do
      local s, e = upper_before:find("%f[%w]" .. kw .. "%f[%W]", p)
      if not s then break end
      if s > best_pos then
        best_pos = s
        best_ctx = "column"
      end
      p = e + 1
    end
  end

  if best_ctx then
    return best_ctx, nil
  end

  return "keyword", nil
end

--- Omnifunc implementation (called by nvim)
--- findstart=1: return column where completion starts
--- findstart=0: return list of matches for `base`
function M.omnifunc(findstart, base)
  if findstart == 1 then
    local line = vim.api.nvim_get_current_line()
    local col = vim.api.nvim_win_get_cursor(0)[2]
    local start = col
    while start > 0 do
      local c = line:sub(start, start)
      if c:match("[%w_.]") then
        start = start - 1
      else
        break
      end
    end

    -- For resolved dot contexts (qualifier found in catalog), return position
    -- after the last dot so completions append after the qualifier.
    -- For unresolved dots (e.g. "snow.tpc.cat"), keep the full word start
    -- so base contains the complete identifier for fuzzy matching.
    local word = line:sub(start + 1, col)
    local last_dot = word:find("%.[^.]*$")
    if last_dot then
      local _, _, resolved = get_context(line, col)
      if resolved then
        return start + last_dot  -- position after the last dot
      end
    end

    return start
  end

  -- findstart == 0: return completions
  ensure_caches()

  local line = vim.api.nvim_get_current_line()
  local col = vim.api.nvim_win_get_cursor(0)[2]
  local context, qualifier, resolved = get_context(line, col)

  -- Resolved dot context: qualifier is in the catalog, base is just the suffix
  -- after the last dot (findstart positioned after the dot)
  if resolved and qualifier then
    if context == "dot_db" then
      return filter_small(schemas_in_db(qualifier), base)
    elseif context == "dot_schema" then
      return filter_small(tables_in_schema(qualifier), base)
    elseif context == "dot_table" or context == "dot_column" then
      return filter_small(column_items_for_table(qualifier), base)
    end
  end

  -- Unresolved dot context or non-dot context: base contains the full word
  -- (findstart positioned at start of full identifier)

  -- Minimum input length for broad contexts (3 chars without dots)
  if base and #base < 3 and (not base:find(".", 1, true)) then
    return {}
  end

  -- Broad context fuzzy search
  if context == "table" then
    return fuzzy_table_context:search(base)
  elseif context == "column" then
    return fuzzy_column_context:search(base)
  end

  -- Dot fallback: if base contains dots, treat as table reference for fuzzy
  if base and base:find(".", 1, true) then
    return fuzzy_table_context:search(base)
  end

  return fuzzy_keyword_context:search(base)
end

return M
