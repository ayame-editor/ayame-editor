<!-- i18n: language-switcher -->
[English](../CLI_REFERENCE.md) | [日本語](CLI_REFERENCE.md)

# CLI リファレンス

*English: [../CLI_REFERENCE.md](../CLI_REFERENCE.md)*

`ayame` はネイティブエディタの起動、ローカル Web エディタの起動、巨大テキスト
ファイルの端末処理を行えます。各コマンドはストリーミングまたは bounded memory
で動くようにしてあり、通常のエディタで開けないサイズのファイルも扱えます。

<div class="doc-jump-grid">
  <a class="doc-jump" href="#commands">調べる・読み出す</a>
  <a class="doc-jump" href="#transform-options">検索・変換する</a>
  <a class="doc-jump" href="#sort-and-group-options">ソート・集計する</a>
  <a class="doc-jump" href="#serve-options">Web エディタを開く</a>
  <a class="doc-jump" href="#update-and-remove">更新・削除する</a>
  <a class="doc-jump" href="#examples">実行例を見る</a>
</div>

## 使い方

```sh
ayame <COMMAND> [OPTIONS]
```

引数なしの場合、GUI build ではネイティブウィンドウを開きます。CLI build では
ヘルプを表示します。

## コマンド { #commands }

| コマンド | 目的 |
| --- | --- |
| `stat <FILE>` | サイズ、行数、文字コード、改行コード、インデックス情報を表示。 |
| `head <FILE> [-n N]` | 先頭 N 行を表示。既定は 10。 |
| `tail <FILE> [-n N]` | 末尾 N 行を表示。既定は 10。 |
| `line <FILE> <N>` | 1-based の 1 行を表示。 |
| `lines <FILE> <START> <COUNT>` | START から COUNT 行を表示。どちらも 1-based。 |
| `search <FILE> <PATTERN>` | リテラル、正規表現、大文字小文字無視、単語単位、件数制限つき検索。 |
| `sort <FILE>` | メモリ制限つき external merge sort。 |
| `replace <FILE> <FIND> <REPL>` | ストリーミング置換を新しいファイルへ書き出し。 |
| `case <FILE> <MODE>` | 大文字小文字変換 (`upper`, `lower`, `camel`, `pascal`, `snake`, `kebab`, `constant`) を新しいファイルへ書き出し。 |
| `grep-lines <FILE> <PATTERN>` | 一致した行だけを新しいファイルへ書き出し。 |
| `split <FILE> --lines N` | N 行ごとのパーツに分割。 |
| `group <FILE> -k COL` | キー列で group-by し、count や数値集計を実行。 |
| `top <FILE> -k COL -n N` | bounded memory でキー上位 N 行を保持。 |
| `distinct <FILE> -k COL` | HyperLogLog で distinct 数を推定。 |
| `gen <FILE> --lines N` | 合成テストデータを生成。 |
| `serve [FILE]` | ローカル Web エディタを起動。 |
| `gui [FILE]` | GUI feature の build でネイティブウィンドウを開く。 |
| `cache [path|info|gc|clear]` | オンディスク index cache を確認 / 掃除。 |
| `update` | GitHub release artifact をダウンロード、検証、インストール。 |
| `remove` | インストール済みの Ayame binary / app bundle を削除。 |
| `version` | バージョンを表示。 |

従来の `diff`、`sortdiff`、`sort-diff` コマンドは v0.7.0 で削除しました。
1 リリースの間は、対応する ayame-diff コマンドを示すエラーを返します。
[比較ワークフローの ayame-diff 移行ガイド](MIGRATING_TO_AYAME_DIFF.md)も参照してください。

## 共通オプション

| オプション | 対象 | メモ |
| --- | --- | --- |
| `--encoding <ENC>` | ファイルを開くコマンド | `utf8`, `utf-16le`, `utf-16be`, `shift_jis`, `euc-jp`, `iso-2022-jp`, `ascii` を指定。 |
| `--stride <N>` | ファイルを開くコマンド | sparse index checkpoint の行間隔。既定は 4096。 |
| `--no-cache` | ファイルを開くコマンド | persistent index cache を読み書きしない。 |
| `--cache-dir <DIR>` | ファイルを開くコマンド | index cache directory を上書き。 |
| `--json` | `stat`, `search`, `split`, `group`, `top`, `distinct`, `cache` | machine-readable output を stdout へ。 |
| `-V`, `--version` | global | バージョンを表示。 |
| `-h`, `--help` | global | ヘルプを表示。 |

## フィールド系オプション

`sort`, `group`, `top`, `distinct` はキー列を指定できます。

| オプション | メモ |
| --- | --- |
| `-k`, `--key <COL[S]>` | 1-based のキー列。`sort` は優先順の複数列（例: `3,1,2`）、その他は1列を指定します。省略時は行全体。 |
| `-t`, `--delim <C>` | 区切り文字。既定は comma。TSV は `\\t` または `tab`。 |
| `--csv` | RFC 4180 CSV parsing。quote 内 delimiter を扱えます。 |
| `--quote <C>` | CSV quote character。既定は `"`. |
| `--numeric` | `sort` / `top` のキーを数値として扱う。 |

## sort / group オプション { #sort-and-group-options }

| オプション | メモ |
| --- | --- |
| `-r`, `--reverse` | 並び順を反転 (`sort`)。 |
| `--budget <SIZE>` | ディスクへ spill する前のメモリ上限 (`sort` / `group`)。既定 256MiB。`512MiB` や `2GiB` の形式。 |
| `--spill-dir <DIR>` | external-merge spill ファイルの出力先 (`sort` / `group`)。 |

## 変換オプション { #transform-options }

| オプション | メモ |
| --- | --- |
| `--out <FILE>` | `sort`, `replace`, `case`, `grep-lines` の出力先。`replace` / `case` / `grep-lines` は必須。 |
| `-i`, `--ignore-case` | 大文字小文字を無視して `replace` / `grep-lines`。 |
| `-e`, `--regex` | `replace` / `grep-lines` の pattern を正規表現として扱う。 |
| `-w`, `--whole-word` | `grep-lines` を単語単位で一致させる。 |
| `--overwrite` | `grep-lines --out` に既存ファイルの上書きを許可する。 |
| `--jobs <N>` | `replace` / `case` / `grep-lines` の並列 worker 数。`0` は Rayon 既定値。 |
| `--chunk-lines <N>` | 並列 chunk の行数。既定は 4000000。 |

出力コマンドは既存ファイルを上書きしません (`grep-lines` は `--overwrite` で
明示的に許可できます)。別の出力先を指定するか、意図して対象ファイルを削除して
から再実行してください。

## split オプション

| オプション | メモ |
| --- | --- |
| `--lines <N>` | 1 パーツあたりの行数。必須、1 以上。 |
| `--out-dir <DIR>` | 出力ディレクトリ。既定は入力ファイルと同じディレクトリ。 |
| `--name <NAME>` | パーツの base file name。既定は入力ファイル名。 |
| `--json` | split 結果 (パーツ一覧と件数) を JSON で出力。 |

既定では `<stem>.partNNNN<.ext>` 形式で出力します。

## search オプション

| オプション | メモ |
| --- | --- |
| `-e`, `--regex` | pattern を正規表現として扱う。 |
| `-i`, `--ignore-case` | 大文字小文字を無視。 |
| `-w`, `--whole-word` | 単語単位で一致。 |
| `--max <N>` | 表示件数を制限。 |
| `--start-byte <N>` | worker/API resume 用の開始 byte offset。 |

## serve オプション { #serve-options }

`ayame serve` は既定で `127.0.0.1:8777` に bind します。

| オプション | メモ |
| --- | --- |
| `--host <ADDR>` | bind address。既定は `127.0.0.1`。 |
| `--port <N>` | port。既定は `8777`。 |
| `--allow-remote` | non-loopback host に必要。認証なしのファイルアクセスをネットワークへ公開します。 |

## group / top / distinct

| コマンド | オプション |
| --- | --- |
| `group` | `--value <COL>` で数値 `sum`, `min`, `max`, `avg`。`--out-groups <FILE>` で TSV 出力。`--json` は run サマリ (`groups`, `runs`, `spill_bytes`) を出力。 |
| `top` | `-n <N>` で件数指定。`--min` で小さい順。`--out-order <FILE>` は row order を little-endian `u64` で保存。`--json` は選択された行を出力。 |
| `distinct` | 選択キー列の approximate distinct count を表示。`--json` は推定値と HyperLogLog 統計を出力。 |

## cache コマンド

| コマンド | メモ |
| --- | --- |
| `cache path` | cache directory を stdout に表示。 |
| `cache info` | cache size と entry summary を表示。 |
| `cache gc` | 古い cache を削除。`--max-size`, `--max-age-days`, `--dry-run` に対応。 |
| `cache clear` | cache entries を削除。 |

`cache path` はパイプ可能な唯一の値なので stdout に出力します。`info` / `gc` /
`clear` の人間向けレポートは stderr に出力し、stdout はパイプ用に空けます。どの
サブコマンドも `--json` を付けると構造化した結果を stdout に出力します。

## update / remove { #update-and-remove }

| コマンド | オプション |
| --- | --- |
| `update` | `--version <VERSION>` で release tag を指定 (`latest` が既定)。`--install-dir <DIR>` は現在の install を置き換えず、その DIR へインストール。`--force` は同じ版または古い版のインストールを許可。`--dry-run` は release 解決だけを行いファイルを変更しません。 |
| `remove` | `--install-dir <DIR>` で現在の install ではなくその install target を削除。`--yes` は確認プロンプトを省略。`--dry-run` は対象だけ表示しファイルを変更しません。 |

`update` は release の `.sha256` を検証してからインストールします。macOS では
`.app` bundle から起動している場合は `Ayame.app` を更新し、それ以外は standalone
binary として置き換えられます。Windows では実行中の exe を直接置き換え / 削除でき
ないため、現在のプロセス終了後に helper が完了させます。`/nix/store` から動いて
いる binary は Nix 管理とみなし変更しません。Nix 側で更新 / 削除するか、
`--install-dir` で standalone release を別の場所へインストールしてください。

## 終了コード

`grep` の慣習に従います:

| コード | 意味 |
| --- | --- |
| `0` | 成功。`search` では 1 件以上一致。 |
| `1` | `search` は正常に完了したが一致なし。 |
| `2` | 使い方の誤り、または実行中の失敗。 |

`search --json` は常に `0` で終了します (一致有無は `hits` 配列で判断)。

## 例 { #examples }

```sh
ayame stat huge.csv
ayame head huge.log -n 20
ayame tail huge.log -n 200
ayame line huge.log 500000
ayame lines huge.log 500000 50
ayame search huge.log 'ERROR' -i --max 50
ayame sort huge.csv -k 1 --csv --out sorted.csv
ayame replace huge.log ERROR WARN --out fixed.log --jobs 0
ayame case huge.csv lower --out lower.csv
ayame grep-lines huge.log 'ERROR' -i --out errors.log
ayame split huge.csv --lines 1000000
ayame group huge.csv -k 3 --value 5
ayame top huge.csv -k 2 -n 100 --numeric
ayame distinct huge.csv -k 4
ayame gen sample.csv --lines 100000
ayame cache info
ayame update --dry-run
ayame serve huge.csv --port 8777
```
