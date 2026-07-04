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

全コマンドは `ayame --help` で確認できます。
