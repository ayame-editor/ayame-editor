# Contributing

Thanks for improving Ayame Editor. This project keeps the core editor in Rust,
with the web UI embedded by Cargo at build time.

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
npx -y -p typescript@5 tsc --noEmit -p crates/ayame-cli/web/tsconfig.json
cargo run --locked -p ayame-cli --features typegen -- typegen --check
cargo clippy --all-targets --locked -- -D warnings
cargo clippy --all-targets --locked --features gui -- -D warnings
```

The CI workflow downloads pinned `oxfmt` and `oxlint` binaries. If you have the
same versions locally, run:

```sh
find crates/ayame-cli/web/src -name '*.ts' ! -name '*.d.ts' -print0 | xargs -0 oxfmt --check
oxlint --max-warnings 0 crates/ayame-cli/web/src
```

## Documentation

Keep English and Japanese docs in sync when changing user-facing behavior:

- English docs live in `docs/*.md`.
- Japanese docs live in `docs/ja/*.md`.
- Navigation lives in `mkdocs.yml`; add both language pages there.
- Build the docs with `mkdocs build --strict --site-dir site` when MkDocs is
  installed.

If a change affects the UI layout, refresh screenshots under `docs/assets/`.
The current set is:

- `screenshot-main.png`
- `screenshot-settings.png`
- `screenshot-tools.png`
- `screenshot-diff.png`

They were captured from `ayame serve` against a small CSV fixture at a 1440px
wide browser viewport.

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
