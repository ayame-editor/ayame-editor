# Development Guide

Ayame is a Rust workspace.

- `ayame-core`: mmap / sparse index / search / editing engine.
- `ayame-cli`: CLI, local web editor, optional native window.

For the full module map, see [Architecture](ARCHITECTURE.md). The short version:
`ayame-core` owns `Document`, search, transforms, `EditSession`, and WAL crash
recovery; `ayame-cli/src/serve` exposes the local `/api/*` router used by both
`ayame serve` and the native window; `crates/ayame-cli/web/src` contains the
TypeScript UI that Cargo embeds into the binary.

The basic loop, common to all OSes:

```sh
cargo fmt --all --check
cargo test --locked
cargo run -p ayame-cli -- --help
```

For ordinary CLI / web editor development, Rust alone is enough. Only when running
the native window (`ayame gui`) do you need `--features gui` and the per-OS WebView
dependencies.

## Nix development shell

The repository includes a Nix flake for a reproducible local shell:

```sh
nix develop
ruby -v
cargo test --locked
python -m mkdocs build --strict
```

The shell includes Rust tooling, Node.js, pnpm, Python/MkDocs, Ruby for Homebrew
manifest checks, `jq`, and Linux GUI build dependencies. If you use direnv,
`.envrc` loads the same shell with `use flake`.

## Developer experience (tooling)

The main code lives under `crates/ayame-core` and `crates/ayame-cli`.

Configuration at the repository root makes results identical on anyone's machine:

- **rust-toolchain.toml** — automatically pins stable + rustfmt + clippy (read by rustup).
- **rustfmt.toml / .editorconfig** — LF line endings; indentation is 4 for Rust, 2 for everything else.
- **Cargo.toml `[workspace.lints]`** — `dbg!` / `todo!` / `unimplemented!` are build errors.
  CI additionally enforces `cargo clippy -D warnings` (including the gui feature).
- **Frontend** — TypeScript ES modules in `crates/ayame-cli/web/src`.
  Cargo embeds type-stripped JS with oxc in `build.rs`; CI checks the sources with `tsc`,
  `oxfmt`, and `oxlint`.

The daily gate is just this:

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked --features ayame-cli/gui -- -D warnings
cargo test --locked
npx -y -p typescript@5 tsc --noEmit -p crates/ayame-cli/web/tsconfig.json
cargo run --locked -p ayame-cli --features typegen -- typegen --check
find crates/ayame-cli/web/src -name '*.ts' ! -name '*.d.ts' -print0 | xargs -0 oxfmt --check
oxlint --max-warnings 0 crates/ayame-cli/web/src
```

`cargo xtask typegen --check` wraps the type binding check. `cargo xtask release`
runs the release preflight, optional version bump, local artifact smoke tests,
tag creation, and GitHub Actions release handoff.

## Windows

Use PowerShell.

### 1. Prerequisites

- Git for Windows
- Visual Studio Build Tools 2022
  - workload: **Desktop development with C++**
- rustup
  - toolchain: `stable-x86_64-pc-windows-msvc`
- Microsoft Edge WebView2 Runtime
  - required for `cargo run ... --features gui -- gui`

### 2. Verify the toolchain

```powershell
rustup default stable-x86_64-pc-windows-msvc
rustc -V
cargo -V
```

### 3. Build and test

```powershell
cargo fmt --all --check
cargo test --locked
cargo build --release --locked
cargo build --release --locked --features gui
```

### 4. Run the CLI / web editor

```powershell
New-Item -ItemType Directory -Force samples
cargo run -p ayame-cli -- gen .\samples\dev.csv --lines 10000
cargo run -p ayame-cli -- stat .\samples\dev.csv
cargo run -p ayame-cli -- serve .\samples\dev.csv --port 8777
```

Open `http://127.0.0.1:8777/` in a browser.

### 5. Run the native window

```powershell
cargo run -p ayame-cli --features gui -- gui .\samples\dev.csv
```

## macOS

Use Terminal.

### 1. Prerequisites

```sh
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Both Apple Silicon and Intel can develop on stable Rust.

### 2. Verify the toolchain

```sh
rustc -V
cargo -V
```

### 3. Build and test

```sh
cargo fmt --all --check
cargo test --locked
cargo build --release --locked
cargo build --release --locked --features gui
```

### 4. Run the CLI / web editor

```sh
mkdir -p samples
cargo run -p ayame-cli -- gen samples/dev.csv --lines 10000
cargo run -p ayame-cli -- stat samples/dev.csv
cargo run -p ayame-cli -- serve samples/dev.csv --port 8777
```

Open `http://127.0.0.1:8777/` in a browser.

### 5. Run the native window

```sh
cargo run -p ayame-cli --features gui -- gui samples/dev.csv
```

## Linux

Use your usual shell.

### 1. Rust

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 2. OS packages

For the CLI alone, Rust and a C toolchain are enough. If you use `--features gui`,
GTK / WebKitGTK are required.

If you run the released Linux binary instead of building from source, install the
WebKitGTK 4.1 runtime package for your distribution:

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

For local development, install the build dependencies instead:

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

### 3. Build and test

```sh
cargo fmt --all --check
cargo test --locked
cargo build --release --locked
cargo build --release --locked --features gui
```

### 4. Run the CLI / web editor

```sh
mkdir -p samples
cargo run -p ayame-cli -- gen samples/dev.csv --lines 10000
cargo run -p ayame-cli -- stat samples/dev.csv
cargo run -p ayame-cli -- serve samples/dev.csv --port 8777
```

Open `http://127.0.0.1:8777/` in a browser.

### 5. Run the native window

```sh
cargo run -p ayame-cli --features gui -- gui samples/dev.csv
```
