# Ayame（菖蒲）

**100億行クラスのテキストを、メモリを溢れさせず・落ちずに開く、ローカル確認用テキストツール。**

ビッグデータ移行の「最終的なローカルでのデータ確認」に耐えるツールが無い、という課題に対する答えです。
DuckDB のような優れた分析基盤はありますが、「巨大ファイルをそのまま開いて、目で見て、検索して、並べ替える」シンプルで安定したテキストツールが不在でした。Ayame はそこを埋めます。

- **安定第一**: ファイルをメモリに丸読みしない。メモリ使用は**ファイルサイズに依存しない**（O(索引＋表示＋ヒット)）。
- **Sakura の機能性 × VSCode の操作感**を志向。日本語エンコーディング（**Shift_JIS / EUC-JP / UTF-8**）を一級サポート。
- 既存エディタが巨大ファイルで落ちる理由は**言語ではなくメモリモデル**。Zed（同じ Rust）でさえ「10GB に 64GB 超」を使い 6GB 以上を拒否します（[根拠](docs/DESIGN.md#付録-主要エビデンス出典)）。Ayame は逆の設計です。

## いまできること（v0.1・実装済み）

- **メモリマップ＋疎行インデックス**: 4096 行ごとに 16 バイトのチェックポイント。**100億行でも索引は約 40–70 MiB**。任意行へ sub-ms でジャンプ。
- **エンコーディング自動判定**（BOM＋chardetng）: UTF-8 / Shift_JIS / EUC-JP / ASCII、CRLF/LF/CR 検出。
- **ストリーミング検索**: リテラル（SIMD `memmem`）＋正規表現。ヒットを行・桁にマッピング。
- **永続インデックスキャッシュ**: 一度開いた巨大ファイルの索引をディスク保存（content-addressed＋checksum trailer＋単一ライタロック、ソース変更で自動失効）。**2回目以降は「構築」→「mmap＋検証」でほぼ瞬時**（実測: 構築 24ms → キャッシュ 0ms）。`--no-cache` / `ayame cache {path,info,clear}`。
- **CLI**: `stat` / `head` / `tail` / `line` / `lines` / `search` / `gen` / `cache`。
- **ローカル Web ビューア**（`ayame serve`）: Rust が配信する VSCode 風の仮想化ビューア。**実行時に Node 不要**、webview も不要（ブラウザで開く＝最も安定）。数十億行をスクロールできるカスタムスクロールバー、行ジャンプ（Ctrl+G）、検索（Ctrl+F / F3）、ステータスバー。

実測（4 vCPU VM、**3億行 / 14.16 GiB**）: コールドで開く＋全索引 **2.3 秒**、索引メモリ **2.0 MiB**、ランダム1行 **0.61 ms**、全文スキャン **5.0 GiB/s**。詳細は [BENCHMARKS.md](BENCHMARKS.md)。

## クイックスタート

```sh
# ビルド（Rust 安定版）
cargo build --release

# 試しに大きな合成データを作る（約9.5 MiB / 20万行）
./target/release/ayame gen sample.csv --lines 200000

# 統計（サイズ・行数・エンコーディング・改行・索引情報）
./target/release/ayame stat sample.csv

# 検索（-i 大小無視, -e 正規表現, --max 上限）
./target/release/ayame search sample.csv 'error' -i --max 50

# GUI（ローカル Web ビューア）— 表示された URL をブラウザで開く
./target/release/ayame serve sample.csv --port 8777
```

日本語（Shift_JIS）データを作って確認する例:

```sh
./target/release/ayame gen sjis.csv --lines 100000 --encoding shift_jis
./target/release/ayame stat sjis.csv          # encoding: Shift_JIS と表示
./target/release/ayame serve sjis.csv          # ビューアは UTF-8 にデコードして表示
```

### Web ビューアの操作（VSCode 風）

| 操作 | キー |
|---|---|
| 行へ移動 | `Ctrl+G` |
| 検索 | `Ctrl+F` |
| 次/前の一致 | `F3` / `Shift+F3` |
| 大小無視 / 正規表現の切替 | `Alt+C` / `Alt+R` |
| 先頭 / 末尾へ | `Ctrl+Home` / `Ctrl+End` |
| ページ送り | `PageUp` / `PageDown` / `Space` |

## 設計とロードマップ

- **[docs/DESIGN.md](docs/DESIGN.md)** — 目標アーキテクチャ（言語選定、Tauri デスクトップ化、**クラッシュ前提のプロセス隔離設計**、ディスクオフロードと SSD 配慮、外部ソート/グループ/grep、SQLite/DuckDB への compute pushdown、トレードオフの明文化）。6本の独立分析＋敵対的レビューで検証済み。
- **[docs/ROADMAP.md](docs/ROADMAP.md)** — v1 最小増分から将来フェーズまで。

要約すると Ayame のトレードオフはこうです:
> **ローカルディスクを使ってメモリ負荷を下げ、安定性を買う**。キャッシュは定期的に・SSD に優しく掃除する（append-only・大ブロック・上限＋TTL）。重い集計は将来 DuckDB に流す（任意機能）。

## アーキテクチャ概要

```
crates/
  ayame-core/   エンジン: mmap / 疎行インデックス / エンコーディング / 検索（純ライブラリ、テスト17件）
  ayame-cli/    bin "ayame": CLI サブコマンド ＋ ローカル Web ビューア（axum、Web 資産を埋め込み）
docs/           設計・ロードマップ
```

## ライセンス

MIT
