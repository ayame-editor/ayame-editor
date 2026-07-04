# Ayame Roadmap

*日本語版: [ja/ROADMAP.md](ja/ROADMAP.md)*

Priorities are always **stability > simple-powerful > VSCode-like UX > Shift_JIS > huge files**.
The capacity floor is **10 billion lines**. This is not an aspiration but a lower bound enforced by design and tests.
Detailed rationale is in [DESIGN.md](DESIGN.md).

## ✅ v0.1 (implemented)

- `ayame-core`: mmap, sparse line index (parallel build), encoding detection (UTF-8/Shift_JIS/EUC-JP), streaming search, differential edit layer, streaming transform/replace. Includes the 10-billion-line capacity guard. 115 unit tests (core 85 + cli 30, always run in CI).
- `ayame` CLI: `stat`/`head`/`tail`/`line`/`lines`/`search`/`gen`.
- `ayame serve`: local web editor (virtualization, go-to-line, search, status bar).
- `ayame serve`: line-level editing (replace/insert/delete/undo/redo), in-place save / save-as-copy, regular range selection, rectangular selection, replace-in-selection. Never reads the whole original file; holds only the mmap base + diffs.
- `ayame sort --out`, `sortdiff`, `replace --out` (line-boundary chunk-parallel bulk replace via `--jobs`/`--chunk-lines`), `case upper|lower --out`. In the web editor, sort overwrites the current file; replace/case conversion save to a separate file.
- Benchmarks: 300M lines / 14 GiB indexed in 2.3 seconds, 2 MiB index, random line access 0.61 ms.

## 🎯 v1 minimal increments (the next 4 steps)

"Do not build the target architecture in one go." Each step falls back safely to v0.1 behavior on failure.

1. **Process isolation (proof of stability)** — ✅ **implemented (op-worker version)**
   `ayame serve` runs heavy ops (search/sort/replace/case/split) in **disposable child processes** equivalent to `--worker` (re-exec of `current_exe`). Even when a worker dies with an **uncatchable SIGABRT**, that request returns 502 while the engine and `/api/lines` (the viewport) keep returning 200. Deterministically demonstrated via the `AYAME_WORKER_CRASH` hook (`scripts/crash-isolation-test.sh`, 10/10 PASS). **Remaining**: the plan where the Tauri window supervises the axum host itself (host-process death = case 3 of DESIGN §4.1) is deferred, since GUI verification is not possible in this environment.
2. **Disk cache (proof of offload)** — ✅ **implemented**
   `LineIndex::to_bytes/from_bytes` (with an FNV-1a checksum trailer), content-addressed cache (`hash(canonical_path+size+mtime+stride)`), `O_EXCL` single-writer lock + tmp → atomic-rename. `Document::open` consults the cache; miss/corruption/staleness falls back safely to `LineIndex::build`. The CLI defaults to ON (`--no-cache`/`--cache-dir`/`ayame cache {path,info,clear}`). Measured: 24ms build → 0ms reopen.
3. **Disposable child-process workers** — ✅ **implemented (web: search/sort/replace/case/split; CLI: group/top/distinct)**
   `/api/search` and `/api/{sort,replace,case,split}/save` spawn a child process → wait → exit, with results handed off as JSON / artifact files. The CLI's `group` / `top` / `distinct` use the same core ops. Workers have timeouts. Minimal form with no heartbeat and no IPC framing.
4. **External-merge SORT** — ✅ **implemented** (`ayame-core::ops::sort` + `ayame sort`)
   Run generation under an explicit memory budget (`par_sort_unstable`) → disk spill → multi-pass heap k-way merge with fan-in 64 → an order-preserving `Vec<u64>` permutation. Numbers use order-preserving encoding; strings are decoded and sorted in **NFC-normalized code point order** (correct ordering for Shift_JIS too); column selection (`-k`/`--delim`) and descending (`-r`). Measured: 5M lines with a 16 MiB budget — 15 runs, 95 MiB spill, 3.25 seconds. **Remaining**: virtual-permutation display in the editor.

## 📦 Release readiness

- ✅ CI: fmt / clippy `-D warnings` / tests / release build.
- ✅ GitHub Releases: pushing a tag produces single binaries for Linux / Windows / macOS with sha256.
- ✅ Native app gate: `--features gui` produces a standalone app using the OS WebView (WKWebView / WebView2 / WebKitGTK). `ayame <FILE>` also supports file-association launch.
- ✅ Local package: `scripts/release-local.sh` produces the native-app build `dist/ayame-v<version>-<target>`.
- ✅ Single-binary verification: `--version`, `gen/stat/group --out-groups/distinct`, checksum, crash isolation.

## 🔭 v2 and beyond (future)

- ✅ **GROUP-BY / TOP-N / DISTINCT / CSV field model** implemented (`ayame-core::ops` + `ayame group|top|distinct`):
  - GROUP-BY: in-memory hash aggregation + partial-aggregate spill on budget overrun → k-way merge. count/sum/min/max/avg.
  - TOP-N: bounded O(N) heap (top/bottom, numeric/string).
  - DISTINCT: HyperLogLog (2^p registers, default p=14 = 16 KB, error ~0.8%, constant memory regardless of cardinality).
  - CSV field model: RFC 4180 quoting via `csv-core` (delimiters inside `"a,b"`, `""` escapes). Enabled with `--csv`.
  - `serve`: currently search/sort/replace/case/split run worker-isolated. Browser-facing GROUP-BY / TOP-N / DISTINCT come later.
  - **Remaining**: **embedded newlines** inside quoted fields (the current assumption is 1 physical line = 1 record), hot-partition re-partitioning, per-group distinct (HLL), integrating an operations panel into the browser UI.
- ✅ **GUI diff review** implemented:
  - Line-level diff, bounded resync window, output hunk/line caps.
  - ✅ GUI side-by-side: the diff modal shows hunk previews of the current buffer (including unsaved edits) and the comparison file side by side (default 80 lines per hunk, API cap 500 lines).
  - ✅ Inline word diff: word-token diffs are overlaid on same-position lines of `replace` hunks (very long lines automatically fall back to line-level display).
  - The CLI remains as a verification subcommand for the same engine (the product path is the GUI).
  - **Remaining**: directory diff, materializing huge diffs as artifacts.
- **OTP-style supervisor** (heartbeat/backoff for long-lived pools), `ayame-ipc` (bincode framing).
- ✅ **Cache GC** implemented (`ayame cache gc --max-size --max-age-days --dry-run`).
- **Advanced cache GC** (low-disk degradation, extension to artifact/job caches).
- **Optional DuckDB backend** (feature-gated): pushdown of multi-key GROUP BY, JOIN, and SQL via `read_csv_auto`. Build a column-projection DB only when committing to heavy analytics.
- **Japanese linguistic collation** (locale collation), UTF-16 index support.
- **Memory-cap hardening** (cgroup v2 / Job Object, or `MAP_NORESERVE`).
- **Incremental indexing** (start browsing from the already-indexed head while the build runs). tail -f style following is already implemented (log auto-follow).

## 🚧 Deliberately not doing / deferring

- **Persisting the edit WAL** (writing the append-only log / piece table to disk). Line-level differential editing, undo/redo (including across saves), in-place/save-as, range and rectangular selection, replace-in-selection, multiple cursors, and whole-file replace/case/sort/split are all implemented; the only untouched piece is this persistence. **A fully in-memory rope will not be built.**
- cgroup RSS caps at the v1 stage, syscall tuning (fadvise/fallocate), DuckDB — all of these wait until there is something to protect.
