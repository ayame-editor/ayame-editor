# Packaging

Ayame's GitHub Releases remain the source of truth. Package manager manifests
should point at those release assets and their checksums.

## Scoop

This repository can be used directly as a Scoop bucket:

```powershell
scoop bucket add ayame-editor https://github.com/hjosugi/ayame-editor
scoop install ayame
scoop update ayame
```

The manifest is [bucket/ayame.json](https://github.com/hjosugi/ayame-editor/blob/main/bucket/ayame.json).
For a dedicated bucket later, copy it into `hjosugi/scoop-bucket`.

## Homebrew

Homebrew tap files are staged under `packaging/homebrew/`:

- `Casks/ayame.rb` installs the macOS `Ayame.app`.
- `Formula/ayame.rb` installs the `ayame` CLI on macOS and Linux.

Publish them from a tap repository such as `hjosugi/homebrew-tap`:

```sh
mkdir -p Casks Formula
cp /path/to/ayame-editor/packaging/homebrew/Casks/ayame.rb Casks/
cp /path/to/ayame-editor/packaging/homebrew/Formula/ayame.rb Formula/
```

Expected user commands after publishing:

```sh
brew install --cask hjosugi/tap/ayame
brew install hjosugi/tap/ayame
brew upgrade ayame
```

## Windows code signing

Accepted open-source projects can use
[SignPath Foundation](https://signpath.org/) for free Authenticode signing.
The release workflow keeps the current unsigned release path when SignPath is
not configured. When all four repository secrets below are present, the
Windows job instead:

1. builds the unsigned `ayame-<tag>-windows-x86_64.exe`;
2. uploads it as a GitHub Actions artifact so SignPath can verify its build
   provenance;
3. submits it through
   [`SignPath/github-action-submit-signing-request`](https://github.com/SignPath/github-action-submit-signing-request);
4. replaces the staged executable with the signed result; and
5. recalculates its `.sha256` before the GitHub Release is published.

This repository therefore never publishes a checksum for the pre-signing
binary. If none of the secrets are set, releases remain unsigned as before. A
partial configuration fails the Windows job instead of silently falling back.

The Windows build embeds `ProductName`, `FileVersion`, and `ProductVersion`
metadata before submission. Product name is fixed to `Ayame Editor`; both
version fields are derived from the Cargo package version. The release workflow
checks these values before uploading the unsigned artifact so the SignPath
artifact configuration can enforce the same restrictions.

| Repository secret | Purpose |
| --- | --- |
| `SIGNPATH_API_TOKEN` | API token for a SignPath user with submitter permission. |
| `SIGNPATH_ORGANIZATION_ID` | SignPath organization ID. |
| `SIGNPATH_PROJECT_SLUG` | SignPath project slug for Ayame Editor. |
| `SIGNPATH_SIGNING_POLICY_SLUG` | Signing policy used for release binaries. |

SignPath setup is an owner task: apply to the Foundation, install/authorize the
SignPath GitHub App, link the repository to the predefined GitHub.com trusted
build system, and configure an artifact configuration that accepts the ZIP
artifact containing the `.exe`. The release workflow uses GitHub-hosted runners
and grants its token `actions: read`, as required for provenance verification.

Verify a downloaded release in PowerShell:

```powershell
Get-AuthenticodeSignature .\ayame-v0.0.0-windows-x86_64.exe | Format-List
Get-FileHash -Algorithm SHA256 .\ayame-v0.0.0-windows-x86_64.exe
```

The first command must report a valid signature. Compare the second command's
hash with the matching `.sha256` file or `SHA256SUMS` from the same release.
SmartScreen reputation can still take time to accumulate even with a valid
certificate.

The public
[code signing policy](https://github.com/hjosugi/ayame-editor#code-signing-policy)
documents the signed files, build provenance, team roles, approval rule, and
[privacy policy](https://github.com/hjosugi/ayame-editor/blob/main/PRIVACY.md)
required for the Foundation application.

## Self-Update Policy

`ayame update` is for standalone installs. If Ayame detects a package-manager
install, it refuses to modify it and points users at the manager-native command:

- Homebrew: `brew upgrade ayame`
- Scoop: `scoop update ayame`
- Nix: update through the Nix profile or flake that provides Ayame

The same rule applies to `ayame remove`: package-manager installs should be
removed with `brew uninstall`, `scoop uninstall`, or Nix.

Source builds include self-update support by default. A server-only deployment
can omit the TLS, checksum, and archive stack:

```sh
cargo build --release --locked -p ayame-cli --no-default-features
```

In that build, `ayame update` and `ayame remove` remain recognizable commands
but explain that package-manager management or a rebuild with
`--features self-update` is required.
