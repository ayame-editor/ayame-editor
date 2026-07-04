# Ayame Editor

*日本語版: [README.ja.md](README.ja.md)*

A fast desktop text editor for huge files.

Ayame opens large logs and CSV files quickly, detects common Japanese encodings
automatically, and runs on macOS, Windows, and Linux.

## Download

Download the build for your OS from the
[latest release](https://github.com/hjosugi/ayame-editor/releases/latest).

- macOS: `Ayame.app`
- Windows: `ayame-*.exe`
- Linux: single executable (`WebKitGTK` required)

You can also install from the terminal.

macOS / Windows:

```sh
curl -fsSL https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.sh | sh
```

Linux: install WebKitGTK 4.1 first, then install Ayame.

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

# Then install Ayame
curl -fsSL https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.sh | sh
```

## Features

- Huge files open quickly by reading only the parts you view.
- UTF-8, Shift_JIS, and EUC-JP are detected automatically.
- Tabs, explorer, search, edit, selection, multiple cursors, and themes.
- Sort, two-file diff, bulk transform, and file split tools.
- Crash recovery for unsaved edits.
- A default folder and name template for new files, so a new buffer saves with `Ctrl+S` and no dialog.

## How to build

```sh
cargo build --release --features gui
./target/release/ayame
```

See [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) for platform-specific setup.

## Documentation

- [Development](docs/DEVELOPMENT.md)
- [Design](docs/DESIGN.md)
- [Benchmarks](docs/BENCHMARKS.md)
- [Roadmap](docs/ROADMAP.md)
- [Release process](docs/RELEASE.md)

## Changelog

See [GitHub Releases](https://github.com/hjosugi/ayame-editor/releases).

## License

MIT
