import { KEYMAP_ACTIONS } from "./state.js";

// Shortcut persistence uses one platform-neutral spelling. Parsing,
// KeyboardEvent conversion, matching, and platform display all live here so
// their modifier/key rules cannot drift apart.
/// Split a shortcut spelling into modifier/key tokens.
///
/// "+" is both the separator and a key people bind (zoom in), so a plain
/// `split("+")` turns `Ctrl++` into two empty segments and loses the key
/// entirely. A "+" that follows another "+" — i.e. one with no token in front
/// of it — is the key rather than a separator.
function shortcutTokens(raw: string) {
  const tokens: string[] = [];
  let token = "";
  for (const ch of raw) {
    if (ch === "+" && token !== "") {
      tokens.push(token);
      token = "";
    } else {
      token += ch;
    }
  }
  if (token) tokens.push(token);
  return tokens.map((p) => p.trim()).filter(Boolean);
}

export function normalizeShortcut(raw) {
  if (!raw) return "";
  const parts = shortcutTokens(String(raw));
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

export function eventShortcut(e) {
  if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return "";
  const key = e.key === " " ? "Space" : e.key;
  return normalizeShortcut(
    [(e.ctrlKey || e.metaKey) && "Ctrl", e.shiftKey && "Shift", e.altKey && "Alt", key]
      .filter(Boolean)
      .join("+"),
  );
}

/// Every spelling an event can satisfy, most specific first.
///
/// Which physical key produces `+`, `=`, `-` or `_` depends on the layout: on
/// a US keyboard "zoom in" is Ctrl+Shift+= and on a Japanese one Ctrl+Shift+;,
/// both reported as the key `+`. Requiring the recorded Shift would make a
/// punctuation binding work only on the keyboard it was recorded on, so an
/// event whose key is a single non-alphanumeric character also answers to the
/// same chord without Shift (#175).
export function eventShortcuts(e) {
  const shortcut = eventShortcut(e);
  if (!shortcut) return [];
  if (!e.shiftKey) return [shortcut];
  const key = e.key;
  if (typeof key !== "string" || Array.from(key).length !== 1 || /[\p{L}\p{N}]/u.test(key)) {
    return [shortcut];
  }
  const unshifted = shortcut.replace("Shift+", "");
  return unshifted === shortcut ? [shortcut] : [shortcut, unshifted];
}

export function matchesShortcut(e, shortcuts) {
  return eventShortcuts(e).some((shortcut) => shortcuts.includes(shortcut));
}

// Keep the storage spelling for matching, but render native macOS modifier
// glyphs so menus, the command palette, and keymap inputs describe real keys.
export function displayShortcut(
  shortcut,
  platform = typeof navigator === "undefined" ? "" : navigator.platform,
) {
  const rendered = String(shortcut || "")
    .replace(/ArrowUp/g, "↑")
    .replace(/ArrowDown/g, "↓")
    .replace(/ArrowLeft/g, "←")
    .replace(/ArrowRight/g, "→");
  if (!/^(?:Mac|iPhone|iPad|iPod)/i.test(String(platform))) return rendered;
  return rendered
    .replace(/Ctrl\+/g, "⌘")
    .replace(/Alt\+/g, "⌥")
    .replace(/Shift\+/g, "⇧");
}

export function isBindableShortcut(shortcut) {
  if (!shortcut) return true;
  const parts = shortcutTokens(String(shortcut));
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
