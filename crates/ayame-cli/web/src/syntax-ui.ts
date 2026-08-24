import { $, button, el, pathBaseName, setModalOpen } from "./dom.js";
import { askPrompt } from "./dialogs.js";
import { focusEditor, invalidateRenderedRows, render } from "./editor.js";
import { t } from "./i18n.js";
import { flashCount } from "./notifications.js";
import { registerModal } from "./modal-state.js";
import { saveSyntaxPreferencesShared } from "./persistence.js";
import { showPopupMenu } from "./popup-menu.js";
import { state } from "./state.js";
import {
  isSchemeId,
  clearSyntaxCache,
  resolveSyntaxScheme,
  schemeDefinition,
  SYNTAX_SCHEMES,
  type SchemeId,
  type SyntaxGlobMapping,
  type SyntaxSelection,
} from "./syntax.js";
import {
  normalizeSyntaxPreferences,
  syntaxPreferenceJson,
  type SyntaxPreferences,
} from "./syntax-preference-model.js";
import { updateStatusMeta } from "./status.js";

let initialized = false;
let draftFavorites: SchemeId[] = [];
let draftMappings: SyntaxGlobMapping[] = [];
let draftOverrides: Record<string, SchemeId> = {};

function activePath() {
  return String(state.doc.stat?.path || "");
}

function currentSelection(path = activePath()): SyntaxSelection {
  return state.syntax.overrides[path] || "auto";
}

function draftSelection(path = activePath()): SyntaxSelection {
  return draftOverrides[path] || "auto";
}

function draftPreferences(selection: SyntaxSelection) {
  const path = activePath();
  const overrides = { ...draftOverrides };
  if (path) {
    if (selection === "auto") delete overrides[path];
    else overrides[path] = selection;
  }
  return normalizeSyntaxPreferences({
    configured: true,
    favorites: draftFavorites,
    mappings: draftMappings,
    overrides,
  });
}

function refreshSyntaxRendering() {
  clearSyntaxCache();
  invalidateRenderedRows();
  updateStatusMeta();
  render();
}

function persistDraft(selection: SyntaxSelection) {
  const clean = draftPreferences(selection);
  draftOverrides = clean.overrides;
  saveSyntaxPreferencesShared({
    configured: true,
    favorites: clean.favorites,
    mappings: clean.mappings,
    overrides: clean.overrides,
  });
  refreshSyntaxRendering();
  return clean.invalid;
}

export function applySyntaxSelection(selection: SyntaxSelection) {
  draftFavorites = [...state.syntax.favorites];
  draftMappings = state.syntax.mappings.map((mapping) => ({ ...mapping }));
  draftOverrides = { ...state.syntax.overrides };
  const invalid = persistDraft(selection);
  if (invalid) flashCount(t("syntax.savePartial", { count: invalid }));
}

function schemeLabel(id: SchemeId) {
  return t(schemeDefinition(id).labelKey);
}

function appendSchemeOptions(select: HTMLSelectElement, includeAuto = false) {
  select.replaceChildren();
  if (includeAuto) {
    const option = el("option", "", t("syntax.auto"));
    option.value = "auto";
    select.append(option);
  }
  for (const scheme of SYNTAX_SCHEMES) {
    const option = el("option", "", schemeLabel(scheme.id));
    option.value = scheme.id;
    select.append(option);
  }
}

function aliasSummary(scheme: (typeof SYNTAX_SCHEMES)[number]) {
  const aliases: {
    extensions?: readonly string[];
    filenames?: readonly string[];
    globs?: readonly string[];
  } = scheme.aliases;
  return [
    ...(aliases.filenames || []),
    ...(aliases.extensions || []).map((extension) => `*.${extension}`),
    ...(aliases.globs || []),
  ].join(", ");
}

function moveFavorite(id: SchemeId, delta: number) {
  const index = draftFavorites.indexOf(id);
  if (index < 0) return;
  const next = index + delta;
  if (next < 0 || next >= draftFavorites.length) return;
  [draftFavorites[index], draftFavorites[next]] = [draftFavorites[next], draftFavorites[index]];
  renderSchemeCatalog();
}

function renderSchemeCatalog() {
  const host = $("syntax-scheme-list");
  const query = String($("syntax-search").value || "")
    .trim()
    .toLocaleLowerCase();
  host.textContent = "";
  const favoriteOrder = new Map(draftFavorites.map((id, index) => [id, index]));
  const schemes = [...SYNTAX_SCHEMES].sort((left, right) => {
    const leftOrder = favoriteOrder.get(left.id);
    const rightOrder = favoriteOrder.get(right.id);
    if (leftOrder != null || rightOrder != null) {
      if (leftOrder == null) return 1;
      if (rightOrder == null) return -1;
      return leftOrder - rightOrder;
    }
    return schemeLabel(left.id).localeCompare(schemeLabel(right.id));
  });
  for (const scheme of schemes) {
    const label = schemeLabel(scheme.id);
    const category = t(scheme.categoryKey);
    const aliases = aliasSummary(scheme);
    if (query && !`${label} ${category} ${aliases}`.toLocaleLowerCase().includes(query)) continue;
    const row = el("div", "syntax-scheme-row");
    row.setAttribute("role", "listitem");
    const text = button("syntax-scheme-text", "", () => {
      ($("syntax-current") as HTMLSelectElement).value = scheme.id;
    });
    text.type = "button";
    text.title = t("syntax.selectScheme", { scheme: label });
    text.setAttribute("aria-label", text.title);
    text.append(el("span", "syntax-scheme-name", label));
    text.append(el("span", "syntax-scheme-meta", [category, aliases].filter(Boolean).join(" · ")));
    const favorite = draftFavorites.includes(scheme.id);
    const star = button("syntax-favorite", favorite ? "★" : "☆", () => {
      if (favorite) draftFavorites = draftFavorites.filter((id) => id !== scheme.id);
      else draftFavorites = [...draftFavorites, scheme.id];
      renderSchemeCatalog();
    });
    star.title = t(favorite ? "syntax.removeFavorite" : "syntax.addFavorite");
    star.setAttribute("aria-label", star.title);
    star.setAttribute("aria-pressed", String(favorite));
    row.append(text, star);
    if (favorite) {
      const up = button("syntax-order", "↑", () => moveFavorite(scheme.id, -1));
      const down = button("syntax-order", "↓", () => moveFavorite(scheme.id, 1));
      up.title = t("syntax.moveUp");
      down.title = t("syntax.moveDown");
      up.setAttribute("aria-label", up.title);
      down.setAttribute("aria-label", down.title);
      up.disabled = draftFavorites[0] === scheme.id;
      down.disabled = draftFavorites.at(-1) === scheme.id;
      row.append(up, down);
    }
    host.append(row);
  }
}

function moveMapping(index: number, delta: number) {
  const next = index + delta;
  if (next < 0 || next >= draftMappings.length) return;
  [draftMappings[index], draftMappings[next]] = [draftMappings[next], draftMappings[index]];
  renderMappings();
}

function renderMappings() {
  const host = $("syntax-mapping-list");
  host.textContent = "";
  draftMappings.forEach((mapping, index) => {
    const row = el("div", "syntax-mapping-row");
    const glob = el("input", "input-control input-control--mono");
    glob.type = "text";
    glob.value = mapping.glob;
    glob.placeholder = "*.conf";
    glob.setAttribute("aria-label", t("syntax.glob"));
    glob.addEventListener("input", () => {
      draftMappings[index] = { ...draftMappings[index], glob: glob.value };
    });
    const scheme = el("select", "input-control");
    appendSchemeOptions(scheme);
    scheme.value = mapping.scheme;
    scheme.setAttribute("aria-label", t("syntax.scheme"));
    scheme.addEventListener("change", () => {
      if (isSchemeId(scheme.value))
        draftMappings[index] = { ...draftMappings[index], scheme: scheme.value };
    });
    const up = button("syntax-order", "↑", () => moveMapping(index, -1));
    const down = button("syntax-order", "↓", () => moveMapping(index, 1));
    const remove = button("syntax-remove", "×", () => {
      draftMappings.splice(index, 1);
      renderMappings();
    });
    up.title = t("syntax.moveUp");
    down.title = t("syntax.moveDown");
    remove.title = t("common.remove");
    up.setAttribute("aria-label", up.title);
    down.setAttribute("aria-label", down.title);
    remove.setAttribute("aria-label", remove.title);
    up.disabled = index === 0;
    down.disabled = index === draftMappings.length - 1;
    row.append(glob, scheme, up, down, remove);
    host.append(row);
  });
}

function hideSyntaxManager() {
  setModalOpen($("syntax-modal"), false);
  focusEditor();
}

export function showSyntaxManager() {
  draftFavorites = [...state.syntax.favorites];
  draftMappings = state.syntax.mappings.map((mapping) => ({ ...mapping }));
  draftOverrides = { ...state.syntax.overrides };
  const current = $("syntax-current") as HTMLSelectElement;
  appendSchemeOptions(current, true);
  current.value = draftSelection();
  $("syntax-search").value = "";
  $("syntax-import-status").textContent = "";
  renderSchemeCatalog();
  renderMappings();
  const settings = document.getElementById("settings");
  if (settings) setModalOpen(settings, false);
  setModalOpen($("syntax-modal"), true);
  queueMicrotask(() => $("syntax-search").focus());
}

function assignCurrentExtension() {
  const path = activePath();
  const name = pathBaseName(path);
  if (!name) return;
  const dot = name.lastIndexOf(".");
  const glob = dot > 0 ? `*.${name.slice(dot + 1)}` : name;
  const selected = $("syntax-current").value as SyntaxSelection;
  const scheme =
    selected === "auto" ? resolveSyntaxScheme(path, "auto", draftMappings) || "plain" : selected;
  if (!isSchemeId(scheme)) return;
  draftMappings = [{ glob, scheme }, ...draftMappings.filter((mapping) => mapping.glob !== glob)];
  renderMappings();
}

async function importPreferences() {
  const value = await askPrompt(t("syntax.import"), t("syntax.importPrompt"), "");
  if (value == null) return;
  try {
    const imported = JSON.parse(value);
    if (!imported || typeof imported !== "object" || Array.isArray(imported)) throw new Error();
    const clean = normalizeSyntaxPreferences({ ...imported, configured: true });
    draftFavorites = clean.favorites;
    draftMappings = clean.mappings;
    draftOverrides = clean.overrides;
    ($("syntax-current") as HTMLSelectElement).value = draftSelection();
    renderSchemeCatalog();
    renderMappings();
    $("syntax-import-status").textContent = clean.invalid
      ? t("syntax.importPartial", { count: clean.invalid })
      : t("syntax.importReady");
  } catch {
    $("syntax-import-status").textContent = t("syntax.importError");
  }
}

async function exportPreferences() {
  const selection = $("syntax-current").value;
  if (!isSchemeId(selection) && selection !== "auto") return;
  const clean = draftPreferences(selection as SyntaxSelection);
  const preferences: SyntaxPreferences = clean;
  try {
    await navigator.clipboard.writeText(JSON.stringify(syntaxPreferenceJson(preferences), null, 2));
    $("syntax-import-status").textContent = t("syntax.exported");
  } catch {
    $("syntax-import-status").textContent = t("syntax.exportError");
  }
}

function showSyntaxQuickMenu() {
  const status = $("st-syntax");
  const rect = status.getBoundingClientRect();
  const selected = currentSelection();
  const items = [
    {
      label: t("syntax.auto"),
      checked: selected === "auto",
      action: () => applySyntaxSelection("auto"),
    },
    {
      label: schemeLabel("plain"),
      checked: selected === "plain",
      action: () => applySyntaxSelection("plain"),
    },
    { separator: true },
    ...state.syntax.favorites
      .filter((id) => id !== "plain")
      .map((id) => ({
        label: schemeLabel(id),
        checked: selected === id,
        action: () => applySyntaxSelection(id),
      })),
    { separator: true },
    { label: t("syntax.manage"), action: showSyntaxManager },
  ];
  showPopupMenu(rect.left, rect.top, items);
}

registerModal("syntax-modal", { onClose: hideSyntaxManager, closeOnBackdrop: true });

export function initSyntaxUi() {
  if (initialized) return;
  initialized = true;
  $("st-syntax").addEventListener("click", showSyntaxQuickMenu);
  $("syntax-manage").addEventListener("click", showSyntaxManager);
  $("syntax-search").addEventListener("input", renderSchemeCatalog);
  $("syntax-mapping-add").addEventListener("click", () => {
    draftMappings.push({ glob: "*.conf", scheme: "plain" });
    renderMappings();
  });
  $("syntax-assign-extension").addEventListener("click", assignCurrentExtension);
  $("syntax-import").addEventListener("click", () => void importPreferences());
  $("syntax-export").addEventListener("click", () => void exportPreferences());
  $("syntax-save").addEventListener("click", () => {
    const selection = $("syntax-current").value;
    if (!isSchemeId(selection) && selection !== "auto") return;
    const invalid = persistDraft(selection as SyntaxSelection);
    if (invalid) flashCount(t("syntax.savePartial", { count: invalid }));
    hideSyntaxManager();
  });
  $("syntax-close").addEventListener("click", hideSyntaxManager);
  $("syntax-cancel").addEventListener("click", hideSyntaxManager);
}
