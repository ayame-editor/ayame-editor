import type { StructureProviderId } from "./syntax.js";

export const CLOSE_PAIRS = {
  "(": ")",
  "[": "]",
  "{": "}",
  '"': '"',
  "'": "'",
  "`": "`",
} as const;

export type PairOpener = keyof typeof CLOSE_PAIRS;

const CLOSERS = new Set(Object.values(CLOSE_PAIRS));

export function pairCloser(text: string): string | null {
  return Object.prototype.hasOwnProperty.call(CLOSE_PAIRS, text)
    ? CLOSE_PAIRS[text as PairOpener]
    : null;
}

export function isPairCloser(text: string) {
  return CLOSERS.has(text as (typeof CLOSE_PAIRS)[PairOpener]);
}

function escapedAt(text: string, col: number) {
  const chars = Array.from(text);
  let slashes = 0;
  for (let index = col - 1; index >= 0 && chars[index] === "\\"; index--) slashes++;
  return slashes % 2 === 1;
}

export function shouldAutoClose(line: string, col: number, opener: string) {
  if (!pairCloser(opener)) return false;
  if ((opener === '"' || opener === "'" || opener === "`") && escapedAt(line, col)) return false;
  return true;
}

export function shouldSkipCloser(line: string, col: number, closer: string) {
  return isPairCloser(closer) && Array.from(line)[col] === closer && !escapedAt(line, col);
}

export function emptyPairRange(line: string, col: number) {
  if (col <= 0) return null;
  const chars = Array.from(line);
  const opener = chars[col - 1];
  const closer = chars[col];
  return pairCloser(opener) === closer ? { start: col - 1, end: col + 1 } : null;
}

function leadingWhitespace(text: string) {
  return text.match(/^[\t ]*/u)?.[0] || "";
}

function safeAdditionalIndent(provider: StructureProviderId | null, beforeCaret: string) {
  const trimmed = beforeCaret.trimEnd();
  if (provider === "indent") return /:\s*(?:#.*)?$/u.test(trimmed);
  if (provider === "brace") return trimmed.endsWith("[") || trimmed.endsWith("{");
  if (provider === "markup") {
    return (
      /<([A-Za-z_][\w:.-]*)(?:\s[^<>]*?)?>\s*$/u.test(trimmed) && !/<\/|\/>\s*$/u.test(trimmed)
    );
  }
  return false;
}

export function newlineIndent(line: string, col: number, provider: StructureProviderId | null) {
  const chars = Array.from(line);
  const beforeCaret = chars.slice(0, Math.max(0, col)).join("");
  const inherited = leadingWhitespace(line);
  const unit = inherited.includes("\t") ? "\t" : "  ";
  return inherited + (safeAdditionalIndent(provider, beforeCaret) ? unit : "");
}

export function completionPrefix(line: string, col: number) {
  const before = Array.from(line).slice(0, Math.max(0, col)).join("");
  return before.match(/[\p{L}_$][\p{L}\p{N}_$]*$/u)?.[0] || "";
}

export function wordsFromText(text: string) {
  return text.match(/[\p{L}_$][\p{L}\p{N}_$]{1,63}/gu) || [];
}
