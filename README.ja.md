# Ayame Editor

*English: [README.md](README.md)*

巨大ファイルをすばやく開けるデスクトップ・テキストエディタです。

大きなログや CSV を待たずに開き、よく使われる日本語文字コードを自動判別します。
macOS、Windows、Linux で動作します。

## ダウンロード

[最新リリース](https://github.com/hjosugi/ayame-editor/releases/latest) から、お使いの OS 向けのビルドをダウンロードしてください。

- macOS: `Ayame.app`
- Windows: `ayame-*.exe`
- Linux: 単体実行ファイル（`WebKitGTK` が必要）

ターミナルからインストールすることもできます。

```sh
curl -fsSL https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.sh | sh
```

## 主な機能

- 表示に必要な部分だけを読むため、巨大ファイルをすばやく開けます。
- UTF-8、Shift_JIS、EUC-JP を自動判別します。
- タブ、エクスプローラー、検索、編集、範囲選択、マルチカーソル、テーマ。
- ソート、2 ファイル差分、一括変換、ファイル分割。
- クイックメモ保存と、未保存編集のクラッシュ復元。

## ビルド方法

```sh
cargo build --release --features gui
./target/release/ayame
```

OS 別のセットアップは [docs/ja/DEVELOPMENT.md](docs/ja/DEVELOPMENT.md) を参照してください。

## ドキュメント

- [開発](docs/ja/DEVELOPMENT.md)
- [設計](docs/ja/DESIGN.md)
- [ベンチマーク](docs/ja/BENCHMARKS.md)
- [ロードマップ](docs/ja/ROADMAP.md)
- [リリース手順](docs/ja/RELEASE.md)

## 変更履歴

[GitHub Releases](https://github.com/hjosugi/ayame-editor/releases) を参照してください。

## ライセンス

MIT
