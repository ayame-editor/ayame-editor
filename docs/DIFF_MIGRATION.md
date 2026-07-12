# Diff Migration

*日本語版: [ja/DIFF_MIGRATION.md](ja/DIFF_MIGRATION.md)*

File comparison moved from Ayame Editor to the sister project
[ayame-diff](https://github.com/hjosugi/ayame-diff). Ayame Editor now focuses on
opening, editing, searching, and transforming very large files.

## Command replacements

| Removed Ayame Editor feature | ayame-diff replacement |
| --- | --- |
| `ayame diff OLD NEW` | `ayame-diff text OLD NEW` |
| `ayame sortdiff OLD NEW` | `ayame-diff sorted OLD NEW` |
| `ayame sort-diff OLD NEW` | `ayame-diff sorted OLD NEW` |
| Tools → Two-file Diff | `ayame-diff gui` or `ayame-diff serve` |

Install from [the latest ayame-diff release](https://github.com/hjosugi/ayame-diff/releases/latest),
or with Go 1.23 or later:

```sh
go install github.com/hjosugi/ayame-diff/cmd/ayame-diff@latest
```

Ayame Editor keeps a migration-only error for the former CLI command names for
one release. The comparison implementation, HTTP endpoint, Web dialog, native
menu entry, and editor-side comparison tests are no longer shipped.
