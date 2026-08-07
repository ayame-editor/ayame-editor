# ユーザー向け

Ayame Editor は巨大ファイル向けのデスクトップ・テキストエディタです。通常の編集はネイティブアプリを使い、ブラウザで開きたい場合はローカル Web エディタを使います。

## インストール

[最新リリース](https://github.com/hjosugi/ayame-editor/releases/latest) から、お使いの OS 向けのビルドをダウンロードしてください。

- macOS: `Ayame.app`
- Windows: `ayame-*.exe`
- Linux: 単体実行ファイル

ターミナルからインストールする場合:

```sh
curl -fsSL https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.sh | sh
```

Windows PowerShell:

```powershell
pwsh -NoProfile -Command "irm https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.ps1 | iex"
```

standalone install の更新:

```sh
ayame update
```

削除:

```sh
ayame remove --yes
```

binary が `/nix/store` から動いている場合、Ayame は Nix 管理とみなします。更新や
削除は Nix 側で行うか、`--install-dir` を指定して store 外へ standalone release
をインストールしてください。

ネイティブアプリは、ウィンドウ表示後に standalone release の更新を確認し、更新が
ある場合はインストール前に確認します。不要な場合は `編集` -> `設定` を開き、
`起動時に更新を確認` をオフにしてください。Nix、Homebrew、Scoop など package
manager 管理の install は自己変更せず、package manager 側で更新します。

## ファイルを開く

```sh
ayame path/to/file.log
```

ファイルなしで開く:

```sh
ayame
```

ブラウザ版を起動:

```sh
ayame serve path/to/file.log --port 8777
```

その後 `http://127.0.0.1:8777/` を開きます。

## CLI コマンド

```sh
ayame stat huge.csv
ayame head huge.log -n 20
ayame tail huge.log -n 200
ayame line huge.log 500000
ayame lines huge.log 500000 50
ayame search huge.log 'ERROR' -i --max 50
ayame sort huge.csv --out sorted.csv
ayame replace huge.log ERROR WARN --out fixed.log
ayame case huge.csv lower --out lower.csv
ayame grep-lines huge.log 'ERROR' -i --out errors.log
ayame split huge.csv --lines 1000000
ayame group huge.csv -k 3 --value 5
ayame top huge.csv -k 2 -n 100 --numeric
ayame distinct huge.csv -k 4
ayame gen sample.csv --lines 100000
ayame cache info
ayame serve huge.csv --port 8777
```

この例は現在の `ayame --help` と一致しています。`sort --out <FILE>` は
並べ替え結果をファイルへ書き出します。`--out` を省略した `sort` は標準出力へ
書きます。`replace` と `case` は `--out <FILE>` が必須です。`split` は既定で
入力ファイルと同じディレクトリに `<stem>.partNNNN<.ext>` 形式の分割ファイルを
作ります。既存ファイルは上書きしないため、出力先がある場合は別名を指定してください。

全コマンドとオプションは [CLI リファレンス](CLI_REFERENCE.md) または
`ayame --help` を参照してください。ファイル比較は姉妹プロジェクト ayame-diff が
提供します。[ayame-diff](https://github.com/hjosugi/ayame-diff)を参照してください。

## 主な機能

- 巨大ファイルを全体読み込みせず、必要な範囲だけ表示します。
- UTF-8、UTF-16LE/BE（BOM あり / なし）、Shift_JIS、EUC-JP、
  ISO-2022-JP、ASCII の読み込みに対応します。文字化けした場合は文字コードを
  指定して開き直せます。
- LF、CRLF、旧 Mac の CR-only 改行を検出します。CR-only の UTF-16 ファイルには
  対応していません。
- 検索、正規表現検索、単語単位検索、大文字小文字を無視した検索に対応します。
- 編集、元に戻す / やり直し、矩形選択、マルチカーソル、選択範囲の保存ができます。
- ソート、置換、フォルダ内検索、grep して保存 (一致行だけを別ファイルへ書き出し)、分割、ケース変換を GUI から実行できます。
- タブ、最近使ったファイル、tail -f 風の末尾追従を使えます。デスクトップ版ではタブを別の Ayame ウィンドウへドラッグしたり、外へドラッグして新しいウィンドウにできます — 未保存の編集もタブと一緒に移動します。
- テーマ、フォント、折り返し、空白表示、全角空白の下線表示、キー設定を変更できます。
- クラッシュ復旧用のログにより、未保存の編集を復元できます。

## 既定ショートカット

`Ctrl` は macOS では `Cmd` として入力できます。キー設定は `編集` -> `設定` ->
`キー設定` から変更できます。`ヘルプ` -> `キーボードショートカット` から直接
開くこともできます。

### ショートカットとキー設定できる操作

| 操作 | 既定ショートカット |
| --- | --- |
| 新規ファイル | `Ctrl+N` |
| 新規ウィンドウ | `Ctrl+Shift+N` |
| 開く | `Ctrl+O` |
| 保存 | `Ctrl+S` |
| 名前を付けて保存 | `Ctrl+Shift+S` |
| タブを閉じる | `Ctrl+W` |
| 閉じたタブを再度開く | `Ctrl+Shift+T` |
| 右側 / すべて / 保存済みのタブを閉じる | 未設定 |
| 次のタブ / 前のタブ | `Ctrl+PageDown`, `Ctrl+PageUp` |
| コマンドパレット | `Ctrl+Shift+P` |
| 検索 | `Ctrl+F` |
| 置換 | `Ctrl+H` |
| 次の一致 / 前の一致 | `F3`, `Shift+F3` |
| 行へ移動 | `Ctrl+G` |
| 元に戻す / やり直し | `Ctrl+Z`, `Ctrl+Y` または `Ctrl+Shift+Z` |
| すべて選択 | `Ctrl+A` |
| 次の一致を選択 | `Ctrl+D` |
| カーソルを上下に追加 | `Ctrl+Alt+↑`, `Ctrl+Alt+↓` |
| 行を複製 | `Ctrl+Shift+D` |
| 行を上下に移動 | `Alt+↑`, `Alt+↓` |
| 行を削除 | `Ctrl+Shift+K` |
| コピー / 切り取り / 貼り付け | `Ctrl+C`, `Ctrl+X`, `Ctrl+V` |
| 検索オプション: 大文字小文字 / 単語 / 正規表現 | `Alt+C`, `Alt+W`, `Alt+R` |
| 文字を大きく / 小さく / 既定に戻す | `Ctrl++`, `Ctrl+-`, `Ctrl+0` |
| 一時ファイルへソートして新しいタブで開く | 未設定 |
| 現在のファイルを分割 | 未設定 |
| フォルダ内検索 | 未設定 |
| grep して保存 (一致行の書き出し) | 未設定 |
| 選択範囲を upper/lower/camel/Pascal/snake/kebab/constant case へ変換 | 未設定 |
| 設定 | 未設定 |
| キー設定 | 未設定 |
| 検索バーやダイアログを閉じる | `Esc` |

未設定の操作は `編集` -> `設定` -> `キー設定` に表示されます。よく使う場合は
任意のショートカットを割り当ててください。

この表の操作はすべて再割り当てできます。文字サイズと貼り付けも含みます。貼り付けは
既定の `Ctrl+V` のままならシステムのクリップボード動作を使い、別のキーに割り当てた
場合はクリップボードを直接読みます (ブラウザによっては許可を求められます)。

`+` や `-` をどの物理キーで入力するかは配列によって異なるため、文字サイズの
ショートカットは配列が要求する `Shift` の有無どちらでも一致します。

### メニュー / ステータス操作

次の操作は現在の build では既定キーがありません。メニュー、ステータスバー、
または記載があるものはコマンドパレット (`Ctrl+Shift+P`) から開きます。

| 操作 | 開き方 |
| --- | --- |
| 末尾に追従 (`tail -f`) | `表示` -> `末尾に追従`、ステータスバーの tail ボタン、またはコマンドパレット |
| 空白・改行を表示 | `表示` -> `空白・改行を表示` またはコマンドパレット |
| 全角空白を下線で表示 | `表示` -> `全角空白を下線で表示` またはコマンドパレット |
| 折り返し | `表示` -> `折り返し` またはコマンドパレット |
| 文字コード / 改行コードを変換して保存 | `ファイル` -> `文字コード / 改行コード...`、またはステータスバーの文字コード / 改行コード表示 |
| 別の文字コードで開き直す | `文字コード / 改行コード...` を開き、文字コードを選んで `開き直す` |
| 選択箇所をファイルに保存 | 選択範囲のコンテキストメニュー |
| 切り取り / コピー / 貼り付け / すべて選択 | `編集` メニュー |
| 他 / 右側 / 保存済み / すべてのタブを閉じる | タブの右クリックメニュー |
| 閉じたタブを再度開く | タブの右クリックメニュー または `Ctrl+Shift+T` |
| 設定 | `編集` -> `設定` |
| キー設定 | `編集` -> `設定` -> `キー設定`、または `ヘルプ` -> `キーボードショートカット` |
