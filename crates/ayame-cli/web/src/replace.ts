// Ayame Editor — in-editor replacement and replace-all batching.
import { $, commas } from "./dom.js";
import { state } from "./state.js";
import { t } from "./i18n.js";
import { api, type FindResponse, type LinesResponse, type SearchResponse } from "./api.js";
import { applyBatchPlain, applyRange, enqueueEdit } from "./edits.js";
import { hideLoading, setLoadingDetail, showLoading } from "./dialogs.js";
import { flashCount } from "./notifications.js";
import { lineByte } from "./editor.js";
import { selRange } from "./selection-model.js";
import {
  expandTemplate,
  replaceWithinWindow,
  scopeFromSelection,
  scopeWindow,
  type ReplaceScope,
} from "./replace-scope.js";
import { charLenOf, utf16IndexOfCol, utf8ByteLength } from "./text.js";
import { buildMatcher, findStep, qs, saveReplaceHistory, updateCount } from "./findbar.js";

export const REPLACE_ALL_MAX = 20000;

// The replacement string sent to the document for one concrete match.
export function replacementFor(match: RegExpMatchArray, replacement: string) {
  return expandTemplate(match, replacement, state.search.regex);
}

export function replaceReady() {
  if (!state.doc.stat?.open) return false;
  if (!state.search.query) {
    flashCount(t("find.enterQuery"), "error");
    return false;
  }
  buildMatcher();
  if (state.search.regexError || !state.search.matcher) {
    flashCount(t("find.regexError"), "error");
    return false;
  }
  if (state.search.matcherWordFallback) {
    // The pattern itself is fine — it is the whole-word wrapper around it that
    // this engine will not accept, so the highlight is a superset of the real
    // matches. Replacing against that superset would rewrite text the user's
    // whole-word setting says to leave alone, hence the refusal; saying
    // "invalid regex" instead just sends people hunting a typo that is not
    // there (#173).
    flashCount(t("find.wordRegexUnsupported"), "error");
    return false;
  }
  return true;
}

// Replace the current match, then move to the next one.
export async function replaceCurrent() {
  if (!replaceReady()) return;
  const replacement = $("replace-input").value;
  saveReplaceHistory(replacement);
  if (!state.search.lastMatch) {
    await findStep("next");
    return;
  }
  try {
    const res = await api<FindResponse>(
      `/api/find?dir=next&from=${state.search.lastMatch.byte}&${qs()}`,
    );
    const h = res.hit;
    if (!h || h.byte !== state.search.lastMatch.byte) {
      await findStep("next");
      return;
    }
    const lr = await api<LinesResponse>(`/api/lines?start=${h.line}&count=1`);
    const text = lr.lines?.[0]?.text ?? "";
    const u16 = utf16IndexOfCol(text, h.column);
    const re = new RegExp(state.search.matcher.source, state.search.matcher.flags);
    re.lastIndex = u16;
    const m = re.exec(text);
    if (!m || m.index !== u16) {
      flashCount(t("find.cannotIdentifyMatch"), "error");
      return;
    }
    const rep = replacementFor(m, replacement);
    const c0 = h.column;
    const c1 = h.column + charLenOf(m[0]);
    await enqueueEdit(() => applyRange(h.line, c0, h.line, c1, rep));
    // Resume just past inserted text so replacement containing the query cannot loop.
    state.search.lastMatch = { byte: h.byte, len: Math.max(1, utf8ByteLength(rep)) };
    await updateCount();
    await findStep("next");
  } catch (e) {
    flashCount(t("find.replaceError"), "error");
    console.error(e);
  }
}

/// The scope replace-all will honour: the current selection when the user asked
/// for "in selection only" and there is one, otherwise the whole document.
export function activeReplaceScope(): ReplaceScope | null {
  if (!state.search.inSelection) return null;
  return scopeFromSelection(selRange());
}

// One whole-line edit per matching line, flushed in bounded batches.
//
// Cancelable: the whole walk happens here in the client — a search sweep, then
// line fetches, then batched edits — so there is no server-side operation to
// cancel and the overlay drives a plain flag instead. It is honoured at every
// boundary that costs a round trip, so Cancel lands within one request rather
// than at the end (#173). Edits already committed stay committed; they are
// ordinary undoable batches, and the count says how many landed.
export async function replaceAll() {
  if (!replaceReady()) return;
  const replacement = $("replace-input").value;
  saveReplaceHistory(replacement);
  const scope = activeReplaceScope();
  let canceled = false;
  showLoading(t(scope ? "find.replacingInSelection" : "find.replacing"), {
    cancel: true,
    onCancel: () => {
      canceled = true;
    },
  });
  let replaced = 0;
  try {
    const lines = await collectMatchingLines(scope, () => canceled);
    if (canceled) return;
    if (!lines.length) {
      flashCount(t(scope ? "find.noMatchInSelection" : "find.noMatch"));
      return;
    }
    let edits = [];
    let pendingBytes = 0;
    const flush = async () => {
      if (!edits.length) return;
      const batch = edits;
      edits = [];
      pendingBytes = 0;
      await enqueueEdit(() => applyBatchPlain(batch));
    };
    // Fetch affected lines in contiguous chunks (≤2000 lines per request) and
    // rewrite each as it arrives, so a cancel does not first pay for fetching
    // every remaining line.
    for (let i = 0; i < lines.length && !canceled; ) {
      let j = i;
      while (j + 1 < lines.length && lines[j + 1] - lines[i] < 2000) j++;
      const from = lines[i];
      const count = lines[j] - lines[i] + 1;
      const r = await api<LinesResponse>(`/api/lines?start=${from}&count=${count}`);
      const texts = new Map<number, string>();
      r.lines.forEach((rec, k) => texts.set(from + k, rec.text ?? ""));
      for (const line of lines.slice(i, j + 1)) {
        const text = texts.get(line);
        if (text == null) continue;
        const window = scopeWindow(scope, line);
        if (!window) continue;
        const next = replaceWithinWindow(
          text,
          state.search.matcher,
          replacement,
          state.search.regex,
          window,
        );
        if (!next.count || next.text === text) continue;
        replaced += next.count;
        edits.push({ l0: line, c0: 0, l1: line, c1: charLenOf(text), text: next.text });
        pendingBytes += next.text.length;
        if (edits.length >= 2000 || pendingBytes > 512 * 1024) await flush();
      }
      i = j + 1;
      setLoadingDetail(
        t("find.replaceProgress", { done: commas(replaced) }),
        (i / lines.length) * 100,
      );
    }
    await flush();
    state.search.lastMatch = null;
    await updateCount();
    if (canceled) {
      flashCount(t("find.replaceCanceled", { n: commas(replaced) }), "error");
      return;
    }
    flashCount(replaced ? t("find.replacedCount", { n: commas(replaced) }) : t("find.noMatch"));
  } catch (e) {
    flashCount(t("find.replaceError"), "error");
    console.error(e);
  } finally {
    hideLoading();
  }
}

// Walk /api/search for the lines holding at least one match, bounded to the
// scope's line range. Returns them sorted and deduped; a line that turns out to
// hold no match inside the scope's column window is simply skipped later.
async function collectMatchingLines(scope: ReplaceScope | null, canceled: () => boolean) {
  const lineSet = new Set<number>();
  let totalHits = 0;
  // Starting the sweep at the scope's first line skips everything above it
  // outright instead of paging through the whole document to throw it away.
  let start = scope ? await lineByte(scope.start.line) : 0;
  for (let pass = 0; pass < 10000 && !canceled(); pass++) {
    const res = await api<SearchResponse>(
      `/api/search?${qs()}&start=${start}&max=${REPLACE_ALL_MAX}`,
    );
    const hits = res.hits || [];
    let past = false;
    for (const h of hits) {
      if (scope && h.line > scope.end.line) {
        past = true;
        break;
      }
      if (scope && h.line < scope.start.line) continue;
      lineSet.add(h.line);
      totalHits++;
    }
    if (past || !hits.length || !res.truncated) break;
    const last = hits[hits.length - 1];
    const next = last.byte + Math.max(1, last.byte_len || 0);
    if (next <= start) break;
    start = next;
    setLoadingDetail(t("find.matchCount", { total: `${commas(totalHits)}+` }));
  }
  return [...lineSet].sort((a, b) => a - b);
}
