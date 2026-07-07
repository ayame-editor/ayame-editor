// Ayame Editor — i18n module. Type-stripped to JS at build time (build.rs, oxc).
import { state } from "./state.js";

// clipboard cap: copy warns, cut refuses beyond this

// ---- i18n -------------------------------------------------------------------
// Dot-namespaced keys; every locale is a full translation table. English (`en`)
// is the reference/fallback language and MUST stay complete — t() falls back
// locale → en → key. Interpolated strings carry {var} placeholders substituted
// by t(key, vars). Static HTML is tagged with data-i18n attributes and
// re-applied per locale by applyStaticI18n().
//
// Adding a language `xx` is DATA-ONLY: add one `xx: { … }` block below — copy
// `en` and translate every key, and include this block's own "language.name"
// (its self-name for the picker), "language.auto", and a `weekday` table
// (short/long arrays, indexed by Date.getDay()). normalizeLanguage(), the
// Settings language picker (populateLanguageSelect), and browserLocale() all
// derive from Object.keys(MESSAGES), so a new language is picked up with no code
// change. (Server-origin errors are a separate concern: they are translated only
// at the en boundary in serverMessage()/SERVER_MSG_EN; N-language coverage waits
// on server-side error codes.)
export const MESSAGES = {
  ja: {
    // -- menu bar and menu items --
    "menu.bar": "メニュー",
    "menu.file": "ファイル",
    "menu.edit": "編集",
    "menu.selection": "選択",
    "menu.view": "表示",
    "menu.tools": "ツール",
    "menu.newFile": "新規ファイル",
    "menu.newWindow": "新規ウィンドウ",
    "menu.open": "開く",
    "menu.save": "保存",
    "menu.saveAs": "名前を付けて保存",
    "menu.encoding": "文字コード / 改行コード…",
    "menu.undo": "元に戻す",
    "menu.redo": "やり直す",
    "menu.find": "検索",
    "menu.replace": "置換",
    "menu.gotoLine": "行へ移動",
    "menu.duplicateLine": "行を複製",
    "menu.moveLineUp": "行を上へ移動",
    "menu.moveLineDown": "行を下へ移動",
    "menu.deleteLine": "行を削除",
    "menu.selectAll": "すべて選択",
    "menu.selectNextOccurrence": "次の一致を選択",
    "menu.copy": "コピー",
    "menu.cut": "切り取り",
    "menu.caseUpper": "大文字に変換",
    "menu.caseLower": "小文字に変換",
    "menu.caseCamel": "camelCase に変換",
    "menu.casePascal": "PascalCase に変換",
    "menu.caseSnake": "snake_case に変換",
    "menu.caseKebab": "kebab-case に変換",
    "menu.caseConstant": "CONSTANT_CASE に変換",
    "menu.addCursorAbove": "カーソルを上に追加",
    "menu.addCursorBelow": "カーソルを下に追加",
    "menu.explorer": "エクスプローラー",
    "menu.findBar": "検索バー",
    "menu.commandPalette": "コマンドパレット",
    "menu.showWhitespace": "空白・改行を表示",
    "menu.syntaxHighlight": "シンタックスハイライト",
    "menu.zenkakuUnderline": "全角空白を下線で表示",
    "menu.wordWrap": "折り返し",
    "menu.followTail": "末尾に追従 (tail -f)",
    "menu.settings": "設定",
    "menu.sort": "ソート",
    "menu.sortTitle":
      "行単位で並び替えて現在のファイルを上書きします。既定は行全体の文字列比較、キー列を指定するとその列で比較 (昇順/降順)",
    "menu.diff": "2ファイル差分",
    "menu.split": "ファイルを分割",
    "menu.splitTitle": "行数を指定して複数ファイルに分割します (例: 100万行ずつ)",
    "menu.grep": "フォルダ内検索",
    "menu.grepTitle":
      "フォルダ内のファイルを再帰的に検索します (ファイル名フィルタ・正規表現に対応)",
    "menu.grepSave": "grep して保存",
    "menu.grepSaveTitle":
      "現在のファイルから一致した行だけを別ファイルに書き出します (正規表現・大小文字・単語単位に対応)",
    // -- toolbar --
    "toolbar.applyTheme": "テーマ適用",
    "toolbar.applyThemeTitle": "このJSONをテーマとして適用",
    "toolbar.applyKeymap": "キー設定適用",
    "toolbar.applyKeymapTitle": "このJSONをキー設定として適用",
    "toolbar.toolsTitle": "ソート・差分・分割",
    "toolbar.newTab": "新規タブ",
    // -- tabs --
    "tab.close": "タブを閉じる",
    "tab.closeName": "{name} を閉じる",
    "tab.confirmDiscard": "{name} の未保存の編集を破棄して閉じますか?",
    "tab.discardClose": "破棄して閉じる",
    "tab.switchError": "タブ切替エラー",
    "tab.closeError": "タブを閉じられません",
    "tab.moveDirty": "未保存のタブは保存してから移動してください",
    "tab.handoffDone": "未保存の編集を引き継ぎました",
    "tab.handoffError": "タブの引き継ぎに失敗しました (タブは元のウィンドウに残っています)",
    // -- explorer sidebar --
    "tree.close": "エクスプローラーを閉じる",
    "tree.actions": "エクスプローラー操作",
    "tree.up": "上の階層へ",
    "tree.back": "戻る",
    "tree.forward": "進む",
    // -- find / replace bar --
    "find.group": "検索と置換",
    "find.showReplace": "置換を表示",
    "find.matchCase": "大文字小文字を区別",
    "find.wholeWord": "単語単位",
    "find.regex": "正規表現",
    "find.prev": "前の一致",
    "find.next": "次の一致",
    "find.close": "検索を閉じる",
    "find.closeTitle": "検索を閉じる (Esc)",
    "find.replaceWith": "置換後",
    "find.replaceOneTitle": "現在の一致を置換して次へ",
    "find.replaceAll": "すべて置換",
    "find.replaceAllTitle": "すべての一致を置換 (元に戻せます)",
    "find.matchCount": "{total} 件",
    "find.noMatch": "一致なし",
    "find.wrapTop": "先頭に戻りました",
    "find.wrapBottom": "末尾に戻りました",
    "find.regexError": "正規表現エラー",
    "find.searchError": "検索エラー",
    "find.noNextOccurrence": "次の一致はありません",
    "find.noWordToSelect": "選択できる単語がありません",
    "find.rectNoCtrlD": "矩形選択では Ctrl+D は使えません",
    "find.multiLineNoCtrlD": "複数行選択では Ctrl+D は使えません",
    "find.enterQuery": "検索文字列を入力してください",
    "find.cannotIdentifyMatch": "一致を特定できません",
    "find.replaceError": "置換エラー",
    "find.replacing": "置換中…",
    "find.replacedCount": "{n} 件置換しました",
    "find.replacedCountPartial":
      "{n} 件置換しました — 一致が多いため一部です。もう一度実行してください",
    // -- status bar --
    "status.saving": "保存中…",
    "status.follow": "追従",
    "status.saved": "保存済",
    "status.unsaved": "未保存",
    "status.indexOk": "索引OK",
    "status.allSaved": "すべての編集は保存済みです",
    "status.line0": "行 0",
    "status.pos": "行 {line}, 列 {col}",
    "status.posCursors": "{pos} · {n} カーソル",
    "status.unsavedDetail": "未保存の編集: +{added} 行追加 / ~{changed} 行変更 / -{deleted} 行削除",
    "status.indexDetail":
      "{lines} 行 / {bytes} / 索引 {checkpoints} 点 ({indexBytes}, {indexMs} ms)",
    "status.encTitle": "文字コードを変換して保存",
    "status.zoomTitle": "表示倍率（クリックで100%に戻す）",
    "status.eolTitle": "改行コードを変換して保存",
    "status.followingTail": "末尾に追従中 (tail -f)",
    "status.followStopped": "追従を停止しました",
    "status.tailFileChanged": "ファイルが外部で変更されました — 追従を停止しました",
    // -- editor (clipboard, edits, caps) --
    "editor.label": "エディタ",
    "editor.copied": "コピーしました",
    "editor.copyError": "コピーエラー",
    "editor.pasteBlocked": "ここからは貼り付けできません — Ctrl+V を使ってください",
    "editor.noSelection": "選択がありません",
    "editor.multiSelUseCopy": "複数選択はコピーまたは切り取りを使ってください",
    "editor.copyCapped": "コピーは先頭 {max} 行まで — 残り {rest} 行はコピーされていません",
    "editor.copyCappedHint":
      "コピーは先頭 {max} 行まで — 残り {rest} 行はコピーされていません。全体は右クリック→「選択箇所をファイルに保存」で書き出せます",
    "editor.cutCapped": "切り取りは {max} 行まで (選択は {total} 行)。削除だけなら Delete キー",
    "editor.cutCappedHint":
      "切り取りは {max} 行まで (選択は {total} 行)。全体を残すなら右クリック→「選択箇所をファイルに保存」、削除だけなら Delete キー",
    "editor.selectRangeFirst": "変換する範囲を選択してください",
    "editor.transformCapped": "変換は一度に {max} 行までです",
    "editor.duplicateCapped": "複製は一度に {max} 行までです",
    "editor.moveCapped": "行の移動は一度に {max} 行までです",
    "editor.editError": "編集エラー",
    "editor.reloadError": "再読込エラー",
    "editor.savingWaitInput": "保存中です — 完了後に入力します",
    "editor.savingWait": "保存中です — 完了までお待ちください",
    // -- editor context menu --
    "ctx.menu": "コンテキストメニュー",
    "ctx.paste": "貼り付け",
    "ctx.saveSelection": "選択箇所をファイルに保存…",
    "ctx.saveSelectionTitle":
      "選択した行だけを別ファイルへ書き出します。コピーの行数上限はありません",
    // -- shared dialog chrome --
    "common.ok": "OK",
    "common.cancel": "キャンセル",
    "common.close": "閉じる",
    "common.closeEsc": "閉じる (Esc)",
    "common.run": "実行",
    "common.confirm": "確認",
    "common.input": "入力",
    "common.options": "オプション",
    "common.error": "エラー",
    // -- open / save dialog --
    "dialog.open.title": "ファイルを開く",
    "dialog.open.path": "パス",
    "dialog.open.fileName": "ファイル名",
    "dialog.open.pathPlaceholder": "ファイルのパスを入力… (例: /var/log/huge.log)",
    "dialog.open.namePlaceholder": "保存するファイル名、またはフルパス",
    "dialog.open.folder": "フォルダ",
    "dialog.open.location": "場所",
    "dialog.open.folderToTree": "表示中のフォルダをツリーに開く",
    "dialog.open.folderToExplorer": "表示中のフォルダをエクスプローラーに表示",
    "dialog.open.hintOpen":
      "ここへファイルをドラッグ＆ドロップしても開けます。大きなファイルはパス指定の方が高速です。",
    "dialog.open.hintSave":
      "フォルダを選び、保存するファイル名を入力します。既存ファイルを選ぶと上書き確認します。",
    "dialog.open.recent": "最近使ったファイル",
    "dialog.open.loading": "読み込み中…",
    "dialog.open.loadingName": "読み込み中… ({name})",
    "dialog.open.loadingFile": "読み込み中… {name}",
    "dialog.open.opening": "開いています…",
    "dialog.open.openingName": "開いています: {name} …",
    "dialog.open.dirError": "ディレクトリを開けません: {msg}",
    "dialog.open.enterFileName": "保存するファイル名を入力してください",
    "dialog.open.pickFolderFirst": "保存先のドライブ・フォルダを選択してください",
    "dialog.open.folderShown": "現在のフォルダをエクスプローラーに表示しました",
    // -- settings --
    "settings.theme": "テーマ",
    "settings.themeMonoPaper": "Mono Paper (単色)",
    "settings.themeDark": "ダーク",
    "settings.themeBlack": "ブラック",
    "settings.background": "背景",
    "settings.bgDefault": "デフォルト",
    "settings.bgSolid": "単色",
    "settings.bgImage": "カスタム画像",
    "settings.bgImagePick": "画像を選択…",
    "settings.bgImageTooLarge": "画像が大きすぎます（最大4MB）",
    "settings.bgImageError": "画像を読み込めませんでした",
    "settings.bgImagePersistError": "画像を保存できませんでした — 今回のみ適用されます",
    "settings.language": "言語",
    // Self-name of this language (shown in the language picker) + the "auto"
    // option label. Every MESSAGES block must define "language.name".
    "language.name": "日本語",
    "language.auto": "自動",
    // Weekday names for the 新規ファイル名 template, indexed by Date.getDay()
    // (0 = Sunday). Part of this language block; en is the fallback.
    weekday: {
      short: ["日", "月", "火", "水", "木", "金", "土"],
      long: ["日曜日", "月曜日", "火曜日", "水曜日", "木曜日", "金曜日", "土曜日"],
    },
    "settings.illustration": "イラスト",
    "settings.font": "フォント",
    "settings.fontMono": "等幅 (Consolas / Menlo)",
    "settings.fontMonoJp": "等幅 + 日本語 (Noto/MS Gothic)",
    "settings.fontSystem": "システムUI",
    "settings.fontSize": "文字サイズ",
    "settings.ruler": "列ルーラー",
    "settings.lineNumberCommas": "行番号をカンマ区切りで表示",
    "settings.restoreSession": "セッション復元",
    "settings.confirmExit": "終了確認",
    "settings.memoName": "新規ファイルの名前",
    "settings.memoNameHint":
      "使える変数: {yyyy} {yy} {mm} {dd} {HH} {MM} {ss} {ddd} {dddd}(曜日) {seq}(連番) {date} {time} {datetime}",
    "settings.sidebar": "サイドバー",
    "settings.sidebarSide": "サイドバー位置",
    "settings.left": "左",
    "settings.right": "右",
    "settings.themeJson": "テーマJSON",
    "settings.editInTab": "タブで編集",
    // -- encoding / line-ending dialog --
    "dialog.convert.title": "文字コード / 改行コード",
    "dialog.convert.encoding": "文字コード",
    "dialog.convert.eol": "改行コード",
    "dialog.convert.eolCr": "CR (旧 Mac)",
    "dialog.convert.bom": "BOMを付ける（UTF-8 / UTF-16）",
    "dialog.convert.reopen": "開き直す",
    "dialog.convert.go": "変換して保存",
    "dialog.convert.savedAs": "{enc} / {eol} で保存しました",
    "dialog.convert.reopenedAs": "{enc} で開き直しました",
    "dialog.convert.saveError": "変換保存エラー",
    "dialog.convert.saveFirst": "先に保存してください",
    "dialog.convert.noSavedFile": "保存されたファイルがありません",
    "dialog.convert.discardAsk": "未保存の編集を破棄して開き直しますか?",
    "dialog.convert.discardOk": "破棄して開き直す",
    "dialog.convert.reopenError": "開き直しエラー",
    // -- key bindings --
    "keymap.title": "キー設定",
    "keymap.hint": "入力欄を選んでキーを押すと変更できます。Backspace / Delete で未設定。",
    "keymap.editJson": "JSONをタブで編集",
    "keymap.reset": "既定に戻す",
    "keymap.resetConfirm": "すべてのキー設定を既定に戻しますか？",
    "keymap.unassigned": "未設定",
    "keymap.conflictKey": "文字入力と衝突するキーは使えません",
    "keymap.searchCase": "検索: 大文字小文字",
    "keymap.searchWord": "検索: 単語単位",
    "keymap.searchRegex": "検索: 正規表現",
    "keymap.toggleSidebar": "エクスプローラー表示",
    "keymap.cannotOpen": "キー設定を開けません",
    "keymap.jsonError": "キー設定 JSON エラー",
    // -- sort dialog --
    "dialog.sort.keyColumn": "キー列 (1始まり)",
    "dialog.sort.keyPlaceholder": "空なら行全体で比較",
    "dialog.sort.keyTitle":
      "空欄: 行全体を文字列として比較 / 数字: 区切り文字で分けたその列をキーとして比較",
    "dialog.sort.delimiter": "区切り文字",
    "dialog.sort.delimiterTitle": "キー列を使うときの列の区切り (例: , やタブ)",
    "dialog.sort.numeric": "数値として比較する",
    "dialog.sort.numericTitle": "10 と 9 を文字列でなく数値の大小で並べます",
    "dialog.sort.order": "並び順",
    "dialog.sort.asc": "昇順 (A→Z, 小→大)",
    "dialog.sort.desc": "降順 (Z→A, 大→小)",
    "dialog.sort.hint":
      "現在のファイルを並び替えて上書きします。未保存の編集も含めて並び替えます。この操作は元に戻せません。",
    "dialog.sort.keyInvalid": "キー列は 1 以上の整数で指定してください",
    "dialog.sort.running": "ソート実行中…",
    "dialog.sort.done": "ソートして上書きしました",
    "dialog.sort.error": "ソートエラー",
    // -- split dialog --
    "dialog.split.linesPer": "1ファイルあたりの行数",
    "dialog.split.outDir": "出力先フォルダ",
    "dialog.split.outDirPlaceholder": "空なら元ファイルと同じ場所",
    "dialog.split.hint":
      "現在のファイルを指定行数ごとに分割して書き出します。未保存の編集も含まれます。元のファイルは変更されません。",
    "dialog.split.go": "分割",
    "dialog.split.linesInvalid": "行数は 1 以上の整数で指定してください",
    "dialog.split.running": "分割実行中…",
    "dialog.split.done": "{count} 個に分割しました: 最初のファイル {path}",
    "dialog.split.error": "分割エラー",
    // -- diff view --
    "dialog.diff.title": "差分",
    "dialog.diff.current": "現在",
    "dialog.diff.compareTo": "比較先",
    "dialog.diff.currentFile": "現在のファイル",
    "dialog.diff.compareFile": "比較先ファイル",
    "dialog.diff.added": "追加",
    "dialog.diff.deleted": "削除",
    "dialog.diff.changed": "変更",
    "dialog.diff.none": "差分はありません",
    "dialog.diff.promptPath": "比較先ファイルパス",
    "dialog.diff.computing": "差分を計算中…",
    "dialog.diff.error": "差分エラー",
    "dialog.diff.hunks": "差分: {n} hunk",
    "dialog.diff.hunkHeader":
      "{kind}  現在: {oldStart} ({oldLen} 行)  比較先: {newStart} ({newLen} 行)",
    "dialog.diff.unsavedIncluded": "未保存編集込み",
    "dialog.diff.hunkTruncated": "このhunkは先頭 {n} 行だけ表示しています",
    "dialog.diff.summary": "{hunks} hunk / +{added}  -{deleted}  ~{modified}",
    "dialog.diff.omitted": "{n} hunk 省略",
    // -- folder search (grep) --
    "dialog.grep.query": "検索語",
    "dialog.grep.queryPlaceholder": "検索する文字列 / 正規表現",
    "dialog.grep.dir": "対象フォルダ",
    "dialog.grep.dirPlaceholder": "空欄で開いているファイルのフォルダ",
    "dialog.grep.glob": "ファイル名フィルタ",
    "dialog.grep.globPlaceholder": "例: *.rs, *.txt (空欄で全て)",
    "dialog.grep.ignoreCase": "大文字小文字を区別しない",
    "dialog.grep.searching": "フォルダ内を検索中…",
    "dialog.grep.error": "フォルダ内検索エラー",
    "dialog.grep.noMatches": "一致はありません",
    "dialog.grep.flash": "フォルダ内検索: {n} 件",
    "dialog.grep.summary": "{hits} 件 / {files} ファイル",
    "dialog.grep.summaryTruncated": "（上限 {max} 件で打ち切り）",
    "dialog.grep.summaryFiles": " / 走査ファイル数の上限に達しました",
    // -- grep to file --
    "dialog.grepSave.hint":
      "一致した行だけを新しいファイルへ書き出します (未保存の編集も反映)。数 GB のファイルもストリーミングで完走します。",
    "dialog.grepSave.go": "保存先を選択",
    "dialog.grepSave.running": "一致行を書き出し中…",
    "dialog.grepSave.error": "grep して保存に失敗しました",
    // -- save-selection dialog --
    "dialog.saveSel.title": "選択箇所をファイルに保存",
    "dialog.saveSel.path": "保存先パス",
    "dialog.saveSel.hint":
      "選択中の {lines} 行を UTF-8 / LF で書き出します。コピーの行数上限 ({max} 行) はかかりません。",
    "dialog.saveSel.writing": "選択を書き出し中…",
    "dialog.saveSel.done": "選択 {lines} 行を保存しました: {path}",
    "dialog.saveSel.error": "選択の保存エラー",
    // -- overwrite confirmation --
    "dialog.overwrite.title": "上書きの確認",
    "dialog.overwrite.ask": "{name} は既に存在します。上書きしますか?",
    "dialog.overwrite.ok": "上書き",
    // -- exit confirmation --
    "dialog.exit.title": "終了の確認",
    "dialog.exit.withoutSaving": "保存せずに終了",
    "dialog.exit.exit": "終了",
    "dialog.exit.savingWillClose": "保存処理中です。完了後に閉じます…",
    "dialog.exit.unsavedAsk": "未保存の編集があります。保存せずに終了しますか?",
    "dialog.exit.lastTabAsk": "最後のタブを閉じると Ayame Editor を終了します。終了しますか?",
    "dialog.exit.unsavedNamed":
      "{name} の未保存の編集があります。保存せずに Ayame Editor を終了しますか?",
    "dialog.exit.moreFiles": "ほか {n} 件",
    // -- goto-line prompt --
    "dialog.gotoLine.label": "行番号",
    // -- crash recovery --
    "recover.title": "クラッシュ復元",
    "recover.found": "クラッシュ前の未保存の編集が見つかりました（{n}件）。復元しますか？",
    "recover.restore": "復元する",
    "recover.discard": "破棄",
    "recover.restored": "クラッシュ前の編集を復元しました（{n}件）",
    "recover.discarded": "クラッシュ前の編集を破棄しました",
    "recover.error": "復元エラー",
    "recover.walDisabled": "自動保存ログが無効になりました: {msg}",
    // -- file operations / errors --
    "file.saved": "保存しました: {path}",
    "error.cannotOpen": "開けません: {msg}",
    "error.loadError": "読み込みエラー",
    "error.loadErrorMsg": "読み込みエラー: {msg}",
    "error.saveError": "保存エラー",
    "error.serverUnreachable": "サーバに接続できません",
    "error.newBuffer": "新規バッファを作成できません: {msg}",
    // -- theme JSON --
    "theme.cannotOpen": "テーマを開けません",
    "theme.missingColor": "color がありません",
    "theme.jsonError": "テーマ JSON エラー",
    "theme.applied": "テーマ適用: {name}",
    // -- app chrome --
    "app.dropToOpen": "ドロップしてファイルを開く",
  },
  en: {
    "menu.bar": "Menu",
    "menu.file": "File",
    "menu.edit": "Edit",
    "menu.selection": "Selection",
    "menu.view": "View",
    "menu.tools": "Tools",
    "menu.newFile": "New File",
    "menu.newWindow": "New Window",
    "menu.open": "Open",
    "menu.save": "Save",
    "menu.saveAs": "Save As",
    "menu.encoding": "Encoding / Line Endings...",
    "menu.undo": "Undo",
    "menu.redo": "Redo",
    "menu.find": "Find",
    "menu.replace": "Replace",
    "menu.gotoLine": "Go to Line",
    "menu.duplicateLine": "Duplicate Line",
    "menu.moveLineUp": "Move Line Up",
    "menu.moveLineDown": "Move Line Down",
    "menu.deleteLine": "Delete Line",
    "menu.selectAll": "Select All",
    "menu.selectNextOccurrence": "Select Next Occurrence",
    "menu.copy": "Copy",
    "menu.cut": "Cut",
    "menu.caseUpper": "Transform to Uppercase",
    "menu.caseLower": "Transform to Lowercase",
    "menu.caseCamel": "Transform to camelCase",
    "menu.casePascal": "Transform to PascalCase",
    "menu.caseSnake": "Transform to snake_case",
    "menu.caseKebab": "Transform to kebab-case",
    "menu.caseConstant": "Transform to CONSTANT_CASE",
    "menu.addCursorAbove": "Add Cursor Above",
    "menu.addCursorBelow": "Add Cursor Below",
    "menu.explorer": "Explorer",
    "menu.findBar": "Find Bar",
    "menu.commandPalette": "Command Palette",
    "menu.showWhitespace": "Show Whitespace and Line Endings",
    "menu.syntaxHighlight": "Syntax Highlighting",
    "menu.zenkakuUnderline": "Underline Full-width Spaces",
    "menu.wordWrap": "Word Wrap",
    "menu.followTail": "Follow Tail (tail -f)",
    "menu.settings": "Settings",
    "menu.sort": "Sort",
    "menu.sortTitle":
      "Sort lines and overwrite the current file. By default it compares whole lines; with a key column it compares that column.",
    "menu.diff": "Two-file Diff",
    "menu.split": "Split File",
    "menu.splitTitle": "Split into multiple files by line count.",
    "menu.grep": "Search Folder",
    "menu.grepTitle":
      "Search files in a folder recursively, with file-name filters and regular expressions.",
    "menu.grepSave": "Grep to File",
    "menu.grepSaveTitle":
      "Write only the lines matching a pattern to a separate file (regex, case, whole-word supported).",
    "toolbar.applyTheme": "Apply Theme",
    "toolbar.applyThemeTitle": "Apply this JSON as a theme",
    "toolbar.applyKeymap": "Apply Key Bindings",
    "toolbar.applyKeymapTitle": "Apply this JSON as key bindings",
    "toolbar.toolsTitle": "Sort, Diff, Split",
    "toolbar.newTab": "New Tab",
    "tab.close": "Close Tab",
    "tab.closeName": "Close {name}",
    "tab.confirmDiscard": "Discard unsaved edits in {name} and close it?",
    "tab.discardClose": "Discard and Close",
    "tab.switchError": "Tab switch error",
    "tab.closeError": "Could not close the tab.",
    "tab.moveDirty": "Save the tab before moving it to another window.",
    "tab.handoffDone": "Unsaved edits carried over.",
    "tab.handoffError": "Tab handoff failed (the tab stays in its original window).",
    "tree.close": "Close Explorer",
    "tree.actions": "Explorer Actions",
    "tree.up": "Up One Level",
    "tree.back": "Back",
    "tree.forward": "Forward",
    "find.group": "Find and Replace",
    "find.showReplace": "Show Replace",
    "find.matchCase": "Match Case",
    "find.wholeWord": "Whole Word",
    "find.regex": "Regular Expression",
    "find.prev": "Previous Match",
    "find.next": "Next Match",
    "find.close": "Close Find",
    "find.closeTitle": "Close Find (Esc)",
    "find.replaceWith": "Replace With",
    "find.replaceOneTitle": "Replace Current Match and Go Next",
    "find.replaceAll": "Replace All",
    "find.replaceAllTitle": "Replace All Matches (undoable)",
    "find.matchCount": "{total} matches",
    "find.noMatch": "No matches",
    "find.wrapTop": "Wrapped to top",
    "find.wrapBottom": "Wrapped to bottom",
    "find.regexError": "Regular expression error",
    "find.searchError": "Search error",
    "find.noNextOccurrence": "No next occurrence.",
    "find.noWordToSelect": "No selectable word.",
    "find.rectNoCtrlD": "Ctrl+D cannot be used with rectangular selection.",
    "find.multiLineNoCtrlD": "Ctrl+D cannot be used with a multi-line selection.",
    "find.enterQuery": "Enter a search string.",
    "find.cannotIdentifyMatch": "Could not identify the match.",
    "find.replaceError": "Replace error",
    "find.replacing": "Replacing...",
    "find.replacedCount": "Replaced {n} matches.",
    "find.replacedCountPartial": "Replaced {n} matches. There are more matches; run it again.",
    "status.saving": "Saving...",
    "status.follow": "Follow",
    "status.saved": "Saved",
    "status.unsaved": "Unsaved",
    "status.indexOk": "Index OK",
    "status.allSaved": "All edits are saved.",
    "status.line0": "Line 0",
    "status.pos": "Line {line}, Column {col}",
    "status.posCursors": "{pos} · {n} cursors",
    "status.unsavedDetail":
      "Unsaved edits: +{added} lines added / ~{changed} lines changed / -{deleted} lines deleted",
    "status.indexDetail":
      "{lines} lines / {bytes} / {checkpoints} index checkpoints ({indexBytes}, {indexMs} ms)",
    "status.encTitle": "Convert Encoding and Save",
    "status.zoomTitle": "Zoom level (click to reset to 100%)",
    "status.eolTitle": "Convert Line Endings and Save",
    "status.followingTail": "Following tail (tail -f)",
    "status.followStopped": "Stopped following tail",
    "status.tailFileChanged": "The file changed externally. Tail following stopped.",
    "editor.label": "Editor",
    "editor.copied": "Copied",
    "editor.copyError": "Copy error",
    "editor.pasteBlocked": "Cannot paste from here. Use Ctrl+V.",
    "editor.noSelection": "No selection",
    "editor.multiSelUseCopy": "Use Copy or Cut for multiple selections.",
    "editor.copyCapped": "Copied only the first {max} lines. {rest} lines were not copied.",
    "editor.copyCappedHint":
      "Copied only the first {max} lines. {rest} lines were not copied. Use right-click > Save Selection to File to write everything.",
    "editor.cutCapped":
      "Cut is limited to {max} lines ({total} selected). Use Delete to delete only.",
    "editor.cutCappedHint":
      "Cut is limited to {max} lines ({total} selected). Use right-click > Save Selection to File to keep everything, or Delete to delete only.",
    "editor.selectRangeFirst": "Select a range to transform.",
    "editor.transformCapped": "Transform is limited to {max} lines at once.",
    "editor.duplicateCapped": "Duplicate is limited to {max} lines at once.",
    "editor.moveCapped": "Moving lines is limited to {max} lines at once.",
    "editor.editError": "Edit error",
    "editor.reloadError": "Reload error",
    "editor.savingWaitInput": "Saving. Input will continue after it finishes.",
    "editor.savingWait": "Saving. Please wait until it finishes.",
    "ctx.menu": "Context Menu",
    "ctx.paste": "Paste",
    "ctx.saveSelection": "Save Selection to File...",
    "ctx.saveSelectionTitle":
      "Write only the selected lines to another file. The clipboard line limit does not apply.",
    "common.ok": "OK",
    "common.cancel": "Cancel",
    "common.close": "Close",
    "common.closeEsc": "Close (Esc)",
    "common.run": "Run",
    "common.confirm": "Confirm",
    "common.input": "Input",
    "common.options": "Options",
    "common.error": "Error",
    "dialog.open.title": "Open File",
    "dialog.open.path": "Path",
    "dialog.open.fileName": "File Name",
    "dialog.open.pathPlaceholder": "Enter a file path... (e.g. /var/log/huge.log)",
    "dialog.open.namePlaceholder": "File name to save, or a full path",
    "dialog.open.folder": "Folder",
    "dialog.open.location": "Location",
    "dialog.open.folderToTree": "Open the current folder in the tree",
    "dialog.open.folderToExplorer": "Show the current folder in Explorer",
    "dialog.open.hintOpen":
      "Drag and drop a file here to open it. For large files, entering a path is faster.",
    "dialog.open.hintSave":
      "Choose a folder and enter a file name. Selecting an existing file asks before overwriting.",
    "dialog.open.recent": "Recent Files",
    "dialog.open.loading": "Loading...",
    "dialog.open.loadingName": "Loading... ({name})",
    "dialog.open.loadingFile": "Loading... {name}",
    "dialog.open.opening": "Opening...",
    "dialog.open.openingName": "Opening: {name} ...",
    "dialog.open.dirError": "Cannot open directory: {msg}",
    "dialog.open.enterFileName": "Enter a file name to save.",
    "dialog.open.pickFolderFirst": "Choose a destination drive and folder first.",
    "dialog.open.folderShown": "Showing the current folder in Explorer.",
    "settings.theme": "Theme",
    "settings.themeMonoPaper": "Mono Paper (Solid)",
    "settings.themeDark": "Dark",
    "settings.themeBlack": "Black",
    "settings.background": "Background",
    "settings.bgDefault": "Default",
    "settings.bgSolid": "Solid",
    "settings.bgImage": "Custom Image",
    "settings.bgImagePick": "Choose Image…",
    "settings.bgImageTooLarge": "Image is too large (max 4MB)",
    "settings.bgImageError": "Could not load the image",
    "settings.bgImagePersistError": "Could not save the image — applied for this session only",
    "settings.language": "Language",
    "language.name": "English",
    "language.auto": "Auto",
    weekday: {
      short: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
      long: ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"],
    },
    "settings.illustration": "Illustration",
    "settings.font": "Font",
    "settings.fontMono": "Monospace (Consolas / Menlo)",
    "settings.fontMonoJp": "Monospace + Japanese (Noto/MS Gothic)",
    "settings.fontSystem": "System UI",
    "settings.fontSize": "Font Size",
    "settings.ruler": "Column Ruler",
    "settings.lineNumberCommas": "Comma-Separate Line Numbers",
    "settings.restoreSession": "Restore Session",
    "settings.confirmExit": "Confirm Exit",
    "settings.memoName": "New file name",
    "settings.memoNameHint":
      "Variables: {yyyy} {yy} {mm} {dd} {HH} {MM} {ss} {ddd} {dddd} (weekday) {seq} (sequence) {date} {time} {datetime}",
    "settings.sidebar": "Sidebar",
    "settings.sidebarSide": "Sidebar Position",
    "settings.left": "Left",
    "settings.right": "Right",
    "settings.themeJson": "Theme JSON",
    "settings.editInTab": "Edit in Tab",
    "dialog.convert.title": "Encoding / Line Endings",
    "dialog.convert.encoding": "Encoding",
    "dialog.convert.eol": "Line Endings",
    "dialog.convert.eolCr": "CR (classic Mac)",
    "dialog.convert.bom": "Add BOM (UTF-8 / UTF-16)",
    "dialog.convert.reopen": "Reopen",
    "dialog.convert.go": "Convert and Save",
    "dialog.convert.savedAs": "Saved as {enc} / {eol}",
    "dialog.convert.reopenedAs": "Reopened as {enc}",
    "dialog.convert.saveError": "Convert Save Error",
    "dialog.convert.saveFirst": "Save the file first.",
    "dialog.convert.noSavedFile": "There is no saved file.",
    "dialog.convert.discardAsk": "Discard unsaved edits and reopen?",
    "dialog.convert.discardOk": "Discard and Reopen",
    "dialog.convert.reopenError": "Reopen Error",
    "keymap.title": "Key Bindings",
    "keymap.hint": "Focus a field and press keys to change it. Backspace / Delete clears it.",
    "keymap.editJson": "Edit JSON in Tab",
    "keymap.reset": "Reset to Defaults",
    "keymap.resetConfirm": "Reset all key bindings to their defaults?",
    "keymap.unassigned": "Unassigned",
    "keymap.conflictKey": "That shortcut conflicts with text input.",
    "keymap.searchCase": "Search: Match Case",
    "keymap.searchWord": "Search: Whole Word",
    "keymap.searchRegex": "Search: Regular Expression",
    "keymap.toggleSidebar": "Toggle Explorer",
    "keymap.cannotOpen": "Could not open key bindings.",
    "keymap.jsonError": "Key bindings JSON error",
    "dialog.sort.keyColumn": "Key Column (1-based)",
    "dialog.sort.keyPlaceholder": "Leave empty to compare whole lines",
    "dialog.sort.keyTitle":
      "Empty: compare whole lines as strings. Number: compare that delimited column as the key.",
    "dialog.sort.delimiter": "Delimiter",
    "dialog.sort.delimiterTitle": "Column delimiter when using a key column, such as comma or tab.",
    "dialog.sort.numeric": "Compare as numbers",
    "dialog.sort.numericTitle": "Sort 10 and 9 by numeric value instead of string order.",
    "dialog.sort.order": "Order",
    "dialog.sort.asc": "Ascending (A to Z, small to large)",
    "dialog.sort.desc": "Descending (Z to A, large to small)",
    "dialog.sort.hint":
      "Sort the current file and overwrite it. Unsaved edits are included. This operation cannot be undone.",
    "dialog.sort.keyInvalid": "Key column must be an integer greater than or equal to 1.",
    "dialog.sort.running": "Sorting...",
    "dialog.sort.done": "Sorted and overwritten.",
    "dialog.sort.error": "Sort error",
    "dialog.split.linesPer": "Lines per File",
    "dialog.split.outDir": "Output Folder",
    "dialog.split.outDirPlaceholder": "Leave empty to use the original file's folder",
    "dialog.split.hint":
      "Write the current file into parts by line count. Unsaved edits are included. The original file is not changed.",
    "dialog.split.go": "Split",
    "dialog.split.linesInvalid": "Line count must be an integer greater than or equal to 1.",
    "dialog.split.running": "Splitting...",
    "dialog.split.done": "Split into {count} files: first file {path}",
    "dialog.split.error": "Split error",
    "dialog.diff.title": "Diff",
    "dialog.diff.current": "Current",
    "dialog.diff.compareTo": "Compare To",
    "dialog.diff.currentFile": "Current File",
    "dialog.diff.compareFile": "Comparison File",
    "dialog.diff.added": "Added",
    "dialog.diff.deleted": "Deleted",
    "dialog.diff.changed": "Changed",
    "dialog.diff.none": "No differences",
    "dialog.diff.promptPath": "Comparison File Path",
    "dialog.diff.computing": "Computing diff...",
    "dialog.diff.error": "Diff error",
    "dialog.diff.hunks": "Diff: {n} hunk(s)",
    "dialog.diff.hunkHeader":
      "{kind}  Current: {oldStart} ({oldLen} lines)  Compare To: {newStart} ({newLen} lines)",
    "dialog.diff.unsavedIncluded": "includes unsaved edits",
    "dialog.diff.hunkTruncated": "This hunk shows only the first {n} lines.",
    "dialog.diff.summary": "{hunks} hunk / +{added}  -{deleted}  ~{modified}",
    "dialog.diff.omitted": "{n} hunk omitted",
    "dialog.grep.query": "Search Term",
    "dialog.grep.queryPlaceholder": "String or regular expression to search for",
    "dialog.grep.dir": "Target Folder",
    "dialog.grep.dirPlaceholder": "Leave empty to use the open file's folder",
    "dialog.grep.glob": "File Name Filter",
    "dialog.grep.globPlaceholder": "Example: *.rs, *.txt (empty for all)",
    "dialog.grep.ignoreCase": "Ignore case",
    "dialog.grep.searching": "Searching folder...",
    "dialog.grep.error": "Folder Search Error",
    "dialog.grep.noMatches": "No matches",
    "dialog.grep.flash": "Folder search: {n} matches",
    "dialog.grep.summary": "{hits} matches / {files} files",
    "dialog.grep.summaryTruncated": " (stopped at the {max} match limit)",
    "dialog.grep.summaryFiles": " / reached the scanned-file limit",
    "dialog.grepSave.hint":
      "Writes only the matching lines to a new file (unsaved edits included). Streams, so multi-GB files complete.",
    "dialog.grepSave.go": "Choose Destination",
    "dialog.grepSave.running": "Extracting matching lines...",
    "dialog.grepSave.error": "Grep to File Error",
    "dialog.saveSel.title": "Save Selection to File",
    "dialog.saveSel.path": "Save Path",
    "dialog.saveSel.hint":
      "Writes the selected {lines} lines as UTF-8 / LF. The clipboard limit of {max} lines does not apply.",
    "dialog.saveSel.writing": "Writing selection...",
    "dialog.saveSel.done": "Saved {lines} selected lines: {path}",
    "dialog.saveSel.error": "Selection Save Error",
    "dialog.overwrite.title": "Overwrite Confirmation",
    "dialog.overwrite.ask": "{name} already exists. Overwrite it?",
    "dialog.overwrite.ok": "Overwrite",
    "dialog.exit.title": "Confirm Exit",
    "dialog.exit.withoutSaving": "Exit Without Saving",
    "dialog.exit.exit": "Exit",
    "dialog.exit.savingWillClose":
      "Saving is in progress. The window will close when it finishes...",
    "dialog.exit.unsavedAsk": "There are unsaved edits. Exit without saving?",
    "dialog.exit.lastTabAsk": "Closing the last tab exits Ayame Editor. Exit?",
    "dialog.exit.unsavedNamed": "{name} has unsaved edits. Exit Ayame Editor without saving?",
    "dialog.exit.moreFiles": "{n} more",
    "dialog.gotoLine.label": "Line Number",
    "recover.title": "Crash Recovery",
    "recover.found": "Found {n} unsaved edit(s) from before a crash. Restore them?",
    "recover.restore": "Restore",
    "recover.discard": "Discard",
    "recover.restored": "Restored {n} pre-crash edit(s)",
    "recover.discarded": "Discarded the pre-crash edits",
    "recover.error": "Recovery error",
    "recover.walDisabled": "Crash-recovery logging was disabled: {msg}",
    "file.saved": "Saved: {path}",
    "error.cannotOpen": "Cannot open: {msg}",
    "error.loadError": "Load error",
    "error.loadErrorMsg": "Load error: {msg}",
    "error.saveError": "Save error",
    "error.serverUnreachable": "Cannot connect to the server",
    "error.newBuffer": "Cannot create a new buffer: {msg}",
    "theme.cannotOpen": "Could not open the theme.",
    "theme.missingColor": "Missing color.",
    "theme.jsonError": "Theme JSON error",
    "theme.applied": "Theme applied: {name}",
    "app.dropToOpen": "Drop to Open File",
  },
};

// ---- server boundary ----------------------------------------------------
// The Rust side still reports errors as Japanese strings; they reach the
// client at runtime in e.message / stat.wal_error. This small map (plus the
// few parameterized patterns below) is the ONLY remaining string-matching
// translation layer — it goes away once the server exposes error codes.
export const SERVER_MSG_EN = {
  保存先パスが空です: "Save path is empty.",
  選択範囲が不正です: "Selection range is invalid.",
  矩形選択の列範囲が不正です: "Rectangle selection column range is invalid.",
  ファイルが開かれていません: "No file is open.",
  "選択範囲が不正です (行が範囲外)": "Selection range is invalid (line is out of range).",
  "書き出し中に編集またはタブ切替が入ったため中断しました。もう一度実行してください":
    "Export was interrupted because an edit or tab switch happened while writing. Please try again.",
  "クラッシュログは無効です（キャッシュディレクトリなし）":
    "The crash log is disabled (no cache directory).",
  復元できるクラッシュログはありません: "There is no crash log to recover.",
  "復元中に編集が入ったため中断しました。ファイルを開き直してください":
    "Recovery was interrupted by an edit. Reopen the file.",
};

export const SERVER_MSG_EN_PATTERNS: [RegExp, (m: any) => string][] = [
  [/^(.+) は既に存在します$/u, (m) => `${m[1]} already exists.`],
  [/^(.+) での保存は未対応です$/u, (m) => `Saving as ${m[1]} is not supported.`],
  [/^(.+) での再読込は未対応です$/u, (m) => `Reopening as ${m[1]} is not supported.`],
  [/^クラッシュログを復元できません: (.+)$/u, (m) => `Cannot recover the crash log: ${m[1]}`],
];

// Translate a raw server-side message for the English UI; Japanese (and any
// unknown string) passes through unchanged.
export function serverMessage(text) {
  const raw = String(text ?? "");
  if (currentLocale() !== "en") return raw;
  const exact = SERVER_MSG_EN[raw];
  if (exact != null) return exact;
  for (const [re, fn] of SERVER_MSG_EN_PATTERNS) {
    const m = raw.match(re);
    if (m) return fn(m);
  }
  return raw;
}

// Server-boundary predicate: the save endpoint reports an existing target with
// a Japanese message (no error codes yet). Kept in the i18n/server-message
// layer so UI modules never embed the raw string themselves.
export function isExistsError(msg) {
  return String(msg ?? "").includes("既に存在");
}

// Available UI locales are exactly the top-level keys of MESSAGES ("auto" is not
// a locale — it defers to the browser). normalizeLanguage, the language picker,
// and browserLocale all derive from this, so adding a language is data-only.
export function availableLocales() {
  return Object.keys(MESSAGES);
}

export function normalizeLanguage(lang) {
  return lang === "auto" || availableLocales().includes(lang) ? lang : "auto";
}

// "auto": the first navigator.languages entry whose primary subtag has a
// MESSAGES table (prefix match, so "zh-TW" resolves to "zh"); English if none.
export function browserLocale() {
  const locales = availableLocales();
  const prefs =
    navigator.languages && navigator.languages.length
      ? navigator.languages
      : [navigator.language || ""];
  for (const pref of prefs) {
    const code = String(pref).toLowerCase().split("-")[0];
    if (locales.includes(code)) return code;
  }
  return "en";
}

export function currentLocale() {
  const lang = normalizeLanguage(state.settings?.language || "auto");
  return lang === "auto" ? browserLocale() : lang;
}

// Display name for a language option: each table names itself in "language.name"
// ("日本語", "English"); "auto" uses the active locale's "language.auto".
export function localeLabel(code) {
  if (code === "auto") return t("language.auto");
  return (MESSAGES[code] && MESSAGES[code]["language.name"]) || code;
}

// Weekday names for the 新規ファイル名 template ({ddd} short / {dddd} long),
// indexed by Date.getDay() (0 = Sunday). Lives in each MESSAGES block so it is
// part of "adding a language"; a block without a weekday table falls back to en.
export function weekdayNames(locale) {
  return (MESSAGES[locale] && MESSAGES[locale].weekday) || MESSAGES.en.weekday;
}

// Look up `key` in the active locale. Fallback chain: locale → en (the key
// language, kept complete) → the key itself. Then substitute {var} placeholders.
export function t(key, vars = null) {
  const table = MESSAGES[currentLocale()] || MESSAGES.en;
  let out = table[key] ?? MESSAGES.en[key] ?? key;
  if (vars) {
    out = out.replace(/\{(\w+)\}/g, (_, name) => String(vars[name] ?? ""));
  }
  return out;
}

// Static HTML is declaratively tagged: data-i18n (textContent) plus
// data-i18n-title / data-i18n-placeholder / data-i18n-aria-label for
// attributes. index.html ships with the Japanese literals in place (the ja
// default before JS runs); applyStaticI18n() re-assigns every tagged node for
// the active locale — including ja, so keys and markup can never drift apart.
export const I18N_ATTR_MAP = [
  ["data-i18n-title", "title"],
  ["data-i18n-placeholder", "placeholder"],
  ["data-i18n-aria-label", "aria-label"],
];

export function applyStaticI18n() {
  document.documentElement.lang = currentLocale();
  document.querySelectorAll("[data-i18n]").forEach((el) => {
    el.textContent = t(el.getAttribute("data-i18n"));
  });
  for (const [dataAttr, attr] of I18N_ATTR_MAP) {
    document.querySelectorAll(`[${dataAttr}]`).forEach((el) => {
      el.setAttribute(attr, t(el.getAttribute(dataAttr)));
    });
  }
}
