# Architecture

*日本語版: [ja/ARCHITECTURE.md](ja/ARCHITECTURE.md)*

Ayame is a Rust workspace with a small browser UI embedded into the Rust binary.
The same engine powers the CLI, `ayame serve`, and the native desktop window.

## Runtime Shape

```text
Native window or browser
        |
        | HTTP /api/*
        v
crates/ayame-cli/src/serve
        |
        | Document, Search, EditSession, WAL, transforms
        v
crates/ayame-core
        |
        | mmap + sparse index + cache files
        v
local filesystem
```

## Crates

| Path | Responsibility |
| --- | --- |
| `crates/ayame-core` | Memory-mapped document access, sparse line index, encoding, search, edit overlay, save logic, transforms, split, aggregate operations, and WAL recovery. |
| `crates/ayame-cli` | CLI subcommands, local server, embedded web assets, optional native window, synthetic data generation, and release-facing binary entry point. |
| `xtask` | Repository automation for releases and type generation checks. |

## Core Engine

`Document` opens the file, detects encoding and line endings, builds a sparse
line index, and memory-maps the bytes. The index stores periodic checkpoints
instead of every line, then walks newlines from the nearest checkpoint for
bounded random access.

Search and viewport reads decode only the requested ranges. Sorting, replacing,
case conversion, splitting, grouping, top-N, and distinct-count operations are
implemented as streaming or bounded-memory operations so the original file does
not need to fit in RAM.

## Edit Session and WAL

The base file remains immutable while editing. `EditSession` stores an overlay
of changed logical lines, undo/redo history, selection-aware batch edits, and
save state.

Crash recovery is handled by `crates/ayame-core/src/wal.rs`:

- every committed edit transaction is appended as JSON Lines;
- the log starts with a header that identifies the base file by length, mtime,
  and encoding;
- compaction writes a full overlay snapshot and retires older transactions;
- save and revert reset the log because the base file identity changed;
- torn trailing records are ignored, but complete malformed records make the log
  invalid instead of silently truncating recovery.

The local server attaches exactly one live WAL writer to the active session.
Background tabs use cloned sessions without WAL writers so they cannot double-log
the same edit stream.

## Local Server

`ayame serve` and the native GUI both build the same Axum router in
`crates/ayame-cli/src/serve/mod.rs`.

Important modules:

| Module | Role |
| --- | --- |
| `assets.rs` | Serves embedded HTML, CSS, TypeScript modules, favicon, logo, and background assets. |
| `state.rs` | Workspace lock, active document, tabs, edit session, WAL setup/recovery, save snapshots, and cleanup. |
| `edit.rs` | Viewport lines, replace range/batch/rectangle, undo/redo, save, selection save, revert, reopen encoding, and WAL recovery endpoints. |
| `ops.rs` | Search, grep, find, diff, sort-save, replace-save, case-save, and split-save endpoints. |
| `workspace.rs` | Open/new/upload/browse/tabs operations and scratch-file cleanup. |
| `security.rs` | Loopback defaults, Host/Origin checks, remote-bind guard, DNS-rebinding and CSRF protection. |

The browser never receives the whole file. `/api/lines` caps each viewport
request, and longer operations run through dedicated endpoints or background
blocking tasks.

## API Surface

The main endpoint groups are:

| Endpoint group | Purpose |
| --- | --- |
| `/api/stat`, `/api/lines`, `/api/linebyte` | File metadata and viewport navigation. |
| `/api/search`, `/api/find`, `/api/grep`, `/api/diff` | Search and comparison. |
| `/api/edit/*` | Replace, save, undo/redo, revert, recover, and reopen with another encoding. |
| `/api/sort/save`, `/api/replace/save`, `/api/case/save`, `/api/split/save` | Long-running file operations exposed from the GUI. |
| `/api/open`, `/api/new`, `/api/upload`, `/api/browse`, `/api/tabs*` | Workspace and tab management. |
| `/api/tail/poll` | Tail-follow polling for appended files. |

## Web UI

The TypeScript source lives in `crates/ayame-cli/web/src`. Cargo embeds the
type-stripped JavaScript through `build.rs`, so ordinary Rust builds do not need
a Node bundler.

The UI keeps only visible lines plus a cache around the viewport. Settings,
recent files, explorer root, and custom key bindings are stored in browser local
storage. Menus and keyboard shortcuts dispatch named action ids, which allows the
native macOS menu and in-page UI to call the same command handlers.

## Type Generation

API request/response structs that need TypeScript mirrors derive `ts-rs` behind
the `typegen` feature. The check is:

```sh
cargo run --locked -p ayame-cli --features typegen -- typegen --check
```

`cargo xtask typegen --check` wraps the same flow. CI fails if generated
bindings drift from the committed web types.

## Release Automation

`cargo xtask release` performs the local release preflight:

1. verify branch, clean tree, and upstream state;
2. optionally bump the workspace version;
3. run the configured gate unless `--skip-gate` is explicit;
4. build and smoke-test local artifacts;
5. create and push a tag;
6. let GitHub Actions build cross-platform release artifacts.

The GitHub release workflow builds GUI artifacts for Windows, macOS Intel,
macOS Apple Silicon, and Linux.
