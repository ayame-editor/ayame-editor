# Changelog

All notable changes to Ayame Editor are tracked here.

## Unreleased

- Fixed new/untitled buffers silently saving into the temp scratch folder:
  the scratch-directory rename broke untitled detection, so 保存 overwrote
  `%TEMP%\ayame-srv-untitled-…\untitled.txt` without a dialog. Untitled
  buffers (and 名前を付けて保存) always go through a save dialog again, even
  when a previous save folder is remembered.
- Changed the first-run save/browse suggestion to the executable's folder
  instead of the temp scratch directory (前回の保存先 takes over once set).
- Added OS-native open/save dialogs in the desktop (gui) build via rfd; the
  browser build keeps the in-app picker.
- Added Windows drive navigation to the in-app picker and file tree: a
  virtual "PC" level lists all ready drives, reachable from every drive root.
- Moved case conversion from the 選択 menu to the ツール menu and added
  camelCase / PascalCase / snake_case / kebab-case / CONSTANT_CASE styles.
  Whole-file conversion (`ayame case`, `/api/case/save`) accepts the new
  styles too and can now run chunk-parallel like replace (`--jobs` /
  `--chunk-lines`).
- Fixed the caret and mouse hit-testing drifting away from the real insert
  position on lines containing tabs (e.g. TSV files): tab stops resolve
  relative to the row including the line-number gutter, and the measurement
  probe now replicates that geometry.
- Fixed whole-file upper/lower conversion corrupting non-ASCII characters in
  UTF-16 files; UTF-16 lines now convert through decode → transform → encode.
- Added session restore for open tabs, active tab persistence, and shared
  server-backed recent-file/search-history state across native windows.
- Expanded UTF-16LE/UTF-16BE support for opening, reopening, converting, saving,
  search indexing, and folder grep.
- Improved Replace All so large result sets are paged automatically and applied
  against the original match set instead of stopping after the first chunk.
- Expanded Settings visibility controls and macOS native menus, including
  dynamic language refresh for the native menu bar.
- Added lightweight visible-row syntax highlighting for common code, JSON,
  Markdown, YAML, SQL, shell, and log files, with a View/Settings toggle.
- Added safe tab drag support for native-window workflows: clean tabs can move
  to another Ayame tabbar or drag out into a new window, while dirty tabs are
  kept in place to avoid losing unsaved edits.
- Added a complete CLI reference for every public `ayame` subcommand.
- Added architecture documentation for the Rust core, local server, web UI,
  crash-recovery WAL, type generation, and release automation.
- Added contributor guidance covering CI gates, Japanese documentation sync, and
  screenshot refreshes.
- Added documentation screenshots and expanded shortcut coverage for GUI actions
  that are configurable but unassigned by default.

Release artifacts are published from GitHub Actions and listed on the
[releases page](https://github.com/hjosugi/ayame-editor/releases).
