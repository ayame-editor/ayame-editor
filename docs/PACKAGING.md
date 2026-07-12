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

## Self-Update Policy

`ayame update` is for standalone installs. If Ayame detects a package-manager
install, it refuses to modify it and points users at the manager-native command:

- Homebrew: `brew upgrade ayame`
- Scoop: `scoop update ayame`
- Nix: update through the Nix profile or flake that provides Ayame

The same rule applies to `ayame remove`: package-manager installs should be
removed with `brew uninstall`, `scoop uninstall`, or Nix.

## Release signing

Self-update trusts an Ed25519 public key compiled into `cli/update.rs`. The
release workflow requires the matching PEM private key in the repository secret
`AYAME_UPDATE_SIGNING_KEY`. It signs the exact bytes of every
`<asset>.sha256` file and publishes the hexadecimal signature as
`<asset>.sha256.sig`; release publication fails closed when the secret is
missing.

Keep the private key outside the repository with mode `0600`, restrict access to
release maintainers, and back it up as a secret. Key rotation requires updating
the compiled public key and shipping that update before releases signed only by
the new key.
