<!-- i18n: language-switcher -->
[English](0001-diff-extraction-and-deprecation.en.md) | [日本語](0001-diff-extraction-and-deprecation.md)

# ADR 0001: diff 機能の ayame-diff への移管方針と非推奨化スケジュール

- ステータス: Accepted（2026-07-10）
- 関連 Issue: hjosugi/ayame-editor#93
- 切り出し Epic: hjosugi/ayame-editor#104
- 移管先ロードマップ: hjosugi/ayame-diff#26

## 背景

姉妹プロジェクト化に伴い、diff 関連機能を hjosugi/ayame-diff へ切り出す。
ayame-editor は **「巨大ファイルを開く・編集する」** に集中し、本格的な比較は
ayame-diff が担う（Sindre Sorhus 流の製品境界の明確化：一つの摩擦を深く解く）。

移管対象：

- `crates/ayame-cli/src/diff.rs`（`diff` / `sortdiff` サブコマンド）
- `serve/ops.rs` の `/api/diff` エンドポイント
- `web/src/search.ts` の 2 ファイル差分ビュー
- ネイティブメニュー / コマンドパレット / ショートカットの diff 項目
- `.grep-panel` に相乗りしている diff 用 CSS

## 決定

### 1. 段階的非推奨（2 段階）

受け皿のないまま機能を消さない。**ayame-diff v0.4.0（受け皿）リリースが先。**

| リリース | 内容 |
| --- | --- |
| **v0.6.0（非推奨化）** | ayame-diff v0.4.0 リリース後。`ayame diff` / `ayame sortdiff` 実行時と Web の diff ダイアログに **非推奨警告 + ayame-diff への誘導** を表示。コード（`diff.rs` 等）は残置し、挙動は変えない。 |
| **v0.7.0（削除）** | 実装・API（`/api/diff`）・Web UI・ネイティブメニュー項目・docs・テストを削除。 |

一括削除ではなく 2 段階にする理由：既存ユーザーに移行期間と明確な乗り換え先を
与えるため。破壊的変更は 1 リリース前に必ず予告される状態を保つ。

### 2. Web UI の diff 導線

v0.7.0 では **単純削除**を基本とする。ayame-diff がインストール済みなら外部
起動する連携は、需要と実装コストを見て **別 Issue（#102 連携）** で判断する
（v0.7.0 のブロッカーにはしない）。非推奨期間（v0.6.0）は誘導バナー（#97）で
ayame-diff へ案内する。

### 3. 移管中の二重メンテ回避（凍結）

切り出し完了まで **editor 側 diff への機能追加は凍結**する。バグ修正は妨げない
が、新機能・アルゴリズム改善は ayame-diff 側（#5〜#8）で行い、editor へは
バックポートしない。二重実装のドリフトを防ぐ。

### 4. 破壊的変更の告知文面

CHANGELOG / リリースノートに以下を明記する：

- v0.6.0: 「`diff` / `sortdiff` は非推奨。後継は ayame-diff（リンク）。
  次リリース v0.7.0 で削除予定。」
- v0.7.0: 「`diff` / `sortdiff`、`/api/diff`、Web 差分ビューを削除。
  比較機能は ayame-diff（リンク）へ移行してください。」

## 依存順序

```
ayame-diff #5〜#8（移植・受け皿実装）
        │
ayame-diff v0.4.0 リリース（#24）   ← ここが先
        │
ayame-editor v0.6.0 非推奨化（#94 #97 #99 #100 #102）
        │
ayame-editor v0.7.0 削除（#94 #95 #96 #97 #98 #99 #101）
        │
ayame-editor #103 リリース
```

## 完了条件（本 ADR で満たすもの）

スケジュール（2 段階）と Web 導線方針（単純削除 + 非推奨期間の誘導）が確定し、
凍結方針と告知文面が定まった。切り出し実装 Issue（#94〜#102）が着手可能。
