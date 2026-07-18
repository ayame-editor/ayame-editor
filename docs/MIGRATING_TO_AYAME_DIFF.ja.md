# 比較ワークフローを ayame-diff へ移行する

Ayame Editor v0.7.0 では比較実装、`/api/diff`、2 ファイル比較 UI を削除しました。
比較機能は姉妹プロジェクト [ayame-diff](https://github.com/hjosugi/ayame-diff)
で継続しています。巨大ファイル向けの同等ワークフローに加え、専用 GUI と多様な
比較モードを提供します。

## インストール

[ayame-diff Releases](https://github.com/hjosugi/ayame-diff/releases/latest) から
各 OS 用ビルドを取得するか、Go 1.23 以降でインストールします。

```sh
go install github.com/hjosugi/ayame-diff/cmd/ayame-diff@latest
```

## コマンド対応表

| Ayame Editor v0.7.0 より前 | ayame-diff |
| --- | --- |
| `ayame diff OLD NEW` | `ayame-diff text OLD NEW` |
| `ayame diff OLD NEW --side-by-side` | `ayame-diff text --side-by-side OLD NEW` |
| `ayame diff OLD NEW --json` | `ayame-diff text --json OLD NEW` |
| `ayame diff OLD NEW --summary` | `ayame-diff text --summary OLD NEW` |
| `ayame sortdiff OLD NEW` | `ayame-diff sorted OLD NEW` |
| Web / ネイティブの「ツール → 2 ファイル差分」 | `ayame-diff --gui OLD NEW` |

Ayame Editor v0.7.0 では旧 CLI 名に移行エラーだけを残し、スクリプトが曖昧に失敗
しないようにしています。この互換期間中にスクリプトを更新してください。旧名は
将来のリリースで完全に削除される可能性があります。

現在のフラグと比較モードは
[ayame-diff ドキュメント](https://hjosugi.github.io/ayame-diff/ja/)を参照してください。
