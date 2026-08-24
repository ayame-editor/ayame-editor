// macOS native menu configuration. The web catalog/keymap is authoritative;
// Rust keeps its static table only as a pre-boot fallback.
import { isNativeApp, postNativeMessage, type NativeMenuItemConfig } from "./app.js";
import { t, type MessageKey } from "./i18n.js";
import { KEYMAP_ACTIONS } from "./state.js";

const NATIVE_MENU_EXTRA_LABEL_KEYS = {
  about: "help.about",
  encoding: "menu.encoding",
  help: "help.open",
  quit: "menu.quit",
  "section.edit": "menu.edit",
  "section.file": "menu.file",
  "section.help": "menu.help",
  "section.selection": "menu.selection",
  "section.tools": "menu.tools",
  "section.view": "menu.view",
  "section.window": "menu.window",
  toggleFollowTail: "menu.followTail",
  toggleWhitespace: "menu.showWhitespace",
  toggleWordWrap: "menu.wordWrap",
  toggleZenkakuUnderline: "menu.zenkakuUnderline",
  "window.minimize": "menu.minimize",
  "window.zoom": "menu.zoom",
} as const satisfies Record<string, MessageKey>;

const NATIVE_MENU_EXTRA_SHORTCUTS: Readonly<Record<string, string>> = {
  quit: "Ctrl+Q",
};

export function buildNativeMenuItems(
  shortcutFor: (action: string) => string,
): NativeMenuItemConfig[] {
  const actions = KEYMAP_ACTIONS.map(([id, labelKey]) => ({
    id,
    label: t(labelKey),
    shortcut: shortcutFor(id),
  }));
  const extras = Object.entries(NATIVE_MENU_EXTRA_LABEL_KEYS).map(([id, labelKey]) => ({
    id,
    label: t(labelKey),
    shortcut: NATIVE_MENU_EXTRA_SHORTCUTS[id] || "",
  }));
  return [...actions, ...extras];
}

export function syncNativeMenu(shortcutFor: (action: string) => string) {
  if (!isNativeApp()) return;
  postNativeMessage({ type: "menu_config", items: buildNativeMenuItems(shortcutFor) });
}
