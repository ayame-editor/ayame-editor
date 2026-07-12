# Migrating comparison workflows to ayame-diff

Ayame Editor v0.7.0 removes its comparison implementation, `/api/diff`, and
two-file comparison UI. Comparison is maintained in the sister project
[ayame-diff](https://github.com/hjosugi/ayame-diff), which supports the same
large-file workflows and adds a dedicated GUI and more comparison modes.

## Install

Download a build from [ayame-diff Releases](https://github.com/hjosugi/ayame-diff/releases/latest),
or install from source with Go 1.23 or newer:

```sh
go install github.com/hjosugi/ayame-diff/cmd/ayame-diff@latest
```

## Command mapping

| Ayame Editor before v0.7.0 | ayame-diff |
| --- | --- |
| `ayame diff OLD NEW` | `ayame-diff text OLD NEW` |
| `ayame diff OLD NEW --side-by-side` | `ayame-diff text --side-by-side OLD NEW` |
| `ayame diff OLD NEW --json` | `ayame-diff text --json OLD NEW` |
| `ayame diff OLD NEW --summary` | `ayame-diff text --summary OLD NEW` |
| `ayame sortdiff OLD NEW` | `ayame-diff sorted OLD NEW` |
| Web or native Tools → Two-file Diff | `ayame-diff --gui OLD NEW` |

Ayame Editor v0.7.0 keeps only an error stub for the old CLI names so scripts
fail clearly instead of silently doing the wrong thing. Update scripts during
this compatibility window; the aliases may disappear in a later release.

For current flags and comparison modes, use the
[ayame-diff documentation](https://hjosugi.github.io/ayame-diff/).
