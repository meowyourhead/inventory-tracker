# Inventory — AI Development Handoff

> **Read this file completely before changing anything.** It is written for a
> new AI with no previous knowledge of this project.

> **Version 1.0.0 (2026-11-16) supersedes older sections below.**
> - Source and releases live at <https://github.com/meowyourhead/inventory-tracker>.
>   Versions are semver from `Cargo.toml`; the old date-stamp releases,
>   `version.txt`, `build.ps1`/`BUILD.cmd`, the USB transfer scripts, the AI
>   bundle scripts, `Create-Direct-Shortcut.cmd` and the `launcher\` folder
>   (and `SETUP-LAUNCHER.md`) were removed. Mentions of them below are history.
> - Updates: `src/update.rs` checks GitHub Releases
>   (`/releases/latest`), downloads the `Inventory.exe` asset, verifies size +
>   SHA-256 (GitHub asset `digest` or an `Inventory.exe.sha256` asset), swaps
>   the running exe with `self_replace`, then `AppEvent::Restart` runs
>   `cleanup()` and relaunches with `--updated-from <pid> <old version>`.
>   API: `update-info`, `update-check`, `update-install`; progress is pushed on
>   the `update-progress` channel. UI: Settings ▸ Updates card + `#updModal`.
>   Release tag must be `v<Cargo version>`.
> - Shared data mode now comes from `inventory.config.json`
>   (`{"dataDir": "..."}`) beside the exe, or `INVENTORY_DATA_DIR`.
> - Removed API methods: `status`, `force`. Removed CLI `--dump-xlsm`.

## 1. What this project is

Inventory is a portable Windows desktop application for laboratory stock
management. The current implementation is a **100% Rust host process** using
Microsoft Edge WebView2 for the user interface. There is no Electron, no Node.js
runtime, no local web server and no console window in release builds.

The application is a rewrite of an older Electron app. The original Electron
sources are kept in the sibling `../electron` folder for reference only. The
Rust version must keep the same features and the same `inventory.json` data
format.

Primary users: laboratory staff who log stock in/out, track lots and expiry
dates, run stock counts, and share one inventory file across several computers.

## 2. Non-negotiable constraints

1. **Rust only for the host.** Do not reintroduce Electron, Node.js, a bundled
   browser, a local HTTP server or a command window.
2. **Preserve existing behaviour and UI.** The complete UI is in
   `ui/index.html`; it is the original interface and must keep working.
3. **Preserve the data format.** Existing `inventory.json` files from the
   Electron version must keep loading and saving without conversion.
4. **Portable by default.** With no `INVENTORY_DATA_DIR`, data lives beside
   `Inventory.exe` (USB stick or local folder).
5. **Shared-folder mode must fail loudly.** If `INVENTORY_DATA_DIR` is set and
   the folder is missing/unwritable, show an error instead of silently starting
   a local divergent copy.
6. **Cross-machine edit lock.** Only one session may edit; others are
   read-only. Do not remove or weaken this.
7. **No remote web content.** The UI is served only from the private custom
   protocol; WebView2 must not load external pages.
8. **Windows 10/11 x64 + WebView2 Runtime.** Keep the release executable a GUI
   subsystem binary (`windows_subsystem = "windows"`).
9. **Do not break deployment.** The network launchers in `launcher\` depend on
   `version.txt`, the exe name and the `data\` layout.
10. **Keep the build reproducible and portable.** No installer, no runtime
    downloads, no machine-specific paths in source.

## 3. Current repository layout

```
Inventory (Rust)\
├── README.md              repository overview
├── electron\              legacy Electron implementation (reference only)
├── rust\                  the current application and everything needed to build it
│   ├── AI-HANDOFF.md      this handoff document
│   ├── Cargo.toml         dependencies, release profile (semver is a placeholder)
│   ├── Cargo.lock         pinned dependency graph
│   ├── build.rs           embeds Windows icon/version resources
│   ├── BUILD.cmd          user-facing build wrapper
│   ├── build.ps1          release build + deployment files + zip
│   ├── TRANSFER-TO-USB.cmd  user-facing USB transfer wrapper
│   ├── transfer-to-usb.ps1  copies only Inventory.exe to a chosen USB drive
│   ├── Create-Direct-Shortcut.cmd  portable desktop shortcut to the local exe
│   ├── MAKE-AI-BUNDLE.cmd creates the AI handoff upload
│   ├── make-ai-bundle.ps1 implementation of the handoff generator
│   ├── ai-bundle\         generated upload files (ZIP + Markdown; not source)
│   ├── src\
│   │   ├── main.rs        native window, WebView2, protocol, API dispatch, CLI
│   │   ├── state.rs       data, lock, backups, watcher, window resize
│   │   └── xlsm.rs        XLSM/XLSX reader (calamine)
│   ├── ui\index.html      entire UI + injected `window.desk` bridge
│   ├── assets\icon.png    window icon, embedded into the exe
│   ├── assets\icon.ico    Windows executable icon, embedded by build.rs
│   ├── assets\cyberpunk-logo.svg  Cyberpunk theme-preview wordmark
│   ├── assets\genshin-logo.svg    Genshin theme-preview wordmark
│   ├── assets\zzz-logo.png        ZZZ theme-preview wordmark (Zenless UI)
│   ├── launcher\          optional network-share launchers
│   ├── testdata.xlsm      sample workbook used by the parser tests
│   ├── zh-cn.ttf          HYWenHei-85W, embedded as the Genshin UI font
│   ├── fonts\             Rajdhani, Share Tech Mono and Alegreya Sans (woff2)
│   │                      used by Cyberpunk/Genshin, embedded in the exe
│   ├── cursors\           Genshin cursors (.cur), embedded
│   ├── README.md          detailed application/build documentation
│   └── SETUP-LAUNCHER.md  shared-folder deployment guide
└── ...
```

Build artifacts in `rust\`: `Inventory.exe`, `version.txt`,
`Inventory-<date-time>.zip`, `target\`, optional `data\`. They are not source.

## 4. Technology stack

Rust 2021, minimum Rust 1.85. Key crates (also see `Cargo.toml` / `Cargo.lock`):

| Crate | Role |
|---|---|
| `tao` | Native window and event loop |
| `wry` | WebView2 host, custom protocol, JavaScript evaluation |
| `serde` / `serde_json` | Inventory JSON and API payloads |
| `calamine` | XLSM/XLSX/XLS/XLSB parsing |
| `rfd` | Native open/save/message dialogs owned by the main window |
| `notify` | File watcher for live reload |
| `chrono` | Timestamps and auto-backup names |
| `base64` | XLSM bytes over the JSON bridge |
| `image` | Decode the PNG window icon |
| `windows-sys` | Atomic file replace and monitor work area |
| `winresource` (build) | Embed `.ico` and version metadata |

Release profile: `opt-level = "s"`, thin LTO, one codegen unit, stripped.

## 5. Architecture and process model

There is one process. `main.rs` creates a `tao` event loop and a `wry` WebView2
window. The UI HTML is `include_bytes!`-embedded into the executable, so there
are no loose UI files at runtime.

- The WebView loads `inventory://localhost/index.html`.
  WebView2 exposes it to JavaScript as `http://inventory.localhost/index.html`.
- `main.rs::serve` answers:
  - `/` or `/index.html` → embedded UI
  - `/build/icon.png` → embedded PNG
  - `/api/<method>` → JSON API, dispatched on a worker thread
- The UI calls the backend with `fetch('/api/...', { method: 'POST', ... })`.
- `ui/index.html` contains a small injected `window.desk` shim at the top. It
  reproduces the old Electron preload API exactly. Do not rename methods
  without updating all UI calls.
- Push events (`data`, `lock-change`) go from Rust to JS through
  `webview.evaluate_script("window.__deskPush(...)")`.
- The window starts hidden and is shown on `PageLoadEvent::Finished`, with a
  6-second safety timer. Startup also prewarms the parsed JSON cache.
- Resize requests are sent as `AppEvent::Resize` to the event loop, applied to
  the native window, and replied to through an `mpsc` channel.
- The main window is borderless (`with_decorations(false)` plus the Windows
  undecorated shadow). The UI draws its own title bar; minimize, maximize,
  close, drag and edge-resize actions are sent as `AppEvent::Window` /
  `AppEvent::WindowResize` and applied to the native window on the event loop.
  The app still starts maximized. Both icons come from `assets/icon.png`:
  `with_window_icon` sets the small icon, and `with_taskbar_icon` sets the big
  icon because tao's window-icon path only sends `ICON_SMALL`; Windows uses
  `ICON_BIG` for the taskbar, so without it other PCs can fall back to a
  cached/generic taskbar icon.
- Automatic backups go to `settings.backupDir` when set, otherwise to
  `AutoBackups` beside the data file. Settings → Data & Backup calls
  `backup-status`, `choose-backup-dir` and `open-backup-dir`; choosing a folder
  persists the path in the shared inventory settings.
- The WebView2 profile is stored in
  `%LOCALAPPDATA%\Inventory\WebView2`, not next to the exe.
- `INVENTORY_DEVTOOLS=1` enables DevTools. It is off by default.

## 6. `window.desk` API contract

All methods are async and return JSON. Defined in `ui/index.html`, dispatched in
`src/main.rs`, implemented in `src/state.rs`.

| Method | Request | Success response |
|---|---|---|
| `load` | none | `{ok:true, data, status}` |
| `save` | full inventory object | `{ok:true}` |
| `activity` | none | `true` (renews heartbeat if lock is ours) |
| `lock` | none | `{ok:true, status}` or `{ok:false, status}` |
| `unlock` | none | `{ok:true, status}` |
| `backup` | none | `{ok:true, path}` or `{canceled:true}` or `{ok:false,error}` |
| `backup-status` | none | `{ok:true, path, isDefault, exists}` |
| `choose-backup-dir` | none | `{ok:true, path}` or `{canceled:true}` |
| `open-backup-dir` | none | `{ok:true, path}` or `{ok:false,error}` |
| `restore` | none | `{ok:true, path, data}` or `{canceled:true}` or `{ok:false,error}` |
| `resize` | `{factor}` | `{ok:true, width, height}` |
| `window-state` | none | `{ok:true, maximized}` |
| `window-minimize` | none | `{ok:true}` |
| `window-toggle-maximize` | none | `{ok:true, maximized}` |
| `window-close` | none | `{ok:true}` (then the process exits) |
| `window-drag` | none | `{ok:true}` (native move loop) |
| `window-resize` | `{direction}` (`n`,`ne`,`e`,`se`,`s`,`sw`,`w`,`nw`) | `{ok:true, maximized}` |
| `read-xlsm` | `{data: base64}` | `{ok:true, workbook}` or `{ok:false,error}` |

`save` is rejected unless this process owns the lock. `restore` and `read-xlsm`
do not require ownership by themselves; the UI blocks restore/import when
read-only.

Unknown API methods return `{ok:false, error:"Unknown API method"}`. The
protocol handler catches panics and returns `{ok:false,error:"Internal backend
error"}` rather than killing the app.

## 7. Inventory data model

Default data directory: folder containing `Inventory.exe`. Override:
`INVENTORY_DATA_DIR`. Files:

```
inventory.json           main dataset, pretty-printed UTF-8, atomic replace
inventory.lock.json      present while a session holds/attempts editing
AutoBackups\             snapshots written on graceful close after a save
```

`inventory.json` root shape:

```json
{
  "items": [
    {
      "id": "string",
      "name": "string",
      "category": "string",
      "color": "#rrggbb or empty",
      "shipmentAction": "none | ltl | qc",
      "catalog": "string",
      "pack": "string",
      "reorder": 0,
      "lt": 0
    }
  ],
  "transactions": [
    {
      "id": "string",
      "type": "in | out | cat_add | cat_remove",
      "itemId": "string",
      "entryId": "string (out only, links to an in transaction)",
      "lot": "string",
      "exp": "YYYY-MM-DD or 9999-12-31",
      "qty": 1,
      "date": "YYYY-MM-DD",
      "by": "string",
      "remarks": "string",
      "ts": 0,
      "undone": false
    }
  ],
  "catalogLog": [],
  "settings": {
    "theme": "glass",
    "mode": "light",
    "font": "sfpro",
    "fontSize": 14,
    "quickLogoutMode": "fefo",
    "appName": "Inventory",
    "backupDir": "optional custom automatic-backup folder"
  }
}
```

Important details:

- Stock balance is derived from transactions. `in` adds quantity; `out`
  subtracts. An `out` consumes a specific `in` entry via `entryId`.
- `undone: true` transactions are ignored for balances and history. The UI has
  undo/restore actions that append or mark records; read the UI code before
  changing this logic.
- `cat_add` / `cat_remove` are catalog audit entries; some carry snapshots used
  by the UI undo function.
- `catalogLog` is optional in old files; the UI initialises it with `??= []`.
- `settings` may be missing/partial in old files; UI `normalizeSettings()` fills
  defaults and migrates old theme/font ids.
- `settings.backupDir` is optional; when empty/missing, automatic backups go to
  `AutoBackups` beside the data file. Because it is stored in the shared JSON,
  every client must be able to reach the chosen path.
- Item `lt` is optional. `reorder` is the reorder point. `shipmentAction` is
  `none`, `ltl` or `qc` (first-use verification prompt).
- Dates are stored as `YYYY-MM-DD`; timestamps are epoch milliseconds.
- The UI maintains localStorage preferences separately (UI scale
  `invUiScaleFactor`, last backup info `invLastBackup`, column choices, etc.).
  Those are per-device and not part of the shared JSON.

## 8. Locking and backups (must stay compatible)

`state.rs`:

- `inventory.lock.json` contains
  `{pid, user, startedAt, heartbeat}`.
- A lock is ours when its `pid` equals this process id.
- A lock whose `heartbeat` is older than **30 seconds** is stale and is deleted
  automatically (`STALE_MS = 30_000`).
- The heartbeat is renewed only on real UI activity (`activity` calls), not by
  a blind timer. A 10-second background checker detects if another user took
  over and pushes `lock-change` to make the UI read-only.
- The UI countdown releases editing on its own after **10 minutes** without
  mouse or keyboard activity (previously 5 minutes).
- Lock ownership is decided by the `session` field (computer name + PID +
  start time), not the PID alone, which can collide across PCs. A free lock
  is taken with `create_new`, so two PCs cannot both acquire it.
- `/api/*` accepts POST only; the WebView cannot navigate away from
  `inventory.localhost` or open popups, and the page is served with a CSP.
- Normal close removes the lock.
- JSON writes use a unique temp file + `MoveFileExW(...REPLACE_EXISTING |
  WRITE_THROUGH)` on Windows. Do not replace this with `fs::rename`; that fails
  when the destination exists on Windows.
- On graceful close, if this session saved at least once, a backup is written
  to `AutoBackups\inventory-backup_YYYY-MM-DD_HH-MM-SS_by-<user>.json`. Keep the
  newest 30. The folder is only needed when there is something to save.
- Restore validates that the chosen JSON has array `items` and `transactions`.
  It rejects invalid JSON and a UTF-8 BOM (same as the Electron version).
- The backup dialog default name is `inventory-backup-YYYY-MM-DD.json`.

## 9. XLSM import

`src/xlsm.rs` uses `calamine` and returns the shape the UI expects from the old
SheetJS code:

```json
{
  "SheetNames": ["Sheet1"],
  "Sheets": { "Sheet1": [["cell", "values"], ["..."]] }
}
```

- Rows are padded to a rectangular grid; leading empty rows/columns are kept.
- Dates are emitted as ISO `YYYY-MM-DD` strings, not JS `Date` objects.
- The UI's XLSM importer (`parseXlsmWorkbook` and related functions in
  `ui/index.html`) handles header rows, categories, lots, quantities, running
  balances and validation. It is complex; read it before changing the parser.
- Ledger workbooks are keyed by catalog number when it is real. Placeholder
  catalog values such as `-`, `N/A`, `none` or `x` are not identities: the
  ledger importer falls back to the item name (then sheet name), so separate
  sheets such as `Gloves XS/S/M/L/XL` stay distinct even when every sheet uses
  `-` as its catalog. Do not reintroduce a raw catalog key for placeholders.
- Import supports merge and replace modes. It requires an edit lock and creates
  a JSON backup before committing.
- `read-xlsm` accepts base64 bytes over the API because WebView2 cannot hand a
  Rust `File` object directly.
- `testdata.xlsm` is used by the two unit tests. The sample workbook's displayed
  dates are authoritative; a SheetJS pre-1901 timezone artifact is intentionally
  not reproduced.

## 10. UI conventions

- The complete UI is one embedded HTML file. It contains CSS, markup and
  vanilla JavaScript. There is no framework, bundler or npm step.
- Keep the existing class names, IDs and inline handlers unless you update every
  reference. Many controls are wired with `$('#id').onclick`/`onsubmit`.
- The top of the file contains the injected `window.desk` shim. Backend calls
  use `fetch('/api/...')`.
- Views/tabs include Dashboard, Scan, Count, History, Charts, Settings, Stock
  Usage and related panels. The UI also has theme/font settings, UI scale,
  column pickers, charts and print tools. There is no example-data generator.
- Catalog rows use `tr.clickRow[data-edit-cat]` and history rows use
  `tr.clickRow[data-tx]`; delegated `tbody` click handlers call `editCat` or
  `editTx`. Do not add per-row edit buttons. Row-local undo/restore/remove
  buttons must stop propagation.
- Settings is a two-column flex layout: Appearance + Workspace on the left,
  Data & Backup + Global scope on the right. Cards use `.settingsCard` and the
  columns `.settingsColumnMain/.settingsColumnSide`; it collapses to one column
  at 1350 px and below.
- Dashboard charts: 1-month usage, expiry, stock cover and stock level are the
  dashboard set and have visibility toggles; 6-month usage and top consumed
  always appear on the Charts tab. The stock-level chart has a hover crosshair
  drawn in SVG (`.stockHover`, `.hvCatcher`) that shows the date, on-hand value
  and last movement. `moveTip()` divides cursor
  coordinates and viewport size by the UI-scale CSS zoom before setting
  `left`/`top`, otherwise tooltips drift toward the bottom-right.
- The window is borderless. `[data-drag]` regions start a native drag after a
  4 px threshold; `.winControls`/`#winMin`/`#winMax`/`#winClose` drive the
  window API; `.wres*` are the eight edge/corner resize handles. Keep the
  `body.isMax` class in sync so the handles hide while maximized. The three
  buttons are identical in every theme: 54 px wide, full header height, flush
  to the top-right corner, transparent, 14 px line icons, hover surface tint and
  `#d13438` close hover. The full-bleed `.top` uses the same `18px/21px` main
  padding, so do not add family-specific window-control geometry. `winControlAt`
  adds a 3 px hit margin around the controls and routes edge clicks to the
  nearest button; keep that guard so a click at the window edge is not stolen by
  the `[data-drag]` header. `.winBtn` must not have an `:active` transform:
  scaling/translating the button on press moves its top edge away from the window
  edge, so a click at y=0 misses and lands on `.top`. Cyberpunk and Genshin used
  to have that transform and were the only themes with the bug.
- Dashboard chart toggle tokens are hidden (`#dashTokens{display:none}`); the
  four Dashboard charts are always shown, and all six Charts-tab charts are
  always shown. The legacy `prefs.charts` flags still exist but are not exposed.
- Genshin widens the sidebar to 112 px (`html[data-family=genshin] .app`) so
  `Dashboard` and the longer nav labels fit; the ≤1000 px layout still collapses
  to one column.
- Scan fits the window at normal widths: `go('scanops')` adds `body.scanActive`,
  which makes `main` a fixed-height flex column (`--vh-fix`). The left form card
  and right catalog/history card are content-sized (`.scanForm`/`.scanLeft`
  height auto); the right page capacity is measured from the panel height so
  sizing the card to content does not reduce the rows shown. `#scanops` uses
  overflow auto only when content spills. The ≤1050 px layout still stacks and
  scrolls.
- `#backToTop` is a 60 px circular button inside `.dashCardsPane`, absolutely
  positioned at `right:18px;bottom:10px` (left of the 9 px scrollbar).
  `updateBackToTop()` shows it only while Dashboard is active and
  `dashboardScrollTop() > 160`; it hides at the top and on every other view.
  It is `visibility:hidden`/`pointer-events:none` when off and `tabIndex=-1`, so
  it cannot be tabbed to or clicked invisibly. `dashboardScrollTop()` includes
  `#items.scrollTop`, so the button appears when the card pane is scrolled;
  `scrollActiveViewTop()` also scrolls `#items` back to the top. The Dashboard
  cards pane has a visible 9 px themed scrollbar, while all page-level and other
  view scrollbars stay hidden.
- Scan layout: `.scanWorkspace` uses `grid-template-rows:minmax(0,1fr)` and
  `.scanRight>.card` is a bounded flex column. Each catalog/history table sits
  in `.rightPaneScroll` (flex:1, min-height:0, overflow:auto) with the pager
  outside it as `flex:0 0 auto`, so page numbers stay pinned and fully visible.
  `scanTableCapacity()` measures the visible scroll area, and `ensureScanFit()`
  trims the page size once if a render still overflows. `renderScanPane()`
  centralises remeasure/render for view entry, tab switches and resizes. At
  `max-height:820px` the Scan inputs and reset button compact; at
  `max-height:760px` `#scanops` falls back to scrolling rather than clipping.
- Dashboard is a fixed one-page flex column: `body:has(#dashboard.active)` locks
  `main` to `--vh-fix`, `.dashCardsPane` is 55% with `#items` as its own scroll
  box, and `.dashChartsPane` is 45% with a 24 px top gap (the chart row sits
  noticeably lower) and a 4-column/2-row chart grid:
  1-month usage is `1/1/3/2` and Stock cover `1/2/3/3` (two narrow,
  full-height columns); Expiry remains `1/3/2/5` and Stock level `2/3/3/5`
  (each two columns wide and half height, i.e. their original size/positions
  in the right half). The monthly/cover headers are single rows: title on the
  left, `.pTools` right-aligned to the panel content edge. The search shrinks
  (76 px max / 44 px min) and the period select uses `3 mo`/`6 mo`/`12 mo`;
  stock cover uses compact `Months` and `Least` labels. The dashboard hides
  `#monthlyTotal` so everything fits the narrow columns. The dashboard search row is centred, with Batch delete
  pinned to the right. `#items` uses `align-content:start`,
  `grid-auto-rows:max-content` and `scrollbar-gutter:stable`, so a sparse
  search result keeps normal card sizes and width. `body:has(#dashboard.active) .top{flex:0 0 auto}` preserves the
  original header spacing (title top 21 px, Dashboard top 92 px). Chart panel subtitles are hidden, controls are 22 px, and rows use
  9–10 px type. Monthly renders its full ranked list and Top renders up to 16
  rows before `fitChartRows()`
  trims to capacity; expiry switches to four full-width rows when its body is
  ≥110 px; usage rows stretch to fill the Charts-tab body.
- The **Charts** tab (`#charts`, below History) shows every chart at full-page
  size in a 2×3 grid. `go('charts')` moves the single `.dashPanels` grid into
  `#chartsHost`; leaving Charts moves it back into `.dashChartsPane`. Charts
  forces `.chartOff` panels visible and keeps the `+N more…` row trimming.
  6-month usage and top consumed are hidden on the Dashboard by CSS and only
  appear in Charts.
- `#monthlySearch` filters the 1-month usage rows and sits directly left of
  `#monthlyBasis` with a 4 px gap (total to its left, expand button to the
  right). `#monthlyTotal` is `white-space:nowrap` so the unit total stays on one
  line. Low-stock cards carry `.low` (red-tinted panel, red quantity/name) and
  no longer render the old Low stock footer. `.expLine .lbl` is a fixed 54 px
  block at 8.5 px with 9 px date pills, so the Earliest and Latest dates start
  at the same x while leaving enough width for the lot number.
  Stock level uses `#stockItemCombo`: `#stockItemBtn` opens `#stockItemMenu`,
  whose `#stockItemSearch` filters `#stockItemList` by name/category/catalog;
  choosing a row updates `stockSelId` and redraws the chart. Escape or an
  outside click closes the menu. `constrainCoverHeight()` caps both the Stock
  cover and Stock level panels at the 1-month panel height (divided by the UI
  zoom factor). Stock cover lists only the rows that fit and appends a
  `+N more…` label. The stock chart meta line is hidden in the dashboard;
  `stockChartHeight()` fills the remaining body and the actual-width SVG
  viewBox in `drawStockChart()` keeps the plot undistorted.
- Count tab follows the reference: pre-filled system quantities, `scTouched` and
  `scChecked` sets, a checkbox column, live variance, undo/discard/apply, and
  `db.settings.countHistory` rendered in the Past stock counts card. There is
  no scan input or pager in this tab.
- Dashboard table view adds `dashSel` checkboxes and `#batchDeleteBtn` beside
  the search field; batch delete logs one `cat_remove` per removed item and can
  restore the whole batch. Catalog `#bulkEditBtn` uses `scanCatFiltered()` to
  apply category/colour, shipment, pack, par level, lead time and no-expiry
  status to all matching rows and logs a `cat_bulk_edit` entry with snapshots
  for undo.
- Compact tables: dashboard rows are 24 px CSS with 12 px text and a 20 px
  quick-logout button; Balance is the last column after Status. History rows
  are 24 px / 11.5 px, Scan rows 23 px / 11.5 px, and Count rows 24 px with
  20 px inputs. `remeasureMainHist()` measures the actual row height and keeps
  one row of slack so the History pager stays inside the card. CSV export was
  removed. Global data scope is now applied in `renderHist()` (both the row
  filter and the filter-dropdown source `activeTx`), `renderScanRecent()` and
  `stockPoints()`; usage/monthly/top/cover already use `usedInMonths()` scope.
  In-stock items still remain visible outside the scope date. The dashboard Print button (and
  browser print) calls `prepareDashboardPrint()`, forces table view, adds
  `body.print-dash`, and prints only the flattened reagent table; `afterprint`
  restores the previous card/table view. The print table hides selection and
  quick-logout columns and uses 8 pt type.
- Catalog items may set `noEx:true` via **No expiry** in the editor. The log-in
  modal pre-fills 9999-12-31 for those items and scan-in falls back to
  9999-12-31 instead of prompting.
- Escape closes the topmost `.modalbg.open` without running its confirm
  callback, and also clears transient popovers (colour menus, notifications,
  column picker, help, expiry popover, bulk-ref dropdown).
- The `#allModal` All items window is a fixed-height flex shell sized from
  `--vh-fix`; `.hd` and the `.allTools` bar (search, category select, match
  count) sit outside `.panel`, which is the scrolling region with a visible
  12 px themed scrollbar. The filters are window-local (`allQuery`, `allCat`)
  and are seeded from `prefs.catFilter` on open; `itemsView(cat,qArg)` and the
  `renderCardsInto(el,rows)` / `renderTableInto(el,rows)` helpers accept
  pre-filtered rows so the window does not read the Dashboard search box.
  Opening it resets the panel scroll. The dashboard `#items` card grid has
  8 px top padding so `.item:hover` `translateY(-2px)` cannot clip the first
  row. Card/table clicks pass `openItem(id,true)`, setting `itemFromAll`;
  `closeItemModal()` then calls `returnToAllItems()`, which reopens the window
  with its filters and panel scroll preserved. Item closes from Dashboard or
  other views pass no flag and do not reopen it.
- `#statModal` (all chart detailed views: usage, monthly, expiry, top, cover,
  stock) uses the same wide fixed-height shell. `.hd` and `#statCtrls` are
  pinned; `#statBody` is the single scrolling region with a 12 px themed
  scrollbar. `openStat('stock')` adds `.statStock`, which switches the modal
  to auto height and makes the body non-scrolling, so the window ends just
  after the 340 px stock chart instead of filling the screen.
- The 1-month usage detailed view adds `#statMonthlySearch` to `#statCtrls`.
  It filters on a local `statMonthlyQuery` and calls `monthlyRows('')`, so it
  ignores the dashboard `monthlyQuery`; the row count and estimated-units KPI
  follow the filtered rows. The search input stays focused after the debounced
  re-render.

- `parseXlsmAlinity()` handles Alinity IMS workbooks before the generic ledger
  parser. `xlsmMasterEntries()` reads Master List headers (List Number, Item
  Name, Kit Size, Total Balance, Latest Expiry, Par Level); each reagent tab
  supplies the authoritative per-unit rows (B expiry, C lot, D item, E log-in,
  F log-out, G initials). Master-only catalogs are imported as opening
  balances. It returns `null` for Atellica/General Store workbooks so they keep
  using `parseXlsmLedger()`.
- Genshin action buttons use `--g-btn-bg`/`--g-btn-fg` off-white/ivory with dark
  text and only switch to the daily elemental deep colour on hover; danger
  buttons stay red.
- Graphics quality: Settings → Theme has `#graphicsMode` (**Low** / **High**),
  persisted as `localStorage.invGraphics` (`low` or `high`). Low sets
  `html[data-graphics="low"]`, hides `#genshinStars` and the `body`/`.app`
  decorative pseudo-elements, and disables backdrop blur, shadows, text shadows,
  transitions and animations. It is a per-device preference so a slower client
  does not change shared settings. The Genshin starfield checks
  `data-graphics` and is rebuilt when switching back to High.
- Motion override: Settings → Theme has `#motionToggle`, persisted as
  `localStorage.invMotionOverride`, which sets `html[data-motion="on|auto"]`.
  Cyberpunk/Genshin motion (HUD sweep/flicker and Genshin glints) is skipped
  when `prefers-reduced-motion: reduce` and the override is off. The
  reduced-motion rules must include `*::before` and `*::after`, because the
  theme animations live on pseudo-elements. Animations pause while the window is
  unfocused or hidden (`html[data-motion-paused="1"]`).
- Genshin's sky is a JS-generated random starfield in `#genshinStars`
  (`buildGenshinStars`, 28–46 static dots + 6–9 glints with random positions,
  sizes, colours and timing). The dots are static; every glint drifts, twinkles
  and spins. `gs-drift` runs once with `forwards`, and `bindGlintLife` listens
  for its `animationend` to call `randomizeGlintDrift`, which gives the glint a
  new position, path, duration and delay. The pattern therefore never repeats
  the old cycle. Do not restore `infinite` on `gs-drift`. The background
  deliberately does **not** follow the cursor: pointer parallax was removed
  because transforming the full-screen starfield on every mouse move caused
  noticeable jank. Do not reintroduce `gsMove` or the old tiled CSS star
  gradients.
- Settings → Data & Backup has the automatic-backup folder path plus
  `#backupDirBtn` (choose) and `#openBackupDirBtn` (open); the path is saved in
  `settings.backupDir`.
- UI scale: factor clamped to `0.85..1.3`, step `0.05`, applied with CSS zoom and
  a matching native window resize (base 1496×946, minimum 1080×720, clamped to
  the monitor work area).
- Theme/font/mode changes are saved into `settings` when editing is enabled;
  read-only sessions preview them locally.
- Native dialogs (`rfd`) are parented to the main window. Keep that behaviour.

### Themes

Themes are plain CSS: an accent pair on the `html[data-family=…]` rules, a
light/dark surface palette, then optional per-theme decoration. The DOM root
gets `data-family="<theme id>"` and `data-mode="light|dark"`. No JS theming
library is involved.

- 14 themes: Glass (default), Cyberpunk, Genshin, ZZZ, Azure, Mint, Indigo,
  Nord, Slate, Dusk, Sangria, Claude, Liquid, Pink.
- Older themes were removed (Teal/`default`, Aurora, Sunset, Lavender, Gold,
  Forest, Emerald, Crimson, Rose; Forest and Emerald → Liquid, Crimson →
  Sangria, Rose → Pink). Old saved ids are remapped by `OLD_MAP` in
  `ui/index.html`, and `normalizeSettings()` falls back to `glass` for anything
  unknown, so existing `inventory.json` files keep loading.
- Liquid replaces the old Emerald palette and follows Apple's Human Interface
  Guidelines materials/color foundations: SF Pro, regular-variant blur and
  vibrancy, one system-blue accent for emphasis, specular rims and inset
  highlights, larger continuous radii, and lighter glass on content cards than
  on the sidebar, header, modals and popovers. It defaults to dark when picked.
- `FAMILIES` (id, label, accent, optional custom `swatch` gradient and
  `defaultMode`) drives the Settings theme grid. Add new themes to both
  `FAMILIES` and the CSS. Picking a theme with `defaultMode` (Cyberpunk,
  Genshin, ZZZ and Liquid use `dark`) also switches the mode and sets
  `settings.modeChosen`.
- Cyberpunk and Genshin were rebuilt as direct ports of the artstyle in
  `kanban-rust-handoff.zip` (the Kanban reference app) on 2026-10-27.
- Cyberpunk is the Kanban "Night City edge HUD": Rajdhani body/headings and
  Share Tech Mono read-outs, black-cherry glass panels, crimson rails,
  scanlines, cut corners, a cyan scan sweep and cyan primary actions. The old
  dotted corner/edge HUD frame was removed at the user's request. Light mode
  uses the reference's rose-paper alternate palette (no black band).
  Cyberpunk and Genshin are dark by default: selecting either theme applies
  the dark palette. A pre-existing save with either theme forced to light and
  no `settings.modeChosen` marker is migrated to dark once; after that the
  light/dark toggle still lets the user choose and persist light.
- Genshin is the Kanban quest-menu UI: slate panels, cream hover/selected rows
  with dark text, gold diamonds, pill controls, and a random starfield with
  drifting glints over a painterly sky, using HYWenHei + Alegreya Sans. Light mode
  uses the reference's paper palette with dark-slate selected rows and a
  **daily random elemental theme** (geo/anemo/electro/dendro/hydro/pyro/cryo,
  stable for the local day) that tints the sky, panels, buttons, nav and
  diamonds. The light skies are saturated pastel gradients (`--g-sky-1..3`,
  rebuilt per element) with only an 8% white wash, so the daily colour reads
  clearly behind the cream panels. Hovering an item card switches every child
  text style to the cream-row palette so labels, lots, dates and quick-logout
  stay readable.
- ZZZ is a port of the Zenless UI component language
  (`https://github.com/ChrisChan13/zenless-ui` and
  `https://chrischan13.github.io/zenless-ui/#/component/backtop`): black glossy
  panels, hard black outlines, `#008bff` primary controls, checkerboard button
  fills, 3 px pill inputs, hazard-striped background and 20 px/zero-corner
  cards and modals. It uses the normal user Typeface setting (unlike
  Cyberpunk/Genshin). The wordmark is `assets/zzz-logo.png`, embedded in the exe
  and served at `/theme/zzz-logo.png` for the Settings swatch. ZZZ light mode
  uses a darker muted/accent/cyan ramp and explicit dark-teal clock time
  (`#06566c`) and date (`#3f444b`) colours so the top bar stays readable on its
  pale header.
- The `gi.zip` cursor set (public domain) is embedded from `rust\cursors\`
  and applied only under `data-family=genshin`: `gi_arrow.cur` for the page,
  `gi_link.cur` for links/controls/text/drag targets, `gi_busy.cur` for disabled
  buttons, `gi_working.cur` while the app is loading/working. Arrow/link
  hotspots are **0 0** (the tip), not the centre. A late `html[data-family=genshin] *`
  arrow rule is the fallback that covers every element and beats base
  `cursor:pointer/text/move` rules; the following `:is(...)` rules restore the
  link cursor for interactive targets. Keep that ordering. Native window-resize
  handles keep their resize cursors as the one deliberate OS affordance.
- Genshin buttons have no leading diamond. Standard/primary actions are
  off-white/ivory with dark text and switch to the daily elemental deep colour
  only on hover. Disabled buttons and `.readOnly .write` controls are fully
  greyed (grey fill, muted text, `grayscale(1)`, no shadow) and use the
  `gi_busy` cursor; no hover/active transform may leak through.
- The reference fonts live in `rust\fonts\` and are embedded in the exe; the
  original `zh-cn.ttf` remains the Genshin face. Theme typefaces override the
  user Typeface setting for these two themes only.
- Genshin's font is `rust\zh-cn.ttf` (family `HYWenHei-85W`, 7.2 MB). It is
  embedded into the exe with `include_bytes!` in `src/main.rs` and served at
  `/fonts/zh-cn.ttf`; the `@font-face` rule in `ui/index.html` aliases it as
  `"Genshin UI"`. The file must stay at `rust\zh-cn.ttf` or the build fails.
  It is included in the AI bundle ZIP so the source still compiles.
- Both reworked themes override the user Typeface setting on purpose: their
  letterforms are part of the look. The stored preference is untouched, so
  other themes still honour it.
- Do not add ambient corner glows or radial colour washes to any theme; the
  shared `body` background is flat `var(--bg)`. Textures (Glass grain,
  Cyberpunk scanlines/rails, the Genshin starfield) are allowed.

## 11. Build, test and deployment

From `rust\`:

```bat
cargo check
cargo test
cargo build --release
BUILD.cmd
TRANSFER-TO-USB.cmd
powershell -NoProfile -ExecutionPolicy Bypass -File .\build.ps1 -NoZip
```

`BUILD.cmd` runs `build.ps1`, which:

1. creates a release timestamp (`yyyy-MM-dd_HH-mm-ss`, local date/time) and
   passes it to the build as `INVENTORY_RELEASE_VERSION`; there is no numbered
   release version;
2. runs the release build;
3. copies the binary to `rust\Inventory.exe`;
4. writes `rust\version.txt` (the release timestamp);
5. creates `rust\data\` only if missing (never clears existing data);
6. removes the obsolete `rust\dist\` folder;
7. creates `rust\Inventory-<date-time>.zip` with the exe, version marker,
   launchers, docs and an empty `data\` entry. Older release zips are removed.
   `-NoZip` skips this.

There is no `dist\` staging folder anymore. The exe is deployed at the root of
`rust\`.

Portable use: copy `Inventory.exe` anywhere writable and run it; data is created
beside it.

### USB transfer

`TRANSFER-TO-USB.cmd` / `transfer-to-usb.ps1` copy only the deployed,
offline-ready application to a USB drive:

1. rebuilds the deployed exe with `build.ps1 -NoZip` (`-NoBuild` skips this);
2. lists removable drives (`Win32_LogicalDisk DriveType=2`) and prompts for one;
3. creates `Inventory-<date-time>\` at the drive root;
4. copies only `Inventory.exe` (everything else is embedded or created at
   runtime; no README, version.txt or shortcut script);
5. verifies the copied exe with SHA-256.

`Create-Direct-Shortcut.cmd` creates `Desktop\Inventory.lnk` with
`TargetPath` set directly to the local `Inventory.exe`, `WorkingDirectory` set
to its folder, and `IconLocation` set to `Inventory.exe,0`, so the shortcut
shows the embedded cat icon. It uses GOTO labels rather than parenthesised IF
blocks because deployment paths such as `Inventory (Rust)` contain `)`, which
cmd.exe would otherwise treat as a block terminator. It also calls
`ie4uinit.exe -show` after saving to refresh Explorer's icon cache.

The target folder is self-contained and portable: it creates fresh inventory
data beside the exe. Live `inventory.json`, `AutoBackups\`, source, `target\`
and the deployment zip are never copied. `-DestinationRoot <path>` bypasses the
drive prompt (used for automated tests); when no removable drive is detected
the script offers a manual path or cancel. The destination is refused if it is
inside the `rust\` project folder.

Shared use: set `INVENTORY_DATA_DIR` to a writable folder and launch the exe.
`SETUP-LAUNCHER.md` and `launcher\` implement an optional local-cache launcher
for a network share. The PowerShell launcher is the primary path; the batch
launcher also supports an offline portable fallback. They compare `version.txt`
and sync with Robocopy, excluding `data\`, `launcher\`, `inventory.json` and
`inventory.lock.json`.

### AI handoff bundle

`MAKE-AI-BUNDLE.cmd` / `make-ai-bundle.ps1` generate two small files in
`ai-bundle\` for handing the project to a fresh AI:

- `Inventory-AI-Bundle.zip`: this handoff, text source, icons, `Cargo.lock`,
  `zh-cn.ttf` (the embedded Genshin font) and the sample workbook;
- `Inventory-AI-Context.md`: a single self-contained Markdown with the handoff
  and all text source files (binary files such as the font are omitted to keep
  the upload small; the ZIP is required for a buildable copy).

The script runs `cargo test`, records the result, and writes a SHA-256 manifest.
It never includes `target\`, `Inventory.exe`, the deployment zip, inventory
data or the legacy Electron folder. Run it again after every development
session; update the session note in this file first.

## 12. Current state and recent changes

As of the last update (2026-10-28, release identified by build date/time):

- Repository split into `rust\` (current) and `electron\` (legacy reference).
- Deployment output moved from `rust\dist\Inventory\` to `rust\Inventory.exe`;
  the obsolete `dist\` folder is removed by the build.
- Release exe is a GUI-subsystem binary (no console window).
- Launchers/shortcut creators default to the app folder `Inventory`.
- USB transfer added: `TRANSFER-TO-USB.cmd` asks which removable drive to use
  and copies only `Inventory.exe`, `version.txt` and a generated `README.txt`
  into `Inventory-<date-time>\`, with a SHA-256 check.
- Dashboard charts now include 1-month usage (default on), opt-in 6-month usage
  (default off), and a stock-level hover crosshair with date/on-hand/last
  movement details.
- The native title bar was replaced by a borderless window with custom
  minimize/maximize/close buttons, a draggable header and edge/corner resize
  handles. The app still starts maximized. Every theme uses the same flush
  54 px controls at the top-right: full header height, 14 px line icons.
- Dashboard chart toggles sit between the stock cards and the charts. Cyberpunk
  and Genshin have Kanban-style motion, with a per-device Motion override in
  Settings → Theme for systems that enable reduced motion. Cyberpunk's dotted
  HUD frame was removed; Genshin uses a random JS starfield with drifting glints
  that re-randomize after every drift. The starfield no longer follows the
  cursor, and motion pauses when the window is unfocused to keep high graphics
  responsive.
- Settings → Theme also has a per-device **Graphics Low/High** control. Low
  removes the starfield/scanline/grain overlays and disables blur, shadows,
  transitions and animations while keeping the theme colours.
- Releases are identified only by the local build date/time
  (`yyyy-MM-dd_HH-mm-ss`) in `version.txt`, the exe properties and the package
  name; there is no numbered release version.
- Genshin light mode picks a stable daily elemental palette (seven colourways);
  the Genshin cursor set is embedded and used only for that theme. Genshin's
  hidden `#edit` button is fixed (`.hide!important`) and hover/pressed states
  were added to buttons, tabs and chart tokens.
- Automatic-backup folder is configurable in Settings → Data & Backup
  (`settings.backupDir`), with choose/open-folder actions; the default remains
  `AutoBackups` beside the data file.
- Verified in the current build: release build, 2 unit tests, portable run with data
  beside the exe, shared `INVENTORY_DATA_DIR` run, package/zip extraction and
  launch, graceful close/auto-backup/lock removal, edit-lock contention and
  force takeover, UI scale resize, XLSM import bridge, watcher reload, backup
  and restore dialogs, custom window minimize/restore/maximize, configured
  backup folder creation and close-time backup, stock-chart hover, and a USB
  transfer whose copied exe launched from the timestamped folder and created
  its data beside itself.
- Theme set reworked: Teal, Aurora, Sunset, Lavender and Gold were removed;
  Glass is now the default and the migration fallback. Cyberpunk and Genshin
  were rebuilt from the Kanban reference artstyle on 2026-10-27; Rajdhani,
  Share Tech Mono and Alegreya Sans are embedded beside the HYWenHei font.
  Forest and Crimson were removed later and ZZZ added; Rose was removed after
  that, leaving 14 themes.
- Verified in the current build: release build, 2 unit tests, portable run with data
  beside the exe, shared `INVENTORY_DATA_DIR` run, package/zip extraction and
  launch, graceful close/auto-backup/lock removal, edit-lock contention and
  force takeover, UI scale resize, XLSM import bridge, watcher reload, backup
  and restore dialogs, and a USB transfer whose copied exe launched from the
  timestamped folder and created its data beside itself.
- Known intentional difference: the Rust XLSM reader differs from SheetJS for
  the pre-1901 placeholder/date artifact; real dates match Excel. Do not “fix”
  this by reintroducing SheetJS.
- No known open bugs at handoff. The next requested feature or bug fix belongs
  in the session note below.

## 13. Working agreement for the next AI

- Do not assume you can run code or read files not supplied. If something is
  missing, ask for that specific file rather than guessing.
- Prefer minimal, targeted changes over rewrites. Preserve all existing
  features and UI text unless explicitly asked.
- When returning code, give complete filenames/paths (for example
  `rust/src/state.rs`) and either full files or precise unified diffs. State
  which files changed.
- If a change affects the data format, lock file, API shape or deployment
  layout, call it out explicitly and provide a migration path.
- If adding a Rust dependency, update `Cargo.toml` and explain why; run
  `cargo check`/`cargo test` locally when possible.
- Always give the user a short verification checklist (commands and expected
  result) for changes you propose.
- At the end of a session, write a concise handoff block that the user can
  paste into the “Session note” section below.

## 14. Session note (edit this before generating the next bundle)

Keep this section short and current. It is the only part that should change
every session.

- **Date:** 2026-11-04
- **Focus / requested change:** Replace the Emerald theme with a new **Liquid**
  theme based on Apple's Human Interface Guidelines materials and foundations;
  later add a portable direct-exe desktop shortcut script and include it in the
  USB transfer; fix the cat icon missing from the running app's taskbar on
  other computers.
- **What was completed:** Removed Emerald from `FAMILIES` and added Liquid
  (default dark, blue/cyan swatch, SF Pro on selection). Replaced the Emerald
  CSS variables with Apple-inspired light/dark palettes and added a Liquid
  surface system: ambient blue/violet/cyan content-layer gradients, regular
  blur + vibrancy, lighter glass cards, stronger glass sidebar/header/modals,
  specular rims and inset highlights, pill buttons/segments, 12 px glass inputs
  and 24-26 px continuous radii. Forest/Emerald/contrast saved ids now migrate
  to Liquid; `normalizeSettings()` treats Liquid as a dark-default theme.
  Incidental fixes found during QA: the brand wordmark no longer truncates
  (removed excess letter spacing) and the Settings global-scope chooser stacks
  above the date fields below 1500 px so four buttons do not overlap their text.
- **Follow-up:** Replaced the text glyph in every dialog close button with a
  symmetric inline SVG cross and added
  `.hd .btn[aria-label="Close"]{justify-content:center}`. Fixed-size close
  buttons (chart details and All items) were previously left-aligned inside
  their 32 px box because `.btn` centres vertically only. The X now measures
  within 0.01 px of the button centre in every dialog.
- **Follow-up:** On the Dashboard, 1-month usage and Stock cover now keep their
  title on the left and their search/filter/detail controls on the same row,
  right-aligned to the chart edge. The monthly search flexes between 76 and
  44 px, the period select uses short `3/6/12 mo` labels, and Stock cover uses
  compact `Months`/`Least` labels. Verified at 1200 and 1366 widths with no
  overflow or title/control collision.
- **Follow-up:** Added an independent search box to the 1-month usage detailed
  view. `monthlyRows(qArg)` accepts a query override, and the detailed view
  filters on `statMonthlyQuery` while ignoring the dashboard `monthlyQuery`;
  the KPI totals/count and empty-state text follow the filtered rows. Focus and
  caret are restored after the debounced re-render.
- **Follow-up:** Added `Create-Direct-Shortcut.cmd`. It writes
  `Desktop\Inventory.lnk` with the local `Inventory.exe` as the target, its
  folder as the working directory and `Inventory.exe,0` as the icon, so the cat
  icon is used. It avoids parenthesised IF blocks (the tested path contains
  `(Rust)`) and refreshes Explorer icons with `ie4uinit.exe -show`.
  `transfer-to-usb.ps1` now copies the script into every USB release folder and
  its generated README documents it. Verified by running the script from the
  parentheses path, reading back the `.lnk` target/icon, and doing a
  `-NoBuild` transfer to a temp destination.
- **Follow-up:** Added `.with_taskbar_icon(load_icon())` alongside
  `.with_window_icon`. Tao's `with_window_icon` only calls
  `WM_SETICON(ICON_SMALL)`; Windows taskbars use `ICON_BIG`, so other PCs could
  keep a cached/generic taskbar icon even though the desktop shortcut and small
  title-bar icon were correct. QA read `WM_GETICON` back from the running
  window: big, small and small2 handles are all non-null and decode to the cat
  PNG.
- **Files changed:** `src/main.rs`, `ui/index.html`,
  `Create-Direct-Shortcut.cmd`, `transfer-to-usb.ps1`, `TRANSFER-TO-USB.cmd`,
  `make-ai-bundle.ps1`, `README.md`, `AI-HANDOFF.md`.
- **Decisions / trade-offs:** Liquid uses one blue accent only for emphasis
  (Apple color guidance), keeps content cards lighter than navigation/popovers
  (materials guidance), and remains CSS-only so no new assets or JS theming
  library are needed. The settings scope layout is now single-column below
  1500 px; on wider screens it keeps the previous side-by-side placement.
- **Known issues:** None known. QA at 1366x768 in dark and light: glass cards,
  header, sidebar, item modal, settings grid and all controls rendered with no
  JavaScript exceptions. A saved Emerald theme migrated to Liquid with its mode
  and font preserved, and all 199 items loaded. The theme grid shows 14 themes
  with Liquid replacing Emerald. Live `inventory.json` was not written by QA.
- **Next step:** Ask the user for the next feature/bug fix, then update this
  note and regenerate the bundle with `MAKE-AI-BUNDLE.cmd`.
