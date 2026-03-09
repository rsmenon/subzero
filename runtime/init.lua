-- sz bundled neovim config
-- Used when launching the embedded nvim editor with --clean -u
vim.opt.number = true
vim.opt.relativenumber = false
vim.opt.signcolumn = "no"
vim.opt.showmode = true
vim.opt.ruler = false
vim.opt.laststatus = 0
vim.opt.cmdheight = 1
vim.opt.shortmess:append("I")
-- Disable the built-in SQL ftplugin's omnifunc (requires dbext)
vim.g.omni_sql_no_default_maps = 1
vim.g.loaded_sql_completion = 1

vim.opt.filetype = "sql"
vim.cmd("syntax on")

-- SQL-friendly editing defaults
vim.opt.tabstop = 2
vim.opt.shiftwidth = 2
vim.opt.expandtab = true
vim.opt.wrap = true

-- Disable mouse so host terminal handles it
vim.opt.mouse = ""

-- Minimal visuals
vim.opt.termguicolors = true
vim.opt.cursorline = true
vim.opt.background = "dark"

-- Gruvbox dark inline colorscheme (no plugin needed)
local hi = function(group, opts) vim.api.nvim_set_hl(0, group, opts) end
-- Base
hi("Normal",       { fg = "#ebdbb2", bg = "#282828" })
hi("NormalFloat",  { fg = "#ebdbb2", bg = "#3c3836" })
hi("FloatBorder",  { fg = "#458588", bg = "#3c3836" })
hi("CursorLine",   { bg = "#3c3836" })
hi("CursorLineNr", { fg = "#fabd2f", bg = "#3c3836", bold = true })
hi("LineNr",       { fg = "#504945" })
hi("Visual",       { bg = "#504945" })
hi("Search",       { fg = "#282828", bg = "#fabd2f" })
hi("IncSearch",    { fg = "#282828", bg = "#fe8019" })
hi("StatusLine",   { fg = "#ebdbb2", bg = "#504945" })
hi("StatusLineNC", { fg = "#928374", bg = "#3c3836" })
hi("VertSplit",    { fg = "#504945" })
hi("Pmenu",        { fg = "#ebdbb2", bg = "#3c3836" })
hi("PmenuSel",     { fg = "#282828", bg = "#83a598", bold = true })
hi("PmenuSbar",    { bg = "#504945" })
hi("PmenuThumb",   { bg = "#928374" })
hi("MatchParen",   { fg = "#fabd2f", bold = true, underline = true })
-- Syntax
hi("Comment",      { fg = "#928374", italic = true })
hi("Constant",     { fg = "#d3869b" })
hi("String",       { fg = "#b8bb26" })
hi("Number",       { fg = "#d3869b" })
hi("Identifier",   { fg = "#83a598" })
hi("Function",     { fg = "#8ec07c", bold = true })
hi("Statement",    { fg = "#fe8019" })
hi("Keyword",      { fg = "#fe8019" })
hi("Operator",     { fg = "#ebdbb2" })
hi("Type",         { fg = "#fabd2f" })
hi("Special",      { fg = "#fe8019" })
hi("PreProc",      { fg = "#8ec07c" })
hi("Error",        { fg = "#fb4934", bold = true })
hi("Todo",         { fg = "#282828", bg = "#fabd2f", bold = true })
-- SQL-specific (mapped to general groups)
hi("sqlKeyword",   { fg = "#fe8019", bold = true })
hi("sqlFunction",  { fg = "#8ec07c", bold = true })
hi("sqlString",    { fg = "#b8bb26" })
hi("sqlNumber",    { fg = "#d3869b" })
hi("sqlOperator",  { fg = "#ebdbb2" })
hi("sqlComment",   { fg = "#928374", italic = true })
hi("sqlType",      { fg = "#fabd2f" })
hi("sqlStatement", { fg = "#fe8019", bold = true })

-- Completion settings: show menu, don't auto-select, don't auto-insert
vim.opt.completeopt = "menu,menuone,noselect,noinsert"

-- Add the bundled plugin to the runtime path
-- The init.lua lives at runtime/init.lua; the plugin is at runtime/lua/sz/
local init_dir = vim.fn.fnamemodify(debug.getinfo(1, "S").source:sub(2), ":h")
vim.opt.runtimepath:prepend(init_dir)

-- Intercept vim file/quit commands to integrate with sz.
-- vim.g.sz_rpc_channel is set by sz after RPC connects (nvim_rpc.rs setup_integration).
local function sz_notify(cmd)
  local ch = vim.g.sz_rpc_channel
  if not ch then return end
  vim.rpcnotify(ch, "sz_cmd", cmd)
end

vim.api.nvim_create_user_command("SzNewQuery",    function() sz_notify("new_query")   end, { bang = true })
vim.api.nvim_create_user_command("SzSave",        function() sz_notify("save_query");  vim.bo.modified = false end, {})
vim.api.nvim_create_user_command("SzSaveAndNew",  function() sz_notify("save_and_new"); vim.bo.modified = false end, {})

-- cabbrev expands the full typed word before <CR>, so these don't conflict with each other:
-- :w<CR>  → "w"  matches, :wq<CR> → "wq" matches (not "w")
vim.cmd([[cnoreabbrev q  SzNewQuery]])
vim.cmd([[cnoreabbrev wq SzSaveAndNew]])
vim.cmd([[cnoreabbrev w  SzSave]])

-- Load the sz completion plugin if catalog path is available
if vim.env.SZ_CATALOG_PATH and vim.env.SZ_CATALOG_PATH ~= "" then
  local ok, sz = pcall(require, "sz")
  if ok then
    sz.setup()

    -- Keep the omnifunc popup alive while typing.
    -- After the user triggers Ctrl+X Ctrl+O, each keystroke re-queries the
    -- omnifunc and refreshes the popup via vim.fn.complete(). The session
    -- ends when the user selects a match or dismisses with Esc/Ctrl-C.
    local ok2, complete_mod = pcall(require, "sz.complete")
    if not ok2 then
      vim.notify("sz: failed to load completion module: " .. tostring(complete_mod), vim.log.levels.WARN)
      complete_mod = nil
    end

    if complete_mod then
      local sz_augroup = vim.api.nvim_create_augroup("sz_completion", { clear = true })

      -- Re-trigger completion on every keystroke while the session is active.
      -- Also closes the session when the cursor leaves the word (e.g. user
      -- deleted the entire word) or when there are no matching items.
      vim.api.nvim_create_autocmd("TextChangedI", {
        group = sz_augroup,
        pattern = "*",
        callback = function()
          if not sz.omnifunc_active then return end
          local col = vim.api.nvim_win_get_cursor(0)[2]
          local line = vim.api.nvim_get_current_line()
          -- End session if cursor is no longer inside a word
          if col == 0 or not line:sub(1, col):match("[%w_.]$") then
            sz.omnifunc_active = false
            return
          end
          local start_col = complete_mod.omnifunc(1, "")
          if start_col < 0 then
            sz.omnifunc_active = false
            return
          end
          local base = line:sub(start_col + 1, col)
          local items = complete_mod.omnifunc(0, base)
          if items and #items > 0 then
            vim.fn.complete(start_col + 1, items)
          else
            -- No matches — close the popup but the session stays alive in case
            -- the user keeps typing and matches reappear
            sz.omnifunc_active = false
          end
        end,
      })

      -- End session when the user actually selects a completion item.
      -- Do NOT end the session when v:completed_item is empty — that happens
      -- when backspace or a failed match closes the popup, and we want
      -- TextChangedI to re-open it with updated results.
      vim.api.nvim_create_autocmd("CompleteDone", {
        group = sz_augroup,
        pattern = "*",
        callback = function()
          local item = vim.v.completed_item
          if item and item.word and item.word ~= "" then
            sz.omnifunc_active = false
          end
        end,
      })

      -- End session when leaving insert mode (Esc, Ctrl-C, etc.)
      vim.api.nvim_create_autocmd("InsertLeave", {
        group = sz_augroup,
        pattern = "*",
        callback = function()
          sz.omnifunc_active = false
        end,
      })

      -- Ctrl-E explicitly cancels the popup without leaving insert mode.
      -- Without this mapping, omnifunc_active would stay true and
      -- TextChangedI would re-open the popup on the next keystroke.
      vim.keymap.set("i", "<C-e>", function()
        if vim.fn.pumvisible() == 1 then
          sz.omnifunc_active = false
        end
        return "<C-e>"
      end, { buffer = true, expr = true, silent = true })
    end
  end
end
