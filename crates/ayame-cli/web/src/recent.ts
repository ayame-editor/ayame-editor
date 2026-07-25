// Ayame Editor — recent file persistence and opener rows.
import { $, iconSvg, isUntitled, pathBaseName, pathDirName } from "./dom.js";
import { state } from "./state.js";
import { t } from "./i18n.js";
import { loadRecentFilesShared, saveRecentFilesShared } from "./persistence.js";
import { prepareOpenerOption, resetOpenerSelection } from "./browse.js";

let openRecentPath = async (_path) => false;

export function setRecentService(service) {
  openRecentPath = service.openPath;
}

export function loadRecentFiles() {
  return loadRecentFilesShared();
}

export function saveRecentFiles(list) {
  saveRecentFilesShared(list);
}

export function pushRecentFile(path) {
  const value = (path || "").trim();
  if (!value || isUntitled(value)) return;
  const list = [value, ...loadRecentFiles().filter((entry) => entry !== value)];
  saveRecentFiles(list);
}

export function dropRecentFile(path) {
  saveRecentFiles(loadRecentFiles().filter((entry) => entry !== path));
}

export async function openRecent(path) {
  const ok = await openRecentPath(path);
  if (!ok) {
    dropRecentFile(path);
    renderRecentFiles();
  }
}

export function renderRecentFiles() {
  const box = $("opener-recent");
  if (!box) return;
  const list = state.openerMode === "open" ? loadRecentFiles() : [];
  box.textContent = "";
  resetOpenerSelection();
  if (!list.length) {
    box.classList.add("hidden");
    return;
  }
  const heading = document.createElement("div");
  heading.className = "opener-recent-head";
  heading.setAttribute("aria-hidden", "true");
  heading.textContent = t("dialog.open.recent");
  box.append(heading);
  for (const path of list) box.append(recentRow(path));
  box.classList.remove("hidden");
}

export function recentRow(path) {
  const row = document.createElement("button");
  row.className = "opener-row recent";
  row.type = "button";
  prepareOpenerOption(row);
  row.title = path;
  row.setAttribute("aria-label", `${t("dialog.open.recent")}: ${pathBaseName(path) || path}`);
  const icon = document.createElement("span");
  icon.className = "ic";
  icon.setAttribute("aria-hidden", "true");
  icon.append(iconSvg("i-clock"));
  const name = document.createElement("span");
  name.className = "nm";
  name.textContent = pathBaseName(path) || path;
  const directory = document.createElement("span");
  directory.className = "sz";
  directory.textContent = pathDirName(path) || "";
  row.append(icon, name, directory);
  row.addEventListener("click", () => openRecent(path));
  return row;
}
