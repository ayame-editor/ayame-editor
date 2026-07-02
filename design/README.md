# Ayame — デザイン案ギャラリー

現行UIの外観リデザイン。狙いは **美しく・軽量・モダン・ハッカブル、そしてエンジニアがワクワクする**こと。
同じ画面（同じツールバー / 同じ 1.36 GiB TSV データ / 同じ検索状態）を描き分けた、純粋な見た目のA/B比較です。

> ## ★ 最新の方向（light only）— Iris Light + JSON テーマ
> フィードバックを受けて **ライト専用**に集約：iris の静けさ × **Fluent 風のわずかな透明感（アクリル）** ×
> **アシンメトリ**な構図。花のイラストは置かず、左サイドは **版画（切り絵／ポショワール）のエッセンス** —
> Matisse『Jazz』のようなフラットな型抜きの形 — を、アイリス紫に少しの緑・金で控えめに。既定テーマは **Sumi Light**。
> テーマは **JSON で定義**でき、アプリ内で編集 → 適用 → **カスタム保存（localStorage）**→ 書き出しまで可能。
> - デザイン＋テーマ編集: [`gui/iris-light.html`](./gui/iris-light.html)（`?theme=iris-mist` / `?panel=0` 対応）
> - テーマ定義とスキーマ: [`themes/`](./themes/)（`iris-light` / `iris-mist` / `iris-dawn` / `sumi-light` / `mono-paper`）
> - サブメニューは **しまえる**／背景は **水彩・単色** を切替（**全単色配慮**の Mono Paper 同梱）／イラスト量も調整可
> - レンダー: `renders/iris-*.png`
>
> 以下の 18 案は、この方向に至るまでの配色・レイアウト探索の記録です。

## 見かた

すばやく比較するなら、PNG中心の静的カタログ [`design-list.html`](./design-list.html) と
一覧表 [`DESIGN_LIST.md`](./DESIGN_LIST.md) を使います。

`design/index.html` をブラウザで開くと、全案のライブサムネイル一覧が出ます（クリックで原寸）。

```sh
# どれでもOK。ビルドもサーバも不要（全案が単一の自己完結HTML）
xdg-open design/index.html        # Linux
open design/index.html            # macOS
# もしくは既存の ayame serve でも配信できます
```

個別に見るなら `design/mockups/NN-*.html` を直接開くだけ。**JavaScript もビルドも外部アセットも不要**、オフラインで開きます。

## 18案（カテゴリ別）

### 和 — Japanese
| # | 名前 | 一言 |
|---|------|------|
| 01 | **Ayame Zen** | 濃紺夜 × アイリス色 × 金箔。和紙テクスチャに文字が浮かぶ本命。 |
| 02 | **Kanagawa 神奈川** | 北斎の富嶽。藍と菫に青海波を忍ばせた、落ち着いた浮世絵配色。 |
| 03 | **Sumi-e 墨絵** | 墨一色。差し色は朱の落款だけ。余白と静けさで見せる究極のミニマル。 |

### Neon & Hacker
| # | 名前 | 一言 |
|---|------|------|
| 04 | **Neon Glass Terminal** | すりガラスのクロムがネオンの街灯りに浮かぶ。行番号がマゼンタ→シアン、ヒットが発光。 |
| 05 | **Synthwave '84** | アウトランの夕焼け。地平線のパースグリッドと発光する文字。 |
| 06 | **Matrix Phosphor** | 漆黒に燐光グリーン。デジタルレイン・走査線・点滅カーソル。ハッカー直球。 |

### Dev classics（開発者に愛される定番）
| # | 名前 | 一言 |
|---|------|------|
| 07 | **Tokyo Night** | 最も愛される夜。青と紫の端正な配色。 |
| 08 | **Catppuccin Mocha** | パステルの温もり。丸みと柔らかさ。 |
| 09 | **Rosé Pine Moon** | iris・rose・gold の上品な夜。 |

### Calm · Light · Retro
| # | 名前 | 一言 |
|---|------|------|
| 10 | **Nord Frost** | 北極の薄明。冷たいスレートに氷色シアン一点。 |
| 11 | **Gruvbox** | 温かいレトロ。琥珀と橙、うっすらフィルムグレイン。 |
| 12 | **Alabaster Light** | 温白色の紙。**明るくても美しい**上質ライトテーマ。 |
| 15 | **Amber CRT** | VT220の郷愁。琥珀の燐光・走査線・湾曲ガラスのビネット。 |

### Modern & Experimental
| # | 名前 | 一言 |
|---|------|------|
| 13 | **Frostbite Wireframe** | Linear級のミニマル。極細グリッド・氷白文字・銀の微光。 |
| 14 | **Aurora Silk Glass** | すりガラスの奥でオーロラがゆっくり呼吸する。Stripe級の質感。 |

### Layout studies — 構造から変える（色替えではない）
| # | 名前 | 一言 |
|---|------|------|
| 16 | **Command Palette (⌘K)** | クロムを細く。全機能を中央のフローティングパレットへ集約（Zed / Raycast）。 |
| 17 | **Power Split** | 検索ヒットを別ペインに保持してジャンプ。川のミニマップ（klogg 方式）。巨大ファイルの本命導線。 |

### Reference
| # | 名前 | 一言 |
|---|------|------|
| 00 | **Graphite (base)** | 全案の土台となる中立クローム（`_base.html`）。 |

## 仕組み — なぜ軽くてハッカブルか

全案は `_base.html` の**同一DOM**を共有し、変えるのは基本 **`:root` のCSSカスタムプロパティ（パレット）** と、少量の *signature effect*（グロウ・オーロラ・走査線など）だけ。つまり:

- **テーマ = ただのCSS変数セット**。実装に落とすときは、現行 `style.css` の `html[data-theme="…"]` を増やすだけで足ります（現行は light/dark/black の3つ）。
- ランタイムは一切増えない（画像なし・JSなし・依存なし）。**軽さは据え置きで見た目だけ跳ね上がる**。
- ユーザが自分のテーマを1ファイルで追加できる → そのまま「ハッカブル」。

### 使っている“驚き”の技（CSSのみ）
- 発光する検索ヒット / カレント行の左に光るアクセントバー（`box-shadow` bloom）
- すりガラスのクロム（`backdrop-filter: blur`）— 04 / 14
- 背景で呼吸するオーロラ、ネオンの街灯り、アウトランの地平線（`radial/linear-gradient` + `@keyframes`）
- 走査線・燐光・点滅カーソルのCRT表現 — 06 / 15
- 行番号ガターのグラデ文字（`background-clip: text`）— 04
- 右端の「川」ミニマップ：検索ヒットが光る点として流れる — 全案
- ステータスバーの Vim風モード灯（NORMAL / INSERT / SEARCH）

## おすすめの進め方（私見）

1. **既定は 01 Ayame Zen**。ブランド名と一致し、紫×金でどのエディタとも被らない“Ayameらしさ”が立つ。
2. **同梱プリセット**に 07 Tokyo Night / 08 Catppuccin / 09 Rosé Pine / 13 Frostbite / 12 Alabaster Light。エンジニアは「自分の色がある」だけで嬉しい。
3. **導線の本命は 17 Power Split と 16 ⌘K**。色より“気持ちよさ”はここで出る。巨大ファイル＝Ayameの勝ち筋を、検索結果の別ペイン＋川ミニマップで見せる。
4. 遊び枠として 06 Matrix / 05 Synthwave / 15 Amber CRT を隠しテーマに。

## 補足 — フロントエンドのモダン化（任意）

見た目を刷新するなら、`crates/ayame-cli/web` の素の JS も同時にモダン化できます（“エンジンは Rust、UIは最新ツールチェーン”という座組は、それ自体がワクワク要素）。いただいた最新スタック整理をそのまま採れます:

| 層 | 提案 | 効果 |
|----|------|------|
| 型 | **tsgo（`@typescript/native-preview` / TypeScript 7 系のGo実装）** | `tsc` 互換のまま桁違いに速い型チェック |
| Lint | **oxlint 1.7x** | `--type-aware`（tsgolint統合）で高速かつ型認識のLint |
| Format | **oxfmt 0.5x** | Prettier適合をほぼ100%維持しつつ大幅高速化 |
| 統合 | **Vite+ / VoidZero（oxc 系）** | 個別ツールを一本のチェーンに束ねる選択肢 |

※これらは**任意の別提案**で、上のデザイン案自体は依存ゼロのまま成立します（現状の軽さを一切損なわない）。採用するかは別途決めましょう。

## 新しい案の足しかた

```
1. _base.html をコピーして design/mockups/NN-yourtheme.html を作る
2. <title> と .theme-tag を差し替える
3. :root { … } のパレット値を変える（必要なら signature effect を <style> 末尾に追記）
4. index.html の DESIGNS 配列に1行足す
```

DOM・データ・レイアウトは共有のまま。**色と少しの光**だけで、驚きは十分に作れます。
