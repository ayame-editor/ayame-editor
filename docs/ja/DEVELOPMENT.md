# 開発手順

*English: [../DEVELOPMENT.md](../DEVELOPMENT.md)*

Ayame は Rust workspace です。

- `ayame-core`: mmap / 疎インデックス / 検索 / 編集エンジン。
- `ayame-cli`: CLI、ローカル Web エディタ、任意のネイティブウィンドウ。

全 OS 共通の基本ループ:

```sh
cargo fmt --all --check
cargo test --locked
cargo run -p ayame-cli -- --help
```

通常の CLI / Web エディタ開発は Rust だけで足ります。ネイティブウィンドウ
（`ayame gui`）を動かす時だけ `--features gui` と OS 別の WebView 依存が必要です。


## 開発体験 (ツーリング)

コードの全体像は [IMPLEMENTATION_NOTES.md](IMPLEMENTATION_NOTES.md)(実装の地図と主要リファクタの記録)から。

リポジトリ直下の設定で、誰の環境でも同じ結果になるようにしてある:

- **rust-toolchain.toml** — stable + rustfmt + clippy を自動で揃える(rustup が読む)。
- **rustfmt.toml / .editorconfig** — 改行は LF、インデントは Rust 4 / それ以外 2。
- **Cargo.toml `[workspace.lints]`** — `dbg!` / `todo!` / `unimplemented!` はビルドエラー。
  CI はさらに `cargo clippy -D warnings`(gui feature 込み)を強制。
- **フロントエンド** — `crates/ayame-cli/web/src` の TypeScript ES modules。
  Cargo build 時に `build.rs` が oxc で型を落とした JS を埋め込む。CI は `tsc`、
  `oxfmt`、`oxlint` でソースを確認する。
- **リリース** — `cargo xtask release --bump patch` の1コマンド(docs/RELEASE.md)。

日常のゲートはこれだけ:

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked --features ayame-cli/gui -- -D warnings
cargo test --locked
npx -y -p typescript@5 tsc --noEmit -p crates/ayame-cli/web/tsconfig.json
find crates/ayame-cli/web/src -name '*.ts' ! -name '*.d.ts' -print0 | xargs -0 oxfmt --check
oxlint --max-warnings 0 crates/ayame-cli/web/src
```

## Windows

PowerShell を使います。

### 1. 必要なもの

- Git for Windows
- Visual Studio Build Tools 2022
  - workload: **Desktop development with C++**
- rustup
  - toolchain: `stable-x86_64-pc-windows-msvc`
- Microsoft Edge WebView2 Runtime
  - `cargo run ... --features gui -- gui` に必要

### 2. ツールチェーン確認

```powershell
rustup default stable-x86_64-pc-windows-msvc
rustc -V
cargo -V
```

### 3. ビルドとテスト

```powershell
cargo fmt --all --check
cargo test --locked
cargo build --release --locked
cargo build --release --locked --features gui
```

### 4. CLI / Web エディタ起動

```powershell
New-Item -ItemType Directory -Force samples
cargo run -p ayame-cli -- gen .\samples\dev.csv --lines 10000
cargo run -p ayame-cli -- stat .\samples\dev.csv
cargo run -p ayame-cli -- serve .\samples\dev.csv --port 8777
```

ブラウザで `http://127.0.0.1:8777/` を開きます。

### 5. ネイティブウィンドウ起動

```powershell
cargo run -p ayame-cli --features gui -- gui .\samples\dev.csv
```

### 6. ローカル release artifact

`scripts/release-local.sh` は Bash スクリプトです。Windows では Git Bash から実行します。
ネイティブアプリ版（`--features gui`）を作るため、WebView2 Runtime も入れておきます。

```sh
bash scripts/release-local.sh x86_64-pc-windows-msvc
```

## macOS

Terminal を使います。

### 1. 必要なもの

```sh
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Apple Silicon / Intel どちらも stable Rust で開発できます。

### 2. ツールチェーン確認

```sh
rustc -V
cargo -V
```

### 3. ビルドとテスト

```sh
cargo fmt --all --check
cargo test --locked
cargo build --release --locked
cargo build --release --locked --features gui
```

### 4. CLI / Web エディタ起動

```sh
mkdir -p samples
cargo run -p ayame-cli -- gen samples/dev.csv --lines 10000
cargo run -p ayame-cli -- stat samples/dev.csv
cargo run -p ayame-cli -- serve samples/dev.csv --port 8777
```

ブラウザで `http://127.0.0.1:8777/` を開きます。

### 5. ネイティブウィンドウ起動

```sh
cargo run -p ayame-cli --features gui -- gui samples/dev.csv
```

### 6. ローカル release artifact

```sh
scripts/release-local.sh
```

## Linux

通常の shell を使います。

### 1. Rust

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 2. OS パッケージ

CLI だけなら Rust と C toolchain で十分です。`--features gui` を使う場合は
GTK / WebKitGTK が必要です。

ソースからビルドせず、リリース済み Linux バイナリを動かすだけなら、
お使いのディストリビューションの WebKitGTK 4.1 runtime package を入れてください。

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
```

ローカル開発では、かわりにビルド依存を入れてください。

Debian / Ubuntu:

```sh
sudo apt update
sudo apt install -y build-essential pkg-config libgtk-3-dev libwebkit2gtk-4.1-dev
```

Fedora:

```sh
sudo dnf install -y gcc gcc-c++ make pkg-config gtk3-devel webkit2gtk4.1-devel
```

Arch:

```sh
sudo pacman -S --needed base-devel pkgconf gtk3 webkit2gtk-4.1
```

### 3. ビルドとテスト

```sh
cargo fmt --all --check
cargo test --locked
cargo build --release --locked
cargo build --release --locked --features gui
```

### 4. CLI / Web エディタ起動

```sh
mkdir -p samples
cargo run -p ayame-cli -- gen samples/dev.csv --lines 10000
cargo run -p ayame-cli -- stat samples/dev.csv
cargo run -p ayame-cli -- serve samples/dev.csv --port 8777
```

ブラウザで `http://127.0.0.1:8777/` を開きます。

### 5. ネイティブウィンドウ起動

```sh
cargo run -p ayame-cli --features gui -- gui samples/dev.csv
```

### 6. ローカル release artifact

```sh
scripts/release-local.sh
```

## Release gate

タグを切る前に実行します。

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
cargo build --release --locked --features gui
scripts/crash-isolation-test.sh
```

GitHub Releases の OS 別 artifact は GitHub Actions が作ります。手元で確認する時は
`scripts/release-local.sh` を使います。
