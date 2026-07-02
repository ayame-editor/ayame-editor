# Ayame(菖蒲)設計ドキュメント — 巨大テキスト編集・検査ツール

- ステータス: ドラフト（設計合意用 / v0.1 実装済みコアの上に積む計画）
- 対象規模: **最低 10^10 行** / 数百GB〜TB のログ・CSV/TSV/JSONL・データ移行ダンプ
- 最優先要件（この順序が全判断を貫く）:
  **安定性（クラッシュ前提で設計）** ＞ 簡潔かつ強力 ＞ VSCode風UX ＞ Sakura系譜（Shift_JIS）＞ 巨大ファイルを快適に開く
- 既存資産: `ayame-core`（mmap + 疎行インデックス + 検索 + 差分編集）, `ayame-cli`（CLI + axum Web エディタ）— **書き直さず、上に積む**

> このドキュメントは、6本の独立分析を並列実行し、敵対的レビュー（adversarial critique）で検証・是正したうえでまとめたものです。レビューで削った/直した項目は §10・§11 に明記しています。

---

## 0. 結論サマリ（TL;DR）

| 論点 | 決定 | 一言理由 |
|---|---|---|
| 既存ツールで足りるか | **足りない。作る価値あり。スコープは「巨大ファイルエディタ＋オフロード型データ操作」に限定** | OSS＋クロスPF＋VSCode風GUI＋TB級O(index)メモリ＋並列ops＋クラッシュ隔離＋Shift_JIS の交差点を満たす単一ツールは存在しない |
| 言語 | **Rust 継続**（Go 不採用） | GoのGCがメモリ北極星と衝突。コアは既にRustで動作・テスト済み |
| GUI | **Tauri 2 ＋ 既存Web UI再利用** | OS webviewでChromium非同梱、Rustコア直リンク、クロスPF |
| 安定性 | **プロセス隔離＋有界予算のops** が本丸。落ちたら**画面は残し**、ワーカーだけ再起動 | プロセス境界がOOM/暴走/パニックの真の安全網 |
| sort/group | **段階的**: grep/top-nは自作、外部sort/hash group-byも自作。DuckDBは将来の任意バックエンド | op毎の予算・部分結果・ジョブ隔離は自作でしか得られない |
| DBへのpushdown | **将来の二級・任意機能**（反復クエリ時のみ列投影） | フル投影はファイルの1.5-3xの書き込み増幅。常道はDBコストゼロ |
| ディスクオフロード | **`ayame-cache`: content-addressed blob ＋ マニフェスト、TTL+LRU+上限** | キャッシュは純アクセラレータ。欠落時はRAM再計算へデグレード |
| SSD配慮 | **append-only/atomic-rename・大ブロック・バッチコミット・in-place再書き込み禁止** | 摩耗の危険は「量」でなく「パターン（書き込み増幅）」 |

---

## 1. 既存ツール調査と差別化（なぜ作るのか）

各候補を Ayame の要件軸（安定性 / 簡潔強力 / VSCode風UX / Shift_JIS / 巨大ファイル / データ操作 / クラッシュ隔離）に当てると、**いずれも2軸以上で落ちる**。

| ツール | メモリモデル | 巨大対応 | データ操作 | GUI | Shift_JIS | OSS/クロスPF | 致命的欠落 |
|---|---|---|---|---|---|---|---|
| **Zed**（Rust） | インメモリrope | ✗ 10GBに>64GB、6GB以上を**ハード拒否** | △ | ◎ | ◎ | 巨大不可（=Ayameの動機） |
| Sublime / Notepad++/NotepadNext | 全読み込み（Scintilla常駐） | ✗（数GBで破綻、4x RAM） | ✗ | ◎ | △ | 巨大不可 |
| **EmEditor** | tempディスク | ◎ 248GB/2.1B行・並列sort | ◎ | ◎ | ◎ | **Windows専用・クローズド・商用**（監査不可＝Zedトラウマと矛盾） |
| klogg / glogg | ディスクから読む | ◎ 16GBを〜30sで索引 | ✗（sort/group無） | C++/Qt | △ | データ操作・編集なし |
| lnav | 内容を非常駐 | ◎ ログをSQLite仮想表に | △（SQLのみ） | TUI | △ | 端末ログ専用 |
| DuckDB | out-of-coreスピル | ◎ 10億行集計をラップトップで | ◎ | ✗ | △（UTF-8前提） | headless、耐障害UI無 |
| qsv（Rust）/ Miller（Go） | 外部マージ/1行常駐 | ◎ | ◎（CSV/JSON） | ✗ | △ | 純CLI |
| VisiData（Py） | 非同期ストリーム | △（フィルタ集合をRAM保持） | ◎ | TUI | △ | Python TUI、実用上限 |

**差別化の核:** どのツールも「**計算/ワーカーが落ちてもビューポートが残り、閲覧を継続できる**」プロセス隔離耐障害を持たない。これが Ayame 最大の差別化で、OSS＋クロスPF＋GUI＋Shift_JIS と並ぶ。

**容量の非交渉ライン:** Ayame は「数GBを開けるツール」ではなく、**最低100億行**を設計下限に置く。既定 stride 4096 では 100億行の疎インデックスは 2,441,407 checkpoints × 16B = 39,062,512B（約37.3 MiB）。この計算は `MINIMUM_SUPPORTED_LINES` とユニットテストで固定する。

**正直なトレードオフ:** 各軸は既存ツールが個別に解決済み。Ayame の価値は**統合・パッケージング**にある（技術的moatは薄い）。巨大ファイル編集は immutable mmap base（元ファイルを直接 mutable mmap しない土台）の上に差分レイヤを載せる。初期実装は行単位の置換/挿入/削除と保存コピーで、以降は undo/redo・範囲編集・上書き保存へ広げる。

一行ポジショニング: **「クラッシュしないUIを持つ、オープンソースのクロスプラットフォーム版 EmEditor（将来はDuckDBエンジン同梱）」**。

---

## 2. 言語選定: Rust 継続（Go 不採用）

| 観点 | Rust | Go | Ayameでの判定 |
|---|---|---|---|
| メモリ決定性 | GCなし。39MBインデックスは39MBのまま | GC（GOGC=100で最大〜2xライブヒープ、ページ返却が遅延） | **GCが北極星 O(index+viewport+hits) と衝突** → Rust |
| mmap+SIMD | memmap2 + memchr（AVX2/SSE2/NEON）、自コードに`unsafe`不要 | 標準にSIMD memchr無、cgo越え〜数十〜数百ns/call | **毎行ループでcgo税が致命的** → Rust |
| データ並列 | rayon ワークスティーリング（CPUバウンドのチャンクスキャンに最適） | goroutineは並行だがデータ並列はrayon優位 | Rust |
| クラッシュ特性 | null無、データ競合はコンパイルエラー、panicはunwindして隔離境界でcatch可 | nilデリファレンス/concurrent map write がプロセス全体を落とす | **要件#1に決定的** → Rust |
| Shift_JIS/EUC-JP | encoding_rs + chardetng（ネイティブ、FFIゼロ） | cgoでICU級コーデック（C toolchain必要） | Rust |
| 実装済み資産 | `ayame-core` 動作・テスト済み | ゼロから書き直し | **書き直しは要件#1に純粋な負価値リスク** → Rust |

決定打: ルート [`Cargo.toml`](../Cargo.toml) の `[profile.release]` に「**意図的に `panic = "abort"` を採用しない**（per-request panic を unwind で隔離する）」と明記済み。**unwindベースのパニック隔離は既に安定性の中核として作動している**。Zed も同じ Rust だが巨大ファイルで落ちる事実が、**差別化は言語でなくメモリモデル**であることを裏づける。

> 他言語の検討（Zig/Erlang/OCaml/Motoko 等）: Zig は2026年時点でまだ pre-1.0（破壊的変更が続く＋borrow checker無＝手動メモリ安全）で「安定第一」に反する。Erlang/OTP は**障害耐性の思想**（let-it-crash＋supervision）を借りるが、GC付きVMでCPUバウンドな100GB走査には不向き → 思想だけ Rust の OS プロセスで実装。OCaml は堅牢だがGC＋この用途のライブラリ/GUIが手薄。Motoko は ICP（ブロックチェーン）専用でローカルファイルに触れず**対象外**。結論は「Rust＋OTP風アーキテクチャ」。

---

## 3. GUI 選定: Tauri 2 シェル ＋ 既存 Web UI 再利用

- **採用: Tauri 2**。OS webview 利用（macOS WKWebView / Windows WebView2 / Linux WebKitGTK）で Chromium 非同梱 → シェル数MB（Electron〜150MB に対し）。Rust ネイティブで `ayame-core` を直リンク。既存の VSCode 風・仮想化 Web UI を**書き直さず流用**。
- **不採用:** GPUI（Zed専用61k行＝差別化対象の複雑性を取り込むだけ）、egui/Slint（VSCode風UXを一から再構築）、Fyne/Wails（Goホスト＝GCのデメリット再導入）。

UI は仮想化テキストリストで CSS 依存が薄く、webview 横断の描画差異リスクは低い。ボトルネックは描画でなくエンジン側。

---

## 4. 安定性の核心 — クラッシュ分類とプロセス隔離

> ここがレビューで最も是正された箇所。「落ちても画面が残る」を**正確に**定義する。

### 4.1 クラッシュ分類（誰が落ちると何が起きるか）

| # | 落ちる対象 | 起こること | 復旧 | 実装コスト |
|---|---|---|---|---|
| 1 | **タスクワーカー**（sort/index/grep の使い捨て子プロセス） | UI 無影響。当該ジョブのみ失敗 | supervisor が再起動 or エラートースト→ユーザ再試行 | **安く、最重要の勝ち** |
| 2 | **温存プールワーカー**（ビューポート取得・逐次検索） | ビューポートが <1s 停止 | 再spawn | 中 |
| 3 | **axum/ホストプロセス**（エンジン本体） | Tauri window は**最後に描画したビューポートを表示し続ける（凍結）**＋「再接続中」表示 | **Tauri がサイドカーを再起動** | 中 |

**重要な訂正:** 「アプリが落ちても画面が残る」を担保するのは**Tauri window とエンジン（サイドカー）のプロセス分離**であって、ディスクjournal“だけ”ではない。ホストが死ねば新データは出せない（凍結表示＋再接続）。よって **Tauri がホストを監督する**最上位の supervisor になる。journal は「ワーカー結果・現在ビューポートの保全」に効くのであって、ホスト死を透過にはしない。

### 4.2 真の安全網 = 「有界予算のops」＋「プロセス境界」

安定性を生むのは2つだけ:
1. **有界予算で動く ops**（external-merge / partitioned-hash は設計上 RAM 上限 B で動く）。これが**第一の保護**。
2. **ハードなプロセス境界**。暴走・OOM・abort 級パニック（スタックオーバフロー/二重パニック/FFI）でも、カーネルが当該ワーカーのページを回収し OOM killer はそのワーカーだけを狙う。UI（mmap ビューポート）は生存。

`catch_unwind`（ワーカー内）は**第二線**（unwind 可能な panic を捕捉、`tower-http` CatchPanicLayer を流用 [`serve/mod.rs`](../crates/ayame-cli/src/serve/mod.rs)）。abort 級は捕えられないので、最後の砦は**プロセス境界**。

> **cgroup v2 `memory.max` / Windows Job Object による RSS 上限は v1 の保証にしない（レビュー指摘）。** 通常のデスクトップ起動では cgroup への自己移動に権限委譲が要り、いつでも掛けられるとは限らない。v1 は「有界予算ops＋プロセス境界」で守り、RSS 上限は**後段のハードニング**として位置づける。

### 4.3 プロセス spawn コストの正当化

Linux のプロセス spawn は〜1-2ms。数秒の sort や分単位の全インデックスビルドに対し無視可能。だが**ビューポート取得は <16ms（60fps）必須**なので、毎回 spawn せず**温存プール**で処理。疎データゆえ耐障害ジャーナルが安価（インデックス=16B×ceil(行/4096)、ビューポートスナップショット=数百行=KB）。

全ワーカーは**単一バイナリの `--worker <role>` 自己 re-exec**（Chrome/Zed 方式）。配布は単一実行ファイルのまま。

---

## 5. データ操作 — 段階的（tiered）設計

「自作 vs DuckDB委譲」を二者択一にせず階層化する。

| op | 実装 | アルゴリズム | メモリ | 流用元 |
|---|---|---|---|---|
| **GREP** | 自作（最優先・最低リスク） | `search::Matcher`(memmem/regex) を rayon par_iter でチャンク並列、**有界**チャネルへhit | O(hits+viewport) | [`search.rs`](../crates/ayame-core/src/search.rs) ほぼ流用 |
| **TOP-N** | 自作 | per-thread `BinaryHeap` サイズN、最後にマージ | O(N×threads) | — |
| **SORT** | 自作（外部マージ） | run生成（予算B=既定512MB-1GB）→ `par_sort_unstable`→ ≥1MiB連続書き込みでスピル、k-way merge（fan-in 64、超過は多段） | O(B + fan-in) | `index.rs::build` のチャンク化 |
| **GROUP-BY** | 自作（分割hash） | key を P 個（256-1024）のスピルパーティションへ hash、各パーティションを並列集計 | O(B + P) | — |
| **重い分析/SQL**（将来） | **DuckDB（feature-gated・任意）** | `read_csv_auto`（コピーなし）で多キーGROUP BY・JOIN | DuckDB自管理 | `duckdb-rs` |

### 5.1 ソート照合（Shift_JIS の訂正）

> **レビュー高優先の訂正:** Shift_JIS/CP932 の**生バイト順は JIS でも Unicode でもない**（多バイト先頭/後続バイトが ASCII/かなと交錯する）。「ホットパスはバイトのまま比較」は **grep/ビューポートでは正しいが、SORT/GROUP-BY キー比較では誤り**。

v1 の方針:
- **キー欄だけ**を UTF-8 にデコードし **NFC 正規化したキー**でソート（＝コードポイント順、正しく決定的）。デコードコストは行全体でなくキーのみ。
- v1 の並び順は明示的に「**コードポイント順であって言語的照合（locale collation）ではない**」とラベルする。日本語の辞書順照合は後段マイルストーン。

### 5.2 結果は「仮想順列」として閲覧（ゼロコピー）

各 op 結果は **ordering（スピルした `Vec<u64>` の行番号列）**として実体化。UI は既存の `index.line_ranges`（[`index.rs`](../crates/ayame-core/src/index.rs)）経由でその順列を見るので、**100億行のソート結果もデータコピーなし**で同じ疎フェッチ経路から表示できる。

### 5.3 CSV/TSV フィールドモデル（レビューで補完）

疎インデックスは**行**指向。sort/group-by はキー**欄**が要るので、フィールド分割が必要:
- v1: 各行アクセス時に `csv-core`（RFC-4180 クォート対応）で分割（再分割コストは許容）。
- 将来: 必要なら「疎フィールドインデックス」を別途持つ（スループット改善）。

### 5.4 バックプレッシャ（レビューで補完）

1TB の grep が webview の消費より速くヒットを生むと、SSE 送信キューに**無限**に溜まり「有界メモリ」が破れる。対策: 有界チャネル＋**drop/pause ポリシ**（送信キューが閾値超で生産側を一時停止、UI は「N件超、絞り込み推奨」を表示）。

---

## 6. ディスクオフロード ＋ キャッシュ管理（SSD配慮）

### 6.1 方針: `ayame-cache` クレート

- ルート: XDG キャッシュ（`~/.cache/ayame/v1/`、`AYAME_CACHE_DIR` で上書き、0700）。
- 構成: **content-addressed イミュータブル blob** ＋ **小さな単一マニフェスト**（rusqlite bundled）。
- キャッシュキー: `blake3(canonical_path) ‖ size ‖ mtime_ns` **＋ encoding/stride オーバライド**（レビュー補完：別 encoding で開いた snapshot の行番号は無効になるため、キーに含める）。
- **疎インデックスは常時キャッシュ**（16B×checkpoint、4096行毎で 10^10 行=〜39MB = 6TBファイルの〜0.0006%）。再オープンを数秒のビルドから **mmap＋検証**へ。
- グローバル上限: **空き容量の5%、[2GiB, 64GiB] にクランプ**。TTL **14日**（未使用）＋ **LRU** 退避（開いているドキュメントの artifact は退避しない）。
- 全 blob は**ソースから再構築可能** → クラッシュ・削除・読取専用/リモートソース・ディスク満杯は「RAM 再計算」へ**デグレードして失敗しない**。

### 6.2 整合性・並行性（レビューで補完）

- **インデックス blob の検証は header だけでは不十分**。`line_count` ＋ **checksum trailer**（checkpoint 配列をカバー）を付け、truncated/部分書き込みを検出してから random access に信用する。検証失敗＝キャッシュミス扱いで `LineIndex::build` にフォールバック。
- **複数プロセス/ウィンドウの競合**: 同一巨大ファイルを2プロセスが開くと同じ blob を二重ビルドし得る。キー毎に **`O_EXCL` の `.building` ロックファイル**（single-writer）。先行者がビルド中なら待つ/RAM ビルドにフォールバック。
- stale 判定: `size + mtime` 再stat。`rsync --times` 等で mtime 保存される稀ケース向けに先頭+末尾 64KiB の blake3 をタイブレーカ（コスト次第で採否）。

### 6.3 SSD摩耗対策（書き込み増幅の最小化）

危険は**量でなくパターン**。消費 TLC SSD は〜600 TBW/1TB（0.3 DWPD×5年）。仮に 20GB/日 のチャーンでも 7.3TB/年=予算の〜1.2%/年で安全 — **書き込み増幅を避ければ**。

| 対策 | 内容 |
|---|---|
| **append-only / atomic-rename** | `<key>.tmp.<pid>` に大バッファで書き、末尾で1回 fsync、rename で配置 |
| **in-place 再書き込み禁止** | インデックス全体の上書きをしない（blob はイミュータブル） |
| **大ブロック書き込み** | スピルは ≥1MiB（できれば1-4MiB）の連続書き込み |
| **バッチ・マニフェスト更新** | record 毎 fsync を避け操作毎に1コミット（WAL + `synchronous=NORMAL` + `busy_timeout`） |
| **GC のスロットル** | 削除は tick 毎 N unlink 上限＋yield。起動時（孤児 `.tmp` 回収兼）/アイドル/明示 `ayame cache gc` |
| **低ディスク処理** | 書き込み前に空きを確認、不足なら ops を RAM-only/ストリーミングへ切替、警告のみで**ユーザ操作は失敗させない** |

---

## 7. 既存コアとの統合（書き直しゼロ）

`ayame-core` は計算カーネルとして不変。新規クレートを足す:

| 新規 | 役割 | 既存への接続点 |
|---|---|---|
| `ayame-core::ops` | budget/spill/chunk/grep/sort/groupby/topn | `index.line_ranges`/`line_of_byte`/`search::Matcher` を流用 |
| `ayame-cache` | XDG root・blob・マニフェスト | `LineIndex::to_bytes/from_bytes`（checkpoints は 16B POD で直列化容易）追加、`Document::open` をキャッシュ対応化 |
| `ayame-worker` | `--worker <role>` re-exec 入口、各 op は `catch_unwind` 内 | `ayame-core` を内部呼び出し |
| `ayame-desktop` | Tauri 2 シェル | 既存 Web 資産を埋め込み、axum をサイドカー起動・監督 |

`Document::open` 改修: キー算出 → `ayame-cache` に有効インデックス blob を問い合わせ → hit なら `from_bytes`（mmap＋trailer検証）して `LineIndex::build` をスキップ、miss なら従来通りビルドして永続化。ops は文書を変更せず、ファイルをヒープにロードせず、`lib.rs` 記載の **O(index+viewport+hits) 不変条件**を保つ。

---

## 8. トレードオフの明文化（あなたの言語化どおり）

- **ディスク ↔ メモリ:** 安定性のためローカルディスクを**意図的に消費**（external sort は〜1x 入力をスピル=2x I/O）。代わりに OOM クラッシュを根絶。**受容済み。**
- **プロセス隔離 ↔ 簡潔さ:** 部品が増える。**単一バイナリ自己 re-exec**で配布は単一実行ファイルのまま緩和。
- **自作ops ↔ DuckDB:** 自作で予算・部分結果・隔離を得る（保守コスト増）。breadth は将来 DuckDB に委譲して両取り。
- **キャッシュ ↔ 摩耗:** インデックスは無条件キャッシュ（サイズ〜0.0006%）。書き込みは append-only＋バッチで摩耗を抑制。上限＋TTL で有界。

---

## 9. v1 最小増分（critique 主導の実行計画）

> 「目標アーキテクチャ」（§3-7）を**一気に作らない**。安定性を壊さず、差別化を最小コストで証明する4ステップ。各ステップは**キャッシュミス/クラッシュ時に現状動作へ安全にフォールバック**する。

1. **Step 1 — 「落ちる前提」を証明（最小）**
   既存 axum を**そのまま**サイドカー子プロセスとして起動し、前段に薄いホスト（Tauri window、または監督する親プロセス）を置く。エンジン子が落ちたら**最後のビューポートを保持**して「エンジン再起動中・表示は保持」を出し、再 spawn。**最初に書くテスト = SIGKILL 注入**（リクエスト中にエンジンを kill → window が最後の表示を保つことを assert）。これ一本が「designed to crash」命題の証明で、supervisor/IPC/spool は不要。

2. **Step 2 — ディスクオフロードを安全に証明**
   `LineIndex::to_bytes/from_bytes`（16B POD 配列で自明）＋ `line_count`＋checksum trailer を追加。`Document::open` を content-addressed キャッシュ（`blake3(path)+size+mtime+encoding+stride`）対応に。キー毎 `O_EXCL` ロックで二重ビルド回避。再オープンが「数秒ビルド」から「mmap＋検証」へ。**ミス/破損は単に `LineIndex::build` へフォールバック**＝動作中エディタにゼロリスク。

3. **Step 3 — ワーカー動物園なしで op を1つ** — 実装済み
   `/api/search` は `ayame search --json --start-byte` を使い捨て子プロセスとして起動し、JSON でヒットを受け取る。heartbeat も IPC フレーミングも無し（spawn→wait→exit code、落ちたらトースト→再試行）。同じ形を sort/group/top/distinct に展開済み。

4. **Step 4 —（1-3 が固まってから）外部マージ SORT**
   同じ使い捨て子プロセス形で、明示予算 B＋≥1MiB 連続スピル、結果は `Vec<u64>` 順列（エディタは `index.line_ranges` で表示＝ゼロコピー仮想順列）。v1 は **デコード＋NFC 正規化キー**でソートし「コードポイント順（言語的照合ではない）」と明記。

---

## 10. v1 以降へ意図的に先送りする項目（過剰設計の回避）

レビューが「安定性（要件#1）に対して、守る対象が存在する前に最も複雑で未実証な機構を先に作る倒錯」と指摘したもの。**v1 では作らない:**

- OTP風 supervisor（2sハートビート・指数バックオフ・MAX_RESTARTS）。使い捨てジョブは「spawn→wait→exitで判定」で足り、再接続state machineは長命プールにだけ後で。
- `ayame-ipc`（length-delimited bincode フレーミング）。最初の使い捨てワーカーは argv 入力＋結果パス出力＋exit code で十分。
- cgroup/Job Object の RSS 上限（ハードニングへ）。
- HyperLogLog・ホットパーティション再帰再分割・`posix_fadvise/madvise`・`fallocate`/スパースファイル等の syscall チューニング（各々が安定性面=未実証）。
- DuckDB バックエンド（大きな C++ 依存＋独自メモリマネージャ＝「有界・予測可能・隔離可能」と相反）。自作 grep/sort/group を先に。
- 「Supervisor 死も透過」という durable-spool 物語（§4.1 のとおり Tauri 分離が担う）。
- **編集エンジン高度化**。初期の行単位編集は入った。v1では最小の差分レイヤに留め、undo/redo、矩形選択、grep置換、上書き保存、永続 WAL は段階的に入れる。immutable mmap base は「元ファイルを直接 mutable mmap しない」という意味であり、編集は mmap base＋差分WAL/piece table で成立させる。

---

## 11. 未解決の問い（要追加検討）

1. 最低ライン（10^10行/TB級）の**実測検証**: 合成 100億行に向けた段階ベンチ（まず 50GB/10億行 JSONL）を EmEditor(Win VM)/klogg/lnav/DuckDB で計測し、「GUI＋ops＋O(index)メモリ＋OSS」の同時成立が他に無いことをエビデンス化。
2. ワーカーの**メモリ上限の実装方式**（RLIMIT_AS は mmap アドレス空間を計上＝1TB mmap で即超過。cgroup/Job Object か `MAP_NORESERVE`＋ヒープ制限か）。
3. DuckDB feature の **feature-gate 境界・CI/配布マトリクス**（既定ビルドを軽量・予測可能に保つ）。
4. **Shift_JIS 照合**の厳密仕様（既定の照合規則、元バイト保持とのラウンドトリップ保証）。
5. blob の **staleness タイブレーカ**（先頭+末尾 64KiB blake3）の費用対効果。
6. （将来）編集 WAL のスキーマと fsync 頻度（mmap base＋差分パッチの append-only、**フルインメモリ rope は決して作らない**）。
7. webview 横断の配布細部（WebKitGTK のフォント/バージョン差、WebView2 ランタイム同梱）。

---

## 付録: 主要エビデンス（出典）

レビューで実ソースに対し全て検証済み。

- **Zed のメモリモデル失敗（動機）:** `refs/zed-main/crates/worktree/src/worktree.rs:1514`「use in excess of 64GB for a 10GB file」, `:1520` `FILE_SIZE_MAX = 6GiB`, `:1524` `bail!("File is too large to load")`。
- **Zed 子プロセステアダウン:** `crates/lsp/src/lsp.rs:429` `.kill_on_drop(true)`。
- **Zed ハートビート/バックオフ:** `crates/remote/src/remote_client.rs:160-165`（HEARTBEAT_INTERVAL=5s 等）。
- **Zed の SQLite 設定（踏襲）:** `crates/db/src/db.rs:130-133`（WAL / NORMAL / busy_timeout）。
- **Ayame コア:** [`crates/ayame-core/src/index.rs`](../crates/ayame-core/src/index.rs)（16B Checkpoint, stride 4096, rayon 並列ビルド, line_ranges/line_of_byte）, [`search.rs`](../crates/ayame-core/src/search.rs)（Matcher）, [`document.rs`](../crates/ayame-core/src/document.rs)（open 統合点）, ルート [`Cargo.toml`](../Cargo.toml)（`panic=abort` 不採用）, [`serve/mod.rs`](../crates/ayame-cli/src/serve/mod.rs)（Shared state + CatchPanicLayer）。
