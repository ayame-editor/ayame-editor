# ユーザー向け

*English: [../USER_GUIDE.md](../USER_GUIDE.md)*

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

## よく使う CLI

```sh
ayame stat huge.csv
ayame search huge.log 'ERROR' -i --max 50
ayame sort huge.csv --out sorted.csv
ayame replace huge.log ERROR WARN --out fixed.log
ayame split huge.csv --lines 1000000
```

この例は現在の `ayame --help` と一致しています。`sort --out <FILE>` は
並べ替え結果をファイルへ書き出します。`--out` を省略した `sort` は標準出力へ
書きます。`replace` と `case` は `--out <FILE>` が必須です。`split` は既定で
入力ファイルと同じディレクトリに `<stem>.partNNNN<.ext>` 形式の分割ファイルを
作ります。既存ファイルは上書きしないため、出力先がある場合は別名を指定してください。

全コマンドとオプションは `ayame --help` で確認できます。

## 主な機能

- 巨大ファイルを全体読み込みせず、必要な範囲だけ表示します。
- UTF-8、Shift_JIS、EUC-JP、ASCII の読み込みに対応します。文字化けした場合は文字コードを指定して開き直せます。
- 検索、正規表現検索、単語単位検索、大文字小文字を無視した検索に対応します。
- 編集、元に戻す / やり直し、矩形選択、マルチカーソル、選択範囲の保存ができます。
- ソート、置換、2 ファイル差分、フォルダ内検索、分割、ASCII 大文字 / 小文字変換を GUI から実行できます。
- タブ、エクスプローラー、最近使ったファイル、tail -f 風の末尾追従を使えます。
- テーマ、フォント、折り返し、空白表示、全角空白の下線表示、キー設定を変更できます。
- クラッシュ復旧用のログにより、未保存の編集を復元できます。

## 既定ショートカット

`Ctrl` は macOS では `Cmd` として入力できます。キー設定は
`設定` -> `キー設定` から変更できます。

| 操作 | ショートカット |
| --- | --- |
| 新規ファイル | `Ctrl+N` |
| 新規ウィンドウ | `Ctrl+Shift+N` |
| 開く | `Ctrl+O` |
| 保存 | `Ctrl+S` |
| 名前を付けて保存 | `Ctrl+Shift+S` |
| タブを閉じる | `Ctrl+W`, `Alt+W` |
| コマンドパレット | `Ctrl+Shift+P` |
| エクスプローラー表示 | `Ctrl+B` |
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
| コピー / 切り取り | `Ctrl+C`, `Ctrl+X` |
| 検索オプション: 大文字小文字 / 単語 / 正規表現 | `Alt+C`, `Alt+W`, `Alt+R` |
| 検索バーやダイアログを閉じる | `Esc` |
