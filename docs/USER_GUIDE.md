# User Guide

Ayame Editor is a desktop text editor for huge files. Use the native app for normal editing, or the local web editor when you want to keep it in a browser.

## Install

Download the build for your OS from the [latest release](https://github.com/ayame-editor/ayame-editor/releases/latest).

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

Update a standalone install:

```sh
ayame update
```

Remove it:

```sh
ayame remove --yes
```

If the binary is running from `/nix/store`, Ayame treats it as Nix-managed. Use
Nix to update or remove it, or pass `--install-dir` to install a standalone
release outside the store.

The native desktop app checks for newer standalone releases after the window
opens and asks before installing. To disable this, open `Edit` -> `Settings`
and turn off `Check for updates on startup`. Package-manager installs such as
Nix, Homebrew, and Scoop are not self-modified; update them through the package
manager.

An update replaces the binary the running editor was started from, so search,
sort, replace, split, and the other file operations need the new version before
they can run again. The app offers to restart once the install finishes; until
it does — including when `ayame update` is run from another terminal — those
operations report that Ayame must be restarted. The open file and any unsaved
edits are unaffected.

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

## CLI Commands

```sh
ayame stat huge.csv
ayame head huge.log -n 20
ayame tail huge.log -n 200
ayame line huge.log 500000
ayame lines huge.log 500000 50
ayame search huge.log 'ERROR' -i --max 50
ayame sort huge.csv --out sorted.csv
ayame replace huge.log ERROR WARN --out fixed.log
ayame case huge.csv lower --out lower.csv
ayame grep-lines huge.log 'ERROR' -i --out errors.log
ayame split huge.csv --lines 1000000
ayame group huge.csv -k 3 --value 5
ayame top huge.csv -k 2 -n 100 --numeric
ayame distinct huge.csv -k 4
ayame gen sample.csv --lines 100000
ayame cache info
ayame serve huge.csv --port 8777
```

These examples match the current `ayame --help` output. `sort --out <FILE>`
writes sorted text to a file; without `--out`, `sort` writes to stdout.
`replace` and `case` require `--out <FILE>`. `split` writes parts next to the
input by default, using `<stem>.partNNNN<.ext>` names. Output commands refuse to
overwrite existing files, so choose a new path when the target already exists.

Use [CLI Reference](CLI_REFERENCE.md) or `ayame --help` for the full command and
option list. File comparison is provided by the sister project
[ayame-diff](https://github.com/ayame-editor/ayame-diff).

## Main Features

- Opens huge files without loading the whole file into memory.
- Supports UTF-8, UTF-16LE/BE (with or without a BOM), Shift_JIS, EUC-JP,
  ISO-2022-JP, and ASCII. If text is garbled, reopen with an explicit encoding.
- Detects LF, CRLF, and classic-Mac CR-only line endings. CR-only UTF-16 files
  are not supported.
- Supports literal search, regex search, whole-word search, and case-insensitive search.
- Provides editing, undo / redo, rectangular selection, multi-cursor editing, and saving a selection to a file.
- Inspects a caret grapheme or bounded selection as Unicode scalars and original
  file bytes, including suspicious invisible/Bidi characters and color literals.
- Runs sort, replace, folder grep, grep-to-file (write only the matching lines to a new file), split, and case conversion from the GUI.
- Includes tabs, recent files, and tail-follow mode for appended logs. In the desktop build, tabs can be dragged to another Ayame window or torn out into a new one — unsaved edits move with the tab.
- Lets you customize themes, fonts, wrapping, whitespace display, zenkaku-space underline, and key bindings.
- Keeps a crash-recovery log for unsaved edits.
- Notices when another program rewrites the open file and asks before that
  change is overwritten — see [External Changes](#external-changes).

## External Changes

Files get rewritten while they are open: a build regenerates them, a log
rotates, another editor saves. Ayame remembers what the file looked like when it
last read or wrote it, and re-checks whenever the window comes back to the
front — with tail-follow on or off.

When the file has changed underneath you, Ayame asks instead of guessing:

- Coming back to the window: reload from disk, or keep what you have.
- Saving over the changed file: overwrite it, or reload first. The save is
  refused until you answer, so no external change is buried silently.

Reloading discards this tab's unsaved edits; overwriting discards what is on
disk. Saving to a different path is never blocked — only writing over the open
file asks.

Files rewritten within the same filesystem timestamp tick, at exactly the same
length, cannot be told apart from an untouched file by any check of this kind.

## Replace

`Ctrl+H` opens the replace row under the find bar.

- **Replace in selection only** (the `▤` toggle) confines `Replace All` to the
  selected text. It turns on by itself when you open the replace row over a
  selection spanning more than one line, and is unavailable when nothing is
  selected. A match that only partly overlaps the selection is left alone.
- `Replace All` can be canceled while it runs. Replacements already made stay
  and can be undone as usual; the message says how many landed.
- The replacement field keeps a history: `↑` / `↓` walk it, like the find field.

Whole-word matching cannot be combined with every regular expression. When it
cannot, the replace is refused rather than run against a wider set of matches
than whole-word means — turn `Whole Word` off to run it.

## Character and Raw-byte Inspector

Press `Ctrl+Alt+I`, or choose `Edit` -> `Inspect Character and Raw Bytes`. The
panel inspects the grapheme under the caret or a bounded normal selection. It
shows Unicode names, categories, scripts, Bidi classes, East Asian and terminal
cell widths, UTF-8/UTF-16 values, the logical position, the original file byte
offset, and both raw and re-encoded bytes. Raw-byte copy is disabled when any
part comes from the unsaved edit overlay; Ayame never silently drops that part.
One request decodes at most 256 KiB from each of 16 lines and returns at most 64
graphemes, 256 scalars, and 16 KiB of inspected text. The panel clearly marks a
result stopped at one of these limits.

Warnings identify Bidi controls, zero-width characters, NBSP, soft hyphen,
variation selectors, replacement characters, and possible mixed-script text.
They are informational and never modify copied text. Enter explicit `U+3042`,
`\u{3042}`, or `\x82\xA0` forms to preview and replace the inspected text. A
replacement that the current file encoding cannot represent is refused before
editing; accepted replacements are normal undoable, crash-recovered edits.

When the caret is on a `#RGB[A]`, `#RRGGBB[AA]`, or `0xRRGGBB[AA]` literal, the
panel also shows a swatch and color picker. Applying a color preserves the
prefix, letter case, and alpha placement; shorthand expands only when retaining
it would lose the selected RGB/alpha precision.

## External Analysis Actions and Recognized Targets

Choose `Tools` -> `External Analysis Action` to run a configured executable.
Arguments are a JSON string array, not a shell command. Ayame calls the
executable directly, so spaces, semicolons, quotes, and other shell characters
inside a path stay inside one argument. The confirmation dialog always shows
the executable and complete argument array before the run. Repository settings
are never imported or trusted implicitly, and Ayame does not generate or run
scripts automatically; any script must be selected and approved explicitly.

Available placeholders are `{file}`, `{dir}`, `{line}`, `{column}`,
`{selection_file}`, and `{snapshot_file}`. Input can be the saved file, one
fixed generation of the current dirty snapshot, or the selection on stdin / in
a private temporary file. Output can be shown as a result, inserted into a new
temporary tab, or written to a user-entered path. `stdout`, `stderr`, exit code,
and duration are reported separately. Timeout and the combined output cap are
enforced by the server; Cancel and timeout terminate the process tree, and
private snapshots/selections are removed afterward. Child processes inherit
Ayame's environment, but environment values and secrets are not displayed or
written to the UI log.

Ayame also recognizes a bounded selected token as an existing file/folder or an
`http`/`https` URL. Use the editor context menu, the bindable `Open Selected
Path or URL` action, or Ctrl+Click. The same confirmation and allowlist apply to
all three paths. Files may include a `:line:column` suffix. Other URL schemes,
including `javascript:`, `data:`, and `file:`, are rejected. In the native app,
folders open in the platform file manager; the browser build only displays the
resolved folder because browsers cannot safely reveal arbitrary local paths.

## Default Shortcuts

`Ctrl` can be entered as `Cmd` on macOS. Shortcuts can be changed from `Edit`
-> `Settings` -> `Key Bindings`, or opened directly from `Help` -> `Keyboard
Shortcuts`.

### Shortcuts and Bindable Actions

| Action | Default shortcut |
| --- | --- |
| New file | `Ctrl+N` |
| New window | `Ctrl+Shift+N` |
| Open | `Ctrl+O` |
| Save | `Ctrl+S` |
| Save as | `Ctrl+Shift+S` |
| Close tab | `Ctrl+W` |
| Reopen closed tab | `Ctrl+Shift+T` |
| Close tabs to the right / all / saved | Unassigned |
| Next / previous tab | `Ctrl+PageDown`, `Ctrl+PageUp` |
| Command palette | `Ctrl+Shift+P` |
| Find | `Ctrl+F` |
| Replace | `Ctrl+H` |
| Next / previous match | `F3`, `Shift+F3` |
| Go to line | `Ctrl+G` |
| Inspect character and raw bytes | `Ctrl+Alt+I` |
| Undo / redo | `Ctrl+Z`, `Ctrl+Y` or `Ctrl+Shift+Z` |
| Select all | `Ctrl+A` |
| Select next occurrence | `Ctrl+D` |
| Add cursor above / below | `Ctrl+Alt+↑`, `Ctrl+Alt+↓` |
| Duplicate line | `Ctrl+Shift+D` |
| Move line up / down | `Alt+↑`, `Alt+↓` |
| Delete line | `Ctrl+Shift+K` |
| Copy / cut / paste | `Ctrl+C`, `Ctrl+X`, `Ctrl+V` |
| Search options: case / word / regex | `Alt+C`, `Alt+W`, `Alt+R` |
| Explicitly open document-word completion | `Ctrl+Space` |
| Increase / decrease / reset font size | `Ctrl++`, `Ctrl+-`, `Ctrl+0` |
| Sort into a new temporary tab | Unassigned |
| Split current file | Unassigned |
| Grep a folder | Unassigned |
| Grep to file (save matching lines) | Unassigned |
| Transform selection to upper/lower/camel/Pascal/snake/kebab/constant case | Unassigned |
| Settings | Unassigned |
| Key bindings | Unassigned |
| Close the find bar or a dialog | `Esc` |

Unassigned actions appear under `Edit` -> `Settings` -> `Key Bindings`; assign
a shortcut if you use them often.

Every action in this table is rebindable, font size and paste included. Paste
keeps using the system clipboard on its default `Ctrl+V`; bound to anything
else it reads the clipboard directly, which some browsers ask permission for.

Which physical key produces `+` or `-` depends on the keyboard layout, so the
font-size bindings match with or without the `Shift` a layout needs for them.

### Menu and Status Operations

These commands have no default key binding in the current build. Open them from
the menu, status bar, or command palette (`Ctrl+Shift+P`) where listed.

| Operation | Where to open it |
| --- | --- |
| Follow Tail (`tail -f`) | `View` -> `Follow Tail`, status tail button, or command palette |
| Show whitespace and line endings | `View` -> `Show Whitespace and Line Endings` or command palette |
| Underline full-width spaces | `View` -> `Underline Full-width Spaces` or command palette |
| Word wrap | `View` -> `Word Wrap` or command palette |
| Fold or navigate document structure | Fold control in the line-number gutter, or `View` -> folding/block actions. JSON/JSONL, YAML, Python, HTML/XML, brace-based code, and multi-line log events are supported |
| Convert encoding / line endings and save | `File` -> `Encoding / Line Endings...`, or click the encoding/EOL status segment |
| Switch syntax scheme between Auto and manual | Click the `Auto · ...` status segment. Search and select non-favorite schemes under `Manage Schemes...` |
| Reopen with a different encoding | Open `Encoding / Line Endings...`, choose an encoding, then use `Reopen` |
| Save selection to file | Selection context menu |
| Cut / copy / paste / select all | `Edit` menu |
| Close other / right-hand / saved / all tabs | Tab context menu (right-click a tab) |
| Reopen closed tab | Tab context menu or `Ctrl+Shift+T` |
| Settings | `Edit` -> `Settings` |
| Key bindings | `Edit` -> `Settings` -> `Key Bindings`, or `Help` -> `Keyboard Shortcuts` |

The syntax-scheme manager edits favorites and their order, plus ordered
`file name/glob -> scheme` mappings. Preferences are stored in shared UI state
and can be imported or exported as JSON. Invalid JSON entries are skipped
individually without discarding valid entries.

Auto indent, bracket/quote closing, selection enclosure, and word completion
can each be toggled under `Edit` -> `Settings`. Input assistance stays off
during IME composition and commit; committed text remains one plain edit.
Automatic word suggestions consult only syntax vocabulary and visible/recent
caches. A document scan runs only after explicit `Ctrl+Space`, with fixed time,
scan-size, candidate-count, and memory budgets. If its deadline expires, the
popup keeps the partial suggestions found so far.

Folding stores only collapsed line intervals and fetches only visible line
ranges. A folded header reports hidden lines and any known bookmark, change,
or match counts. Search and marker navigation automatically expand a hidden
destination; editing, undo/redo, or an external reload clears stale folds.
