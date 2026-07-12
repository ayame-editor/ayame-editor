# アーキテクチャ

*English: [../ARCHITECTURE.md](../ARCHITECTURE.md)*

Ayame は Rust workspace です。小さな browser UI を Rust binary に埋め込み、
CLI、`ayame serve`、ネイティブデスクトップウィンドウが同じ engine を使います。

## 実行時の形

```text
ネイティブウィンドウまたはブラウザ
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

| Path | 役割 |
| --- | --- |
| `crates/ayame-core` | mmap document access、sparse line index、encoding、search、edit overlay、save logic、transforms、split、aggregate operations、WAL recovery。 |
| `crates/ayame-cli` | CLI subcommands、local server、embedded web assets、任意の native window、synthetic data generation、binary entry point。 |
| `xtask` | release と type generation check の repo automation。 |

## Core Engine

`Document` はファイルを開き、文字コードと改行コードを検出し、sparse line index を
作り、bytes を memory-map します。index は全行ではなく定期 checkpoint を保存し、
近い checkpoint から newline を走査して bounded random access を実現します。

検索と viewport read は必要な範囲だけを decode します。sort、replace、case
conversion、split、group、top-N、distinct count は streaming または bounded-memory
operation として実装され、元ファイル全体を RAM に載せません。

## Edit Session と WAL

編集中も base file は immutable のままです。`EditSession` が変更済み logical lines、
undo/redo history、selection-aware batch edits、save state を overlay として持ちます。

Crash recovery は `crates/ayame-core/src/wal.rs` が担当します。

- committed edit transaction を JSON Lines として追記します。
- log header は base file を length、mtime、encoding で識別します。
- compaction は full overlay snapshot を書き、古い transaction を退避します。
- save と revert は base file identity が変わるため log を reset します。
- crash で切れた末尾 record は無視しますが、完結した malformed record は invalid として扱い、recover を黙って途中で切りません。

local server は active session にだけ live WAL writer を attach します。background tab は
WAL writer を持たない clone session なので、同じ edit stream を二重記録しません。

## Local Server

`ayame serve` と native GUI はどちらも
`crates/ayame-cli/src/serve/mod.rs` の同じ Axum router を使います。

| Module | 役割 |
| --- | --- |
| `assets.rs` | embedded HTML、CSS、TypeScript modules、favicon、logo、background assets を配信。 |
| `state.rs` | workspace lock、active document、tabs、edit session、WAL setup/recovery、save snapshots、cleanup。 |
| `edit.rs` | viewport lines、replace range/batch/rectangle、undo/redo、save、selection save、revert、reopen encoding、WAL recovery endpoints。 |
| `ops.rs` | search、grep、find、sort-save、replace-save、case-save、split-save endpoints。 |
| `workspace.rs` | open/new/upload/browse/tabs operations と scratch-file cleanup。 |
| `security.rs` | loopback default、Host/Origin check、remote-bind guard、DNS-rebinding / CSRF protection。 |

browser はファイル全体を受け取りません。`/api/lines` は 1 回の viewport request を
cap し、長い処理は専用 endpoint または blocking task に逃がします。

## API Surface

| Endpoint group | 目的 |
| --- | --- |
| `/api/stat`, `/api/lines`, `/api/linebyte` | file metadata と viewport navigation。 |
| `/api/search`, `/api/find`, `/api/grep` | search。 |
| `/api/edit/*` | replace、save、undo/redo、revert、recover、別 encoding で reopen。 |
| `/api/sort/save`, `/api/replace/save`, `/api/case/save`, `/api/split/save` | GUI から実行する長い file operations。 |
| `/api/open`, `/api/new`, `/api/upload`, `/api/browse`, `/api/tabs*` | workspace と tab management。 |
| `/api/tail/poll` | 追記ファイルの tail-follow polling。 |

## Web UI

TypeScript source は `crates/ayame-cli/web/src` にあります。Cargo build 時に
`build.rs` が型を落とした JavaScript を埋め込むため、通常の Rust build に Node
bundler は不要です。

UI は visible lines と viewport 周辺 cache だけを保持します。settings、recent
files、最後に開いた in-app file-picker directory、custom key bindings は browser local storage に保存します。
menu と keyboard shortcuts は action id で dispatch されるため、native macOS menu
と in-page UI が同じ command handler を呼べます。

## Type Generation

TypeScript mirror が必要な API request/response structs は `typegen` feature の
下で `ts-rs` derive を使います。

```sh
cargo run --locked -p ayame-cli --features typegen -- typegen --check
```

`cargo xtask typegen --check` も同じ flow を呼びます。CI は generated bindings と
committed web types が drift したら失敗します。

## Release Automation

`cargo xtask release` の local preflight:

1. branch、clean tree、upstream state を確認。
2. 必要なら workspace version を bump。
3. 明示的な `--skip-gate` がない限り gate を実行。
4. local artifacts を build して smoke test。
5. tag を作成して push。
6. GitHub Actions が cross-platform release artifacts を build。

GitHub release workflow は Windows、macOS Intel、macOS Apple Silicon、Linux の
GUI artifacts を build します。公開前に `AYAME_UPDATE_SIGNING_KEY` の Ed25519 鍵で
各 artifact checksum を署名し、自己更新側は署名と checksum の両方を検証します。

serve / GUI の scratch data は `--scratch-dir`（または
`AYAME_SCRATCH_DIR`、未指定時は platform の temporary directory）配下に置きます。
各 operation は予測不能かつ排他生成された private subdirectory を使います。`/tmp` が
tmpfs の環境で巨大ファイルを扱う場合は、disk-backed volume を指定してください。
