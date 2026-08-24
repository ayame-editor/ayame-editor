// Ayame Editor — folder grep dialog and result rendering.
import { $, button, commas, displayPath, el, pathDirName, setModalOpen } from "./dom.js";
import { BROWSE_KEY, state } from "./state.js";
import { serverMessage, t } from "./i18n.js";
import { apiPost } from "./api.js";
import { focusEditor, formatLineNo, lineNumberChars } from "./editor.js";
import { gotoLine } from "./edits.js";
import { askForm, hideLoading, showLoading, showMessage } from "./dialogs.js";
import { anyModalOpen } from "./modal-state.js";
import { openPath, showFolderDialog } from "./workspace.js";
import { flashCount } from "./notifications.js";
import { lastGrep } from "./grep-state.js";
import type { GrepRequest } from "./types/api.js";

export { lastGrep };

type GrepResponse = {
  hits: { path: string; line: number; col: number; text: string }[];
  truncated: boolean;
  files_scanned: number;
  files_truncated: boolean;
};

export function grepVisible() {
  return !$("grep-modal").classList.contains("hidden");
}

export function hideGrep() {
  setModalOpen($("grep-modal"), false);
  focusEditor();
}

export async function grepFolder() {
  if (anyModalOpen()) return;
  const base =
    lastGrep.dir ||
    localStorage.getItem(BROWSE_KEY) ||
    pathDirName(state.doc.stat?.path || "") ||
    "";
  const form = await askForm(
    t("menu.grep"),
    [
      {
        id: "query",
        type: "text",
        label: t("dialog.grep.query"),
        value: lastGrep.query,
        placeholder: t("dialog.grep.queryPlaceholder"),
      },
      {
        id: "dir",
        type: "path",
        label: t("dialog.grep.dir"),
        value: base,
        placeholder: t("dialog.grep.dirPlaceholder"),
        onBrowse: (cur) =>
          showFolderDialog(t("dialog.open.chooseFolder"), (cur || base || "").trim()),
      },
      {
        id: "glob",
        type: "text",
        label: t("dialog.grep.glob"),
        value: lastGrep.glob,
        placeholder: t("dialog.grep.globPlaceholder"),
      },
      { id: "ci", type: "check", label: t("dialog.grep.ignoreCase"), value: lastGrep.ci },
      { id: "word", type: "check", label: t("find.wholeWord"), value: lastGrep.word },
      { id: "regex", type: "check", label: t("find.regex"), value: lastGrep.regex },
    ],
    t("menu.find"),
  );
  if (!form) return;
  const query = (form.query || "").trim();
  if (!query) return;
  Object.assign(lastGrep, {
    query: form.query,
    dir: (form.dir || "").trim(),
    glob: form.glob || "",
    ci: !!form.ci,
    word: !!form.word,
    regex: !!form.regex,
  });
  showLoading(t("dialog.grep.searching"));
  try {
    const res = await apiPost<GrepResponse, GrepRequest>("/api/grep", {
      query,
      dir: lastGrep.dir || null,
      glob: (form.glob || "").trim(),
      ci: lastGrep.ci,
      word: lastGrep.word,
      regex: lastGrep.regex,
      max: 2000,
    });
    flashCount(t("dialog.grep.flash", { n: commas(res.hits.length) }));
    showGrep(res, query, lastGrep.regex);
  } catch (e) {
    flashCount(t("dialog.grep.error"), "error");
    showMessage(t("dialog.grep.error"), serverMessage(e));
  } finally {
    hideLoading();
  }
}

export function showGrep(res, query, regex) {
  const files = new Set(res.hits.map((h) => h.path)).size;
  $("grep-summary").textContent =
    t("dialog.grep.summary", { hits: commas(res.hits.length), files: commas(files) }) +
    (res.truncated ? t("dialog.grep.summaryTruncated", { max: commas(res.hits.length) }) : "") +
    (res.files_truncated ? t("dialog.grep.summaryFiles") : "");
  renderGrepResults(res, query, regex);
  setModalOpen($("grep-modal"), true);
}

// Highlight the literal match inside a preview line ([col, col+queryChars]).
export function appendGrepText(host, text, col, query, regex) {
  const chars = Array.from(text);
  const qlen = regex ? 0 : Array.from(query).length;
  if (!qlen || col < 0 || col > chars.length) {
    host.textContent = text;
    return;
  }
  const before = chars.slice(0, col).join("");
  const mid = chars.slice(col, col + qlen).join("");
  const after = chars.slice(col + qlen).join("");
  if (before) host.append(document.createTextNode(before));
  const mark = el("span", "grep-match", mid);
  host.append(mark);
  if (after) host.append(document.createTextNode(after));
}

export function renderGrepResults(res, query, regex) {
  const view = $("grep-results");
  view.textContent = "";
  const hits = res.hits || [];
  const maxLine = hits.reduce((max, hit) => Math.max(max, hit.line + 1), 0);
  view.style.setProperty("--gutter-ch", `${lineNumberChars(maxLine)}ch`);
  if (hits.length === 0) {
    const empty = el("div", "grep-empty", t("dialog.grep.noMatches"));
    view.append(empty);
    return;
  }
  const frag = document.createDocumentFragment();
  let group = null;
  let currentPath = null;
  for (const h of hits) {
    if (h.path !== currentPath) {
      currentPath = h.path;
      group = el("section", "grep-file");
      const head = el("div", "grep-file-head", displayPath(h.path));
      head.title = displayPath(h.path);
      group.append(head);
      frag.append(group);
    }
    const row = button("grep-hit", "");
    const ln = el("span", "grep-ln", formatLineNo(h.line + 1));
    const tx = el("span", "grep-tx");
    appendGrepText(tx, h.text, h.col, query, regex);
    row.append(ln, tx);
    row.addEventListener("click", () => openGrepHit(h.path, h.line));
    group.append(row);
  }
  view.append(frag);
}

export async function openGrepHit(path, line) {
  hideGrep();
  await openPath(path);
  gotoLine(line + 1);
}
