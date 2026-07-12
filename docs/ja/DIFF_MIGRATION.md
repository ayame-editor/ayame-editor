# diff 移行ガイド

*English: [../DIFF_MIGRATION.md](../DIFF_MIGRATION.md)*

ファイル比較は Ayame Editor から姉妹プロジェクト
[ayame-diff](https://github.com/hjosugi/ayame-diff) へ移管されました。Ayame Editor は
巨大ファイルを開く・編集する・検索する・変換する機能に集中します。

## コマンド対応表

| 削除された Ayame Editor 機能 | ayame-diff での置き換え |
| --- | --- |
| `ayame diff OLD NEW` | `ayame-diff text OLD NEW` |
| `ayame sortdiff OLD NEW` | `ayame-diff sorted OLD NEW` |
| `ayame sort-diff OLD NEW` | `ayame-diff sorted OLD NEW` |
| ツール → 2 ファイル差分 | `ayame-diff gui` または `ayame-diff serve` |

[ayame-diff の最新リリース](https://github.com/hjosugi/ayame-diff/releases/latest) から
インストールするか、Go 1.23 以降がある場合は次を実行してください。

```sh
go install github.com/hjosugi/ayame-diff/cmd/ayame-diff@latest
```

旧 CLI コマンド名は 1 リリースの間、移行案内だけを返します。比較実装、HTTP
endpoint、Web dialog、native menu、editor 側の比較テストは配布物から削除されます。
