// Pure parsing of what the user types into Go to Line.
//
// The prompt used to strip every non-digit and jump to whatever number was
// left: `120:8` went to line 1208 and `50%` to line 50. In a file with millions
// of lines the useful targets are exactly the ones that are not a bare line
// number — a position some way through the file, a hop relative to the caret,
// the `line:column` a compiler or log points at — so each of those is a form
// here, and anything else is refused instead of guessed at.

export type GotoTarget = {
  /** 1-based line, clamped to the document. */
  line: number;
  /** 1-based Unicode-scalar column; the server clamps it to the line. */
  column: number;
};

export type GotoContext = {
  /** 1-based line the caret is on, for the relative forms. */
  currentLine: number;
  totalLines: number;
};

// A whole number, optionally grouped the way the gutter prints line numbers.
// Grouping is strict: `1,234,567` is a number, `12,5` is not — it is far more
// likely a mistyped `12:5` than a line number, and guessing either way would
// put the caret somewhere the user did not ask for.
const INTEGER = String.raw`\d{1,3}(?:,\d{3})+|\d+`;
const TARGET = new RegExp(
  String.raw`^(?:(?<percent>\d+(?:\.\d+)?)%|(?<sign>[+-])?(?<line>${INTEGER}))(?::(?<column>${INTEGER}))?$`,
);

function integer(text: string) {
  const value = Number(text.replace(/,/g, ""));
  return Number.isSafeInteger(value) ? value : null;
}

/// The position a Go to Line entry names, or `null` when it names none.
///
///   120        line 120
///   1,234,567  line 1234567
///   120:8      line 120, column 8
///   50%        halfway through the file (0% is the first line, 100% the last)
///   +20 / -20  twenty lines below / above the caret
///
/// Full-width digits and punctuation are folded first (`１２０：８`, `５０％`):
/// with a Japanese IME left on, that is what the same keys produce.
export function parseGotoTarget(input: string, context: GotoContext): GotoTarget | null {
  const text = String(input ?? "")
    .normalize("NFKC")
    .replace(/\s+/g, "");
  const groups = TARGET.exec(text)?.groups;
  if (!groups) return null;
  const last = Math.max(1, Math.floor(context.totalLines) || 0);

  let line: number;
  if (groups.percent !== undefined) {
    const percent = Number(groups.percent);
    if (!Number.isFinite(percent) || percent > 100) return null;
    line = Math.round((percent / 100) * (last - 1)) + 1;
  } else {
    const value = integer(groups.line);
    if (value === null) return null;
    if (groups.sign === "+") line = context.currentLine + value;
    else if (groups.sign === "-") line = context.currentLine - value;
    else line = value;
  }

  let column = 1;
  if (groups.column !== undefined) {
    const value = integer(groups.column);
    if (value === null) return null;
    column = Math.max(1, value);
  }
  return { line: Math.min(last, Math.max(1, line)), column };
}
