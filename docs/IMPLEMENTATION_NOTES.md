# Implementation Notes — What Is Built, and How (v0.3.x)

*日本語版: [ja/IMPLEMENTATION_NOTES.md](ja/IMPLEMENTATION_NOTES.md)*

A map to read before reading the code. For each mechanism, this collects "why this shape" and which file it lives in.
Background for the design decisions is in [DESIGN.md](DESIGN.md); remaining tasks are in [ROADMAP.md](ROADMAP.md).

## Overall structure

```
ayame (single binary)
├─ ayame-core     engine: mmap + sparse index + edit overlay + transform/search/split
├─ ayame-cli
│   ├─ cli/       CLI subcommands (stat/search/sort/replace/case/split/gen/diff/…)
│   ├─ serve/     local HTTP server = UI layer (axum, loopback by default)
│   ├─ gui.rs     native window (tao + wry = the OS WebView). Launches serve in the background and points at it
│   └─ web/       frontend (plain JS, no build step, embedded in the binary)
└─ xtask          repository automation (cargo xtask release)
```

The native app is the product. `gui` launches `serve` in the background on an ephemeral port and
simply points the OS WebView (WKWebView / WebView2 / WebKitGTK) at it — there is only one UI stack.

## Engine (ayame-core)

### Reading: mmap + sparse line index
- `document.rs` / `index.rs`: the file is mmap'd immutably, and only a checkpoint (16B) per
  `stride` (default 4096) lines is kept. Access to an arbitrary line is "walk with memchr from the
  nearest checkpoint" — O(stride). Even at 10 billion lines the index is a few MB. The index is
  disk-cached, so reopening is instant.
- **No fully in-memory rope / piece table will be built** (a design-level prohibition). Never
  holding a structure whose memory explodes on huge files is the foundation of "does not crash".

### Editing: mmap base + sparse overlay (`edit.rs`)
- Edits never touch the original bytes; an `EditSession` holds line-anchor-level diffs
  (replaced/deleted/inserted lines) in a BTreeMap. The view is the composition of "base + overlay".
- **undo/redo uses inverse deltas**: each edit transaction records only "the minimal sequence of steps
  that undoes what actually changed" (`UndoRecord`). Abandoning the snapshot approach (full overlay
  clone × 256 generations) took one keystroke after a huge paste from 1.47s to 0.63ms.
  Applying an undo returns its own inverse (= the redo record), so undo and redo are one mechanism.
- **Multiple cursors use `replace_batch`**: N edits are applied in descending order (bottom-up) to
  avoid coordinate invalidation, and the inverse deltas are bundled into one record = **one undo
  reverts all cursors**. Post-apply caret positions are recomputed by an ascending replay
  (line deltas + column deltas within the same line).

### Saving: "never reopen" is what protects undo
- Writing out is not a loop over all lines; it **copies the intact spans between edit anchors as
  contiguous byte ranges** (O(edits×stride + bytes); 2M lines × 3 edits went from 25s to 15ms).
- **Undo across saves** (the two-generation `content_gen`/`saved_gen` scheme):
  `revision` remains a monotonically increasing optimistic-locking counter, while a separate
  "content generation" is stored in each history record and restored by undo/redo.
  dirty = `content_gen != saved_gen`. This lets undo→redo land exactly on the saved state
  (impossible with a monotonic counter comparison alone).
- A save commit **renames the old file aside within the same directory** and renames the staged
  file into the real name. The document is never reopened, so the mmap, overlay, and undo history
  survive intact (on Unix the old inode stays alive under the mmap and the aside file is deleted
  immediately; on Windows it is cleaned up at close time).
  Revert means "back to the last saved state" = reload from disk.

### Search, transform, split
- `search.rs`: UTF-8 uses memmem. **Legacy encodings (Shift_JIS/EUC-JP) verify character boundaries
  for literal matches** (the 0x5C problem: rejecting false hits where `\` matches the trailing byte
  of "ソ"); case-insensitive/regex decode line by line. Backward search (previous match) scans
  chunks walking backward.
- `transform.rs` / `split.rs`: replace, case conversion, and line splitting are **byte-preserving**
  streaming. Split produces `<stem>.partNNNN<.ext>`, and tests guarantee that concatenating the
  parts reproduces the original byte sequence.

## Server (serve/)

### One lock + optimistic commits (`state.rs`)
- The doc / edit overlay / tabs all live under **a single `RwLock<Workspace>`**
  (they used to be separate locks, and edits could vanish in the gap between a save and a tab switch).
- Long-running work (save, sort) holds no lock: snapshot → work without the lock →
  at commit time re-verify **doc identity (Arc::ptr_eq) + revision**; if they diverged, 409 and retry.
  "Silently interleaved / silently lost" is ruled out mechanically.
- Lock poisoning is recovered with `into_inner()` (one panic cannot permanently halt the editor).

### Worker isolation and dirty materialization (`ops.rs`)
- Sort, replace, case, split, and search (counts) run as **child processes** (the binary re-invokes
  itself with a subcommand). Runaway, OOM, or abort cannot kill the editor itself
  (the crash-isolation test enforces this in CI).
- When there are unsaved edits, a shared helper **materializes the buffer into a temporary file**
  handed to the worker. Interactive "find next" jumps use a revision-keyed snapshot cache
  (built once per edit generation, reused afterwards).
- Anywhere a path reaches the client goes through a display helper that strips Windows' `\\?\`
  verbatim prefix.

### Network boundary (`security.rs`)
- Loopback-only by default. Non-loopback requires `--allow-remote`.
- Host validation on every request (anti DNS rebinding) and Origin validation on state-changing
  endpoints (anti CSRF).

## Frontend (web/)

- **Virtualization is a given**: the DOM contains only the visible range plus pad lines, and
  scrolling uses a hand-rolled line-number-based scrollbar (pixel coordinates cannot represent
  10 billion lines within browser height limits).
- Edits are serialized through a single queue (`enqueueEdit`). Saving waits for the queue to drain
  before snapshotting; new edits during a save are blocked with a spinner (IME commits are held and
  applied after the save).
- Multiple cursors are "several carets + one batch = one undo". Deletions that touch lines outside
  the cache first fetch those lines before building the edges (so a line is not mistaken for
  zero-length, corrupting the previous line).
- Commands are unified in `runMenuAction`: the menu, context menu, command palette, and the native
  macOS menu (`__ayameMenu`) all hit the same table. Labels and shortcut hints have a single source
  of truth in `KEYMAP_ACTIONS`.

## Native window (gui.rs)

- Startup "shows the window first": a FILE argument is `/api/open`ed asynchronously, and the window
  is visible with a progress overlay even during the initial index build. Created hidden +
  `ayame:ready` eliminates the white flash.
- Drag & drop uses wry's native handler to **mmap the real path directly** (no full upload through
  the DOM).
- The WebView profile is pinned via `WebContext` to the **OS-standard cache location**
  (`%LOCALAPPDATA%\ayame\webview` / `~/Library/Caches/ayame/webview` / `~/.cache/ayame/webview`)
  — suppressing WebView2's default behavior of creating `ayame.exe.WebView2` next to the exe.
- Window position/size/maximized state persist in the same cache. If the close-confirmation does
  not respond, a timeout forces termination (never create a window that cannot be closed).
- macOS only: a native menu via muda (to make WKWebView's Cmd+C/V work; paste alone is routed
  through the native selector into a DOM paste).

## Release automation (xtask/)

`cargo xtask release [--bump patch|minor|major|X.Y.Z] [--yes|--dry-run|--skip-gate]`
— pure Rust, so it behaves identically on every OS (no bash/node dependency).
Gate (fmt / clippy±gui / test / release build / crash isolation) → dist artifacts + sha256 →
CLI smoke → tag → push → watch the GitHub Actions Release — all in one command.

## Record of major refactors (v0.2.4 → v0.3.x)

| Area | Before | After |
|---|---|---|
| Undo history | full-overlay snapshots × 256 | inverse-delta records (~2300x) |
| Save | random access over all lines O(lines×stride) | contiguous copies between anchors (~1700x) |
| Save vs undo | history destroyed by save (reopen) | generation markers + rename-aside keep history alive |
| serve state | doc/edits/tabs under separate locks | single Workspace lock + optimistic commit (409) |
| SJIS search | raw-byte memmem (false hits / zero hits) | boundary verification + line decoding |
| main.rs | a single ~1200-line file | 9 modules under cli/ |
| Releases | a manual checklist | cargo xtask release |

## Crash-persistence WAL (crates/ayame-core/src/wal.rs + serve wiring)

Crash durability for unsaved edits. The overlay itself remains in-process memory only, as before,
with committed transactions mirrored to an append-only JSON Lines log.

- **Format**: one externally tagged JSON record per line. The first record is always a Header
  (the base file's len / mtime_ms / encoding = identity). Then Txn records (recording the public
  API's logical-coordinate calls verbatim) and Snapshot records (compaction points = the full
  overlay). Location: the same root as the index cache, `<cache>/wal/<FNV-1a hash of path>.wal`
  (follows `--cache-dir` / `AYAME_CACHE_DIR`; `--no-cache` disables the WAL too).
- **flush/fsync policy**: flush to the OS on every commit (process-crash durability).
  fsync (power-loss durability), compaction beyond 64 MiB, and the one-shot UI notification for
  write errors (`stat.wal_error`) are handled by serve's policy thread on a ~3-second cycle.
  Log I/O failure never blocks editing: the writer is dropped and operation continues degraded.
- **Recovery flow**: on open, `inspect` — if Recoverable, nothing is auto-applied;
  `stat.recoverable = n` is returned and the frontend shows a confirmation dialog (restore/discard).
  `POST /api/edit/recover` (`{}` = replay, `{"discard":true}` = delete the log).
  Replay happens in a scratch session outside the lock, and installation re-verifies doc identity +
  revision 0 (the same optimistic discipline as save commits). Stale/Invalid logs are silently
  deleted on open.
- **Lifecycle**: only the workspace's "live" session holds the writer (clones and parked tabs do
  not — on tab reactivation it re-attaches and takes a full snapshot). After an in-place save
  commit / revert / encoding reload / in-place sort, the log resets under the new file identity.
  Save-as (switch) starts a fresh log at the new path and deletes the old path's log. Closing a tab
  deletes its log (logs pending a restore decision are kept). On clean shutdown only the logs of
  clean sessions are deleted; dirty ones are kept (candidates for restore next time).
- **Known limits**: undo/redo into history older than a reset/compaction point degrades to a full
  snapshot (recovered sessions only have undo history for the suffix since the log started).
  Post-save snapshots are **rebased onto the new base**: `reset_for_save` captures the overlay that
  produced the saved file, so undoing across a save point never yields a mismatched restore. If the
  legacy plain-reset path is ever taken, the log is rewritten clean and recovery for that window is
  disabled — a wrong restore is structurally impossible. A corrupted newline-terminated record marks
  the whole log Invalid (recovery refused); only an unterminated EOF fragment is ignored as torn.

## Deliberately not done

- A fully in-memory rope / piece table (prohibited — DESIGN.md).
- ~~Edit-WAL persistence not started~~ → **implemented in v0.3.x** (see "Crash-persistence WAL" above).
  What remains unimplemented is persistence of the undo history itself (after recovery, only the
  replayed suffix is undoable).
- cgroup RSS caps, fadvise/fallocate tuning, DuckDB integration (waiting until there is something to protect).
- Browser-only operation and multi-user (the server is the UI layer of the native window).
