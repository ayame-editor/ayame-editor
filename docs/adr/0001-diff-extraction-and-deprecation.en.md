<!-- i18n: language-switcher -->
[English](0001-diff-extraction-and-deprecation.en.md) | [日本語](0001-diff-extraction-and-deprecation.md)

# ADR 0001: Policy for Transferring diff Functionality to ayame-diff and Deprecation Schedule

- Status: Accepted (2026-07-10)
- Related Issue: hjosugi/ayame-editor#93
- Epic for Extraction: hjosugi/ayame-editor#104
- Transfer Roadmap: hjosugi/ayame-diff#26

## Background

With the formation of a sister project, the diff-related features will be extracted to hjosugi/ayame-diff.
ayame-editor will focus on **"Opening and editing large files"**, while comprehensive comparison features will be handled by
ayame-diff (clarifying product boundaries following Sindre Sorhus's approach: deeply resolving a single friction point).

Extraction targets:

- `crates/ayame-cli/src/diff.rs` (subcommands `diff` / `sortdiff`)
- `/api/diff` endpoint in `serve/ops.rs`
- The two-file diff view in `web/src/search.ts`
- Diff items in native menus / command palette / shortcuts
- CSS for diff styling that overlays `.grep-panel`

## Decisions

### 1. Gradual Deprecation (Two Phases)

Do not remove the functionality without a replacement. **Release ayame-diff v0.4.0 (the replacement)** first.

| Release | Details |
| --- | --- |
| **v0.6.0 (Deprecation)** | After releasing ayame-diff v0.4.0. Show **deprecation warnings + guidance to ayame-diff** when executing `ayame diff` / `ayame sortdiff` and in the Web diff dialog. Keep the code (`diff.rs`, etc.) intact without changing behavior. |
| **v0.7.0 (Removal)** | Remove implementation, API (`/api/diff`), Web UI, native menu items, docs, and tests. |

The reason for two phases instead of a single removal: to give existing users a transition period and a clear alternative.
Breaking changes will always be announced at least one release prior.

### 2. Diff Path in Web UI

v0.7.0 will primarily perform a **simple removal**.
If ayame-diff is installed, whether to invoke it externally will be decided based on demand and implementation cost, in a **separate issue (#102 integration)** (not a blocker for v0.7.0).
During the deprecation period (v0.6.0), guidance banners (#97) will direct users to ayame-diff.

### 3. Avoiding Dual Maintenance During Transfer (Freeze)

Until the extraction is complete, **adding new diff features on the editor side will be frozen**.
Bug fixes are allowed, but new features and algorithm improvements will be made on the ayame-diff side (#5–#8), and not backported to the editor.
This prevents drift from maintaining two implementations.

### 4. Notification Text for Breaking Changes

The following will be clearly stated in the CHANGELOG / release notes:

- v0.6.0: "`diff` / `sortdiff` are deprecated. The successor is ayame-diff (link).
  Scheduled for removal in the next release v0.7.0."
- v0.7.0: "`diff` / `sortdiff`, `/api/diff`, and Web diff view are removed.
  Please migrate comparison features to ayame-diff (link)."

## Dependency Order

```
ayame-diff #5〜#8 (Migration and replacement implementation)
        │
ayame-diff v0.4.0 release (#24)   ← This comes first
        │
ayame-editor v0.6.0 deprecation (#94 #97 #99 #100 #102)
        │
ayame-editor v0.7.0 removal (#94 #95 #96 #97 #98 #99 #101)
        │
ayame-editor #103 release
```

## Completion Criteria (Fulfilled by this ADR)

The schedule (two phases) and Web UI guidance (simple removal + deprecation guidance during the deprecation period) are finalized,
the freeze policy and notification wording are determined.
The extraction implementation issue (#94–#102) can now be started.