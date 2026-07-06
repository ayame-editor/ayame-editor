// Ayame Editor — dialogs module. Type-stripped to JS at build time (build.rs, oxc).
import { $, setModalOpen } from "./dom.js";
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
export function showLoading(text) {
  const o = $("overlay");
  o.textContent = text || t("dialog.open.loading");
  o.classList.remove("hidden");
}

export function hideLoading() {
  $("overlay").classList.add("hidden");
}
