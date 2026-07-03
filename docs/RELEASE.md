# Release

## One command

Everything below (gate → artifact → smoke → tag → push → watch the workflow)
is scripted:

```sh
scripts/release.sh                  # release the version already in Cargo.toml
scripts/release.sh --bump patch     # bump + commit "release: vX.Y.Z" + release
scripts/release.sh --dry-run        # run every check, stop before tag/push
```

The sections below document what the script does, and the manual platform
checks it cannot automate.

## Local Gate

Run the same checks CI protects, plus the GUI release build because the
desktop app is the shipped product:

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
cargo build --release --locked --features gui
scripts/crash-isolation-test.sh
```

On Linux, install the WebKitGTK development packages before the GUI commands.
See [DEVELOPMENT.md](DEVELOPMENT.md) for distro-specific package names.

## Local Artifact

```sh
scripts/release-local.sh
version="$(cargo pkgid -p ayame-cli | sed 's/.*#//')"
target="$(rustc -Vv | awk '/^host:/ { print $2 }')"
./dist/ayame-v${version}-${target} --version
```

The binary embeds the web assets used by `ayame serve`; no `web/` directory is
needed next to the executable.

## Platform Smoke Checks

Before tagging, verify the local artifact on the platform you are holding:

- `ayame --version`
- `ayame` opens a native window without a file
- `ayame <FILE>` opens a file in the native window
- `gen/stat/search/find/sort/replace/case/split` smoke tests on a small UTF-8 file
- `--encoding Shift_JIS` smoke test for search and worker-backed operations
- dirty edit save / save-as / close-with-unsaved-confirm flow

macOS and Windows checks are done through the release workflow artifacts and,
when hardware is available, the manual issue checklist for native menu,
WebView keyboard shortcuts, Dock/taskbar icon, drag-and-drop, and save cleanup.

## GitHub Release

The workflow can be started either by pushing a tag or manually from
GitHub Actions -> Release -> Run workflow.

```sh
version="$(cargo pkgid -p ayame-cli | sed 's/.*#//')"
git tag "v${version}"
git push origin "v${version}"
```

The release workflow uploads:

- `ayame-v<version>-linux-x86_64`
- `ayame-v<version>-windows-x86_64.exe`
- `ayame-v<version>-macos-x86_64.zip` containing `Ayame.app`
- `ayame-v<version>-macos-aarch64.zip` containing `Ayame.app`
- per-file `.sha256`
- `SHA256SUMS`

After downloading release assets, verify them with:

```sh
sha256sum -c SHA256SUMS
```
