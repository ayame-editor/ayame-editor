# Changelog

All notable changes to Ayame Editor are tracked here.

## Unreleased

- Added "grep して保存" (Tools menu, `/api/grep/save`, `ayame grep-lines`):
  extract only the lines matching a pattern — with the search bar's exact
  regex / case / whole-word semantics, unsaved edits included — into a file
  picked with the save dialog. Streams through an isolated worker with the
  same `--jobs` / `--chunk-lines` parallelism as replace, so multi-GB files
  complete in bounded memory. (#38)
- Added dirty-tab handoff between native windows: dragging an unsaved tab to
  another Ayame window (or out into a new one) now moves the unsaved edits
  with it. The source window detaches the tab keeping its fsynced crash log
  (`/api/tabs/detach`), and the adopting window replays that log silently —
  no crash prompt, no data loss. Untitled buffers still stay put. (#35)
- Added per-column graduations to the column ruler: a short tick every
  column and a taller one every 5, sakura-editor style, drawn with repeating
  CSS gradients so no DOM node exists per column. (#43)
- Fixed the window/taskbar icon looking vertically stretched: the flower
  mark is widened to a near-square silhouette (like the favicon) and reads
  bolder at 16x16 titlebar size; its aspect ratio is now unit-tested. (#51)
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
- Fixed a case-insensitive literal replace expanding `$` in the replacement as
  capture-group syntax, so a literal `$10` silently became empty; non-regex
  replaces now emit the replacement verbatim. (#67)
- Fixed `diff --side-by-side` panicking on CJK / multibyte lines longer than the
  column width (byte-index truncation split a character). (#69)
- Fixed the default keymap giving Alt+W to both タブを閉じる and the whole-word
  search toggle — Alt+W closed the tab and the toggle was unreachable; Alt+W is
  now the search toggle only. (#70)
- Fixed the "Match Case" find-bar button being inverted: it toggled the
  ignore-case flag, so its lit state and label disagreed. It now lights up when
  case is actually matched. (#70)
- Fixed the final session snapshot being lost on page unload (a plain fetch was
  aborted); it is now flushed with `sendBeacon`. (#73)
- Fixed the two-file diff summary line ("N hunk / … hunk omitted") being
  hardcoded English; it now uses the i18n tables. (#74)
- Fixed uploaded / sort-result scratch tabs being written into the restorable
  session and failing to reopen, by using the authoritative scratch-path
  detection. (#75)
- Fixed search on UTF-16 files: literal matches at misaligned (odd) byte offsets
  are rejected, and case-insensitive / regex queries decode each line first
  instead of scanning interleaved-NUL bytes and matching nothing. (#68)
- Fixed Backspace / Delete doing nothing on a zero-width rectangular (column)
  selection; they now delete one character per covered line and keep the column
  caret alive for repeats. (#74)
- Fixed long operations (Replace All, sort, grep, diff) not blocking editor
  input: the busy overlay now counts as a modal, so typing or IME input can no
  longer be spliced into a running operation and invalidate it. (#72)
- Fixed the Enter that confirms a Japanese (IME) conversion also inserting a
  stray newline in the WebKit / WKWebView build (Safari and the macOS app): that
  post-composition Enter is now swallowed. (#71)
- Fixed F3 / Shift+F3 resuming from a stale byte anchor after an edit shifted the
  text, which could skip a match or land mid-character; search anchors are now
  dropped on every edit so the next step re-anchors from the caret. (#74)
- Fixed opening a file wiping the server-stored search history / session when the
  initial UI-state load had failed: a partial write now re-reads the current
  state first and skips the write rather than overwriting it with empties. (#73)
- Fixed find / search / two-file diff on a buffer with unsaved edits re-running
  encoding auto-detection instead of honoring an encoding the user had chosen
  with "reopen with encoding", so Japanese queries could stop matching after an
  edit; the dirty snapshot now opens under the live document's encoding. (#75)
- Fixed tail-follow (`tail -f`) re-scanning the whole file and writing a fresh
  index-cache blob on every poll of a growing file, leaving an unbounded trail of
  dead cache blobs on disk; growth is now followed without the index cache. (#76)

Release artifacts are published from GitHub Actions and listed on the
[releases page](https://github.com/hjosugi/ayame-editor/releases).
