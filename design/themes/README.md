# Ayame — テーマ（JSON 定義）

Ayame の見た目は **JSON で定義**します。ライト専用の iris 系プリセットを同梱し、
アプリ内の「テーマ」パネルで **編集 → 適用 → カスタム保存**（`localStorage`）→ **書き出し（JSON）** ができます。

プレビュー: [`../gui/iris-light.html`](../gui/iris-light.html) を開く。右下の「テーマ」パネル（サブメニュー）で
プリセット切替・**背景（水彩／単色）**・**イラスト量**・JSON 編集ができます。パネルは **✕ でしまえて**、
小さな「テーマ」タブから開き直せます。（`?theme=mono-paper` 等でプリセット指定、`?panel=0` で最初からしまった状態。）

## 同梱プリセット（すべて light）

| ファイル | 名前 | 雰囲気 |
|---|---|---|
| `iris-light.json` | Iris Light | 温白色の紙 × アイリス紫 × 金。定番。 |
| `iris-mist.json`  | Iris Mist  | 涼しい薄青〜アクア。白多めで軽やか。 |
| `iris-dawn.json`  | Iris Dawn  | 暁の蘭色×薔薇×金。あたたかい。 |
| `sumi-light.json` | Sumi Light | ほぼ白の墨。差し色はアイリス一点。極ミニマル。 |
| `mono-paper.json` | Mono Paper (単色) | 背景を単色にした**全単色配慮**プリセット。イラストなし。 |

## スキーマ

正式定義は [`theme.schema.json`](./theme.schema.json)。要点だけ:

```jsonc
{
  "name": "Iris Light",          // 表示名（カスタム保存のキーにもなる）
  "type": "light",               // 当面 light のみ
  "radius": 12,                  // パネル角丸(px)
  "font": { "mono": "…", "ui": "…" },   // 任意
  "color": {
    "paper": "#FBF8F1", "paper2": "#F1E9DA",   // 本文背景 / 環境グラデの基調
    "ink": "#2A2140", "inkDim": "#6E6383", "inkFaint": "#A99DBC",
    "accent": "#7A5CC0", "accent2": "#B98BD6", // アイリス（主/副）
    "gold": "#C79A2E", "stem": "#4E9C77",      // 花芯の金 / 茎の緑
    "edge": "rgba(90,70,140,0.16)",            // 罫線
    "ok": "#4E8C4A", "warn": "#B5771D", "err": "#C0506A",
    "markBg": "rgba(199,154,46,0.20)", "markFg": "#6B5510",
    "markCur": "#E8B84B", "markCurFg": "#2A2205"  // 検索ヒット / 現在ヒット
  },
  "acrylic": { "tint": "rgba(255,253,248,0.66)", "blur": 30, "noise": 0.05 },
  // ↑ Fluent 風アクリル：パネルの半透明フィル / ぼかし / 粒子
  "background": { "mode": "watercolor", "solid": "#F1E9DA" },
  // ↑ 背景（ユーザーが変更可・サブメニューの背後にも反映）。
  //    mode:"solid" で単色になり、全単色にしたいユーザーへ配慮できる。
  "illustration": 0.4,          // 左サイドの切り絵イラストの濃さ 0..1（0 で非表示）
  "watercolor": [               // 背景のアイリス・水彩の滲み（複数レイヤ）
    { "x": "12%", "y": "78%", "r": "46vh", "color": "rgba(122,92,192,0.30)" }
  ]
}
```

`color.*` は CSS カスタムプロパティ（`--paper`, `--accent` …）に、`acrylic.*` は
アクリルパネルの `background` / `backdrop-filter` に、`watercolor[]` は背景の
`radial-gradient` レイヤ列に、そのまま反映されます。左サイドの切り絵（版画）の形も
`accent / accent2 / gold / stem` で染まります。

## 実装メモ（本実装への落とし込み）

- 現行 `crates/ayame-cli/web/style.css` は `html[data-theme]` 方式。ここへ **JSON→CSS変数**の
  薄いローダを足すだけで移行できます（プリセットは `themes/*.json` をそのまま同梱）。
- カスタムは `localStorage["ayame.theme.customs.v1"]`。将来はユーザ設定ディレクトリの
  `~/.config/ayame/themes/*.json` を読む形へ拡張可能（配布物ゼロ依存のまま）。
- 追加ランタイムなし・画像なし。**軽さは据え置き**で、テーマ＝1 JSON ファイルとして共有・配布できます。
