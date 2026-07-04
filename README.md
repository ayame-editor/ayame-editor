# Ayame

*日本語版: [README.ja.md](README.ja.md)*

A desktop text editor that opens huge files instantly — no waiting, no crashing.

Drop in a log file of hundreds of megabytes or a CSV of tens of gigabytes and start scrolling the moment it opens. Character encodings (UTF-8 / Shift_JIS / EUC-JP) are detected automatically. Runs on macOS, Windows, and Linux.

## Install

Download the build for your OS from the [latest release](https://github.com/hjosugi/ayame-editor/releases/latest) and double-click to launch.

- **macOS** — `Ayame.app` (first launch only: right-click → "Open")
- **Windows** — `ayame-*.exe`
- **Linux** — a single executable (`WebKitGTK` required, e.g. `libwebkit2gtk-4.1-0` on Ubuntu)

If you prefer the terminal, this works too:

```sh
curl -fsSL https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.sh | sh
```

Once installed, running it with no arguments opens an empty new document; pass a file to open it in a native window:

```sh
ayame
ayame ./huge.log
```

## What it can do

- **Seriously fast** — even at the 10-billion-line scale, only the parts you look at are read, so files open instantly and scroll smoothly. It does not crash.
- **Tabs** — open multiple files and switch between them. `Ctrl+N` for a new tab, `Ctrl+W` to close one. Closing the last tab asks for confirmation, which can be turned off in settings.
- **Explorer** — toggle with the leftmost toolbar button (`Ctrl+B`). Browse files starting from a folder.
- **Search** — `Ctrl+F` floats a search bar at the top right of the editor. Case sensitivity, whole-word, and regular expressions are supported. `F3` / `Shift+F3` jump to the next / previous match.
- **Sort** — "Sort" under Tools ▾. Sorts by whole line or by a key column you choose, ascending or descending, and overwrites the current file in place (unsaved edits are included in the sort).
- **Two-file diff** — "Two-file diff" under Tools ▾ compares the current file with another file side by side. Changed lines can also be inspected word by word.
- **Bulk transform** — run replace / case conversion from Tools ▾ and save the result to a separate file. The work is parallelized over line-boundary chunks, so memory stays bounded even on huge files.
- **File split** — "Split file" under Tools ▾. Specify a line count — say, one million lines per part — and write the pieces out as multiple files (the original file is not modified).
- **Edit like Notepad** — type at the position you click, press `Enter` for a newline, and multi-line paste just works.
- **Selections** — select multiple lines by dragging or `Shift`+click. `Alt`+drag for rectangular selection. Double-click selects a word, triple-click a line. Copy / cut / delete / replace the selection.
- **Context menu** — cut, copy, paste, and select all, plus "Save selection to file" (server-side export with no line-count cap, unlike copy), and quick access to sort / replace / diff / split from the same menu.
- **Multiple cursors** — `Ctrl`+click adds a cursor, `Ctrl+Alt+↑/↓` adds one above/below. Type, delete, and paste at all cursors at once, and a single `Ctrl+Z` undoes it all together. `Esc` collapses back to one cursor.
- **Themes** — the bright, quiet **Iris Light** (default), plus Iris Mist / Dawn, Sumi Light, the monochrome Mono Paper, and dark / black themes. Backgrounds can be watercolor or solid, and themes can be exported as JSON — or write your own (⚙ Settings).

- **Quick memo** — set a folder as the "memo save location" in ⚙ Settings and new documents are saved straight there with `Ctrl+S` (no dialog). The filename comes from the "memo name" template (default `memo-{yyyy}{mm}{dd}.txt`); if the name already exists, `-2`, `-3`, … are appended automatically. Available variables: `{yyyy}` `{yy}` `{mm}` `{dd}` `{HH}` `{MM}` `{ss}` `{date}` `{time}` `{datetime}`. If no save location is set, the usual save dialog opens as before, pre-filled with the expanded template name and the folder you last saved to.

The default display name of new tabs can be changed with `AYAME_UNTITLED_NAME`. `{date}` / `{time}` / `{datetime}` / `{pid}` are available.

```sh
AYAME_UNTITLED_NAME='memo-{date}-{time}.txt' ayame
```

### Keyboard

| Action | Keys |
|---|---|
| Open file | `Ctrl+O` |
| New tab / close tab | `Ctrl+N` / `Ctrl+W` |
| New window | `Ctrl+Shift+N` |
| Explorer | `Ctrl+B` |
| Search / next・previous match | `Ctrl+F` / `F3`・`Shift+F3` |
| Go to line | `Ctrl+G` |
| Copy / cut / select all | `Ctrl+C` / `Ctrl+X` / `Ctrl+A` |
| Add cursor (above / below) | `Ctrl+Alt+↑` / `Ctrl+Alt+↓` (or `Ctrl`+click) |
| Undo / redo | `Ctrl+Z` / `Ctrl+Y` |
| Save / save as | `Ctrl+S` / `Ctrl+Shift+S` |

## Building from source

```sh
cargo build --release --features gui
./target/release/ayame
```

Per-OS development guides for Windows / macOS / Linux are in [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

## License

MIT
