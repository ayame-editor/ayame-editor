// Ayame Editor — dom module. Type-stripped to JS at build time (build.rs, oxc).
import { currentLocale } from "./i18n.js";

// Transitional default for unported call sites. New control lookups specify
// their real DOM type (`$<HTMLInputElement>(...)`); shrink this surface as each
// feature is touched until `$` can default to plain HTMLElement.
type LegacyControlElement = HTMLElement & {
  checked: boolean;
  disabled: boolean;
  files: FileList | null;
  placeholder: string;
  selected: boolean;
  type: string;
  value: string;
  select(): void;
};

export function $<T extends HTMLElement = LegacyControlElement>(id: string): T {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing element #${id}`);
  return el as T;
}

export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className = "",
  text?: string,
): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  if (className) element.className = className;
  if (text != null) element.textContent = text;
  return element;
}

export function button(className: string, label: string, onClick?: () => void) {
  const element = el("button", className, label);
  element.type = "button";
  if (onClick) element.addEventListener("click", onClick);
  return element;
}

export function commas(n) {
  return n.toLocaleString(currentLocale());
}

// `locale` picks the decimal separator for the two-fraction-digit sizes; pass
// currentLocale() so the opener/status size follows the selected language.
export function humanBytes(n, locale = "en-US") {
  const u = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
  let v = n,
    i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  if (i === 0) return `${n} ${u[0]}`;
  const num = v.toLocaleString(locale, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  return `${num} ${u[i]}`;
}

export function escapeRegExp(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// Windows extended-length paths come back from canonicalize with a "\\?\"
// prefix; never show that to the user ("保存しました: \\?\C:\…" reads broken).
export function displayPath(path) {
  const s = String(path || "");
  if (s.startsWith("\\\\?\\UNC\\")) return "\\\\" + s.slice(8);
  if (s.startsWith("\\\\?\\")) return s.slice(4);
  return s;
}

// The stack of currently-open modal dialogs. The last entry is the top-most,
// active dialog; everything behind it (the app chrome and any lower modals) is
// made inert so Tab and the screen-reader cursor can't escape behind it (#160).
const modalStack: HTMLElement[] = [];

type ModalRegistration = {
  closeOnBackdrop?: boolean;
  onClose?: () => void;
};

const modalRegistry = new Map<string, ModalRegistration>();
const backdropBound = new WeakSet<HTMLElement>();

function bindRegisteredModal(id: string) {
  const modal = document.getElementById(id);
  if (!modal) return;
  modal.setAttribute("aria-hidden", modal.classList.contains("hidden") ? "true" : "false");
  if (backdropBound.has(modal)) return;
  modal.addEventListener("mousedown", (event) => {
    const registration = modalRegistry.get(id);
    if (event.target === modal && registration?.closeOnBackdrop) registration.onClose?.();
  });
  backdropBound.add(modal);
}

// One registration owns a modal's close behavior. Visibility, Escape order,
// backdrop dismissal, and aria-hidden are derived from this registry and the
// same LIFO stack maintained by setModalOpen.
export function registerModal(id: string, registration: ModalRegistration = {}) {
  modalRegistry.set(id, registration);
  bindRegisteredModal(id);
  return () => {
    if (modalRegistry.get(id) === registration) modalRegistry.delete(id);
  };
}

export function initModalRegistry() {
  for (const id of modalRegistry.keys()) bindRegisteredModal(id);
}

// Drop any modals that left the DOM without a proper close, so a stray removal
// can never strand the backdrop in the inert state.
function pruneStack() {
  for (let i = modalStack.length - 1; i >= 0; i--) {
    if (!modalStack[i].isConnected) modalStack.splice(i, 1);
  }
}

export function anyModalOpen() {
  pruneStack();
  if (modalStack.length > 0) return true;
  for (const id of modalRegistry.keys()) {
    if (modalVisible(id)) return true;
  }
  return false;
}

export function closeTopModal() {
  pruneStack();
  // The operation overlay lives inside #app instead of in modalStack, but is
  // visually above every dialog and therefore owns Escape while visible.
  const overlay = modalRegistry.has("overlay") && modalVisible("overlay") ? "overlay" : null;
  const stacked = modalStack[modalStack.length - 1];
  const fallback = [...modalRegistry.keys()].reverse().find((id) => modalVisible(id));
  const id = overlay || stacked?.id || fallback;
  if (!id) return false;
  const onClose = modalRegistry.get(id)?.onClose;
  if (!onClose) return false;
  onClose();
  return true;
}

// Re-derive inert / aria-hidden for the whole document from the modal stack:
// #app plus every top-level modal is considered (the loading overlay lives
// inside #app, so it is trapped separately — see activeTrapRoot). Only the
// top-most open dialog stays interactive and exposed to assistive tech.
function refreshInert() {
  pruneStack();
  const top = modalStack[modalStack.length - 1] || null;
  const app = document.getElementById("app");
  if (app) {
    app.toggleAttribute("inert", !!top);
    if (top) app.setAttribute("aria-hidden", "true");
    else app.removeAttribute("aria-hidden");
  }
  for (const m of document.querySelectorAll<HTMLElement>(".modal")) {
    const isTop = m === top;
    m.toggleAttribute("inert", !!top && !isTop);
    // Leave closed modals as setModalOpen left them; only re-assert exposure
    // while a stack exists so a buried-but-open dialog is hidden from SR.
    if (top) m.setAttribute("aria-hidden", isTop ? "false" : "true");
  }
}

// Show/hide one modal element, keeping the .hidden class and aria-hidden in
// step (every modal in the app pairs the two), and maintaining the inert
// backdrop + focus-trap bookkeeping so the dialog truly owns the keyboard.
export function setModalOpen(modal, open) {
  modal.classList.toggle("hidden", !open);
  modal.setAttribute("aria-hidden", open ? "false" : "true");
  // The loading overlay is a registered dialog inside #app, not a top-level
  // .modal. It shares visibility/ARIA handling without making its own parent
  // inert or participating in the top-level modal stack.
  if (!modal.classList.contains("modal")) return;
  const i = modalStack.indexOf(modal);
  if (open) {
    if (i === -1) modalStack.push(modal);
  } else if (i !== -1) {
    modalStack.splice(i, 1);
  }
  refreshInert();
}

export function modalVisible(id: string) {
  const modal = document.getElementById(id);
  return !!modal && !modal.classList.contains("hidden");
}

// Tab-focusable descendants, in DOM order, skipping disabled controls and
// anything inside a hidden/inert subtree (jsdom has no layout, so we test the
// class/attribute rather than offsetParent).
const FOCUSABLE = 'a[href], button, input, select, textarea, [tabindex]:not([tabindex="-1"])';
function focusables(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => {
    if ((el as LegacyControlElement).disabled) return false;
    if (el.getAttribute("tabindex") === "-1") return false;
    return !el.closest(".hidden") && !el.closest("[inert]");
  });
}

// The container that should trap focus right now: the top-most modal, or the
// loading overlay when it is the only thing up (it lives inside #app, so it is
// never on the modal stack). Exposed for the overlay's own focus handling.
export function activeTrapRoot(): HTMLElement | null {
  pruneStack();
  if (modalStack.length) return modalStack[modalStack.length - 1];
  const overlay = document.getElementById("overlay");
  if (overlay && !overlay.classList.contains("hidden")) return overlay;
  return null;
}

// Install the single, app-wide focus trap. While a dialog (or the overlay) is
// up, Tab / Shift+Tab cycle within it instead of reaching the inert backdrop.
export function initModalFocusTrap() {
  document.addEventListener(
    "keydown",
    (e) => {
      if (e.key !== "Tab") return;
      const root = activeTrapRoot();
      if (!root) return;
      const items = focusables(root);
      if (items.length === 0) {
        e.preventDefault(); // nothing to focus — keep it off the backdrop
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      const active = document.activeElement as HTMLElement | null;
      if (!active || !root.contains(active)) {
        e.preventDefault();
        (e.shiftKey ? last : first).focus();
      } else if (e.shiftKey && active === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && active === last) {
        e.preventDefault();
        first.focus();
      }
    },
    true,
  );
}

// Build an <svg><use href="#id"></use></svg> node for a sprite symbol from
// index.html. Purely decorative: callers keep the accessible name on the
// element (aria-label / visible text) — the icon itself is aria-hidden.
export function iconSvg(id, cls = "ay-icon") {
  const NS = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(NS, "svg");
  svg.setAttribute("class", cls);
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("focusable", "false");
  const use = document.createElementNS(NS, "use");
  use.setAttribute("href", `#${id}`);
  svg.append(use);
  return svg;
}

// The server parks untitled buffers in a private scratch directory named
// "ayame-srv-untitled-…" (older sessions used "ayame-untitled-<pid>"); the
// marker in the path is the client's only way to tell scratch from real files.
export function isUntitled(path) {
  return !!path && /ayame-(?:srv-)?untitled-/.test(String(path));
}

export function untitledName(path) {
  const base = pathBaseName(path);
  return base && base !== "untitled.txt" ? base : "untitled";
}

// Show a short, friendly name in the toolbar (basename, or "untitled").
export function displayName(path) {
  if (!path) return "—";
  if (isUntitled(path)) return untitledName(path);
  const parts = path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || path;
}

export function pathBaseName(path) {
  if (!path) return "";
  const clean = String(path).replace(/^\\\\\?\\/, "");
  const parts = clean.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || clean;
}

export function pathDirName(path) {
  if (!path) return null;
  const clean = String(path).replace(/^\\\\\?\\/, "");
  const i = Math.max(clean.lastIndexOf("/"), clean.lastIndexOf("\\"));
  if (i < 0) return null;
  if (i === 0) return clean.slice(0, 1);
  return clean.slice(0, i);
}

export function isAbsolutePath(path) {
  return /^(?:[A-Za-z]:[\\/]|\/|\\\\)/.test(String(path || ""));
}

export function joinPath(dir, name) {
  const n = String(name || "").trim();
  if (!n) return "";
  if (isAbsolutePath(n)) return n;
  const d = String(dir || "").replace(/[\\/]+$/, "");
  if (!d) return n;
  const sep = d.includes("\\") && !d.includes("/") ? "\\" : "/";
  return `${d}${sep}${n}`;
}

export function pathCrumbs(path) {
  const clean = String(path || "").replace(/^\\\\\?\\/, "");
  if (!clean) return [];
  const winDrive = clean.match(/^([A-Za-z]:)[\\/](.*)$/);
  if (winDrive) {
    const sep = "\\";
    let acc = `${winDrive[1]}${sep}`;
    const out = [{ label: winDrive[1], path: acc }];
    for (const part of winDrive[2].split(/[\\/]+/).filter(Boolean)) {
      acc = acc.endsWith(sep) ? `${acc}${part}` : `${acc}${sep}${part}`;
      out.push({ label: part, path: acc });
    }
    return out;
  }
  if (clean.startsWith("\\\\")) {
    const parts = clean.split(/[\\/]+/).filter(Boolean);
    if (parts.length < 2) return [{ label: clean, path: clean }];
    let acc = `\\\\${parts[0]}\\${parts[1]}`;
    const out = [{ label: `\\\\${parts[0]}\\${parts[1]}`, path: acc }];
    for (const part of parts.slice(2)) {
      acc = `${acc}\\${part}`;
      out.push({ label: part, path: acc });
    }
    return out;
  }
  if (clean.startsWith("/")) {
    let acc = "";
    const out = [{ label: "/", path: "/" }];
    for (const part of clean.split("/").filter(Boolean)) {
      acc += `/${part}`;
      out.push({ label: part, path: acc });
    }
    return out;
  }
  let acc = "";
  return clean
    .split(/[\\/]+/)
    .filter(Boolean)
    .map((part) => {
      acc = acc ? `${acc}/${part}` : part;
      return { label: part, path: acc };
    });
}
