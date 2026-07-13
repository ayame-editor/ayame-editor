# Changelog

All notable changes to Ayame Editor are tracked here.

## v0.7.3 - 2026-07-13

- Data ops now understand RFC-4180 records (#199): with `--csv`, a quoted
  field containing a newline keys as ONE record for sort/group/distinct/top
  instead of two broken "lines" — Excel/pandas exports with embedded newlines
  finally aggregate correctly. Multi-line records are merged by quote parity
  as zero-copy slices of the mmap, sort's ordering/offset artifacts are
  record-numbered (a sorted multi-line record moves byte-exactly as one unit),
  top selections carry their full byte range, and a stray unclosed quote is
  capped (4096 lines / 8 MiB per record) instead of fusing the rest of the
  file into a single record. Non-CSV mode is unchanged.

## v0.7.2 - 2026-07-13

- Fixed three encoding/EOL detection failures (#196): ASCII-heavy UTF-16
  without a BOM is now detected by NUL-byte parity instead of short-circuiting
  to UTF-8 (which rendered interleaved NULs and built garbage sort keys);
  ISO-2022-JP is a first-class encoding — detected by its JIS escape sequences
  (deliberately not by the `ESC ( B` designation that also appears in colored
  terminal logs), decoded, searchable (always through the decoded-text plan,
  since raw-byte scans both miss mid-run matches and false-positive on escape
  bytes), and available to convert-save and `--encoding`; and classic-Mac
  CR-only files finally split into lines — the line index can now terminate on
  lone `\r`, so `stat` no longer reports "1 line" next to "line ending CR",
  and sort/group/split/tail-follow see real records. Cached indexes remember
  their newline strategy, so an LF index cached by an older build cannot serve
  a CR-only file.

## v0.7.1 - 2026-07-13

- group-by aggregates are now deterministic and clean: `sum`/`avg` accumulate
  through an exact (correctly-rounded, order-independent) summation, so the
  result no longer changes with the spill budget or loses low bits past 2^53;
  non-finite value strings (`NaN`, `inf`, `1e999`…) no longer poison a group's
  aggregates; and `min`/`max` report nothing instead of leaking the internal
  `±inf` sentinels when a group has no numeric values. (#197)
- The bounded-memory contract now survives files with no (or absurdly distant)
  newlines: view APIs decode at most 4 MiB per line (`Line.truncated` /
  `EditLine.truncated` report the cut, `ayame line` and sorted outputs still
  decode in full), op keys for sort/group/top/distinct are built from at most
  the first 64 KiB of a field (longer fields fall back to sort's stable
  original-order tie-break), and a match's character column is computed with a
  streaming counter instead of decoding the whole line prefix. In-place edits
  of over-cap lines are refused with a clear error instead of silently
  truncating them, and selection export refuses truncated lines. (#201)
- sort/group no longer strand spill runs, partial `*.ordering.bin` /
  `*.lines.bin` artifacts, or their private spill directory when an operation
  fails or a callback panics mid-run — cleanup now runs in a drop guard
  covering every exit path. (#201)
- The editor no longer dies with an uncatchable `SIGBUS` when another process
  truncates (or rotates a shorter file over) a file it has memory-mapped. A
  process-wide fault absorber (`ayame-core::mapfault`) turns the fault into a
  sticky per-mapping flag; every read path — viewport, search, sort/group/
  distinct/top scans, grep's per-file maps, and the spill offset tables —
  now surfaces a clean "base file changed on disk" error (HTTP 409 in the web
  UI) and requires a reopen, instead of aborting the whole process. Save,
  split, sort and transform verify the base file before committing output, so
  bytes read through a shrunk mapping can never be laundered into a saved
  file. (#200)

## v0.7.0 - 2026-07-12

- **Breaking:** removed the `diff` / `sortdiff` implementations, `/api/diff`,
  the Web two-file diff view, native/menu commands, diff-only CSS and tests.
  Comparison now lives in [ayame-diff](https://github.com/hjosugi/ayame-diff):
  replace `ayame diff OLD NEW` with `ayame-diff text OLD NEW`, and replace
  `ayame sortdiff OLD NEW` with `ayame-diff sorted OLD NEW`. The old CLI names
  return an actionable migration error for this release. (#94–#104)
- Improved keyboard and assistive-technology support: search toggles expose
  their labels and pressed state, status controls announce their values, the
  command palette tracks its active option, all major controls have a visible
  theme-aware focus ring, and persistent motion stops when the OS requests
  reduced motion. (#156, #171, #183)
- Made each visual theme own its syntax palette, with genuinely monochrome
  highlighting in Mono Paper, and removed duplicate menu entries and the
  double-bound encoding conversion action. (#154, #181)

## v0.6.1 - 2026-07-12

- Stopped `/api/lines` viewport reads from deep-cloning up to 256 undo/redo
  generations on every scroll or edit refresh. Read snapshots now copy only the
  sparse overlay and required generation metadata while preserving identical
  rendered content. (#139)

## v0.6.0 - 2026-07-12

- Hardened `cargo xtask release`: workspace versions are now read and edited
  structurally, Cargo metadata is parsed as JSON, release phases are separated,
  and GitHub Actions polling is pinned to the exact release commit instead of
  whichever Release run happens to be newest. (#145)
- Unified Web UI typography, control radii, shadows, syntax colors, and diff
  colors behind semantic design tokens shared with ayame-diff. Dark and black
  themes now define their own accent/status colors, inline deletions/additions
  use distinct red/green word tokens, and the default runtime monospace stack
  retains Japanese CJK fallbacks. Added token-drift tests. (#131, #132, #133,
  #134, #135)
- Deprecated the `diff` and `sortdiff` subcommands (#94). They now print a
  notice pointing to the sister project
  [ayame-diff](https://github.com/hjosugi/ayame-diff), whose `text` / `sorted`
  subcommands supersede them; the editor's copy is frozen and slated for removal
  in a later release (see #93, #104). The notice goes to stderr (stdout output
  is unchanged) and can be silenced with `AYAME_NO_DEPRECATION`.

- Added standalone app self-update and removal commands: `ayame update`
  resolves GitHub Releases, verifies `.sha256`, installs the matching
  Linux/Windows/macOS artifact, and `ayame remove` removes standalone installs
  with confirmation / dry-run support. Managed installs from Nix, Homebrew, and
  Scoop are detected and left to their package manager instead.
- Added startup update checks in the native desktop app. After the window is
  shown, Ayame checks for a newer standalone release in the background, asks the
  user before installing, and can be disabled from Settings with 起動時に更新を確認
  / Check for updates on startup.
- Added Nix development shell files, Homebrew/Scoop packaging stubs, packaging
  documentation, and a redesigned MkDocs landing page.
- Refactored the out-of-core data ops (#81.1, the last of #81): `sort` and
  `group` no longer each hand-roll the external-merge machinery. A shared
  `ops/spill.rs` now owns the run codec, run reader, sorted-run writer, and
  k-way-merge heap element, generic over the payload each op carries (the line
  number for sort, the aggregate accumulator for group); the two keep only their
  distinct merge policy (sort's bounded multi-pass fan-in vs group's
  single-pass key fold). The whole-document scan loop that was copy-pasted
  across sort/group/top/distinct moved behind `Document::for_each_raw_line` /
  `try_for_each_raw_line`, and the 256 MiB spill budget lives in one constant.
  Behavior-preserving — the existing spill-equivalence and stable-sort tests
  hold, plus a new group spill-vs-in-memory differential test and a
  reverse-sort-with-spill stability test.
- Fixed the "名前を付けて保存" overwrite confirmation not appearing when saving
  onto an existing file by a typed path (browser build): structured API errors
  (v0.5.15) attach a `code` to every response, and the exists-check was
  short-circuiting on any code instead of falling back to the "already exists"
  message — so the main save endpoint's conflict was misread as a hard error.
- Fixed a slow memory leak in `ayame serve`: progress-tracked operations (sort,
  split, grep-save, replace, case) registered a per-run entry that was never
  removed, so a long session accumulated one handle per operation. Entries are
  now evicted when the operation finishes.
- Added a confirmation before in-place Sort and a non-destructive "sort into a
  new file" choice: the sort dialog now offers 新しいタブにソート結果を作成 /
  現在のファイルを上書き, and overwriting asks first because it reorders the file
  on disk and clears undo history. (#77)
- Added determinate progress and a Cancel button to long operations (sort,
  split, grep-save): the busy overlay shows a progress bar fed by the worker's
  line count via `/api/ops/status`, cancels the worker child through
  `/api/ops/cancel`, and now blocks edits behind it — so a finished op can no
  longer be thrown away by a racing edit (409). CLI workers gained a
  `--progress` line-counter on stderr. (#78)
- Added file/folder pickers for the two-file diff target and the folder-grep
  root, replacing hand-typed absolute paths, and Ctrl+PageDown / Ctrl+PageUp
  shortcuts (rebindable, in the command palette) to switch tabs. (#79)
- CLI consistency pass: adopted grep-style exit codes (0 = match, 1 = `search`
  found nothing, 2 = error), added `--json` to `group` / `top` / `distinct` /
  `cache`, moved the `cache` info/gc/clear reports to stderr so stdout stays
  pipeable, and regenerated the help text and `CLI_REFERENCE` to cover `case`'s
  seven modes, `sort --reverse/--budget/--spill-dir`, and `split --json`. (#80)
- Structured API errors: responses are now JSON `{code, message}` behind an
  `ApiError` type that maps `ayame_core::Error::Conflict` to HTTP 409, so the
  web client branches on a stable machine-readable code (e.g. `exists`) instead
  of matching Japanese message text. Also deduped the two breadcrumb renderers
  into `renderPathCrumbs` and added a round-trip test that pins the serve→worker
  CLI argument contract. (#81)
- Fixed Settings labels wrapping onto a second line in both Japanese and
  English: the label column is now sized per locale to its longest label and
  never wraps; on phone widths the label stacks above its control instead.
- Added a custom-image background option: 背景 is now デフォルト / 単色 /
  カスタム画像. A picked image persists in Settings (4MB cap) and covers the
  desk over the theme paper, with a re-pick row showing the current file name.
  The 単色 option drops its （全単色配慮） parenthetical.
- Added a confirmation dialog before 既定に戻す resets all key bindings.
- Fixed the last line and the `[EOF]` marker being clipped at the bottom of
  the scroll range on large files: the fully-scrolled position now shows the
  final row in full instead of hanging it past the viewport edge.
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
