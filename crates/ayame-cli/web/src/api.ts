// Ayame Editor — api module. Type-stripped to JS at build time (build.rs, oxc).
// ---- tiny helpers -----------------------------------------------------------

export type {
  ChangeHistoryResponse,
  CompletionRequest,
  CompletionResponse,
  FindResponse,
  GrepResponse,
  InspectRequest,
  InspectResponse,
  LineByteResponse,
  LineRecord,
  LinesResponse,
  PositionResolveRequest,
  PositionResolveResponse,
  RecognizeRequest,
  RecognizeResponse,
  ExternalActionRequest,
  ExternalActionResponse,
  MarkerBulkResponse,
  MarkerListResponse,
  MarkerRangeCountsResponse,
  MarkerMutationResponse,
  MarkerNavigateResponse,
  MarkerPreviewResponse,
  MarkerSaveRequest,
  MarkerSaveResponse,
  OpenResponse,
  ParseEscapeRequest,
  ParseEscapeResponse,
  SearchHit,
  SearchResponse,
  StatResponse,
  TailPollResponse,
} from "./types/api.js";

export type BatchEditResponse = {
  carets?: { line: number; col: number }[];
  stats: { total_lines: number };
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
  signal?: AbortSignal,
): Promise<T> {
  const r = await fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });
  if (!r.ok) throw await errorFromResponse(r);
  return r.json();
}
