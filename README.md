# Ayame（菖蒲）

最低100億行クラスのテキストでも、メモリを溢れさせず・落ちずに、開く／編集する／検索する／並べ替える／集計する**ネイティブのデスクトップエディタ**。
Shift_JIS / EUC-JP / UTF-8 を自動判定。ウィンドウは OS 標準の webview（macOS=WKWebView / Windows=WebView2 / Linux=WebKitGTK）で、Chromium を同梱しないため軽量です。

100億行は「いつかの目標」ではなく最低設計ラインです。既定 stride 4096 の疎行インデックスでは、100億行の索引は約39 MiBです。

> 同じ実行ファイルには CLI エンジン（`search` / `sort` / `group` など）も内蔵していますが、配布は**デスクトップアプリのみ**です（CLI 単体版は配布していません）。

## インストール

[GitHub Releases](https://github.com/hjosugi/ayame-editor/releases/latest) から各 OS 向けのアプリをダウンロードして起動します（**ダブルクリック**で起動）。

- **Windows**: `ayame-*-windows-x86_64.exe`
- **macOS**: `ayame-*-macos-<arch>.zip`（展開すると `Ayame.app`）。初回は右クリック→「開く」で Gatekeeper を許可
- **Linux**: `ayame-*-linux-x86_64`（実行時に **WebKitGTK** が必要。例: Debian/Ubuntu `libwebkit2gtk-4.1-0`）

インストーラ経由でも入れられます（Windows/Linux は実行ファイルを PATH へ、macOS は `Ayame.app` を `~/Applications` へ配置）:

```sh
curl -fsSL https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.sh | sh
```

バージョンや配置先を固定したい場合:

```sh
curl -fsSL https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.sh \
  | AYAME_VERSION=0.1.16 AYAME_INSTALL_DIR="$HOME/.local/bin" sh
```

### ソースからビルドする場合

デスクトップアプリ（webview 同梱ではなく OS の webview を利用）:

```sh
cargo build --release --features gui
./target/release/ayame            # ダブルクリック相当（引数なしでウィンドウ起動）
```

Linux でのビルドには `libwebkit2gtk-4.1-dev` と `libgtk-3-dev` が必要です。feature を付けずにビルドすると、内蔵 CLI エンジンのみ（webview 非依存）で動作します。

## 内蔵 CLI エンジン（上級者向け）

デスクトップアプリと同じ実行ファイルは、`search` / `sort` / `group` などのコマンドも実行できます（アプリ内部の処理にも使われています）。ターミナルから直接使う場合の例です。以降は `ayame` が PATH 上にある前提です（macOS の `.app` 内なら `Ayame.app/Contents/MacOS/ayame`）。

```sh
# 試しに大きな合成データを作る（約9.5 MiB / 20万行・CSV）
ayame gen sample.csv --lines 200000

ayame stat sample.csv                       # サイズ・行数・エンコーディング・改行・索引
ayame search sample.csv 'error' -i          # 大小無視で検索
ayame diff old.txt new.txt                  # 差分
ayame sort sample.csv -k 5 -n -r --out sorted.csv  # 5列目を数値・降順で並べ替え
ayame sortdiff old.csv new.csv -k 1 --summary      # 並び順を無視して比較
ayame replace sample.csv ERROR WARN --out fixed.csv
ayame case sample.csv lower --out lower.csv
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
| `sortdiff <OLD> <NEW>` | 同じ条件で両方をソートしてから比較 |
| `replace <FILE> <FIND> <REPL>` | ストリーミング置換（`--out`, `-i`, `-e`） |
| `case   <FILE> <upper\|lower>` | エンコーディング安全な ASCII ケース変換（`--out`） |
| `group  <FILE> -k COL` | グループ集計（件数、`--value` で sum/min/max/avg） |
| `top    <FILE> -k COL -n N` | 上位 N 行（`--min` で下位） |
| `distinct <FILE> -k COL` | 近似ユニーク数（HyperLogLog） |
| `gen    <FILE> --lines N` | 合成テストデータ生成（`--cols`, `--encoding`） |
| `serve  [<FILE>]` | ローカル Web エディタを起動（`--host`, `--port`）。FILE 省略で空の状態から開始 |
| `gui    [<FILE>]` | ネイティブのデスクトップウィンドウでエディタを開く（GUI ビルドのみ） |
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
ayame sort sample.csv -k 5 -n -r --out sorted.csv
ayame sort huge.csv  -k 1 --budget 64MiB --out-order order.bin
ayame sortdiff old.csv new.csv -k 1 --numeric --summary
```

| オプション | 既定 | 説明 |
|---|---|---|
| `-n, --numeric` | off | キーを数値として比較（既定はコードポイント順） |
| `-r, --reverse` | off | 降順 |
| `--budget <SIZE>` | `256MiB` | メモリ上限（`K`/`M`/`G` 可）。超過分は spill |
| `--out-order <FILE>` | — | 行番号の順列を書き出し（巨大結果向け） |
| `--out <FILE>` | — | ソート済みテキストを書き出し |
| `--spill-dir <DIR>` | 一時 | spill 先ディレクトリ |

### 置換・ケース変換

```sh
ayame replace huge.log ERROR WARN --out fixed.log       # literal は高速 byte path
ayame replace huge.log 'error\\d+' WARN -e --out fixed.log
ayame replace huge.log error WARN -i --out fixed.log
ayame case huge.csv upper --out upper.csv
ayame case sjis.csv lower --out lower.csv
```

`replace` / `case` は元ファイルを変更せず、新しいファイルへストリーミング保存します。
通常の UTF-8/ASCII literal 置換は raw byte fast path、Shift_JIS では文字境界を壊さない安全経路を使います。

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
ayame serve sjis.csv                         # Web エディタは UTF-8 にデコードして表示
```

### Web エディタ（serve）

```sh
ayame serve huge.csv --port 8777            # http://127.0.0.1:8777 を開く
ayame serve --port 8777                     # FILE 省略。ブラウザで開くファイルを選ぶ
```

最低100億行規模をスクロールできる VSCode 風の仮想化エディタ。実行に Node も webview も不要。
編集は元ファイルを丸ごと rope 化せず、mmap base の上に差分レイヤを持ちます。
未編集行は元 bytes のまま保存コピーへストリームし、編集行だけ元 encoding に再エンコードします。
行単位の置換/挿入/削除に加え、差分レイヤだけを対象にした undo/redo を使えます。

**ワークスペース（ファイルを開く）**：FILE を渡さずに起動でき、起動時は空の **untitled** ページが開きます（ダイアログは出ません）。

- ツールバーの **開く**（`Ctrl+O`）でファイルピッカーを表示。サーバ側のディレクトリを辿るか、パスを直接入力して開きます。巨大ファイルはパス指定が最速（mmap のためコピーなし）。
- **ドラッグ＆ドロップ**：ファイルを画面へ落とすと、その中身を一時ファイルへストリーム保存してから開きます（手元の便利ファイル向け。ディスク上の巨大ファイルはパス指定推奨）。
- 開いているファイルはいつでも別ファイルに切り替えられます（未保存の編集があれば確認）。

**タブ（複数ファイル）**：ファイルはタブで開き、複数を同時に扱えます。タブのクリックで切替、`✕`（または中クリック）で閉じる、`＋` で新規。未保存タブには印（`•`）が付きます。ショートカット: 新規 `Ctrl+N` / 閉じる `Ctrl+W`。

**エクスプローラー（ファイルツリー）**：ツールバーの `☰`（`Ctrl+B`）でサイドバーを開閉。フォルダを展開してファイルをクリックするとタブで開きます。ルートは「開く」ダイアログの **フォルダ** ボタンで表示中のフォルダに設定でき、`↑` で親フォルダへ移動できます（開閉状態はブラウザに保存）。

**メモ帳風の編集**：行をクリックするとその位置にカーソルが入り、そのまま入力できます。`Enter` で行を分割、`Backspace`（行頭）/`Delete`（行末）で前後の行と結合、`↑`/`↓`/`←`/`→` で行をまたいでカーソル移動、**複数行の貼り付け**はそのまま複数行に展開されます。行末には `[EOF]` を表示します。（現状、行をまたぐ範囲選択・コピーは未対応です。）

**設定**（ツールバーの ⚙）：テーマ（**ライト**（既定）/ ダーク / ブラック）、フォント、文字サイズ、**列ルーラー**（本文上部の桁目盛り。既定オン）を切り替えられます（ブラウザに保存）。

| 操作 | キー |
|---|---|
| ファイルを開く | `Ctrl+O` |
| 行へ移動 | `Ctrl+G` |
| 検索 | `Ctrl+F` |
| 次 / 前の一致 | `F3` / `Shift+F3` |
| 行編集 | `Enter` / `F2` / ダブルクリック |
| 行挿入 / 行削除 | `Insert` / `Delete` |
| 元に戻す / やり直す | `Ctrl+Z` / `Ctrl+Y` |
| 編集内容を別ファイルへ保存 | `Ctrl+S` |
| ソート / 置換 / 大文字小文字変換を別ファイルへ保存 | ツールバー |
| 大小無視 / 正規表現の切替 | `Alt+C` / `Alt+R` |
| 先頭 / 末尾へ | `Ctrl+Home` / `Ctrl+End` |
| ページ送り | `PageUp` / `PageDown` / `Space` |

### ネイティブアプリ（gui）

ブラウザのタブではなく、**独立したデスクトップウィンドウ**でエディタを開きます。中身は
`serve` と同じローカルサーバ＋Web UI を、OS 標準の webview（macOS=WKWebView /
Windows=WebView2 / Linux=WebKitGTK）で表示します。Chromium を同梱しないため Electron
より軽量です。

```sh
ayame gui                 # 空の状態でウィンドウを開く（ファイルは中で開く）
ayame gui huge.csv        # ファイルを開いた状態でウィンドウを起動
```

配布されるネイティブアプリ（`ayame-gui-*` / macOS は `Ayame.app`）は、**ダブルクリック**
で起動します。サーバはランダムなローカルポートで自動起動し、ウィンドウを閉じると終了します。

- macOS / Windows: OS 内蔵の webview を使うため追加ランタイム不要。
- Linux: 実行時に WebKitGTK が必要です（例: Debian/Ubuntu 系 `libwebkit2gtk-4.1-0`）。
  単体 static な CLI バイナリ（`ayame`）はこの依存を持たず、従来どおり動きます。

> ソースからネイティブアプリをビルドするには feature を有効にします:
> `cargo build --release --features gui`（Linux は `libwebkit2gtk-4.1-dev`, `libgtk-3-dev` が必要）。

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
