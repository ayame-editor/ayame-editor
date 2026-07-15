<!-- i18n: language-switcher -->
[English](README.md) | [日本語](README.ja.md)

# Ayame Editor

*English: [README.md](README.md)*

巨大ファイルをすばやく開けるデスクトップ・テキストエディタです。

macOS、Windows、Linux で動作します。

> **ファイル比較は？** 比較機能は姉妹プロジェクト
> **[ayame-diff](https://github.com/hjosugi/ayame-diff)** へ移管しました。
> Ayame Editor v0.7.0 では `diff` / `sortdiff` の実装と 2 ファイル比較 UI を
> 削除します。[移行ガイド](docs/ja/MIGRATING_TO_AYAME_DIFF.md)をご覧ください。

## 主な機能

- 巨大ファイルを全体読み込みせずに表示・検索・編集できます。
- UTF-8、UTF-16LE/BE（BOM あり / なし）、Shift_JIS、EUC-JP、
  ISO-2022-JP、ASCII に対応します。
- GUI では検索、置換、ソート、フォルダ内検索、ファイル分割を実行できます。
- CLI では `stat`、`head`、`tail`、`line`、`lines`、`search`、`sort`、
  `replace`、`case`、`grep-lines`、`split`、`group`、`top`、`distinct`、
  `gen`、`serve`、`gui`、`cache`、`update`、`remove` などを使えます。
- タブ、矩形選択、マルチカーソル、tail -f 風の末尾追従を備えています。
- テーマ、フォント、折り返し、空白表示、キー設定を変更できます。

## インストール

[最新リリース](https://github.com/hjosugi/ayame-editor/releases/latest) から、お使いの OS 向けのビルドをダウンロードしてください。

- macOS: `Ayame.app`
- Windows: `ayame-*.exe`
- Linux: 単体実行ファイル

ターミナルからインストールすることもできます。

Scoop はこのリポジトリを bucket として使えます。

```powershell
scoop bucket add ayame-editor https://github.com/hjosugi/ayame-editor
scoop install ayame
```

macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.sh | sh
```

Windows (PowerShell):

```powershell
pwsh -NoProfile -Command "irm https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.ps1 | iex"
```

Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.sh | sh
```

以後は `ayame update` で更新できます。standalone install の削除は
`ayame remove --yes` です。Nix 管理の binary は `/nix/store` を自己変更せず、
Nix 側で更新・削除してください。

Homebrew tap 用 template は `packaging/homebrew/` にあります。
`brew install --cask hjosugi/tap/ayame` と `brew install hjosugi/tap/ayame`
で配れる形です。

## 姉妹プロジェクト

[ayame-diff](https://github.com/hjosugi/ayame-diff) はテキスト、ソート済み
テキスト、CSV/TSV、フォルダ、アーカイブ、バイナリ、3-way 比較を CLI と GUI で
提供します。巨大ファイルの閲覧・編集には Ayame Editor、比較には ayame-diff を
使い分けてください。

## 詳細

ユーザー向けガイド、全 CLI リファレンス、アーキテクチャ、既定ショートカット、
インストール、ビルド手順、Linux の実行時パッケージは
[ドキュメントサイト](https://hjosugi.github.io/ayame-editor/ja/) にまとめています。

1 バイトの誤りも許されない用途向けに、[データ完全性の保証](docs/ja/DATA_INTEGRITY.md)
（バイト正確な保存・クラッシュ復元・エンコーディング往復などの正確性の約束と、
それを検証するテスト）をまとめています。

## Code signing policy（コード署名ポリシー）

Project の審査通過・設定完了後は、[SignPath.io](https://signpath.io/) による無料の
コード署名（証明書: [SignPath Foundation](https://signpath.org/)）を利用します。
それまでは Windows release は未署名です。

- Committer、reviewer、signing approver: repository owner の
  [hjosugi](https://github.com/hjosugi)。Repository collaborator は割り当てられた
  GitHub 権限に従って参加し、外部 contribution は release 署名前に owner が
  review します。
- Privacy: Ayame は開いた document の内容や telemetry を upload しません。
  Network request は、GitHub release の確認・download など、明示的または設定済みの
  操作に限られます。

release workflow、secret、検証方法は
[Windows コード署名](docs/ja/PACKAGING.md#windows-コード署名)を参照してください。

## ライセンス

0BSD。ほぼすべての目的で、このプロジェクトを使用、コピー、変更、配布できます。
