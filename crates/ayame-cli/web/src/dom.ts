// Ayame Editor — dom module. Type-stripped to JS at build time (build.rs, oxc).
type AyameElement = HTMLElement & {
  checked: boolean;
  disabled: boolean;
  files: FileList | null;
  placeholder: string;
  selected: boolean;
  type: string;
  value: string;
  select(): void;
};

export function $<T extends HTMLElement = AyameElement>(id: string): T {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing element #${id}`);
  return el as T;
}

export function commas(n) {
  return Number(n).toLocaleString(numberLocale());
}

export function humanBytes(n) {
  const u = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
  let v = n,
    i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return i === 0
    ? `${Number(n).toLocaleString(numberLocale())} B`
    : `${new Intl.NumberFormat(numberLocale(), {
        minimumFractionDigits: 2,
        maximumFractionDigits: 2,
      }).format(v)} ${u[i]}`;
}

function numberLocale() {
  return document.documentElement.lang || navigator.language || "en";
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

// Shortcuts are stored with KeyboardEvent key names ("Ctrl+Alt+ArrowUp");
// menus and hints render the arrows as glyphs so labels stay compact.
export function displayShortcut(shortcut, mac = isMacPlatform()) {
  const display = String(shortcut || "")
    .replace(/ArrowUp/g, "↑")
    .replace(/ArrowDown/g, "↓")
    .replace(/ArrowLeft/g, "←")
    .replace(/ArrowRight/g, "→");
  if (!mac) return display;
  const parts = display.split("+");
  const modifiers = parts
    .slice(0, -1)
    .map((part) => ({ Ctrl: "⌘", Shift: "⇧", Alt: "⌥", Meta: "⌘" })[part] || part)
    .join("");
  return modifiers + (parts.at(-1) || "");
}

function isMacPlatform() {
  return /Mac|iPhone|iPad|iPod/i.test(navigator.platform || navigator.userAgent || "");
}

const FOCUSABLE = [
  "button:not([disabled])",
  "a[href]",
  "input:not([disabled]):not([type='hidden'])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "[contenteditable='true']",
  "[tabindex]:not([tabindex='-1'])",
].join(",");

let modalStack: HTMLElement[] = [];
const modalReturnFocus = new WeakMap<HTMLElement, HTMLElement>();
let modalTrapInstalled = false;

function liveModalStack() {
  modalStack = modalStack.filter(
    (modal) => modal.isConnected && !modal.classList.contains("hidden"),
  );
  return modalStack;
}

function modalFocusables(modal: HTMLElement) {
  return [...modal.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
    (el) => !el.hidden && !el.closest("[inert]") && el.getAttribute("aria-hidden") !== "true",
  );
}

function syncModalIsolation() {
  const stack = liveModalStack();
  const top = stack.at(-1);
  const app = document.getElementById("app");
  if (app) {
    app.inert = !!top;
    if (top) app.setAttribute("aria-hidden", "true");
    else app.removeAttribute("aria-hidden");
  }
  for (const modal of document.querySelectorAll<HTMLElement>(".modal")) {
    const active = modal === top;
    modal.inert = !active && !modal.classList.contains("hidden");
    modal.setAttribute("aria-hidden", active ? "false" : "true");
  }
}

function trapModalFocus(e: KeyboardEvent) {
  if (e.key !== "Tab") return;
  const top = liveModalStack().at(-1);
  if (!top) return;
  const focusables = modalFocusables(top);
  if (!focusables.length) {
    e.preventDefault();
    top.tabIndex = -1;
    top.focus();
    return;
  }
  const first = focusables[0];
  const last = focusables.at(-1)!;
  const active = document.activeElement;
  if (
    !top.contains(active) ||
    (e.shiftKey && active === first) ||
    (!e.shiftKey && active === last)
  ) {
    e.preventDefault();
    (e.shiftKey ? last : first).focus();
  }
}

function ensureModalTrap() {
  if (modalTrapInstalled) return;
  document.addEventListener("keydown", trapModalFocus, true);
  modalTrapInstalled = true;
}

// Show/hide one modal while isolating the page behind it, trapping Tab inside
// the topmost dialog, and restoring the previous focus on close.
export function setModalOpen(modal, open) {
  ensureModalTrap();
  liveModalStack();
  const wasOpen = !modal.classList.contains("hidden");
  if (open && !wasOpen) {
    const active = document.activeElement;
    if (active instanceof HTMLElement) modalReturnFocus.set(modal, active);
    modalStack.push(modal);
  } else if (!open) {
    modalStack = modalStack.filter((entry) => entry !== modal);
  }
  modal.classList.toggle("hidden", !open);
  modal.setAttribute("aria-hidden", open ? "false" : "true");
  syncModalIsolation();

  if (open && !wasOpen) {
    queueMicrotask(() => {
      if (liveModalStack().at(-1) !== modal || modal.contains(document.activeElement)) return;
      const target =
        (modal.querySelector("[autofocus]") as HTMLElement | null) || modalFocusables(modal)[0];
      if (target) target.focus();
      else {
        modal.tabIndex = -1;
        modal.focus();
      }
    });
  } else if (!open && wasOpen) {
    const target = modalReturnFocus.get(modal);
    if (target?.isConnected && !target.closest("[inert]")) target.focus();
    else {
      const top = liveModalStack().at(-1);
      if (top) modalFocusables(top)[0]?.focus();
    }
  }
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
