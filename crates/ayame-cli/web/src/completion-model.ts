import { wordsFromText } from "./input-assist.js";

export const COMPLETION_MAX_CANDIDATES = 256;
export const COMPLETION_MAX_CANDIDATE_BYTES = 64 * 1024;
export const COMPLETION_MAX_LOCAL_SOURCE_BYTES = 64 * 1024;
export const COMPLETION_MAX_DOM_ROWS = 12;

function utf8CodePointBytes(codePoint: number) {
  if (codePoint <= 0x7f) return 1;
  if (codePoint <= 0x7ff) return 2;
  if (codePoint <= 0xffff) return 3;
  return 4;
}

function utf8Bytes(text: string) {
  let bytes = 0;
  for (const char of text) bytes += utf8CodePointBytes(char.codePointAt(0)!);
  return bytes;
}

export class CompletionCandidates {
  readonly words = new Set<string>();
  bytes = 0;
  truncated = false;

  constructor(
    readonly prefix: string,
    readonly maxCandidates = COMPLETION_MAX_CANDIDATES,
    readonly maxBytes = COMPLETION_MAX_CANDIDATE_BYTES,
  ) {}

  add(word: string) {
    const lowerPrefix = this.prefix.toLocaleLowerCase();
    const lowerWord = word.toLocaleLowerCase();
    if (!lowerWord.startsWith(lowerPrefix) || lowerWord === lowerPrefix || this.words.has(word)) {
      return;
    }
    const wordBytes = utf8Bytes(word);
    if (this.words.size >= this.maxCandidates || this.bytes + wordBytes > this.maxBytes) {
      this.truncated = true;
      return;
    }
    this.words.add(word);
    this.bytes += wordBytes;
  }

  addAll(words: Iterable<string>) {
    for (const word of words) {
      this.add(word);
      if (this.words.size >= this.maxCandidates) break;
    }
  }

  sorted() {
    return [...this.words].sort((left, right) =>
      left.localeCompare(right, undefined, { sensitivity: "base" }),
    );
  }
}

function boundedPrefix(text: string, byteBudget: number) {
  let bytes = 0;
  let end = 0;
  while (end < text.length) {
    const codePoint = text.codePointAt(end)!;
    const nextBytes = utf8CodePointBytes(codePoint);
    if (bytes + nextBytes > byteBudget) break;
    bytes += nextBytes;
    end += codePoint > 0xffff ? 2 : 1;
  }
  return { text: end === text.length ? text : text.slice(0, end), bytes };
}

export function localCompletionCandidates(
  prefix: string,
  schemeWords: Iterable<string>,
  texts: Iterable<string>,
) {
  const candidates = new CompletionCandidates(prefix);
  candidates.addAll(schemeWords);
  let sourceBytes = 0;
  for (const text of texts) {
    if (sourceBytes >= COMPLETION_MAX_LOCAL_SOURCE_BYTES) {
      candidates.truncated = true;
      break;
    }
    const remaining = COMPLETION_MAX_LOCAL_SOURCE_BYTES - sourceBytes;
    const bounded = boundedPrefix(text, remaining);
    sourceBytes += bounded.bytes;
    candidates.addAll(wordsFromText(bounded.text));
    if (candidates.words.size >= COMPLETION_MAX_CANDIDATES) break;
  }
  return { candidates: candidates.sorted(), sourceBytes, truncated: candidates.truncated };
}
