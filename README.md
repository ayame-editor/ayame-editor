# Ayame Editor

*日本語版: [README.ja.md](README.ja.md)*

A fast desktop text editor for huge files.

Runs on macOS, Windows, and Linux.

![Ayame Editor main window](docs/assets/screenshot-main.png)

## Features

- View, search, and edit huge files without loading the whole file into memory.
- Supports UTF-8, Shift_JIS, EUC-JP, and ASCII.
- Run search, replace, sort, two-file diff, folder grep, and file splitting from the GUI.
- Use CLI commands such as `stat`, `head`, `tail`, `line`, `lines`, `search`, `diff`, `sort`, `sortdiff`, `replace`, `case`, `split`, `group`, `top`, `distinct`, `gen`, `serve`, and `cache`.
- Includes tabs, an explorer, rectangular selection, multi-cursor editing, and tail-follow mode.
- Customizable themes, fonts, wrapping, whitespace display, and key bindings.

## Install

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

Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.sh | sh
```

## More

The docs site includes the user guide, full CLI reference, architecture notes,
default shortcuts, install notes, build steps, and Linux runtime packages:
[docs site](https://hjosugi.github.io/ayame-editor/).

For files where a single wrong byte is unacceptable, see the
[data integrity guarantees](docs/DATA_INTEGRITY.md) — the correctness promises
(byte-exact save, crash recovery, encoding round-trips) and the tests that
verify each one.

## License

0BSD. You can use, copy, modify, and distribute this project for almost any purpose.
