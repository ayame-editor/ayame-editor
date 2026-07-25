// Ayame Editor — ARIA menubar interaction.
import { $ } from "./dom.js";
import { anyModalOpen } from "./modal-state.js";
import { APP_MENUS, DROPDOWN_MENUS, fileMenuVisible, hideFileMenu } from "./menu-surface.js";
import { runMenuAction } from "./menu-actions.js";
import { showAppMenu } from "./menu-ui.js";

function menubarButtons(): HTMLElement[] {
  return APP_MENUS.map((id) => $(`${id}-menu-button`));
}

function menuItemsOf(id): HTMLElement[] {
  return [...$(`${id}-menu`).querySelectorAll<HTMLElement>(".menu-item")].filter(
    (element) => !(element as HTMLButtonElement).disabled,
  );
}

function setMenubarRoving(idx: number) {
  const buttons = menubarButtons();
  const i = ((idx % buttons.length) + buttons.length) % buttons.length;
  buttons.forEach((button, j) => button.setAttribute("tabindex", j === i ? "0" : "-1"));
  return buttons[i];
}

function openMenuWithFocus(id, edge: "first" | "last" = "first") {
  showAppMenu(id);
  const buttonIndex = APP_MENUS.indexOf(id);
  if (buttonIndex >= 0) setMenubarRoving(buttonIndex);
  const items = menuItemsOf(id);
  if (items.length) (edge === "last" ? items[items.length - 1] : items[0]).focus();
}

export function focusMenubar() {
  if (anyModalOpen()) return;
  const buttons = menubarButtons();
  const active = buttons.find((button) => button.getAttribute("tabindex") === "0") || buttons[0];
  active.focus();
}

function onMenubarButtonKey(event: KeyboardEvent, id) {
  const idx = APP_MENUS.indexOf(id);
  const last = APP_MENUS.length - 1;
  const moveTo = (nextIndex: number) => {
    const wasOpen = fileMenuVisible();
    setMenubarRoving(nextIndex).focus();
    if (wasOpen) {
      const wrapped = ((nextIndex % APP_MENUS.length) + APP_MENUS.length) % APP_MENUS.length;
      showAppMenu(APP_MENUS[wrapped]);
    }
  };
  switch (event.key) {
    case "ArrowRight":
      event.preventDefault();
      moveTo(idx + 1);
      break;
    case "ArrowLeft":
      event.preventDefault();
      moveTo(idx - 1);
      break;
    case "Home":
      event.preventDefault();
      moveTo(0);
      break;
    case "End":
      event.preventDefault();
      moveTo(last);
      break;
    case "ArrowDown":
      event.preventDefault();
      openMenuWithFocus(id, "first");
      break;
    case "ArrowUp":
      event.preventDefault();
      openMenuWithFocus(id, "last");
      break;
    case "Escape":
      if (fileMenuVisible()) {
        event.preventDefault();
        hideFileMenu();
      }
      break;
  }
}

function onMenuKey(event: KeyboardEvent, id) {
  const items = menuItemsOf(id);
  const menuIndex = APP_MENUS.indexOf(id);
  const position = items.indexOf(document.activeElement as HTMLElement);
  switch (event.key) {
    case "ArrowDown":
      event.preventDefault();
      if (items.length) items[position < 0 ? 0 : (position + 1) % items.length].focus();
      break;
    case "ArrowUp":
      event.preventDefault();
      if (items.length) {
        items[
          position < 0 ? items.length - 1 : (position - 1 + items.length) % items.length
        ].focus();
      }
      break;
    case "Home":
      event.preventDefault();
      items[0]?.focus();
      break;
    case "End":
      event.preventDefault();
      items[items.length - 1]?.focus();
      break;
    case "ArrowRight":
      if (menuIndex >= 0) {
        event.preventDefault();
        openMenuWithFocus(APP_MENUS[(menuIndex + 1) % APP_MENUS.length], "first");
      }
      break;
    case "ArrowLeft":
      if (menuIndex >= 0) {
        event.preventDefault();
        openMenuWithFocus(
          APP_MENUS[(menuIndex - 1 + APP_MENUS.length) % APP_MENUS.length],
          "first",
        );
      }
      break;
    case "Escape":
      event.preventDefault();
      hideFileMenu();
      $(`${id}-menu-button`).focus();
      break;
    case "Tab":
      hideFileMenu();
      break;
  }
}

export function initMenuBar() {
  for (const id of DROPDOWN_MENUS) {
    const button = $(`${id}-menu-button`);
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      const open = !$(`${id}-menu`).classList.contains("hidden");
      if (open) {
        hideFileMenu();
      } else if ((event as MouseEvent).detail === 0) {
        openMenuWithFocus(id, "first");
      } else {
        showAppMenu(id);
      }
    });
    button.addEventListener("keydown", (event) => onMenubarButtonKey(event, id));
    $(`${id}-menu`).addEventListener("keydown", (event) => onMenuKey(event, id));
    menuItemsOf(id).forEach((item) => item.setAttribute("tabindex", "-1"));
    if (APP_MENUS.includes(id)) {
      button.addEventListener("pointerenter", () => {
        if (fileMenuVisible()) showAppMenu(id);
      });
    }
  }
  const buttons = menubarButtons();
  buttons.forEach((button, index) => button.setAttribute("tabindex", index === 0 ? "0" : "-1"));
  $("menubar").addEventListener("focusin", (event) => {
    const index = buttons.indexOf(event.target as HTMLElement);
    if (index >= 0) {
      buttons.forEach((button, j) => button.setAttribute("tabindex", j === index ? "0" : "-1"));
    }
  });
  document.addEventListener("keydown", (event) => {
    if (
      event.key === "F10" &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey &&
      !event.shiftKey &&
      !anyModalOpen()
    ) {
      event.preventDefault();
      focusMenubar();
    }
  });
  document.querySelectorAll("[data-menu-action]").forEach((item) => {
    item.addEventListener("click", () => runMenuAction((item as any).dataset.menuAction));
  });
}
