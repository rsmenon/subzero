-- sz nvim plugin entry point
-- Loads catalog data and sets up SQL completions

local catalog = require("sz.catalog")
local complete = require("sz.complete")

local M = {}

function M.setup()
  -- Load catalog from the JSON file written by sz
  catalog.load()

  -- Set omnifunc now and re-set it after any FileType event
  -- (prevents the built-in sql ftplugin from overriding it)
  vim.bo.omnifunc = "v:lua.sz_complete"
  local sz_ft_augroup = vim.api.nvim_create_augroup("sz_filetype", { clear = true })
  vim.api.nvim_create_autocmd("FileType", {
    group = sz_ft_augroup,
    pattern = "sql",
    callback = function()
      vim.bo.omnifunc = "v:lua.sz_complete"
    end,
  })

  -- Watch the catalog file for changes (instant reload on refresh)
  catalog.watch()
end

-- Tracks whether an omnifunc completion session is active.
-- Set to true when the user triggers Ctrl+X Ctrl+O; cleared on CompleteDone.
M.omnifunc_active = false

-- Global omnifunc entry point (called by nvim on Ctrl+X Ctrl+O)
function _G.sz_complete(findstart, base)
  if findstart == 1 then
    M.omnifunc_active = true
  end
  return complete.omnifunc(findstart, base)
end

return M
