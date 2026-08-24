import type { StructureBlock } from "./fold-map.js";
import type { StructureProviderId } from "./syntax.js";

export type StructurePoint = { line: number; col: number };

export interface StructureLineSource {
  total: number;
  get(line: number): Promise<string | null>;
}

type BraceToken = StructurePoint & { char: "(" | ")" | "[" | "]" | "{" | "}" };
type MarkupToken = StructurePoint & { name: string; closing: boolean; selfClosing: boolean };
type BraceLexState = { blockComment: boolean };

const OPEN_TO_CLOSE = { "(": ")", "[": "]", "{": "}" } as const;
const CLOSE_TO_OPEN = { ")": "(", "]": "[", "}": "{" } as const;

function scalarAt(text: string, codeUnitIndex: number) {
  return Array.from(text.slice(0, codeUnitIndex)).length;
}

function leadingIndent(text: string) {
  let indent = 0;
  for (const char of text) {
    if (char === " ") indent++;
    else if (char === "\t") indent += 4;
    else break;
  }
  return indent;
}

function braceTokens(
  text: string,
  line: number,
  state: BraceLexState = { blockComment: false },
): BraceToken[] {
  const out: BraceToken[] = [];
  let quote = "";
  for (let index = 0; index < text.length; ) {
    const point = text.codePointAt(index)!;
    const char = String.fromCodePoint(point);
    const width = char.length;
    const next = text[index + width] || "";
    if (state.blockComment) {
      if (char === "*" && next === "/") {
        state.blockComment = false;
        index += width + 1;
      } else {
        index += width;
      }
      continue;
    }
    if (quote) {
      if (char === "\\")
        index += width + (next ? String.fromCodePoint(text.codePointAt(index + width)!).length : 0);
      else {
        if (char === quote) quote = "";
        index += width;
      }
      continue;
    }
    if (char === "/" && next === "/") break;
    if (char === "/" && next === "*") {
      state.blockComment = true;
      index += width + 1;
      continue;
    }
    if (char === '"' || char === "'" || char === "`") {
      quote = char;
      index += width;
      continue;
    }
    if ("()[]{}".includes(char)) {
      out.push({ line, col: scalarAt(text, index), char: char as BraceToken["char"] });
    }
    index += width;
  }
  return out;
}

function markupTokens(text: string, line: number): MarkupToken[] {
  const out: MarkupToken[] = [];
  const expression = /<!--.*?-->|<\s*(\/?)\s*([A-Za-z_][\w:.-]*)(?:\s[^<>]*?)?(\/?)\s*>/gu;
  for (const match of text.matchAll(expression)) {
    if (match[0].startsWith("<!--")) continue;
    out.push({
      line,
      col: scalarAt(text, match.index!),
      name: match[2].toLowerCase(),
      closing: !!match[1],
      selfClosing: !!match[3] || /^<(?:!|\?)/u.test(match[0]),
    });
  }
  return out;
}

function isIndentHeader(text: string) {
  const trimmed = text.trim();
  return !!trimmed && !trimmed.startsWith("#") && /:\s*(?:#.*)?$/u.test(trimmed);
}

function isLogBoundary(text: string) {
  return /^(?:\[?\d{4}-\d\d-\d\d[T\s]|TRACE\b|DEBUG\b|INFO\b|WARN(?:ING)?\b|ERROR\b|FATAL\b|CRITICAL\b)/u.test(
    text.trimStart(),
  );
}

async function previousNonBlank(source: StructureLineSource, start: number) {
  for (let line = Math.min(start, source.total - 1); line >= 0; line--) {
    const text = await source.get(line);
    if (text != null && text.trim()) return { line, text };
  }
  return null;
}

async function braceBlockFromOpening(
  source: StructureLineSource,
  opening: BraceToken,
): Promise<StructureBlock | null> {
  if (!(opening.char in OPEN_TO_CLOSE) || opening.char === "(") return null;
  const openingText = await source.get(opening.line);
  if (openingText == null) return null;
  const level = leadingIndent(openingText);
  const stack: string[] = [];
  const lexState: BraceLexState = { blockComment: false };
  for (let line = opening.line; line < source.total; line++) {
    const text = await source.get(line);
    if (text == null) return null;
    const tokens = braceTokens(text, line, lexState).filter(
      (token) => line !== opening.line || token.col >= opening.col,
    );
    for (const token of tokens) {
      if (token.char in OPEN_TO_CLOSE)
        stack.push(OPEN_TO_CLOSE[token.char as keyof typeof OPEN_TO_CLOSE]);
      else if (stack.at(-1) === token.char) {
        stack.pop();
        if (!stack.length) {
          return token.line > opening.line ? { start: opening.line, end: token.line, level } : null;
        }
      }
    }
  }
  return null;
}

async function braceOpeningForLine(
  source: StructureLineSource,
  line: number,
  col: number,
): Promise<BraceToken | null> {
  const stack: BraceToken[] = [];
  const lexState: BraceLexState = { blockComment: false };
  for (let at = 0; at <= line; at++) {
    const text = await source.get(at);
    if (text == null) return null;
    let tokens = braceTokens(text, at, lexState);
    if (at === line) {
      const direct = tokens.find((token) => token.char === "{" || token.char === "[");
      if (direct) return direct;
      tokens = tokens.filter((token) => token.col < col);
    }
    for (const token of tokens) {
      if (token.char in OPEN_TO_CLOSE) stack.push(token);
      else {
        const opening = stack.at(-1);
        if (opening && OPEN_TO_CLOSE[opening.char as keyof typeof OPEN_TO_CLOSE] === token.char) {
          stack.pop();
        }
      }
    }
  }
  for (let index = stack.length - 1; index >= 0; index--) {
    if (stack[index].char === "{" || stack[index].char === "[") return stack[index];
  }
  return null;
}

async function findBraceBlock(
  source: StructureLineSource,
  line: number,
  col: number,
): Promise<StructureBlock | null> {
  const opening = await braceOpeningForLine(source, line, col);
  return opening ? braceBlockFromOpening(source, opening) : null;
}

async function indentBlockFromHeader(
  source: StructureLineSource,
  start: number,
  text: string,
): Promise<StructureBlock | null> {
  const base = leadingIndent(text);
  let lastContent = start;
  for (let line = start + 1; line < source.total; line++) {
    const next = await source.get(line);
    if (next == null) return null;
    if (!next.trim()) continue;
    if (leadingIndent(next) <= base) break;
    lastContent = line;
  }
  return lastContent > start ? { start, end: lastContent, level: base } : null;
}

async function findIndentBlock(source: StructureLineSource, line: number) {
  const target = await previousNonBlank(source, line);
  if (!target) return null;
  if (target.line === line && isIndentHeader(target.text)) {
    const direct = await indentBlockFromHeader(source, target.line, target.text);
    if (direct) return direct;
  }
  const targetIndent = leadingIndent(target.text);
  for (let at = target.line - 1; at >= 0; at--) {
    const text = await source.get(at);
    if (text == null) return null;
    if (!text.trim() || !isIndentHeader(text) || leadingIndent(text) >= targetIndent) continue;
    const block = await indentBlockFromHeader(source, at, text);
    if (block && target.line <= block.end) return block;
  }
  return null;
}

async function markupBlockFromOpening(
  source: StructureLineSource,
  opening: MarkupToken,
): Promise<StructureBlock | null> {
  const openingText = await source.get(opening.line);
  if (openingText == null) return null;
  const level = leadingIndent(openingText);
  let depth = 0;
  for (let line = opening.line; line < source.total; line++) {
    const text = await source.get(line);
    if (text == null) return null;
    for (const token of markupTokens(text, line)) {
      if (line === opening.line && token.col < opening.col) continue;
      if (token.name !== opening.name || token.selfClosing) continue;
      depth += token.closing ? -1 : 1;
      if (depth === 0) {
        return line > opening.line ? { start: opening.line, end: line, level } : null;
      }
    }
  }
  return null;
}

async function findMarkupBlock(source: StructureLineSource, line: number) {
  const current = await source.get(line);
  if (current == null) return null;
  const direct = markupTokens(current, line).find((token) => !token.closing && !token.selfClosing);
  if (direct) return markupBlockFromOpening(source, direct);
  const expected: string[] = [];
  for (let at = line; at >= 0; at--) {
    const text = at === line ? current : await source.get(at);
    if (text == null) return null;
    const tokens = markupTokens(text, at);
    for (let index = tokens.length - 1; index >= 0; index--) {
      const token = tokens[index];
      if (token.selfClosing) continue;
      if (token.closing) expected.push(token.name);
      else if (expected.at(-1) === token.name) expected.pop();
      else if (!expected.length) return markupBlockFromOpening(source, token);
    }
  }
  return null;
}

async function findLogBlock(source: StructureLineSource, line: number) {
  let start = Math.max(0, Math.min(line, source.total - 1));
  while (start > 0) {
    const text = await source.get(start);
    if (text != null && isLogBoundary(text)) break;
    start--;
  }
  let end = start;
  for (let at = start + 1; at < source.total; at++) {
    const text = await source.get(at);
    if (text == null || isLogBoundary(text)) break;
    end = at;
  }
  return end > start ? { start, end, level: 0 } : null;
}

export function lineMayStartStructure(provider: StructureProviderId, text: string) {
  if (provider === "brace")
    return braceTokens(text, 0).some((token) => token.char === "{" || token.char === "[");
  if (provider === "indent") return isIndentHeader(text);
  if (provider === "markup")
    return markupTokens(text, 0).some((token) => !token.closing && !token.selfClosing);
  return isLogBoundary(text);
}

export async function findStructureBlock(
  provider: StructureProviderId,
  source: StructureLineSource,
  line: number,
  col = 0,
): Promise<StructureBlock | null> {
  if (source.total <= 0) return null;
  const clamped = Math.max(0, Math.min(line, source.total - 1));
  if (provider === "brace") return findBraceBlock(source, clamped, col);
  if (provider === "indent") return findIndentBlock(source, clamped);
  if (provider === "markup") return findMarkupBlock(source, clamped);
  return findLogBlock(source, clamped);
}

export async function structureBlockStartingAt(
  provider: StructureProviderId,
  source: StructureLineSource,
  line: number,
): Promise<StructureBlock | null> {
  const text = await source.get(line);
  if (text == null || !lineMayStartStructure(provider, text)) return null;
  if (provider === "brace") {
    const opening = braceTokens(text, line).find(
      (token) => token.char === "{" || token.char === "[",
    );
    return opening ? braceBlockFromOpening(source, opening) : null;
  }
  if (provider === "indent") return indentBlockFromHeader(source, line, text);
  if (provider === "markup") {
    const opening = markupTokens(text, line).find((token) => !token.closing && !token.selfClosing);
    return opening ? markupBlockFromOpening(source, opening) : null;
  }
  return findLogBlock(source, line);
}

export async function findSiblingBlock(
  provider: StructureProviderId,
  source: StructureLineSource,
  current: StructureBlock,
  direction: -1 | 1,
) {
  if (direction > 0) {
    for (let line = current.end + 1; line < source.total; line++) {
      const block = await structureBlockStartingAt(provider, source, line);
      if (block && block.level === current.level) return block;
    }
    return null;
  }
  for (let line = current.start - 1; line >= 0; line--) {
    const block = await structureBlockStartingAt(provider, source, line);
    if (block && block.end < current.start && block.level === current.level) return block;
  }
  return null;
}

export async function findMatchingBrace(
  source: StructureLineSource,
  line: number,
  col: number,
): Promise<StructurePoint | null> {
  const text = await source.get(line);
  if (text == null) return null;
  const token = braceTokens(text, line)
    .filter((candidate) => Math.abs(candidate.col - col) <= 1)
    .sort((left, right) => Math.abs(left.col - col) - Math.abs(right.col - col))[0];
  if (!token) return null;
  if (token.char in OPEN_TO_CLOSE) {
    const expected: string[] = [];
    const lexState: BraceLexState = { blockComment: false };
    for (let at = line; at < source.total; at++) {
      const next = at === line ? text : await source.get(at);
      if (next == null) return null;
      for (const candidate of braceTokens(next, at, lexState)) {
        if (at === line && candidate.col < token.col) continue;
        if (candidate.char in OPEN_TO_CLOSE)
          expected.push(OPEN_TO_CLOSE[candidate.char as keyof typeof OPEN_TO_CLOSE]);
        else if (expected.at(-1) === candidate.char) {
          expected.pop();
          if (!expected.length) return { line: at, col: candidate.col };
        }
      }
    }
    return null;
  }
  const expected: string[] = [];
  for (let at = line; at >= 0; at--) {
    const previous = at === line ? text : await source.get(at);
    if (previous == null) return null;
    const tokens = braceTokens(previous, at);
    for (let index = tokens.length - 1; index >= 0; index--) {
      const candidate = tokens[index];
      if (at === line && candidate.col > token.col) continue;
      if (candidate.char in CLOSE_TO_OPEN)
        expected.push(CLOSE_TO_OPEN[candidate.char as keyof typeof CLOSE_TO_OPEN]);
      else if (expected.at(-1) === candidate.char) {
        expected.pop();
        if (!expected.length) return { line: at, col: candidate.col };
      }
    }
  }
  return null;
}
