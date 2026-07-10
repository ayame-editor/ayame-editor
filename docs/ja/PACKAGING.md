# 配布

Ayame の GitHub Releases を配布元の source of truth にします。各 package manager
の manifest は release asset と checksum を参照します。

## Scoop

このリポジトリはそのまま Scoop bucket として使えます。

```powershell
scoop bucket add ayame-editor https://github.com/hjosugi/ayame-editor
scoop install ayame
scoop update ayame
```

manifest は [bucket/ayame.json](https://github.com/hjosugi/ayame-editor/blob/main/bucket/ayame.json)
です。後で専用 bucket を作る場合は、`hjosugi/scoop-bucket` へコピーします。

## Homebrew

Homebrew tap 用ファイルは `packaging/homebrew/` に置いています。

- `Casks/ayame.rb`: macOS の `Ayame.app` をインストール。
- `Formula/ayame.rb`: macOS / Linux の `ayame` CLI をインストール。

`hjosugi/homebrew-tap` のような tap repository へ配置します。

```sh
mkdir -p Casks Formula
cp /path/to/ayame-editor/packaging/homebrew/Casks/ayame.rb Casks/
cp /path/to/ayame-editor/packaging/homebrew/Formula/ayame.rb Formula/
```

公開後の想定コマンド:

```sh
brew install --cask hjosugi/tap/ayame
brew install hjosugi/tap/ayame
brew upgrade ayame
```

## self-update の扱い

`ayame update` は standalone install 用です。Package manager 管理下の Ayame を検出
した場合は自己変更せず、manager 側のコマンドを案内します。

- Homebrew: `brew upgrade ayame`
- Scoop: `scoop update ayame`
- Nix: Ayame を提供している Nix profile / flake 側で更新

`ayame remove` も同じ方針です。Package manager 管理下の install は
`brew uninstall`、`scoop uninstall`、または Nix 側で削除します。
