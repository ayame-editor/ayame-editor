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
- Linux: single executable

You can also install from the terminal.

macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.sh | sh
```

Windows (PowerShell):

```powershell
pwsh -NoProfile -Command "irm https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.ps1 | iex"
```

This installs `ayame.exe`, adds it to the user `PATH`, and updates the Desktop /
Start Menu shortcuts. You can also download `ayame-*.exe` from Releases.

Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.sh | sh
```

## Features

- Huge files open quickly by reading only the parts you view.
- UTF-8, Shift_JIS, and EUC-JP are detected automatically.
- Tabs, explorer, search, edit, selection, multiple cursors, and themes.
- Sort, two-file diff, bulk transform, and file split tools.
- Crash recovery for unsaved edits.

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
