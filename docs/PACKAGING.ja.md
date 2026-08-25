# 配布

Ayame の GitHub Releases を配布元の source of truth にします。各 package manager
の manifest は release asset と checksum を参照します。

## Scoop

このリポジトリはそのまま Scoop bucket として使えます。

```powershell
scoop bucket add ayame-editor https://github.com/ayame-editor/ayame-editor
scoop install ayame
scoop update ayame
```

manifest は [bucket/ayame.json](https://github.com/ayame-editor/ayame-editor/blob/main/bucket/ayame.json)
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

## Windows コード署名

審査を通過した OSS project は
[SignPath Foundation](https://signpath.org/) の Authenticode 署名を無料で
利用できます。SignPath 未設定時、release workflow は従来の未署名 release 経路を
維持します。次の repository secret が 4 つすべて設定されている場合、Windows job は
次の順で処理します。

1. 未署名の `ayame-<tag>-windows-x86_64.exe` を build する。
2. SignPath が build provenance を検証できるよう GitHub Actions artifact として
   upload する。
3. [`SignPath/github-action-submit-signing-request`](https://github.com/SignPath/github-action-submit-signing-request)
   へ送信する。
4. staging 中の executable を署名済み結果で置き換える。
5. GitHub Release の公開前に `.sha256` を再計算する。

そのため、署名前 binary の checksum が公開されることはありません。secret が
すべて未設定なら従来どおり未署名で release し、一部だけ設定されている場合は
意図しない fallback を避けるため Windows job を失敗させます。

Windows build は、SignPath への送信前に `ProductName`、`FileVersion`、
`ProductVersion` metadata を埋め込みます。Product name は `Ayame Editor` に固定し、
2 つの version field は Cargo package version から生成します。Release workflow は
未署名 artifact の upload 前にこれらを検証するため、SignPath の artifact
configuration でも同じ制約を適用できます。

| Repository secret | 用途 |
| --- | --- |
| `SIGNPATH_API_TOKEN` | Submitter 権限を持つ SignPath user の API token。 |
| `SIGNPATH_ORGANIZATION_ID` | SignPath organization ID。 |
| `SIGNPATH_PROJECT_SLUG` | Ayame Editor 用 SignPath project slug。 |
| `SIGNPATH_SIGNING_POLICY_SLUG` | release binary に使う signing policy。 |

SignPath の初期設定は owner 作業です。Foundation への申請、SignPath GitHub App の
install/許可、repository と既定の GitHub.com trusted build system の関連付け、
`.exe` を含む ZIP artifact を受け付ける artifact configuration の作成が必要です。
release workflow は GitHub-hosted runner を使い、provenance 検証に必要な
`actions: read` を token へ付与します。

download した release は PowerShell で検証できます。

```powershell
Get-AuthenticodeSignature .\ayame-v0.0.0-windows-x86_64.exe | Format-List
Get-FileHash -Algorithm SHA256 .\ayame-v0.0.0-windows-x86_64.exe
```

最初の command が有効な署名を報告することを確認し、2 番目の hash を同じ release
の `.sha256` または `SHA256SUMS` と比較します。有効な証明書があっても
SmartScreen reputation の蓄積には時間がかかる場合があります。

公開
[コード署名ポリシー](https://github.com/ayame-editor/ayame-editor/blob/main/README.ja.md#code-signing-policyコード署名ポリシー)
には、Foundation 申請に必要な署名対象、build provenance、team role、承認ルール、
および
[プライバシーポリシー](https://github.com/ayame-editor/ayame-editor/blob/main/PRIVACY.ja.md)
を記載しています。

## self-update の扱い

`ayame update` は standalone install 用です。Package manager 管理下の Ayame を検出
した場合は自己変更せず、manager 側のコマンドを案内します。

- Homebrew: `brew upgrade ayame`
- Scoop: `scoop update ayame`
- Nix: Ayame を提供している Nix profile / flake 側で更新

`ayame remove` も同じ方針です。Package manager 管理下の install は
`brew uninstall`、`scoop uninstall`、または Nix 側で削除します。

ソースからの通常ビルドでは self-update が既定で有効です。サーバー専用配布では
TLS・checksum・archive の依存スタックを除外できます。

```sh
cargo build --release --locked -p ayame-cli --no-default-features
```

このビルドでも `ayame update` / `ayame remove` は認識されますが、package manager
での管理、または `--features self-update` を付けた再ビルドが必要であることを案内します。

## リリース署名

`ayame update` は 3 段の連鎖をたどります。Ed25519 署名が `.sha256` チェックサム
ファイルを保証し、チェックサムがアーティファクトを保証し、そこで初めてインス
トールされます。署名が無ければチェックサムは何も証明しません — リリース資産を
差し替えられる者は、そのチェックサムも差し替えられるからです。

鍵ペアは一度だけ生成し、2 か所に置きます。

```sh
cargo xtask keygen
```

- **秘密鍵**は `AYAME_UPDATE_SIGNING_KEY` リポジトリ secret とし、リリース
  ワークフローの署名ステップだけが使います
- **公開鍵**は `AYAME_UPDATE_PUBKEY` リポジトリ変数とし、リリースビルド時に
  バイナリへ埋め込まれます

両者は必ず揃えて設定します。リリースワークフローは片方だけの状態を拒否します。
公開鍵だけならビルドがあらゆる更新を拒否することになり、secret だけならどのビルド
も検証しない署名を配布することになるからです。後者の場合は設定すべき値を出力します
（公開鍵はログに出しても安全です）。ローカルでも取得できます。

```sh
AYAME_UPDATE_SIGNING_KEY=... cargo xtask pubkey
```

さらに `cargo xtask sign` は鍵の対応が取れていなければ拒否し、`AYAME_UPDATE_PUBKEY`
の形式が不正ならビルド時点で失敗します（更新時ではありません）。

`AYAME_UPDATE_PUBKEY` を持たないビルド（ローカルビルドやフォーク）は従来どおり
チェックサムのみで動作し、更新時にその旨を表示します。鍵を持つビルドは署名済み
リリースしかインストールしません。`.sha256.sig` の欠落や不一致は常に失敗であり、
フォールバックはしません。

鍵をローテーションするときは `cargo xtask keygen` を再実行し、両方の設定を更新
します。旧鍵を信頼する出荷済みビルドがすべて置き換わるまで、旧鍵は有効に保ちます。
