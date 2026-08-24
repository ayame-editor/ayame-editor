// Ayame Editor — recent file persistence and opener rows.
import { $, button, el, iconSvg, isUntitled, pathBaseName, pathDirName } from "./dom.js";
import { t } from "./i18n.js";
import { loadRecentFilesShared, saveRecentFilesShared } from "./persistence.js";
import { prepareOpenerOption, resetOpenerSelection } from "./browse.js";
import { currentOpenerMode } from "./opener-state.js";

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
  const list = currentOpenerMode() === "open" ? loadRecentFiles() : [];
  box.textContent = "";
  resetOpenerSelection();
  if (!list.length) {
    box.classList.add("hidden");
    return;
  }
  const heading = el("div", "opener-recent-head", t("dialog.open.recent"));
  heading.setAttribute("aria-hidden", "true");
  box.append(heading);
  for (const path of list) box.append(recentRow(path));
  box.classList.remove("hidden");
}

export function recentRow(path) {
  const row = button("opener-row recent", "");
  prepareOpenerOption(row);
  row.title = path;
  row.setAttribute("aria-label", `${t("dialog.open.recent")}: ${pathBaseName(path) || path}`);
  const icon = el("span", "ic");
  icon.setAttribute("aria-hidden", "true");
  icon.append(iconSvg("i-clock"));
  const name = el("span", "nm", pathBaseName(path) || path);
  const directory = el("span", "sz", pathDirName(path) || "");
  row.append(icon, name, directory);
  row.addEventListener("click", () => openRecent(path));
  return row;
}
