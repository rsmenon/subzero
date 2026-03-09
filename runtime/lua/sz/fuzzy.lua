-- Scored fuzzy subsequence matcher for completion candidates
-- Supports dot-segmented matching (e.g. "snow.cds100.catsale" matches
-- "SNOWFLAKE_SAMPLE_DATA.TPCDS_SF100.CATALOG_SALES")
--
-- Uses incremental narrowing: extending a query searches only the previous
-- result set instead of the full catalog. Backspace walks back to a cached level.

local Fuzzy = {}
Fuzzy.__index = Fuzzy

local MAX_RESULTS = 50
local MAX_LEVEL_ENTRIES = 500

--- Greedy forward subsequence match with scoring.
--- Returns numeric score if query is a subsequence of target, nil otherwise.
--- Both query and target should be pre-uppercased.
---
--- Tries all positions where query[1] matches in target and returns the
--- maximum score across all starting positions. This avoids the greedy trap
--- where the first match of query[1] leads to a suboptimal alignment.
---
--- Scoring:
---   +1 per matched char
---   +3 if match is at position 1 of target (prefix bonus, mutually exclusive with boundary)
---   +2 if match is at a word boundary (after '_'), only when not at position 1
---   +2 if match is consecutive with the previous match

--- Attempt a greedy subsequence scan starting at start_ti for query[1].
--- Returns numeric score or nil if the subsequence cannot be completed.
local function attempt_from(query, target, start_ti, qlen, tlen)
  local qi = 1
  local prev_ti = 0
  local score = 0

  for ti = start_ti, tlen do
    if qi > qlen then break end
    -- Early exit: more query chars left than target chars remaining
    if (qlen - qi) > (tlen - ti) then return nil end
    if target:byte(ti) == query:byte(qi) then
      score = score + 1
      -- Prefix bonus and boundary bonus are mutually exclusive
      if ti == 1 then
        score = score + 3  -- prefix bonus
      elseif target:byte(ti - 1) == 95 then  -- 95 = '_'
        score = score + 2  -- boundary bonus
      end
      if ti == prev_ti + 1 then
        score = score + 2  -- consecutive bonus
      end
      prev_ti = ti
      qi = qi + 1
    end
  end

  if qi > qlen then return score end
  return nil
end

function Fuzzy.subseq_score(query, target)
  local best = nil
  local qlen = #query
  local tlen = #target
  local q1 = query:byte(1)

  -- Try each position where target matches query[1]
  for start = 1, tlen - qlen + 1 do
    if target:byte(start) == q1 then
      local score = attempt_from(query, target, start, qlen, tlen)
      if score and (not best or score > best) then
        best = score
      end
    end
  end

  return best
end

--- Score a single entry against a query (handles dot-segmented and plain)
--- Returns numeric score or nil
local function score_entry(upper_query, has_dots, query_segments, num_qseg, entry)
  if has_dots then
    local item_segments = {}
    for seg in entry.upper:gmatch("[^.]+") do
      item_segments[#item_segments + 1] = seg
    end
    local num_iseg = #item_segments
    if num_qseg > num_iseg then return nil end

    local total_score = 0
    for si = 1, num_qseg do
      local qseg = query_segments[si]
      if qseg ~= "" then
        local seg_score = Fuzzy.subseq_score(qseg, item_segments[si])
        if not seg_score then return nil end
        total_score = total_score + seg_score
      end
    end
    return total_score
  else
    return Fuzzy.subseq_score(upper_query, entry.upper)
  end
end

--- Prepare query info (parsed once, reused across items)
local function parse_query(query)
  local upper_query = query:upper()
  local has_dots = upper_query:find(".", 1, true) ~= nil
  local query_segments = nil
  local num_qseg = 0

  if has_dots then
    query_segments = {}
    for seg in upper_query:gmatch("[^.]+") do
      query_segments[#query_segments + 1] = seg
    end
    if upper_query:sub(-1) == "." then
      query_segments[#query_segments + 1] = ""
    end
    num_qseg = #query_segments
  end

  return upper_query, has_dots, query_segments, num_qseg
end

--- Scan a list of entries, return scored matches (sorted).
--- Returns all_entries (full match set for narrowing) and display_items (capped for UI).
local function scan_and_rank(entries, upper_query, has_dots, query_segments, num_qseg)
  local matches = {}
  for i = 1, #entries do
    local entry = entries[i]
    local s = score_entry(upper_query, has_dots, query_segments, num_qseg, entry)
    if s then
      -- Boost database matches when query has no dot qualifier
      if not has_dots and entry.item.kind == "database" then
        s = s + 5
      end
      matches[#matches + 1] = { entry = entry, score = s }
    end
  end

  table.sort(matches, function(a, b)
    if a.score ~= b.score then return a.score > b.score end
    return #a.entry.upper < #b.entry.upper
  end)

  -- Keep matches for incremental narrowing, capped at MAX_LEVEL_ENTRIES.
  -- When truncated, backspace must fall back to a full rescan.
  local total_matches = #matches
  local level_n = total_matches
  local truncated = false
  if level_n > MAX_LEVEL_ENTRIES then
    level_n = MAX_LEVEL_ENTRIES
    truncated = true
  end
  local all_entries = {}
  for i = 1, level_n do
    all_entries[i] = matches[i].entry
  end

  -- Cap what's returned to nvim for display
  local n = total_matches
  if n > MAX_RESULTS then n = MAX_RESULTS end
  local display_items = {}
  for i = 1, n do
    display_items[i] = matches[i].entry.item
  end

  return all_entries, display_items, truncated
end

--- Create a new empty fuzzy matcher
function Fuzzy.new()
  return setmetatable({
    items = {},          -- all entries: { item = completion_item, upper = "UPPERCASED" }
    sorted_items = nil,  -- cached sorted order for empty-query results
    session = nil,       -- incremental narrowing state
  }, Fuzzy)
end

--- Add a completion item to the matcher
--- item.word is the string used for matching
function Fuzzy:add(item)
  if not item.word or item.word == "" then
    return
  end
  self.items[#self.items + 1] = { item = item, upper = item.word:upper() }
  self.sorted_items = nil  -- invalidate cached sort order
end

--- Reset the incremental narrowing session (called on catalog rebuild)
function Fuzzy:reset_session()
  self.session = nil
end

--- Search for items matching query using fuzzy subsequence matching.
--- Uses incremental narrowing: extending a previous query searches only
--- the previous match set.
--- Returns matched items sorted by score descending, capped at MAX_RESULTS.
function Fuzzy:search(query)
  -- Empty query: return all items sorted alphabetically (capped)
  if not query or query == "" then
    self.session = nil
    -- Cache sorted order so we don't re-sort on every empty call
    if not self.sorted_items then
      local sorted = {}
      for i = 1, #self.items do
        sorted[i] = self.items[i]
      end
      table.sort(sorted, function(a, b) return a.upper < b.upper end)
      self.sorted_items = sorted
    end
    local n = #self.sorted_items
    if n > MAX_RESULTS then n = MAX_RESULTS end
    local result = {}
    for i = 1, n do
      result[i] = self.sorted_items[i].item
    end
    return result
  end

  local upper_query, has_dots, query_segments, num_qseg = parse_query(query)

  -- Determine source entries: full catalog or narrowed from session
  local source_entries = self.items
  local session = self.session

  if session then
    if #upper_query > #session.upper_query
        and upper_query:sub(1, #session.upper_query) == session.upper_query then
      -- Query extends previous → narrow from previous matches
      source_entries = session.entries
    elseif #upper_query < #session.upper_query
        and session.upper_query:sub(1, #upper_query) == upper_query then
      -- Backspace → find the right level to resume from
      local found = false
      for li = #session.levels, 1, -1 do
        local level = session.levels[li]
        if #upper_query >= #level.upper_query
            and upper_query:sub(1, #level.upper_query) == level.upper_query then
          -- If this level was truncated, we can't safely narrow from it;
          -- fall back to the full item set for a complete rescan
          if level.truncated then
            source_entries = self.items
          else
            source_entries = level.entries
          end
          -- Trim levels to this point
          for j = #session.levels, li + 1, -1 do
            session.levels[j] = nil
          end
          found = true
          break
        end
      end
      if not found then
        source_entries = self.items
      end
    elseif upper_query == session.upper_query then
      -- Same query, return cached results
      return session.items
    else
      -- Unrelated query, full scan
      source_entries = self.items
    end
  end

  local all_entries, display_items, truncated = scan_and_rank(
    source_entries, upper_query, has_dots, query_segments, num_qseg
  )

  -- Update session (stores matches for narrowing, capped at MAX_LEVEL_ENTRIES)
  if not self.session or source_entries == self.items then
    -- New session
    self.session = {
      upper_query = upper_query,
      entries = all_entries,
      items = display_items,
      levels = { { upper_query = upper_query, entries = all_entries, truncated = truncated } },
    }
  else
    -- Extend existing session
    self.session.upper_query = upper_query
    self.session.entries = all_entries
    self.session.items = display_items
    self.session.levels[#self.session.levels + 1] = {
      upper_query = upper_query,
      entries = all_entries,
      truncated = truncated,
    }
  end

  return display_items
end

return Fuzzy
