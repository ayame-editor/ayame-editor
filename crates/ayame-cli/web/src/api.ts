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

// Server errors are JSON `{ code, message }` (issue #81.2). Parse them into an
// Error whose `.message` is the human text and whose `.code` is the stable
// machine-readable slug, so callers branch on `code` instead of matching
// localized message text. Non-JSON bodies (routing 404s, upstream proxies) fall
// back to the raw text.
async function errorFromResponse(r: Response): Promise<Error> {
  const text = await r.text();
  let message = text || r.statusText;
  let code: string | undefined;
  try {
    const body = JSON.parse(text);
    if (body && typeof body === "object" && typeof body.message === "string") {
      message = body.message;
      if (typeof body.code === "string") code = body.code;
    }
  } catch {
    // Not JSON — keep the raw text as the message.
  }
  const err = new Error(message) as Error & { code?: string };
  if (code) err.code = code;
  return err;
}

export async function api<T = unknown>(path: string): Promise<T> {
  const r = await fetch(path);
  if (!r.ok) throw await errorFromResponse(r);
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
  if (!r.ok) throw await errorFromResponse(r);
  return r.json();
}
