// Ayame Editor — api module. Type-stripped to JS at build time (build.rs, oxc).
// ---- tiny helpers -----------------------------------------------------------

export type LineRecord = {
  inserted?: boolean;
  number?: number;
  text?: string;
};

export type LinesResponse = {
  lines: LineRecord[];
  total: number;
};

export type LineByteResponse = {
  byte?: number;
};

export type FindHit = {
  byte: number;
  byte_len: number;
  column: number;
  line: number;
};

export type FindResponse = {
  hit: FindHit | null;
};

export type SearchHit = FindHit & {
  text?: string;
};

export type SearchResponse = {
  hits: SearchHit[];
  truncated: boolean;
};

export type BatchEditResponse = {
  carets?: { line: number; col: number }[];
  stats: { total_lines: number };
};

export type DiffResponse = {
  hunk_count: number;
  [key: string]: unknown;
};

// A failed API call throws this: it carries the server's machine-readable
// `code` (see the Rust `ApiErrorBody`) alongside the human `message`, so UI
// code can branch on `code` (e.g. isExistsError) and localize the message
// without string-matching.
export class ApiError extends Error {
  code: string;
  constructor(message: string, code: string) {
    super(message);
    this.name = "ApiError";
    this.code = code;
  }
}

// Overwrite-conflict predicate, keyed on the stable server code — the single
// definition every module shares (save.ts and selection.ts import it).
export function isExistsError(e: unknown): boolean {
  return (e as { code?: unknown } | null | undefined)?.code === "exists";
}

// Turn a non-OK response into an ApiError. The body is the JSON `{code,
// message}` shape; a non-JSON/unparseable body falls back to code "error".
async function throwResponseError(r: Response): Promise<never> {
  const text = await r.text();
  try {
    const body = JSON.parse(text);
    if (body && typeof body === "object" && typeof body.code === "string") {
      throw new ApiError(String(body.message ?? r.statusText), body.code);
    }
  } catch (e) {
    if (e instanceof ApiError) throw e;
    // Not JSON (or no code field): fall through to the generic error below.
  }
  throw new ApiError(text || r.statusText, "error");
}

export async function api<T = unknown>(path: string): Promise<T> {
  const r = await fetch(path);
  if (!r.ok) await throwResponseError(r);
  return r.json();
}

export async function apiPost<T = unknown, B = Record<string, unknown>>(
  path: string,
  body: B = {} as B,
): Promise<T> {
  const r = await fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!r.ok) await throwResponseError(r);
  return r.json();
}
