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

macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.sh | sh
```

Windows は Releases から `ayame-*.exe` をダウンロードしてください。

Linux: 先に WebKitGTK 4.1 を入れてから Ayame をインストールします。

```sh
# Debian / Ubuntu / Linux Mint / Pop!_OS
sudo apt update
sudo apt install -y libwebkit2gtk-4.1-0

# Fedora
sudo dnf install -y webkit2gtk4.1

# RHEL / Rocky Linux / AlmaLinux / CentOS Stream
sudo dnf install -y epel-release
sudo dnf install -y webkit2gtk4.1

# Arch Linux / Manjaro / EndeavourOS
sudo pacman -Syu webkit2gtk-4.1

# openSUSE
sudo zypper refresh
sudo zypper install -y libwebkit2gtk-4_1-0

# Alpine Linux
sudo apk add webkit2gtk-4.1

# Gentoo
sudo emerge --ask net-libs/webkit-gtk

# そのあと Ayame をインストール
curl -fsSL https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.sh | sh
```

## 主な機能

- 表示に必要な部分だけを読むため、巨大ファイルをすばやく開けます。
- UTF-8、Shift_JIS、EUC-JP を自動判別します。
- タブ、エクスプローラー、検索、編集、範囲選択、マルチカーソル、テーマ。
- ソート、2 ファイル差分、一括変換、ファイル分割。
- 未保存編集のクラッシュ復元。
- 新規ファイルの既定の保存先と名前を設定でき、新規バッファは `Ctrl+S` でダイアログなしに保存。

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
