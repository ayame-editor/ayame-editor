// Lightweight visible-row syntax highlighting. This deliberately stays
// line-local: no parser state, no whole-file scans, and no dependency that can
// make huge files feel less like plain text.

import type { MessageKey } from "./i18n.js";

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

export type SyntaxHighlighter = (text: string, line: number) => SyntaxSpan[] | null;

export type StructureProviderId = "brace" | "indent" | "log" | "markup";

export type SyntaxSchemeDefinition = {
  id: string;
  labelKey: MessageKey;
  categoryKey: MessageKey;
  aliases: {
    extensions?: readonly string[];
    filenames?: readonly string[];
    globs?: readonly string[];
  };
  priority: number;
  tokenKinds: readonly SyntaxKind[];
  /** Reserved, bounded look-behind for a future line-local stateful scheme. */
  contextLines?: number;
  structure?: StructureProviderId;
  highlight: SyntaxHighlighter;
};

// One registry drives detection, the scheme picker, favorites, validation and
// tests. Highlighters stay line-local and receive only the visible row (#244).
export const SYNTAX_SCHEMES = [
  {
    id: "plain",
    labelKey: "syntax.scheme.plain",
    categoryKey: "syntax.category.text",
    aliases: {},
    priority: 0,
    tokenKinds: [],
    highlight: plainSpans,
  },
  {
    id: "json",
    labelKey: "syntax.scheme.json",
    categoryKey: "syntax.category.data",
    aliases: {
      extensions: ["json", "jsonc", "jsonl", "lock"],
      filenames: ["Cargo.lock", "package-lock.json"],
    },
    priority: 40,
    tokenKinds: ["comment", "key", "string", "number", "literal", "op"],
    structure: "brace",
    highlight: jsonSpans,
  },
  {
    id: "yaml",
    labelKey: "syntax.scheme.yaml",
    categoryKey: "syntax.category.data",
    aliases: { extensions: ["yaml", "yml"], filenames: ["pnpm-lock.yaml"] },
    priority: 40,
    tokenKinds: ["comment", "key", "string", "number", "literal", "op"],
    structure: "indent",
    highlight: yamlSpans,
  },
  {
    id: "toml",
    labelKey: "syntax.scheme.toml",
    categoryKey: "syntax.category.config",
    aliases: { extensions: ["toml"], filenames: ["Cargo.toml", "pyproject.toml"] },
    priority: 50,
    tokenKinds: ["comment", "heading", "key", "string", "number", "literal", "op"],
    highlight: tomlSpans,
  },
  {
    id: "ini",
    labelKey: "syntax.scheme.ini",
    categoryKey: "syntax.category.config",
    aliases: { extensions: ["ini", "cfg", "conf", "properties"] },
    priority: 10,
    tokenKinds: ["comment", "heading", "key", "string", "number", "literal", "op"],
    highlight: iniSpans,
  },
  {
    id: "csv",
    labelKey: "syntax.scheme.csv",
    categoryKey: "syntax.category.data",
    aliases: { extensions: ["csv"] },
    priority: 30,
    tokenKinds: ["heading", "string", "number", "literal", "op"],
    highlight: (text, line) => delimitedSpans(text, ",", line),
  },
  {
    id: "tsv",
    labelKey: "syntax.scheme.tsv",
    categoryKey: "syntax.category.data",
    aliases: { extensions: ["tsv", "tab"] },
    priority: 30,
    tokenKinds: ["heading", "string", "number", "literal", "op"],
    highlight: (text, line) => delimitedSpans(text, "\t", line),
  },
  {
    id: "markdown",
    labelKey: "syntax.scheme.markdown",
    categoryKey: "syntax.category.markup",
    aliases: { extensions: ["md", "mdx", "markdown"] },
    priority: 30,
    tokenKinds: ["comment", "heading", "keyword", "string", "link"],
    highlight: markdownSpans,
  },
  {
    id: "html",
    labelKey: "syntax.scheme.html",
    categoryKey: "syntax.category.markup",
    aliases: { extensions: ["html", "htm"] },
    priority: 30,
    tokenKinds: ["comment", "keyword", "key", "string", "op"],
    structure: "markup",
    highlight: markupSpans,
  },
  {
    id: "xml",
    labelKey: "syntax.scheme.xml",
    categoryKey: "syntax.category.markup",
    aliases: { extensions: ["xml", "xsl", "xslt", "svg"] },
    priority: 35,
    tokenKinds: ["comment", "keyword", "key", "string", "op"],
    structure: "markup",
    highlight: markupSpans,
  },
  {
    id: "log",
    labelKey: "syntax.scheme.log",
    categoryKey: "syntax.category.log",
    aliases: { extensions: ["log", "out"] },
    priority: 30,
    tokenKinds: ["number", "level-trace", "level-debug", "level-info", "level-warn", "level-error"],
    structure: "log",
    highlight: logSpans,
  },
  {
    id: "javascript",
    labelKey: "syntax.scheme.javascript",
    categoryKey: "syntax.category.programming",
    aliases: { extensions: ["js", "jsx", "mjs", "cjs", "ts", "tsx"] },
    priority: 30,
    tokenKinds: ["comment", "keyword", "string", "number", "literal", "function", "op"],
    structure: "brace",
    highlight: (text) => codeSpans(text, "javascript"),
  },
  {
    id: "python",
    labelKey: "syntax.scheme.python",
    categoryKey: "syntax.category.programming",
    aliases: { extensions: ["py", "pyi"], filenames: ["SConstruct"] },
    priority: 30,
    tokenKinds: ["comment", "keyword", "string", "number", "literal", "function", "op"],
    structure: "indent",
    highlight: (text) => codeSpans(text, "python"),
  },
  {
    id: "rust",
    labelKey: "syntax.scheme.rust",
    categoryKey: "syntax.category.programming",
    aliases: { extensions: ["rs"] },
    priority: 30,
    tokenKinds: ["comment", "keyword", "string", "number", "literal", "function", "op"],
    structure: "brace",
    highlight: (text) => codeSpans(text, "rust"),
  },
  {
    id: "go",
    labelKey: "syntax.scheme.go",
    categoryKey: "syntax.category.programming",
    aliases: { extensions: ["go"] },
    priority: 30,
    tokenKinds: ["comment", "keyword", "string", "number", "literal", "function", "op"],
    structure: "brace",
    highlight: (text) => codeSpans(text, "go"),
  },
  {
    id: "c",
    labelKey: "syntax.scheme.c",
    categoryKey: "syntax.category.programming",
    aliases: { extensions: ["c", "h", "cc", "cpp", "cxx", "hpp", "hxx"] },
    priority: 30,
    tokenKinds: ["comment", "keyword", "string", "number", "literal", "function", "op"],
    structure: "brace",
    highlight: (text) => codeSpans(text, "c"),
  },
  {
    id: "java",
    labelKey: "syntax.scheme.java",
    categoryKey: "syntax.category.programming",
    aliases: { extensions: ["java"] },
    priority: 30,
    tokenKinds: ["comment", "keyword", "string", "number", "literal", "function", "op"],
    structure: "brace",
    highlight: (text) => codeSpans(text, "java"),
  },
  {
    id: "csharp",
    labelKey: "syntax.scheme.csharp",
    categoryKey: "syntax.category.programming",
    aliases: { extensions: ["cs"] },
    priority: 30,
    tokenKinds: ["comment", "keyword", "string", "number", "literal", "function", "op"],
    structure: "brace",
    highlight: (text) => codeSpans(text, "csharp"),
  },
  {
    id: "sql",
    labelKey: "syntax.scheme.sql",
    categoryKey: "syntax.category.data",
    aliases: { extensions: ["sql"] },
    priority: 30,
    tokenKinds: ["comment", "keyword", "string", "number", "literal", "function", "op"],
    highlight: (text) => codeSpans(text, "sql"),
  },
  {
    id: "shell",
    labelKey: "syntax.scheme.shell",
    categoryKey: "syntax.category.programming",
    aliases: {
      extensions: ["sh", "bash", "zsh", "fish"],
      filenames: [".bashrc", ".zshrc", ".profile"],
    },
    priority: 30,
    tokenKinds: ["comment", "keyword", "string", "number", "literal", "function", "op"],
    highlight: (text) => codeSpans(text, "shell"),
  },
  {
    id: "css",
    labelKey: "syntax.scheme.css",
    categoryKey: "syntax.category.markup",
    aliases: { extensions: ["css", "scss", "less"] },
    priority: 30,
    tokenKinds: ["comment", "keyword", "string", "number", "function", "op"],
    structure: "brace",
    highlight: (text) => codeSpans(text, "css"),
  },
  {
    id: "dockerfile",
    labelKey: "syntax.scheme.dockerfile",
    categoryKey: "syntax.category.build",
    aliases: { filenames: ["Dockerfile", "Containerfile"], globs: ["Dockerfile.*"] },
    priority: 80,
    tokenKinds: ["comment", "keyword", "string", "number", "literal", "function", "op"],
    highlight: dockerfileSpans,
  },
  {
    id: "makefile",
    labelKey: "syntax.scheme.makefile",
    categoryKey: "syntax.category.build",
    aliases: { filenames: ["Makefile", "GNUmakefile"], extensions: ["mk"] },
    priority: 80,
    tokenKinds: ["comment", "key", "keyword", "string", "number", "literal", "function", "op"],
    highlight: makefileSpans,
  },
  {
    id: "nginx",
    labelKey: "syntax.scheme.nginx",
    categoryKey: "syntax.category.config",
    aliases: { filenames: ["nginx.conf"], globs: ["*/sites-available/*", "*/sites-enabled/*"] },
    priority: 100,
    tokenKinds: ["comment", "keyword", "string", "number", "literal", "function", "op"],
    highlight: configSpans,
  },
  {
    id: "apache",
    labelKey: "syntax.scheme.apache",
    categoryKey: "syntax.category.config",
    aliases: { filenames: ["httpd.conf", "apache2.conf", ".htaccess"] },
    priority: 100,
    tokenKinds: ["comment", "keyword", "string", "number", "literal", "function", "op"],
    highlight: configSpans,
  },
] as const satisfies readonly SyntaxSchemeDefinition[];

export type SchemeId = (typeof SYNTAX_SCHEMES)[number]["id"];
export type SyntaxSelection = "auto" | SchemeId;

export type SyntaxGlobMapping = {
  glob: string;
  scheme: SchemeId;
};

const SCHEME_BY_ID = new Map<SchemeId, (typeof SYNTAX_SCHEMES)[number]>(
  SYNTAX_SCHEMES.map((scheme) => [scheme.id, scheme]),
);

export function isSchemeId(value: unknown): value is SchemeId {
  return typeof value === "string" && SCHEME_BY_ID.has(value as SchemeId);
}

export function schemeDefinition(id: SchemeId): SyntaxSchemeDefinition {
  return SCHEME_BY_ID.get(id)!;
}

const COMMON_LITERALS = new Set(["false", "nil", "null", "None", "true"]);

const KEYWORDS: Record<string, Set<string>> = {
  c: new Set([
    "auto",
    "break",
    "case",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extern",
    "float",
    "for",
    "if",
    "inline",
    "int",
    "long",
    "namespace",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "switch",
    "template",
    "typedef",
    "union",
    "unsigned",
    "using",
    "virtual",
    "void",
    "volatile",
    "while",
  ]),
  csharp: new Set([
    "abstract",
    "as",
    "async",
    "await",
    "base",
    "bool",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "decimal",
    "default",
    "delegate",
    "do",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "false",
    "finally",
    "fixed",
    "for",
    "foreach",
    "if",
    "implicit",
    "in",
    "interface",
    "internal",
    "is",
    "lock",
    "namespace",
    "new",
    "null",
    "operator",
    "out",
    "override",
    "params",
    "private",
    "protected",
    "public",
    "readonly",
    "record",
    "ref",
    "return",
    "sealed",
    "static",
    "struct",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "using",
    "var",
    "virtual",
    "void",
    "while",
  ]),
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
  java: new Set([
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "final",
    "finally",
    "float",
    "for",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "package",
    "private",
    "protected",
    "public",
    "record",
    "return",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "try",
    "var",
    "void",
    "volatile",
    "while",
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

export function syntaxGlobMatches(pattern: string, path: string): boolean {
  const normalizedPattern = String(pattern || "")
    .trim()
    .replaceAll("\\", "/")
    .toLowerCase();
  const normalizedPath = String(path || "")
    .replaceAll("\\", "/")
    .toLowerCase();
  if (!normalizedPattern || normalizedPattern.length > 256) return false;
  const value = normalizedPattern.includes("/") ? normalizedPath : basename(normalizedPath);
  let patternIndex = 0;
  let valueIndex = 0;
  let starIndex = -1;
  let starValueIndex = -1;
  while (valueIndex < value.length) {
    if (
      patternIndex < normalizedPattern.length &&
      (normalizedPattern[patternIndex] === "?" ||
        normalizedPattern[patternIndex] === value[valueIndex])
    ) {
      patternIndex++;
      valueIndex++;
    } else if (normalizedPattern[patternIndex] === "*") {
      starIndex = patternIndex++;
      starValueIndex = valueIndex;
    } else if (starIndex >= 0) {
      patternIndex = starIndex + 1;
      valueIndex = ++starValueIndex;
    } else {
      return false;
    }
  }
  while (normalizedPattern[patternIndex] === "*") patternIndex++;
  return patternIndex === normalizedPattern.length;
}

export function languageForPath(
  path: string,
  mappings: readonly SyntaxGlobMapping[] = [],
): SchemeId | null {
  for (const mapping of mappings) {
    if (isSchemeId(mapping.scheme) && syntaxGlobMatches(mapping.glob, path)) return mapping.scheme;
  }
  const normalized = String(path || "").replaceAll("\\", "/");
  const name = basename(normalized).toLowerCase();
  const dot = name.lastIndexOf(".");
  const extension = dot >= 0 ? name.slice(dot + 1) : "";
  let best: { id: SchemeId; score: number } | null = null;
  for (const scheme of SYNTAX_SCHEMES) {
    const aliases: SyntaxSchemeDefinition["aliases"] = scheme.aliases;
    let match = 0;
    if (aliases.filenames?.some((candidate) => candidate.toLowerCase() === name)) match = 300;
    else if (aliases.globs?.some((glob) => syntaxGlobMatches(glob, normalized))) match = 200;
    else if (extension && aliases.extensions?.some((candidate) => candidate === extension)) {
      match = 100;
    }
    const score = match + scheme.priority;
    if (match && (!best || score > best.score)) best = { id: scheme.id, score };
  }
  return best?.id ?? null;
}

export function resolveSyntaxScheme(
  path: string,
  selection: SyntaxSelection = "auto",
  mappings: readonly SyntaxGlobMapping[] = [],
): SchemeId | null {
  return selection === "auto" ? languageForPath(path, mappings) : selection;
}

// ---- memoization (#142) -----------------------------------------------------
//
// Every visible line was re-tokenized on every frame, including the frames
// where only the caret moved. Rows are now skipped when nothing about them
// changed, but a scroll still walks the same lines back and forth, and one
// document's lines repeat: the same `text` tokenizes to the same spans.
//
// Bounded on both axes. Entries are capped so a long scroll cannot grow the
// cache without limit, and long lines are not cached at all — one 10 MB line
// would cost more to hold than to re-tokenize, and it is exactly the shape a
// giant-file editor meets.

const SPAN_CACHE_MAX = 2048;
const SPAN_CACHE_MAX_TEXT = 4096;
export const SYNTAX_LINE_LIMIT = 32 * 1024;

// Insertion-ordered, so the oldest key is the first one Map iteration yields.
const spanCache = new Map<string, SyntaxSpan[] | null>();

export function clearSyntaxCache() {
  spanCache.clear();
  pathLanguage = { path: "\0", lang: null };
}

export function syntaxCacheSize() {
  return spanCache.size;
}

// Every row of a render asks about the same path; parsing the basename and
// walking the extension table once per line is pure repetition.
let pathLanguage: { path: string; lang: SchemeId | null } = { path: "\0", lang: null };

function languageForPathCached(path: string): SchemeId | null {
  if (pathLanguage.path !== path) pathLanguage = { path, lang: languageForPath(path) };
  return pathLanguage.lang;
}

function tokenize(text: string, scheme: SchemeId, line: number): SyntaxSpan[] | null {
  return schemeDefinition(scheme).highlight(text, line);
}

/// Spans for one line, or `null` when no language applies.
///
/// The returned array is shared with other callers asking for the same
/// (path, text) — treat it as read-only.
export function highlightSpans(
  text: string,
  path = "",
  options: {
    line?: number;
    mappings?: readonly SyntaxGlobMapping[];
    scheme?: SchemeId | null;
  } = {},
): SyntaxSpan[] | null {
  const line = options.line ?? -1;
  const pathScheme =
    options.scheme !== undefined
      ? options.scheme
      : options.mappings?.length
        ? languageForPath(path, options.mappings)
        : languageForPathCached(path);
  const source = text.length > SYNTAX_LINE_LIMIT ? text.slice(0, SYNTAX_LINE_LIMIT) : text;
  const scheme = pathScheme || inferLanguage(source);
  const tokenizeBounded = () => {
    const spans = scheme ? tokenize(source, scheme, line) : null;
    if (spans && source.length < text.length) push(spans, "plain", text.slice(source.length));
    return spans;
  };
  if (text.length > SPAN_CACHE_MAX_TEXT) {
    return tokenizeBounded();
  }
  // The language is part of the key: `inferLanguage` reads the line itself, so
  // the same text in a `.json` tab and an extensionless one can differ.
  const key = `${scheme ?? ""}\0${line === 0 ? "header" : "row"}\0${text}`;
  const hit = spanCache.get(key);
  if (hit !== undefined) {
    // Refresh recency: re-inserting moves the key to the end of the order.
    spanCache.delete(key);
    spanCache.set(key, hit);
    return hit;
  }
  const spans = tokenizeBounded();
  if (spanCache.size >= SPAN_CACHE_MAX) {
    const oldest = spanCache.keys().next().value;
    if (oldest !== undefined) spanCache.delete(oldest);
  }
  spanCache.set(key, spans);
  return spans;
}

function inferLanguage(text: string): SchemeId | null {
  if (/^\s*(TRACE|DEBUG|INFO|WARN|WARNING|ERROR|FATAL|CRITICAL)\b/u.test(text)) return "log";
  if (/^\s*\d{4}-\d\d-\d\d[T\s]\d\d:\d\d:\d\d/u.test(text)) return "log";
  if (/^\s*[{[]\s*$/u.test(text) || /^\s*"[^"]+"\s*:/u.test(text)) return "json";
  return null;
}

function codeSpans(text: string, lang: string): SyntaxSpan[] {
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
    if (ch === '"' || ch === "'" || (ch === "`" && lang === "javascript")) {
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
      const kind =
        keywords.has(word) || keywords.has(lower)
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

function plainSpans(): null {
  return null;
}

function tomlSpans(text: string): SyntaxSpan[] {
  const trimmed = text.trimStart();
  if (trimmed.startsWith("#")) {
    const out: SyntaxSpan[] = [];
    push(out, "plain", text.slice(0, text.length - trimmed.length));
    push(out, "comment", trimmed);
    return out;
  }
  if (/^\[\[?.+\]\]?\s*(?:#.*)?$/u.test(trimmed)) {
    const hash = text.indexOf("#");
    return hash < 0
      ? [{ kind: "heading", text }]
      : [
          { kind: "heading", text: text.slice(0, hash) },
          { kind: "comment", text: text.slice(hash) },
        ];
  }
  return assignmentSpans(text, ["#"], ["="]);
}

function iniSpans(text: string): SyntaxSpan[] {
  const trimmed = text.trimStart();
  if (trimmed.startsWith("#") || trimmed.startsWith(";")) {
    const out: SyntaxSpan[] = [];
    push(out, "plain", text.slice(0, text.length - trimmed.length));
    push(out, "comment", trimmed);
    return out;
  }
  if (/^\[[^\]]+\]\s*$/u.test(trimmed)) return [{ kind: "heading", text }];
  return assignmentSpans(text, ["#", ";"], ["=", ":"]);
}

function assignmentSpans(
  text: string,
  commentMarkers: readonly string[],
  separators: readonly string[],
): SyntaxSpan[] {
  const out: SyntaxSpan[] = [];
  let commentAt = -1;
  let quote = "";
  for (let index = 0; index < text.length; index++) {
    const char = text[index];
    if (quote) {
      if (char === "\\") index++;
      else if (char === quote) quote = "";
      continue;
    }
    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }
    if (commentMarkers.includes(char)) {
      commentAt = index;
      break;
    }
  }
  const code = commentAt < 0 ? text : text.slice(0, commentAt);
  const separatorAt = separators
    .map((separator) => code.indexOf(separator))
    .filter((index) => index >= 0)
    .sort((a, b) => a - b)[0];
  if (separatorAt == null) appendScalar(out, code);
  else {
    push(out, "key", code.slice(0, separatorAt));
    push(out, "op", code[separatorAt]);
    appendScalar(out, code.slice(separatorAt + 1));
  }
  if (commentAt >= 0) push(out, "comment", text.slice(commentAt));
  return out;
}

function markupSpans(text: string): SyntaxSpan[] {
  const out: SyntaxSpan[] = [];
  let index = 0;
  while (index < text.length) {
    if (text.startsWith("<!--", index)) {
      const close = text.indexOf("-->", index + 4);
      const end = close < 0 ? text.length : close + 3;
      push(out, "comment", text.slice(index, end));
      index = end;
      continue;
    }
    if (text[index] !== "<") {
      const next = text.indexOf("<", index);
      const end = next < 0 ? text.length : next;
      push(out, "plain", text.slice(index, end));
      index = end;
      continue;
    }
    push(out, "op", "<");
    index++;
    if (text[index] === "/" || text[index] === "!" || text[index] === "?") {
      push(out, "op", text[index++]);
    }
    const tag = text.slice(index).match(/^[A-Za-z_][\w:.-]*/u)?.[0];
    if (tag) {
      push(out, "keyword", tag);
      index += tag.length;
    }
    while (index < text.length && text[index] !== ">") {
      const char = text[index];
      if (char === '"' || char === "'") {
        const end = quotedEnd(text, index, char);
        push(out, "string", text.slice(index, end));
        index = end;
        continue;
      }
      const attribute = text.slice(index).match(/^[A-Za-z_:][\w:.-]*/u)?.[0];
      if (attribute) {
        push(out, "key", attribute);
        index += attribute.length;
        continue;
      }
      push(out, /^[=/]$/u.test(char) ? "op" : "plain", char);
      index++;
    }
    if (text[index] === ">") push(out, "op", text[index++]);
  }
  return out;
}

function dockerfileSpans(text: string): SyntaxSpan[] {
  const out: SyntaxSpan[] = [];
  const match = text.match(/^(\s*)([A-Za-z]+)(\s+|$)(.*)$/u);
  if (!match || match[2].startsWith("#")) return configSpans(text);
  push(out, "plain", match[1]);
  push(out, "keyword", match[2]);
  push(out, "plain", match[3]);
  for (const span of codeSpans(match[4], "shell")) push(out, span.kind, span.text);
  return out;
}

function makefileSpans(text: string): SyntaxSpan[] {
  if (text.startsWith("\t")) {
    const out: SyntaxSpan[] = [{ kind: "plain", text: "\t" }];
    for (const span of codeSpans(text.slice(1), "shell")) push(out, span.kind, span.text);
    return out;
  }
  const trimmed = text.trimStart();
  if (trimmed.startsWith("#")) return configSpans(text);
  const match = text.match(/^(\s*)([^:=]+)(::?=|\??=|\+=|:)(.*)$/u);
  if (!match) return configSpans(text);
  const out: SyntaxSpan[] = [];
  push(out, "plain", match[1]);
  push(out, "key", match[2]);
  push(out, "op", match[3]);
  for (const span of codeSpans(match[4], "shell")) push(out, span.kind, span.text);
  return out;
}

function configSpans(text: string): SyntaxSpan[] {
  const out: SyntaxSpan[] = [];
  const match = text.match(/^(\s*)(#.*|[A-Za-z_][\w.-]*)(.*)$/u);
  if (!match) return [{ kind: "plain", text }];
  push(out, "plain", match[1]);
  if (match[2].startsWith("#")) {
    push(out, "comment", match[2] + match[3]);
    return out;
  }
  push(out, "keyword", match[2]);
  for (const span of codeSpans(match[3], "shell")) push(out, span.kind, span.text);
  return out;
}

function delimitedSpans(text: string, delimiter: string, line: number): SyntaxSpan[] {
  const out: SyntaxSpan[] = [];
  let start = 0;
  let quoted = false;
  for (let index = 0; index <= text.length; index++) {
    const char = text[index];
    if (char === '"') {
      if (quoted && text[index + 1] === '"') index++;
      else quoted = !quoted;
    }
    if (index < text.length && (char !== delimiter || quoted)) continue;
    appendDelimitedField(out, text.slice(start, index), line === 0);
    if (index < text.length) push(out, "op", delimiter);
    start = index + 1;
  }
  return out;
}

function appendDelimitedField(out: SyntaxSpan[], field: string, header: boolean) {
  if (header) {
    push(out, "heading", field);
    return;
  }
  const trimmed = field.trim();
  const start = field.indexOf(trimmed);
  if (start > 0) push(out, "plain", field.slice(0, start));
  const kind =
    /^"(?:[^"]|"")*"$/u.test(trimmed) || /^'.*'$/u.test(trimmed)
      ? "string"
      : /^[-+]?\d+(?:\.\d+)?(?:e[-+]?\d+)?$/iu.test(trimmed)
        ? "number"
        : /^(?:true|false|null)$/iu.test(trimmed)
          ? "literal"
          : "plain";
  push(out, kind, trimmed);
  push(out, "plain", field.slice(start + trimmed.length));
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
    if (text[i] === '"') {
      const j = quotedEnd(text, i, '"');
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
    if (/^[{}[\],:]+$/u.test(text[i])) push(out, "op", text[i]);
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
    if (text[i] === '"' || text[i] === "'") {
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
  const ts = text.match(
    /^(\s*(?:\d{4}-\d\d-\d\d[T\s]\d\d:\d\d:\d\d(?:[.,]\d+)?Z?|\[\d{4}-\d\d-\d\d[^\]]*\]))/u,
  );
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

function commentPrefix(lang: string): string {
  if (lang === "python" || lang === "shell" || lang === "yaml") return "#";
  if (lang === "sql") return "--";
  if (
    lang === "c" ||
    lang === "csharp" ||
    lang === "go" ||
    lang === "java" ||
    lang === "javascript" ||
    lang === "rust"
  ) {
    return "//";
  }
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
