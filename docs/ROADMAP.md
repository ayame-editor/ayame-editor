# Ayame ロードマップ

優先順位は常に **安定性 ＞ 簡潔強力 ＞ VSCode風UX ＞ Shift_JIS ＞ 巨大ファイル**。
詳細な根拠は [DESIGN.md](DESIGN.md)。

## ✅ v0.1（実装済み）

- `ayame-core`: mmap、疎行インデックス（並列構築）、エンコーディング判定（UTF-8/Shift_JIS/EUC-JP）、ストリーミング検索。ユニットテスト17件。
- `ayame` CLI: `stat`/`head`/`tail`/`line`/`lines`/`search`/`gen`。
- `ayame serve`: ローカル Web ビューア（仮想化、行ジャンプ、検索、ステータスバー）。
- ベンチ: 3億行/14 GiB を 2.3 秒で索引、索引 2 MiB、ランダム行 0.61 ms。

## 🎯 v1 最小増分（次の4ステップ）

「目標アーキテクチャを一気に作らない」。各ステップは失敗時に v0.1 動作へ安全にフォールバックする。

1. **デスクトップシェル＋プロセス隔離（安定性の証明）**
   既存 axum をサイドカー子プロセス化し、Tauri window（または監督親）が前段に立つ。エンジンが落ちても**最後のビューポートを保持**して再 spawn。最初に **SIGKILL 注入テスト**を書く。
2. **ディスクキャッシュ（オフロードの証明）** — ✅ **実装済み**
   `LineIndex::to_bytes/from_bytes`（FNV-1a checksum trailer 付き）、content-addressed キャッシュ（`hash(canonical_path+size+mtime+stride)`）、`O_EXCL` 単一ライタロック＋tmp→atomic-rename。`Document::open` がキャッシュ参照、ミス/破損/失効は `LineIndex::build` へ安全にフォールバック。CLI は既定ON（`--no-cache`/`--cache-dir`/`ayame cache {path,info,clear}`）。実測: 構築 24ms → 再オープン 0ms。
3. **GREP を使い捨て子プロセスで**
   rayon 並列スキャン、結果＝行番号、spawn→wait→exit。ジョブ毎隔離と「結果を仮想順列として閲覧」を最低リスクで検証。
4. **外部マージ SORT** — ✅ **実装済み**（`ayame-core::ops::sort` ＋ `ayame sort`）
   明示メモリ予算でラン生成（`par_sort_unstable`）→ ディスク spill → ヒープ k-way マージ → 順序保存の `Vec<u64>` 順列。数値は順序保存エンコード、文字はデコードして**コードポイント順**（Shift_JIS も正しい順序）、列指定（`-k`/`--delim`）・降順（`-r`）。実測: 500万行を 16 MiB 予算で 15 ラン・95 MiB spill・3.25 秒。**未了**: NFC 正規化・多段マージ（ラン数が fan-in 上限超のとき）・使い捨て子プロセス隔離（Step 3 後）・ビューア統合。

## 🔭 v2 以降（将来）

- ✅ **GROUP-BY** 実装済み（`ayame-core::ops::group` ＋ `ayame group`）: メモリ内ハッシュ集計＋予算超過で部分集計 spill→k-way マージ。count/sum/min/max/avg。**未了**: TOP-N / DISTINCT（HLL）、CSV/TSV フィールドモデル（`csv-core` でのクォート対応）、ホットパーティション再分割。
- **OTP風 supervisor**（長命プールのハートビート/バックオフ）、`ayame-ipc`（bincode フレーミング）。
- **キャッシュ GC の高度化**（LRU+TTL+上限、低ディスク時デグレード、`ayame cache {info,gc,clear}`）。
- **DuckDB 任意バックエンド**（feature-gated）: `read_csv_auto` で多キー GROUP BY・JOIN・SQL を pushdown。重い分析にコミットした時だけ列投影 DB を構築。
- **日本語の言語的照合**（locale collation）、UTF-16 索引対応。
- **メモリ上限のハードニング**（cgroup v2 / Job Object、または `MAP_NORESERVE`）。
- **インクリメンタル索引**（構築中の先頭から閲覧開始）、巨大ファイルの **tail -f 追従**。

## 🚧 意図的に「やらない／後回し」

- **インプレース編集**（巨大ファイルの編集）。今のスコープは**ビューア＋データ操作**。編集を入れる時も**フルインメモリ rope は作らず** mmap＋append-only 編集 WAL で行う方針（Zed を沈めた構造を避ける）。
- v1 段階での cgroup RSS 上限、HyperLogLog、syscall チューニング（fadvise/fallocate）、DuckDB —— いずれも「守る対象」が出来てから。
