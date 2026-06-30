# Release

## Local Gate

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
scripts/crash-isolation-test.sh
```

## Local Artifact

```sh
scripts/release-local.sh
./dist/ayame-v0.1.7-$(rustc -Vv | awk '/^host:/ { print $2 }') --version
```

The binary embeds the web assets used by `ayame serve`; no `web/` directory is
needed next to the executable.

## GitHub Release

```sh
git tag v0.1.7
git push origin v0.1.7
```

The release workflow uploads:

- `ayame-<tag>-linux-x86_64-musl` (static Linux binary)
- `ayame-<tag>-windows-x86_64.exe` (static CRT)
- `ayame-<tag>-macos-x86_64`
- `ayame-<tag>-macos-aarch64`
- per-file `.sha256`
- `SHA256SUMS`

Manual release is also available from GitHub Actions → Release → Run workflow.

After downloading release assets, verify them with:

```sh
sha256sum -c SHA256SUMS
```
