# Ayame ロードマップ

優先順位は常に **安定性 ＞ 簡潔強力 ＞ VSCode風UX ＞ Shift_JIS ＞ 巨大ファイル**。
容量の最低ラインは **100億行**。これは希望値ではなく、設計・テストで守る下限。
詳細な根拠は [DESIGN.md](DESIGN.md)。

## ✅ v0.1（実装済み）

- `ayame-core`: mmap、疎行インデックス（並列構築）、エンコーディング判定（UTF-8/Shift_JIS/EUC-JP）、ストリーミング検索、差分編集レイヤ、ストリーミング変換/置換。100億行容量ガード込み。ユニットテスト115件（core 85 + cli 30、CIで常時実行）。
- `ayame` CLI: `stat`/`head`/`tail`/`line`/`lines`/`search`/`gen`。
- `ayame serve`: ローカル Web エディタ（仮想化、行ジャンプ、検索、ステータスバー）。
- `ayame serve`: 行単位編集（置換/挿入/削除/undo/redo）、上書き保存 / 保存コピー、通常範囲選択、矩形選択、選択範囲置換。元ファイルを全読み込みせず、mmap base + 差分だけを保持。
- `ayame sort --out`、`sortdiff`、`replace --out`（`--jobs`/`--chunk-lines` による行境界chunk並列一括置換）、`case upper|lower --out`。Web エディタではソートを現在ファイルへ上書き、置換/ケース変換を別ファイル保存。
- ベンチ: 3億行/14 GiB を 2.3 秒で索引、索引 2 MiB、ランダム行 0.61 ms。

## 🎯 v1 最小増分（次の4ステップ）

「目標アーキテクチャを一気に作らない」。各ステップは失敗時に v0.1 動作へ安全にフォールバックする。

1. **プロセス隔離（安定性の証明）** — ✅ **実装済み（op ワーカー版）**
   `ayame serve` は重い op（search/sort/replace/case/split）を `--worker` 相当の**使い捨て子プロセス**（`current_exe` を re-exec）で実行。ワーカーが**捕捉不能な SIGABRT でも**落ちると当該リクエストは 502、エンジンと `/api/lines`（ビューポート）は 200 を維持。`AYAME_WORKER_CRASH` フックで決定的に実証（`scripts/crash-isolation-test.sh`、10/10 PASS）。**未了**: Tauri window が axum ホスト自体を監督する案（ホストプロセス死＝DESIGN §4.1 の case 3）はこの環境では GUI 検証不可のため後続。
2. **ディスクキャッシュ（オフロードの証明）** — ✅ **実装済み**
   `LineIndex::to_bytes/from_bytes`（FNV-1a checksum trailer 付き）、content-addressed キャッシュ（`hash(canonical_path+size+mtime+stride)`）、`O_EXCL` 単一ライタロック＋tmp→atomic-rename。`Document::open` がキャッシュ参照、ミス/破損/失効は `LineIndex::build` へ安全にフォールバック。CLI は既定ON（`--no-cache`/`--cache-dir`/`ayame cache {path,info,clear}`）。実測: 構築 24ms → 再オープン 0ms。
3. **使い捨て子プロセスのワーカー** — ✅ **実装済み（Web: search/sort/replace/case/split、CLI: group/top/distinct）**
   `/api/search` と `/api/{sort,replace,case,split}/save` は子プロセスを spawn→wait→exit、結果は JSON / artifact ファイルでハンドオフ。CLI の `group` / `top` / `distinct` も同じ core ops を使う。ワーカー timeout 付き。ハートビートも IPC フレーミングも無しの最小形。
4. **外部マージ SORT** — ✅ **実装済み**（`ayame-core::ops::sort` ＋ `ayame sort`）
   明示メモリ予算でラン生成（`par_sort_unstable`）→ ディスク spill → fan-in 64 の多段ヒープ k-way マージ → 順序保存の `Vec<u64>` 順列。数値は順序保存エンコード、文字はデコードして **NFC 正規化済みコードポイント順**（Shift_JIS も正しい順序）、列指定（`-k`/`--delim`）・降順（`-r`）。実測: 500万行を 16 MiB 予算で 15 ラン・95 MiB spill・3.25 秒。**未了**: エディタでの仮想順列表示。

## 📦 release readiness

- ✅ CI: fmt / clippy `-D warnings` / tests / release build。
- ✅ GitHub Releases: タグ push で Linux / Windows / macOS の単体バイナリと sha256 を生成。
- ✅ Native app gate: `--features gui` で OS WebView（WKWebView / WebView2 / WebKitGTK）を使う単体アプリを生成。`ayame <FILE>` でファイル関連付け起動にも対応。
- ✅ Local package: `scripts/release-local.sh` でネイティブアプリ版 `dist/ayame-v<version>-<target>` を生成。
- ✅ 単体バイナリ検証: `--version`、`gen/stat/group --out-groups/distinct`、checksum、crash isolation。

## 🔭 v2 以降（将来）

- ✅ **GROUP-BY / TOP-N / DISTINCT / CSV欄モデル** 実装済み（`ayame-core::ops` ＋ `ayame group|top|distinct`）:
  - GROUP-BY: メモリ内ハッシュ集計＋予算超過で部分集計 spill→k-way マージ。count/sum/min/max/avg。
  - TOP-N: 有界 O(N) ヒープ（上位/下位、数値/文字）。
  - DISTINCT: HyperLogLog（2^p レジスタ、既定 p=14＝16 KB・誤差 ~0.8%、基数に依らず一定メモリ）。
  - CSV欄モデル: `csv-core` で RFC-4180 クォート（区切りを含む `"a,b"`、`""` エスケープ）。`--csv` で有効化。
  - `serve`: 現在は search/sort/replace/case/split を worker 隔離。GROUP-BY / TOP-N / DISTINCT のブラウザ操作化は後続。
  - **未了**: 引用フィールド内の**埋め込み改行**（1物理行=1レコード前提）、ホットパーティション再分割、per-group の distinct（HLL）、ブラウザ UI への操作パネル統合。
- ✅ **GUI diff確認** 実装済み:
  - 行単位 diff、bounded resync window、出力 hunk/line 上限。
  - ✅ GUI side-by-side: 差分モーダルで現在バッファ（未保存編集込み）と比較先ファイルの hunk preview を左右表示（1hunk 既定80行、API上限500行）。
  - ✅ inline word diff: `replace` hunk の同位置行に単語トークン差分を重ねて表示（長大行は自動で行単位表示へフォールバック）。
  - CLI は同じエンジンの検証用サブコマンドとして残置（製品導線は GUI）。
  - **未了**: directory diff、巨大差分 artifact 化。
- **OTP風 supervisor**（長命プールのハートビート/バックオフ）、`ayame-ipc`（bincode フレーミング）。
- ✅ **キャッシュ GC** 実装済み（`ayame cache gc --max-size --max-age-days --dry-run`）。
- **キャッシュ GC の高度化**（低ディスク時デグレード、artifact/job cache への拡張）。
- **DuckDB 任意バックエンド**（feature-gated）: `read_csv_auto` で多キー GROUP BY・JOIN・SQL を pushdown。重い分析にコミットした時だけ列投影 DB を構築。
- **日本語の言語的照合**（locale collation）、UTF-16 索引対応。
- **メモリ上限のハードニング**（cgroup v2 / Job Object、または `MAP_NORESERVE`）。
- **インクリメンタル索引**（構築中の先頭から閲覧開始）、巨大ファイルの **tail -f 追従**。

## 🚧 意図的に「やらない／後回し」

- **巨大ファイル編集の高度化**。行単位差分編集、undo/redo、上書き保存、別名保存、範囲選択、矩形選択、選択範囲置換、別ファイルへの全体置換/ケース変換は実装済み。次は append-only 編集 WAL / piece table の永続化へ進める。**フルインメモリ rope は作らない**。
- v1 段階での cgroup RSS 上限、syscall チューニング（fadvise/fallocate）、DuckDB —— いずれも「守る対象」が出来てから。
