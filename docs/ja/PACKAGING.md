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

## release 署名

自己更新は `cli/update.rs` に組み込んだ Ed25519 公開鍵を trust anchor にします。
release workflow は対応する PEM 秘密鍵を repository secret
`AYAME_UPDATE_SIGNING_KEY` から受け取り、各 `<asset>.sha256` の正確な bytes を署名し、
16 進表現の `<asset>.sha256.sig` を公開します。secret が無い場合、release 公開は
fail closed します。

秘密鍵は repository 外で mode `0600` にし、release maintainer だけが参照できる状態で
secret backup を保管してください。鍵 rotation では、組み込み公開鍵を更新した版を先に
配布してから、新しい鍵だけで署名した release へ切り替える必要があります。
