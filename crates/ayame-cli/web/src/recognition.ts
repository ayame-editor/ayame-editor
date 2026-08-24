// Selected path/URL recognition shared by context menu, keymap and Ctrl+Click.
import { state } from "./state.js";
import { t } from "./i18n.js";
import { apiPost } from "./api.js";
import { askConfirm, showMessage } from "./dialogs.js";
import { lineChars } from "./editor.js";
import { rectRange, selectionRanges, selRange } from "./selection-model.js";
import { openPath } from "./workspace.js";
import { gotoLaunchPosition } from "./edits.js";
import { isNativeApp, postNativeMessage } from "./app.js";

const MAX_CANDIDATE_CHARS = 4096;

type Recognized = {
  kind: "file" | "directory" | "url";
  target: string;
  line?: number;
  column?: number;
};

function selectedCandidate(point) {
  if (!pointInsideSelection(point) || rectRange()) return "";
  const range = selRange();
  if (!range || range.start.line !== range.end.line) return "";
  const length = range.end.col - range.start.col;
  if (length <= 0 || length > MAX_CANDIDATE_CHARS) return "";
  return lineChars(range.start.line).slice(range.start.col, range.end.col).join("");
}

function pointInsideSelection(point) {
  const rect = rectRange();
  if (rect) {
    return (
      point.line >= rect.l0 && point.line <= rect.l1 && point.col >= rect.c0 && point.col <= rect.c1
    );
  }
  return selectionRanges().some((range) => {
    if (point.line < range.start.line || point.line > range.end.line) return false;
    if (point.line === range.start.line && point.col < range.start.col) return false;
    if (point.line === range.end.line && point.col > range.end.col) return false;
    return true;
  });
}

function tokenCandidate(point) {
  const chars = lineChars(point.line);
  if (!chars.length) return "";
  const boundary = (char) => char == null || /[\s'"`<>]/u.test(char);
  let pivot = Math.min(point.col, chars.length - 1);
  if (boundary(chars[pivot]) && pivot > 0 && !boundary(chars[pivot - 1])) pivot--;
  if (boundary(chars[pivot])) return "";
  let start = pivot;
  let end = pivot + 1;
  while (start > 0 && !boundary(chars[start - 1]) && end - start < MAX_CANDIDATE_CHARS) start--;
  while (end < chars.length && !boundary(chars[end]) && end - start < MAX_CANDIDATE_CHARS) end++;
  return chars.slice(start, end).join("");
}

export function candidateAt(point = state.caret.position) {
  return selectedCandidate(point) || tokenCandidate(point);
}

export async function recognizeAt(point = state.caret.position): Promise<Recognized | null> {
  const candidate = candidateAt(point);
  if (!candidate || Array.from(candidate).length > MAX_CANDIDATE_CHARS) return null;
  return apiPost<Recognized | null>("/api/selection/recognize", { candidate });
}

export async function openRecognizedAt(point = state.caret.position): Promise<boolean> {
  let recognized: Recognized | null;
  try {
    recognized = await recognizeAt(point);
  } catch {
    return false;
  }
  if (!recognized) return false;
  const approved = await askConfirm(
    t("recognition.confirmTitle"),
    `${t(`recognition.kind.${recognized.kind}`)}\n\n${recognized.target}`,
    { okLabel: t("recognition.open") },
  );
  if (!approved) return true;

  if (recognized.kind === "url") {
    if (isNativeApp()) postNativeMessage({ type: "open_external_url", url: recognized.target });
    else window.open(recognized.target, "_blank", "noopener,noreferrer");
    return true;
  }
  if (recognized.kind === "directory") {
    if (isNativeApp()) postNativeMessage({ type: "reveal_path", path: recognized.target });
    else await showMessage(t("recognition.directory"), recognized.target);
    return true;
  }

  await openPath(recognized.target);
  if (recognized.line != null) {
    await gotoLaunchPosition({ line: recognized.line, column: recognized.column ?? 1 });
  }
  return true;
}

export async function openRecognizedSelection() {
  await openRecognizedAt(state.caret.position);
}
