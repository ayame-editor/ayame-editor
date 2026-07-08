// Ayame Editor — long-op progress runner. Type-stripped to JS at build time.
//
// Drives a worker-backed op (sort / grep-save / split) with a determinate
// progress bar and a Cancel button (#78): it injects a client-generated
// `op_id` into the request, polls `/api/op/progress` while the POST is in
// flight, and hits `/api/op/cancel` when the user cancels. The overlay it
// raises counts as a modal, so edits stay blocked for the duration.
import { apiPost } from "./api.js";
import { hideLoading, showProgress } from "./dialogs.js";

// Thrown when the user cancelled the op — callers can treat it as a benign
// no-op rather than an error to surface.
export class OpCancelled extends Error {
  constructor() {
    super("cancelled");
    this.name = "OpCancelled";
  }
}

export function isOpCancelled(e: unknown): boolean {
  return e instanceof OpCancelled;
}

let opSeq = 0;

function newOpId(): string {
  opSeq += 1;
  return `op-${opSeq}-${Math.random().toString(36).slice(2)}`;
}

type ProgressReply = { done: number; total: number; active: boolean };

// Run a long worker op with progress + cancel. `body` is posted to `url` with
// the `op_id` field filled in here. Resolves with the response, or rejects with
// `OpCancelled` if the user cancelled (or the underlying fetch error otherwise).
export async function runTrackedOp<Res, Req extends { op_id?: string | null }>(
  label: string,
  url: string,
  body: Omit<Req, "op_id">,
): Promise<Res> {
  const opId = newOpId();
  let cancelled = false;

  const handle = showProgress(label, () => {
    cancelled = true;
    // Fire-and-forget: the main POST will reject once the worker is killed.
    void fetch(`/api/op/cancel?id=${encodeURIComponent(opId)}`, { method: "POST" }).catch(() => {});
  });

  const poll = async () => {
    try {
      const r = await fetch(`/api/op/progress?id=${encodeURIComponent(opId)}`);
      if (!r.ok) return;
      const p: ProgressReply = await r.json();
      handle.setProgress(p.done, p.total);
    } catch {
      // Transient poll failures are ignored; the bar just stops advancing.
    }
  };
  const timer = setInterval(() => void poll(), 300);

  try {
    return await apiPost<Res, Req>(url, { ...body, op_id: opId } as Req);
  } catch (e) {
    if (cancelled) throw new OpCancelled();
    throw e;
  } finally {
    clearInterval(timer);
    hideLoading();
  }
}
