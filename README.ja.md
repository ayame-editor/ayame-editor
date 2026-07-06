# Ayame Editor

*English: [README.md](README.md)*

巨大ファイルをすばやく開けるデスクトップ・テキストエディタです。

macOS、Windows、Linux で動作します。

![Ayame Editor main window](docs/assets/screenshot-main.png)

## 主な機能

- 巨大ファイルを全体読み込みせずに表示・検索・編集できます。
- UTF-8、Shift_JIS、EUC-JP、ASCII に対応します。
- GUI では検索、置換、ソート、2 ファイル差分、フォルダ内検索、ファイル分割を実行できます。
- CLI では `stat`、`head`、`tail`、`line`、`lines`、`search`、`diff`、`sort`、`sortdiff`、`replace`、`case`、`split`、`group`、`top`、`distinct`、`gen`、`serve`、`cache` などを使えます。
- タブ、エクスプローラー、矩形選択、マルチカーソル、tail -f 風の末尾追従を備えています。
- テーマ、フォント、折り返し、空白表示、キー設定を変更できます。

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

ユーザー向けガイド、全 CLI リファレンス、アーキテクチャ、既定ショートカット、
インストール、ビルド手順、Linux の実行時パッケージは
[ドキュメントサイト](https://hjosugi.github.io/ayame-editor/ja/) にまとめています。

1 バイトの誤りも許されない用途向けに、[データ完全性の保証](docs/ja/DATA_INTEGRITY.md)
（バイト正確な保存・クラッシュ復元・エンコーディング往復などの正確性の約束と、
それを検証するテスト）をまとめています。

## ライセンス

0BSD。ほぼすべての目的で、このプロジェクトを使用、コピー、変更、配布できます。
