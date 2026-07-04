# Ayame Editor

*English: [README.md](README.md)*

巨大ファイルをすばやく開けるデスクトップ・テキストエディタです。

macOS、Windows、Linux で動作します。

## インストール

[最新リリース](https://github.com/hjosugi/ayame-editor/releases/latest) から、お使いの OS 向けのビルドをダウンロードしてください。

- macOS: `Ayame.app`
- Windows: `ayame-*.exe`
- Linux: 単体実行ファイル

ターミナルからインストールすることもできます。

macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.sh | sh
```

Windows (PowerShell):

```powershell
pwsh -NoProfile -Command "irm https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.ps1 | iex"
```

Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.sh | sh
```

## 詳細

ビルド手順、Linux の実行時パッケージ、設計メモ、リリース情報は
[docs](docs/ja/) にまとめています。

## ライセンス

MIT
