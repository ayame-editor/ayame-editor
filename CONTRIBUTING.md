# Contributing

Thanks for improving Ayame Editor. This project keeps the core editor in Rust,
with the web UI embedded by Cargo at build time.

By participating, you agree to follow the
[Code of Conduct](CODE_OF_CONDUCT.md).

## Development Loop

Run these before sending a pull request:

```sh
cargo fmt --all --check
cargo test --locked
cargo run -p ayame-cli -- --help
```

For changes that touch the web editor, API types, release flow, or native GUI,
also run the relevant CI gates:

```sh
npm ci --prefix crates/ayame-cli/web            # once: install pinned tooling
npm run typecheck --prefix crates/ayame-cli/web # tsc --noEmit
npm test --prefix crates/ayame-cli/web          # vitest
npm run fmt:check --prefix crates/ayame-cli/web # oxfmt (use `npm run fmt` to fix)
npm run lint --prefix crates/ayame-cli/web      # oxlint
cargo run --locked -p ayame-cli --features typegen -- typegen --check
cargo clippy --all-targets --locked -- -D warnings
cargo clippy --all-targets --locked --features gui -- -D warnings
```

`tsc`, `vitest`, `oxfmt`, and `oxlint` are pinned in
`crates/ayame-cli/web/package.json`, so `npm ci` gives you the exact versions CI
and `cargo xtask release` run — no separate binary downloads to keep in sync.
`cargo xtask release` runs all of these gates before it tags, so a release can't
land on `main` with red CI.

## Documentation

Keep English and Japanese docs in sync when changing user-facing behavior:

- English docs live in `docs/*.md`.
- Japanese docs use the `docs/*.ja.md` suffix convention.
- Navigation lives in `mkdocs.yml`; add both language pages there.
- Build the docs with `mkdocs build --strict --site-dir site` when MkDocs is
  installed.

If documentation includes UI screenshots, capture them from the current
`ayame serve` build against a small CSV fixture at a 1440px-wide browser
viewport. Refresh or remove a screenshot as soon as its controls no longer
match the current UI.

## Architecture Touch Points

- `crates/ayame-core`: mmap-backed document model, sparse line index, search,
  transforms, edit overlay, and WAL crash recovery.
- `crates/ayame-cli/src/cli`: command-line subcommands and option parsing.
- `crates/ayame-cli/src/serve`: local Axum server and `/api/*` endpoints used by
  both `ayame serve` and the native window.
- `crates/ayame-cli/web/src`: TypeScript UI sources that are type-stripped and
  embedded during the Rust build.
- `xtask`: repository automation, including release and type generation helpers.

See `docs/ARCHITECTURE.md` for the longer map.

## Pull Requests

- Keep changes scoped to one bug, feature, or documentation update.
- Include tests or an explicit reason when a change is intentionally docs-only.
- Do not check in local build output, scratch data, or downloaded third-party
  archives.
- Avoid adding new runtime dependencies unless they remove meaningful
  complexity or match an existing project boundary.

## Releases

The release path is:

```sh
cargo xtask release --bump patch
```

Use `--dry-run` to inspect the planned work without tagging. The release task
expects a clean tree and delegates cross-platform artifacts to the GitHub
Actions release workflow.
