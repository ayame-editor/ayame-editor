// Ayame Editor — in-editor replacement and replace-all batching.
import { $, commas } from "./dom.js";
import { state } from "./state.js";
import { t } from "./i18n.js";
import { api, type FindResponse, type LinesResponse, type SearchResponse } from "./api.js";
import { applyBatchPlain, applyRange, enqueueEdit } from "./edits.js";
import { hideLoading, showLoading } from "./dialogs.js";
import { flashCount } from "./notifications.js";
import { charLenOf, utf16IndexOfCol, utf8ByteLength } from "./text.js";
import { buildMatcher, findStep, qs, updateCount } from "./findbar.js";

export const REPLACE_ALL_MAX = 20000;

// The replacement string sent to the document for one concrete match.
export function replacementFor(matchText, replacement) {
  if (!state.regex) return replacement;
  const single = new RegExp(state.matcher.source, state.matcher.flags.replace("g", ""));
  return matchText.replace(single, replacement);
}

// In literal mode "$" has no special meaning; escape it for String.replace.
export function literalReplacement(replacement) {
  return replacement.replace(/\$/g, "$$$$");
}

export function replaceReady() {
  if (!state.stat?.open) return false;
  if (!state.query) {
    flashCount(t("find.enterQuery"), "error");
    return false;
  }
  buildMatcher();
  if (state.regexError || !state.matcher) {
    flashCount(t("find.regexError"), "error");
    return false;
  }
  if (state.matcherWordFallback) {
    flashCount(t("find.regexError"), "error");
    return false;
  }
  return true;
}

// Replace the current match, then move to the next one.
export async function replaceCurrent() {
  if (!replaceReady()) return;
  const replacement = $("replace-input").value;
  if (!state.lastMatch) {
    await findStep("next");
    return;
  }
  try {
    const res = await api<FindResponse>(`/api/find?dir=next&from=${state.lastMatch.byte}&${qs()}`);
    const h = res.hit;
    if (!h || h.byte !== state.lastMatch.byte) {
      await findStep("next");
      return;
    }
    const lr = await api<LinesResponse>(`/api/lines?start=${h.line}&count=1`);
    const text = lr.lines?.[0]?.text ?? "";
    const u16 = utf16IndexOfCol(text, h.column);
    const re = new RegExp(state.matcher.source, state.matcher.flags);
    re.lastIndex = u16;
    const m = re.exec(text);
    if (!m || m.index !== u16) {
      flashCount(t("find.cannotIdentifyMatch"), "error");
      return;
    }
    const rep = replacementFor(m[0], replacement);
    const c0 = h.column;
    const c1 = h.column + charLenOf(m[0]);
    await enqueueEdit(() => applyRange(h.line, c0, h.line, c1, rep));
    // Resume just past inserted text so replacement containing the query cannot loop.
    state.lastMatch = { byte: h.byte, len: Math.max(1, utf8ByteLength(rep)) };
    await updateCount();
    await findStep("next");
  } catch (e) {
    flashCount(t("find.replaceError"), "error");
    console.error(e);
  }
}

// One whole-line edit per matching line, flushed in bounded batches.
export async function replaceAll() {
  if (!replaceReady()) return;
  const replacement = $("replace-input").value;
  const literal = literalReplacement(replacement);
  showLoading(t("find.replacing"));
  try {
    const lineSet = new Set<number>();
    let totalHits = 0;
    let start = 0;
    for (let pass = 0; pass < 10000; pass++) {
      const res = await api<SearchResponse>(
        `/api/search?${qs()}&start=${start}&max=${REPLACE_ALL_MAX}`,
      );
      const hits = res.hits || [];
      for (const h of hits) lineSet.add(h.line);
      totalHits += hits.length;
      if (!hits.length || !res.truncated) break;
      const last = hits[hits.length - 1];
      const next = last.byte + Math.max(1, last.byte_len || 0);
      if (next <= start) break;
      start = next;
      flashCount(t("find.matchCount", { total: `${commas(totalHits)}+` }));
    }
    if (!lineSet.size) {
      flashCount(t("find.noMatch"));
      return;
    }
    const lines: number[] = [...lineSet].sort((a, b) => a - b);
    // Fetch affected lines in contiguous chunks (≤2000 lines per request).
    const texts = new Map();
    for (let i = 0; i < lines.length; ) {
      let j = i;
      while (j + 1 < lines.length && lines[j + 1] - lines[i] < 2000) j++;
      const start = lines[i];
      const count = lines[j] - lines[i] + 1;
      const r = await api<LinesResponse>(`/api/lines?start=${start}&count=${count}`);
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
    flashCount(replaced ? t("find.replacedCount", { n: commas(replaced) }) : t("find.noMatch"));
  } catch (e) {
    flashCount(t("find.replaceError"), "error");
    console.error(e);
  } finally {
    hideLoading();
  }
}
