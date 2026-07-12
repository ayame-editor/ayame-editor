# ADR 0001: Extract diff into ayame-diff

*日本語版: [0001-diff-extraction-and-deprecation.md](0001-diff-extraction-and-deprecation.md)*

- Status: Accepted (2026-07-10)
- Related issue: hjosugi/ayame-editor#93
- Extraction epic: hjosugi/ayame-editor#104
- Destination roadmap: hjosugi/ayame-diff#26

## Context

Ayame Editor should focus on opening and editing very large files. Full file,
directory, archive, and structured-data comparison belongs in the sister
project ayame-diff. Maintaining both implementations would split testing and
allow behavior to drift.

The extracted surface included the `diff` and `sortdiff` CLI implementation,
the `/api/diff` endpoint, the Web comparison dialog, native and command menu
entries, comparison CSS, and editor-side regression tests.

## Decision

The move uses two releases:

| Release | Change |
| --- | --- |
| v0.6.0 | Deprecate editor comparison after ayame-diff v0.4.0 is available. Keep behavior working and show migration guidance. |
| v0.7.0 | Remove implementation, API, UI, menu entries, documentation of the old feature, and editor-side comparison tests. |

No new comparison features are added to Ayame Editor during migration. Bug
fixes remain allowed, while algorithm and output improvements go to ayame-diff.
After removal, the former CLI names return migration guidance for one release;
automatic GUI handoff is not a release blocker.

## Outcome

The v0.6.0 deprecation shipped first. The removal work then deleted the editor
implementation, HTTP endpoint, Web/native UI, and related tests. The
[diff migration guide](../DIFF_MIGRATION.md) provides the command and GUI
replacement table.
