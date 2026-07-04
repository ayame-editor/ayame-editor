# リリース

*English: [../RELEASE.md](../RELEASE.md)*

## 1コマンド

以下のすべて（ゲート → 成果物 → スモーク → タグ → push → ワークフロー監視）は
`xtask` クレートで自動化されている — 純 Rust なので bash/node 依存なしに
Linux・macOS・Windows で同一に動く:

```sh
cargo xtask release                  # Cargo.toml にあるバージョンのままリリース
cargo xtask release --bump patch     # bump + "release: vX.Y.Z" コミット + リリース
cargo xtask release --dry-run        # 全チェックを実行し、タグ/push の手前で停止
```

（`scripts/release.sh` は同じコマンドの薄いラッパー。）

以下の各節は、自動化が何をしているか、および自動化でカバーできない手動の
プラットフォーム確認を記録するもの。

## ローカルゲート

CI が守っているのと同じチェックに加えて、デスクトップアプリが出荷物であるため
GUI のリリースビルドも実行する:

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
cargo build --release --locked --features gui
scripts/crash-isolation-test.sh
```

Linux では GUI コマンドの前に WebKitGTK の開発パッケージを入れておく。
ディストリビューション別のパッケージ名は [DEVELOPMENT.md](DEVELOPMENT.md) を参照。

## ローカル成果物

```sh
scripts/release-local.sh
version="$(cargo pkgid -p ayame-cli | sed 's/.*#//')"
target="$(rustc -Vv | awk '/^host:/ { print $2 }')"
./dist/ayame-v${version}-${target} --version
```

バイナリには `ayame serve` が使う web アセットが埋め込まれており、実行ファイルの
横に `web/` ディレクトリを置く必要はない。

## プラットフォーム別スモーク確認

タグを切る前に、手元にあるプラットフォームでローカル成果物を確認する:

- `ayame --version`
- `ayame` がファイルなしでネイティブウィンドウを開く
- `ayame <FILE>` がファイルをネイティブウィンドウで開く
- 小さな UTF-8 ファイルで `gen/stat/search/find/sort/replace/case/split` のスモークテスト
- 検索とワーカー実行の操作で `--encoding Shift_JIS` のスモークテスト
- ダーティ編集の保存 / 別名保存 / 未保存確認付きクローズの一連フロー

macOS と Windows の確認はリリースワークフローの成果物経由で行い、実機が使える
場合はネイティブメニュー・WebView キーボードショートカット・Dock/タスクバー
アイコン・ドラッグ&ドロップ・保存後クリーンアップの手動 issue チェックリストで行う。

## GitHub リリース

ワークフローはタグの push、または GitHub Actions -> Release -> Run workflow の
手動実行のどちらでも開始できる。

```sh
version="$(cargo pkgid -p ayame-cli | sed 's/.*#//')"
git tag "v${version}"
git push origin "v${version}"
```

リリースワークフローがアップロードするもの:

- `ayame-v<version>-linux-x86_64`
- `ayame-v<version>-windows-x86_64.exe`
- `Ayame.app` を含む `ayame-v<version>-macos-x86_64.zip`
- `Ayame.app` を含む `ayame-v<version>-macos-aarch64.zip`
- ファイル毎の `.sha256`
- `SHA256SUMS`

リリースアセットのダウンロード後は次で検証する:

```sh
sha256sum -c SHA256SUMS
```
