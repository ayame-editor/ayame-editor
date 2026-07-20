// Lightweight reusable context menu. Kept outside menus.ts so feature modules
// can offer contextual actions without forming a feature <-> command-registry
// import cycle.

export interface PopupMenuItem {
  label?: string;
  action?: () => void;
  disabled?: boolean;
  checked?: boolean;
  separator?: boolean;
}

export function showPopupMenu(x: number, y: number, items: PopupMenuItem[]) {
  document.getElementById("popup-menu")?.remove();
  const menu = document.createElement("div");
  menu.id = "popup-menu";
  menu.className = "file-menu ctx-menu";
  menu.setAttribute("role", "menu");
  const close = () => {
    menu.remove();
    document.removeEventListener("pointerdown", onDown, true);
    document.removeEventListener("keydown", onKey, true);
  };
  const onDown = (event: Event) => {
    if (!(event.target as HTMLElement | null)?.closest?.("#popup-menu")) close();
  };
  const onKey = (event: KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      close();
    }
  };
  for (const item of items) {
    if (item.separator) {
      const separator = document.createElement("div");
      separator.className = "menu-sep";
      menu.append(separator);
      continue;
    }
    const button = document.createElement("button");
    button.type = "button";
    button.className = `menu-item${item.checked ? " checked" : ""}`;
    button.setAttribute("role", "menuitem");
    button.disabled = !!item.disabled;
    const label = document.createElement("span");
    label.className = "menu-label";
    label.textContent = item.label || "";
    button.append(label);
    button.addEventListener("click", () => {
      close();
      item.action?.();
    });
    menu.append(button);
  }
  document.body.append(menu);
  const width = menu.offsetWidth;
  const height = menu.offsetHeight;
  menu.style.left = `${Math.max(4, Math.min(x, window.innerWidth - width - 8))}px`;
  menu.style.top = `${Math.max(4, Math.min(y, window.innerHeight - height - 8))}px`;
  // The opening pointer event is still bubbling; arm outside-dismiss after it.
  setTimeout(() => {
    document.addEventListener("pointerdown", onDown, true);
    document.addEventListener("keydown", onKey, true);
  }, 0);
}
