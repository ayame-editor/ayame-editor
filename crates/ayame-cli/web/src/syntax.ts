// Lightweight visible-row syntax highlighting. This deliberately stays
// line-local: no parser state, no whole-file scans, and no dependency that can
// make huge files feel less like plain text.

export type SyntaxKind =
  | "plain"
  | "comment"
  | "keyword"
  | "string"
  | "number"
  | "literal"
  | "function"
  | "key"
  | "op"
  | "heading"
  | "link"
  | "level-trace"
  | "level-debug"
  | "level-info"
  | "level-warn"
  | "level-error";

export type SyntaxSpan = {
  kind: SyntaxKind;
  text: string;
};

type Language =
  | "json"
  | "markdown"
  | "log"
  | "javascript"
  | "python"
  | "rust"
  | "go"
  | "sql"
  | "shell"
  | "css"
  | "html"
  | "yaml";

const EXT_LANGUAGE: Record<string, Language> = {
  cjs: "javascript",
  css: "css",
  go: "go",
  htm: "html",
  html: "html",
  js: "javascript",
  json: "json",
  jsonc: "json",
  jsx: "javascript",
  lock: "json",
  log: "log",
  mjs: "javascript",
  md: "markdown",
  mdx: "markdown",
  py: "python",
  rs: "rust",
  sh: "shell",
  sql: "sql",
  ts: "javascript",
  tsx: "javascript",
  yaml: "yaml",
  yml: "yaml",
};

const COMMON_LITERALS = new Set(["false", "nil", "null", "None", "true"]);

const KEYWORDS: Record<Language, Set<string>> = {
  css: new Set(["and", "from", "import", "not", "or", "url"]),
  go: new Set([
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
  ]),
  html: new Set(["DOCTYPE"]),
  javascript: new Set([
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "of",
    "return",
    "switch",
    "throw",
    "try",
    "type",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
  ]),
  json: new Set(),
  log: new Set(),
  markdown: new Set(),
  python: new Set([
    "and",
    "as",
    "assert",
    "async",
    "await",
    "break",
    "class",
    "continue",
    "def",
    "del",
    "elif",
    "else",
    "except",
    "finally",
    "for",
    "from",
    "global",
    "if",
    "import",
    "in",
    "is",
    "lambda",
    "nonlocal",
    "not",
    "or",
    "pass",
    "raise",
    "return",
    "try",
    "while",
    "with",
    "yield",
  ]),
  rust: new Set([
    "as",
    "async",
    "await",
    "break",
    "const",
    "continue",
    "crate",
    "dyn",
    "else",
    "enum",
    "extern",
    "fn",
    "for",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "pub",
    "ref",
    "return",
    "self",
    "Self",
    "static",
    "struct",
    "super",
    "trait",
    "type",
    "unsafe",
    "use",
    "where",
    "while",
  ]),
  shell: new Set([
    "case",
    "do",
    "done",
    "elif",
    "else",
    "esac",
    "export",
    "fi",
    "for",
    "function",
    "if",
    "in",
    "local",
    "set",
    "then",
    "while",
  ]),
  sql: new Set([
    "alter",
    "and",
    "as",
    "by",
    "case",
    "create",
    "delete",
    "desc",
    "distinct",
    "drop",
    "else",
    "end",
    "from",
    "group",
    "having",
    "in",
    "inner",
    "insert",
    "into",
    "is",
    "join",
    "left",
    "like",
    "limit",
    "not",
    "null",
    "on",
    "or",
    "order",
    "outer",
    "right",
    "select",
    "set",
    "table",
    "then",
    "union",
    "update",
    "values",
    "when",
    "where",
  ]),
  yaml: new Set(["false", "null", "true"]),
};

export function languageForPath(path: string): Language | null {
  const name = basename(path).toLowerCase();
  if (name === "dockerfile" || name === "makefile") return "shell";
  if (name === "cargo.lock" || name === "package-lock.json" || name === "pnpm-lock.yaml") {
    return name.endsWith(".yaml") ? "yaml" : "json";
  }
  const dot = name.lastIndexOf(".");
  if (dot < 0) return null;
  return EXT_LANGUAGE[name.slice(dot + 1)] || null;
}

export function highlightSpans(text: string, path = ""): SyntaxSpan[] | null {
  const lang = languageForPath(path) || inferLanguage(text);
  if (!lang) return null;
  if (lang === "json") return jsonSpans(text);
  if (lang === "markdown") return markdownSpans(text);
  if (lang === "log") return logSpans(text);
  if (lang === "yaml") return yamlSpans(text);
  return codeSpans(text, lang);
}

function inferLanguage(text: string): Language | null {
  if (/^\s*(TRACE|DEBUG|INFO|WARN|WARNING|ERROR|FATAL|CRITICAL)\b/u.test(text)) return "log";
  if (/^\s*\d{4}-\d\d-\d\d[T\s]\d\d:\d\d:\d\d/u.test(text)) return "log";
  if (/^\s*[{\[]\s*$/u.test(text) || /^\s*"[^"]+"\s*:/u.test(text)) return "json";
  return null;
}

function codeSpans(text: string, lang: Language): SyntaxSpan[] {
  const out: SyntaxSpan[] = [];
  const keywords = KEYWORDS[lang] || new Set<string>();
  const lineComment = commentPrefix(lang);
  let i = 0;
  while (i < text.length) {
    const rest = text.slice(i);
    if (lineComment && rest.startsWith(lineComment)) {
      push(out, "comment", rest);
      break;
    }
    if (rest.startsWith("/*")) {
      const end = text.indexOf("*/", i + 2);
      const j = end < 0 ? text.length : end + 2;
      push(out, "comment", text.slice(i, j));
      i = j;
      continue;
    }
    const ch = text[i];
    if (ch === "\"" || ch === "'" || (ch === "`" && lang === "javascript")) {
      const j = quotedEnd(text, i, ch);
      push(out, "string", text.slice(i, j));
      i = j;
      continue;
    }
    const num = rest.match(/^(?:0x[\da-f]+|\d+(?:\.\d+)?(?:e[+-]?\d+)?)/iu);
    if (num) {
      push(out, "number", num[0]);
      i += num[0].length;
      continue;
    }
    const ident = rest.match(/^[A-Za-z_$][\w$]*/u);
    if (ident) {
      const word = ident[0];
      const lower = word.toLowerCase();
      const kind = keywords.has(word) || keywords.has(lower)
        ? "keyword"
        : COMMON_LITERALS.has(word)
          ? "literal"
          : nextNonSpace(text, i + word.length) === "("
            ? "function"
            : "plain";
      push(out, kind, word);
      i += word.length;
      continue;
    }
    if (/^[{}()[\].,;:+\-*/%=&|!<>?]+$/u.test(ch)) {
      push(out, "op", ch);
    } else {
      push(out, "plain", ch);
    }
    i++;
  }
  return out;
}

function jsonSpans(text: string): SyntaxSpan[] {
  const out: SyntaxSpan[] = [];
  let i = 0;
  while (i < text.length) {
    const rest = text.slice(i);
    if (rest.startsWith("//")) {
      push(out, "comment", rest);
      break;
    }
    if (rest.startsWith("/*")) {
      const end = text.indexOf("*/", i + 2);
      const j = end < 0 ? text.length : end + 2;
      push(out, "comment", text.slice(i, j));
      i = j;
      continue;
    }
    if (text[i] === "\"") {
      const j = quotedEnd(text, i, "\"");
      push(out, nextNonSpace(text, j) === ":" ? "key" : "string", text.slice(i, j));
      i = j;
      continue;
    }
    const lit = rest.match(/^(true|false|null)\b/u);
    if (lit) {
      push(out, "literal", lit[0]);
      i += lit[0].length;
      continue;
    }
    const num = rest.match(/^-?\d+(?:\.\d+)?(?:e[+-]?\d+)?/iu);
    if (num) {
      push(out, "number", num[0]);
      i += num[0].length;
      continue;
    }
    if (/^[{}\[\],:]+$/u.test(text[i])) push(out, "op", text[i]);
    else push(out, "plain", text[i]);
    i++;
  }
  return out;
}

function yamlSpans(text: string): SyntaxSpan[] {
  const hash = text.indexOf("#");
  const code = hash >= 0 ? text.slice(0, hash) : text;
  const out: SyntaxSpan[] = [];
  const m = code.match(/^(\s*)([-?]\s+)?([^:\s][^:]*)(:)(.*)$/u);
  if (m) {
    push(out, "plain", m[1]);
    if (m[2]) push(out, "op", m[2]);
    push(out, "key", m[3]);
    push(out, "op", m[4]);
    appendScalar(out, m[5]);
  } else {
    appendScalar(out, code);
  }
  if (hash >= 0) push(out, "comment", text.slice(hash));
  return out;
}

function appendScalar(out: SyntaxSpan[], text: string) {
  let i = 0;
  while (i < text.length) {
    const rest = text.slice(i);
    const lit = rest.match(/^(true|false|null|yes|no)\b/iu);
    if (lit) {
      push(out, "literal", lit[0]);
      i += lit[0].length;
      continue;
    }
    const num = rest.match(/^-?\d+(?:\.\d+)?\b/u);
    if (num) {
      push(out, "number", num[0]);
      i += num[0].length;
      continue;
    }
    if (text[i] === "\"" || text[i] === "'") {
      const j = quotedEnd(text, i, text[i]);
      push(out, "string", text.slice(i, j));
      i = j;
      continue;
    }
    push(out, "plain", text[i]);
    i++;
  }
}

function markdownSpans(text: string): SyntaxSpan[] {
  const out: SyntaxSpan[] = [];
  const heading = text.match(/^(#{1,6})(\s+.*)$/u);
  if (heading) {
    push(out, "heading", heading[1]);
    push(out, "plain", heading[2]);
    return out;
  }
  const quote = text.match(/^(\s*>+)(.*)$/u);
  if (quote) {
    push(out, "comment", quote[1]);
    inlineMarkdown(out, quote[2]);
    return out;
  }
  const list = text.match(/^(\s*(?:[-*+]|\d+[.)])\s+)(.*)$/u);
  if (list) {
    push(out, "keyword", list[1]);
    inlineMarkdown(out, list[2]);
    return out;
  }
  if (/^\s*`{3,}/u.test(text)) {
    push(out, "string", text);
    return out;
  }
  inlineMarkdown(out, text);
  return out;
}

function inlineMarkdown(out: SyntaxSpan[], text: string) {
  let i = 0;
  const re = /(`[^`]*`|\[[^\]]+\]\([^)]+\)|https?:\/\/\S+)/gu;
  for (const m of text.matchAll(re)) {
    if (m.index! > i) push(out, "plain", text.slice(i, m.index));
    push(out, m[0].startsWith("`") ? "string" : "link", m[0]);
    i = m.index! + m[0].length;
  }
  if (i < text.length) push(out, "plain", text.slice(i));
}

function logSpans(text: string): SyntaxSpan[] {
  const out: SyntaxSpan[] = [];
  const ts = text.match(/^(\s*(?:\d{4}-\d\d-\d\d[T\s]\d\d:\d\d:\d\d(?:[.,]\d+)?Z?|\[\d{4}-\d\d-\d\d[^\]]*\]))/u);
  let offset = 0;
  if (ts) {
    push(out, "number", ts[1]);
    offset = ts[1].length;
  }
  const rest = text.slice(offset);
  const level = rest.match(/\b(TRACE|DEBUG|INFO|WARN|WARNING|ERROR|FATAL|CRITICAL)\b/u);
  if (!level || level.index == null) {
    push(out, "plain", rest);
    return out;
  }
  push(out, "plain", rest.slice(0, level.index));
  push(out, logLevelKind(level[1]), level[1]);
  push(out, "plain", rest.slice(level.index + level[1].length));
  return out;
}

function logLevelKind(level: string): SyntaxKind {
  if (level === "TRACE") return "level-trace";
  if (level === "DEBUG") return "level-debug";
  if (level === "INFO") return "level-info";
  if (level === "WARN" || level === "WARNING") return "level-warn";
  return "level-error";
}

function commentPrefix(lang: Language): string {
  if (lang === "python" || lang === "shell" || lang === "yaml") return "#";
  if (lang === "sql") return "--";
  if (lang === "css" || lang === "go" || lang === "javascript" || lang === "rust") return "//";
  return "";
}

function quotedEnd(text: string, start: number, quote: string): number {
  let i = start + 1;
  while (i < text.length) {
    if (text[i] === "\\") {
      i += 2;
      continue;
    }
    if (text[i] === quote) return i + 1;
    i++;
  }
  return text.length;
}

function nextNonSpace(text: string, start: number): string {
  for (let i = start; i < text.length; i++) {
    if (!/\s/u.test(text[i])) return text[i];
  }
  return "";
}

function basename(path: string): string {
  const normalized = String(path || "").replaceAll("\\", "/");
  return normalized.slice(normalized.lastIndexOf("/") + 1);
}

function push(out: SyntaxSpan[], kind: SyntaxKind, text: string) {
  if (!text) return;
  const last = out[out.length - 1];
  if (last && last.kind === kind) {
    last.text += text;
  } else {
    out.push({ kind, text });
  }
}
