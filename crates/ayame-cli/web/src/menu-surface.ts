// Ayame Editor — DOM-only state for the application dropdown menus.
import { $ } from "./dom.js";

export const APP_MENUS = ["file", "edit", "selection", "view", "help"];
export const DROPDOWN_MENUS = [...APP_MENUS, "tools"];

export function fileMenuVisible() {
  return DROPDOWN_MENUS.some((id) => !$(`${id}-menu`).classList.contains("hidden"));
}

export function hideFileMenu(focusButton = false) {
  let focused = false;
  for (const id of DROPDOWN_MENUS) {
    const menu = $(`${id}-menu`);
    const button = $(`${id}-menu-button`);
    const wasOpen = !menu.classList.contains("hidden");
    menu.classList.add("hidden");
    button.classList.remove("on");
    button.setAttribute("aria-expanded", "false");
    if (focusButton && wasOpen && !focused) {
      button.focus();
      focused = true;
    }
  }
}
