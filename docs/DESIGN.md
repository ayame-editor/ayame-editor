# Ayame (菖蒲) Design Document — Huge-Text Editing and Inspection Tool

*日本語版: [ja/DESIGN.md](ja/DESIGN.md)*

- Status: draft (for design consensus / a plan built on top of the already-implemented v0.1 core)
- Target scale: **at least 10^10 lines** / hundreds of GB to TB of logs, CSV/TSV/JSONL, and data-migration dumps
- Top-priority requirements (this ordering drives every decision):
  **Stability (designed to crash)** > simple yet powerful > VSCode-like UX > Sakura lineage (Shift_JIS) > opening huge files comfortably
- Existing assets: `ayame-core` (mmap + sparse line index + search + differential editing), `ayame-cli` (CLI + axum web editor) — **do not rewrite; build on top**

> This document was produced by running six independent analyses in parallel and validating/correcting them through an adversarial critique. Items cut or fixed by the review are called out explicitly in §10 and §11.

---

## 0. Conclusion Summary (TL;DR)

| Question | Decision | Rationale in one line |
|---|---|---|
| Do existing tools suffice? | **No. Worth building. Scope limited to "huge-file editor + offloaded data operations"** | No single tool sits at the intersection of OSS + cross-platform + VSCode-like GUI + TB-scale O(index) memory + parallel ops + crash isolation + Shift_JIS |
| Language | **Stay with Rust** (Go rejected) | Go's GC conflicts with the memory north star. The core already works and is tested in Rust |
| GUI | **Tauri 2 + reuse of the existing web UI** | OS webview means no bundled Chromium, direct linking of the Rust core, cross-platform |
| Stability | **Process isolation + bounded-budget ops** is the heart. When something dies, **keep the screen** and restart only the worker | A process boundary is the true safety net against OOM / runaway / panic |
| sort/group | **Tiered**: grep/top-n built in-house, external sort / hash group-by also in-house. DuckDB as an optional future backend | Per-op budgets, partial results, and job isolation can only be obtained by building them ourselves |
| Pushdown to a DB | **A future, second-class, optional feature** (column projection only for repeated queries) | Full projection means 1.5–3x write amplification of the file. The common path costs zero DB work |
| Disk offload | **`ayame-cache`: content-addressed blobs + manifest, TTL + LRU + cap** | The cache is a pure accelerator. On a miss, degrade to RAM recomputation |
| SSD care | **append-only / atomic-rename, large blocks, batched commits, no in-place rewrites** | The wear hazard is not "volume" but the "pattern" (write amplification) |

---

## 1. Survey of Existing Tools and Differentiation (Why Build This)

Mapping each candidate onto Ayame's requirement axes (stability / simple-powerful / VSCode-like UX / Shift_JIS / huge files / data operations / crash isolation), **every one of them fails on two or more axes**.

| Tool | Memory model | Huge files | Data ops | GUI | Shift_JIS | OSS/cross-PF | Fatal gap |
|---|---|---|---|---|---|---|---|
| **Zed** (Rust) | in-memory rope | ✗ >64GB for a 10GB file, **hard-rejects** anything over 6GB | △ | ◎ | ◎ | ◎ | Cannot do huge files (= Ayame's motivation) |
| Sublime / Notepad++ / NotepadNext | full load (Scintilla resident) | ✗ (breaks down at a few GB, 4x RAM) | ✗ | ◎ | △ | △ | Cannot do huge files |
| **EmEditor** | temp disk | ◎ 248GB / 2.1B lines, parallel sort | ◎ | ◎ | ◎ | ✗ | **Windows-only, closed, commercial** (unauditable = contradicts the Zed trauma) |
| klogg / glogg | reads from disk | ◎ indexes 16GB in ~30s | ✗ (no sort/group) | C++/Qt | △ | ◎ | No data operations, no editing |
| lnav | content not resident | ◎ logs into SQLite virtual tables | △ (SQL only) | TUI | △ | ◎ | Terminal logs only |
| DuckDB | out-of-core spill | ◎ aggregates a billion rows on a laptop | ◎ | ✗ | △ (UTF-8 assumed) | ◎ | Headless, no fault-tolerant UI |
| qsv (Rust) / Miller (Go) | external merge / one line resident | ◎ | ◎ (CSV/JSON) | ✗ | △ | ◎ | Pure CLI |
| VisiData (Py) | async streaming | △ (keeps filter sets in RAM) | ◎ | TUI | △ | ◎ | Python TUI, practical ceiling |

**The core differentiator:** none of these tools has process-isolated fault tolerance where **the viewport survives a compute/worker crash and browsing continues**. That is Ayame's biggest differentiator, alongside OSS + cross-platform + GUI + Shift_JIS.

**The non-negotiable capacity line:** Ayame is not "a tool that can open a few GB" — it sets **at least 10 billion lines** as its design floor. With the default stride of 4096, the sparse index for 10 billion lines is 2,441,407 checkpoints × 16B = 39,062,512B (about 37.3 MiB). This calculation is pinned by `MINIMUM_SUPPORTED_LINES` and a unit test.

**The honest trade-off:** each individual axis has already been solved by some existing tool. Ayame's value lies in **integration and packaging** (the technical moat is thin). Huge-file editing stacks a differential layer on top of an immutable mmap base (a foundation that never mutable-mmaps the original file directly). The initial implementation covers line-level replace/insert/delete and save-as-copy; from there it expands to undo/redo, range edits, and in-place save.

One-line positioning: **"an open-source, cross-platform EmEditor with a crash-proof UI (with a DuckDB engine bundled in the future)."**

---

## 2. Language Choice: Stay with Rust (Go Rejected)

| Aspect | Rust | Go | Verdict for Ayame |
|---|---|---|---|
| Memory determinism | No GC. A 39MB index stays 39MB | GC (up to ~2x live heap at GOGC=100, page release is lazy) | **GC conflicts with the north star O(index+viewport+hits)** → Rust |
| mmap+SIMD | memmap2 + memchr (AVX2/SSE2/NEON), no `unsafe` needed in our own code | No SIMD memchr in the stdlib, tens to hundreds of ns/call across cgo | **The cgo tax in a per-line loop is fatal** → Rust |
| Data parallelism | rayon work stealing (ideal for CPU-bound chunk scans) | goroutines are concurrent, but for data parallelism rayon wins | Rust |
| Crash characteristics | No null, data races are compile errors, panics unwind and can be caught at isolation boundaries | nil dereference / concurrent map write take down the whole process | **Decisive for requirement #1** → Rust |
| Shift_JIS/EUC-JP | encoding_rs + chardetng (native, zero FFI) | ICU-class codecs via cgo (C toolchain required) | Rust |
| Implemented assets | `ayame-core` working and tested | Rewrite from scratch | **A rewrite is pure negative-value risk against requirement #1** → Rust |

The clincher: the root [`Cargo.toml`](https://github.com/hjosugi/ayame-editor/blob/main/Cargo.toml) explicitly documents in `[profile.release]` that we "**deliberately do not adopt `panic = "abort"`** (per-request panics are isolated via unwinding)". **Unwind-based panic isolation is already operating as a core piece of stability.** Zed is also Rust yet falls over on huge files — evidence that **the differentiator is the memory model, not the language**.

> Other languages considered (Zig/Erlang/OCaml/Motoko, etc.): Zig is still pre-1.0 as of 2026 (ongoing breaking changes + no borrow checker = manual memory safety), which contradicts "stability first". From Erlang/OTP we borrow the **fault-tolerance philosophy** (let-it-crash + supervision), but a GC'd VM is ill-suited to CPU-bound scans of 100GB → implement the philosophy with OS processes in Rust. OCaml is solid but has a GC plus thin libraries/GUI for this use case. Motoko is ICP (blockchain)-only and cannot touch local files — **out of scope**. Conclusion: "Rust + an OTP-style architecture".

---

## 3. GUI Choice: Tauri 2 Shell + Reuse of the Existing Web UI

- **Adopted: Tauri 2.** Uses the OS webview (macOS WKWebView / Windows WebView2 / Linux WebKitGTK), so no bundled Chromium → a shell of a few MB (vs ~150MB for Electron). Native Rust, linking `ayame-core` directly. The existing VSCode-style virtualized web UI is **reused, not rewritten**.
- **Rejected:** GPUI (61k lines of Zed-specific code = importing exactly the complexity we differentiate against), egui/Slint (would rebuild the VSCode-like UX from scratch), Fyne/Wails (a Go host = reintroducing the GC downsides).

The UI is a virtualized text list with light CSS dependence, so cross-webview rendering-difference risk is low. The bottleneck is the engine, not rendering.

---

## 4. The Heart of Stability — Crash Taxonomy and Process Isolation

> This is the section the review corrected the most. Define "the screen survives a crash" **precisely**.

### 4.1 Crash taxonomy (who dies, and what happens)

| # | What dies | What happens | Recovery | Implementation cost |
|---|---|---|---|---|
| 1 | **Task worker** (disposable child process for sort/index/grep) | UI unaffected. Only that job fails | The supervisor restarts it, or an error toast → user retries | **Cheap — the most important win** |
| 2 | **Warm-pool worker** (viewport fetch, incremental search) | Viewport stalls for <1s | Re-spawn | Medium |
| 3 | **axum/host process** (the engine itself) | The Tauri window **keeps displaying the last rendered viewport (frozen)** + a "reconnecting" indicator | **Tauri restarts the sidecar** | Medium |

**Important correction:** what guarantees "the screen survives even if the app dies" is **the process separation between the Tauri window and the engine (sidecar)** — not the disk journal *alone*. If the host dies, no new data can be produced (frozen display + reconnect). Therefore **Tauri is the top-level supervisor** that watches over the host. The journal helps preserve worker results and the current viewport; it does not make host death transparent.

### 4.2 The true safety net = "bounded-budget ops" + "process boundaries"

Only two things produce stability:
1. **Ops that run within bounded budgets** (external-merge / partitioned-hash are designed to run within a RAM cap B). This is the **first line of protection**.
2. **Hard process boundaries.** Even under runaway allocation, OOM, or abort-class panics (stack overflow / double panic / FFI), the kernel reclaims that worker's pages and the OOM killer targets only that worker. The UI (mmap viewport) survives.

`catch_unwind` (inside workers) is the **second line** (catches unwindable panics; we reuse `tower-http`'s CatchPanicLayer, [`serve/mod.rs`](https://github.com/hjosugi/ayame-editor/blob/main/crates/ayame-cli/src/serve/mod.rs)). Abort-class failures cannot be caught, so the last line of defense is the **process boundary**.

> **RSS caps via cgroup v2 `memory.max` / Windows Job Objects are not a v1 guarantee (review finding).** In a normal desktop launch, moving oneself into a cgroup requires privilege delegation and cannot always be applied. v1 is protected by "bounded-budget ops + process boundaries"; RSS caps are positioned as **later hardening**.

### 4.3 Justifying process spawn cost

Process spawn on Linux is ~1–2ms — negligible against multi-second sorts or minutes-long full index builds. But **viewport fetches must complete in <16ms (60fps)**, so they are served by a **warm pool** rather than spawning every time. Because the data is sparse, a fault-tolerance journal is cheap (index = 16B × ceil(lines/4096); a viewport snapshot = a few hundred lines = KB).

All workers are **a single binary self-re-exec'd with `--worker <role>`** (the Chrome/Zed approach). Distribution remains a single executable.

---

## 5. Data Operations — a Tiered Design

Do not frame "build in-house vs delegate to DuckDB" as either-or; layer it.

| Op | Implementation | Algorithm | Memory | Reused from |
|---|---|---|---|---|
| **GREP** | in-house (top priority, lowest risk) | `search::Matcher` (memmem/regex) chunk-parallel via rayon par_iter, hits into a **bounded** channel | O(hits+viewport) | [`search.rs`](https://github.com/hjosugi/ayame-editor/blob/main/crates/ayame-core/src/search.rs) reused almost as-is |
| **TOP-N** | in-house | per-thread `BinaryHeap` of size N, merged at the end | O(N×threads) | — |
| **SORT** | in-house (external merge) | run generation (budget B = default 512MB–1GB) → `par_sort_unstable` → spill in ≥1MiB sequential writes, k-way merge (fan-in 64, multi-pass beyond that) | O(B + fan-in) | chunking from `index.rs::build` |
| **GROUP-BY** | in-house (partitioned hash) | hash keys into P (256–1024) spill partitions, aggregate each partition in parallel | O(B + P) | — |
| **Heavy analytics/SQL** (future) | **DuckDB (feature-gated, optional)** | `read_csv_auto` (no copy) for multi-key GROUP BY / JOIN | DuckDB self-managed | `duckdb-rs` |

### 5.1 Sort collation (the Shift_JIS correction)

> **High-priority review correction:** the **raw byte order of Shift_JIS/CP932 is neither JIS order nor Unicode order** (multi-byte lead/trail bytes interleave with ASCII and kana). "Compare raw bytes on the hot path" is **correct for grep/viewport but wrong for SORT/GROUP-BY key comparison**.

The v1 policy:
- Decode **only the key field** to UTF-8 and sort by the **NFC-normalized key** (= code point order, correct and deterministic). Decode cost applies to the key only, not the whole line.
- Explicitly label v1's ordering as "**code point order, not linguistic (locale) collation**". Japanese dictionary-order collation is a later milestone.

### 5.2 Results viewed as a "virtual permutation" (zero copy)

Each op result is materialized as an **ordering (a spilled `Vec<u64>` of line numbers)**. The UI views that permutation through the existing `index.line_ranges` ([`index.rs`](https://github.com/hjosugi/ayame-editor/blob/main/crates/ayame-core/src/index.rs)), so **even the sort result of 10 billion lines is displayed with no data copy**, through the same sparse-fetch path.

### 5.3 CSV/TSV field model (added by the review)

The sparse index is **line**-oriented. sort/group-by need key **fields**, so field splitting is required:
- v1: split each line on access with `csv-core` (RFC 4180 quote-aware); the re-splitting cost is acceptable.
- Future: if needed, keep a separate "sparse field index" (throughput improvement).

### 5.4 Backpressure (added by the review)

If a 1TB grep produces hits faster than the webview consumes them, the SSE send queue grows **without bound** and "bounded memory" is broken. Countermeasure: a bounded channel + a **drop/pause policy** (when the send queue exceeds a threshold, pause the producer; the UI shows "over N hits — consider narrowing").

---

## 6. Disk Offload + Cache Management (SSD Care)

### 6.1 Policy: the `ayame-cache` crate

- Root: the XDG cache (`~/.cache/ayame/v1/`, overridable via `AYAME_CACHE_DIR`, mode 0700).
- Structure: **content-addressed immutable blobs** + **a small single manifest** (rusqlite bundled).
- Cache key: `blake3(canonical_path) ‖ size ‖ mtime_ns` **+ encoding/stride overrides** (review addition: line numbers of a snapshot opened under a different encoding are invalid, so the key must include them).
- **The sparse index is always cached** (16B × checkpoint; at 4096 lines per checkpoint, 10^10 lines = ~39MB = ~0.0006% of a 6TB file). Reopen goes from a multi-second build to **mmap + verify**.
- Global cap: **5% of free space, clamped to [2GiB, 64GiB]**. TTL of **14 days** (unused) + **LRU** eviction (artifacts of currently open documents are never evicted).
- Every blob is **reconstructible from source** → crashes, deletion, read-only/remote sources, and full disks all **degrade to RAM recomputation instead of failing**.

### 6.2 Integrity and concurrency (added by the review)

- **Validating an index blob by header alone is insufficient.** Attach `line_count` + a **checksum trailer** (covering the checkpoint array) to detect truncated/partial writes before trusting the blob for random access. Validation failure = treated as a cache miss, falling back to `LineIndex::build`.
- **Multi-process/window contention:** two processes opening the same huge file could double-build the same blob. Use a per-key **`O_EXCL` `.building` lock file** (single writer). If a peer is building, wait or fall back to a RAM build.
- Staleness check: re-stat `size + mtime`. For the rare case where mtime is preserved (e.g. `rsync --times`), a blake3 of the first + last 64KiB as a tiebreaker (adopt depending on cost).

### 6.3 SSD wear countermeasures (minimizing write amplification)

The danger is the **pattern, not the volume**. Consumer TLC SSDs are ~600 TBW per 1TB (0.3 DWPD × 5 years). Even hypothetical churn of 20GB/day is 7.3TB/year = ~1.2% of the budget per year — safe, **as long as write amplification is avoided**.

| Countermeasure | Details |
|---|---|
| **append-only / atomic-rename** | Write to `<key>.tmp.<pid>` with large buffers, one fsync at the end, rename into place |
| **No in-place rewrites** | Never overwrite an entire index (blobs are immutable) |
| **Large-block writes** | Spills are ≥1MiB (ideally 1–4MiB) sequential writes |
| **Batched manifest updates** | Avoid per-record fsync; one commit per operation (WAL + `synchronous=NORMAL` + `busy_timeout`) |
| **Throttled GC** | Deletions capped at N unlinks per tick + yield. At startup (doubling as orphaned `.tmp` collection) / idle / explicit `ayame cache gc` |
| **Low-disk handling** | Check free space before writing; if short, switch ops to RAM-only/streaming, warn only, and **never fail the user's action** |

---

## 7. Integration with the Existing Core (Zero Rewrites)

`ayame-core` stays unchanged as the compute kernel. New crates are added:

| New | Role | Connection points to existing code |
|---|---|---|
| `ayame-core::ops` | budget/spill/chunk/grep/sort/groupby/topn | reuses `index.line_ranges`/`line_of_byte`/`search::Matcher` |
| `ayame-cache` | XDG root, blobs, manifest | add `LineIndex::to_bytes/from_bytes` (checkpoints are 16B POD, trivially serializable), make `Document::open` cache-aware |
| `ayame-worker` | `--worker <role>` re-exec entry point; each op runs inside `catch_unwind` | calls into `ayame-core` internally |
| `ayame-desktop` | Tauri 2 shell | embeds the existing web assets, launches and supervises axum as a sidecar |

`Document::open` change: compute the key → ask `ayame-cache` for a valid index blob → on hit, `from_bytes` (mmap + trailer verification) and skip `LineIndex::build`; on miss, build as before and persist. Ops never modify the document, never load the file onto the heap, and preserve the **O(index+viewport+hits) invariant** documented in `lib.rs`.

---

## 8. Trade-offs Made Explicit (As You Articulated Them)

- **Disk ↔ memory:** local disk is **deliberately consumed** for stability (external sort spills ~1x the input = 2x I/O). In exchange, OOM crashes are eradicated. **Accepted.**
- **Process isolation ↔ simplicity:** more moving parts. Mitigated by **single-binary self re-exec** so distribution stays a single executable.
- **In-house ops ↔ DuckDB:** building in-house buys budgets, partial results, and isolation (at higher maintenance cost). Breadth is later delegated to DuckDB — best of both.
- **Cache ↔ wear:** the index is cached unconditionally (size ~0.0006%). Writes are append-only + batched to curb wear; bounded by cap + TTL.

---

## 9. v1 Minimal Increments (Critique-Driven Execution Plan)

> Do **not** build the "target architecture" (§3–7) in one go. Four steps that prove the differentiator at minimal cost without breaking stability. Each step **falls back safely to current behavior on cache miss/crash**.

1. **Step 1 — Prove "designed to crash" (minimal)**
   Launch the existing axum **as-is** as a sidecar child process, with a thin host in front (a Tauri window, or a supervising parent process). If the engine child dies, **keep the last viewport**, show "engine restarting — display preserved", and re-spawn. **The first test to write = SIGKILL injection** (kill the engine mid-request → assert the window keeps its last display). This single test proves the "designed to crash" thesis; no supervisor/IPC/spool needed.

2. **Step 2 — Prove disk offload safely**
   Add `LineIndex::to_bytes/from_bytes` (trivial for a 16B POD array) + `line_count` + a checksum trailer. Make `Document::open` use a content-addressed cache (`blake3(path)+size+mtime+encoding+stride`). Per-key `O_EXCL` locks avoid double builds. Reopen goes from "seconds of building" to "mmap + verify". **Miss/corruption simply falls back to `LineIndex::build`** = zero risk to the working editor.

3. **Step 3 — One op with no worker zoo** — implemented
   `/api/search` launches `ayame search --json --start-byte` as a disposable child process and receives hits as JSON. No heartbeat, no IPC framing (spawn→wait→exit code; on crash, a toast → retry). The same shape has been extended to sort/group/top/distinct.

4. **Step 4 — (once 1–3 are solid) external-merge SORT**
   Same disposable-child-process shape, with an explicit budget B + ≥1MiB sequential spills; the result is a `Vec<u64>` permutation (the editor displays it via `index.line_ranges` = a zero-copy virtual permutation). v1 sorts by **decoded + NFC-normalized keys** and explicitly labels the order as "code point order (not linguistic collation)".

---

## 10. Items Deliberately Deferred Beyond v1 (Avoiding Over-Engineering)

Things the review flagged as "a perversion, given stability (requirement #1): building the most complex, unproven machinery first, before the thing it protects even exists". **Not built in v1:**

- An OTP-style supervisor (2s heartbeats, exponential backoff, MAX_RESTARTS). Disposable jobs only need "spawn→wait→judge by exit"; a reconnect state machine comes later, and only for long-lived pools.
- `ayame-ipc` (length-delimited bincode framing). The first disposable workers are fine with argv input + result-path output + exit code.
- cgroup/Job Object RSS caps (moved to hardening).
- HyperLogLog, recursive re-partitioning of hot partitions, `posix_fadvise/madvise`, `fallocate`/sparse files, and other syscall tuning (each unproven on the stability axis).
- The DuckDB backend (a large C++ dependency + its own memory manager = at odds with "bounded, predictable, isolatable"). In-house grep/sort/group come first.
- The durable-spool narrative of "supervisor death is transparent too" (per §4.1, Tauri separation covers this).
- **Advanced editing engine.** Initial line-level editing is in. v1 keeps the minimal differential layer; undo/redo, rectangular selection, grep-replace, in-place save, and a persistent WAL land incrementally. "Immutable mmap base" means "never mutable-mmap the original file directly"; editing is realized as mmap base + differential WAL / piece table.

---

## 11. Open Questions (Need Further Study)

1. **Empirical validation** of the floor (10^10 lines / TB scale): staged benchmarks toward a synthetic 10-billion-line file (starting with 50GB / 1B-line JSONL), measured against EmEditor (Windows VM) / klogg / lnav / DuckDB, producing evidence that no other tool simultaneously satisfies "GUI + ops + O(index) memory + OSS".
2. **How to implement worker memory caps** (RLIMIT_AS counts mmap address space = instantly exceeded by a 1TB mmap; cgroup/Job Object, or `MAP_NORESERVE` + heap limits?).
3. The DuckDB feature's **feature-gate boundary and CI/distribution matrix** (keep the default build lightweight and predictable).
4. A rigorous spec for **Shift_JIS collation** (default collation rules, round-trip guarantees with original-byte preservation).
5. Cost-effectiveness of the blob **staleness tiebreaker** (blake3 of first + last 64KiB).
6. (Future) the edit WAL's schema and fsync cadence (append-only mmap base + differential patches; **never build a fully in-memory rope**).
7. Cross-webview distribution details (WebKitGTK font/version differences, bundling the WebView2 runtime).

---

## Appendix: Key Evidence (Sources)

All verified against real sources during the review.

- **Zed's memory-model failure (the motivation):** `refs/zed-main/crates/worktree/src/worktree.rs:1514` "use in excess of 64GB for a 10GB file", `:1520` `FILE_SIZE_MAX = 6GiB`, `:1524` `bail!("File is too large to load")`.
- **Zed child-process teardown:** `crates/lsp/src/lsp.rs:429` `.kill_on_drop(true)`.
- **Zed heartbeat/backoff:** `crates/remote/src/remote_client.rs:160-165` (HEARTBEAT_INTERVAL=5s, etc.).
- **Zed's SQLite settings (adopted):** `crates/db/src/db.rs:130-133` (WAL / NORMAL / busy_timeout).
- **Ayame core:** [`crates/ayame-core/src/index.rs`](https://github.com/hjosugi/ayame-editor/blob/main/crates/ayame-core/src/index.rs) (16B Checkpoint, stride 4096, rayon parallel build, line_ranges/line_of_byte), [`search.rs`](https://github.com/hjosugi/ayame-editor/blob/main/crates/ayame-core/src/search.rs) (Matcher), [`document.rs`](https://github.com/hjosugi/ayame-editor/blob/main/crates/ayame-core/src/document.rs) (the open integration point), root [`Cargo.toml`](https://github.com/hjosugi/ayame-editor/blob/main/Cargo.toml) (`panic=abort` not adopted), [`serve/mod.rs`](https://github.com/hjosugi/ayame-editor/blob/main/crates/ayame-cli/src/serve/mod.rs) (shared state + CatchPanicLayer).
