# User Guide

*日本語版: [ja/USER_GUIDE.md](ja/USER_GUIDE.md)*

Ayame Editor is a desktop text editor for huge files. Use the native app for normal editing, or the local web editor when you want to keep it in a browser.

## Install

Download the build for your OS from the [latest release](https://github.com/hjosugi/ayame-editor/releases/latest).

- macOS: `Ayame.app`
- Windows: `ayame-*.exe`
- Linux: single executable

Terminal install:

```sh
curl -fsSL https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.sh | sh
```

Windows PowerShell:

```powershell
pwsh -NoProfile -Command "irm https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.ps1 | iex"
```

## Open Files

```sh
ayame path/to/file.log
```

Open without a file:

```sh
ayame
```

Run the browser-based editor:

```sh
ayame serve path/to/file.log --port 8777
```

Then open `http://127.0.0.1:8777/`.

## Useful CLI Commands

```sh
ayame stat huge.csv
ayame search huge.log 'ERROR' -i --max 50
ayame sort huge.csv --out sorted.csv
ayame replace huge.log ERROR WARN --out fixed.log
ayame split huge.csv --lines 1000000
```

Use `ayame --help` for the full command list.
