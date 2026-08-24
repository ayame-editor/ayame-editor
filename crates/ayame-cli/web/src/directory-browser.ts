// Inline directory browser shared by form fields that need a server-side path
// without opening a second modal (#172).
import { api } from "./api.js";
import { button, el } from "./dom.js";
import { serverMessage, t } from "./i18n.js";
import type { BrowseResponse } from "./types/api.js";

type DirectoryBrowserOptions = {
  fieldId: string;
  label?: string;
};

export function createDirectoryBrowser(input: HTMLInputElement, options: DirectoryBrowserOptions) {
  const browser = el("div", "form-path-browser hidden");
  const browserPath = el("div", "form-path-browser-path");
  const browserStatus = el("div", "form-path-browser-status");
  const browserList = el("div", "form-path-browser-list");
  const browserId = `form-path-browser-${options.fieldId}`;
  browser.id = browserId;
  browser.setAttribute("role", "group");
  browser.setAttribute("aria-label", t("dialog.open.chooseFolder"));
  browserStatus.setAttribute("role", "status");
  browserStatus.setAttribute("aria-live", "polite");
  browser.append(browserPath, browserStatus, browserList);

  let requestSequence = 0;
  const browse = async (dir: string) => {
    const sequence = ++requestSequence;
    browser.classList.remove("hidden");
    browseButton.setAttribute("aria-expanded", "true");
    browser.setAttribute("aria-busy", "true");
    browserStatus.textContent = t("dialog.open.loading");
    browserList.textContent = "";
    try {
      const query = dir.trim() ? `?dir=${encodeURIComponent(dir.trim())}` : "";
      const response = await api<BrowseResponse>(`/api/browse${query}`);
      if (sequence !== requestSequence) return;
      input.value = response.dir;
      browserPath.textContent = response.dir;
      browserPath.title = response.dir;
      browserStatus.textContent = "";
      const addDirectory = (label: string, path: string, parent = false) => {
        const directory = button(
          parent ? "form-path-dir form-path-dir--parent" : "form-path-dir",
          label,
          () => void browse(path),
        );
        directory.dataset.path = path;
        directory.title = path;
        browserList.append(directory);
      };
      if (response.parent) addDirectory(t("dialog.open.up"), response.parent, true);
      for (const entry of response.entries || []) {
        if (entry.is_dir) addDirectory(entry.name, entry.path);
      }
    } catch (error) {
      if (sequence !== requestSequence) return;
      browserStatus.textContent = t("dialog.open.dirError", {
        msg: serverMessage(error),
      });
    } finally {
      if (sequence === requestSequence) browser.setAttribute("aria-busy", "false");
    }
  };

  const browseButton = button(
    "cmd",
    options.label || t("dialog.open.chooseFolder"),
    () => void browse(input.value),
  );
  browseButton.setAttribute("aria-controls", browserId);
  browseButton.setAttribute("aria-expanded", "false");
  return { browser, browseButton };
}
