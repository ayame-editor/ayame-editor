# Data Integrity Guarantees

Ayame is built to edit files where **a single wrong byte is unacceptable** —
financial records, legal text, scientific data. This page states the integrity
guarantees the editor makes, the mechanism behind each one, and the automated
test that verifies it. Every check listed here runs on ordinary
`cargo test --locked`, so it executes on every CI run.

## What "correct" means here

A guarantee is only meaningful if it is mechanically verified. Each guarantee
below is paired with an executable test under `crates/ayame-core/tests/`. The
tests compare against a trusted reference (a plain `Vec` model, a sequential
re-run, or the original bytes) and assert **byte-for-byte** equality, using a
SHA-256 checksum utility for legible failures.

| # | Guarantee | Mechanism | Verified by |
|---|-----------|-----------|-------------|
| 1 | Saving reproduces the exact bytes (BOM, per-line terminator, and a missing final newline all preserved). | Untouched lines are copied verbatim from the memory map; only edited lines are re-encoded. | `tests/roundtrip_bytes.rs` |
| 2 | The edit overlay is equivalent to a trusted model under any sequence of insert / replace / delete / undo / redo. | Overlay edits and history are diffed against a `Vec` reference after every step. | `tests/edit_overlay_equivalence.rs` |
| 3 | The line and column the editor shows are exactly where an edit lands — including wide characters, tabs, multi-cursor, and immediately after scrolling. | Logical line numbers and Unicode-scalar columns are stable across viewport windows and batch edits. | `tests/caret_coordinates.rs` |
| 4 | Encoding round-trips are byte-exact, and an unrepresentable character aborts instead of writing a lossy file. | The encoder reports unmappable characters; untouched lines are never re-encoded. | `tests/encoding_roundtrip.rs` |
| 5 | Crash recovery restores edits up to the last committed transaction, and no crash corrupts the original file. | A write-ahead log (WAL) records each committed edit; a torn trailing record is ignored, corruption is refused. | `tests/crash_recovery.rs` |
| 6 | Parallel transforms are byte-identical to sequential ones (sort / replace / case / split), across chunk boundaries, mixed EOL, and a missing final newline. | The parallel path fans out over line chunks but preserves per-line terminators and order. | `tests/transform_equivalence.rs` |
| 7 | Correctness holds at the ten-billion-line design target: the sparse-index arithmetic stays exact and bounded, and arbitrary-position resolution / tail edits stay byte-exact. | Index checkpoint/memory math is unit-tested at 1e10 lines; the resolve/edit/save pipeline runs on a real million-line file with a tiny stride. Front-end line numbers stay exact below 2^53. | `tests/extreme_scale.rs`, `web/test/scale.test.ts` |

## Atomic save

Saving never leaves a half-written file where the target used to be. The core
save path (`EditSession::save_to_path`) is:

1. Stream the full new content to a **temporary sibling file** opened with
   `create_new` (so a stale temp can never be reused).
2. `flush` and **`fsync` the temporary file** so its bytes reach the disk.
3. **Atomically rename** the temp over the target.
4. **`fsync` the target's parent directory** so the rename itself is durable.

A reader therefore only ever observes the complete old file or the complete new
file — never a partial write. `tests/crash_recovery.rs` asserts the postcondition
(complete target, no temp residue, source untouched on a save-as).

## Crash recovery (write-ahead log)

While a file is open for editing, the edit overlay lives in memory. Its only
on-disk trace is the write-ahead log, which appends one record per committed
transaction. On restart:

- A **clean** log replays every committed edit and restores the exact pre-crash
  view (content and dirtiness).
- A **torn trailing record** — an append cut short by a crash, recognizable
  because it has no terminating newline — is ignored; the committed prefix still
  replays.
- A **corrupt** record (complete but unparseable) makes the whole log
  `Invalid`: recovery refuses rather than guessing, and the original file is
  left untouched.
- A **stale** log, whose base file changed underneath it, is refused so edits
  are never applied to the wrong content.

## Scratch and spill placement

Out-of-core work needs disk: dirty sessions materialize a worker input, the
external-merge sort spills runs, and uploads stage bytes. These go to a
**disk-backed scratch base**, never RAM-backed tmpfs, so a huge operation
cannot OOM/ENOSPC the way it would under Linux's default `/tmp`. Resolution
order:

1. `--scratch-dir DIR` (serve/gui) or `$AYAME_SCRATCH_DIR`.
2. `--cache-dir DIR/scratch` when a cache dir is given.
3. Otherwise a `scratch/` subdir of the per-user cache root (the same
   disk-backed location the index cache uses: `~/.cache/ayame`,
   `~/Library/Caches/ayame`, `%LOCALAPPDATA%\ayame`).
4. The OS temp dir only when no home/cache location is discoverable at all.

`sort`/`group` also accept `--spill-dir` to point the spill somewhere explicit
(e.g. the same volume as a very large input). Put the scratch base on a volume
with room for the answer plus its spill.

## Known non-guarantees

Honesty about the edges is part of the guarantee:

- **External modification of an open file.** Ayame memory-maps the file it is
  viewing. If another process rewrites that file underneath a live session, the
  in-memory view can be inconsistent until reload. The WAL's base-identity check
  detects this on recovery (reported as *stale*), but it is not prevented during
  a live session.
- **The `ASCII` label is lenient.** Text tagged `ASCII` is encoded as UTF-8
  rather than rejected, because ASCII is a UTF-8 subset. Byte-level abort on
  unrepresentable characters applies to the narrow codecs (Shift_JIS, EUC-JP).
- **Scale.** Extreme scale is verified in two halves (see guarantee 7): the
  ten-billion-line **index arithmetic** is unit-tested directly, and the
  resolve/edit/save **pipeline** is exercised on a real million-line file with a
  tiny stride. A full ten-billion-line file (~100 GB) is never materialized in
  CI, so that end-to-end case rests on the arithmetic plus the pipeline test,
  not on a physical fixture.

## Running the checks

```sh
cargo test --locked -p ayame-core
```

The data-integrity suite is the set of integration tests under
`crates/ayame-core/tests/`; they run alongside the crate's unit tests on every
`cargo test` and in CI.
