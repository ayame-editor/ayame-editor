# Ayame（菖蒲）

100億行クラスのテキストでも、メモリを溢れさせず・落ちずに、開く／検索する／並べ替える／集計するローカルツール。
Shift_JIS / EUC-JP / UTF-8 を自動判定。CLI とローカル Web ビューアで使えます。

## インストール

```sh
# GitHub Releases の配布版は単体実行ファイルです。
# Windows: ayame.exe / Linux: ayame / macOS: ayame

cargo build --release          # Rust 安定版
./target/release/ayame --help
```

以降の例では `ayame` = `./target/release/ayame`。

## クイックスタート

```sh
# 試しに大きな合成データを作る（約9.5 MiB / 20万行・CSV）
ayame gen sample.csv --lines 200000

ayame stat sample.csv                       # サイズ・行数・エンコーディング・改行・索引
ayame search sample.csv 'error' -i          # 大小無視で検索
ayame diff old.txt new.txt                  # 差分
ayame sort sample.csv -k 5 -n -r | head     # 5列目を数値・降順で並べ替え
ayame group sample.csv -k 4                 # 4列目（status）ごとに件数
ayame serve sample.csv --port 8777          # 表示された URL をブラウザで開く
```

## コマンド一覧

| コマンド | 説明 |
|---|---|
| `stat   <FILE>` | サイズ・行数・エンコーディング・改行・索引情報（`--json`） |
| `head   <FILE> [-n N]` | 先頭 N 行（既定 10） |
| `tail   <FILE> [-n N]` | 末尾 N 行（既定 10） |
| `line   <FILE> <N>` | N 行目（1始まり） |
| `lines  <FILE> <START> <COUNT>` | START から COUNT 行（`行番号<TAB>本文`） |
| `search <FILE> <PATTERN>` | 検索（`-i` 大小無視, `-e` 正規表現, `--max N`, `--json`） |
| `diff   <OLD> <NEW>` | 行単位 diff（`--summary`, `--json`, `--max-hunks N`） |
| `sort   <FILE>` | 外部マージソート（メモリ予算＋ディスク spill） |
| `group  <FILE> -k COL` | グループ集計（件数、`--value` で sum/min/max/avg） |
| `top    <FILE> -k COL -n N` | 上位 N 行（`--min` で下位） |
| `distinct <FILE> -k COL` | 近似ユニーク数（HyperLogLog） |
| `gen    <FILE> --lines N` | 合成テストデータ生成（`--cols`, `--encoding`） |
| `serve  <FILE>` | ローカル Web ビューアを起動（`--host`, `--port`） |
| `cache  [path\|info\|clear]` | 索引キャッシュの確認・消去 |

## 使い方

### 検索

```sh
ayame search huge.log 'ERROR' -i --max 50     # 大小無視・最大50件
ayame search huge.log 'error' -w              # 単語単位
ayame search huge.log '\bWARN\b' -e            # 正規表現
ayame search huge.log 'failed' --json          # 機械可読出力
```

出力は `行:桁: 本文`。`--max`（既定 1000）に達すると打ち切り表示します。

### 差分（diff）

```sh
ayame diff old.txt new.txt
ayame diff old.txt new.txt --summary
ayame diff old.txt new.txt --json
```

行単位で比較し、近い範囲の挿入/削除は `--window`（既定 128 行）内で再同期します。

### 並べ替え（sort）

RAM を超える行数も、メモリ予算を超えた分だけディスクへ退避して安定ソートします。

```sh
ayame sort sample.csv -k 5 -n -r | head                 # 5列目を数値・降順
ayame sort huge.csv  -k 1 --budget 64MiB --out-order order.bin
```

| オプション | 既定 | 説明 |
|---|---|---|
| `-n, --numeric` | off | キーを数値として比較（既定はコードポイント順） |
| `-r, --reverse` | off | 降順 |
| `--budget <SIZE>` | `256MiB` | メモリ上限（`K`/`M`/`G` 可）。超過分は spill |
| `--out-order <FILE>` | — | 行番号の順列を書き出し（巨大結果向け） |
| `--spill-dir <DIR>` | 一時 | spill 先ディレクトリ |

### 集計（group / top / distinct）

```sh
ayame group sample.csv -k 4                 # status別の件数
ayame group sample.csv -k 3 --value 5       # word別の count/sum/min/max/avg
ayame group huge.csv -k 1 --out-groups groups.tsv
ayame top   sample.csv -k 5 -n 10 --numeric # 値の大きい上位10
ayame top   sample.csv -k 5 -n 10 --numeric --min   # 下位10
ayame distinct sample.csv -k 1              # 近似ユニーク数（既定 16KB・誤差 ~0.8%）
ayame distinct sample.csv -k 1 -p 16        # 精度を上げる（メモリ増）
```

`group` の出力はタブ区切り（`キー<TAB>件数` / `--value` 時は `キー<TAB>件数<TAB>sum<TAB>min<TAB>max<TAB>avg`）。
少数グループはメモリ内で処理し、高カーディナリティのみディスクへ spill します。

### CSV（クォート対応）

`--csv` で RFC-4180 解釈になり、区切りを含むフィールド `"a,b"` や `""` エスケープを正しく扱います。

```sh
ayame group data.csv -k 1 --csv             # "Tokyo, JP" を1キーとして集計
```

### 日本語（Shift_JIS）

```sh
ayame gen sjis.csv --lines 100000 --encoding shift_jis
ayame stat sjis.csv                         # encoding: Shift_JIS
ayame serve sjis.csv                         # ビューアは UTF-8 にデコードして表示
```

### Web ビューア（serve）

```sh
ayame serve huge.csv --port 8777            # http://127.0.0.1:8777 を開く
```

数十億行をスクロールできる VSCode 風の仮想化ビューア。実行に Node も webview も不要。

| 操作 | キー |
|---|---|
| 行へ移動 | `Ctrl+G` |
| 検索 | `Ctrl+F` |
| 次 / 前の一致 | `F3` / `Shift+F3` |
| 大小無視 / 正規表現の切替 | `Alt+C` / `Alt+R` |
| 先頭 / 末尾へ | `Ctrl+Home` / `Ctrl+End` |
| ページ送り | `PageUp` / `PageDown` / `Space` |

### 索引キャッシュ

一度開いたファイルの索引はディスクに保存され、次回以降はほぼ瞬時に開きます。

```sh
ayame cache info                 # 保存先・件数・サイズ
ayame cache gc --max-size 5GiB   # 古い/上限超過の索引だけ削除
ayame cache gc --dry-run         # 削除せず確認
ayame cache clear                # 消去
ayame stat huge.csv --no-cache   # このコマンドだけキャッシュを使わない
```

保存先は `AYAME_CACHE_DIR` → `$XDG_CACHE_HOME/ayame` → `$HOME/.cache/ayame` の順。

## オプション早見表

**フィールド系**（`sort` / `group` / `top` / `distinct`）

| オプション | 既定 | 説明 |
|---|---|---|
| `-k, --key <COL>` | 行全体 | キー列（1始まり） |
| `-t, --delim <C>` | `,` | 区切り文字 |
| `--csv` | off | RFC-4180 解釈（クォート対応） |
| `--quote <C>` | `"` | `--csv` の引用符 |
| `--numeric` | off | キーを数値扱い（`sort`/`top`。`sort` は `-n` も可） |

**共通**

| オプション | 説明 |
|---|---|
| `--encoding <ENC>` | `utf8` / `shift_jis` / `euc-jp` / `ascii` を強制 |
| `--stride <N>` | 索引チェックポイント間隔（既定 4096） |
| `--no-cache` | 索引キャッシュを使わない |
| `--cache-dir <DIR>` | キャッシュ保存先を指定 |
| `--json` | 機械可読出力（`stat` / `search`） |
| `-h, --help` | ヘルプ |

## ライセンス

MIT
