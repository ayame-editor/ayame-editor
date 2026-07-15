<!-- i18n: language-switcher -->
[English](../DEVELOPMENT.md) | [日本語](DEVELOPMENT.md)

# 開発手順

*English: [../DEVELOPMENT.md](../DEVELOPMENT.md)*

Ayame は Rust workspace です。

- `ayame-core`: mmap / 疎インデックス / 検索 / 編集エンジン。
- `ayame-cli`: CLI、ローカル Web エディタ、任意のネイティブウィンドウ。

詳しい module map は [アーキテクチャ](ARCHITECTURE.md) を参照してください。短く言うと、
`ayame-core` が `Document`、search、transforms、`EditSession`、WAL crash
recovery を担当し、`ayame-cli/src/serve` が `ayame serve` と native window の
両方で使う local `/api/*` router を公開し、`crates/ayame-cli/web/src` の
TypeScript UI を Cargo が binary に埋め込みます。

全 OS 共通の基本ループ:

```sh
cargo fmt --all --check
cargo test --locked
cargo run -p ayame-cli -- --help
```

通常の CLI / Web エディタ開発は Rust だけで足ります。ネイティブウィンドウ
（`ayame gui`）を動かす時だけ `--features gui` と OS 別の WebView 依存が必要です。

## Nix development shell

リポジトリには再現性のある local shell 用の Nix flake があります。

```sh
nix develop
ruby -v
cargo test --locked
python -m mkdocs build --strict
```

shell には Rust tooling、Node.js、pnpm、Python/MkDocs、Homebrew manifest 確認用の
Ruby、`jq`、Linux GUI build dependencies が入っています。direnv を使う場合は
`.envrc` の `use flake` で同じ shell を読み込みます。

## 開発体験 (ツーリング)

主なコードは `crates/ayame-core` と `crates/ayame-cli` にあります。

リポジトリ直下の設定で、誰の環境でも同じ結果になるようにしてある:

- **rust-toolchain.toml** — stable + rustfmt + clippy を自動で揃える(rustup が読む)。
- **rustfmt.toml / .editorconfig** — 改行は LF、インデントは Rust 4 / それ以外 2。
- **Cargo.toml `[workspace.lints]`** — `dbg!` / `todo!` / `unimplemented!` はビルドエラー。
  CI はさらに `cargo clippy -D warnings`(gui feature 込み)を強制。
- **フロントエンド** — `crates/ayame-cli/web/src` の TypeScript ES modules。
  Cargo build 時に `build.rs` が oxc で型を落とした JS を埋め込む。CI は `tsc`、
  `oxfmt`、`oxlint` でソースを確認する。

日常のゲートはこれだけ:

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked --features ayame-cli/gui -- -D warnings
cargo test --locked
npx -y -p typescript@5 tsc --noEmit -p crates/ayame-cli/web/tsconfig.json
cargo run --locked -p ayame-cli --features typegen -- typegen --check
find crates/ayame-cli/web/src -name '*.ts' ! -name '*.d.ts' -print0 | xargs -0 oxfmt --check
oxlint --max-warnings 0 crates/ayame-cli/web/src
```

`cargo xtask typegen --check` は type binding check を包んだものです。
`cargo xtask release` は release preflight、任意の version bump、local artifact
smoke test、tag 作成、GitHub Actions release への handoff を行います。

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
