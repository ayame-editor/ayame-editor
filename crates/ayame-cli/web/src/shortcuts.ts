import { KEYMAP_ACTIONS } from "./state.js";

export function normalizeShortcut(raw) {
  if (!raw) return "";
  const parts = String(raw)
    .split("+")
    .map((p) => p.trim())
    .filter(Boolean);
  const mods = { Ctrl: false, Shift: false, Alt: false };
  let key = "";
  const namedKeys = {
    arrowup: "ArrowUp",
    arrowdown: "ArrowDown",
    arrowleft: "ArrowLeft",
    arrowright: "ArrowRight",
    pageup: "PageUp",
    pagedown: "PageDown",
    home: "Home",
    end: "End",
    insert: "Insert",
    delete: "Delete",
    backspace: "Backspace",
    enter: "Enter",
    escape: "Escape",
    esc: "Escape",
    tab: "Tab",
    space: "Space",
  };
  for (const part of parts) {
    const low = part.toLowerCase();
    if (low === "ctrl" || low === "control" || low === "cmd" || low === "command" || low === "meta")
      mods.Ctrl = true;
    else if (low === "shift") mods.Shift = true;
    else if (low === "alt" || low === "option") mods.Alt = true;
    else if (namedKeys[low]) key = namedKeys[low];
    else if (/^f\d+$/i.test(part)) key = part.toUpperCase();
    else key = part.length === 1 ? part.toUpperCase() : part[0].toUpperCase() + part.slice(1);
  }
  if (!key || ["Ctrl", "Shift", "Alt"].includes(key)) return "";
  return [mods.Ctrl && "Ctrl", mods.Shift && "Shift", mods.Alt && "Alt", key]
    .filter(Boolean)
    .join("+");
}

export function isBindableShortcut(shortcut) {
  if (!shortcut) return true;
  const parts = shortcut.split("+");
  const key = parts[parts.length - 1];
  return parts.includes("Ctrl") || parts.includes("Alt") || /^F\d+$/i.test(key);
}

export function sanitizeKeymap(raw) {
  const src = raw && typeof raw === "object" ? raw : {};
  const clean = {};
  for (const [action] of KEYMAP_ACTIONS) {
    if (!Object.prototype.hasOwnProperty.call(src, action)) continue;
    if (Array.isArray(src[action])) {
      clean[action] = src[action].map(normalizeShortcut).filter((v) => v && isBindableShortcut(v));
    } else {
      const v = normalizeShortcut(src[action]);
      clean[action] = isBindableShortcut(v) ? v : "";
    }
  }
  return clean;
}
