# Inventory

A portable laboratory inventory application for Windows. The complete desktop
host is written in Rust; the original HTML interface runs inside the Microsoft
Edge WebView2 control. There is no Electron, no Node.js runtime and no console
window.

Current version: **1.0.2** (see [Releases](https://github.com/meowyourhead/inventory-tracker/releases)).

## Quick start

1. Download `Inventory.exe` from the
   [latest release](https://github.com/meowyourhead/inventory-tracker/releases/latest).
2. Put it in any writable folder (a local folder or a USB stick) and
   double-click it.
3. The app creates its data beside the executable and starts maximized.

No installer and no configuration file are required. For a desktop shortcut,
right-click `Inventory.exe` → **Send to → Desktop (create shortcut)**.

```
E:\Inventory\
├── Inventory.exe
├── inventory.json            created on first launch
├── inventory.lock.json       exists only while someone is editing
└── AutoBackups\              created after the first saved change
```

Keep the folder writable. If the stick is write-protected the app cannot create
`inventory.json` and shows an error dialog.

`AutoBackups\` is the default automatic-backup folder. In **Settings → Data &
Backup** you can choose a different folder (for example a local drive, a USB
disk or a shared path) and open it in Explorer. The choice is stored in the
inventory settings, so it is shared by every client that opens the same
`inventory.json`; use a path every client can reach.

## Updates

Inventory updates itself from this repository's GitHub Releases.
**Settings → Updates** shows the installed version, build date, latest
version and when it last checked, with a **Check for updates** button.

- With **Install automatically** on (the default, stored per computer), the
  app checks about 4 seconds after start-up and every 6 hours.
- When a newer release exists it downloads `Inventory.exe` with a progress
  bar, verifies its size and SHA-256 checksum, replaces the running exe in
  place (via the `self-replace` crate), then restarts into the new version.
  The usual close steps run first: automatic backup and lock release.
- Only the exe is replaced. `inventory.json`, `AutoBackups\` and
  `inventory.config.json` beside it are never touched.
- The exe's folder must be writable for an update to install. A computer
  without internet access keeps working; the check just reports that GitHub
  is unreachable.

### Publishing a new version

1. Bump `version` in `Cargo.toml` (for example `1.1.0`).
2. `cargo build --release`
3. Create a GitHub release whose tag matches the version (`v1.1.0`) and attach
   `target\release\Inventory.exe` under exactly that name. GitHub records the
   asset's SHA-256 automatically; also attaching an `Inventory.exe.sha256`
   file is optional.

```bat
gh release create v1.1.0 target\release\Inventory.exe --title "Inventory 1.1.0" --notes "What changed"
```

Running clients pick the release up on their next check.

## Requirements

- Windows 10 or Windows 11, 64-bit.
- Microsoft Edge WebView2 Runtime (Evergreen). It is already present on current
  Windows 10/11 installations and on machines with Microsoft Edge. If it is
  missing, the app shows a clear message; install the runtime from
  <https://developer.microsoft.com/microsoft-edge/webview2/> and start it again.
- The first start on a machine creates the WebView2 profile in
  `%LOCALAPPDATA%\Inventory\WebView2`. Inventory data itself stays beside the
  executable.

## Features

- Item catalogue, stock entries, lots, expiry dates and stock movement history.
- Catalog and history rows open their edit dialog when clicked. Per-row edit
  buttons were removed; undo, restore and remove actions remain as buttons.
- Count tab: rows start blank; type the shelf quantity or press ✓ when it
  matches the system. Find or scan an item to jump to its row, filter by
  category and show All / Not counted / Differences. The count in progress is
  kept on this computer until **Review & apply** (which lists every change
  first) or **Discard count**. Printable count sheets and a collapsible
  **Past stock counts** history.
- Dashboard card view has a **Sort** menu (name, low stock first, least/most
  stock, expiring soonest, category). It shares the table's sort order.
- Scan/Catalog tab: choosing an item shows its open lots (earliest expiry
  first); click one to fill Lot and Expiry. Quantity has −/+ steppers and
  "By" shows who is recorded when left blank.
- Dashboard table view supports checkbox selection, a **Batch delete** button
  beside the search field, and a compact single-line layout with **Balance**
  after **Status**. Dashboard cards use dense spacing and compact quantity
  bars. Printing always outputs the compact reagent table, even from card
  view, with the rest of the interface removed.
- Catalog **Batch edit** applies category/colour, shipment task, pack, par
  level, lead time and no-expiry status to every item matching the current
  search/filter, including pages not shown, with undo.
- FEFO/FIFO quick log-out, par levels and expiry warnings.
- Catalog items can be marked **No expiry**; their log-in form defaults to
  31 Dec 9999.
- Every popup dialog closes with **Esc** without saving or applying changes.
- Settings for theme, light/dark mode, font and font size. 14 themes are
  included: Glass (default), Cyberpunk, Genshin, ZZZ, Azure, Mint, Indigo,
  Nord, Slate, Dusk, Sangria, Claude, Liquid and Pink. Themes saved by
  older versions (Teal, Aurora, Sunset, Lavender, Gold, Forest, Emerald,
  Crimson, Rose) are mapped to a current theme automatically; Forest and
  Emerald now map to Liquid. Cyberpunk and Genshin are faithful
  ports of the Kanban reference artstyle: Rajdhani + Share Tech Mono with
  crimson/cyan HUD styling, and HYWenHei + Alegreya Sans with slate panels,
  cream selection rows and gold diamonds. Both override the Typeface setting;
  all other themes honour it. ZZZ is a port of the Zenless UI component kit
  (ChrisChan13/zenless-ui): glossy black panels, hard black outlines, blue
  primary controls, checkerboard button fills, pill inputs, hazard striping and
  the ZZZ wordmark, based on
  [ChrisChan13/zenless-ui](https://github.com/ChrisChan13/zenless-ui) and its
  [component docs](https://chrischan13.github.io/zenless-ui/). Liquid follows
  Apple's [Human Interface Guidelines materials](https://developer.apple.com/design/human-interface-guidelines/materials):
  SF Pro, regular-variant blur and vibrancy, a single system-blue accent,
  specular glass rims and floating functional layers over a colourful content
  layer, with darker, thicker glass for sidebars and popovers. Cyberpunk,
  Genshin, ZZZ and Liquid open in dark mode by default when selected; the mode
  toggle still switches them to light.
- Display scale toggle (90 / 100 / 120 / 140%) that resizes and centres the
  window on the current monitor.
- Cross-machine edit lock: one editor at a time, everyone else is read-only.
  The lock heartbeat goes stale after 30 seconds of inactivity so another user
  can take over. A silent idle timer shows a "Still editing?" reminder after
  15 minutes without activity and releases editing 15 minutes later unless the
  user chooses Keep editing; both periods are set in Settings ▸ Editing timeout.
- Dashboard charts: 6-month usage by item, a 1-month usage estimate per item
  (3/6/12-month average), expiry, top consumed, stock cover and stock level,
  with instant hover details and expandable detailed views.
- Dashboard fits one page: cards occupy a scrolling top pane (~55%) and the
  compact four-chart grid occupies the bottom (~45%). 1-month usage and Stock
  cover become two narrow, full-height columns; Expiry status and Stock level
  keep their original half-width, half-height positions stacked on the right.
  The dashboard search box is centred and the chart row sits slightly lower
  with extra breathing room under the cards. 1-month usage and Stock cover place
  their search/filter/detail controls on the same row as the title, aligned to
  the chart's right edge. Low-stock cards are tinted red instead of carrying a
  separate Low stock label. Dashboard cards keep a small top gap so the hover
  lift never clips the first row.
- **Open all items in a large window** opens a wide, fixed-height All items
  window. Its title, close button and a search / category filter bar stay
  pinned while the card grid or table scrolls with a visible themed scrollbar;
  a live count shows how many items match. Opening a reagent here and closing
  it returns to the same window, filters and scroll position.
- A **Charts** tab below **History** shows every chart at full-page size in a
  2×3 grid. Dashboard keeps 1-month usage, expiry, stock cover and stock level;
  6-month usage and top consumed live in Charts.
- Charts include 6-month usage, a 1-month usage estimate, expiry, top consumed,
  stock cover and stock level. The stock-level graph has a hover crosshair with
  the date, on-hand units and the last movement; all charts have expandable
  detailed views. Detailed views open in a wide, fixed-height window with the
  title, close button and controls pinned above a scrolling table. The
  1-month usage detailed view has its own item search, independent of the
  dashboard search. The Stock level detailed view is sized to its chart
  instead of filling the screen.
  **1-month usage**, **Top consumed** and **Stock cover** list only the rows
  that fit and show a `+N more…` label; Stock cover uses the same treatment.
  The **1-month usage** search box sits immediately left of its averaging
  period dropdown. **Stock level** integrates item search into its selector:
  click the selector to open a searchable item list. Stock cover and Stock
  level are sized to the same height as the 1-month usage panel.
- Custom borderless window chrome with minimize, maximize/restore and close
  buttons, plus edge/corner resizing. The title bar is a compact 42 px strip
  (no greeting or clock) with 30 px controls; every theme uses the same
  full-height 44 px window buttons flush to the top-right corner. History
  times, lock/backup times and count-sheet timestamps use a 12-hour clock with
  AM/PM.
- Cyberpunk/Genshin motion (HUD sweep/flicker, drifting Genshin glints) with a
  **Motion override** switch in Settings → Theme for systems that enable
  reduced motion. The Genshin background no longer follows the cursor, Genshin
  glints choose a new random position and path after every drift, and theme
  animation pauses while the window is unfocused.
- **Graphics** selector in Settings → Theme with **Low** and **High** quality.
  Low turns off decorative overlays, backdrop blur, shadows and theme animation
  for slower computers; the choice is stored per computer.
- Genshin uses the `gi.zip` embedded cursor set and picks one of seven elemental
  colourways each day in light mode. All ordinary UI targets use a Genshin
  cursor, including text fields, clickable cards, chart hover areas and drag
  regions; only native window-resize edges keep resize cursors. Its action
  buttons stay off-white until hovered, then take the daily elemental colour;
  disabled controls are fully greyed. Primary buttons no longer carry a
  decorative diamond.
- Cyberpunk, Genshin and ZZZ show their wordmarks in the Settings theme grid.
- The Scan tab is sized to the current window. The catalog/history cards fill
  the remaining height, keep their page numbers pinned at the bottom, and fit
  their rows without inner scrolling; the Scan form compacts slightly on
  shorter screens.
- Native page scrollbars stay hidden on every tab. The Dashboard card pane has
  its own visible 9 px scrollbar; a large **Top** button sits at the bottom of
  that pane, just left of the scrollbar, and appears only when the cards are
  scrolled.
- Dashboard chart toggle tokens were removed; all four dashboard charts and all
  six Charts-tab charts are always shown. Global data scope applies to Dashboard
  history, Scan history, usage and stock-level charts; in-stock items remain
  visible.
- Configurable automatic-backup folder with **Choose folder** and **Open
  folder** actions in Settings.
- JSON backup and restore with native Windows file dialogs.
- XLSM/XLSX restoration with preview, merge/replace modes, validation and an
  automatic JSON backup before import. Ledger workbooks with placeholder
  catalog numbers (`-`, `N/A`) key items by name, so glove sizes and similar
  per-sheet items remain separate. Alinity IMS workbooks read the Master List
  catalog, item, pack, balance, expiry and par level, then take movements and
  balances from each reagent tab (catalog in B1, expiry/lot/item/log-in/log-out
  rows below) because the tab data is more current than the Master List.
- Live reload when `inventory.json` changes on disk (for example from another
  client on a shared folder).
- Automatic backups of saved changes on normal exit, newest 30 kept, in the
  configured backup folder.
- Delete-all action for clearing the data set, guarded by the edit lock and a
  typed `DELETE` confirmation. Restore asks for confirmation first.
- Interface scale and window sizing preserved from the original application.

## Where data lives

| Mode | Data folder | How to enable |
|---|---|---|
| Portable (default) | The folder containing `Inventory.exe` | Just run the exe |
| Shared folder | `dataDir` from `inventory.config.json` beside the exe | Create the config file (below) |
| Shared folder | The folder named by `INVENTORY_DATA_DIR` | Set the variable before launch (overrides the config file) |

To share one dataset between several PCs, install `Inventory.exe` locally on
each PC and put this `inventory.config.json` beside it:

```json
{ "dataDir": "\\\\server\\share\\Inventory\\data" }
```

Each PC then updates its own exe from GitHub, while all of them use the same
`inventory.json`, lock file and `AutoBackups` folder. In shared-folder mode the
folder is created if needed and must be writable. The app fails loudly instead
of starting a diverging local copy when the share is unreachable or read-only.
The folder must allow create, rename and delete. Keep client clocks
synchronised, because the edit lock heartbeat is timestamp-based.

## Upgrading from the Electron version

The data format is unchanged. Choose one of these:

- Copy the existing `inventory.json` and `AutoBackups\` folder next to the new
  `Inventory.exe`, or
- Point `INVENTORY_DATA_DIR` at the folder that already contains them.

Close the old app first so no stale lock file remains. If you only copy
`inventory.json`, a new empty `AutoBackups\` folder is created after the first
saved change.

## Building from source

Prerequisites:

- Rust stable 1.88 or newer (`rustup`).
- Visual Studio Build Tools with the Desktop development with C++ workload,
  including the Windows SDK.

```bat
cargo test
cargo build --release
```

The release executable is `target\release\Inventory.exe`. The UI, icons,
fonts and cursors are embedded, so that single file is the whole app. Its
version comes from `Cargo.toml`. To publish it, see
[Publishing a new version](#publishing-a-new-version).

Development builds (`cargo run`) can check for updates but never install one.

## Project layout

| Path | Purpose |
|---|---|
| `src/main.rs` | Native window, WebView2 host, custom protocol, API dispatch |
| `src/state.rs` | Data cache, atomic saves, edit lock, auto-backups, file watcher, update orchestration |
| `src/update.rs` | GitHub Releases update channel: version check, verified download, self-replace |
| `src/xlsm.rs` | XLSM/XLSX parser (calamine); dates emitted as `YYYY-MM-DD` |
| `ui/index.html` | The whole user interface plus the `window.desk` bridge (embedded in the exe) |
| `build.rs` | Embeds the icon, version and build date into the executable |
| `AI-HANDOFF.md` | Durable context for a new AI developer |

Inventory data (`inventory.json`, `AutoBackups\`), workbooks (`*.xlsm`) and
built executables are git-ignored and never committed. The parser tests build
their own synthetic workbook.

## Notes

- Backups, restore and XLSM import use native dialogs owned by the main window.
- The app writes the JSON file atomically, so a crash or network interruption
  cannot leave a half-written `inventory.json`.
- A restore validates the file as inventory JSON before replacing the current
  data. A UTF-8 BOM causes the same rejection as in the Electron version.
- Inventory never writes to the WebView2 profile folder for data; deleting
  `%LOCALAPPDATA%\Inventory\WebView2` only clears the browser cache and is safe.
