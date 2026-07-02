# Ayame

巨大なテキストでも、待たされず・落ちずに開けるデスクトップ・エディタ。

数百MBのログでも、数十GBのCSVでも、ドロップした瞬間に開いてスクロールできます。
文字コード（UTF-8 / Shift_JIS / EUC-JP）は自動で判別。macOS・Windows・Linux で動きます。

## インストール

[最新リリース](https://github.com/hjosugi/ayame-editor/releases/latest)から、お使いの OS 向けをダウンロードしてダブルクリックで起動します。

- **macOS** — `Ayame.app`（初回のみ右クリック →「開く」）
- **Windows** — `ayame-*.exe`
- **Linux** — 実行ファイル（`WebKitGTK` が必要。例: Ubuntu の `libwebkit2gtk-4.1-0`）

ターミナル派の方はこれでも入ります:

```sh
curl -fsSL https://raw.githubusercontent.com/hjosugi/ayame-editor/main/scripts/install.sh | sh
```

インストール後は、引数なしなら空の新規テキスト、ファイルを渡せばそのファイルをネイティブウィンドウで開きます:

```sh
ayame
ayame ./huge.log
```

## できること

- **とにかく速い** — 100 億行クラスでも、必要な部分だけを読むので一瞬で開いてスクロールできます。落ちません。
- **タブ** — 複数のファイルを開いて切り替え。`Ctrl+N` で新規、`Ctrl+W` で閉じる。
- **エクスプローラー** — ツールバー左端のボタン（`Ctrl+B`）で開閉。フォルダからファイルをたどれます。
- **検索** — `Ctrl+F` でエディタ右上に検索バーが浮かびます。大文字小文字・単語単位・正規表現に対応。`F3` / `Shift+F3` で次・前へ。
- **ソート** — ツール ▾ の「ソート」。行全体または指定したキー列で、昇順・降順に並び替えて結果を新しいタブに開きます（元ファイルは変更しません）。
- **2ファイル差分** — ツール ▾ の「2ファイル差分」で、現在のファイルと別ファイルを左右比較。変更行は単語単位でも確認できます。
- **一括変換** — ツール ▾ から置換・ケース変換を別ファイルへ保存。行境界chunkに分けた並列処理なので巨大ファイルでも有界メモリです。
- **メモ帳のように編集** — クリックした位置に入力、`Enter` で改行、複数行の貼り付けもそのまま。
- **範囲選択** — ドラッグや `Shift`＋クリックで複数行を選択。`Alt`＋ドラッグで矩形選択。コピー / カット / 削除 / 置換できます。
- **テーマ** — 明るく静かな **Iris Light**（既定）ほか、Iris Mist / Dawn、Sumi Light、単色の Mono Paper、ダーク／ブラック。背景は水彩か単色を選べ、テーマは JSON で書き出し・自作もできます（⚙ 設定）。

新規テキストの既定名は `AYAME_UNTITLED_NAME` で変えられます。`{date}` / `{time}` / `{datetime}` / `{pid}` が使えます。

```sh
AYAME_UNTITLED_NAME='memo-{date}-{time}.txt' ayame
```

### キーボード

| 操作 | キー |
|---|---|
| ファイルを開く | `Ctrl+O` |
| 新しいタブ / 閉じる | `Ctrl+N` / `Ctrl+W` |
| エクスプローラー | `Ctrl+B` |
| 検索 / 次・前の一致 | `Ctrl+F` / `F3`・`Shift+F3` |
| 行へ移動 | `Ctrl+G` |
| コピー / カット / 全選択 | `Ctrl+C` / `Ctrl+X` / `Ctrl+A` |
| 元に戻す / やり直す | `Ctrl+Z` / `Ctrl+Y` |
| 上書き保存 / 別名で保存 | `Ctrl+S` / `Ctrl+Shift+S` |

## ソースからビルド

```sh
cargo build --release --features gui
./target/release/ayame
```

Windows / macOS / Linux それぞれの開発手順は [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) にまとめています。

## ライセンス

MIT
