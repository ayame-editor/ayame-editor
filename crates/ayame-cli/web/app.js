// Ayame Editor front-end.
//
// Design rule: the browser never holds more than the visible window. Lines are
// fetched on demand from the local server; vertical position is tracked as a
// *line number* (not pixels), so navigation is exact for any file size — ten
// lines or Ayame Editor's minimum ten-billion-line scale. A custom scrollbar maps line
// position to a thumb, side-stepping the browser's ~33M-pixel element-height
// ceiling entirely.

const $ = (id) => document.getElementById(id);
let LINE_HEIGHT = 18; // tracks --line-height; updated by Settings (font size)
const OVERSCAN = 6;
const PAD = 400; // extra lines fetched around the viewport and cached
const SEARCH_HISTORY_KEY = "ayame.searchHistory.v1";
const SETTINGS_KEY = "ayame.settings.v1";
const TREE_KEY = "ayame.treeRoot.v1";
const RECENT_KEY = "ayame.recentFiles.v1";
const RECENT_MAX = 12; // cap on 最近使ったファイル entries
const MAX_COPY_LINES = 20000; // clipboard cap: copy warns, cut refuses beyond this

// ---- i18n -------------------------------------------------------------------
// Dot-namespaced English keys; both locales are translations. Japanese is the
// complete reference catalog — `en` falls back to `ja`, and `ja` falls back to
// the key itself. Interpolated strings carry {var} placeholders, substituted
// by t(key, vars). Static HTML is tagged with data-i18n attributes and
// re-applied per locale by applyStaticI18n().
const MESSAGES = {
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
    "menu.addCursorAbove": "カーソルを上に追加",
    "menu.addCursorBelow": "カーソルを下に追加",
    "menu.explorer": "エクスプローラー",
    "menu.findBar": "検索バー",
    "menu.commandPalette": "コマンドパレット",
    "menu.showWhitespace": "空白・改行を表示",
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
    "menu.grepTitle": "フォルダ内のファイルを再帰的に検索します (ファイル名フィルタ・正規表現に対応)",
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
    // -- explorer sidebar --
    "tree.close": "エクスプローラーを閉じる",
    "tree.actions": "エクスプローラー操作",
    "tree.up": "上の階層へ",
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
    "status.indexDetail": "{lines} 行 / {bytes} / 索引 {checkpoints} 点 ({indexBytes}, {indexMs} ms)",
    "status.encTitle": "文字コードを変換して保存",
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
    "dialog.open.folderShown": "現在のフォルダをエクスプローラーに表示しました",
    // -- settings --
    "settings.theme": "テーマ",
    "settings.themeMonoPaper": "Mono Paper (単色)",
    "settings.themeDark": "ダーク",
    "settings.themeBlack": "ブラック",
    "settings.background": "背景",
    "settings.bgWatercolor": "水彩",
    "settings.bgSolid": "単色（全単色配慮）",
    "settings.language": "言語",
    "settings.langAuto": "自動",
    "settings.langJa": "日本語",
    "settings.langEn": "英語",
    "settings.illustration": "イラスト",
    "settings.font": "フォント",
    "settings.fontMono": "等幅 (Consolas / Menlo)",
    "settings.fontMonoJp": "等幅 + 日本語 (Noto/MS Gothic)",
    "settings.fontSystem": "システムUI",
    "settings.fontSize": "文字サイズ",
    "settings.ruler": "列ルーラー",
    "settings.confirmExit": "終了確認",
    "settings.memoDir": "メモの保存先",
    "settings.memoDirPlaceholder": "例: /home/you/memo — 空なら保存ダイアログ",
    "settings.memoName": "メモの名前",
    "settings.memoNameHint":
      "使える変数: {yyyy} {yy} {mm} {dd} {HH} {MM} {ss} {date} {time} {datetime}",
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
    "dialog.convert.bom": "BOMを付ける（UTF-8のみ）",
    "dialog.convert.noteReopen":
      "「開き直す」= 選んだ文字コードで読み直し（保存しません／未保存の編集は破棄）。文字化け時の復帰用。",
    "dialog.convert.noteConvert":
      "「変換して保存」= 選んだ文字コード・改行コードで上書き保存（表せない文字があると中止）。",
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
    "error.memoDir": "メモの保存先を開けません: {dir}",
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
    "menu.addCursorAbove": "Add Cursor Above",
    "menu.addCursorBelow": "Add Cursor Below",
    "menu.explorer": "Explorer",
    "menu.findBar": "Find Bar",
    "menu.commandPalette": "Command Palette",
    "menu.showWhitespace": "Show Whitespace and Line Endings",
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
    "tree.close": "Close Explorer",
    "tree.actions": "Explorer Actions",
    "tree.up": "Up One Level",
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
    "editor.cutCapped": "Cut is limited to {max} lines ({total} selected). Use Delete to delete only.",
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
    "dialog.open.folderShown": "Showing the current folder in Explorer.",
    "settings.theme": "Theme",
    "settings.themeMonoPaper": "Mono Paper (Solid)",
    "settings.themeDark": "Dark",
    "settings.themeBlack": "Black",
    "settings.background": "Background",
    "settings.bgWatercolor": "Watercolor",
    "settings.bgSolid": "Solid",
    "settings.language": "Language",
    "settings.langAuto": "Auto",
    "settings.langJa": "Japanese",
    "settings.langEn": "English",
    "settings.illustration": "Illustration",
    "settings.font": "Font",
    "settings.fontMono": "Monospace (Consolas / Menlo)",
    "settings.fontMonoJp": "Monospace + Japanese (Noto/MS Gothic)",
    "settings.fontSystem": "System UI",
    "settings.fontSize": "Font Size",
    "settings.ruler": "Column Ruler",
    "settings.confirmExit": "Confirm Exit",
    "settings.memoDir": "Memo Folder",
    "settings.memoDirPlaceholder": "Example: /home/you/memo - empty uses the save dialog",
    "settings.memoName": "Memo Name",
    "settings.memoNameHint":
      "Available variables: {yyyy} {yy} {mm} {dd} {HH} {MM} {ss} {date} {time} {datetime}",
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
    "dialog.convert.bom": "Add BOM (UTF-8 only)",
    "dialog.convert.noteReopen":
      '"Reopen" reloads with the selected encoding without saving and discards unsaved edits. Use it to recover mojibake.',
    "dialog.convert.noteConvert":
      '"Convert and Save" overwrites with the selected encoding and line endings, stopping if any character cannot be represented.',
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
    "dialog.sort.delimiterTitle":
      "Column delimiter when using a key column, such as comma or tab.",
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
    "error.memoDir": "Cannot open memo folder: {dir}",
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
const SERVER_MSG_EN = {
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
const SERVER_MSG_EN_PATTERNS = [
  [/^(.+) は既に存在します$/u, (m) => `${m[1]} already exists.`],
  [/^(.+) での保存は未対応です$/u, (m) => `Saving as ${m[1]} is not supported.`],
  [/^(.+) での再読込は未対応です$/u, (m) => `Reopening as ${m[1]} is not supported.`],
  [/^クラッシュログを復元できません: (.+)$/u, (m) => `Cannot recover the crash log: ${m[1]}`],
];

// Translate a raw server-side message for the English UI; Japanese (and any
// unknown string) passes through unchanged.
function serverMessage(text) {
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

function normalizeLanguage(lang) {
  return ["auto", "ja", "en"].includes(lang) ? lang : "auto";
}

function browserLocale() {
  return String(navigator.language || "")
    .toLowerCase()
    .startsWith("ja")
    ? "ja"
    : "en";
}

function currentLocale() {
  const lang = normalizeLanguage(state.settings?.language || "auto");
  return lang === "auto" ? browserLocale() : lang;
}

// Look up `key` in the active locale (fallback chain: locale → ja → key),
// then substitute {var} placeholders from `vars`.
function t(key, vars = null) {
  const table = MESSAGES[currentLocale()] || MESSAGES.ja;
  let out = table[key] ?? MESSAGES.ja[key] ?? key;
  if (vars) {
    out = out.replace(/\{(\w+)\}/g, (_, name) => String(vars[name] ?? ""));
  }
  return out;
}

const FONT_STACKS = {
  mono: '"SFMono-Regular","Menlo","Consolas","DejaVu Sans Mono",monospace',
  "mono-jp": '"Consolas","Menlo","Noto Sans Mono CJK JP","MS Gothic",monospace',
  system: '"Segoe UI","Hiragino Kaku Gothic ProN","Noto Sans JP",system-ui,sans-serif',
};
const DEFAULT_SETTINGS = {
  theme: "iris-light",
  font: "mono",
  fontSize: 13,
  sidebar: false,
  sidebarSide: "left",
  ruler: true,
  confirmLastTabClose: true,
  showWhitespace: false,
  zenkakuUnderline: false,
  wordWrap: false,
  bgMode: "watercolor",
  illus: null,
  language: "auto",
  keymap: {},
  customThemes: {},
  // クイックメモ: 保存先フォルダ (空 = 保存ダイアログ) と名前テンプレート。
  memoDir: "",
  memoName: "memo-{yyyy}{mm}{dd}.txt",
  // 前回の保存先: last save-as folder, suggested for new untitled buffers.
  lastSaveDir: "",
};

// [action id, i18n label key, default shortcut(s)]
const KEYMAP_ACTIONS = [
  ["newFile", "menu.newFile", "Ctrl+N"],
  ["newWindow", "menu.newWindow", "Ctrl+Shift+N"],
  ["openFile", "menu.open", "Ctrl+O"],
  ["saveFile", "menu.save", "Ctrl+S"],
  ["saveAs", "menu.saveAs", "Ctrl+Shift+S"],
  ["closeTab", "tab.close", "Ctrl+W"],
  ["commandPalette", "menu.commandPalette", "Ctrl+Shift+P"],
  ["toggleSidebar", "keymap.toggleSidebar", "Ctrl+B"],
  ["find", "menu.find", "Ctrl+F"],
  ["replace", "menu.replace", "Ctrl+H"],
  ["findNext", "find.next", "F3"],
  ["findPrev", "find.prev", "Shift+F3"],
  ["gotoLine", "menu.gotoLine", "Ctrl+G"],
  ["undo", "menu.undo", "Ctrl+Z"],
  ["redo", "menu.redo", ["Ctrl+Y", "Ctrl+Shift+Z"]],
  ["selectAll", "menu.selectAll", "Ctrl+A"],
  ["selectNextOccurrence", "menu.selectNextOccurrence", "Ctrl+D"],
  ["addCursorAbove", "menu.addCursorAbove", "Ctrl+Alt+ArrowUp"],
  ["addCursorBelow", "menu.addCursorBelow", "Ctrl+Alt+ArrowDown"],
  ["duplicateLine", "menu.duplicateLine", "Ctrl+Shift+D"],
  ["moveLineUp", "menu.moveLineUp", "Alt+ArrowUp"],
  ["moveLineDown", "menu.moveLineDown", "Alt+ArrowDown"],
  ["deleteLine", "menu.deleteLine", "Ctrl+Shift+K"],
  ["copy", "menu.copy", "Ctrl+C"],
  ["cut", "menu.cut", "Ctrl+X"],
  ["searchCase", "keymap.searchCase", "Alt+C"],
  ["searchWord", "keymap.searchWord", "Alt+W"],
  ["searchRegex", "keymap.searchRegex", "Alt+R"],
  ["sortSave", "menu.sort", ""],
  ["diffFile", "menu.diff", ""],
  ["splitFile", "menu.split", ""],
  ["grepFolder", "menu.grep", ""],
  ["caseUpper", "menu.caseUpper", ""],
  ["caseLower", "menu.caseLower", ""],
  ["settings", "menu.settings", ""],
  ["keymap", "keymap.title", ""],
];
const DEFAULT_KEYMAP = Object.fromEntries(
  KEYMAP_ACTIONS.map(([id, _label, shortcut]) => [id, shortcut]),
);

const state = {
  total: 0,
  first: 0, // top visible line (0-based)
  fracAcc: 0, // sub-line wheel accumulator
  cache: { start: 0, lines: [] },
  loadToken: 0,
  stat: null,
  // search
  query: "",
  regex: false,
  ci: false,
  word: false,
  matcher: null,
  regexError: false,
  activeLine: -1,
  lastMatch: null, // { byte, len }
  searchHits: null,
  searchTruncated: false,
  findOpen: false,
  replaceOpen: false,
  history: [],
  historyIndex: -1,
  settings: { ...DEFAULT_SETTINGS },
  tabs: [], // open tabs from /api/tabs
  followTail: false, // 末尾に追従 (tail -f): poll for appended data and auto-scroll
  tailTimer: null, // setInterval handle while following; cleared when off
  treeParent: null, // parent of the current tree root (for the "up" button)
  treeLoaded: false,
  openerDir: null, // directory currently shown in the open dialog
  openerMode: "open", // "open" | "save"
  openerEntries: [],
  openerResolve: null,
  // ---- caret-based (Notepad-style) editing ----
  caret: { line: 0, col: 0 }, // (line, col) in Unicode scalars, like the backend
  goalCol: 0, // remembered column for vertical caret motion
  editGen: 0, // bumps on every user caret move; lets an in-flight edit detect it
  docGen: 0, // bumps whenever the active document/tab changes; cancels stale queued edits
  composing: false, // an IME composition is in progress
  focused: false, // the hidden text input holds focus (draw the caret)
  sel: null, // selection: { anchor: {line,col}, head: {line,col}, rect?: bool } or null
  extraCursors: [], // multi-cursor: additional carets [{line,col}]; primary is state.caret
  dragging: false,
  dragMoved: false,
  dragAnchor: null, // caret at mouse-down, promoted to a selection once it moves
  dragRect: false,
};

const pool = [];
let renderQueued = false;
let lastNativeTitle = "";

// ---- tiny helpers -----------------------------------------------------------

async function api(path) {
  const r = await fetch(path);
  if (!r.ok) throw new Error((await r.text()) || r.statusText);
  return r.json();
}

async function apiPost(path, body = {}) {
  const r = await fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!r.ok) throw new Error((await r.text()) || r.statusText);
  return r.json();
}

function commas(n) {
  return n.toLocaleString("en-US");
}

function humanBytes(n) {
  const u = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
  let v = n,
    i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return i === 0 ? `${n} B` : `${v.toFixed(2)} ${u[i]}`;
}

function escapeRegExp(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// Windows extended-length paths come back from canonicalize with a "\\?\"
// prefix; never show that to the user ("保存しました: \\?\C:\…" reads broken).
function displayPath(path) {
  const s = String(path || "");
  if (s.startsWith("\\\\?\\UNC\\")) return "\\\\" + s.slice(8);
  if (s.startsWith("\\\\?\\")) return s.slice(4);
  return s;
}

// Shortcuts are stored with KeyboardEvent key names ("Ctrl+Alt+ArrowUp");
// menus and hints render the arrows as glyphs so labels stay compact.
function displayShortcut(shortcut) {
  return String(shortcut || "")
    .replace(/ArrowUp/g, "↑")
    .replace(/ArrowDown/g, "↓")
    .replace(/ArrowLeft/g, "←")
    .replace(/ArrowRight/g, "→");
}

// Show/hide one modal element, keeping the .hidden class and aria-hidden in
// step (every modal in the app pairs the two).
function setModalOpen(modal, open) {
  modal.classList.toggle("hidden", !open);
  modal.setAttribute("aria-hidden", open ? "false" : "true");
}

// Build an <svg><use href="#id"></use></svg> node for a sprite symbol from
// index.html. Purely decorative: callers keep the accessible name on the
// element (aria-label / visible text) — the icon itself is aria-hidden.
function iconSvg(id, cls = "ay-icon") {
  const NS = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(NS, "svg");
  svg.setAttribute("class", cls);
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("focusable", "false");
  const use = document.createElementNS(NS, "use");
  use.setAttribute("href", `#${id}`);
  svg.append(use);
  return svg;
}

const APP_MENUS = ["file", "edit", "selection", "view", "tools"];
const MENU_ID_ACTIONS = [
  ["new-file", "newFile"],
  ["open-file", "openFile"],
  ["save-file", "saveFile"],
  ["save-copy", "saveAs"],
];

function fileMenuVisible() {
  return APP_MENUS.some((id) => !$(`${id}-menu`).classList.contains("hidden"));
}

function showAppMenu(id) {
  hideFileMenu();
  if (id === "view") {
    const ws = $("menu-toggle-ws");
    if (ws) {
      const on = !!state.settings.showWhitespace;
      ws.classList.toggle("checked", on);
      ws.setAttribute("aria-checked", String(on));
    }
    const zu = $("menu-toggle-zsp-underline");
    if (zu) {
      const zon = !!state.settings.zenkakuUnderline;
      zu.classList.toggle("checked", zon);
      zu.setAttribute("aria-checked", String(zon));
    }
    const wrap = $("menu-toggle-wrap");
    if (wrap) {
      const on = !!state.settings.wordWrap;
      wrap.classList.toggle("checked", on);
      wrap.setAttribute("aria-checked", String(on));
    }
    const tail = $("menu-toggle-tail");
    if (tail) {
      tail.classList.toggle("checked", state.followTail);
      tail.setAttribute("aria-checked", String(state.followTail));
    }
  }
  $(`${id}-menu`).classList.remove("hidden");
  $(`${id}-menu-button`).classList.add("on");
  $(`${id}-menu-button`).setAttribute("aria-expanded", "true");
}

function hideFileMenu(focusButton = false) {
  let focused = false;
  for (const id of APP_MENUS) {
    const menu = $(`${id}-menu`);
    const button = $(`${id}-menu-button`);
    const wasOpen = !menu.classList.contains("hidden");
    menu.classList.add("hidden");
    button.classList.remove("on");
    button.setAttribute("aria-expanded", "false");
    if (focusButton && wasOpen && !focused) {
      button.focus();
      focused = true;
    }
  }
}

const STATIC_I18N_SKIP = [
  "#content",
  "#tabs",
  "#tree",
  "#opener-list",
  "#opener-recent",
  "#opener-cwd",
  "#st-msg",
  "#overlay",
  "#find-count",
  "#form-body",
  "#confirm-message",
  "#prompt-label",
  "#keymap-list",
  "#palette-list",
  "#diff-view",
  "#grep-results",
].join(",");
const I18N_ATTRS = ["aria-label", "title", "placeholder"];

function shouldSkipStaticI18nNode(node) {
  const el = node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement;
  return !!(el && el.closest(STATIC_I18N_SKIP));
}

function translatePreservingSpace(source) {
  const leading = source.match(/^\s*/)?.[0] || "";
  const trailing = source.match(/\s*$/)?.[0] || "";
  const core = source.trim();
  if (!core) return source;
  return `${leading}${translateText(core)}${trailing}`;
}

function applyStaticI18n(root = document.body) {
  document.documentElement.lang = currentLocale();
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      if (!node.nodeValue.trim() || shouldSkipStaticI18nNode(node)) {
        return NodeFilter.FILTER_REJECT;
      }
      return NodeFilter.FILTER_ACCEPT;
    },
  });
  const nodes = [];
  while (walker.nextNode()) nodes.push(walker.currentNode);
  for (const node of nodes) {
    if (node.__ayameI18nSource == null) node.__ayameI18nSource = node.nodeValue;
    node.nodeValue = translatePreservingSpace(node.__ayameI18nSource);
  }
  root.querySelectorAll("*").forEach((el) => {
    if (shouldSkipStaticI18nNode(el)) return;
    for (const attr of I18N_ATTRS) {
      if (!el.hasAttribute(attr)) continue;
      const store = `__ayameI18nAttr_${attr}`;
      if (el[store] == null) el[store] = el.getAttribute(attr);
      el.setAttribute(attr, translateText(el[store]));
    }
  });
}

function applyLocale() {
  applyStaticI18n();
  updateKeyHints();
  updateStatusMeta();
  updateStatusPos();
  updateFindCountLabel();
  updateTailUI();
  if (state.tabs?.length) renderTabs(state.tabs);
  if (keymapVisible()) renderKeymapRows();
  if (commandPaletteVisible()) {
    paletteItems = commandPaletteItems();
    renderCommandPalette();
  }
  renderRecentFiles();
}

function normalizeShortcut(raw) {
  if (!raw) return "";
  const parts = String(raw)
    .split("+")
    .map((p) => p.trim())
    .filter(Boolean);
  const mods = { Ctrl: false, Shift: false, Alt: false };
  let key = "";
  for (const part of parts) {
    const low = part.toLowerCase();
    if (low === "ctrl" || low === "control" || low === "cmd" || low === "command" || low === "meta")
      mods.Ctrl = true;
    else if (low === "shift") mods.Shift = true;
    else if (low === "alt" || low === "option") mods.Alt = true;
    else key = part.length === 1 ? part.toUpperCase() : part[0].toUpperCase() + part.slice(1);
  }
  if (!key || ["Ctrl", "Shift", "Alt"].includes(key)) return "";
  return [mods.Ctrl && "Ctrl", mods.Shift && "Shift", mods.Alt && "Alt", key]
    .filter(Boolean)
    .join("+");
}

function isBindableShortcut(shortcut) {
  if (!shortcut) return true;
  const parts = shortcut.split("+");
  const key = parts[parts.length - 1];
  return parts.includes("Ctrl") || parts.includes("Alt") || /^F\d+$/i.test(key);
}

function sanitizeKeymap(raw) {
  const src = raw && typeof raw === "object" ? raw : {};
  const clean = {};
  for (const [action] of KEYMAP_ACTIONS) {
    if (!Object.prototype.hasOwnProperty.call(src, action)) continue;
    if (Array.isArray(src[action])) {
      clean[action] = src[action].map(normalizeShortcut).filter((v) => v && isBindableShortcut(v));
    } else {
      const v = normalizeShortcut(src[action]);
      clean[action] = isBindableShortcut(v) ? v : "";
    }
  }
  return clean;
}

function eventShortcut(e) {
  if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return "";
  let key = e.key;
  if (key.length === 1) key = key.toUpperCase();
  else if (/^f\d+$/i.test(key)) key = key.toUpperCase();
  else key = key[0].toUpperCase() + key.slice(1);
  return [(e.ctrlKey || e.metaKey) && "Ctrl", e.shiftKey && "Shift", e.altKey && "Alt", key]
    .filter(Boolean)
    .join("+");
}

function shortcutList(action) {
  const custom =
    state.settings.keymap && Object.prototype.hasOwnProperty.call(state.settings.keymap, action)
      ? state.settings.keymap[action]
      : DEFAULT_KEYMAP[action];
  const list = Array.isArray(custom) ? custom : [custom];
  return list.map(normalizeShortcut).filter(Boolean);
}

function shortcutFor(action) {
  return shortcutList(action)[0] || "";
}

function matchesShortcut(e, action) {
  const ev = eventShortcut(e);
  return !!ev && shortcutList(action).includes(ev);
}

function postNativeMessage(msg) {
  try {
    if (window.ipc && typeof window.ipc.postMessage === "function") {
      window.ipc.postMessage(msg);
    }
  } catch {
    // The web build has no native IPC; title/close still work in the browser.
  }
}

function setAppTitle(title) {
  const next = title || "Ayame Editor";
  document.title = next;
  if (lastNativeTitle !== next) {
    lastNativeTitle = next;
    postNativeMessage(`ayame:title:${next}`);
  }
}

function dirtyTabNames() {
  const names = [];
  for (const t of state.tabs || []) {
    if (t.dirty && t.name) names.push(t.name);
  }
  if (state.stat?.dirty && names.length === 0) names.push(displayName(state.stat.path));
  return [...new Set(names)].filter(Boolean);
}

function hasDirtyDocuments() {
  return !!state.stat?.dirty || dirtyTabNames().length > 0;
}

function dirtyCloseMessage() {
  const dirty = dirtyTabNames();
  const shown = dirty.slice(0, 5).join(", ");
  const more =
    dirty.length > 5
      ? currentLocale() === "en"
        ? ` ${dirty.length - 5} more`
        : ` ほか ${dirty.length - 5} 件`
      : "";
  const suffix = shown ? `\n\n${shown}${more}` : "";
  return `${t("未保存の編集があります。保存せずに終了しますか?")}${suffix}`;
}

function isNativeApp() {
  return !!(window.ipc && typeof window.ipc.postMessage === "function");
}

function requestEditorClose() {
  if (isNativeApp()) {
    postNativeMessage("ayame:close-ok");
    return true;
  }
  window.close();
  return false;
}

// 新規ウィンドウ: native builds ask the Rust side to spawn a fresh window
// process (contract: the "ayame:new-window" IPC message); the plain browser
// build just opens the app URL in a new tab/window.
function openNewWindow() {
  if (isNativeApp()) {
    postNativeMessage("ayame:new-window");
    return;
  }
  window.open(location.href, "_blank");
}

async function confirmCloseLastTab(t) {
  if (t?.dirty) {
    return askConfirm(
      "終了の確認",
      `${t.name} の未保存の編集があります。保存せずに Ayame Editor を終了しますか?`,
      { okLabel: "保存せずに終了", danger: true },
    );
  }
  if (state.settings.confirmLastTabClose === false) return true;
  return askConfirm("終了の確認", "最後のタブを閉じると Ayame Editor を終了します。終了しますか?", {
    okLabel: "終了",
  });
}

// Never let the native window kill the process while a save is in flight; the
// close request is answered "cancel" and retried once the save settles.
// While saving: key edits are blocked (onEditKey) and IME/beforeinput commits
// wait inside enqueueEdit so confirmed text is delayed, not lost.
let savingCount = 0;
let savingWaiters = [];
function setSavingUI() {
  const on = savingCount > 0;
  document.documentElement.classList.toggle("saving", on);
  $("st-saving")?.classList.toggle("hidden", !on);
  if (!on && savingWaiters.length) {
    const waiters = savingWaiters;
    savingWaiters = [];
    for (const resolve of waiters) resolve();
  }
}
function waitForSavingDone() {
  if (savingCount === 0) return Promise.resolve();
  return new Promise((resolve) => savingWaiters.push(resolve));
}
let pendingNativeClose = false;
window.__ayameNativeCloseRequested = () => {
  if (savingCount > 0) {
    pendingNativeClose = true;
    flashCount("保存処理中です。完了後に閉じます…");
    postNativeMessage("ayame:close-cancel");
    return;
  }
  if (!hasDirtyDocuments()) {
    postNativeMessage("ayame:close-ok");
    return;
  }
  // Release the native close request first (it times out after a few
  // seconds), then ask with the in-app dialog; a confirmed close posts the
  // ok separately — the Rust side exits on it regardless of timing.
  postNativeMessage("ayame:close-cancel");
  askConfirm("終了の確認", dirtyCloseMessage(), {
    okLabel: "保存せずに終了",
    danger: true,
  }).then((ok) => {
    if (ok) postNativeMessage("ayame:close-ok");
  });
};

function retryPendingNativeClose() {
  if (pendingNativeClose && savingCount === 0) {
    pendingNativeClose = false;
    window.__ayameNativeCloseRequested();
  }
}

window.addEventListener("beforeunload", (e) => {
  if (!hasDirtyDocuments()) return;
  e.preventDefault();
  e.returnValue = "";
});

function setKeymap(action, shortcut) {
  const normalized = normalizeShortcut(shortcut);
  if (normalized && !isBindableShortcut(normalized)) {
    flashCount("文字入力と衝突するキーは使えません");
    return;
  }
  state.settings = {
    ...state.settings,
    keymap: { ...state.settings.keymap, [action]: normalized },
  };
  saveSettings(state.settings);
  updateKeyHints();
  renderKeymapRows();
}

function resetKeymap() {
  state.settings = { ...state.settings, keymap: {} };
  saveSettings(state.settings);
  updateKeyHints();
  renderKeymapRows();
}

function updateKeyHints() {
  document.querySelectorAll("[data-key-action]").forEach((el) => {
    el.textContent = displayShortcut(shortcutFor(el.dataset.keyAction));
  });
  const hint = (label, action) => {
    const key = displayShortcut(shortcutFor(action));
    const text = t(label);
    return key ? `${text} (${key})` : text;
  };
  $("toggle-sidebar").title = hint("エクスプローラー", "toggleSidebar");
  $("toggle-sidebar").setAttribute("aria-label", t("エクスプローラー"));
  $("undo-edit").title = hint("元に戻す", "undo");
  $("undo-edit").setAttribute("aria-label", t("元に戻す"));
  $("redo-edit").title = hint("やり直す", "redo");
  $("redo-edit").setAttribute("aria-label", t("やり直す"));
  $("find").placeholder = hint("検索", "find");
  $("find-expand").title = hint("置換を表示", "replace");
  $("find-expand").setAttribute("aria-label", t("置換を表示"));
  $("find-prev").title = hint("前の一致", "findPrev");
  $("find-prev").setAttribute("aria-label", t("前の一致"));
  $("find-next").title = hint("次の一致", "findNext");
  $("find-next").setAttribute("aria-label", t("次の一致"));
  $("opt-case").title = hint("大文字小文字を区別", "searchCase");
  $("opt-word").title = hint("単語単位", "searchWord");
  $("opt-regex").title = hint("正規表現", "searchRegex");
  $("new-tab").title = hint("新規タブ", "newFile");
  $("new-tab").setAttribute("aria-label", t("新規タブ"));
  $("hidden-input").setAttribute("aria-label", t("エディタ"));
}

function keymapVisible() {
  return !$("keymap-modal").classList.contains("hidden");
}

function showKeymap() {
  hideSettings();
  renderKeymapRows();
  setModalOpen($("keymap-modal"), true);
  queueMicrotask(() => $("keymap-list").querySelector("input")?.focus());
}

function hideKeymap() {
  setModalOpen($("keymap-modal"), false);
  focusEditor();
}

function renderKeymapRows() {
  const list = $("keymap-list");
  if (!list) return;
  const used = new Map();
  for (const [action] of KEYMAP_ACTIONS) {
    for (const key of shortcutList(action)) used.set(key, (used.get(key) || 0) + 1);
  }
  list.textContent = "";
  const frag = document.createDocumentFragment();
  for (const [action, label] of KEYMAP_ACTIONS) {
    const row = document.createElement("label");
    const shortcut = shortcutFor(action);
    row.className = "keymap-row";
    if (shortcut && used.get(shortcut) > 1) row.classList.add("conflict");
    const name = document.createElement("span");
    name.className = "keymap-label";
    name.textContent = t(label);
    const input = document.createElement("input");
    input.className = "keymap-input";
    input.readOnly = true;
    input.value = displayShortcut(shortcut);
    input.placeholder = t("未設定");
    input.dataset.action = action;
    input.addEventListener("keydown", (e) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        hideKeymap();
        return;
      }
      if (e.key === "Backspace" || e.key === "Delete") {
        setKeymap(action, "");
        return;
      }
      const shortcut = eventShortcut(e);
      if (shortcut) setKeymap(action, shortcut);
    });
    row.append(name, input);
    frag.append(row);
  }
  list.append(frag);
}

let paletteItems = [];
let paletteIndex = 0;

function commandPaletteVisible() {
  return !$("command-palette").classList.contains("hidden");
}

function commandLabelFromElement(el) {
  return el?.querySelector(".menu-label")?.textContent?.trim() || "";
}

function commandPaletteItems() {
  const keymapLabels = new Map(KEYMAP_ACTIONS.map(([action, label]) => [action, t(label)]));
  const seen = new Set();
  const items = [];
  const add = (action, label = "") => {
    if (!action || seen.has(action)) return;
    seen.add(action);
    const text = label ? translateText(label) : keymapLabels.get(action) || action;
    items.push({
      action,
      label: text.replace(/\.\.\.$/, ""),
      shortcut: shortcutFor(action),
    });
  };
  for (const [id, action] of MENU_ID_ACTIONS) add(action, commandLabelFromElement($(id)));
  document.querySelectorAll("[data-menu-action]").forEach((el) => {
    add(el.dataset.menuAction, commandLabelFromElement(el));
  });
  for (const [action, label] of KEYMAP_ACTIONS) add(action, label);
  return items;
}

function paletteMatches(item, query) {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  const hay = `${item.label} ${item.action} ${item.shortcut}`.toLowerCase();
  return q.split(/\s+/).every((part) => hay.includes(part));
}

function renderCommandPalette() {
  const list = $("palette-list");
  const query = $("palette-input").value;
  const visible = paletteItems.filter((item) => paletteMatches(item, query));
  paletteIndex = Math.max(0, Math.min(paletteIndex, visible.length - 1));
  list.textContent = "";
  const frag = document.createDocumentFragment();
  visible.forEach((item, index) => {
    const row = document.createElement("button");
    row.type = "button";
    row.className = "palette-row";
    row.classList.toggle("active", index === paletteIndex);
    row.setAttribute("role", "option");
    row.setAttribute("aria-selected", index === paletteIndex ? "true" : "false");
    const label = document.createElement("span");
    label.className = "palette-label";
    label.textContent = item.label;
    const key = document.createElement("span");
    key.className = "palette-key";
    key.textContent = displayShortcut(item.shortcut);
    row.append(label, key);
    row.addEventListener("mouseenter", () => {
      if (paletteIndex === index) return;
      paletteIndex = index;
      renderCommandPalette();
    });
    row.addEventListener("click", () => executePaletteItem(item));
    frag.append(row);
  });
  list.append(frag);
  list.querySelector(".palette-row.active")?.scrollIntoView({ block: "nearest" });
}

function showCommandPalette() {
  hideFileMenu();
  if (promptVisible() || formVisible() || commandPaletteVisible()) return;
  paletteItems = commandPaletteItems();
  paletteIndex = 0;
  $("palette-input").value = "";
  setModalOpen($("command-palette"), true);
  renderCommandPalette();
  queueMicrotask(() => $("palette-input").focus());
}

function hideCommandPalette() {
  setModalOpen($("command-palette"), false);
  focusEditor();
}

function movePalette(delta) {
  const visible = paletteItems.filter((item) => paletteMatches(item, $("palette-input").value));
  if (!visible.length) return;
  paletteIndex = (paletteIndex + delta + visible.length) % visible.length;
  renderCommandPalette();
}

function executePaletteItem(item) {
  if (!item) return;
  hideCommandPalette();
  queueMicrotask(() => runMenuAction(item.action));
}

function initCommandPalette() {
  $("palette-close").addEventListener("click", hideCommandPalette);
  $("command-palette").addEventListener("click", (e) => {
    if (e.target === $("command-palette")) hideCommandPalette();
  });
  $("palette-input").addEventListener("input", () => {
    paletteIndex = 0;
    renderCommandPalette();
  });
  $("palette-input").addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      hideCommandPalette();
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      movePalette(1);
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      movePalette(-1);
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      const visible = paletteItems.filter((item) => paletteMatches(item, $("palette-input").value));
      executePaletteItem(visible[paletteIndex]);
    }
  });
}

function rowsVisible() {
  const h = $("viewport").clientHeight - (state.settings && state.settings.ruler ? 18 : 0);
  return Math.max(1, Math.ceil(h / LINE_HEIGHT));
}

// ---- column ruler ----------------------------------------------------------

let _rulerKey = "";
function buildRuler() {
  const vp = $("viewport");
  if (!state.settings.ruler) {
    vp.classList.remove("has-ruler");
    return;
  }
  vp.classList.add("has-ruler");
  // Gutter width, measured from a visible row so ticks line up with the text.
  let gutterPx = 0;
  for (const row of pool) {
    if (row.style.display !== "none") {
      gutterPx = row.firstChild.getBoundingClientRect().width;
      break;
    }
  }
  const cw = charWidth();
  const inner = $("ruler-inner");
  const key = `${Math.round(gutterPx)}|${cw.toFixed(2)}`;
  if (key !== _rulerKey && gutterPx > 0) {
    _rulerKey = key;
    $("ruler-corner").style.width = `${gutterPx}px`;
    inner.textContent = "";
    for (let c = 10; c <= 500; c += 10) {
      const t = document.createElement("span");
      t.className = "rtick";
      t.style.left = `${gutterPx + c * cw}px`;
      t.textContent = String(c);
      inner.append(t);
    }
  }
  inner.style.transform = `translateX(${-$("content").scrollLeft}px)`;
}

function maxFirst() {
  return Math.max(0, state.total - rowsVisible());
}

// ---- data ------------------------------------------------------------------

function cachedLine(line) {
  const c = state.cache;
  const i = line - c.start;
  return i >= 0 && i < c.lines.length ? c.lines[i] : null;
}

function ensureData(start, count) {
  const need0 = start;
  const need1 = Math.min(state.total, start + count);
  const c = state.cache;
  if (c.lines.length && need0 >= c.start && need1 <= c.start + c.lines.length) {
    return; // already cached
  }
  const fstart = Math.max(0, start - PAD);
  const fcount = count + 2 * PAD;
  const token = ++state.loadToken;
  api(`/api/lines?start=${fstart}&count=${fcount}`)
    .then((res) => {
      if (token !== state.loadToken) return; // a newer request superseded us
      state.cache = { start: fstart, lines: res.lines };
      state.total = res.total;
      render();
    })
    .catch((e) => console.error("lines fetch failed", e));
}

async function lineByte(line, col = null) {
  try {
    const q = col == null ? "" : `&col=${Math.max(0, col)}`;
    const r = await api(`/api/linebyte?line=${Math.max(0, line)}${q}`);
    return r.byte ?? 0;
  } catch {
    return 0;
  }
}

// ---- rendering -------------------------------------------------------------

function ensurePool(count) {
  const content = $("content");
  while (pool.length < count) {
    const row = document.createElement("div");
    row.className = "row";
    const ln = document.createElement("span");
    ln.className = "ln";
    const tx = document.createElement("span");
    tx.className = "tx";
    row.append(ln, tx);
    // Mouse selection/caret is handled at the #content level (see initSelection),
    // so it works uniformly across the pooled rows and beyond the viewport.
    content.append(row);
    pool.push(row);
  }
}

function fillRow(row, line, rec, gutterWidth) {
  const ln = row.firstChild;
  const tx = row.lastChild;
  row.className = "row";
  row.dataset.line = String(line);
  ln.textContent = String(line + 1).padStart(gutterWidth, " ");
  tx.textContent = "";
  tx.classList.remove("pending");
  row.classList.toggle("inserted", !!rec?.inserted);
  if (rec == null) {
    tx.classList.add("pending");
    tx.textContent = "⋯";
  } else if (state.matcher) {
    appendHighlighted(tx, rec.text);
  } else if (state.settings.showWhitespace) {
    appendText(tx, rec.text, true);
  } else {
    tx.textContent = rec.text;
  }
  if (rec != null && state.settings.showWhitespace) appendEol(tx);
  // Hide the current-line highlight while a selection exists — the two
  // washes stack otherwise and the selection becomes hard to read.
  row.classList.toggle("active", line === state.activeLine && !hasSelection());
}

function fillEofRow(row) {
  row.className = "row eof";
  row.dataset.line = "-1";
  row.firstChild.textContent = "";
  const tx = row.lastChild;
  tx.className = "tx";
  tx.textContent = "[EOF]";
}

// Character width of the monospace content font, for a rough fallback only
// (real caret/selection geometry is measured from the actual glyphs below, so
// CJK, tabs and proportional fallbacks all line up).
let _charW = 0;
function charWidth() {
  if (_charW) return _charW;
  _charW = measureTextWidth("0".repeat(100)) / 100 || 8;
  return _charW;
}

// Measure the rendered pixel width of `str` in the content font. One reused,
// hidden probe kept inside #content so it inherits the exact font metrics.
let _measSpan = null;
function measureTextWidth(str) {
  if (!str) return 0;
  if (!_measSpan) {
    _measSpan = document.createElement("span");
    _measSpan.style.cssText =
      "position:absolute;visibility:hidden;white-space:pre;top:-9999px;left:0;pointer-events:none;";
    $("content").appendChild(_measSpan);
  }
  _measSpan.textContent = str;
  return _measSpan.getBoundingClientRect().width;
}

// Unicode-scalar view of a line's text (matches the backend's char columns).
function lineChars(line) {
  return Array.from(cachedLine(line)?.text ?? "");
}
function lineLen(line) {
  return lineChars(line).length;
}

// Pixel x (in #content coordinates, gutter included) of column `col` on `line`.
function caretX(line, col) {
  const cs = lineChars(line);
  const head = cs.slice(0, Math.max(0, Math.min(col, cs.length))).join("");
  return gutterPixels() + measureTextWidth(head);
}

// Inverse of caretX: nearest column boundary to pixel x (content coordinates).
function colFromX(line, x) {
  const cs = lineChars(line);
  const local = x - gutterPixels();
  if (local <= 0) return 0;
  const full = measureTextWidth(cs.join(""));
  if (local >= full) return cs.length;
  let lo = 0,
    hi = cs.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (measureTextWidth(cs.slice(0, mid).join("")) < local) lo = mid + 1;
    else hi = mid;
  }
  const wLo = measureTextWidth(cs.slice(0, lo).join(""));
  const wPrev = lo > 0 ? measureTextWidth(cs.slice(0, lo - 1).join("")) : 0;
  return local - wPrev < wLo - local ? lo - 1 : lo;
}

// ---- selection (multi-line, coordinate-based) ------------------------------

// Width in px of the line-number gutter, measured from a visible row.
function gutterPixels() {
  for (const row of pool) {
    if (row.style.display !== "none" && row.firstChild) {
      return row.firstChild.getBoundingClientRect().width;
    }
  }
  return 7 * charWidth() + 29; // fallback: 8 + 20 padding + 1 border
}

// Map a mouse event to a {line, col} position in the document.
function coordsFromEvent(e) {
  const content = $("content");
  const rect = content.getBoundingClientRect();
  const rowInView = Math.floor((e.clientY - rect.top) / LINE_HEIGHT);
  let line = state.first + Math.max(0, rowInView);
  line = Math.max(0, Math.min(line, Math.max(0, state.total - 1)));
  const x = e.clientX - rect.left + content.scrollLeft; // #content coordinates
  return { line, col: colFromX(line, x) };
}

// Normalized selection: { start, end } with start <= end, or null.
function selRange() {
  if (!state.sel) return null;
  const { anchor: a, head: h } = state.sel;
  const forward = a.line < h.line || (a.line === h.line && a.col <= h.col);
  const r = forward ? { start: a, end: h } : { start: h, end: a };
  r.rect = !!state.sel.rect;
  return r;
}

function rectRange() {
  if (!state.sel?.rect) return null;
  const a = state.sel.anchor;
  const h = state.sel.head;
  return {
    l0: Math.min(a.line, h.line),
    l1: Math.max(a.line, h.line),
    c0: Math.min(a.col, h.col),
    c1: Math.max(a.col, h.col),
  };
}

function hasSelection() {
  const rr = rectRange();
  if (rr) return rr.l0 !== rr.l1 || rr.c0 !== rr.c1;
  const r = selRange();
  return (!!r && !rangeEmpty(r)) || selectionRanges().length > 0;
}

// Like hasSelection(), but a zero-width rect (c0 == c1 across several lines)
// counts as empty: it selects no characters, so text-producing actions
// (copy / cut / save-selection) treat it as "no selection".
function hasTextSelection() {
  const rr = rectRange();
  if (rr) return rr.c0 !== rr.c1;
  return selectionRanges().length > 0;
}

function appendSelectionRect(layer, line, startCol, endCol, trailingNewline = false) {
  const left = caretX(line, startCol);
  const trail = trailingNewline ? charWidth() * 0.6 : 0;
  const width = caretX(line, endCol) - left + trail;
  const rect = document.createElement("div");
  rect.className = "selrect";
  rect.style.left = `${left}px`;
  rect.style.top = `${(line - state.first) * LINE_HEIGHT}px`;
  rect.style.width = `${Math.max(2, width)}px`;
  layer.append(rect);
}

function renderRangeSelection(layer, r) {
  if (!r || rangeEmpty(r)) return;
  const vis = rowsVisible() + OVERSCAN;
  const from = Math.max(r.start.line, state.first);
  const to = Math.min(r.end.line, state.first + vis);
  for (let line = from; line <= to; line++) {
    const startCol = line === r.start.line ? r.start.col : 0;
    const len = lineLen(line);
    // A line selected through its end extends a hair past the text so the
    // trailing newline reads as selected, like a normal editor.
    const endCol = line === r.end.line ? Math.min(r.end.col, len) : len;
    appendSelectionRect(layer, line, startCol, endCol, line !== r.end.line);
  }
}

function renderSelection() {
  const layer = $("sel-layer");
  layer.textContent = "";
  const rr = rectRange();
  if (rr) {
    const vis = rowsVisible() + OVERSCAN;
    const from = Math.max(rr.l0, state.first);
    const to = Math.min(rr.l1, state.first + vis);
    for (let line = from; line <= to; line++) {
      appendSelectionRect(layer, line, rr.c0, rr.c1);
    }
    return;
  }
  for (const r of selectionRanges()) renderRangeSelection(layer, r);
}

function initSelection() {
  const content = $("content");
  content.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    e.preventDefault(); // keep focus on the hidden input, not the div
    const p = coordsFromEvent(e);
    if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey) {
      // Ctrl+Click (Cmd+Click on mac): toggle an extra cursor at the point.
      if (state.stat?.open) toggleExtraCursorAt(p.line, p.col);
      focusEditor();
      return;
    }
    if (e.detail >= 3) {
      // Triple-click: select the whole line (newline included when possible).
      selectLineAt(p.line);
      return;
    }
    if (e.shiftKey) {
      const anchor = state.sel ? state.sel.anchor : { ...state.caret };
      state.sel = { anchor, head: p, rect: e.altKey };
      state.dragAnchor = anchor;
      state.dragMoved = true;
    } else {
      state.sel = null; // a bare click collapses any selection to a caret
      state.dragAnchor = p;
      state.dragMoved = false;
    }
    state.dragRect = e.altKey;
    setCaret(p.line, p.col);
    state.dragging = true;
    focusEditor();
  });

  window.addEventListener("mousemove", (e) => {
    if (!state.dragging) return;
    const p = coordsFromEvent(e);
    const a = state.dragAnchor;
    if (p.line !== a.line || p.col !== a.col) state.dragMoved = true;
    if (state.dragMoved) state.sel = { anchor: a, head: p, rect: state.dragRect };
    setCaret(p.line, p.col);
    // Auto-scroll when dragging past the top/bottom edge.
    const rect = content.getBoundingClientRect();
    if (e.clientY < rect.top + 14) setFirst(state.first - 2);
    else if (e.clientY > rect.bottom - 14) setFirst(state.first + 2);
    scheduleRender();
  });

  window.addEventListener("mouseup", () => {
    if (!state.dragging) return;
    state.dragging = false;
    state.dragRect = false;
    if (!state.dragMoved) state.sel = null; // plain click → just the caret
    scheduleRender();
  });

  // Double-click selects the run under the caret: a word, or (on symbols /
  // whitespace) the contiguous run of the same class, editor-style.
  content.addEventListener("dblclick", (e) => {
    e.preventDefault();
    const p = coordsFromEvent(e);
    const cs = lineChars(p.line);
    if (cs.length === 0) return;
    const classOf = (ch) => {
      if (ch == null) return null;
      if (/[\p{L}\p{N}_]/u.test(ch)) return "word";
      if (/\s/.test(ch)) return "space";
      return "punct";
    };
    // Prefer the char at the caret, else the one before it (click at run end).
    const pivot = cs[p.col] != null ? p.col : p.col - 1;
    const cls = classOf(cs[pivot]);
    if (cls == null) return;
    let a = pivot,
      b = pivot + 1;
    while (a > 0 && classOf(cs[a - 1]) === cls) a--;
    while (b < cs.length && classOf(cs[b]) === cls) b++;
    state.sel = { anchor: { line: p.line, col: a }, head: { line: p.line, col: b } };
    setCaret(p.line, b);
    focusEditor();
  });
}

// Select one whole line; the newline is included by anchoring the head at the
// start of the next line (matches VS Code's triple-click).
function selectLineAt(line) {
  if (state.total === 0) return;
  const l = Math.max(0, Math.min(line, state.total - 1));
  const hasNext = l + 1 < state.total;
  const head = hasNext ? { line: l + 1, col: 0 } : { line: l, col: lineLen(l) };
  state.sel = { anchor: { line: l, col: 0 }, head };
  setCaret(head.line, head.col);
  focusEditor();
  scheduleRender();
}

// ---- editor context menu ----------------------------------------------------

function ctxMenuVisible() {
  return !$("ctx-menu").classList.contains("hidden");
}

function hideCtxMenu() {
  $("ctx-menu").classList.add("hidden");
}

function posInsideSelection(p) {
  const rr = rectRange();
  if (rr) return p.line >= rr.l0 && p.line <= rr.l1 && p.col >= rr.c0 && p.col <= rr.c1;
  return selectionRanges().some((r) => {
    if (p.line < r.start.line || p.line > r.end.line) return false;
    if (p.line === r.start.line && p.col < r.start.col) return false;
    if (p.line === r.end.line && p.col > r.end.col) return false;
    return true;
  });
}

async function pasteFromClipboard() {
  try {
    const text = await navigator.clipboard.readText();
    if (text) pasteText(text);
  } catch {
    // Clipboard read needs a permission some webviews withhold; the keyboard
    // path (paste event on the hidden textarea) always works.
    flashCount("ここからは貼り付けできません — Ctrl+V を使ってください", "error");
  }
  focusEditor();
}

// Save the selected lines to a file server-side: streamed in batches, so the
// clipboard cap does not apply. Output matches what copy would produce.
async function saveSelectionToFile() {
  const rr = rectRange();
  const ranges = selectionRanges();
  const r = selRange() || (ranges.length === 1 ? ranges[0] : null);
  if ((!rr && !r) || !hasTextSelection()) {
    // A zero-width rect selects no characters — nothing to write out.
    flashCount("選択がありません", "error");
    return;
  }
  if (!rr && ranges.length > 1) {
    flashCount("複数選択はコピーまたは切り取りを使ってください", "error");
    return;
  }
  const total = rr ? rr.l1 - rr.l0 + 1 : r.end.line - r.start.line + 1;
  const base = state.stat?.path || "selection";
  const f = await askForm(
    "選択箇所をファイルに保存",
    [
      { id: "path", type: "text", label: "保存先パス", value: `${base}.selection.txt` },
      {
        id: "_hint",
        type: "hint",
        label: `選択中の ${commas(total)} 行を UTF-8 / LF で書き出します。コピーの行数上限 (${commas(MAX_COPY_LINES)} 行) はかかりません。`,
      },
    ],
    "保存",
  );
  if (!f || !f.path.trim()) return;
  const body = rr
    ? { path: f.path.trim(), rect: true, l0: rr.l0, c0: rr.c0, l1: rr.l1, c1: rr.c1 }
    : {
        path: f.path.trim(),
        rect: false,
        l0: r.start.line,
        c0: r.start.col,
        l1: r.end.line,
        c1: r.end.col,
      };
  showLoading("選択を書き出し中…");
  try {
    const res = await apiPost("/api/selection/save", body);
    flashCount(`選択 ${commas(res.lines)} 行を保存しました: ${displayPath(res.path)}`);
  } catch (e) {
    hideLoading();
    if (String(e.message || "").includes("既に存在")) {
      const overwrite = await askConfirm(
        "上書きの確認",
        `${displayPath(f.path.trim())} は既に存在します。上書きしますか?`,
        { okLabel: "上書き", danger: true },
      );
      if (overwrite) {
        showLoading("選択を書き出し中…");
        try {
          const res = await apiPost("/api/selection/save", { ...body, overwrite: true });
          flashCount(`選択 ${commas(res.lines)} 行を保存しました: ${displayPath(res.path)}`);
        } catch (e2) {
          flashCount("選択の保存エラー", "error");
          showMessage("選択の保存エラー", e2.message);
        }
      }
    } else {
      flashCount("選択の保存エラー", "error");
      showMessage("選択の保存エラー", e.message);
    }
  } finally {
    hideLoading();
  }
}

function runCtxAction(action) {
  hideCtxMenu();
  // Only the two context-menu-specific actions live here; everything else
  // (cut / copy / selectAll / find / replace / sortSave / diffFile /
  // splitFile) shares the menu dispatcher.
  let out;
  if (action === "paste") out = pasteFromClipboard();
  else if (action === "saveSelection") out = saveSelectionToFile();
  else out = runMenuAction(action);
  // A context-menu click leaves focus on the (now hidden) menu item, killing
  // keyboard input after cut/copy etc. Put focus back in the editor once the
  // action settles — unless it opened its own focus target (a modal, or the
  // find bar).
  return Promise.resolve(out).finally(() => {
    if (!anyModalOpen() && !state.findOpen) focusEditor();
  });
}

function initContextMenu() {
  const menu = $("ctx-menu");
  $("viewport").addEventListener("contextmenu", (e) => {
    e.preventDefault();
    if (!state.stat?.open || anyModalOpen()) return;
    // Right-click inside the selection keeps it as the action target;
    // outside it, the caret moves to the click point first (editor standard).
    const p = coordsFromEvent(e);
    if (!posInsideSelection(p)) {
      state.sel = null;
      setCaret(p.line, p.col);
      scheduleRender();
    }
    // Zero-width rect selections count as empty for the text actions.
    const hasSel = hasTextSelection();
    menu.querySelectorAll("[data-ctx]").forEach((el) => {
      const a = el.dataset.ctx;
      el.disabled = (a === "cut" || a === "copy" || a === "saveSelection") && !hasSel;
    });
    menu.classList.remove("hidden");
    const mw = menu.offsetWidth;
    const mh = menu.offsetHeight;
    menu.style.left = `${Math.max(4, Math.min(e.clientX, window.innerWidth - mw - 8))}px`;
    menu.style.top = `${Math.max(4, Math.min(e.clientY, window.innerHeight - mh - 8))}px`;
  });
  menu.querySelectorAll("[data-ctx]").forEach((el) => {
    el.addEventListener("click", () => runCtxAction(el.dataset.ctx));
  });
  document.addEventListener("pointerdown", (e) => {
    if (ctxMenuVisible() && !e.target.closest("#ctx-menu")) hideCtxMenu();
  });
}

function selectAll() {
  if (state.total === 0) return;
  const last = state.total - 1;
  state.sel = {
    anchor: { line: 0, col: 0 },
    head: { line: last, col: lineLen(last) },
  };
  setCaret(last, lineLen(last));
  focusEditor();
}

function rangeLineCount(r) {
  return r.end.line - r.start.line + 1;
}

function selectionLineCount(r = null) {
  const rr = rectRange();
  if (rr) return rr.l1 - rr.l0 + 1;
  if (r) return rangeLineCount(r);
  return selectionRanges().reduce((n, range) => n + rangeLineCount(range), 0);
}

async function selectedTextForRange(r, maxLines = MAX_COPY_LINES) {
  const count = Math.min(rangeLineCount(r), maxLines);
  const res = await api(`/api/lines?start=${r.start.line}&count=${count}`);
  // Columns are Unicode scalar counts (the server contract); slicing UTF-16
  // units here would split surrogate pairs (emoji etc.).
  const L = res.lines.map((x) => Array.from(x.text ?? ""));
  if (!L.length) return "";
  const complete = count >= rangeLineCount(r);
  if (L.length === 1) {
    const endCol = complete && r.start.line === r.end.line ? r.end.col : L[0].length;
    return L[0].slice(r.start.col, endCol).join("");
  }
  const out = [L[0].slice(r.start.col).join("")];
  for (let i = 1; i < L.length - 1; i++) out.push(L[i].join(""));
  if (L.length > 1) {
    const last = L[L.length - 1];
    out.push(last.slice(0, complete ? r.end.col : last.length).join(""));
  }
  return out.join("\n");
}

// Fetch the selected text (bounded) and join with newlines.
async function selectedText(r = null) {
  const rr = rectRange();
  if (rr) {
    const count = Math.min(rr.l1 - rr.l0 + 1, MAX_COPY_LINES);
    const res = await api(`/api/lines?start=${rr.l0}&count=${count}`);
    return res.lines
      .map((x) => {
        const chars = Array.from(x.text ?? "");
        return chars.slice(rr.c0, rr.c1).join("");
      })
      .join("\n");
  }
  const ranges = r ? [r] : selectionRanges();
  const out = [];
  let remaining = MAX_COPY_LINES;
  for (const range of ranges) {
    if (remaining <= 0) break;
    out.push(await selectedTextForRange(range, remaining));
    remaining -= Math.min(rangeLineCount(range), remaining);
  }
  return out.join("\n");
}

async function copyToClipboard(text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // Fallback for webviews without the async clipboard API.
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.cssText = "position:fixed;opacity:0;";
    document.body.append(ta);
    ta.select();
    try {
      document.execCommand("copy");
    } catch {
      /* give up silently */
    }
    ta.remove();
  }
}

async function copySelection() {
  if (!hasTextSelection()) return;
  try {
    const total = selectionLineCount();
    await copyToClipboard(await selectedText());
    if (total > MAX_COPY_LINES) {
      const multi = selectionRanges().length > 1;
      flashCount(
        multi
          ? `コピーは先頭 ${commas(MAX_COPY_LINES)} 行まで — 残り ${commas(total - MAX_COPY_LINES)} 行はコピーされていません`
          : `コピーは先頭 ${commas(MAX_COPY_LINES)} 行まで — 残り ${commas(total - MAX_COPY_LINES)} 行はコピーされていません。全体は右クリック→「選択箇所をファイルに保存」で書き出せます`,
        "error",
      );
    } else {
      flashCount("コピーしました");
    }
  } catch (e) {
    flashCount("コピーエラー", "error");
    console.error(e);
  }
}

function deleteSelection() {
  if (!hasSelection()) return;
  typeText(""); // replace the selection with nothing
}

async function cutSelection() {
  if (!hasTextSelection()) return;
  // Never delete more than what reached the clipboard: a capped copy followed
  // by a full delete would silently destroy data.
  const total = selectionLineCount();
  if (total > MAX_COPY_LINES) {
    const multi = selectionRanges().length > 1;
    flashCount(
      multi
        ? `切り取りは ${commas(MAX_COPY_LINES)} 行まで (選択は ${commas(total)} 行)。削除だけなら Delete キー`
        : `切り取りは ${commas(MAX_COPY_LINES)} 行まで (選択は ${commas(total)} 行)。全体を残すなら右クリック→「選択箇所をファイルに保存」、削除だけなら Delete キー`,
      "error",
    );
    return;
  }
  await copyToClipboard(await selectedText());
  deleteSelection();
}

// Append plain text to a row, rendering tabs as a faint arrow glyph when
// "空白・改行を表示" is on. The real \t stays in the DOM (the arrow is an
// absolutely-positioned ::before), so glyph widths — and therefore caret and
// selection geometry, which are measured from the logical text — never shift.
// `endsLine` marks the final piece of a line so its trailing ASCII spaces get
// a middle-dot overlay (only meaningful at a real line end).
function appendText(container, str, endsLine) {
  // Count the run of trailing half-width spaces so a line made purely of ASCII
  // spaces still gets dots (and so the fast path below is skipped when needed).
  let trail = 0;
  if (endsLine && state.settings.showWhitespace) {
    while (trail < str.length && str.charCodeAt(str.length - 1 - trail) === 0x20) trail++;
  }
  if (!state.settings.showWhitespace || (!/[\t　]/.test(str) && trail === 0)) {
    if (str) container.appendChild(document.createTextNode(str));
    return;
  }
  const body = trail ? str.slice(0, str.length - trail) : str;
  // Split keeping tabs and full-width (zenkaku) spaces as their own pieces so
  // each can be wrapped in a width-preserving overlay span.
  for (const p of body.split(/(\t|　)/)) {
    if (p === "") continue;
    if (p === "\t") {
      const t = document.createElement("span");
      t.className = "ws-tab";
      t.textContent = "\t";
      container.appendChild(t);
    } else if (p === "　") {
      const s = document.createElement("span");
      s.className = "ws-zsp";
      s.textContent = "　";
      container.appendChild(s);
    } else {
      container.appendChild(document.createTextNode(p));
    }
  }
  // Trailing spaces before the line end: one dot overlay per space, real space
  // kept so caret columns are unchanged.
  for (let i = 0; i < trail; i++) {
    const s = document.createElement("span");
    s.className = "ws-trail";
    s.textContent = " ";
    container.appendChild(s);
  }
}

// A faint end-of-line marker (↵) drawn after the text. It sits past every
// column, so it adds no width before any caret position.
function appendEol(container) {
  const el = document.createElement("span");
  el.className = "ws-eol";
  el.textContent = "↵";
  container.appendChild(el);
}

function appendHighlighted(container, text) {
  const re = state.matcher;
  re.lastIndex = 0;
  let last = 0;
  let m;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) {
      appendText(container, text.slice(last, m.index));
    }
    const mk = document.createElement("mark");
    appendText(mk, m[0]);
    container.appendChild(mk);
    last = m.index + m[0].length;
    if (m[0].length === 0) re.lastIndex++; // never stall on empty matches
  }
  if (last < text.length) {
    appendText(container, text.slice(last), true);
  }
}

function render() {
  renderQueued = false;
  const vis = rowsVisible();
  const count = vis + OVERSCAN;
  ensurePool(count);
  ensureData(state.first, count);

  const gutterWidth = Math.max(4, String(state.total).length);
  for (let r = 0; r < pool.length; r++) {
    const row = pool[r];
    const line = state.first + r;
    if (r >= count || line > state.total) {
      row.style.display = "none";
      continue;
    }
    row.style.display = "";
    if (line === state.total) {
      fillEofRow(row); // one marker row just past the last line
    } else {
      fillRow(row, line, cachedLine(line), gutterWidth);
    }
  }
  buildRuler();
  renderSelection();
  positionCaret();
  updateScrollbar();
  updateStatusPos();
}

function scheduleRender() {
  if (renderQueued) return;
  renderQueued = true;
  requestAnimationFrame(render);
}

function setFirst(line) {
  state.first = Math.min(Math.max(0, Math.round(line)), maxFirst());
  scheduleRender();
}

// ---- custom scrollbar ------------------------------------------------------

function updateScrollbar() {
  const vh = $("viewport").clientHeight;
  const thumb = $("vthumb");
  const vis = rowsVisible();
  const ratio = state.total > 0 ? Math.min(1, vis / state.total) : 1;
  const thumbH = Math.max(24, vh * ratio);
  const mf = maxFirst();
  const top = mf > 0 ? (vh - thumbH) * (state.first / mf) : 0;
  thumb.style.height = `${thumbH}px`;
  thumb.style.transform = `translateY(${top}px)`;
  renderSearchTicks(vh);
}

function renderSearchTicks(vh) {
  const ticks = $("vticks");
  if (!ticks) return;
  ticks.textContent = "";
  if (!state.query || !state.searchHits || state.searchHits.length === 0 || state.total <= 1)
    return;
  const frag = document.createDocumentFragment();
  const maxTicks = 700;
  const step = Math.max(1, Math.ceil(state.searchHits.length / maxTicks));
  const denom = Math.max(1, state.total - 1);
  for (let i = 0; i < state.searchHits.length; i += step) {
    const h = state.searchHits[i];
    if (typeof h.line !== "number") continue;
    const t = document.createElement("div");
    t.className = "vtick";
    if (state.lastMatch && h.byte === state.lastMatch.byte) t.classList.add("current");
    const y = Math.max(0, Math.min(vh - 3, (h.line / denom) * (vh - 3)));
    t.style.transform = `translateY(${y}px)`;
    frag.append(t);
  }
  ticks.append(frag);
}

function initScrollbar() {
  const bar = $("vscrollbar");
  const thumb = $("vthumb");
  let dragging = false;
  let startY = 0;
  let startFirst = 0;

  thumb.addEventListener("mousedown", (e) => {
    dragging = true;
    startY = e.clientY;
    startFirst = state.first;
    thumb.classList.add("drag");
    e.preventDefault();
    e.stopPropagation();
  });
  window.addEventListener("mousemove", (e) => {
    if (!dragging) return;
    const vh = $("viewport").clientHeight;
    const thumbH = thumb.getBoundingClientRect().height;
    const usable = Math.max(1, vh - thumbH);
    const dr = (e.clientY - startY) / usable;
    setFirst(startFirst + dr * maxFirst());
  });
  window.addEventListener("mouseup", () => {
    dragging = false;
    thumb.classList.remove("drag");
  });
  // Click on the track pages toward the click.
  bar.addEventListener("mousedown", (e) => {
    if (e.target === thumb) return;
    const rect = bar.getBoundingClientRect();
    const above = e.clientY < thumb.getBoundingClientRect().top;
    setFirst(state.first + (above ? -1 : 1) * rowsVisible());
    void rect;
  });
}

// ---- status bar ------------------------------------------------------------

function updateStatusMeta() {
  const s = state.stat;
  if (!s) {
    setAppTitle("Ayame Editor");
    return;
  }
  if (!s.open) {
    for (const id of ["st-enc", "st-eol", "st-edit", "st-index"]) {
      $(id).textContent = "—";
    }
    $("st-edit").title = "";
    $("st-index").title = "";
    $("st-pos").textContent = t("行 0");
    $("undo-edit").disabled = true;
    $("redo-edit").disabled = true;
    $("apply-theme").classList.add("hidden");
    $("apply-keymap").classList.add("hidden");
    setAppTitle("Ayame Editor");
    return;
  }
  const name = displayName(s.path);
  const dirtyMark = s.dirty ? "* " : "";
  setAppTitle(`${dirtyMark}${name} - Ayame Editor`);
  $("apply-theme").classList.toggle("hidden", !isThemeDoc(s.path));
  $("apply-keymap").classList.toggle("hidden", !isKeymapDoc(s.path));
  const lines = s.view_lines ?? s.lines;
  $("st-enc").textContent = s.bom_bytes > 0 ? `${enc(s.encoding)} (BOM)` : enc(s.encoding);
  $("st-eol").textContent = eol(s.eol);
  // Deliberately terse: the bar shows state, the tooltip carries the numbers.
  $("st-edit").textContent = s.dirty ? t("未保存") : t("保存済");
  $("st-edit").title = s.dirty
    ? translateText(
        `未保存の編集: +${commas(s.inserted_lines)} 行追加 / ~${commas(s.replaced_lines)} 行変更 / -${commas(s.deleted_lines)} 行削除`,
      )
    : t("すべての編集は保存済みです");
  $("undo-edit").disabled = !s.can_undo;
  $("redo-edit").disabled = !s.can_redo;
  $("st-index").textContent = t("索引OK");
  $("st-index").title =
    currentLocale() === "en"
      ? `${commas(lines)} lines / ${humanBytes(s.bytes)} / ${commas(s.checkpoints)} index checkpoints (${humanBytes(s.index_bytes)}, ${s.index_ms} ms)`
      : `${commas(lines)} 行 / ${humanBytes(s.bytes)} / 索引 ${commas(s.checkpoints)} 点 (${humanBytes(s.index_bytes)}, ${s.index_ms} ms)`;
  // Keep the active tab's unsaved-dot (and the tabs model behind
  // beforeunload / close confirmations) in sync as you type.
  const at = $("tabs").querySelector(".tab.active");
  if (at) at.classList.toggle("dirty", !!s.dirty);
  const activeTab = (state.tabs || []).find((t) => t.active);
  if (activeTab) activeTab.dirty = !!s.dirty;
}

function isUntitled(path) {
  return !!path && path.includes("ayame-untitled-");
}

function untitledName(path) {
  const base = pathBaseName(path);
  return base && base !== "untitled.txt" ? base : "untitled";
}

// Show a short, friendly name in the toolbar (basename, or "untitled").
function displayName(path) {
  if (!path) return "—";
  if (isUntitled(path)) return untitledName(path);
  const parts = path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || path;
}

function pathBaseName(path) {
  if (!path) return "";
  const clean = String(path).replace(/^\\\\\?\\/, "");
  const parts = clean.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || clean;
}

function pathDirName(path) {
  if (!path) return null;
  const clean = String(path).replace(/^\\\\\?\\/, "");
  const i = Math.max(clean.lastIndexOf("/"), clean.lastIndexOf("\\"));
  if (i < 0) return null;
  if (i === 0) return clean.slice(0, 1);
  return clean.slice(0, i);
}

function isAbsolutePath(path) {
  return /^(?:[A-Za-z]:[\\/]|\/|\\\\)/.test(String(path || ""));
}

function joinPath(dir, name) {
  const n = String(name || "").trim();
  if (!n) return "";
  if (isAbsolutePath(n)) return n;
  const d = String(dir || "").replace(/[\\/]+$/, "");
  if (!d) return n;
  const sep = d.includes("\\") && !d.includes("/") ? "\\" : "/";
  return `${d}${sep}${n}`;
}

function pathCrumbs(path) {
  const clean = String(path || "").replace(/^\\\\\?\\/, "");
  if (!clean) return [];
  const winDrive = clean.match(/^([A-Za-z]:)[\\/](.*)$/);
  if (winDrive) {
    const sep = "\\";
    let acc = `${winDrive[1]}${sep}`;
    const out = [{ label: winDrive[1], path: acc }];
    for (const part of winDrive[2].split(/[\\/]+/).filter(Boolean)) {
      acc = acc.endsWith(sep) ? `${acc}${part}` : `${acc}${sep}${part}`;
      out.push({ label: part, path: acc });
    }
    return out;
  }
  if (clean.startsWith("\\\\")) {
    const parts = clean.split(/[\\/]+/).filter(Boolean);
    if (parts.length < 2) return [{ label: clean, path: clean }];
    let acc = `\\\\${parts[0]}\\${parts[1]}`;
    const out = [{ label: `\\\\${parts[0]}\\${parts[1]}`, path: acc }];
    for (const part of parts.slice(2)) {
      acc = `${acc}\\${part}`;
      out.push({ label: part, path: acc });
    }
    return out;
  }
  if (clean.startsWith("/")) {
    let acc = "";
    const out = [{ label: "/", path: "/" }];
    for (const part of clean.split("/").filter(Boolean)) {
      acc += `/${part}`;
      out.push({ label: part, path: acc });
    }
    return out;
  }
  let acc = "";
  return clean
    .split(/[\\/]+/)
    .filter(Boolean)
    .map((part) => {
      acc = acc ? `${acc}/${part}` : part;
      return { label: part, path: acc };
    });
}

function enc(e) {
  // Keys match the core Encoding enum's kebab-case serialization (Utf8 → "utf8").
  return (
    {
      utf8: "UTF-8",
      "utf-8": "UTF-8",
      "shift-jis": "Shift_JIS",
      "euc-jp": "EUC-JP",
      ascii: "ASCII",
    }[e] || String(e)
  );
}
function eol(e) {
  return { lf: "LF", crlf: "CRLF", cr: "CR", mixed: "Mixed", none: "None" }[e] || String(e);
}

function updateStatusPos() {
  if (state.total === 0) {
    $("st-pos").textContent = t("行 0");
    return;
  }
  const pos = translateText(
    `行 ${commas(state.caret.line + 1)}, 列 ${commas(state.caret.col + 1)}`,
  );
  const n = state.extraCursors.length;
  $("st-pos").textContent = n ? translateText(`${pos} · ${n + 1} カーソル`) : pos;
}

// ---- search ----------------------------------------------------------------

function showFind(withReplace = false) {
  state.findOpen = true;
  document.documentElement.classList.add("find-open");
  if (withReplace) setReplaceRow(true);
  const f = withReplace && state.query ? $("replace-input") : $("find");
  queueMicrotask(() => {
    f.focus();
    f.select();
  });
}

function hideFind() {
  state.findOpen = false;
  document.documentElement.classList.remove("find-open");
  setReplaceRow(false);
}

function setReplaceRow(open) {
  state.replaceOpen = open;
  document.documentElement.classList.toggle("replace-open", open);
  $("find-expand").setAttribute("aria-expanded", open ? "true" : "false");
}

function buildMatcher() {
  state.regexError = false;
  $("find").parentElement.classList.remove("error");
  if (!state.query) {
    state.matcher = null;
    return;
  }
  const src = state.regex ? state.query : escapeRegExp(state.query);
  const flags = "g" + (state.ci ? "i" : "");
  try {
    // Mirror the server's whole-word rule so the highlight matches the count.
    state.matcher = state.word
      ? new RegExp(`(?<![\\p{L}\\p{N}_])(?:${src})(?![\\p{L}\\p{N}_])`, flags + "u")
      : new RegExp(src, flags);
    return;
  } catch {
    // The word/unicode wrapper can reject patterns the plain form accepts.
  }
  try {
    state.matcher = new RegExp(src, flags); // fall back: highlight the superset
  } catch {
    state.regexError = true;
    state.matcher = null; // invalid regex while typing — just don't highlight
    $("find").parentElement.classList.add("error");
  }
}

function qs() {
  return `q=${encodeURIComponent(state.query)}&regex=${state.regex}&ci=${state.ci}&word=${state.word}`;
}

async function findStep(dir) {
  if (!state.query) return;
  buildMatcher();
  if (state.regexError) {
    flashCount("正規表現エラー", "error");
    return;
  }
  saveSearchHistory(state.query);
  let from;
  if (dir === "next") {
    from = state.lastMatch
      ? state.lastMatch.byte + Math.max(1, state.lastMatch.len)
      : await lineByte(state.first);
  } else {
    from = state.lastMatch
      ? state.lastMatch.byte
      : await lineByte(Math.min(state.total, state.first + rowsVisible()));
  }
  try {
    const res = await api(`/api/find?dir=${dir}&from=${from}&${qs()}`);
    if (!res.hit) {
      flashCount("一致なし");
      return;
    }
    const h = res.hit;
    state.lastMatch = { byte: h.byte, len: h.byte_len };
    state.sel = null;
    setCaret(h.line, 0);
    revealLine(h.line);
    updateCount();
  } catch (e) {
    flashCount("エラー");
    console.error(e);
  }
}

function wordRangeAt(p) {
  const cs = lineChars(p.line);
  if (!cs.length) return null;
  let i = Math.min(p.col, cs.length - 1);
  if (!isWordChar(cs[i]) && p.col > 0 && isWordChar(cs[p.col - 1])) i = p.col - 1;
  if (!isWordChar(cs[i])) return null;
  let a = i;
  let b = i + 1;
  while (a > 0 && isWordChar(cs[a - 1])) a--;
  while (b < cs.length && isWordChar(cs[b])) b++;
  return { start: { line: p.line, col: a }, end: { line: p.line, col: b } };
}

function selectPrimaryRange(r) {
  state.sel = { anchor: clonePoint(r.start), head: clonePoint(r.end) };
  state.caret = clonePoint(r.end);
  state.activeLine = state.caret.line;
  state.goalCol = state.caret.col;
  state.editGen++;
  revealLine(state.caret.line);
  focusEditor();
  scheduleRender();
}

function promoteSelectionRange(r) {
  const nextKey = rangeKey(r);
  const old =
    state.sel && !state.sel.rect ? normalizedRange(state.sel.anchor, state.sel.head) : null;
  if (old && !rangeEmpty(old) && rangeKey(old) !== nextKey) {
    const exists = state.extraCursors.some((c) => {
      const cr = cursorSelectionRange(c);
      return cr && rangeKey(cr) === rangeKey(old);
    });
    if (!exists) {
      state.extraCursors.push({
        line: state.sel.head.line,
        col: state.sel.head.col,
        sel: cloneSelection(state.sel),
      });
    }
  }
  state.extraCursors = state.extraCursors.filter((c) => {
    const cr = cursorSelectionRange(c);
    return !cr || rangeKey(cr) !== nextKey;
  });
  selectPrimaryRange(r);
}

async function findNextOccurrenceRange(query, fromByte, existing) {
  const selected = new Set(existing.map(rangeKey));
  const charLen = Array.from(query).length;
  let from = fromByte;
  let wrapped = false;
  for (let i = 0; i < existing.length + 3; i++) {
    const params = new URLSearchParams({
      dir: "next",
      from: String(from),
      q: query,
      regex: "false",
      ci: "false",
      word: "false",
    });
    const res = await api(`/api/find?${params.toString()}`);
    if (!res.hit) {
      if (wrapped) return null;
      from = 0;
      wrapped = true;
      continue;
    }
    const h = res.hit;
    const r = {
      start: { line: h.line, col: h.column },
      end: { line: h.line, col: h.column + charLen },
    };
    if (!selected.has(rangeKey(r))) return r;
    from = h.byte + Math.max(1, h.byte_len);
  }
  return null;
}

async function selectNextOccurrence() {
  if (!state.stat?.open) return;
  if (rectRange()) {
    flashCount("矩形選択では Ctrl+D は使えません", "error");
    return;
  }
  let ranges = selectionRanges();
  if (!ranges.length) {
    const r = wordRangeAt(state.caret);
    if (!r) {
      flashCount("選択できる単語がありません");
      return;
    }
    selectPrimaryRange(r);
    return;
  }
  const query = await selectedTextForRange(ranges[0]);
  if (!query || query.includes("\n")) {
    flashCount("複数行選択では Ctrl+D は使えません", "error");
    return;
  }
  ranges = selectionRanges();
  const last = ranges[ranges.length - 1];
  const from = await lineByte(last.end.line, last.end.col);
  try {
    const next = await findNextOccurrenceRange(query, from, ranges);
    if (!next) {
      flashCount("次の一致はありません");
      return;
    }
    promoteSelectionRange(next);
  } catch (e) {
    flashCount("検索エラー", "error");
    console.error(e);
  }
}

async function updateCount() {
  if (!state.query) {
    $("find-count").textContent = "";
    state.searchHits = null;
    state.searchTruncated = false;
    return;
  }
  try {
    const res = await api(`/api/search?${qs()}&start=0&max=2000`);
    state.searchHits = res.hits;
    state.searchTruncated = res.truncated;
    updateFindCountLabel();
    scheduleRender();
  } catch {
    $("find-count").textContent = t("正規表現エラー");
    $("find").parentElement.classList.add("error");
    scheduleRender();
  }
}

function updateFindCountLabel() {
  const hits = state.searchHits;
  if (!hits || !state.query) {
    $("find-count").textContent = "";
    return;
  }
  const total = state.searchTruncated ? `${commas(hits.length)}+` : commas(hits.length);
  if (state.lastMatch) {
    const idx = hits.findIndex((h) => h.byte === state.lastMatch.byte);
    if (idx >= 0) {
      $("find-count").textContent = `${commas(idx + 1)} / ${total}`;
      return;
    }
  }
  $("find-count").textContent = translateText(`${total} 件`);
}

// Operation feedback goes to the always-visible status bar (aria-live), and is
// mirrored into the find bar when that is open. Errors stay a little longer.
let stMsgTimer = 0;
function flashCount(msg, kind = "") {
  msg = msg ? translateText(msg) : "";
  const isError = kind === "error";
  const el = $("st-msg");
  if (el) {
    el.textContent = msg || "";
    el.classList.toggle("error", isError);
    clearTimeout(stMsgTimer);
    if (msg) {
      stMsgTimer = setTimeout(
        () => {
          el.textContent = "";
          el.classList.remove("error");
        },
        isError ? 10000 : 6000,
      );
    }
  }
  if (state.findOpen) $("find-count").textContent = msg;
}

function loadSearchHistory() {
  try {
    const raw = JSON.parse(localStorage.getItem(SEARCH_HISTORY_KEY) || "[]");
    return Array.isArray(raw) ? raw.filter((x) => typeof x === "string").slice(0, 50) : [];
  } catch {
    return [];
  }
}

function saveSearchHistory(q) {
  const value = q.trim();
  if (!value) return;
  state.history = [value, ...state.history.filter((x) => x !== value)].slice(0, 50);
  state.historyIndex = -1;
  try {
    localStorage.setItem(SEARCH_HISTORY_KEY, JSON.stringify(state.history));
  } catch {
    // Ignore private-mode quota errors; search still works.
  }
}

function showSearchHistory(delta) {
  if (!state.history.length) return false;
  if (state.historyIndex < 0) {
    state.historyIndex = delta < 0 ? 0 : state.history.length - 1;
  } else {
    state.historyIndex = Math.min(
      state.history.length - 1,
      Math.max(0, state.historyIndex + delta),
    );
  }
  $("find").value = state.history[state.historyIndex];
  setQueryFromInput();
  return true;
}

function revealLine(line) {
  const vis = rowsVisible();
  if (line < state.first || line >= state.first + vis) {
    setFirst(line - Math.floor(vis / 3));
  } else {
    scheduleRender();
  }
  updateStatusPos();
}

async function refreshStat() {
  state.stat = await api("/api/stat");
  state.total = state.stat.view_lines ?? state.stat.lines;
  noteWalError(state.stat);
  updateStatusMeta();
}

// One-shot warning when the server had to disable its crash log (an I/O
// problem with the log never blocks editing; the stat response carries the
// reason exactly once, so showing it whenever present shows it once).
function noteWalError(stat) {
  if (stat && stat.wal_error) {
    flashCount(`自動保存ログが無効になりました: ${stat.wal_error}`, "error");
  }
}

function clearLineCache() {
  state.cache = { start: 0, lines: [] };
  state.loadToken++;
}

// ===========================================================================
//  Caret-based editing (Notepad / Sakura Editor style)
//
//  There is a single fluid caret (state.caret) you can place anywhere and type
//  across lines. Every mutation is expressed as one range replacement and sent
//  to the backend's /api/edit/replace_range, which records it as a single undo
//  step and returns the resulting caret. Edits are serialized through a small
//  queue so fast typing/IME stays in order; the visible window is re-fetched
//  after each edit (cheap over loopback) rather than mirrored optimistically.
// ===========================================================================

// Move the caret without touching the selection model wholesale (callers set
// state.sel themselves). Keeps the active-line highlight on the caret line.
function setCaret(line, col) {
  line = Math.max(0, Math.min(line, Math.max(0, state.total - 1)));
  col = Math.max(0, Math.min(col, lineLen(line)));
  state.caret = { line, col };
  state.activeLine = line;
  state.extraCursors = []; // any explicit caret placement collapses multi-cursor
  state.editGen++; // user-driven caret placement (click, search, open, …)
}

// Caret motion for the keyboard: `extend` grows the selection from its anchor.
function moveCaret(line, col, extend) {
  line = Math.max(0, Math.min(line, Math.max(0, state.total - 1)));
  col = Math.max(0, Math.min(col, lineLen(line)));
  if (extend) {
    const anchor = state.sel ? state.sel.anchor : { ...state.caret };
    state.sel = { anchor, head: { line, col } };
    state.extraCursors = []; // entering a selection collapses multi-cursor
  } else {
    state.sel = null;
  }
  state.caret = { line, col };
  state.activeLine = line;
  state.editGen++; // user-driven caret motion (arrows, Home/End, PageUp/Down)
  revealCaret();
  scheduleRender();
}

// ---- multi-cursor -----------------------------------------------------------

function clonePoint(p) {
  return { line: p.line, col: p.col };
}

function cloneSelection(sel) {
  return sel
    ? { anchor: clonePoint(sel.anchor), head: clonePoint(sel.head), rect: !!sel.rect }
    : null;
}

function normalizedRange(anchor, head) {
  const forward = anchor.line < head.line || (anchor.line === head.line && anchor.col <= head.col);
  return forward
    ? { start: clonePoint(anchor), end: clonePoint(head) }
    : { start: clonePoint(head), end: clonePoint(anchor) };
}

function rangeEmpty(r) {
  return r.start.line === r.end.line && r.start.col === r.end.col;
}

function rangeKey(r) {
  return `${r.start.line}:${r.start.col}:${r.end.line}:${r.end.col}`;
}

// Primary caret plus the extra cursors, deduped and in document order. The
// entry carrying `primary: true` mirrors state.caret.
function allCursors() {
  const out = [];
  const seen = new Set();
  const push = (c, primary) => {
    const k = `${c.line}:${c.col}`;
    if (seen.has(k)) return;
    seen.add(k);
    out.push({
      line: c.line,
      col: c.col,
      primary,
      sel: primary
        ? cloneSelection(state.sel && !state.sel.rect ? state.sel : null)
        : cloneSelection(c.sel),
    });
  };
  push(state.caret, true);
  for (const c of state.extraCursors) push(c, false);
  out.sort((a, b) => a.line - b.line || a.col - b.col);
  return out;
}

function cursorSelectionRange(c) {
  const sel = c.primary ? state.sel : c.sel;
  if (!sel || sel.rect) return null;
  const r = normalizedRange(sel.anchor, sel.head);
  return rangeEmpty(r) ? null : r;
}

function selectionRanges() {
  const ranges = [];
  const seen = new Set();
  const add = (r, primary = false) => {
    if (!r || rangeEmpty(r)) return;
    const key = rangeKey(r);
    if (seen.has(key)) return;
    seen.add(key);
    ranges.push({ ...r, primary });
  };
  const rr = rectRange();
  if (rr) return ranges;
  add(selRange(), true);
  for (const c of state.extraCursors) add(cursorSelectionRange(c), false);
  ranges.sort((a, b) => a.start.line - b.start.line || a.start.col - b.start.col);
  return ranges;
}

function hasCursorSelections() {
  if (state.sel && !state.sel.rect && selRange() && !rangeEmpty(selRange())) return true;
  return state.extraCursors.some((c) => {
    const r = cursorSelectionRange(c);
    return r && !rangeEmpty(r);
  });
}

function clearExtraCursors() {
  if (state.extraCursors.length) {
    state.extraCursors = [];
    scheduleRender();
  }
}

function clearExtraSelections() {
  for (const c of state.extraCursors) c.sel = null;
}

// Ctrl+Click: add a caret; clicking an existing extra caret removes it; the
// primary caret is left alone.
function toggleExtraCursorAt(line, col) {
  if (line === state.caret.line && col === state.caret.col) return;
  const i = state.extraCursors.findIndex((c) => c.line === line && c.col === col);
  if (i >= 0) state.extraCursors.splice(i, 1);
  else {
    state.sel = null;
    clearExtraSelections();
    state.extraCursors.push({ line, col, sel: null });
  }
  state.editGen++; // user cursor action: an in-flight edit must not clobber it
  scheduleRender();
}

function addExtraCursorAt(line, col) {
  if (line === state.caret.line && col === state.caret.col) return;
  if (state.extraCursors.some((c) => c.line === line && c.col === col)) return;
  state.sel = null;
  clearExtraSelections();
  state.extraCursors.push({ line, col, sel: null });
  state.editGen++; // user cursor action: an in-flight edit must not clobber it
  // Keep the newest cursor visible, like revealCaret does for the primary.
  const vis = rowsVisible();
  if (line < state.first) setFirst(line);
  else if (line >= state.first + vis) setFirst(line - vis + 1);
  focusEditor();
  scheduleRender();
}

// Ctrl+Alt+ArrowUp / ArrowDown: grow the cursor column one line beyond the
// topmost / bottommost cursor, preserving its column — clamped to the target
// line's REAL length, which may need a fetch when it is outside the cache.
async function addCursorAbove() {
  if (!state.stat?.open || state.total === 0) return;
  const top = allCursors()[0];
  if (top.line <= 0) return;
  const line = top.line - 1;
  const lens = await lineLensFor([line]);
  addExtraCursorAt(line, Math.min(top.col, lens.get(line) ?? 0));
}

async function addCursorBelow() {
  if (!state.stat?.open || state.total === 0) return;
  const cs = allCursors();
  const bottom = cs[cs.length - 1];
  if (bottom.line >= state.total - 1) return;
  const line = bottom.line + 1;
  const lens = await lineLensFor([line]);
  addExtraCursorAt(line, Math.min(bottom.col, lens.get(line) ?? 0));
}

function focusEditor() {
  const hi = $("hidden-input");
  if (hi && document.activeElement !== hi) hi.focus({ preventScroll: true });
  state.focused = true;
  scheduleRender();
}

// Bring the caret into view: scroll vertically (whole lines) and horizontally
// (#content is the horizontal scroll container).
function revealCaret() {
  const vis = rowsVisible();
  if (state.caret.line < state.first) {
    state.first = Math.min(state.caret.line, maxFirst());
  } else if (state.caret.line >= state.first + vis) {
    state.first = Math.min(Math.max(0, state.caret.line - vis + 1), maxFirst());
  }
  const content = $("content");
  const x = caretX(state.caret.line, state.caret.col);
  const view = content.clientWidth;
  const margin = 24;
  if (x - margin < content.scrollLeft) {
    content.scrollLeft = Math.max(0, x - margin);
  } else if (x + margin > content.scrollLeft + view) {
    content.scrollLeft = x + margin - view;
  }
}

// Position the caret element and the hidden IME input at the caret pixel.
function positionCaret() {
  const caretEl = $("caret");
  const hi = $("hidden-input");
  if (!caretEl || !hi) return;
  const vis = rowsVisible();
  const focusVisible = state.focused && !anyModalOpen() && !state.composing;
  positionExtraCarets(vis, focusVisible);
  const onScreen =
    !!state.stat?.open && state.caret.line >= state.first && state.caret.line < state.first + vis;
  const show = onScreen && state.focused && !anyModalOpen();
  caretEl.classList.toggle("on", show && !state.composing);
  if (!onScreen) return;
  const x = caretX(state.caret.line, state.caret.col);
  const y = (state.caret.line - state.first) * LINE_HEIGHT;
  caretEl.style.transform = `translate(${x}px, ${y}px)`;
  hi.style.transform = `translate(${x}px, ${y}px)`;
}

// Mirror #caret for every extra cursor: same transform math and the same
// visibility rules (focus, modal open, IME composition, offscreen). The divs
// live in a small pool inside #content and are trimmed when cursors go away.
const extraCaretPool = [];
function positionExtraCarets(vis, focusVisible) {
  const cursors = state.extraCursors;
  while (extraCaretPool.length < cursors.length) {
    const el = document.createElement("div");
    el.className = "caret extra";
    el.setAttribute("aria-hidden", "true");
    $("content").append(el);
    extraCaretPool.push(el);
  }
  while (extraCaretPool.length > cursors.length) extraCaretPool.pop().remove();
  for (let i = 0; i < cursors.length; i++) {
    const c = cursors[i];
    const el = extraCaretPool[i];
    const onScreen = !!state.stat?.open && c.line >= state.first && c.line < state.first + vis;
    el.classList.toggle("on", onScreen && focusVisible);
    if (onScreen) {
      const x = caretX(c.line, c.col);
      const y = (c.line - state.first) * LINE_HEIGHT;
      el.style.transform = `translate(${x}px, ${y}px)`;
    }
  }
}

function anyModalOpen() {
  return (
    promptVisible() ||
    formVisible() ||
    confirmVisible() ||
    settingsVisible() ||
    keymapVisible() ||
    commandPaletteVisible() ||
    diffVisible() ||
    grepVisible() ||
    openerVisible() ||
    convertVisible()
  );
}

// ---- the serialized edit queue --------------------------------------------

let editChain = Promise.resolve();
function editContext() {
  return { docGen: state.docGen };
}
function sameEditContext(ctx) {
  return !!state.stat?.open && state.docGen === ctx.docGen;
}
async function settleEditQueue() {
  await editChain;
}
function enqueueEdit(fn) {
  const ctx = editContext();
  editChain = editChain
    .then(async () => {
      if (!sameEditContext(ctx)) return null;
      if (savingCount > 0) {
        flashCount("保存中です — 完了後に入力します");
        await waitForSavingDone();
        if (!sameEditContext(ctx)) return null;
      }
      return fn();
    })
    .catch((e) => {
      flashCount("編集エラー");
      console.error(e);
    });
  return editChain;
}

// Re-fetch the padded window around state.first into the cache in one shot, so
// the text never blinks to the "⋯" pending placeholder between keystrokes.
async function reloadViewport() {
  const start = Math.max(0, state.first - PAD);
  const count = rowsVisible() + OVERSCAN + 2 * PAD;
  const res = await api(`/api/lines?start=${start}&count=${count}`);
  state.cache = { start, lines: res.lines };
  state.total = res.total;
  state.loadToken++; // cancel any in-flight ensureData for the old contents
}

// ---- 末尾に追従 (tail -f) ----------------------------------------------------
//
// While following, poll /api/tail/poll on an interval. On growth the server has
// already extended the line index in place over just the appended bytes, so the
// client only re-fetches the visible window. If the viewport was sitting at the
// bottom we auto-scroll to the new bottom (follow); if the user had scrolled up
// we keep their position and just let the scrollbar grow. Following pauses while
// the session has unsaved edits (the overlay would not line up) and stops on an
// external truncation/rotation, prompting the user to reopen.

const TAIL_POLL_MS = 1000;

// "At the bottom" = the last line sits within the current view; decides whether
// a growth should follow (auto-scroll) or merely extend the scrollbar in place.
function tailAtBottom() {
  return state.first >= maxFirst() - 2;
}

function updateTailUI() {
  const btn = $("st-tail");
  if (btn) btn.classList.toggle("on", state.followTail);
  const item = $("menu-toggle-tail");
  if (item) {
    item.classList.toggle("checked", state.followTail);
    item.setAttribute("aria-checked", String(state.followTail));
  }
}

function setFollowTail(on) {
  on = !!on && !!state.stat?.open;
  const was = state.followTail;
  state.followTail = on;
  if (state.tailTimer) {
    clearInterval(state.tailTimer);
    state.tailTimer = null;
  }
  if (on) {
    state.tailTimer = setInterval(pollTail, TAIL_POLL_MS);
    setFirst(maxFirst()); // jump to the tail so following starts from the end
    flashCount("末尾に追従中 (tail -f)");
    pollTail(); // don't wait a whole interval for the first check
  } else if (was) {
    flashCount("追従を停止しました");
  }
  updateTailUI();
}

async function pollTail() {
  if (!state.followTail || !state.stat?.open) return;
  if (savingCount > 0) return; // never poll mid-save
  let resp;
  try {
    resp = await apiPost("/api/tail/poll");
  } catch {
    return; // transient (e.g. a racing reload); try again next tick
  }
  if (!state.followTail) return; // toggled off during the round-trip
  if (!resp.open) {
    setFollowTail(false);
    return;
  }
  if (resp.changed) {
    // Truncated / rotated / replaced under us: stop and let the user reopen.
    setFollowTail(false);
    flashCount("ファイルが外部で変更されました — 追従を停止しました", "error");
    return;
  }
  // resp.pending_edits: growth seen but not followed (unsaved edits) — pause
  // silently and resume once the overlay is clear. resp.grew false: nothing new.
  if (resp.pending_edits || !resp.grew) return;
  // Auto-scroll only if we were already at the bottom; otherwise adopt the new
  // total so the scrollbar grows but the user's position is left untouched.
  const stick = tailAtBottom();
  state.total = resp.lines;
  if (stick) state.first = maxFirst();
  try {
    await reloadViewport();
    await refreshStat();
  } catch {
    return;
  }
  if (!state.followTail) return;
  if (stick) state.first = maxFirst();
  render();
}

// The range the next text insertion replaces: the selection, or the caret.
function replaceTarget() {
  const r = selRange();
  if (r) return { l0: r.start.line, c0: r.start.col, l1: r.end.line, c1: r.end.col };
  return { l0: state.caret.line, c0: state.caret.col, l1: state.caret.line, c1: state.caret.col };
}

// The one primitive every edit funnels through. The backend returns the
// authoritative post-edit caret (already column-clamped against the real
// document), so we commit it — and the new line count — to local state
// immediately, before any await that could reject. That keeps the caret/cache
// from going stale while the document advanced, and lets the next queued edit
// resolve its range against a correct caret even if the refresh below fails.
async function applyRange(l0, c0, l1, c1, text) {
  const ctx = editContext();
  const gen = state.editGen;
  const res = await apiPost("/api/edit/replace_range", { l0, c0, l1, c1, text });
  if (!sameEditContext(ctx)) return;
  state.total = res.stats.total_lines;
  if (state.editGen === gen) {
    // No user navigation happened during the round-trip: honor the edit caret.
    const line = Math.min(res.caret_line, Math.max(0, state.total - 1));
    state.sel = null;
    state.caret = { line, col: res.caret_col };
    state.activeLine = line;
    state.goalCol = res.caret_col;
  } else {
    // The user moved the caret mid-edit — keep their position, re-clamped to
    // the new line count (don't clobber it with the edit's caret).
    const line = Math.min(state.caret.line, Math.max(0, state.total - 1));
    state.caret = { line, col: state.caret.col };
    state.activeLine = line;
  }
  revealCaret(); // scroll so the caret line is covered by the reload below
  try {
    await reloadViewport();
    await refreshStat();
  } catch (e) {
    console.error("post-edit refresh failed", e);
    flashCount("再読込エラー");
  }
  if (!sameEditContext(ctx)) return;
  revealCaret();
  render();
}

async function applyRect(l0, l1, c0, c1, text) {
  const ctx = editContext();
  const gen = state.editGen;
  const res = await apiPost("/api/edit/replace_rect", { l0, l1, c0, c1, text });
  if (!sameEditContext(ctx)) return;
  state.total = res.stats.total_lines;
  if (state.editGen === gen) {
    const line = Math.min(res.caret_line, Math.max(0, state.total - 1));
    state.sel = null;
    state.caret = { line, col: res.caret_col };
    state.activeLine = line;
    state.goalCol = res.caret_col;
  }
  revealCaret();
  try {
    await reloadViewport();
    await refreshStat();
  } catch (e) {
    console.error("post-rect-edit refresh failed", e);
    flashCount("再読込エラー");
  }
  if (!sameEditContext(ctx)) return;
  revealCaret();
  render();
}

// Multi-cursor edits: send every cursor's replacement as ONE batch (the server
// records it as a single undo step) and adopt the returned carets. `cursors`
// is the sorted allCursors() list; `editOf[i]` is the index of cursor i's edit
// in `edits`, or -1 when the cursor contributed no edit (only possible at the
// document origin, whose position an edit batch cannot move).
async function applyBatch(edits, cursors, editOf) {
  const ctx = editContext();
  const gen = state.editGen;
  const res = await apiPost("/api/edit/replace_batch", { edits });
  if (!sameEditContext(ctx)) return;
  state.total = res.stats.total_lines;
  if (state.editGen === gen) {
    const clampLine = (l) => Math.min(l, Math.max(0, state.total - 1));
    const next = cursors.map((c, i) => {
      const k = editOf[i];
      const p = k >= 0 && res.carets?.[k] ? res.carets[k] : c;
      return { line: clampLine(p.line), col: p.col, primary: c.primary };
    });
    const primary = next.find((c) => c.primary) || next[0];
    state.sel = null;
    state.caret = { line: primary.line, col: primary.col };
    state.activeLine = primary.line;
    state.goalCol = primary.col;
    state.extraCursors = next
      .filter((c) => c !== primary)
      .map((c) => ({ line: c.line, col: c.col }));
  } else {
    // The user moved the caret mid-edit: keep their position, re-clamped to
    // the new line count (don't clobber it with the edit's caret).
    const line = Math.min(state.caret.line, Math.max(0, state.total - 1));
    state.caret = { line, col: state.caret.col };
    state.activeLine = line;
    if (state.extraCursors.length) {
      // Plain caret motion / cursor-adds keep the extras alive mid-flight.
      // Remap ONLY the cursors this batch owned (matched by their batch-start
      // position) onto their post-edit positions; cursors the user added or
      // removed while the batch was in flight are left exactly as they are.
      const clampLine = (l) => Math.min(l, Math.max(0, state.total - 1));
      const moved = new Map();
      cursors.forEach((c, i) => {
        const k = editOf[i];
        const p = k >= 0 && res.carets?.[k] ? res.carets[k] : c;
        moved.set(`${c.line}:${c.col}`, { line: clampLine(p.line), col: p.col });
      });
      const seen = new Set();
      state.extraCursors = state.extraCursors.flatMap((c) => {
        const next = moved.get(`${c.line}:${c.col}`) || { line: clampLine(c.line), col: c.col };
        const key = `${next.line}:${next.col}`;
        if (seen.has(key)) return []; // two cursors landing together collapse
        seen.add(key);
        return [next];
      });
    }
  }
  revealCaret();
  try {
    await reloadViewport();
    await refreshStat();
  } catch (e) {
    console.error("post-batch-edit refresh failed", e);
    flashCount("再読込エラー");
  }
  if (!sameEditContext(ctx)) return;
  revealCaret();
  render();
}

// One same-shaped insertion per cursor. `textFor(i)` is the string inserted at
// cursor i (document order) — a constant for typing, per-line for paste.
function multiInsert(cursors, textFor) {
  const edits = cursors.map((c, i) => ({
    l0: c.line,
    c0: c.col,
    l1: c.line,
    c1: c.col,
    text: textFor(i),
  }));
  return applyBatch(
    edits,
    cursors,
    cursors.map((_, i) => i),
  );
}

function cursorReplaceRange(c) {
  const r = cursorSelectionRange(c);
  if (r) return { l0: r.start.line, c0: r.start.col, l1: r.end.line, c1: r.end.col };
  return { l0: c.line, c0: c.col, l1: c.line, c1: c.col };
}

function multiReplace(cursors, textFor) {
  const edits = cursors.map((c, i) => ({ ...cursorReplaceRange(c), text: textFor(i) }));
  return applyBatch(
    edits,
    cursors,
    cursors.map((_, i) => i),
  );
}

// Insert (or replace the selection with) `text`, which may contain newlines.
// The target range is resolved *inside* the queued step, so a burst of
// keystrokes each sees the caret left by the previous edit (never a stale one).
// A 0-line document (an empty file) is editable too: replaceTarget yields the
// (0,0)..(0,0) origin range, which the backend accepts to seed the first line.
function typeText(text) {
  if (!state.stat?.open) return;
  enqueueEdit(() => {
    if (state.extraCursors.length) {
      // Multi-cursor: the same text goes in at every caret, or replaces each
      // cursor's selection, as one undo step.
      const cursors = allCursors();
      return hasCursorSelections()
        ? multiReplace(cursors, () => text)
        : multiInsert(cursors, () => text);
    }
    const rr = rectRange();
    if (rr) {
      return applyRect(rr.l0, rr.l1, rr.c0, rr.c1, text);
    }
    const t = replaceTarget();
    return applyRange(t.l0, t.c0, t.l1, t.c1, text);
  });
}

function insertNewline() {
  typeText("\n");
}

// Decoded length (in Unicode scalars) of each requested line, as a Map. Lines
// inside the local cache are read from it; anything else is fetched, because
// lineLen() silently reads 0 for uncached lines — and multi-cursor edits can
// reference lines far outside the viewport±PAD cache window, where a guessed 0
// would turn a delete edge into "delete the whole line". Lines whose length
// cannot be resolved are absent from the map; callers must skip those edits.
async function lineLensFor(lineNumbers) {
  const out = new Map();
  const missing = new Set();
  for (const l of lineNumbers) {
    if (l < 0 || l >= state.total || out.has(l) || missing.has(l)) continue;
    const rec = cachedLine(l);
    if (rec != null) out.set(l, Array.from(rec.text ?? "").length);
    else missing.add(l);
  }
  await Promise.all(
    [...missing].map(async (l) => {
      try {
        const res = await api(`/api/lines?start=${l}&count=1`);
        const text = res.lines?.[0]?.text;
        if (text != null) out.set(l, Array.from(text).length);
      } catch {
        // Leave the line out: the caller drops that cursor's edit, never guesses.
      }
    }),
  );
  return out;
}

// The shared "a selection is active" arm of every delete command: remove the
// rect or range selection as one edit. Returns null when nothing is selected
// (callers then handle their caret-relative case). Call inside enqueueEdit.
function deleteSelectionEdit() {
  if (!hasSelection()) return null;
  if (state.extraCursors.length && hasCursorSelections()) {
    return multiReplace(allCursors(), () => "");
  }
  const rr = rectRange();
  if (rr) return applyRect(rr.l0, rr.l1, rr.c0, rr.c1, "");
  const t = replaceTarget();
  return applyRange(t.l0, t.c0, t.l1, t.c1, "");
}

function backspace() {
  enqueueEdit(async () => {
    const del = deleteSelectionEdit();
    if (del) return del;
    if (state.extraCursors.length) {
      // Per cursor: delete one char before the caret (line-join at col 0).
      // allCursors() dedupes positions, so ranges may touch but never overlap;
      // a cursor at the document origin contributes no edit. Join edges need
      // the previous line's REAL length, which may live outside the cache.
      const cursors = allCursors();
      const lens = await lineLensFor(
        cursors.filter((c) => c.col === 0 && c.line > 0).map((c) => c.line - 1),
      );
      const edits = [];
      const editOf = cursors.map((c) => {
        if (c.col > 0) {
          edits.push({ l0: c.line, c0: c.col - 1, l1: c.line, c1: c.col, text: "" });
        } else if (c.line > 0 && lens.has(c.line - 1)) {
          edits.push({ l0: c.line - 1, c0: lens.get(c.line - 1), l1: c.line, c1: 0, text: "" });
        } else {
          return -1; // document origin, or an unresolvable line length
        }
        return edits.length - 1;
      });
      if (!edits.length) return null;
      return applyBatch(edits, cursors, editOf);
    }
    const { line, col } = state.caret;
    if (col > 0) return applyRange(line, col - 1, line, col, "");
    if (line > 0) {
      const lens = await lineLensFor([line - 1]);
      if (!lens.has(line - 1)) return null;
      return applyRange(line - 1, lens.get(line - 1), line, 0, "");
    }
    return null;
  });
}

function forwardDelete() {
  enqueueEdit(async () => {
    const del = deleteSelectionEdit();
    if (del) return del;
    if (state.extraCursors.length) {
      // Per cursor: delete one char after the caret (line-join at EOL). Same
      // dedupe rule as backspace; the very end of the document yields no edit.
      // The char-vs-join decision needs each cursor line's REAL length.
      const cursors = allCursors();
      const lens = await lineLensFor(cursors.map((c) => c.line));
      const edits = [];
      const editOf = cursors.map((c) => {
        if (!lens.has(c.line)) return -1; // unresolvable length: never guess 0
        if (c.col < lens.get(c.line)) {
          edits.push({ l0: c.line, c0: c.col, l1: c.line, c1: c.col + 1, text: "" });
        } else if (c.line < state.total - 1) {
          edits.push({ l0: c.line, c0: c.col, l1: c.line + 1, c1: 0, text: "" });
        } else {
          return -1;
        }
        return edits.length - 1;
      });
      if (!edits.length) return null;
      return applyBatch(edits, cursors, editOf);
    }
    const { line, col } = state.caret;
    const lens = await lineLensFor([line]);
    if (!lens.has(line)) return null;
    if (col < lens.get(line)) return applyRange(line, col, line, col + 1, "");
    if (line < state.total - 1) return applyRange(line, col, line + 1, 0, "");
    return null;
  });
}

function pasteText(raw) {
  const text = raw.replace(/\r\n?/g, "\n");
  if (!state.extraCursors.length) {
    typeText(text);
    return;
  }
  if (!state.stat?.open) return;
  enqueueEdit(() => {
    if (!state.extraCursors.length) {
      // Collapsed while the paste was queued: normal single-caret insert.
      const t = replaceTarget();
      return applyRange(t.l0, t.c0, t.l1, t.c1, text);
    }
    // VS Code rule: N clipboard lines onto N cursors paste line i at cursor i
    // (document order); any other shape inserts the whole text at every caret.
    const cursors = allCursors();
    const lines = text.split("\n");
    const perCursor = lines.length === cursors.length ? lines : null;
    const textFor = (i) => (perCursor ? perCursor[i] : text);
    return hasCursorSelections() ? multiReplace(cursors, textFor) : multiInsert(cursors, textFor);
  });
}

// Shared tail of every save-as-style save (名前を付けて保存 / クイックメモ保存):
// the current tab becomes the saved file (the server swaps the active tab's
// document to the new path), exactly like every desktop editor — no leftover
// untitled tab, no extra tab for the saved file. Also remembers the folder as
// 前回の保存先 for the next untitled buffer.
async function finishSaveAs(res) {
  if (res.switched) {
    // Same tab, new document identity: refresh in place, keep the caret.
    state.docGen++;
    state.editGen++;
    clearLineCache();
    await refreshStat();
    await reloadViewport();
    setCaret(Math.min(state.caret.line, Math.max(0, state.total - 1)), state.caret.col);
    render();
    refreshTabs();
    updateTreeActive();
  } else {
    // The workspace changed while saving (rare): fall back to focusing the
    // saved file — the server dedupes, so this never duplicates a tab.
    onDocumentOpened(await apiPost("/api/open", { path: res.path }));
  }
  rememberSaveDir(res.path);
  flashCount(`保存しました: ${displayPath(res.path)}`);
}

// 前回の保存先: persisted so untitled buffers suggest the folder you last
// saved into (and so the memo flow survives restarts).
function rememberSaveDir(path) {
  const dir = pathDirName(displayPath(path));
  if (!dir) return;
  state.settings = { ...state.settings, lastSaveDir: dir };
  saveSettings(state.settings);
}

async function saveCopy() {
  if (savingCount > 0) {
    flashCount("保存中です — 完了までお待ちください");
    return;
  }
  await settleEditQueue();
  const target = await showSaveDialog("名前を付けて保存", suggestedSaveAsPath());
  if (!target) return;
  savingCount++;
  setSavingUI();
  try {
    const res = await apiPost("/api/edit/save", { ...target, switch_to_saved: true });
    await finishSaveAs(res);
  } catch (e) {
    flashCount("保存エラー", "error");
    showMessage("保存エラー", e.message);
  } finally {
    savingCount--;
    setSavingUI();
    retryPendingNativeClose();
  }
}

// 名前を付けて保存 opens on the current file's own folder and name (Windows
// standard); untitled buffers suggest the expanded メモの名前 template inside
// 前回の保存先 when one is known (else the dialog falls back as before).
function suggestedSaveAsPath() {
  const p = state.stat?.path || "";
  if (p && !isUntitled(p)) return p;
  const name =
    expandNameTemplate(state.settings.memoName || DEFAULT_SETTINGS.memoName).trim() ||
    "untitled.txt";
  const dir = (state.settings.lastSaveDir || "").trim();
  return dir ? joinPath(dir, name) : name;
}

// Expand the メモの名前 template tokens from the current local time.
// {date}=YYYYMMDD, {time}=HHMMSS, {datetime}=YYYYMMDD-HHMMSS; note {mm} is the
// month and {MM} the minutes (all zero-padded). The parameter is deliberately
// not named `t` — that is the i18n helper.
function expandNameTemplate(tpl) {
  const d = new Date();
  const p2 = (n) => String(n).padStart(2, "0");
  const yyyy = String(d.getFullYear()).padStart(4, "0");
  const map = {
    "{yyyy}": yyyy,
    "{yy}": yyyy.slice(-2),
    "{mm}": p2(d.getMonth() + 1),
    "{dd}": p2(d.getDate()),
    "{HH}": p2(d.getHours()),
    "{MM}": p2(d.getMinutes()),
    "{ss}": p2(d.getSeconds()),
  };
  map["{date}"] = `${yyyy}${map["{mm}"]}${map["{dd}"]}`;
  map["{time}"] = `${map["{HH}"]}${map["{MM}"]}${map["{ss}"]}`;
  map["{datetime}"] = `${map["{date}"]}-${map["{time}"]}`;
  return String(tpl || "").replace(
    /\{(?:yyyy|yy|mm|dd|HH|MM|ss|date|time|datetime)\}/g,
    (m) => map[m],
  );
}

// "memo.txt" taken → "memo-2.txt", "memo-3.txt", … (before the extension).
// null after 99 collisions — something is off, let the dialog decide.
function freeMemoName(name, taken) {
  if (!taken.has(name)) return name;
  const dot = name.lastIndexOf(".");
  const stem = dot > 0 ? name.slice(0, dot) : name;
  const ext = dot > 0 ? name.slice(dot) : "";
  for (let i = 2; i <= 99; i++) {
    const cand = `${stem}-${i}${ext}`;
    if (!taken.has(cand)) return cand;
  }
  return null;
}

// クイックメモ保存: with 設定 → メモの保存先 set, an untitled buffer saves
// straight into that folder under the expanded メモの名前 (auto-numbered on
// collision) — Ctrl+S and done, no dialog. Returns true when the save was
// handled (or failed with its own error UI); false falls back to the dialog.
async function quickMemoSave(memoDir) {
  const name =
    expandNameTemplate(state.settings.memoName || DEFAULT_SETTINGS.memoName).trim() || "memo.txt";
  let listing;
  try {
    listing = await api(`/api/browse?dir=${encodeURIComponent(memoDir)}`);
  } catch {
    flashCount(`メモの保存先を開けません: ${memoDir}`, "error");
    return false; // dir missing / typo → the dialog still gets the memo saved
  }
  const taken = new Set((listing.entries || []).filter((e) => !e.is_dir).map((e) => e.name));
  const free = freeMemoName(name, taken);
  if (!free) return false;
  savingCount++;
  setSavingUI();
  try {
    const res = await apiPost("/api/edit/save", {
      path: joinPath(listing.dir, free),
      switch_to_saved: true,
    });
    await finishSaveAs(res);
    return true;
  } catch (e) {
    flashCount("保存エラー", "error");
    showMessage("保存エラー", e.message);
    return true; // reported here — don't surprise with a dialog on top
  } finally {
    savingCount--;
    setSavingUI();
    retryPendingNativeClose();
  }
}

async function saveFile() {
  if (!state.stat?.open) return;
  if (savingCount > 0) {
    flashCount("保存中です — 完了までお待ちください");
    return;
  }
  await settleEditQueue();
  if (isUntitled(state.stat.path)) {
    const memoDir = (state.settings.memoDir || "").trim();
    if (memoDir && (await quickMemoSave(memoDir))) return;
    await saveCopy();
    return;
  }
  savingCount++;
  setSavingUI();
  try {
    const res = await apiPost("/api/edit/save", { overwrite: true });
    clearLineCache();
    await refreshStat();
    await reloadViewport();
    render();
    flashCount(`保存しました: ${displayPath(res.path)}`);
  } catch (e) {
    flashCount("保存エラー", "error");
    showMessage("保存エラー", e.message);
  } finally {
    savingCount--;
    setSavingUI();
    retryPendingNativeClose();
  }
}

// ---- 変換して保存 (文字コード / 改行コード) --------------------------------

function convertVisible() {
  return !$("convert-modal").classList.contains("hidden");
}

function showConvert() {
  if (!state.stat?.open) return;
  if (isUntitled(state.stat.path)) {
    // Nothing on disk to convert yet — save it first.
    flashCount("先に保存してください");
    saveCopy();
    return;
  }
  hideFileMenu();
  // Prefill the pickers with the file's current encoding / line ending. The
  // stat strings are the core enum's kebab-case (Utf8 → "utf8"); map them onto
  // the select's option values.
  const encOpt = { utf8: "utf-8", "utf-8": "utf-8", "shift-jis": "shift-jis", "euc-jp": "euc-jp" };
  $("convert-enc").value = encOpt[state.stat.encoding] || "utf-8";
  const l = state.stat.eol;
  $("convert-eol").value = ["lf", "crlf", "cr"].includes(l) ? l : "lf";
  // Prefill 「BOMを付ける」 from the file's current BOM, then gray it out unless
  // the chosen 文字コード is UTF-8 (a UTF-8 BOM is the only one we can write).
  $("convert-bom").checked = state.stat.bom_bytes > 0;
  syncConvertBom();
  setModalOpen($("convert-modal"), true);
  queueMicrotask(() => $("convert-enc").focus());
}

// The BOM option only applies to UTF-8 output; disable it otherwise.
function syncConvertBom() {
  const isUtf8 = $("convert-enc").value === "utf-8";
  $("convert-bom").disabled = !isUtf8;
  $("convert-bom-row").classList.toggle("disabled", !isUtf8);
}

function hideConvert() {
  setModalOpen($("convert-modal"), false);
  focusEditor();
}

// Rewrite the current file in the chosen 文字コード / 改行コード. Every line is
// re-encoded server-side, so the active tab reloads the converted bytes.
async function convertSave(encoding, lineEnding, bom) {
  if (!state.stat?.open) return;
  if (savingCount > 0) {
    flashCount("保存中です — 完了までお待ちください");
    return;
  }
  await settleEditQueue();
  savingCount++;
  setSavingUI();
  try {
    const res = await apiPost("/api/edit/save", {
      overwrite: true,
      encoding,
      eol: lineEnding,
      bom,
    });
    if (res.switched) {
      state.docGen++;
      state.editGen++;
      clearLineCache();
      await refreshStat();
      await reloadViewport();
      setCaret(Math.min(state.caret.line, Math.max(0, state.total - 1)), state.caret.col);
      render();
      refreshTabs();
    } else {
      onDocumentOpened(await apiPost("/api/open", { path: res.path }));
    }
    flashCount(`${enc(encoding)} / ${eol(lineEnding)} で保存しました`);
  } catch (e) {
    flashCount("変換保存エラー", "error");
    showMessage("変換して保存", e.message);
  } finally {
    savingCount--;
    setSavingUI();
    retryPendingNativeClose();
  }
}

// Re-read the current file forcing a 文字コード — recovery when auto-detection
// guessed wrong and the text shows mojibake. Non-destructive to the file, but
// drops any unsaved edits, so confirm first.
async function reopenWithEncoding(encoding) {
  if (!state.stat?.open) return;
  if (isUntitled(state.stat.path)) {
    flashCount("保存されたファイルがありません");
    return;
  }
  if (state.stat.dirty) {
    const ok = await askConfirm("開き直す", "未保存の編集を破棄して開き直しますか?", {
      okLabel: "破棄して開き直す",
      danger: true,
    });
    if (!ok) return;
  }
  await settleEditQueue();
  try {
    await apiPost("/api/reopen_encoding", { encoding });
    state.docGen++;
    state.editGen++;
    clearLineCache();
    await refreshStat();
    await reloadViewport();
    setCaret(Math.min(state.caret.line, Math.max(0, state.total - 1)), state.caret.col);
    render();
    refreshTabs();
    flashCount(`${enc(encoding)} で開き直しました`);
  } catch (e) {
    flashCount("開き直しエラー", "error");
    showMessage("開き直す", e.message);
  }
}

async function undoEdit() {
  enqueueEdit(async () => {
    await apiPost("/api/edit/undo", {});
    state.sel = null;
    state.extraCursors = []; // a multi-cursor batch undoes as one step

    await refreshStat();
    await reloadViewport();
    setCaret(state.caret.line, state.caret.col); // re-clamp into the new bounds
    revealCaret();
    render();
  });
}

async function redoEdit() {
  enqueueEdit(async () => {
    await apiPost("/api/edit/redo", {});
    state.sel = null;
    state.extraCursors = []; // a multi-cursor batch redoes as one step

    await refreshStat();
    await reloadViewport();
    setCaret(state.caret.line, state.caret.col);
    revealCaret();
    render();
  });
}

// ソート: sorts the current tab in place — unsaved edits included — and
// overwrites the original file on disk. All options sit in one form.
async function sortSave() {
  if (!state.stat?.open) return;
  const f = await askForm(
    "ソート",
    [
      {
        id: "key",
        type: "text",
        label: "キー列 (1始まり)",
        placeholder: "空なら行全体で比較",
        title: "空欄: 行全体を文字列として比較 / 数字: 区切り文字で分けたその列をキーとして比較",
      },
      {
        id: "delim",
        type: "text",
        label: "区切り文字",
        value: ",",
        placeholder: ",",
        title: "キー列を使うときの列の区切り (例: , やタブ)",
      },
      {
        id: "numeric",
        type: "check",
        label: "数値として比較する",
        value: false,
        title: "10 と 9 を文字列でなく数値の大小で並べます",
      },
      {
        id: "order",
        type: "select",
        label: "並び順",
        options: [
          ["asc", "昇順 (A→Z, 小→大)"],
          ["desc", "降順 (Z→A, 大→小)"],
        ],
      },
      {
        id: "_hint",
        type: "hint",
        label:
          "現在のファイルを並び替えて上書きします。未保存の編集も含めて並び替えます。この操作は元に戻せません。",
      },
    ],
    "ソート",
  );
  if (!f) return;
  const keyText = String(f.key || "").trim();
  const key = keyText === "" ? null : Number(keyText);
  if (keyText !== "" && (!Number.isInteger(key) || key < 1)) {
    flashCount("キー列は 1 以上の整数で指定してください", "error");
    return;
  }
  showLoading("ソート実行中…");
  try {
    await apiPost("/api/sort/save", {
      in_place: true,
      key,
      numeric: !!f.numeric,
      reverse: f.order === "desc",
      delim: key != null && f.delim ? f.delim : null,
    });
    state.sel = null;
    state.extraCursors = [];
    setCaret(0, 0);
    clearLineCache();
    await refreshStat();
    await reloadViewport();
    render();
    flashCount("ソートして上書きしました");
  } catch (e) {
    flashCount("ソートエラー", "error");
    showMessage("ソートエラー", e.message);
  } finally {
    hideLoading();
  }
}

// ファイル分割: writes the current document (unsaved edits included) out as
// multiple files of at most N lines each; the original file is untouched.
async function splitFile() {
  if (!state.stat?.open) return;
  const f = await askForm(
    "ファイルを分割",
    [
      { id: "lines", type: "text", label: "1ファイルあたりの行数", value: "1000000" },
      {
        id: "dir",
        type: "text",
        label: "出力先フォルダ",
        value: "",
        placeholder: "空なら元ファイルと同じ場所",
      },
      {
        id: "_hint",
        type: "hint",
        label:
          "現在のファイルを指定行数ごとに分割して書き出します。未保存の編集も含まれます。元のファイルは変更されません。",
      },
    ],
    "分割",
  );
  if (!f) return;
  const lines = Number(String(f.lines || "").trim());
  if (!Number.isInteger(lines) || lines < 1) {
    flashCount("行数は 1 以上の整数で指定してください", "error");
    return;
  }
  showLoading("分割実行中…");
  try {
    const dir = String(f.dir || "").trim();
    const res = await apiPost("/api/split/save", { lines, dir: dir || null });
    flashCount(`${res.count} 個に分割しました: 最初のファイル ${displayPath(res.files[0])}`);
  } catch (e) {
    flashCount("分割エラー", "error");
    showMessage("分割エラー", e.message);
  } finally {
    hideLoading();
  }
}

// ---- in-editor replace (the find bar's replace row) -------------------------
//
// Replacements are ordinary edit-session batches (/api/edit/replace_batch), so
// they show up in the view immediately and undo like any other edit — no
// separate output file. Matching lines come from the server (the same engine
// the counter uses); the replacement text per line is computed with the same
// JS matcher that drives the highlights, so regex group references ($1, $&)
// work in regex mode. In literal mode the replacement is inserted verbatim.

const REPLACE_ALL_MAX = 20000; // hits per pass; the message says when to rerun

function charLenOf(str) {
  return Array.from(str).length;
}

function utf8ByteLength(str) {
  return new TextEncoder().encode(str).length;
}

// UTF-16 index of Unicode-scalar column `col` in `text` (surrogate-safe).
function utf16IndexOfCol(text, col) {
  let idx = 0;
  let c = 0;
  for (const ch of text) {
    if (c >= col) break;
    idx += ch.length;
    c++;
  }
  return idx;
}

// The replacement string sent to the document for one concrete match.
function replacementFor(matchText, replacement) {
  if (!state.regex) return replacement;
  const single = new RegExp(state.matcher.source, state.matcher.flags.replace("g", ""));
  return matchText.replace(single, replacement);
}

// In literal mode "$" has no special meaning; escape it for String.replace.
function literalReplacement(replacement) {
  return replacement.replace(/\$/g, "$$$$");
}

function replaceReady() {
  if (!state.stat?.open) return false;
  if (!state.query) {
    flashCount("検索文字列を入力してください", "error");
    return false;
  }
  buildMatcher();
  if (state.regexError || !state.matcher) {
    flashCount("正規表現エラー", "error");
    return false;
  }
  return true;
}

// 置換: replace the current match, then move to the next one. Without a
// current match this just selects the first one (Notepad-style two-step).
async function replaceCurrent() {
  if (!replaceReady()) return;
  const replacement = $("replace-input").value;
  if (!state.lastMatch) {
    await findStep("next");
    return;
  }
  try {
    const res = await api(`/api/find?dir=next&from=${state.lastMatch.byte}&${qs()}`);
    const h = res.hit;
    if (!h || h.byte !== state.lastMatch.byte) {
      await findStep("next");
      return;
    }
    const lr = await api(`/api/lines?start=${h.line}&count=1`);
    const text = lr.lines?.[0]?.text ?? "";
    const u16 = utf16IndexOfCol(text, h.column);
    const re = new RegExp(state.matcher.source, state.matcher.flags);
    re.lastIndex = u16;
    const m = re.exec(text);
    if (!m || m.index !== u16) {
      flashCount("一致を特定できません", "error");
      return;
    }
    const rep = replacementFor(m[0], replacement);
    const c0 = h.column;
    const c1 = h.column + charLenOf(m[0]);
    await enqueueEdit(() => applyRange(h.line, c0, h.line, c1, rep));
    // Resume the scan just past the inserted text so a replacement that
    // contains the query can never loop.
    state.lastMatch = { byte: h.byte, len: Math.max(1, utf8ByteLength(rep)) };
    await updateCount();
    await findStep("next");
  } catch (e) {
    flashCount("置換エラー", "error");
    console.error(e);
  }
}

// すべて置換: one whole-line edit per matching line, flushed in batches. Line
// numbers never change (line-based matches cannot introduce newlines), so
// every batch keeps referring to valid coordinates.
async function replaceAll() {
  if (!replaceReady()) return;
  const replacement = $("replace-input").value;
  const literal = literalReplacement(replacement);
  showLoading("置換中…");
  try {
    const res = await api(`/api/search?${qs()}&start=0&max=${REPLACE_ALL_MAX}`);
    const hits = res.hits || [];
    if (!hits.length) {
      flashCount("一致なし");
      return;
    }
    const lines = [...new Set(hits.map((h) => h.line))].sort((a, b) => a - b);
    // Fetch the affected lines in contiguous chunks (≤2000 lines per request).
    const texts = new Map();
    for (let i = 0; i < lines.length; ) {
      let j = i;
      while (j + 1 < lines.length && lines[j + 1] - lines[i] < 2000) j++;
      const start = lines[i];
      const count = lines[j] - lines[i] + 1;
      const r = await api(`/api/lines?start=${start}&count=${count}`);
      r.lines.forEach((rec, k) => texts.set(start + k, rec.text ?? ""));
      i = j + 1;
    }
    let replaced = 0;
    let edits = [];
    let pendingBytes = 0;
    const flush = async () => {
      if (!edits.length) return;
      const batch = edits;
      edits = [];
      pendingBytes = 0;
      await enqueueEdit(() => applyBatchPlain(batch));
    };
    for (const line of lines) {
      const text = texts.get(line);
      if (text == null) continue;
      const re = new RegExp(state.matcher.source, state.matcher.flags);
      const count = [...text.matchAll(re)].length;
      if (!count) continue;
      const next = text.replace(re, state.regex ? replacement : literal);
      if (next === text) continue;
      replaced += count;
      edits.push({ l0: line, c0: 0, l1: line, c1: charLenOf(text), text: next });
      pendingBytes += next.length;
      if (edits.length >= 2000 || pendingBytes > 512 * 1024) await flush();
    }
    await flush();
    state.lastMatch = null;
    await updateCount();
    flashCount(
      replaced
        ? `${commas(replaced)} 件置換しました${res.truncated ? " — 一致が多いため一部です。もう一度実行してください" : ""}`
        : "一致なし",
    );
  } catch (e) {
    flashCount("置換エラー", "error");
    console.error(e);
  } finally {
    hideLoading();
  }
}

function diffVisible() {
  return !$("diff-modal").classList.contains("hidden");
}

function hideDiff() {
  setModalOpen($("diff-modal"), false);
  focusEditor();
}

function showDiff(res) {
  $("diff-summary").textContent =
    `${commas(res.hunk_count)} hunk / +${commas(res.added)}  -${commas(res.deleted)}  ~${commas(res.modified)}` +
    (res.current_dirty ? ` / ${t("未保存編集込み")}` : "") +
    (res.omitted_hunks ? ` / ${commas(res.omitted_hunks)} hunk omitted` : "");
  $("diff-old-path").textContent =
    displayPath(res.old_path || t("現在のファイル")) + (res.current_dirty ? " *" : "");
  $("diff-new-path").textContent = displayPath(res.new_path || t("比較先"));
  renderDiffView(res);
  setModalOpen($("diff-modal"), true);
}

function diffKindLabel(kind) {
  if (kind === "insert") return t("追加");
  if (kind === "delete") return t("削除");
  return t("変更");
}

const INLINE_DIFF_MAX_CHARS = 2000;
const INLINE_DIFF_MAX_TOKENS = 260;

function inlineTokens(text) {
  const tokens = [];
  const re = /(\s+|[\p{Letter}\p{Number}_]+|[^\s\p{Letter}\p{Number}_]+)/gu;
  for (const m of String(text || "").matchAll(re)) tokens.push(m[0]);
  return tokens;
}

function pushDiffPart(parts, text, changed) {
  if (!text) return;
  const last = parts[parts.length - 1];
  if (last && last.changed === changed) last.text += text;
  else parts.push({ text, changed });
}

function inlineWordDiff(oldText, newText) {
  oldText = String(oldText || "");
  newText = String(newText || "");
  if (oldText === newText) return null;
  if (oldText.length + newText.length > INLINE_DIFF_MAX_CHARS) return null;
  const oldTokens = inlineTokens(oldText);
  const newTokens = inlineTokens(newText);
  if (oldTokens.length + newTokens.length > INLINE_DIFF_MAX_TOKENS) return null;

  const m = oldTokens.length;
  const n = newTokens.length;
  const dp = Array.from({ length: m + 1 }, () => new Uint16Array(n + 1));
  for (let i = m - 1; i >= 0; i--) {
    for (let j = n - 1; j >= 0; j--) {
      dp[i][j] =
        oldTokens[i] === newTokens[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }

  const oldParts = [];
  const newParts = [];
  let i = 0;
  let j = 0;
  while (i < m || j < n) {
    if (i < m && j < n && oldTokens[i] === newTokens[j]) {
      pushDiffPart(oldParts, oldTokens[i], false);
      pushDiffPart(newParts, newTokens[j], false);
      i++;
      j++;
    } else if (j >= n || (i < m && dp[i + 1][j] >= dp[i][j + 1])) {
      pushDiffPart(oldParts, oldTokens[i], true);
      i++;
    } else {
      pushDiffPart(newParts, newTokens[j], true);
      j++;
    }
  }
  return { oldParts, newParts };
}

function appendDiffText(el, line, parts) {
  if (!line) return;
  if (!parts) {
    el.textContent = line.text;
    return;
  }
  for (const part of parts) {
    const span = document.createElement("span");
    span.className = part.changed ? "diff-word changed" : "diff-word";
    span.textContent = part.text;
    el.append(span);
  }
}

function diffCell(line, cls, parts = null) {
  const cell = document.createElement("div");
  cell.className = "diff-cell " + (cls || "");
  const ln = document.createElement("span");
  ln.className = "diff-ln";
  ln.textContent = line ? String(line.number + 1) : "";
  const tx = document.createElement("span");
  tx.className = "diff-tx";
  appendDiffText(tx, line, parts);
  cell.append(ln, tx);
  return cell;
}

function renderDiffView(res) {
  const view = $("diff-view");
  view.textContent = "";
  const frag = document.createDocumentFragment();
  for (const h of res.hunks || []) {
    const hunk = document.createElement("section");
    hunk.className = "diff-hunk";
    const title = document.createElement("div");
    title.className = "diff-hunk-title";
    title.textContent =
      `${diffKindLabel(h.kind)}  ${t("現在")}: ${commas(h.old_start + 1)} (${commas(h.old_len)} ${currentLocale() === "en" ? "lines" : "行"})  ` +
      `${t("比較先")}: ${commas(h.new_start + 1)} (${commas(h.new_len)} ${currentLocale() === "en" ? "lines" : "行"})`;
    hunk.append(title);
    const oldRows = h.old_preview || [];
    const newRows = h.new_preview || [];
    const max = Math.max(oldRows.length, newRows.length, 1);
    for (let i = 0; i < max; i++) {
      const row = document.createElement("div");
      row.className = "diff-row";
      const oldLine = oldRows[i] || null;
      const newLine = newRows[i] || null;
      const oldCls =
        h.kind === "insert" ? "blank" : h.kind === "delete" ? "del" : oldLine ? "mod" : "blank";
      const newCls =
        h.kind === "delete" ? "blank" : h.kind === "insert" ? "add" : newLine ? "mod" : "blank";
      const wordDiff =
        h.kind === "replace" && oldLine && newLine
          ? inlineWordDiff(oldLine.text, newLine.text)
          : null;
      row.append(
        diffCell(oldLine, oldCls, wordDiff?.oldParts),
        diffCell(newLine, newCls, wordDiff?.newParts),
      );
      hunk.append(row);
    }
    if (h.old_truncated || h.new_truncated) {
      const tr = document.createElement("div");
      tr.className = "diff-truncated";
      tr.textContent = translateText(
        `このhunkは先頭 ${commas(res.max_lines_per_hunk || 80)} 行だけ表示しています`,
      );
      hunk.append(tr);
    }
    frag.append(hunk);
  }
  if (!res.hunks || res.hunks.length === 0) {
    const empty = document.createElement("div");
    empty.className = "diff-truncated";
    empty.textContent = t("差分はありません");
    frag.append(empty);
  }
  view.append(frag);
}

async function diffFile() {
  const base = state.stat?.path || "";
  const path = await askPrompt("2ファイル差分", "比較先ファイルパス", base);
  if (path == null || path.trim() === "") return;
  showLoading("差分を計算中…");
  try {
    const res = await api(
      `/api/diff?path=${encodeURIComponent(path.trim())}&max_hunks=200&max_lines=80&window=128`,
    );
    flashCount(`差分: ${commas(res.hunk_count)} hunk`);
    showDiff(res);
  } catch (e) {
    flashCount("差分エラー", "error");
    showMessage("差分エラー", e.message);
  } finally {
    hideLoading();
  }
}

// ---- フォルダ内検索 (Grep): recursive multi-file search -------------------
// Prompts for a query + options, streams the hits from /api/grep, and shows
// them in a results panel modeled on the diff view. Clicking a hit opens that
// file and jumps to the line.
let lastGrep = { query: "", dir: "", glob: "", ci: false, word: false, regex: false };

function grepVisible() {
  return !$("grep-modal").classList.contains("hidden");
}

function hideGrep() {
  setModalOpen($("grep-modal"), false);
  focusEditor();
}

async function grepFolder() {
  if (anyModalOpen()) return;
  const base =
    lastGrep.dir || localStorage.getItem(TREE_KEY) || pathDirName(state.stat?.path || "") || "";
  const form = await askForm(
    "フォルダ内検索",
    [
      {
        id: "query",
        type: "text",
        label: "検索語",
        value: lastGrep.query,
        placeholder: "検索する文字列 / 正規表現",
      },
      {
        id: "dir",
        type: "text",
        label: "対象フォルダ",
        value: base,
        placeholder: "空欄で開いているファイルのフォルダ",
      },
      {
        id: "glob",
        type: "text",
        label: "ファイル名フィルタ",
        value: lastGrep.glob,
        placeholder: "例: *.rs, *.txt (空欄で全て)",
      },
      { id: "ci", type: "check", label: "大文字小文字を区別しない", value: lastGrep.ci },
      { id: "word", type: "check", label: "単語単位", value: lastGrep.word },
      { id: "regex", type: "check", label: "正規表現", value: lastGrep.regex },
    ],
    "検索",
  );
  if (!form) return;
  const query = (form.query || "").trim();
  if (!query) return;
  lastGrep = {
    query: form.query,
    dir: (form.dir || "").trim(),
    glob: form.glob || "",
    ci: !!form.ci,
    word: !!form.word,
    regex: !!form.regex,
  };
  showLoading("フォルダ内を検索中…");
  try {
    const res = await apiPost("/api/grep", {
      query,
      dir: lastGrep.dir,
      glob: (form.glob || "").trim(),
      ci: lastGrep.ci,
      word: lastGrep.word,
      regex: lastGrep.regex,
    });
    flashCount(`フォルダ内検索: ${commas(res.hits.length)} 件`);
    showGrep(res, query, lastGrep.regex);
  } catch (e) {
    flashCount("フォルダ内検索エラー", "error");
    showMessage("フォルダ内検索エラー", e.message);
  } finally {
    hideLoading();
  }
}

function showGrep(res, query, regex) {
  const files = new Set(res.hits.map((h) => h.path)).size;
  $("grep-summary").textContent = translateText(
    `${commas(res.hits.length)} 件 / ${commas(files)} ファイル` +
      (res.truncated ? `（上限 ${commas(res.hits.length)} 件で打ち切り）` : "") +
      (res.files_truncated ? " / 走査ファイル数の上限に達しました" : ""),
  );
  renderGrepResults(res, query, regex);
  setModalOpen($("grep-modal"), true);
}

// Highlight the literal match inside a preview line ([col, col+queryChars]).
// Regex matches have a variable span we don't return, so those aren't marked.
function appendGrepText(el, text, col, query, regex) {
  const chars = Array.from(text);
  const qlen = regex ? 0 : Array.from(query).length;
  if (!qlen || col < 0 || col > chars.length) {
    el.textContent = text;
    return;
  }
  const before = chars.slice(0, col).join("");
  const mid = chars.slice(col, col + qlen).join("");
  const after = chars.slice(col + qlen).join("");
  if (before) el.append(document.createTextNode(before));
  const mark = document.createElement("span");
  mark.className = "grep-match";
  mark.textContent = mid;
  el.append(mark);
  if (after) el.append(document.createTextNode(after));
}

function renderGrepResults(res, query, regex) {
  const view = $("grep-results");
  view.textContent = "";
  const hits = res.hits || [];
  if (hits.length === 0) {
    const empty = document.createElement("div");
    empty.className = "grep-empty";
    empty.textContent = t("一致はありません");
    view.append(empty);
    return;
  }
  const frag = document.createDocumentFragment();
  let group = null;
  let currentPath = null;
  for (const h of hits) {
    if (h.path !== currentPath) {
      currentPath = h.path;
      group = document.createElement("section");
      group.className = "grep-file";
      const head = document.createElement("div");
      head.className = "grep-file-head";
      head.textContent = displayPath(h.path);
      head.title = displayPath(h.path);
      group.append(head);
      frag.append(group);
    }
    const row = document.createElement("button");
    row.className = "grep-hit";
    row.type = "button";
    const ln = document.createElement("span");
    ln.className = "grep-ln";
    ln.textContent = commas(h.line + 1);
    const tx = document.createElement("span");
    tx.className = "grep-tx";
    appendGrepText(tx, h.text, h.col, query, regex);
    row.append(ln, tx);
    row.addEventListener("click", () => openGrepHit(h.path, h.line));
    group.append(row);
  }
  view.append(frag);
}

async function openGrepHit(path, line) {
  hideGrep();
  await openPath(path);
  gotoLine(line + 1);
}

// 選択メニュー「大文字に変換 / 小文字に変換」: transform the selection in the
// editor as one undoable edit — nothing is written to disk until 保存.
async function transformSelection(mode) {
  if (!state.stat?.open) return;
  const fn = mode === "upper" ? (s) => s.toUpperCase() : (s) => s.toLowerCase();
  if (!hasTextSelection()) {
    flashCount("変換する範囲を選択してください", "error");
    return;
  }
  if (selectionLineCount() > MAX_COPY_LINES) {
    flashCount(`変換は一度に ${commas(MAX_COPY_LINES)} 行までです`, "error");
    return;
  }
  const rr = rectRange();
  enqueueEdit(async () => {
    if (rr) {
      const total = rr.l1 - rr.l0 + 1;
      const res = await api(`/api/lines?start=${rr.l0}&count=${total}`);
      const edits = [];
      res.lines.forEach((rec, i) => {
        const chars = Array.from(rec.text ?? "");
        const c0 = Math.min(rr.c0, chars.length);
        const c1 = Math.min(rr.c1, chars.length);
        const piece = chars.slice(c0, c1).join("");
        const next = fn(piece);
        if (c1 > c0 && next !== piece) {
          edits.push({ l0: rr.l0 + i, c0, l1: rr.l0 + i, c1, text: next });
        }
      });
      return edits.length ? applyBatchPlain(edits) : null;
    }
    // Normal / multi-cursor selections: one edit per cursor, one undo step.
    const cursors = allCursors();
    const texts = [];
    for (const c of cursors) {
      const r = cursorSelectionRange(c);
      texts.push(r ? fn(await selectedTextForRange(r)) : null);
    }
    const edits = [];
    const editOf = cursors.map((c, i) => {
      if (texts[i] == null) return -1;
      edits.push({ ...cursorReplaceRange(c), text: texts[i] });
      return edits.length - 1;
    });
    if (!edits.length) return null;
    return applyBatch(edits, cursors, editOf);
  });
}

// Apply a prepared edit batch that is not tied to the cursors (rect case
// transform, replace-all) and refresh the view around the existing caret.
async function applyBatchPlain(edits) {
  const ctx = editContext();
  await apiPost("/api/edit/replace_batch", { edits });
  if (!sameEditContext(ctx)) return;
  state.sel = null;
  state.extraCursors = [];
  try {
    await reloadViewport();
    await refreshStat();
  } catch (e) {
    console.error("post-batch refresh failed", e);
    flashCount("再読込エラー");
  }
  if (!sameEditContext(ctx)) return;
  setCaret(Math.min(state.caret.line, Math.max(0, state.total - 1)), state.caret.col);
  revealCaret();
  render();
}

// ---- whole-line operations (行を複製 / 移動 / 削除) -------------------------
// Each command edits whole lines as ONE batch edit (a single undo step) built
// against the pre-edit view, then commits through applyLineEdit so the caret —
// and, for a selection, the covered block — follows the lines to their new home.
// A multi-line selection acts on every line it covers; multi-cursor collapses to
// the primary caret, like the word-delete commands.

// The whole-line span a line op acts on: the lines the selection covers, or the
// caret's line when there is no selection. A selection ending exactly at column
// 0 of a line does not pull that trailing line in.
function lineOpSpan() {
  const r = selRange();
  if (r && !rangeEmpty(r)) {
    let l0 = r.start.line;
    let l1 = r.end.line;
    if (!r.rect && l1 > l0 && r.end.col === 0) l1 -= 1;
    return { l0, l1 };
  }
  return { l0: state.caret.line, l1: state.caret.line };
}

// Decoded text of one line — from the cache when resident, else a targeted
// fetch (a selection can reach past the cached window).
async function oneLineText(line) {
  const c = cachedLine(line);
  if (c != null) return c.text ?? "";
  const res = await api(`/api/lines?start=${line}&count=1`);
  return res.lines[0]?.text ?? "";
}

async function lineLenAt(line) {
  return charLenOf(await oneLineText(line));
}

// Decoded text of the lines [start, start+count) as a plain string array.
async function lineTextsFor(start, count) {
  const res = await api(`/api/lines?start=${start}&count=${count}`);
  return res.lines.map((r) => r.text ?? "");
}

// Shift a (non-rect) selection's endpoints by `delta` whole lines so it keeps
// hugging the block it covered after a move.
function shiftLineSelection(sel, delta) {
  if (!sel) return null;
  return {
    anchor: { line: sel.anchor.line + delta, col: sel.anchor.col },
    head: { line: sel.head.line + delta, col: sel.head.col },
  };
}

// Commit a line op's batch edit, refresh the view, then place the caret and
// (optionally) restore the selection. Mirrors applyBatchPlain but positions the
// caret/selection deliberately instead of collapsing to the pre-edit caret.
async function applyLineEdit(edits, caret, sel) {
  const ctx = editContext();
  await apiPost("/api/edit/replace_batch", { edits });
  if (!sameEditContext(ctx)) return;
  state.extraCursors = []; // line ops are single-caret
  try {
    await reloadViewport();
    await refreshStat();
  } catch (e) {
    console.error("post-line-edit refresh failed", e);
    flashCount("再読込エラー");
  }
  if (!sameEditContext(ctx)) return;
  const last = Math.max(0, state.total - 1);
  const place = (p) => {
    const line = Math.min(Math.max(0, p.line), last);
    const cached = cachedLine(line);
    const col = cached ? Math.min(p.col, charLenOf(cached.text ?? "")) : Math.max(0, p.col);
    return { line, col };
  };
  const c = place(caret);
  state.caret = c;
  state.activeLine = c.line;
  state.goalCol = c.col;
  state.sel = sel ? { anchor: place(sel.anchor), head: place(sel.head) } : null;
  revealCaret();
  render();
}

// 行を複製: duplicate the covered line block just below itself.
function duplicateLines() {
  if (!state.stat?.open || state.total === 0) return;
  const { l0, l1 } = lineOpSpan();
  if (l1 - l0 + 1 > MAX_COPY_LINES) {
    flashCount(`複製は一度に ${commas(MAX_COPY_LINES)} 行までです`, "error");
    return;
  }
  const caret = { ...state.caret }; // the copy lands below; the caret stays put
  const sel = cloneSelection(state.sel && !state.sel.rect ? state.sel : null);
  enqueueEdit(async () => {
    const texts = await lineTextsFor(l0, l1 - l0 + 1);
    const endCol = charLenOf(texts[texts.length - 1]);
    const edit = { l0: l1, c0: endCol, l1, c1: endCol, text: "\n" + texts.join("\n") };
    return applyLineEdit([edit], caret, sel);
  });
}

// 行を上へ / 下へ移動: swap the covered block with its neighbouring line.
function moveLines(dir) {
  if (!state.stat?.open || state.total === 0) return;
  const { l0, l1 } = lineOpSpan();
  if (dir < 0 ? l0 === 0 : l1 >= state.total - 1) return; // already at the edge
  if (l1 - l0 + 1 > MAX_COPY_LINES) {
    flashCount(`行の移動は一度に ${commas(MAX_COPY_LINES)} 行までです`, "error");
    return;
  }
  const caret = { line: state.caret.line + dir, col: state.caret.col };
  const sel = shiftLineSelection(
    cloneSelection(state.sel && !state.sel.rect ? state.sel : null),
    dir,
  );
  enqueueEdit(async () => {
    const block = await lineTextsFor(l0, l1 - l0 + 1);
    if (dir < 0) {
      // Up: line (l0-1) drops below the block.
      const above = await oneLineText(l0 - 1);
      const edit = {
        l0: l0 - 1,
        c0: 0,
        l1,
        c1: charLenOf(block[block.length - 1]),
        text: block.join("\n") + "\n" + above,
      };
      return applyLineEdit([edit], caret, sel);
    }
    // Down: line (l1+1) rises above the block.
    const below = await oneLineText(l1 + 1);
    const edit = {
      l0,
      c0: 0,
      l1: l1 + 1,
      c1: charLenOf(below),
      text: below + "\n" + block.join("\n"),
    };
    return applyLineEdit([edit], caret, sel);
  });
}

// 行を削除: drop the covered line block entirely.
function deleteLines() {
  if (!state.stat?.open || state.total === 0) return;
  const { l0, l1 } = lineOpSpan();
  enqueueEdit(async () => {
    let edit;
    let caret;
    if (l1 < state.total - 1) {
      // Lines survive below: drop the block; the next line slides up to l0.
      edit = { l0, c0: 0, l1: l1 + 1, c1: 0, text: "" };
      caret = { line: l0, col: 0 };
    } else if (l0 === 0) {
      // The whole document: collapse to a single empty line.
      edit = { l0: 0, c0: 0, l1, c1: await lineLenAt(l1), text: "" };
      caret = { line: 0, col: 0 };
    } else {
      // The block runs to EOF: fold it into the previous line's tail.
      const prevLen = await lineLenAt(l0 - 1);
      edit = { l0: l0 - 1, c0: prevLen, l1, c1: await lineLenAt(l1), text: "" };
      caret = { line: l0 - 1, col: prevLen };
    }
    return applyLineEdit([edit], caret, null);
  });
}

// ---- input wiring ----------------------------------------------------------

function setQueryFromInput() {
  state.query = $("find").value;
  state.lastMatch = null;
  state.searchHits = null;
  state.searchTruncated = false;
  buildMatcher();
  $("find-count").textContent = state.regexError ? t("正規表現エラー") : "";
  scheduleRender();
}

function initEvents() {
  const vp = $("viewport");

  vp.addEventListener(
    "wheel",
    (e) => {
      e.preventDefault();
      let dy = e.deltaY;
      if (e.deltaMode === 1) dy *= LINE_HEIGHT;
      else if (e.deltaMode === 2) dy *= vp.clientHeight;
      state.fracAcc += dy / LINE_HEIGHT;
      const whole = Math.trunc(state.fracAcc);
      state.fracAcc -= whole;
      if (whole !== 0) setFirst(state.first + whole);
    },
    { passive: false },
  );

  const find = $("find");
  find.addEventListener("input", setQueryFromInput);
  find.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      updateCount();
      findStep(e.shiftKey ? "prev" : "next");
    } else if (e.key === "ArrowUp" || e.key === "ArrowDown") {
      if (showSearchHistory(e.key === "ArrowUp" ? -1 : 1)) {
        e.preventDefault();
      }
    } else if (e.key === "Escape") {
      hideFind();
      focusEditor();
    }
  });

  $("find-close").addEventListener("click", () => {
    hideFind();
    focusEditor();
  });
  $("find-expand").addEventListener("click", () => setReplaceRow(!state.replaceOpen));
  $("replace-one").addEventListener("click", () => replaceCurrent());
  $("replace-all").addEventListener("click", () => replaceAll());
  $("replace-input").addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      replaceCurrent();
    } else if (e.key === "Escape") {
      hideFind();
      focusEditor();
    }
  });
  $("find-next").addEventListener("click", () => findStep("next"));
  $("find-prev").addEventListener("click", () => findStep("prev"));
  $("opt-case").addEventListener("click", () => toggleOpt("ci", "opt-case"));
  $("opt-word").addEventListener("click", () => toggleOpt("word", "opt-word"));
  $("opt-regex").addEventListener("click", () => toggleOpt("regex", "opt-regex"));
  $("save-file").addEventListener("click", () => {
    hideFileMenu();
    saveFile();
  });
  $("save-copy").addEventListener("click", () => {
    hideFileMenu();
    saveCopy();
  });
  $("convert-save-item").addEventListener("click", showConvert);
  $("convert-close").addEventListener("click", hideConvert);
  $("convert-cancel").addEventListener("click", hideConvert);
  $("convert-enc").addEventListener("change", syncConvertBom);
  $("convert-go").addEventListener("click", () => {
    const encoding = $("convert-enc").value;
    const eolVal = $("convert-eol").value;
    // A UTF-8 BOM is the only one we emit; force it off for other encodings.
    const bom = encoding === "utf-8" && $("convert-bom").checked;
    hideConvert();
    convertSave(encoding, eolVal, bom);
  });
  $("reopen-go").addEventListener("click", () => {
    const encoding = $("convert-enc").value;
    hideConvert();
    reopenWithEncoding(encoding);
  });
  $("convert-modal").addEventListener("click", (e) => {
    if (e.target === $("convert-modal")) hideConvert();
  });
  $("st-enc").addEventListener("click", showConvert);
  $("st-eol").addEventListener("click", showConvert);
  $("st-tail").addEventListener("click", () => setFollowTail(!state.followTail));
  $("apply-theme").addEventListener("click", applyThemeFromBuffer);
  $("apply-keymap").addEventListener("click", applyKeymapFromBuffer);
  $("undo-edit").addEventListener("click", undoEdit);
  $("redo-edit").addEventListener("click", redoEdit);
  $("diff-close").addEventListener("click", hideDiff);
  $("diff-modal").addEventListener("click", (e) => {
    if (e.target === $("diff-modal")) hideDiff();
  });
  $("grep-close").addEventListener("click", hideGrep);
  $("grep-modal").addEventListener("click", (e) => {
    if (e.target === $("grep-modal")) hideGrep();
  });

  // Keep the column ruler aligned as the text scrolls horizontally.
  $("content").addEventListener("scroll", () => {
    if (state.settings.ruler) {
      $("ruler-inner").style.transform = `translateX(${-$("content").scrollLeft}px)`;
    }
  });

  document.addEventListener("keydown", onGlobalKey);
  window.addEventListener("resize", scheduleRender);
}

function toggleOpt(key, id) {
  state[key] = !state[key];
  $(id).classList.toggle("on", state[key]);
  state.lastMatch = null;
  state.searchHits = null;
  state.searchTruncated = false;
  buildMatcher();
  scheduleRender();
  if (state.query) updateCount();
}

// ---- generic confirm / message dialog (replaces window.confirm/alert) -----
// The browser dialogs leak the server origin into their chrome
// ("127.0.0.1:PORT の内容"); everything user-facing goes through this modal.
function confirmVisible() {
  return !$("confirm").classList.contains("hidden");
}

function askConfirm(title, message, opts = {}) {
  return new Promise((resolve) => {
    const modal = $("confirm");
    const okBtn = $("confirm-ok");
    const cancelBtn = $("confirm-cancel");
    $("confirm-title").textContent = translateText(title || "確認");
    $("confirm-message").textContent = translateText(message || "");
    okBtn.textContent = translateText(opts.okLabel || "OK");
    okBtn.classList.toggle("danger", !!opts.danger);
    okBtn.classList.toggle("primary", !opts.danger);
    cancelBtn.textContent = translateText(opts.cancelLabel || "キャンセル");
    cancelBtn.classList.toggle("hidden", !!opts.alert);
    setModalOpen(modal, true);
    queueMicrotask(() => okBtn.focus());
    const finish = (val) => {
      setModalOpen(modal, false);
      okBtn.removeEventListener("click", onOk);
      cancelBtn.removeEventListener("click", onCancel);
      $("confirm-close").removeEventListener("click", onCancel);
      modal.removeEventListener("mousedown", onBackdrop);
      modal.removeEventListener("keydown", onKey);
      focusEditor();
      resolve(val);
    };
    const onOk = () => finish(true);
    const onCancel = () => finish(false);
    const onKey = (ev) => {
      ev.stopPropagation();
      if (ev.key === "Enter") {
        ev.preventDefault();
        finish(true);
      } else if (ev.key === "Escape") {
        ev.preventDefault();
        finish(false);
      }
    };
    const onBackdrop = (ev) => {
      if (ev.target === modal) finish(false);
    };
    okBtn.addEventListener("click", onOk);
    cancelBtn.addEventListener("click", onCancel);
    $("confirm-close").addEventListener("click", onCancel);
    modal.addEventListener("mousedown", onBackdrop);
    modal.addEventListener("keydown", onKey);
  });
}

// OK-only variant for error details and notices (replaces window.alert).
function showMessage(title, message) {
  return askConfirm(title, message, { alert: true });
}

// ---- generic input prompt (replaces the browser's window.prompt) ---------
function promptVisible() {
  return !$("prompt").classList.contains("hidden");
}
function askPrompt(title, label, value = "") {
  return new Promise((resolve) => {
    const modal = $("prompt");
    $("prompt-title").textContent = translateText(title || "入力");
    $("prompt-label").textContent = translateText(label || "");
    const input = $("prompt-input");
    input.value = value;
    setModalOpen(modal, true);
    setTimeout(() => {
      input.focus();
      input.select();
    }, 0);
    const finish = (val) => {
      setModalOpen(modal, false);
      input.removeEventListener("keydown", onKey);
      $("prompt-ok").removeEventListener("click", onOk);
      $("prompt-cancel").removeEventListener("click", onCancel);
      $("prompt-close").removeEventListener("click", onCancel);
      modal.removeEventListener("mousedown", onBackdrop);
      focusEditor();
      resolve(val);
    };
    const onOk = () => finish(input.value);
    const onCancel = () => finish(null);
    const onKey = (ev) => {
      ev.stopPropagation();
      if (ev.key === "Enter") {
        ev.preventDefault();
        finish(input.value);
      } else if (ev.key === "Escape") {
        ev.preventDefault();
        finish(null);
      }
    };
    const onBackdrop = (ev) => {
      if (ev.target === modal) finish(null);
    };
    input.addEventListener("keydown", onKey);
    $("prompt-ok").addEventListener("click", onOk);
    $("prompt-cancel").addEventListener("click", onCancel);
    $("prompt-close").addEventListener("click", onCancel);
    modal.addEventListener("mousedown", onBackdrop);
  });
}

// ---- generic small form dialog (sort / replace / case options) ------------
function formVisible() {
  return !$("form-modal").classList.contains("hidden");
}

// fields: {id, type: "text"|"check"|"select"|"hint", label, value, placeholder,
// title, options}. Resolves to {id: value} or null on cancel.
function askForm(title, fields, okLabel = "実行") {
  return new Promise((resolve) => {
    const modal = $("form-modal");
    const body = $("form-body");
    $("form-title").textContent = translateText(title || "オプション");
    $("form-ok").textContent = translateText(okLabel);
    body.textContent = "";
    const readers = {};
    for (const f of fields) {
      if (f.type === "hint") {
        const hint = document.createElement("div");
        hint.className = "form-hint";
        hint.textContent = translateText(f.label);
        body.append(hint);
        continue;
      }
      if (f.type === "check") {
        const lab = document.createElement("label");
        lab.className = "form-check";
        if (f.title) lab.title = translateText(f.title);
        const cb = document.createElement("input");
        cb.type = "checkbox";
        cb.checked = !!f.value;
        lab.append(cb, document.createTextNode(translateText(f.label)));
        body.append(lab);
        readers[f.id] = () => cb.checked;
        continue;
      }
      const row = document.createElement("label");
      row.className = "form-row";
      const span = document.createElement("span");
      span.textContent = translateText(f.label);
      row.append(span);
      if (f.type === "select") {
        const sel = document.createElement("select");
        for (const [v, text] of f.options || []) {
          const o = document.createElement("option");
          o.value = v;
          o.textContent = translateText(text);
          sel.append(o);
        }
        if (f.value != null) sel.value = f.value;
        row.append(sel);
        readers[f.id] = () => sel.value;
      } else {
        const input = document.createElement("input");
        input.type = "text";
        input.value = f.value ?? "";
        input.placeholder = translateText(f.placeholder ?? "");
        if (f.title) input.title = translateText(f.title);
        row.append(input);
        readers[f.id] = () => input.value;
      }
      body.append(row);
    }
    setModalOpen(modal, true);
    queueMicrotask(() => body.querySelector("input, select")?.focus());
    const finish = (val) => {
      setModalOpen(modal, false);
      $("form-ok").removeEventListener("click", onOk);
      $("form-cancel").removeEventListener("click", onCancel);
      $("form-close").removeEventListener("click", onCancel);
      modal.removeEventListener("mousedown", onBackdrop);
      modal.removeEventListener("keydown", onKey);
      focusEditor();
      resolve(val);
    };
    const collect = () =>
      Object.fromEntries(Object.entries(readers).map(([k, read]) => [k, read()]));
    const onOk = () => finish(collect());
    const onCancel = () => finish(null);
    const onKey = (ev) => {
      ev.stopPropagation();
      if (ev.key === "Enter" && ev.target.tagName !== "SELECT") {
        ev.preventDefault();
        finish(collect());
      } else if (ev.key === "Escape") {
        ev.preventDefault();
        finish(null);
      }
    };
    const onBackdrop = (ev) => {
      if (ev.target === modal) finish(null);
    };
    $("form-ok").addEventListener("click", onOk);
    $("form-cancel").addEventListener("click", onCancel);
    $("form-close").addEventListener("click", onCancel);
    modal.addEventListener("mousedown", onBackdrop);
    modal.addEventListener("keydown", onKey);
  });
}

// ---- loading overlay ------------------------------------------------------
function showLoading(text) {
  const o = $("overlay");
  o.textContent = translateText(text || "読み込み中…");
  o.classList.remove("hidden");
}
function hideLoading() {
  $("overlay").classList.add("hidden");
}

// Jump the caret to a 1-based line number.
function gotoLine(n) {
  const v = parseInt(String(n).replace(/[^0-9]/g, ""), 10);
  if (!Number.isFinite(v) || v < 1) return;
  const line = Math.min(v - 1, Math.max(0, state.total - 1));
  state.sel = null;
  setCaret(line, 0);
  revealLine(line);
  focusEditor();
}

// App-level shortcuts. Caret motion and text editing live in onEditKey (bound
// to the hidden input); those keys never reach here because onEditKey stops
// their propagation. `inField` is true only for the real text inputs (find /
// opener / prompt / settings), never the editor's hidden textarea.
function onGlobalKey(e) {
  const inField = e.target.tagName === "INPUT";
  if (promptVisible() || formVisible() || confirmVisible()) return;
  if (e.key === "Escape" && ctxMenuVisible()) {
    e.preventDefault();
    hideCtxMenu();
    return;
  }
  if (e.key === "Escape" && fileMenuVisible()) {
    e.preventDefault();
    hideFileMenu(true);
    return;
  }
  if (e.key === "Escape" && keymapVisible()) {
    e.preventDefault();
    hideKeymap();
    return;
  }
  if (e.key === "Escape" && commandPaletteVisible()) {
    e.preventDefault();
    hideCommandPalette();
    return;
  }
  if (e.key === "Escape" && diffVisible()) {
    e.preventDefault();
    hideDiff();
    return;
  }
  if (e.key === "Escape" && grepVisible()) {
    e.preventDefault();
    hideGrep();
    return;
  }
  if (e.key === "Escape" && settingsVisible()) {
    e.preventDefault();
    hideSettings();
    return;
  }
  if (e.key === "Escape" && convertVisible()) {
    e.preventDefault();
    hideConvert();
    return;
  }
  if (e.key === "Escape" && openerVisible()) {
    e.preventDefault();
    hideOpener();
    return;
  }
  if (!anyModalOpen() && matchesShortcut(e, "commandPalette")) {
    e.preventDefault();
    showCommandPalette();
    return;
  }
  // A modal owns the keyboard: never run editor/clipboard/history/nav commands
  // against the hidden document behind Settings / the Opener / a prompt.
  if (anyModalOpen()) return;
  if (matchesShortcut(e, "openFile")) {
    e.preventDefault();
    hideFileMenu();
    showOpener();
    return;
  }
  if (matchesShortcut(e, "toggleSidebar")) {
    e.preventDefault();
    setSidebar(!sidebarOpen());
    return;
  }
  if (matchesShortcut(e, "newFile")) {
    e.preventDefault();
    hideFileMenu();
    newUntitled();
    return;
  }
  if (matchesShortcut(e, "newWindow")) {
    e.preventDefault();
    hideFileMenu();
    openNewWindow();
    return;
  }
  if (matchesShortcut(e, "gotoLine")) {
    e.preventDefault();
    askPrompt("行へ移動", "行番号").then((v) => {
      if (v != null) gotoLine(v);
    });
    return;
  }
  if (matchesShortcut(e, "closeTab")) {
    e.preventDefault();
    const active = state.tabs.find((t) => t.active);
    if (active) closeTab(active.id);
    return;
  }
  if (matchesShortcut(e, "find")) {
    e.preventDefault();
    showFind();
    return;
  }
  if (matchesShortcut(e, "replace")) {
    e.preventDefault();
    showFind(true);
    return;
  }
  if (matchesShortcut(e, "saveAs")) {
    e.preventDefault();
    hideFileMenu();
    saveCopy();
    return;
  }
  if (matchesShortcut(e, "saveFile")) {
    e.preventDefault();
    hideFileMenu();
    saveFile();
    return;
  }
  if (matchesShortcut(e, "findPrev")) {
    e.preventDefault();
    findStep("prev");
    return;
  }
  if (matchesShortcut(e, "findNext")) {
    e.preventDefault();
    findStep("next");
    return;
  }
  if (matchesShortcut(e, "searchCase")) {
    e.preventDefault();
    toggleOpt("ci", "opt-case");
    return;
  }
  if (matchesShortcut(e, "searchRegex")) {
    e.preventDefault();
    toggleOpt("regex", "opt-regex");
    return;
  }
  if (matchesShortcut(e, "searchWord")) {
    e.preventDefault();
    toggleOpt("word", "opt-word");
    return;
  }
  if (matchesShortcut(e, "sortSave")) {
    e.preventDefault();
    sortSave();
    return;
  }
  if (matchesShortcut(e, "diffFile")) {
    e.preventDefault();
    diffFile();
    return;
  }
  if (matchesShortcut(e, "splitFile")) {
    e.preventDefault();
    splitFile();
    return;
  }
  if (matchesShortcut(e, "grepFolder")) {
    e.preventDefault();
    grepFolder();
    return;
  }
  if (matchesShortcut(e, "settings")) {
    e.preventDefault();
    showSettings();
    return;
  }
  if (matchesShortcut(e, "keymap")) {
    e.preventDefault();
    showKeymap();
    return;
  }
  // Editor clipboard / history — not while typing in a search or dialog field.
  if (inField) return;
  if (matchesShortcut(e, "selectAll")) {
    e.preventDefault();
    selectAll();
    return;
  }
  if (matchesShortcut(e, "copy")) {
    e.preventDefault();
    copySelection();
    return;
  }
  if (matchesShortcut(e, "cut")) {
    e.preventDefault();
    cutSelection();
    return;
  }
  if (matchesShortcut(e, "caseUpper")) {
    e.preventDefault();
    transformSelection("upper");
    return;
  }
  if (matchesShortcut(e, "caseLower")) {
    e.preventDefault();
    transformSelection("lower");
    return;
  }
  if (matchesShortcut(e, "redo")) {
    e.preventDefault();
    redoEdit();
    return;
  }
  if (matchesShortcut(e, "undo")) {
    e.preventDefault();
    undoEdit();
    return;
  }
}

// ---- editor keyboard: caret motion + structural edits ----------------------

const isWordChar = (ch) => /[\p{L}\p{N}_]/u.test(ch || "");

function wordLeft(line, col) {
  const cs = lineChars(line);
  if (col === 0) return line > 0 ? [line - 1, lineLen(line - 1)] : [line, 0];
  let i = col;
  while (i > 0 && !isWordChar(cs[i - 1])) i--;
  while (i > 0 && isWordChar(cs[i - 1])) i--;
  return [line, i];
}

function wordRight(line, col) {
  const cs = lineChars(line);
  const len = cs.length;
  if (col >= len) return line < state.total - 1 ? [line + 1, 0] : [line, len];
  let i = col;
  while (i < len && !isWordChar(cs[i])) i++;
  while (i < len && isWordChar(cs[i])) i++;
  return [line, i];
}

function deleteWordBack() {
  enqueueEdit(() => {
    const del = deleteSelectionEdit();
    if (del) return del;
    clearExtraCursors(); // word-delete is single-cursor: collapse to the primary
    const c = state.caret;
    const [l, col] = wordLeft(c.line, c.col);
    if (l === c.line && col === c.col) return null;
    return applyRange(l, col, c.line, c.col, "");
  });
}

function deleteWordFwd() {
  enqueueEdit(() => {
    const del = deleteSelectionEdit();
    if (del) return del;
    clearExtraCursors(); // word-delete is single-cursor: collapse to the primary
    const c = state.caret;
    const [l, col] = wordRight(c.line, c.col);
    if (l === c.line && col === c.col) return null;
    return applyRange(c.line, c.col, l, col, "");
  });
}

function onEditKey(e) {
  if (state.composing || e.isComposing) return; // IME owns the keyboard
  if (anyModalOpen()) return; // a dialog is up; don't edit behind it
  if (savingCount > 0) {
    // Edits are blocked while a save is in flight; swallow the key so the
    // hidden textarea can't buffer text that would never be applied.
    e.preventDefault();
    flashCount("保存中です — 完了までお待ちください");
    return;
  }
  const mod = e.ctrlKey || e.metaKey;
  const shift = e.shiftKey;
  const c = state.caret;
  const take = () => {
    e.preventDefault();
    e.stopPropagation();
  };
  // Multi-cursor: add a caret above/below (default Ctrl+Alt+ArrowUp/Down).
  // Checked before the switch so the plain-arrow cases never swallow them.
  if (matchesShortcut(e, "addCursorAbove")) {
    take();
    addCursorAbove();
    return;
  }
  if (matchesShortcut(e, "addCursorBelow")) {
    take();
    addCursorBelow();
    return;
  }
  if (matchesShortcut(e, "selectNextOccurrence")) {
    take();
    selectNextOccurrence();
    return;
  }
  // Whole-line ops: checked before the switch so the plain-arrow cases never
  // swallow 行を上へ/下へ移動 (default Alt+ArrowUp/Down).
  if (matchesShortcut(e, "duplicateLine")) {
    take();
    duplicateLines();
    return;
  }
  if (matchesShortcut(e, "moveLineUp")) {
    take();
    moveLines(-1);
    return;
  }
  if (matchesShortcut(e, "moveLineDown")) {
    take();
    moveLines(1);
    return;
  }
  if (matchesShortcut(e, "deleteLine")) {
    take();
    deleteLines();
    return;
  }
  switch (e.key) {
    case "ArrowLeft":
      take();
      if (mod) {
        const [l, col] = wordLeft(c.line, c.col);
        moveCaret(l, col, shift);
      } else if (c.col > 0) moveCaret(c.line, c.col - 1, shift);
      else if (c.line > 0) moveCaret(c.line - 1, lineLen(c.line - 1), shift);
      state.goalCol = state.caret.col;
      return;
    case "ArrowRight":
      take();
      if (mod) {
        const [l, col] = wordRight(c.line, c.col);
        moveCaret(l, col, shift);
      } else if (c.col < lineLen(c.line)) moveCaret(c.line, c.col + 1, shift);
      else if (c.line < state.total - 1) moveCaret(c.line + 1, 0, shift);
      state.goalCol = state.caret.col;
      return;
    case "ArrowUp":
      take();
      if (mod) setFirst(state.first - 1);
      else if (c.line > 0) moveCaret(c.line - 1, state.goalCol, shift);
      return;
    case "ArrowDown":
      take();
      if (mod) setFirst(state.first + 1);
      else if (c.line < state.total - 1) moveCaret(c.line + 1, state.goalCol, shift);
      return;
    case "Home":
      take();
      moveCaret(mod ? 0 : c.line, 0, shift);
      state.goalCol = state.caret.col;
      return;
    case "End":
      take();
      if (mod) {
        const last = state.total - 1;
        moveCaret(last, lineLen(last), shift);
      } else moveCaret(c.line, lineLen(c.line), shift);
      state.goalCol = state.caret.col;
      return;
    case "PageUp":
      take();
      moveCaret(c.line - rowsVisible(), state.goalCol, shift);
      return;
    case "PageDown":
      take();
      moveCaret(c.line + rowsVisible(), state.goalCol, shift);
      return;
    case "Backspace":
      take();
      if (mod) deleteWordBack();
      else backspace();
      return;
    case "Delete":
      take();
      if (mod) deleteWordFwd();
      else forwardDelete();
      return;
    case "Enter":
      take();
      insertNewline();
      return;
    case "Tab":
      if (mod) return; // don't trap window focus-cycling combos
      take();
      typeText("\t");
      return;
    case "Escape":
      // Collapsing multi-cursor wins over every other Escape meaning here
      // (modals/find never reach this handler — see the guards above).
      if (state.extraCursors.length) {
        take();
        clearExtraCursors();
        return;
      }
      if (state.sel) {
        take();
        state.sel = null;
        scheduleRender();
      }
      return;
    default:
      return; // printable input flows through beforeinput / composition
  }
}

function onBeforeInput(e) {
  if (state.composing) return; // composition text is committed on compositionend
  if (anyModalOpen()) {
    e.preventDefault();
    return;
  }
  switch (e.inputType) {
    case "insertText":
      e.preventDefault();
      if (e.data != null) typeText(e.data);
      break;
    case "insertLineBreak":
    case "insertParagraph":
      e.preventDefault();
      insertNewline();
      break;
    case "deleteContentBackward":
    case "deleteSoftLineBackward":
      e.preventDefault();
      backspace();
      break;
    case "deleteWordBackward":
      e.preventDefault();
      deleteWordBack();
      break;
    case "deleteContentForward":
    case "deleteSoftLineForward":
      e.preventDefault();
      forwardDelete();
      break;
    case "deleteWordForward":
      e.preventDefault();
      deleteWordFwd();
      break;
    case "insertFromPaste":
      e.preventDefault(); // the paste event carries the clipboard text
      break;
    default:
      break;
  }
}

function onPaste(e) {
  const text = (e.clipboardData || window.clipboardData)?.getData("text") ?? "";
  e.preventDefault();
  if (text) pasteText(text);
}

function onCompStart() {
  state.composing = true;
  $("hidden-input").classList.add("composing");
  positionCaret();
}
function onCompUpdate() {
  positionCaret(); // the textarea itself renders the composing string
}
function onCompEnd(e) {
  state.composing = false;
  const hi = $("hidden-input");
  hi.classList.remove("composing");
  const data = e.data || "";
  hi.value = "";
  if (data) typeText(data);
  else scheduleRender();
}

function initEditor() {
  const hi = $("hidden-input");
  hi.addEventListener("keydown", onEditKey);
  hi.addEventListener("beforeinput", onBeforeInput);
  hi.addEventListener("input", () => {
    if (!state.composing) hi.value = "";
  });
  hi.addEventListener("paste", onPaste);
  hi.addEventListener("compositionstart", onCompStart);
  hi.addEventListener("compositionupdate", onCompUpdate);
  hi.addEventListener("compositionend", onCompEnd);
  hi.addEventListener("focus", () => {
    state.focused = true;
    scheduleRender();
  });
  hi.addEventListener("blur", () => {
    state.focused = false;
    scheduleRender();
  });
  // Keep the caret glued to its cell during horizontal scroll.
  $("content").addEventListener("scroll", positionCaret);
}

// ---- workspace: open / browse / drag&drop ----------------------------------

function openerVisible() {
  return !$("opener").classList.contains("hidden");
}

function showOpener() {
  configureOpener("open");
  setModalOpen($("opener"), true);
  browse(null);
  const inp = $("opener-input");
  inp.value = "";
  queueMicrotask(() => inp.focus());
}

function showSaveDialog(title, suggestedPath) {
  return new Promise((resolve) => {
    configureOpener("save", title);
    state.openerResolve = resolve;
    const inp = $("opener-input");
    const dir = pathDirName(suggestedPath) || localStorage.getItem(TREE_KEY) || ".";
    inp.value = pathBaseName(suggestedPath) || "untitled.txt";
    setModalOpen($("opener"), true);
    browse(dir);
    queueMicrotask(() => {
      inp.focus();
      inp.select();
    });
  });
}

function configureOpener(mode, title) {
  state.openerMode = mode;
  const save = mode === "save";
  const m = $("opener");
  m.classList.toggle("save-mode", save);
  $("opener-title").textContent = translateText(
    title || (save ? "名前を付けて保存" : "ファイルを開く"),
  );
  $("opener-input-label").textContent = save ? t("ファイル名") : t("パス");
  $("opener-input").placeholder = save
    ? t("保存するファイル名、またはフルパス")
    : t("ファイルのパスを入力… (例: /var/log/huge.log)");
  $("opener-open").textContent = save ? t("保存") : t("開く");
  $("opener-folder").textContent = save ? t("場所") : t("フォルダ");
  $("opener-folder").title = save
    ? t("表示中のフォルダをエクスプローラーに表示")
    : t("表示中のフォルダをツリーに開く");
  $("opener-hint").textContent = save
    ? t("フォルダを選び、保存するファイル名を入力します。既存ファイルを選ぶと上書き確認します。")
    : t(
        "ここへファイルをドラッグ＆ドロップしても開けます。大きなファイルはパス指定の方が高速です。",
      );
  openerMsg("");
  renderRecentFiles();
}

function hideOpener() {
  if (state.openerMode === "save") {
    finishSaveDialog(null);
    return;
  }
  // The opener doubles as the welcome screen: don't let it close while there is
  // no document to fall back to.
  if (!state.stat?.open) return;
  setModalOpen($("opener"), false);
  focusEditor();
}

function finishSaveDialog(value) {
  const resolve = state.openerResolve;
  state.openerResolve = null;
  state.openerMode = "open";
  setModalOpen($("opener"), false);
  configureOpener("open");
  focusEditor();
  if (resolve) resolve(value);
}

function openerMsg(text, busy = false) {
  const el = $("opener-msg");
  el.textContent = translateText(text || "");
  el.classList.toggle("busy", !!text && busy);
}

async function browse(dir) {
  openerMsg("読み込み中…", true);
  try {
    const q = dir == null ? "" : `?dir=${encodeURIComponent(dir)}`;
    const res = await api(`/api/browse${q}`);
    renderBrowse(res);
    openerMsg("");
  } catch (e) {
    openerMsg("ディレクトリを開けません: " + e.message);
  }
}

function renderBrowse(res) {
  state.openerDir = res.dir;
  state.openerEntries = res.entries || [];
  renderCwdCrumbs(res.dir);
  const list = $("opener-list");
  list.textContent = "";
  if (res.parent) {
    list.append(browseRow({ name: "..", path: res.parent, is_dir: true }, true));
  }
  for (const ent of res.entries) list.append(browseRow(ent, false));
  list.scrollTop = 0;
}

function renderCwdCrumbs(path) {
  const cwd = $("opener-cwd");
  const clean = String(path || "").replace(/^\\\\\?\\/, "");
  cwd.textContent = "";
  cwd.title = clean;
  for (const [i, crumb] of pathCrumbs(clean).entries()) {
    if (i > 0) {
      const sep = document.createElement("span");
      sep.className = "cwd-sep";
      sep.setAttribute("aria-hidden", "true");
      sep.append(iconSvg("i-chevron-right"));
      cwd.append(sep);
    }
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "cwd-crumb";
    btn.textContent = crumb.label;
    btn.title = crumb.path;
    btn.addEventListener("click", () => browse(crumb.path));
    cwd.append(btn);
  }
}

function browseRow(ent, isUp) {
  const row = document.createElement("button");
  row.className = "opener-row" + (ent.is_dir ? " dir" : "") + (isUp ? " up" : "");
  row.type = "button";
  // The kind ("フォルダ" / "ファイル") moved from visible text into the icon;
  // keep it for screen readers via the row's accessible name.
  row.setAttribute(
    "aria-label",
    isUp ? t("上の階層へ") : `${ent.is_dir ? t("フォルダ") : t("ファイル")}: ${ent.name}`,
  );
  const ic = document.createElement("span");
  ic.className = "ic";
  ic.setAttribute("aria-hidden", "true");
  ic.append(iconSvg(isUp ? "i-folder-up" : ent.is_dir ? "i-folder" : "i-file"));
  const nm = document.createElement("span");
  nm.className = "nm";
  nm.textContent = isUp ? t("上の階層へ") : ent.name;
  const sz = document.createElement("span");
  sz.className = "sz";
  sz.textContent = ent.is_dir ? "" : humanBytes(ent.size);
  row.append(ic, nm, sz);
  row.addEventListener("click", () => {
    if (ent.is_dir) browse(ent.path);
    else if (state.openerMode === "save") {
      $("opener-input").value = ent.name;
      markPickedFile(ent.name);
      $("opener-input").focus();
    } else openPath(ent.path);
  });
  row.addEventListener("dblclick", () => {
    if (!ent.is_dir && state.openerMode === "save") commitOpener();
  });
  return row;
}

// ---- recent files (最近使ったファイル) --------------------------------------
//
// A best-effort, browser-only history of recently opened paths. Kept in
// localStorage (most-recent-first, deduped, capped) so it survives reloads
// without any server/state changes. Surfaced as a shortcut list in the opener.

function loadRecentFiles() {
  try {
    const raw = JSON.parse(localStorage.getItem(RECENT_KEY) || "[]");
    return Array.isArray(raw) ? raw.filter((x) => typeof x === "string").slice(0, RECENT_MAX) : [];
  } catch {
    return [];
  }
}

function saveRecentFiles(list) {
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(list.slice(0, RECENT_MAX)));
  } catch {
    // ignore private-mode quota errors; recents are best-effort
  }
}

// Record a freshly opened file at the head of the list. Untitled scratch
// buffers never qualify.
function pushRecentFile(path) {
  const p = (path || "").trim();
  if (!p || isUntitled(p)) return;
  const list = [p, ...loadRecentFiles().filter((x) => x !== p)].slice(0, RECENT_MAX);
  saveRecentFiles(list);
}

// Forget a path (e.g. it no longer opens) so the list stays trustworthy.
function dropRecentFile(path) {
  saveRecentFiles(loadRecentFiles().filter((x) => x !== path));
}

// Open a recent entry through the normal open path; drop it if it's gone.
async function openRecent(path) {
  const ok = await openPath(path);
  if (!ok) {
    dropRecentFile(path);
    renderRecentFiles();
  }
}

function renderRecentFiles() {
  const box = $("opener-recent");
  if (!box) return;
  // The recent shortcut only makes sense when opening, not when saving.
  const list = state.openerMode === "save" ? [] : loadRecentFiles();
  box.textContent = "";
  if (!list.length) {
    box.classList.add("hidden");
    return;
  }
  const head = document.createElement("div");
  head.className = "opener-recent-head";
  head.textContent = t("最近使ったファイル");
  box.append(head);
  for (const path of list) box.append(recentRow(path));
  box.classList.remove("hidden");
}

function recentRow(path) {
  const row = document.createElement("button");
  row.className = "opener-row recent";
  row.type = "button";
  row.title = path;
  row.setAttribute("aria-label", `${t("最近使ったファイル")}: ${pathBaseName(path) || path}`);
  const ic = document.createElement("span");
  ic.className = "ic";
  ic.setAttribute("aria-hidden", "true");
  ic.append(iconSvg("i-clock"));
  const nm = document.createElement("span");
  nm.className = "nm";
  nm.textContent = pathBaseName(path) || path;
  const dir = document.createElement("span");
  dir.className = "sz";
  dir.textContent = pathDirName(path) || "";
  row.append(ic, nm, dir);
  row.addEventListener("click", () => openRecent(path));
  return row;
}

function markPickedFile(name) {
  for (const row of $("opener-list").querySelectorAll(".opener-row")) {
    row.classList.toggle("picked", row.querySelector(".nm")?.textContent === name);
  }
}

async function saveDialogTarget() {
  const raw = $("opener-input").value.trim();
  if (!raw) {
    openerMsg("保存するファイル名を入力してください");
    return null;
  }
  const path = isAbsolutePath(raw) ? raw : joinPath(state.openerDir, raw);
  const base = pathBaseName(path);
  const existing = state.openerEntries.find((e) => !e.is_dir && e.name === base);
  const overwrite = !!existing;
  if (overwrite) {
    const ok = await askConfirm("上書きの確認", `${base} は既に存在します。上書きしますか?`, {
      okLabel: "上書き",
      danger: true,
    });
    if (!ok) return null;
  }
  return { path, overwrite };
}

async function commitOpener() {
  if (state.openerMode === "save") {
    const target = await saveDialogTarget();
    if (target) finishSaveDialog(target);
    return;
  }
  openPath($("opener-input").value);
}

// A pristine untitled buffer (never typed in, never saved) is replaced when a
// real file is opened, Notepad++/VS Code-style — otherwise every launch would
// leave an empty "untitled" tab dangling next to the opened file.
function pristineUntitledTabId() {
  const active = (state.tabs || []).find((t) => t.active);
  if (!active || active.dirty || !isUntitled(active.path)) return null;
  if (state.stat?.dirty || state.stat?.can_undo) return null;
  return active.id;
}

async function closeTabSilently(id) {
  if (id == null) return;
  try {
    // Re-check against the server's current truth: only a still-open,
    // background, still-clean tab is closed.
    const r = await api("/api/tabs");
    const tab = (r.tabs || []).find((t) => t.id === id);
    if (!tab || tab.active || tab.dirty) return;
    await apiPost("/api/tabs/close", { id });
    refreshTabs();
  } catch {
    // non-fatal: the extra tab just stays
  }
}

async function openPath(path) {
  const p = (path || "").trim();
  if (!p) return false;
  await settleEditQueue();
  const pristine = pristineUntitledTabId();
  openerMsg("開いています…", true);
  try {
    const stat = await apiPost("/api/open", { path: p });
    onDocumentOpened(stat);
    await closeTabSilently(pristine);
    return true;
  } catch (e) {
    reportOpenError("開けません: " + e.message);
    return false;
  }
}

async function uploadFile(file) {
  await settleEditQueue();
  const pristine = pristineUntitledTabId();
  openerMsg(`読み込み中… (${file.name})`, true);
  showLoading(`読み込み中… ${file.name}`);
  try {
    const r = await fetch(`/api/upload?name=${encodeURIComponent(file.name)}`, {
      method: "POST",
      body: file,
    });
    if (!r.ok) throw new Error((await r.text()) || r.statusText);
    onDocumentOpened(await r.json());
    await closeTabSilently(pristine);
  } catch (e) {
    reportOpenError("読み込みエラー: " + e.message);
  } finally {
    hideLoading();
  }
}

// Surface an open/upload failure where the user is looking: inside the opener if
// it's up, otherwise in the toolbar (and an alert if a doc is already open).
function reportOpenError(msg) {
  if (openerVisible()) {
    openerMsg(msg);
  } else if (state.stat?.open) {
    flashCount("読み込みエラー", "error");
    showMessage("読み込みエラー", msg);
  } else {
    showOpener();
    openerMsg(msg);
  }
}

// ---- crash recovery (server-side WAL) ---------------------------------------

// Guard: one recoverable document produces one dialog, even if open/select
// events race while the modal is up.
let walPromptBusy = false;

// The server found a crash log with unsaved edits for the just-opened
// document (stat.recoverable). Nothing is applied automatically: offer the
// choice — 復元 replays the log into the live session, 破棄 deletes it.
async function maybeOfferWalRecovery(stat) {
  const n = stat?.recoverable;
  if (!n || walPromptBusy) return;
  walPromptBusy = true;
  try {
    const restore = await askConfirm(
      "クラッシュ復元",
      `クラッシュ前の未保存の編集が見つかりました（${commas(n)}件）。復元しますか？`,
      { okLabel: "復元する", cancelLabel: "破棄" },
    );
    await apiPost("/api/edit/recover", restore ? {} : { discard: true });
    clearLineCache();
    await refreshStat();
    await reloadViewport();
    render();
    if (restore) {
      flashCount(`クラッシュ前の編集を復元しました（${commas(n)}件）`);
    } else {
      flashCount("クラッシュ前の編集を破棄しました");
    }
  } catch (e) {
    flashCount("復元エラー", "error");
    console.error(e);
  } finally {
    walPromptBusy = false;
  }
}

function onDocumentOpened(stat) {
  state.docGen++;
  state.editGen++; // stale in-flight edit responses must not reposition this tab
  setFollowTail(false); // following is per-document; a new doc/tab starts un-followed
  state.stat = stat;
  pushRecentFile(stat.path);
  state.total = stat.view_lines ?? stat.lines ?? 0;
  // Fresh document: reset navigation, search, and caret state.
  state.first = 0;
  state.caret = { line: 0, col: 0 };
  state.goalCol = 0;
  state.activeLine = 0;
  state.sel = null;
  state.extraCursors = [];
  state.lastMatch = null;
  state.searchHits = null;
  state.searchTruncated = false;
  $("find-count").textContent = "";
  clearLineCache();
  setModalOpen($("opener"), false);
  updateStatusMeta();
  render();
  refreshTabs();
  updateTreeActive();
  focusEditor();
  noteWalError(stat);
  maybeOfferWalRecovery(stat); // async on purpose: the open itself is done
}

function hasFiles(e) {
  const t = e.dataTransfer;
  return !!t && Array.from(t.types || []).includes("Files");
}

function initDropZone() {
  const dz = $("dropzone");
  let depth = 0;
  window.addEventListener("dragenter", (e) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    depth++;
    dz.classList.remove("hidden");
  });
  window.addEventListener("dragover", (e) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
  });
  window.addEventListener("dragleave", (e) => {
    if (!hasFiles(e)) return;
    depth = Math.max(0, depth - 1);
    if (depth === 0) dz.classList.add("hidden");
  });
  window.addEventListener("drop", (e) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    depth = 0;
    dz.classList.add("hidden");
    const file = e.dataTransfer.files[0];
    if (file) uploadFile(file);
  });
}

// ---- tabs ------------------------------------------------------------------

async function refreshTabs() {
  try {
    const r = await api("/api/tabs");
    renderTabs(r.tabs);
  } catch {
    // non-fatal: the tab bar just won't update
  }
}

function renderTabs(list) {
  state.tabs = list;
  const c = $("tabs");
  c.setAttribute("role", "tablist");
  c.textContent = "";
  for (const t of list) {
    const el = document.createElement("div");
    el.className = "tab" + (t.active ? " active" : "") + (t.dirty ? " dirty" : "");
    el.dataset.id = String(t.id);
    el.title = displayPath(t.path);
    el.setAttribute("role", "tab");
    el.setAttribute("aria-selected", t.active ? "true" : "false");
    el.tabIndex = 0;
    const dot = document.createElement("span");
    dot.className = "tab-dot";
    const nm = document.createElement("span");
    nm.className = "tab-name";
    nm.textContent = t.name;
    const x = document.createElement("button");
    x.type = "button";
    x.className = "tab-x";
    x.append(iconSvg("i-close"));
    x.title = translateText("閉じる");
    x.setAttribute("aria-label", translateText(`${t.name} を閉じる`));
    el.append(dot, nm, x);
    el.addEventListener("click", () => {
      if (!t.active) selectTab(t.id);
    });
    el.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        if (!t.active) selectTab(t.id);
      } else if (e.key === "Delete") {
        e.preventDefault();
        closeTab(t.id);
      }
    });
    el.addEventListener("mousedown", (e) => {
      if (e.button === 1) {
        e.preventDefault();
        closeTab(t.id); // middle-click closes
      }
    });
    x.addEventListener("click", (e) => {
      e.stopPropagation();
      closeTab(t.id);
    });
    c.append(el);
  }
}

async function selectTab(id) {
  try {
    await settleEditQueue();
    onDocumentOpened(await apiPost("/api/tabs/select", { id }));
  } catch (e) {
    flashCount("タブ切替エラー");
    console.error(e);
  }
}

async function closeTab(id) {
  await settleEditQueue();
  const t = state.tabs.find((x) => x.id === id);
  const isLast = (state.tabs || []).length <= 1;
  if (isLast) {
    if (savingCount > 0) {
      flashCount("保存中です — 完了までお待ちください");
      return;
    }
    if (!(await confirmCloseLastTab(t))) return;
    if (requestEditorClose()) return;
  } else if (t && t.dirty) {
    const ok = await askConfirm("タブを閉じる", `${t.name} の未保存の編集を破棄して閉じますか?`, {
      okLabel: "破棄して閉じる",
      danger: true,
    });
    if (!ok) return;
  }
  try {
    const stat = await apiPost("/api/tabs/close", { id });
    if (!stat.open) {
      await newUntitled(); // closed the last tab → open a fresh page
    } else {
      onDocumentOpened(stat);
    }
  } catch (e) {
    flashCount("タブを閉じられません");
    console.error(e);
  }
}

// ---- sidebar file tree ------------------------------------------------------

function sidebarOpen() {
  return !$("sidebar").classList.contains("hidden");
}

function setSidebar(open) {
  $("sidebar").classList.toggle("hidden", !open);
  $("toggle-sidebar").classList.toggle("on", open);
  state.settings = { ...state.settings, sidebar: open };
  saveSettings(state.settings);
  if (open && !state.treeLoaded) {
    state.treeLoaded = true;
    treeSetRoot(localStorage.getItem(TREE_KEY) || null);
  }
  scheduleRender(); // viewport width changed
}

// Load `dir` (or the server default when null) as the tree root.
async function treeSetRoot(dir) {
  try {
    const q = dir ? `?dir=${encodeURIComponent(dir)}` : "";
    const res = await api(`/api/browse${q}`);
    state.treeParent = res.parent;
    $("sb-root").textContent = displayPath(res.dir);
    $("sb-root").title = displayPath(res.dir);
    try {
      localStorage.setItem(TREE_KEY, res.dir);
    } catch {
      // ignore quota
    }
    const tree = $("tree");
    tree.textContent = "";
    tree.append(renderTreeEntries(res.entries, 0));
  } catch {
    // A stale saved root: fall back to the server default once.
    if (dir) {
      treeSetRoot(null);
    } else {
      $("tree").textContent = "";
    }
  }
}

function renderTreeEntries(entries, depth) {
  const frag = document.createDocumentFragment();
  for (const ent of entries) frag.append(renderTreeNode(ent, depth));
  return frag;
}

function renderTreeNode(ent, depth) {
  const row = document.createElement("div");
  row.className = "tnode " + (ent.is_dir ? "dir" : "file");
  row.dataset.path = ent.path;
  row.style.setProperty("--depth", String(depth));
  if (!ent.is_dir && ent.path === state.stat?.path) row.classList.add("active");
  const indent = document.createElement("span");
  indent.className = "tindent";
  for (let i = 0; i < depth; i++) {
    const guide = document.createElement("span");
    guide.className = "tguide";
    indent.append(guide);
  }
  const chev = document.createElement("span");
  chev.className = "chev";
  chev.setAttribute("aria-hidden", "true");
  const icon = document.createElement("span");
  icon.className = "ticon " + (ent.is_dir ? "folder" : `file ${treeFileClass(ent.name)}`);
  icon.setAttribute("aria-hidden", "true");
  const nm = document.createElement("span");
  nm.className = "tname";
  nm.textContent = ent.name;
  row.append(indent, chev, icon, nm);

  if (!ent.is_dir) {
    if (typeof ent.size === "number") {
      const meta = document.createElement("span");
      meta.className = "tmeta";
      meta.textContent = humanBytes(ent.size);
      row.append(meta);
    }
    row.title = displayPath(ent.path);
    row.addEventListener("click", (e) => {
      e.stopPropagation();
      // Opens in a new tab; a file that is already open just gets focused
      // (the server dedupes by path).
      openPath(ent.path);
    });
    return row;
  }

  // Folder: lazily load children on first expand.
  const kids = document.createElement("div");
  kids.className = "tkids";
  kids.style.display = "none";
  let loaded = false;
  row.addEventListener("click", async (e) => {
    e.stopPropagation();
    const opening = kids.style.display === "none";
    row.classList.toggle("open", opening);
    if (opening && !loaded) {
      loaded = true;
      try {
        const res = await api(`/api/browse?dir=${encodeURIComponent(ent.path)}`);
        kids.append(renderTreeEntries(res.entries, depth + 1));
      } catch {
        loaded = false;
      }
    }
    kids.style.display = opening ? "block" : "none";
  });
  const frag = document.createDocumentFragment();
  frag.append(row, kids);
  return frag;
}

function treeFileClass(name) {
  const ext =
    String(name || "")
      .split(".")
      .pop()
      ?.toLowerCase() || "";
  if (ext === "md" || ext === "markdown") return "md";
  if (ext === "py") return "py";
  if (ext === "json") return "json";
  if (ext === "csv" || ext === "tsv" || ext === "xlsx") return "data";
  return "text";
}

function updateTreeActive() {
  const path = state.stat?.path || "";
  document.querySelectorAll("#tree .tnode.file").forEach((row) => {
    row.classList.toggle("active", !!path && row.dataset.path === path);
  });
}

function initTree() {
  $("toggle-sidebar").addEventListener("click", () => setSidebar(!sidebarOpen()));
  $("sb-close").addEventListener("click", () => setSidebar(false));
  $("sb-up").addEventListener("click", () => {
    if (state.treeParent) treeSetRoot(state.treeParent);
  });
  $("opener-folder").addEventListener("click", () => {
    if (!state.openerDir) return;
    if (!sidebarOpen()) setSidebar(true);
    state.treeLoaded = true;
    treeSetRoot(state.openerDir);
    if (state.openerMode === "save") {
      openerMsg("現在のフォルダをエクスプローラーに表示しました");
      return;
    }
    hideOpener();
  });
  // Apply persisted visibility.
  if (state.settings.sidebar) setSidebar(true);
}

// Start a fresh empty "untitled" buffer with a blank editable first line, so
// the app opens to a usable page (like Notepad) instead of a dialog.
async function newUntitled() {
  try {
    await settleEditQueue();
    onDocumentOpened(await apiPost("/api/new", {}));
    // The buffer already has one empty line; drop the caret in, Notepad-style.
    setCaret(0, 0);
    focusEditor();
  } catch (e) {
    showOpener();
    openerMsg("新規バッファを作成できません: " + e.message);
  }
}

function runMenuAction(action) {
  hideFileMenu();
  // A modal owns the UI. Every menu action either opens a dialog or acts on
  // the document hidden behind the modal, and the native macOS menu can fire
  // at any time — so ALL actions are ignored while a modal is open. (In-page
  // menus are unreachable then; this guards the native path.)
  if (anyModalOpen()) return;
  if (action === "commandPalette") return showCommandPalette();
  if (action === "undo") return undoEdit();
  if (action === "redo") return redoEdit();
  if (action === "find") return showFind();
  if (action === "replace") return showFind(true);
  if (action === "gotoLine") {
    askPrompt("行へ移動", "行番号").then((v) => {
      if (v != null) gotoLine(v);
    });
    return;
  }
  if (action === "selectAll") return selectAll();
  if (action === "selectNextOccurrence") return selectNextOccurrence();
  if (action === "addCursorAbove") return addCursorAbove();
  if (action === "addCursorBelow") return addCursorBelow();
  if (action === "duplicateLine") return duplicateLines();
  if (action === "moveLineUp") return moveLines(-1);
  if (action === "moveLineDown") return moveLines(1);
  if (action === "deleteLine") return deleteLines();
  if (action === "copy") return copySelection();
  if (action === "cut") return cutSelection();
  if (action === "toggleSidebar") return setSidebar(!sidebarOpen());
  if (action === "toggleWhitespace")
    return updateSetting("showWhitespace", !state.settings.showWhitespace);
  if (action === "toggleZenkakuUnderline")
    return updateSetting("zenkakuUnderline", !state.settings.zenkakuUnderline);
  if (action === "toggleWordWrap") return updateSetting("wordWrap", !state.settings.wordWrap);
  if (action === "toggleFollowTail") return setFollowTail(!state.followTail);
  if (action === "settings") return showSettings();
  if (action === "sortSave") return sortSave();
  if (action === "diffFile") return diffFile();
  if (action === "splitFile") return splitFile();
  if (action === "grepFolder") return grepFolder();
  if (action === "caseUpper") return transformSelection("upper");
  if (action === "caseLower") return transformSelection("lower");
  if (action === "keymap") return showKeymap();
  if (action === "newFile") return newUntitled();
  if (action === "newWindow") return openNewWindow();
  if (action === "openFile") return showOpener();
  if (action === "saveFile") return saveFile();
  if (action === "saveAs") return saveCopy();
  if (action === "closeTab") {
    const active = state.tabs.find((t) => t.active);
    if (active) closeTab(active.id);
  }
}

// Native menu dispatcher: the macOS (Rust) side calls this via evaluate_script
// with the same action ids the in-page menus use.
window.__ayameMenu = runMenuAction;

function initMenuBar() {
  for (const id of APP_MENUS) {
    const button = $(`${id}-menu-button`);
    button.addEventListener("click", (e) => {
      e.stopPropagation();
      const open = !$(`${id}-menu`).classList.contains("hidden");
      if (open) hideFileMenu();
      else showAppMenu(id);
    });
    button.addEventListener("pointerenter", () => {
      if (fileMenuVisible()) showAppMenu(id);
    });
  }
  document.querySelectorAll("[data-menu-action]").forEach((item) => {
    item.addEventListener("click", () => runMenuAction(item.dataset.menuAction));
  });
}

function initWorkspace() {
  initMenuBar();
  document.addEventListener("pointerdown", (e) => {
    if (fileMenuVisible() && !e.target.closest(".menu-shell")) hideFileMenu();
  });
  $("new-file").addEventListener("click", () => {
    hideFileMenu();
    newUntitled();
  });
  $("open-file").addEventListener("click", () => {
    hideFileMenu();
    showOpener();
  });
  $("opener-close").addEventListener("click", hideOpener);
  $("opener-open").addEventListener("click", commitOpener);
  $("opener-input").addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      commitOpener();
    } else if (e.key === "Escape") {
      e.preventDefault();
      hideOpener();
    }
  });
  // Click on the dim backdrop (outside the panel) closes the dialog.
  $("opener").addEventListener("click", (e) => {
    if (e.target === $("opener")) hideOpener();
  });
  $("new-tab").addEventListener("click", () => newUntitled());
  initDropZone();
}

// ---- settings (theme / font) -----------------------------------------------

function loadSettings() {
  try {
    const raw = JSON.parse(localStorage.getItem(SETTINGS_KEY) || "{}");
    const merged = { ...DEFAULT_SETTINGS, ...(raw && typeof raw === "object" ? raw : {}) };
    merged.sidebarSide = merged.sidebarSide === "right" ? "right" : "left";
    merged.language = normalizeLanguage(merged.language);
    merged.keymap = sanitizeKeymap(merged.keymap);
    return merged;
  } catch {
    return { ...DEFAULT_SETTINGS };
  }
}

function saveSettings(s) {
  try {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(s));
  } catch {
    // ignore private-mode quota errors
  }
}

// Built-in themes are also defined as CSS `html[data-theme=...]` blocks in
// style.css; these JSON mirrors let the Settings JSON editor show/export them
// and act as a base for custom themes. Custom themes apply at runtime by
// setting the same CSS variables the built-ins use.
const THEME_PRESETS = {
  "iris-light": {
    name: "Iris Light",
    type: "light",
    radius: 10,
    color: {
      paper: "#FBF8F1",
      paper2: "#FDFCF8",
      ink: "#2A2140",
      inkDim: "#6E6383",
      inkFaint: "#A99DBC",
      accent: "#7A5CC0",
      accent2: "#6A4CB0",
      gold: "#C79A2E",
      edge: "#E7E0D3",
      err: "#C0506A",
      markBg: "#FBEBB0",
      markFg: "#6B5510",
      markCur: "#E8B84B",
      markCurFg: "#2A2205",
    },
    acrylic: { tint: "rgba(255,253,248,0.72)", blur: 20 },
    background: { mode: "watercolor", solid: "#FBF8F1" },
    illustration: 0.18,
    watercolor: [
      { x: "12%", y: "84%", r: "46vh", color: "rgba(122,92,192,0.12)" },
      { x: "88%", y: "14%", r: "42vh", color: "rgba(185,139,214,0.10)" },
      { x: "70%", y: "96%", r: "30vh", color: "rgba(231,197,107,0.08)" },
    ],
  },
  "iris-mist": {
    name: "Iris Mist",
    type: "light",
    radius: 12,
    color: {
      paper: "#F7F9FC",
      paper2: "#FDFEFF",
      ink: "#26314A",
      inkDim: "#5E6E8A",
      inkFaint: "#9DAAC0",
      accent: "#5B79C9",
      accent2: "#4A68B8",
      gold: "#C9A24E",
      edge: "#DCE4EF",
      err: "#C05C74",
      markBg: "#E3ECFB",
      markFg: "#2C3E6B",
      markCur: "#7EC7C0",
      markCurFg: "#0F2A28",
    },
    acrylic: { tint: "rgba(250,252,255,0.68)", blur: 24 },
    background: { mode: "watercolor", solid: "#F7F9FC" },
    illustration: 0.22,
    watercolor: [
      { x: "14%", y: "82%", r: "44vh", color: "rgba(91,121,201,0.12)" },
      { x: "86%", y: "16%", r: "42vh", color: "rgba(143,182,224,0.10)" },
      { x: "74%", y: "96%", r: "30vh", color: "rgba(126,199,192,0.08)" },
    ],
  },
  "iris-dawn": {
    name: "Iris Dawn",
    type: "light",
    radius: 10,
    color: {
      paper: "#FDF6EE",
      paper2: "#FFFBF7",
      ink: "#3A2438",
      inkDim: "#7A5A6E",
      inkFaint: "#B79AA6",
      accent: "#A65CB0",
      accent2: "#944EA0",
      gold: "#E0A94E",
      edge: "#EFE0D6",
      err: "#D96A86",
      markBg: "#FBE7C8",
      markFg: "#7A4A16",
      markCur: "#F0B85A",
      markCurFg: "#3A2205",
    },
    acrylic: { tint: "rgba(255,250,244,0.70)", blur: 20 },
    background: { mode: "watercolor", solid: "#FDF6EE" },
    illustration: 0.22,
    watercolor: [
      { x: "12%", y: "84%", r: "46vh", color: "rgba(166,92,176,0.13)" },
      { x: "84%", y: "16%", r: "42vh", color: "rgba(224,169,78,0.11)" },
      { x: "70%", y: "96%", r: "30vh", color: "rgba(227,154,176,0.10)" },
    ],
  },
  "sumi-light": {
    name: "Sumi Light",
    type: "light",
    radius: 10,
    color: {
      paper: "#FAFAF8",
      paper2: "#FFFFFF",
      ink: "#222024",
      inkDim: "#63616A",
      inkFaint: "#A7A4AE",
      accent: "#7A5CC0",
      accent2: "#6A4CB0",
      gold: "#B7912F",
      edge: "#E6E4DE",
      err: "#B24A5E",
      markBg: "#ECE6FA",
      markFg: "#3E2E63",
      markCur: "#7A5CC0",
      markCurFg: "#FFFFFF",
    },
    acrylic: { tint: "rgba(252,252,250,0.74)", blur: 22 },
    background: { mode: "watercolor", solid: "#FAFAF8" },
    illustration: 0.16,
    watercolor: [
      { x: "16%", y: "82%", r: "40vh", color: "rgba(122,92,192,0.07)" },
      { x: "84%", y: "20%", r: "34vh", color: "rgba(40,36,48,0.03)" },
    ],
  },
  "mono-paper": {
    name: "Mono Paper (単色)",
    type: "light",
    radius: 10,
    color: {
      paper: "#F5F3ED",
      paper2: "#FBFAF5",
      ink: "#24231F",
      inkDim: "#6C6A63",
      inkFaint: "#A9A69D",
      accent: "#6F6B79",
      accent2: "#605C6C",
      gold: "#7A7568",
      edge: "#E2DFD6",
      err: "#9A6A6A",
      markBg: "#E7E4EC",
      markFg: "#3A3745",
      markCur: "#6F6B79",
      markCurFg: "#FFFFFF",
    },
    acrylic: { tint: "rgba(245,243,237,0.92)", blur: 8 },
    background: { mode: "solid", solid: "#F4F2EC" },
    illustration: 0,
    watercolor: [],
  },
};

// CSS variables a custom/JSON theme drives (cleared when switching back to a
// built-in data-theme so its CSS block wins).
const THEME_VARS = [
  "--bg",
  "--bg-elevated",
  "--bg-toolbar",
  "--bg-active-line",
  "--gutter-bg",
  "--edit-bg",
  "--fg",
  "--fg-dim",
  "--fg-faint",
  "--border",
  "--accent",
  "--accent-bright",
  "--status",
  "--status-fg",
  "--gutter-fg",
  "--mark-bg",
  "--mark-fg",
  "--mark-active-bg",
  "--mark-active-fg",
  "--danger",
  "--gold",
  "--desk",
  "--illus",
  "--radius",
  "--acrylic-blur",
];
function clearCustomVars() {
  const r = document.documentElement.style;
  THEME_VARS.forEach((v) => r.removeProperty(v));
}
function deskFrom(t) {
  const bg = t.background || { mode: "watercolor" };
  if (bg.mode === "solid") return bg.solid || t.color.paper2 || t.color.paper;
  const layers = (t.watercolor || []).map(
    (b) => `radial-gradient(${b.r} ${b.r} at ${b.x} ${b.y}, ${b.color}, transparent 62%)`,
  );
  layers.push(t.color.paper);
  return layers.join(", ");
}
function applyCustomVars(t) {
  const r = document.documentElement.style,
    c = t.color || {};
  const S = (k, v) => v != null && r.setProperty(k, v);
  S("--bg", c.paper);
  S("--bg-elevated", c.paper2 || c.paper);
  S("--bg-toolbar", (t.acrylic && t.acrylic.tint) || c.paper);
  S("--bg-active-line", `color-mix(in srgb, ${c.accent} 14%, ${c.paper})`);
  S("--gutter-bg", c.paper);
  S("--edit-bg", c.paper2 || c.paper);
  S("--fg", c.ink);
  S("--fg-dim", c.inkDim);
  S("--fg-faint", c.inkFaint);
  S("--border", c.edge);
  S("--accent", c.accent);
  S("--accent-bright", c.accent2 || c.accent);
  S("--status", (t.acrylic && t.acrylic.tint) || c.paper);
  S("--status-fg", c.inkDim);
  S("--gutter-fg", c.inkFaint);
  S("--mark-bg", c.markBg);
  S("--mark-fg", c.markFg);
  S("--mark-active-bg", c.markCur);
  S("--mark-active-fg", c.markCurFg);
  S("--danger", c.err);
  S("--gold", c.gold);
  S("--radius", (t.radius || 10) + "px");
  S("--acrylic-blur", ((t.acrylic && t.acrylic.blur) ?? 20) + "px");
  S("--desk", deskFrom(t));
  S("--illus", String(t.illustration ?? 0.2));
}

function applySettings(s) {
  const root = document.documentElement;
  // ---- theme (built-in CSS block, or a custom JSON theme at runtime) ----
  clearCustomVars();
  if (s.theme && s.theme.startsWith("custom:")) {
    const t = (s.customThemes || {})[s.theme.slice(7)];
    root.dataset.theme = "custom";
    if (t) applyCustomVars(t);
  } else {
    root.dataset.theme = s.theme || "iris-light"; // iris-* | dark | black (unknown → :root)
  }
  root.dataset.sidebarSide = s.sidebarSide === "right" ? "right" : "left";
  // ---- whitespace glyphs: swap the zenkaku-space box for an underline ----
  root.classList.toggle("zenkaku-underline", !!s.zenkakuUnderline);
  // ---- background mode + illustration (user overrides on top of the theme) ----
  if (s.bgMode === "solid") {
    const flat = getComputedStyle(root).getPropertyValue("--bg").trim() || "#FBF8F1";
    root.style.setProperty("--desk", flat);
  }
  if (typeof s.illus === "number") root.style.setProperty("--illus", String(s.illus));
  // ---- font / size ----
  root.style.setProperty("--mono", FONT_STACKS[s.font] || FONT_STACKS.mono);
  const fs = Math.max(11, Math.min(22, Number(s.fontSize) || 13));
  root.style.setProperty("--font-size", `${fs}px`);
  const lh = fs + 6;
  root.style.setProperty("--line-height", `${lh}px`);
  LINE_HEIGHT = lh; // keep virtualization math in sync with the CSS
  _charW = 0; // font metrics changed → remeasure on next click
  _rulerKey = ""; // force the ruler to rebuild against the new metrics
  // ---- long-line wrapping (折り返し) ----
  // Purely a CSS switch on #content: rows go white-space:pre-wrap and grow past
  // one LINE_HEIGHT so long lines wrap instead of scrolling horizontally. The
  // virtual scroll still steps one *logical* line per row, so files whose lines
  // fit the viewport render (and caret/select) exactly as before. See style.css
  // #content.wrap for the documented limitations on genuinely wrapped lines.
  $("content").classList.toggle("wrap", !!s.wordWrap);
  scheduleRender();
}

function updateSetting(key, value) {
  if (key === "language") value = normalizeLanguage(value);
  state.settings = { ...state.settings, [key]: value };
  applySettings(state.settings);
  saveSettings(state.settings);
  if (key === "sidebarSide") updateSidebarSideButtons();
  if (key === "language") applyLocale();
}

function settingsVisible() {
  return !$("settings").classList.contains("hidden");
}
function showSettings() {
  setModalOpen($("settings"), true);
}
function hideSettings() {
  setModalOpen($("settings"), false);
  focusEditor();
}

// ---- theme JSON editor (in Settings) --------------------------------------

function themeJSONFor(id) {
  if (id && id.startsWith("custom:"))
    return (state.settings.customThemes || {})[id.slice(7)] || null;
  return THEME_PRESETS[id] || null;
}
function themeIllusPct(id) {
  const t = themeJSONFor(id);
  return Math.round(((t && t.illustration) ?? 0) * 100);
}
function populateThemeSelect() {
  const sel = $("set-theme");
  [...sel.querySelectorAll("option[data-custom]")].forEach((o) => o.remove());
  for (const name of Object.keys(state.settings.customThemes || {})) {
    const o = document.createElement("option");
    o.value = "custom:" + name;
    o.textContent = "★ " + name;
    o.dataset.custom = "1";
    sel.appendChild(o);
  }
}
function persistCustomTheme(t) {
  const customs = { ...state.settings.customThemes };
  customs[t.name] = t;
  state.settings = {
    ...state.settings,
    customThemes: customs,
    theme: "custom:" + t.name,
    illus: null,
    bgMode: (t.background && t.background.mode) || "watercolor",
  };
  saveSettings(state.settings);
  populateThemeSelect();
  if ($("set-theme")) $("set-theme").value = "custom:" + t.name;
}

// Open the current theme's JSON as an ordinary editor tab, so it can be edited
// like any text file (edit / undo / Ctrl+S), then applied with テーマ適用.
async function openThemeJsonDoc() {
  const id = state.settings.theme;
  const t = themeJSONFor(id) || THEME_PRESETS["iris-light"];
  const jsonText = JSON.stringify(t, null, 2);
  const base = (id ? id.replace(/^custom:/, "") : "theme") || "theme";
  hideSettings();
  try {
    await settleEditQueue();
    const r = await fetch("/api/upload?name=" + encodeURIComponent(base + ".ayame-theme.json"), {
      method: "POST",
      body: jsonText,
    });
    if (!r.ok) throw new Error(await r.text());
    onDocumentOpened(await r.json());
  } catch (e) {
    flashCount("テーマを開けません");
    console.error(e);
  }
}

// Apply the theme JSON in the active buffer (a *.ayame-theme.json tab).
async function applyThemeFromBuffer() {
  try {
    const count = Math.min(state.total, MAX_COPY_LINES);
    const r = await api(`/api/lines?start=0&count=${count}`);
    const text = r.lines.map((l) => l.text).join("\n");
    const t = JSON.parse(text);
    if (!t.color) return flashCount("color がありません");
    document.documentElement.dataset.theme = "custom";
    clearCustomVars();
    applyCustomVars(t);
    if (t.name) persistCustomTheme(t);
    flashCount(`テーマ適用${t.name ? `: ${t.name}` : ""}`);
  } catch (e) {
    flashCount("テーマ JSON エラー");
    console.error(e);
  }
}
function isThemeDoc(path) {
  return !!path && /\.ayame-theme\.json$/i.test(path);
}

function keymapJSONForEditor() {
  const out = {};
  for (const [action] of KEYMAP_ACTIONS) {
    out[action] = Object.prototype.hasOwnProperty.call(state.settings.keymap || {}, action)
      ? state.settings.keymap[action]
      : DEFAULT_KEYMAP[action];
  }
  return out;
}

async function openKeymapJsonDoc() {
  hideKeymap();
  try {
    await settleEditQueue();
    const r = await fetch("/api/upload?name=" + encodeURIComponent("keymap.ayame-keys.json"), {
      method: "POST",
      body: JSON.stringify(keymapJSONForEditor(), null, 2),
    });
    if (!r.ok) throw new Error(await r.text());
    onDocumentOpened(await r.json());
  } catch (e) {
    flashCount("キー設定を開けません");
    console.error(e);
  }
}

async function applyKeymapFromBuffer() {
  try {
    const count = Math.min(state.total, MAX_COPY_LINES);
    const r = await api(`/api/lines?start=0&count=${count}`);
    const text = r.lines.map((l) => l.text).join("\n");
    const parsed = JSON.parse(text);
    const clean = sanitizeKeymap(parsed);
    state.settings = { ...state.settings, keymap: clean };
    saveSettings(state.settings);
    updateKeyHints();
    renderKeymapRows();
    flashCount("キー設定適用");
  } catch (e) {
    flashCount("キー設定 JSON エラー");
    console.error(e);
  }
}

function isKeymapDoc(path) {
  return !!path && /\.ayame-keys\.json$/i.test(path);
}

function updateSidebarSideButtons() {
  const side = state.settings.sidebarSide === "right" ? "right" : "left";
  document.querySelectorAll("button[data-sidebar-side]").forEach((btn) => {
    const on = btn.dataset.sidebarSide === side;
    btn.classList.toggle("on", on);
    btn.setAttribute("aria-pressed", on ? "true" : "false");
  });
}

function initSettings() {
  state.settings = loadSettings();
  applySettings(state.settings);
  populateThemeSelect();
  $("set-theme").value = state.settings.theme;
  $("set-bg").value = state.settings.bgMode || "watercolor";
  $("set-language").value = normalizeLanguage(state.settings.language);
  const illusPct =
    state.settings.illus == null
      ? themeIllusPct(state.settings.theme)
      : Math.round(state.settings.illus * 100);
  $("set-illus").value = illusPct;
  $("set-illus-val").textContent = illusPct + "%";
  $("set-font").value = state.settings.font;
  $("set-fontsize").value = state.settings.fontSize;
  $("set-fontsize-val").textContent = `${state.settings.fontSize}px`;

  $("set-theme").addEventListener("change", () => {
    const id = $("set-theme").value;
    state.settings = { ...state.settings, theme: id, illus: null };
    saveSettings(state.settings);
    applySettings(state.settings);
    const pct = themeIllusPct(id);
    $("set-illus").value = pct;
    $("set-illus-val").textContent = pct + "%";
  });
  $("set-bg").addEventListener("change", () => updateSetting("bgMode", $("set-bg").value));
  $("set-language").addEventListener("change", () =>
    updateSetting("language", $("set-language").value),
  );
  $("set-illus").addEventListener("input", () => {
    const v = Number($("set-illus").value);
    $("set-illus-val").textContent = v + "%";
    updateSetting("illus", v / 100);
  });
  $("set-font").addEventListener("change", () => updateSetting("font", $("set-font").value));
  $("set-fontsize").addEventListener("input", () => {
    const v = Number($("set-fontsize").value);
    $("set-fontsize-val").textContent = `${v}px`;
    updateSetting("fontSize", v);
  });
  $("set-ruler").checked = !!state.settings.ruler;
  $("set-ruler").addEventListener("change", () => updateSetting("ruler", $("set-ruler").checked));
  $("set-confirm-last-tab-close").checked = state.settings.confirmLastTabClose !== false;
  $("set-confirm-last-tab-close").addEventListener("change", () =>
    updateSetting("confirmLastTabClose", $("set-confirm-last-tab-close").checked),
  );
  $("set-memo-dir").value = state.settings.memoDir || "";
  $("set-memo-dir").addEventListener("input", () =>
    updateSetting("memoDir", $("set-memo-dir").value),
  );
  $("set-memo-name").value = state.settings.memoName || DEFAULT_SETTINGS.memoName;
  $("set-memo-name").addEventListener("input", () =>
    updateSetting("memoName", $("set-memo-name").value),
  );
  updateSidebarSideButtons();
  document.querySelectorAll("button[data-sidebar-side]").forEach((btn) => {
    btn.addEventListener("click", () => updateSetting("sidebarSide", btn.dataset.sidebarSide));
  });
  $("theme-json-edit").addEventListener("click", openThemeJsonDoc);
  $("keymap-open").addEventListener("click", showKeymap);
  $("keymap-close").addEventListener("click", hideKeymap);
  $("keymap-done").addEventListener("click", hideKeymap);
  $("keymap-reset").addEventListener("click", resetKeymap);
  $("keymap-json-edit").addEventListener("click", openKeymapJsonDoc);
  $("keymap-modal").addEventListener("click", (e) => {
    if (e.target === $("keymap-modal")) hideKeymap();
  });

  $("settings-close").addEventListener("click", hideSettings);
  $("settings").addEventListener("click", (e) => {
    if (e.target === $("settings")) hideSettings();
  });
  applyLocale();
}

// ---- boot ------------------------------------------------------------------

// Native window: open files dropped onto the window (real paths, no copy).
window.__ayameOpenNativePaths = (paths) => {
  if (!Array.isArray(paths)) return;
  (async () => {
    for (const p of paths) {
      if (typeof p !== "string" || !p) continue;
      try {
        await openPath(p);
      } catch (e) {
        flashCount(`開けません: ${p}`, "error");
        console.error(e);
      }
    }
  })();
};

async function boot() {
  state.history = loadSearchHistory();
  initSettings();
  initCommandPalette();
  initScrollbar();
  initEvents();
  initEditor();
  initSelection();
  initWorkspace();
  initTree();
  initContextMenu();
  try {
    await refreshStat();
  } catch (e) {
    $("overlay").classList.remove("hidden");
    $("overlay").textContent = `${t("サーバに接続できません")}: ${e.message}`;
    postNativeMessage("ayame:ready"); // still show the window so the error is visible
    return;
  }
  updateStatusMeta();
  // Native launch with a FILE argument: the window appears immediately and the
  // (possibly long) first-index happens behind this progress overlay.
  const pending = typeof window.__ayamePendingOpen === "string" ? window.__ayamePendingOpen : "";
  if (!state.stat.open && pending) {
    showLoading(`開いています: ${displayName(pending)} …`);
    postNativeMessage("ayame:ready");
    try {
      onDocumentOpened(await apiPost("/api/open", { path: pending }));
    } catch (e) {
      flashCount(`開けません: ${pending}`, "error");
      console.error(e);
      await newUntitled();
    } finally {
      hideLoading();
    }
    return;
  }
  if (!state.stat.open) {
    await newUntitled(); // open to a blank untitled page, not the file dialog
  } else {
    focusEditor();
    render();
    refreshTabs();
    // A document passed on the command line goes through refreshStat, not
    // onDocumentOpened — offer its crash recovery here.
    maybeOfferWalRecovery(state.stat);
  }
  postNativeMessage("ayame:ready");
}

boot();
