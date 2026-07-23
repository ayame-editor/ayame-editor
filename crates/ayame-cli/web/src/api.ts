// Ayame Editor — api module. Type-stripped to JS at build time (build.rs, oxc).
// ---- tiny helpers -----------------------------------------------------------

export type LineRecord = {
  inserted?: boolean;
  number?: number;
  text?: string;
};

export type LinesResponse = {
  lines: LineRecord[];
  markers?: { kind: string; line: number }[];
  total: number;
};

export type ChangeMarkerOverview = {
  count: number;
  histogram: number[];
};

export type ChangeHistoryResponse = {
  revision: number;
  total_lines: number;
  saved: ChangeMarkerOverview;
  unsaved: ChangeMarkerOverview;
  deleted: ChangeMarkerOverview;
  limit_reached: boolean;
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

export type MarkerMutationResponse = {
  kind: string;
  line: number;
  marked: boolean;
  count: number;
  limit: number;
};

export type MarkerListResponse = {
  kind: string;
  total: number;
  lines: number[];
  truncated: boolean;
};

export type MarkerBulkResponse = {
  kind: string;
  added: number;
  count: number;
  limit: number;
  limit_reached: boolean;
};

export type MarkerSaveRequest = {
  kind: string;
  path: string;
  overwrite: boolean;
};

export type MarkerSaveResponse = {
  path: string;
  lines: number;
  bytes: number;
};

export type MarkerNavigateResponse = {
  kind: string;
  line: number | null;
  count: number;
  wrapped: boolean;
};

export type MarkerPreviewResponse = {
  kind: string;
  total: number;
  entries: { line: number; text: string; truncated: boolean }[];
  truncated: boolean;
};

export type ApiError = Error & { code?: string };

export function isApiErrorCode(error: unknown, code: string): boolean {
  return !!error && typeof error === "object" && (error as ApiError).code === code;
}

// Server errors are JSON `{ code, message }` (issue #81.2). Parse them into an
// Error whose `.message` is the human text and whose `.code` is the stable
// machine-readable slug, so callers branch on `code` instead of matching
// localized message text. Non-JSON bodies (routing 404s, upstream proxies) fall
// back to the raw text.
async function errorFromResponse(r: Response): Promise<ApiError> {
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
  const err = new Error(message) as ApiError;
  if (code) err.code = code;
  return err;
}

export async function api<T = unknown>(path: string, init?: RequestInit): Promise<T> {
  const r = await fetch(path, init);
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
