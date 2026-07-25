// Compatibility facade for menu-related responsibilities.
export * from "./menu-constants.js";
export * from "./keymap-menu.js";
export * from "./menu-ui.js";
export * from "./palette.js";
export * from "./context-menu.js";
export * from "./menu-actions.js";
export * from "./menubar.js";
export { showPopupMenu } from "./popup-menu.js";
export { isBindableShortcut, normalizeShortcut, sanitizeKeymap } from "./keys.js";
export { APP_MENUS, fileMenuVisible, hideFileMenu } from "./menu-surface.js";
export { updateStatusMeta, updateStatusPos } from "./status.js";
