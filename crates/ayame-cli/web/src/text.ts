// Ayame Editor — side-effect-free text helpers shared by editing and search.

export function charLenOf(str) {
  return Array.from(str).length;
}

export function utf8ByteLength(str) {
  return new TextEncoder().encode(str).length;
}

// UTF-16 index of Unicode-scalar column `col` in `text` (surrogate-safe).
export function utf16IndexOfCol(text, col) {
  let idx = 0;
  let c = 0;
  for (const ch of text) {
    if (c >= col) break;
    idx += ch.length;
    c++;
  }
  return idx;
}

export const isWordChar = (ch) => /[\p{L}\p{N}_]/u.test(ch || "");
