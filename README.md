# Ayame Editor

*日本語版: [README.ja.md](README.ja.md)*

A fast desktop text editor for huge files.

Runs on macOS, Windows, and Linux.

## Features

- View, search, and edit huge files without loading the whole file into memory.
- Supports UTF-8, Shift_JIS, EUC-JP, and ASCII.
- Run search, replace, sort, two-file diff, folder grep, and file splitting from the GUI.
- Use CLI commands such as `stat`, `search`, `sort`, `replace`, and `split`.
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

Useful CLI commands, default shortcuts, install notes, build steps, and Linux runtime packages are in the
[docs site](https://hjosugi.github.io/ayame-editor/).

## License

0BSD. You can use, copy, modify, and distribute this project for almost any purpose.


MIT
