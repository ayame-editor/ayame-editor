// Ayame Editor — dialogs module. Type-stripped to JS at build time (build.rs, oxc).
import { api, apiPost } from "./api.js";
import { $, commas, setModalOpen } from "./dom.js";
import { t } from "./i18n.js";
import { focusEditor } from "./editor.js";
import type { ArtifactOpStatus, OperationCancelRequest } from "./types/api.js";

// ---- generic confirm / message dialog (replaces window.confirm/alert) -----
// The browser dialogs leak the server origin into their chrome
// ("127.0.0.1:PORT の内容"); everything user-facing goes through this modal.
export function confirmVisible() {
  return !$("confirm").classList.contains("hidden");
}

type Listener = [EventTarget, string, EventListener];

function runModal(
  modal,
  focus: () => void,
  setup: (
    finish: (value: any) => void,
    on: (target: EventTarget, event: string, listener: EventListener) => void,
  ) => void,
): Promise<any> {
  return new Promise((resolve) => {
    const listeners: Listener[] = [];
    const on = (target: EventTarget, event: string, listener: EventListener) => {
      target.addEventListener(event, listener);
      listeners.push([target, event, listener]);
    };
    const finish = (value) => {
      setModalOpen(modal, false);
      for (const [target, event, listener] of listeners) {
        target.removeEventListener(event, listener);
      }
      focusEditor();
      resolve(value);
    };
    setModalOpen(modal, true);
    setup(finish, on);
    focus();
  });
}

function backdropCancel(modal, finish: (value: any) => void) {
  return (ev) => {
    if (ev.target === modal) finish(null);
  };
}

// Titles, messages and button labels arrive already localized (t() results);
// server-side error details go through serverMessage() at the call site.
export function askConfirm(title, message, opts: any = {}): Promise<any> {
  const modal = $("confirm");
  const okBtn = $("confirm-ok");
  const cancelBtn = $("confirm-cancel");
  $("confirm-title").textContent = title || t("common.confirm");
  $("confirm-message").textContent = message || "";
  okBtn.textContent = opts.okLabel || t("common.ok");
  okBtn.classList.toggle("danger", !!opts.danger);
  okBtn.classList.toggle("primary", !opts.danger);
  cancelBtn.textContent = opts.cancelLabel || t("common.cancel");
  cancelBtn.classList.toggle("hidden", !!opts.alert);
  return runModal(
    modal,
    () => queueMicrotask(() => okBtn.focus()),
    (finish, on) => {
      const onOk = () => finish(true);
      const onCancel = () => finish(false);
      const onKey = (ev) => {
        ev.stopPropagation();
        if (ev.key === "Enter") {
          ev.preventDefault();
          finish(true);
        } else if (ev.key === "Escape") {
          ev.preventDefault();
          finish(false);
        } else if (ev.key === "ArrowLeft" || ev.key === "ArrowRight") {
          // Move focus between the Cancel / OK buttons, like a native dialog.
          if (opts.alert) return; // OK-only: nothing to move between
          ev.preventDefault();
          (document.activeElement === okBtn ? cancelBtn : okBtn).focus();
        }
      };
      const onBackdrop = (ev) => {
        if (ev.target === modal) finish(false);
      };
      on(okBtn, "click", onOk);
      on(cancelBtn, "click", onCancel);
      on($("confirm-close"), "click", onCancel);
      on(modal, "mousedown", onBackdrop);
      on(modal, "keydown", onKey);
    },
  );
}

// OK-only variant for error details and notices (replaces window.alert).
export function showMessage(title, message) {
  return askConfirm(title, message, { alert: true });
}

// ---- generic input prompt (replaces the browser's window.prompt) ---------
export function promptVisible() {
  return !$("prompt").classList.contains("hidden");
}

export function askPrompt(title, label, value = ""): Promise<any> {
  const modal = $("prompt");
  $("prompt-title").textContent = title || t("common.input");
  $("prompt-label").textContent = label || "";
  const input = $("prompt-input");
  input.value = value;
  return runModal(
    modal,
    () =>
      setTimeout(() => {
        input.focus();
        input.select();
      }, 0),
    (finish, on) => {
      const onOk = () => finish(input.value);
      const onCancel = () => finish(null);
      const onKey = (ev) => {
        ev.stopPropagation();
        if (ev.key === "Enter") {
          ev.preventDefault();
          finish(input.value);
        } else if (ev.key === "Escape") {
          ev.preventDefault();
          finish(null);
        }
      };
      on(input, "keydown", onKey);
      on(modal, "keydown", onKey);
      on($("prompt-ok"), "click", onOk);
      on($("prompt-cancel"), "click", onCancel);
      on($("prompt-close"), "click", onCancel);
      on(modal, "mousedown", backdropCancel(modal, finish));
    },
  );
}

// ---- generic small form dialog (sort / replace / case options) ------------
export function formVisible() {
  return !$("form-modal").classList.contains("hidden");
}

// fields: {id, type: "text"|"check"|"select"|"hint", label, value, placeholder,
// title, options}. All labels/placeholders/titles arrive already localized.
// Resolves to {id: value} or null on cancel.
export function askForm(title, fields, okLabel = null): Promise<any> {
  const modal = $("form-modal");
  const body = $("form-body");
  $("form-title").textContent = title || t("common.options");
  $("form-ok").textContent = okLabel || t("common.run");
  body.textContent = "";
  const readers: Record<string, () => any> = {};
  for (const f of fields) {
    if (f.type === "hint") {
      const hint = document.createElement("div");
      hint.className = "form-hint";
      hint.textContent = f.label;
      body.append(hint);
      continue;
    }
    if (f.type === "check") {
      const lab = document.createElement("label");
      lab.className = "form-check";
      if (f.title) lab.title = f.title;
      const cb = document.createElement("input");
      cb.type = "checkbox";
      cb.checked = !!f.value;
      lab.append(cb, document.createTextNode(f.label));
      body.append(lab);
      readers[f.id] = () => cb.checked;
      continue;
    }
    if (f.type === "path") {
      // A text field with a "Choose Folder" button that runs the caller's
      // picker (`onBrowse`) and writes the chosen path back (issue #79.1).
      const prow = document.createElement("div");
      prow.className = "form-row";
      const plabel = document.createElement("span");
      plabel.textContent = f.label;
      const wrap = document.createElement("div");
      wrap.className = "form-path";
      const input = document.createElement("input");
      input.type = "text";
      input.value = f.value ?? "";
      input.placeholder = f.placeholder ?? "";
      if (f.title) input.title = f.title;
      const browseBtn = document.createElement("button");
      browseBtn.type = "button";
      browseBtn.className = "cmd";
      browseBtn.textContent = f.browseLabel || t("dialog.open.chooseFolder");
      browseBtn.addEventListener("click", async () => {
        if (!f.onBrowse) return;
        const picked = await f.onBrowse(input.value);
        if (picked != null && picked !== "") input.value = picked;
        input.focus();
      });
      wrap.append(input, browseBtn);
      prow.append(plabel, wrap);
      body.append(prow);
      readers[f.id] = () => input.value;
      continue;
    }
    const row = document.createElement("label");
    row.className = "form-row";
    const span = document.createElement("span");
    span.textContent = f.label;
    row.append(span);
    if (f.type === "select") {
      const sel = document.createElement("select");
      for (const [v, text] of f.options || []) {
        const o = document.createElement("option");
        o.value = v;
        o.textContent = text;
        sel.append(o);
      }
      if (f.value != null) sel.value = f.value;
      row.append(sel);
      readers[f.id] = () => sel.value;
    } else {
      const input = document.createElement("input");
      input.type = "text";
      input.value = f.value ?? "";
      input.placeholder = f.placeholder ?? "";
      if (f.title) input.title = f.title;
      row.append(input);
      readers[f.id] = () => input.value;
    }
    body.append(row);
  }
  return runModal(
    modal,
    () =>
      queueMicrotask(() =>
        body.querySelector<HTMLInputElement | HTMLSelectElement>("input, select")?.focus(),
      ),
    (finish, on) => {
      const collect = () =>
        Object.fromEntries(Object.entries(readers).map(([k, read]) => [k, read()]));
      const onOk = () => finish(collect());
      const onCancel = () => finish(null);
      const onKey = (ev) => {
        ev.stopPropagation();
        if (ev.key === "Enter" && ev.target.tagName !== "SELECT") {
          ev.preventDefault();
          finish(collect());
        } else if (ev.key === "Escape") {
          ev.preventDefault();
          finish(null);
        }
      };
      on($("form-ok"), "click", onOk);
      on($("form-cancel"), "click", onCancel);
      on($("form-close"), "click", onCancel);
      on(modal, "mousedown", backdropCancel(modal, finish));
      on(modal, "keydown", onKey);
    },
  );
}

// ---- loading overlay ------------------------------------------------------
let loadingPoll: ReturnType<typeof setInterval> | null = null;
let loadingOpId: string | null = null;
// True once a status poll has seen the tracked op, so a later "not_found" means
// the worker finished and the op was evicted (finalizing) — not "no such op".
let loadingSeenOp = false;
// True after the user hits Cancel, so the post-eviction 404 keeps showing the
// canceling state instead of switching to the finalizing one.
let loadingCanceling = false;

export function newOperationId(kind = "op") {
  const rand =
    globalThis.crypto && "randomUUID" in globalThis.crypto
      ? globalThis.crypto.randomUUID()
      : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  return `${kind}:${rand}`;
}

function loadingParts() {
  const o = $("overlay");
  let box = document.getElementById("overlay-box");
  if (!box) {
    o.textContent = "";
    box = document.createElement("div");
    box.id = "overlay-box";
    box.className = "overlay-box";
    const textEl = document.createElement("div");
    textEl.id = "overlay-text";
    textEl.className = "overlay-text";
    const bar = document.createElement("progress");
    bar.id = "overlay-progress";
    bar.className = "overlay-progress";
    bar.max = 100;
    bar.value = 0;
    const detail = document.createElement("div");
    detail.id = "overlay-detail";
    detail.className = "overlay-detail";
    const cancel = document.createElement("button");
    cancel.id = "overlay-cancel";
    cancel.className = "cmd danger";
    cancel.type = "button";
    cancel.textContent = t("common.cancel");
    box.append(textEl, bar, detail, cancel);
    o.append(box);
  }
  return {
    text: $("overlay-text"),
    progress: $("overlay-progress") as HTMLProgressElement,
    detail: $("overlay-detail"),
    cancel: $("overlay-cancel") as HTMLButtonElement,
  };
}

function stopLoadingPoll() {
  if (loadingPoll != null) {
    clearInterval(loadingPoll);
    loadingPoll = null;
  }
  loadingOpId = null;
}

function updateOperationProgress(status: ArtifactOpStatus) {
  const parts = loadingParts();
  const percent = Math.max(0, Math.min(100, Math.round(Number(status.percent) || 0)));
  parts.progress.value = percent;
  parts.detail.textContent =
    status.total_lines > 0
      ? t("dialog.operation.progress", {
          done: commas(status.processed_lines),
          total: commas(status.total_lines),
          percent,
        })
      : "";
  if (status.canceled) parts.detail.textContent = t("dialog.operation.canceling");
}

// The worker has finished (so the server evicted its tracked op and the status
// poll now 404s), but our request is still in flight: the heavy phase is done
// and the server/client is finalizing — for an in-place sort that means
// re-indexing and reloading the rewritten file, slow on a huge file. Show a
// "finishing" state so the card doesn't sit frozen at its last percentage until
// the request finally returns and hides it.
function showFinalizing() {
  const parts = loadingParts();
  parts.text.textContent = t("dialog.operation.finalizing");
  parts.progress.value = 100;
  parts.detail.textContent = "";
  parts.cancel.classList.add("hidden");
}

async function pollLoadingOperation(opId: string) {
  try {
    const status = await api<ArtifactOpStatus>(`/api/ops/status?id=${encodeURIComponent(opId)}`);
    if (loadingOpId !== opId) return;
    loadingSeenOp = true;
    updateOperationProgress(status);
  } catch (e) {
    // Only a "not_found" after we've already seen the op means it finished and
    // was evicted; any other error is a transient blip and polling is only UI.
    if (
      loadingOpId === opId &&
      loadingSeenOp &&
      !loadingCanceling &&
      (e as { code?: string })?.code === "not_found"
    ) {
      showFinalizing();
    }
  }
}

export function showLoading(text, opts: { opId?: string | null; cancel?: boolean } = {}) {
  stopLoadingPoll();
  const o = $("overlay");
  const parts = loadingParts();
  parts.text.textContent = text || t("dialog.open.loading");
  parts.progress.classList.toggle("hidden", !opts.opId);
  parts.detail.classList.toggle("hidden", !opts.opId);
  parts.detail.textContent = opts.opId ? t("dialog.operation.starting") : "";
  parts.cancel.textContent = t("common.cancel");
  parts.cancel.classList.toggle("hidden", !opts.opId || !opts.cancel);
  parts.cancel.disabled = false;
  parts.cancel.onclick = () => {
    const id = loadingOpId;
    if (!id) return;
    loadingCanceling = true;
    parts.cancel.disabled = true;
    parts.detail.textContent = t("dialog.operation.canceling");
    apiPost<ArtifactOpStatus, OperationCancelRequest>("/api/ops/cancel", { id }).catch(() => {});
  };
  if (opts.opId) {
    loadingOpId = opts.opId;
    loadingSeenOp = false;
    loadingCanceling = false;
    pollLoadingOperation(opts.opId);
    loadingPoll = setInterval(() => pollLoadingOperation(opts.opId!), 500);
  }
  o.classList.remove("hidden");
}

export function hideLoading() {
  stopLoadingPoll();
  $("overlay").classList.add("hidden");
}

// True while the blocking loading overlay is up (open, sort, replace-all, …).
// Counted as a modal so edits and shortcuts can't race a long operation (#72).
export function loadingVisible() {
  return !$("overlay").classList.contains("hidden");
}
