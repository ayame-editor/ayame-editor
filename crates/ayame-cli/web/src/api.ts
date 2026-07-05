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

export async function api<T = unknown>(path: string): Promise<T> {
  const r = await fetch(path);
  if (!r.ok) throw new Error((await r.text()) || r.statusText);
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
  if (!r.ok) throw new Error((await r.text()) || r.statusText);
  return r.json();
}
