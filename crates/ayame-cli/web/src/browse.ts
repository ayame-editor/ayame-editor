// Ayame Editor — server-side file browser and opener list behavior.
import { $, button, el, humanBytes, iconSvg, pathCrumbs } from "./dom.js";
import { BROWSE_KEY, state } from "./state.js";
import { currentLocale, serverMessage, t } from "./i18n.js";
import { api } from "./api.js";
import type { BrowseResponse } from "./types/api.js";
import { currentOpenerMode } from "./opener-state.js";

let commitPickedFile = () => {};
let openPickedPath = (_path) => {};
let openerActiveIndex = -1;
let openerOptionSeq = 0;

export function setBrowseService(service) {
  commitPickedFile = service.commitOpener;
  openPickedPath = service.openPath;
}

export function openerOptions(): HTMLElement[] {
  return [
    ...document.querySelectorAll<HTMLElement>(
      "#opener-recent .opener-row, #opener-list .opener-row",
    ),
  ];
}

export function resetOpenerSelection() {
  openerActiveIndex = -1;
  for (const id of ["opener-recent", "opener-list"]) {
    document.getElementById(id)?.removeAttribute("aria-activedescendant");
  }
  for (const row of openerOptions()) {
    row.classList.remove("active");
    row.setAttribute("aria-selected", "false");
  }
}

export function setOpenerActiveIndex(index, focusList = false) {
  const options = openerOptions();
  if (!options.length) {
    resetOpenerSelection();
    return null;
  }
  openerActiveIndex = (index + options.length) % options.length;
  const active = options[openerActiveIndex];
  for (const row of options) {
    const selected = row === active;
    row.classList.toggle("active", selected);
    row.setAttribute("aria-selected", String(selected));
  }
  for (const id of ["opener-recent", "opener-list"]) {
    document.getElementById(id)?.removeAttribute("aria-activedescendant");
  }
  const owner = active.closest<HTMLElement>('[role="listbox"]');
  owner?.setAttribute("aria-activedescendant", active.id);
  if (focusList) owner?.focus();
  active.scrollIntoView?.({ block: "nearest" });
  return active;
}

export function moveOpenerSelection(delta, focusList = true) {
  const options = openerOptions();
  if (!options.length) return null;
  const next =
    openerActiveIndex < 0 ? (delta < 0 ? options.length - 1 : 0) : openerActiveIndex + delta;
  return setOpenerActiveIndex(next, focusList);
}

export function prepareOpenerOption(row: HTMLElement) {
  row.id = `opener-option-${++openerOptionSeq}`;
  row.tabIndex = -1;
  row.setAttribute("role", "option");
  row.setAttribute("aria-selected", "false");
  row.addEventListener("mouseenter", () => {
    const index = openerOptions().indexOf(row);
    if (index >= 0) setOpenerActiveIndex(index);
  });
}

export function onOpenerListFocus(event) {
  const owner = event.currentTarget as HTMLElement;
  const options = openerOptions();
  const active = options[openerActiveIndex];
  if (active && owner.contains(active)) return;
  const first = options.findIndex((row) => owner.contains(row));
  if (first >= 0) setOpenerActiveIndex(first);
}

export function handleOpenerListKeydown(event, hideOpener) {
  if (event.key === "Escape") {
    event.preventDefault();
    hideOpener();
    return;
  }
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    moveOpenerSelection(event.key === "ArrowDown" ? 1 : -1);
    return;
  }
  if (event.key === "Home" || event.key === "End") {
    event.preventDefault();
    const options = openerOptions();
    if (options.length) {
      setOpenerActiveIndex(event.key === "Home" ? 0 : options.length - 1, true);
    }
    return;
  }
  if (event.key === "Enter" || event.key === " ") {
    const active = openerOptions()[openerActiveIndex];
    if (active) {
      event.preventDefault();
      active.click();
    }
  }
}

export function openerMsg(text, busy = false) {
  const element = $("opener-msg");
  element.textContent = text || "";
  element.classList.toggle("busy", !!text && busy);
}

export async function browse(dir) {
  openerMsg(t("dialog.open.loading"), true);
  try {
    const query = dir == null ? "" : `?dir=${encodeURIComponent(dir)}`;
    const response = await api<BrowseResponse>(`/api/browse${query}`);
    renderBrowse(response);
    openerMsg("");
  } catch (error) {
    openerMsg(t("dialog.open.dirError", { msg: serverMessage(error) }));
  }
}

export function renderBrowse(response) {
  state.opener.dir = response.dir;
  state.opener.entries = response.entries || [];
  try {
    localStorage.setItem(BROWSE_KEY, response.dir);
  } catch {
    // Browser storage is only a convenience.
  }
  renderCwdCrumbs(response.dir);
  if (currentOpenerMode() === "folder") $("opener-input").value = response.dir;
  const list = $("opener-list");
  list.textContent = "";
  if (response.parent) {
    list.append(browseRow({ name: "..", path: response.parent, is_dir: true }, true));
  }
  for (const entry of response.entries) list.append(browseRow(entry, false));
  list.scrollTop = 0;
  resetOpenerSelection();
}

export const DRIVES_DIR = "::";

export function renderPathCrumbs(host: HTMLElement, path, onNavigate: (value: string) => void) {
  const clean = String(path || "").replace(/^\\\\\?\\/, "");
  host.textContent = "";
  host.title = clean;
  let crumbs = pathCrumbs(clean);
  if (clean === DRIVES_DIR) {
    crumbs = [{ label: t("dialog.open.thisPc"), path: DRIVES_DIR }];
  } else if (/^[A-Za-z]:[\\/]/.test(clean)) {
    crumbs = [{ label: t("dialog.open.thisPc"), path: DRIVES_DIR }, ...crumbs];
  }
  for (const [index, crumb] of crumbs.entries()) {
    if (index > 0) {
      const separator = el("span", "cwd-sep");
      separator.setAttribute("aria-hidden", "true");
      separator.append(iconSvg("i-chevron-right"));
      host.append(separator);
    }
    const crumbButton = button("cwd-crumb", crumb.label, () => onNavigate(crumb.path));
    crumbButton.title = crumb.path;
    host.append(crumbButton);
  }
}

export function renderCwdCrumbs(path) {
  renderPathCrumbs($("opener-cwd"), path, browse);
}

export function browseRow(entry, isUp) {
  const row = button("opener-row" + (entry.is_dir ? " dir" : "") + (isUp ? " up" : ""), "");
  prepareOpenerOption(row);
  row.setAttribute(
    "aria-label",
    isUp
      ? t("dialog.open.up")
      : `${entry.is_dir ? t("dialog.open.folder") : t("menu.file")}: ${entry.name}`,
  );
  const icon = el("span", "ic");
  icon.setAttribute("aria-hidden", "true");
  icon.append(iconSvg(isUp ? "i-folder-up" : entry.is_dir ? "i-folder" : "i-file"));
  const name = el("span", "nm", isUp ? t("dialog.open.up") : entry.name);
  const size = el("span", "sz", entry.is_dir ? "" : humanBytes(entry.size, currentLocale()));
  row.append(icon, name, size);
  row.addEventListener("click", () => {
    if (entry.is_dir) browse(entry.path);
    else if (currentOpenerMode() === "save") {
      $("opener-input").value = entry.name;
      markPickedFile(entry.name);
      $("opener-input").focus();
    } else if (currentOpenerMode() === "open") {
      openPickedPath(entry.path);
    }
  });
  row.addEventListener("dblclick", () => {
    if (!entry.is_dir && currentOpenerMode() === "save") commitPickedFile();
  });
  return row;
}

export function markPickedFile(name) {
  for (const row of $("opener-list").querySelectorAll(".opener-row")) {
    row.classList.toggle("picked", row.querySelector(".nm")?.textContent === name);
  }
}
