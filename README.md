# SubZero

SubZero (`sz`) is an app for quick data explorations in Snowflake from the terminal.
Explore your Snowflake catalog, execute queries with an embedded neovim editor, and visualize simple line and bar charts.

## Installation

> [!NOTE]
> This has only been tested on macOS, but it should work on Linux with appropriate changes to the package manager commands. 

### Install dependencies via Homebrew

```sh
brew install rust neovim snowflake-cli
```

Verify both are on your PATH:

```sh
nvim --version
snow --version
```

### Configure Snowflake

If you haven't already, set up a connection in `~/.snowflake/connections.toml`:

```toml
[connections.default]
account = "your-account"
user = "your-user"
authenticator = "externalbrowser"  # or other auth method
warehouse = "YOUR_WH"
database = "YOUR_DB"
role = "YOUR_ROLE"
```

Test it:

```sh
snow sql -q "SELECT 1"
```

### Install subzero via Cargo

```sh
cargo install --git https://github.com/rsmenon/subzero.git
```

Launch the binary by running `sz` in your terminal. Data is stored in `~/.subzero/` (catalog cache, query history, saved queries, logs).

## Documentation

### Basic Navigation

Subzero has three pages — **Explore**, **Query**, and **Settings** — accessible from the top bar. The active pane is highlighted with a blue border.

<img width="1454" height="57" src="https://github.com/user-attachments/assets/d1faa866-aea5-4b7c-97a9-188209d48928" />


| Action | Key |
|--------|-----|
| Enter Explore mode | <kbd>E</kbd> |
| Enter Query mode | <kbd>Q</kbd> |
| Enter Settings mode | <kbd>S</kbd> |
| Focus next pane | <kbd>Tab</kbd> |
| Focus previous pane | <kbd>Shift+Tab</kbd> |
| Return focus to top bar | <kbd>Esc</kbd> |
| Quit | <kbd>Ctrl+D</kbd> |

---

### Explore

The Explore tab lets you browse your Snowflake catalog — databases, schemas, tables, and views — and inspect column definitions and sample data without writing any SQL.

<img width="1470" height="923" src="https://github.com/user-attachments/assets/f16363a2-e78e-46bf-9a3e-6b0e5ea85c37" />


**Tree pane** (left): Navigate the catalog hierarchy.

| Action | Key |
|--------|-----|
| Move up / down | <kbd>↑</kbd> / <kbd>↓</kbd> or <kbd>k</kbd> / <kbd>j</kbd> |
| Expand / collapse node | <kbd>Enter</kbd>|
| Collapse / go to parent | <kbd>←</kbd> |
| Fuzzy search catalog | <kbd>/</kbd> |
| Refresh catalog from Snowflake | <kbd>R</kbd> |
| Move focus to detail pane | <kbd>Tab</kbd> |

The catalog is cached locally for speed. On first launch, press <kbd>R</kbd> to populate it. Refreshes run in the background — you can keep working while they complete.

**Detail pane** (right): Shows metadata for the selected table or view. Two tabs:

- **Columns** — column names, types, and nullability. Press <kbd>/</kbd> to filter by name.
- **Preview** — first rows of the table, fetched on demand. Press <kbd>h</kbd> / <kbd>l</kbd> to scroll horizontally. Press <kbd>Enter</kbd> on any row to open a full row detail popup.

Switch tabs with <kbd>C</kbd> (Columns) or <kbd>P</kbd> (Preview).

---

### Query

The Query tab is where you write and run SQL. The editor is a fully embedded Neovim instance, so all normal Vim motions, registers, and undo history work as expected. Autocomplete is context-aware and draws from the cached catalog — trigger it with the standard Neovim <kbd>Ctrl-X</kbd><kbd>Ctrl-O</kbd>.

<img width="1470" height="923" src="https://github.com/user-attachments/assets/52576868-79d6-4f58-a7ba-cceecd53b91f" />


**Editor pane** (top):

| Action | Key |
|--------|-----|
| Run query | <kbd>Ctrl+R</kbd> |
| Save query | <kbd>Ctrl+S</kbd> |
| Export results to CSV | <kbd>Ctrl+E</kbd> |
| New query (clear editor + results) | <kbd>Ctrl+N</kbd> |
| Open query history | <kbd>Ctrl+P</kbd> |
| Open saved queries | <kbd>Ctrl+L</kbd> |
| Move focus to results pane | <kbd>Tab</kbd> |

**Results pane** (bottom): Displays query output as a scrollable table.

| Action | Key |
|--------|-----|
| Navigate rows | <kbd>↑</kbd> / <kbd>↓</kbd> or <kbd>k</kbd> / <kbd>j</kbd> |
| Scroll columns | <kbd>←</kbd> / <kbd>→</kbd> or <kbd>h</kbd> / <kbd>l</kbd> |
| Page up / down | <kbd>PageUp</kbd> / <kbd>PageDown</kbd> |
| Jump to first / last row | <kbd>Home</kbd> / <kbd>End</kbd> |
| Open row detail popup | <kbd>Enter</kbd> |
| Move focus to editor | <kbd>Tab</kbd> |

**Charts**: After running a query, press <kbd>Ctrl+N</kbd> (from the results pane) to add a chart tab. Select a chart type, X-axis, and Y-axis in the settings panel, then choose **Generate** to render. You can have up to 9 chart tabs per query. Switch between tabs with number keys (<kbd>1</kbd>–<kbd>9</kbd>) or return to the table with <kbd>T</kbd>.

<img width="1466" height="499" src="https://github.com/user-attachments/assets/2ab6dfde-fc50-4435-8060-81ff844f9ba3" />


**Saved queries**: Press <kbd>Ctrl+S</kbd> to save the current query with a name. <kbd>Ctrl+L</kbd> opens the saved queries overlay where you can load, rename (<kbd>e</kbd>), or delete (<kbd>d</kbd>) entries. <kbd>Ctrl+P</kbd> opens run history (all previously executed queries, filterable).

---

### Settings

The Settings tab has three panes: **Connection**, **Cache**, and **Theme**. Navigate between them with <kbd>Tab</kbd> / <kbd>Shift+Tab</kbd>.

<img width="1470" height="923" src="https://github.com/user-attachments/assets/1400351c-9c7b-429d-9c82-9b7d70874362" />


**Connection pane**: Switch the active Snowflake connection. Press <kbd>Enter</kbd> to open a picker listing all connections defined in `~/.snowflake/connections.toml`. Selecting one updates the warehouse, database, and role fields automatically and applies immediately.

**Cache pane**: Manage the local catalog cache.

- **Clear Cache** — removes cached data; the tree will be empty until you refresh.
- **Refresh Now** — triggers a background fetch from Snowflake to rebuild the cache.

Use <kbd>h</kbd> / <kbd>l</kbd> to move between buttons, <kbd>Enter</kbd> to activate.

**Theme pane**: Customize the color palette. Each UI element is listed with its current hex color. Navigate with <kbd>j</kbd> / <kbd>k</kbd>, press <kbd>Enter</kbd> to edit a value, and type a valid hex code (e.g., `#ebdbb2`). Press <kbd>Enter</kbd> to confirm or <kbd>Esc</kbd> to cancel. Use **Reset** to restore Gruvbox dark defaults, or **Save** to save the current theme to `~/.subzero/theme.yaml` for persistence across restarts.

