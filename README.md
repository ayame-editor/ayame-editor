<!-- i18n: language-switcher -->
[English](README.md) | [日本語](README.ja.md)

# Ayame Editor

*日本語版: [README.ja.md](README.ja.md)*

A fast desktop text editor for huge files.

Runs on macOS, Windows, and Linux.

> **Comparing files?** Comparison moved to the sister project
> **[ayame-diff](https://github.com/ayame-editor/ayame-diff)**. Ayame Editor v0.7.0
> removes its `diff` / `sortdiff` implementations and two-file comparison UI.

## Features

- View, search, and edit huge files without loading the whole file into memory.
- Supports UTF-8, UTF-16LE/BE (with or without a BOM), Shift_JIS, EUC-JP,
  ISO-2022-JP, and ASCII.
- Run search, replace, sort, folder grep, and file splitting from the GUI.
- Use CLI commands such as `stat`, `head`, `tail`, `line`, `lines`, `search`,
  `sort`, `replace`, `case`, `grep-lines`, `split`, `group`, `top`, `distinct`,
  `gen`, `serve`, `gui`, `cache`, `update`, and `remove`.
- Includes tabs, rectangular selection, multi-cursor editing, and tail-follow mode.
- Customizable themes, fonts, wrapping, whitespace display, and key bindings.

## Install

Download the build for your OS from the
[latest release](https://github.com/ayame-editor/ayame-editor/releases/latest).

- macOS: `Ayame.app`
- Windows: `ayame-*.exe`
- Linux: single executable

You can also install from the terminal.

Scoop users can install from this repository bucket:

```powershell
scoop bucket add ayame-editor https://github.com/ayame-editor/ayame-editor
scoop install ayame
```

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

Update later with `ayame update`. Remove a standalone install with
`ayame remove --yes`. Nix-managed binaries should be updated or removed through
Nix instead of self-modifying `/nix/store`.

Homebrew tap templates are in `packaging/homebrew/` for publishing
`brew install --cask hjosugi/tap/ayame` and `brew install hjosugi/tap/ayame`.

## Sister Project

[ayame-diff](https://github.com/ayame-editor/ayame-diff) handles text, sorted-text,
CSV/TSV, directory, archive, binary, and three-way comparisons from its CLI or
GUI. Use Ayame Editor to open and edit huge files, and ayame-diff to compare them.

## More

The docs site includes the user guide, full CLI reference, architecture notes,
default shortcuts, install notes, build steps, and Linux runtime packages:
[docs site](https://ayame-editor.github.io/ayame-editor/).
Project participation is governed by the [Code of Conduct](CODE_OF_CONDUCT.md).

For files where a single wrong byte is unacceptable, see the
[data integrity guarantees](docs/DATA_INTEGRITY.md) — the correctness promises
(byte-exact save, crash recovery, encoding round-trips) and the tests that
verify each one.

## Code signing policy

Status: pending SignPath Foundation approval and production configuration.
Until then, Windows releases remain unsigned.

> Free code signing provided by [SignPath.io](https://signpath.io/), certificate
> by [SignPath Foundation](https://signpath.org/).

### What will be signed

- Native Windows executables published by this project on
  [GitHub Releases](https://github.com/ayame-editor/ayame-editor/releases).
- macOS and Linux artifacts are currently outside this code-signing policy.

### Build and signing process

- Release artifacts are built from this public repository by
  [GitHub Actions](https://github.com/ayame-editor/ayame-editor/actions).
- Only artifacts produced by the repository's release workflow are submitted
  to SignPath. SignPath holds the private signing key; this repository does not
  store or handle it.

### Team roles

- Authors: [hjosugi](https://github.com/hjosugi), the repository owner, may
  modify the repository without an additional review.
- Reviewers: [hjosugi](https://github.com/hjosugi) reviews changes proposed by
  external contributors before merge.
- Approvers: [hjosugi](https://github.com/hjosugi) explicitly approves every
  signing request before an artifact is signed.

### Privacy

Ayame does not upload opened document contents or telemetry. Its optional and
configured network behavior, including GitHub release checks and downloads, is
documented in the [privacy policy](PRIVACY.md).

See [Windows code signing](docs/PACKAGING.md#windows-code-signing) for the
release workflow, secrets, and verification procedure.

## License

0BSD. You can use, copy, modify, and distribute this project for almost any purpose.
