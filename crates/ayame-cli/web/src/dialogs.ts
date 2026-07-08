// Ayame Editor — dialogs module. Type-stripped to JS at build time (build.rs, oxc).
import { $, commas, setModalOpen } from "./dom.js";
import { t } from "./i18n.js";
import { focusEditor } from "./editor.js";

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
// title, options, browse}. All labels/placeholders/titles arrive already
// localized. A text field may carry `browse: () => Promise<string|null>` to
// render a "参照…" button that fills the field from a picker (#79).
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
      // Optional "参照…" button: `f.browse` is a picker the caller supplies
      // (file or folder), so dialogs.ts stays free of a workspace import (#79).
      if (typeof f.browse === "function") {
        const wrap = document.createElement("span");
        wrap.className = "form-input-browse";
        const btn = document.createElement("button");
        btn.type = "button";
        btn.className = "cmd form-browse-btn";
        btn.textContent = t("dialog.pick.browse");
        btn.addEventListener("click", async () => {
          const picked = await f.browse();
          if (picked != null && String(picked) !== "") {
            input.value = picked;
            input.focus();
          }
        });
        wrap.append(input, btn);
        row.append(wrap);
      } else {
        row.append(input);
      }
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
export function showLoading(text) {
  const o = $("overlay");
  o.textContent = text || t("dialog.open.loading");
  o.classList.remove("hidden");
}

export function hideLoading() {
  const o = $("overlay");
  o.classList.add("hidden");
  o.textContent = "";
}

// A determinate progress card for long worker ops (sort/grep/split) — #78.
// Shows a bar, a "done / total lines" readout, and (when `onCancel` is given) a
// Cancel button. Returns a handle to feed progress and to tear it down. The
// overlay counts as a modal (see `loadingVisible`), so edits stay blocked.
export type ProgressHandle = {
  setProgress: (done: number, total: number) => void;
  close: () => void;
};

export function showProgress(label: string, onCancel?: () => void): ProgressHandle {
  const o = $("overlay");
  o.textContent = "";
  const card = document.createElement("div");
  card.className = "progress-card";

  const labelEl = document.createElement("div");
  labelEl.className = "progress-label";
  labelEl.textContent = label;

  const track = document.createElement("div");
  track.className = "progress-track indeterminate";
  const fill = document.createElement("div");
  fill.className = "progress-fill";
  track.appendChild(fill);

  const meta = document.createElement("div");
  meta.className = "progress-meta";
  const count = document.createElement("span");
  const pct = document.createElement("span");
  meta.append(count, pct);

  card.append(labelEl, track, meta);

  if (onCancel) {
    const actions = document.createElement("div");
    actions.className = "progress-actions";
    const btn = document.createElement("button");
    btn.className = "cmd";
    btn.textContent = t("dialog.progress.cancel");
    btn.addEventListener("click", () => {
      btn.disabled = true;
      labelEl.textContent = t("dialog.progress.cancelling");
      onCancel();
    });
    actions.appendChild(btn);
    card.appendChild(actions);
  }

  o.appendChild(card);
  o.classList.remove("hidden");

  return {
    setProgress(done, total) {
      if (total > 0) {
        track.classList.remove("indeterminate");
        const ratio = Math.max(0, Math.min(1, done / total));
        fill.style.width = `${(ratio * 100).toFixed(1)}%`;
        count.textContent = t("dialog.progress.lines", {
          done: commas(done),
          total: commas(total),
        });
        pct.textContent = `${(ratio * 100).toFixed(0)}%`;
      }
    },
    close() {
      hideLoading();
    },
  };
}

// True while the blocking loading overlay is up (open, sort, replace-all, …).
// Counted as a modal so edits and shortcuts can't race a long operation (#72).
export function loadingVisible() {
  return !$("overlay").classList.contains("hidden");
}
