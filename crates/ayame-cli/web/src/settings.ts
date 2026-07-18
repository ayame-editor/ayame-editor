// Ayame Editor — settings module. Type-stripped to JS at build time (build.rs, oxc).
import { $, setModalOpen } from "./dom.js";
import {
  DEFAULT_KEYMAP,
  DEFAULT_SETTINGS,
  FONT_STACKS,
  KEYMAP_ACTIONS,
  MAX_COPY_LINES,
  SETTINGS_KEY,
  setLineHeight,
  state,
} from "./state.js";
import { availableLocales, localeLabel, normalizeLanguage, t } from "./i18n.js";
import { sanitizeKeymap } from "./keys.js";
import { api, type LinesResponse } from "./api.js";
import { focusEditor, invalidateFontMetrics, scheduleRender } from "./editor.js";
import { postNativeMessage } from "./app.js";
import {
  applyLocale,
  hideKeymap,
  renderKeymapRows,
  resetKeymap,
  showKeymap,
  updateKeyHints,
} from "./menus.js";
import { askConfirm } from "./dialogs.js";
import { settleEditQueue } from "./edits.js";
import { flashCount } from "./search.js";
import { onDocumentOpened } from "./workspace.js";

// ---- settings (theme / font) -----------------------------------------------

export const FONT_SIZE_MIN = 6;
export const FONT_SIZE_MAX = 48;
export const FONT_SIZE_STEP = 1;

export function clampFontSize(value) {
  const parsed = Number(value);
  const px = Number.isFinite(parsed) && parsed > 0 ? Math.round(parsed) : DEFAULT_SETTINGS.fontSize;
  return Math.max(FONT_SIZE_MIN, Math.min(FONT_SIZE_MAX, px));
}

function clampLegacyZoom(value) {
  const parsed = Math.round(Number(value) || 100);
  return Math.max(50, Math.min(300, parsed));
}

// Before issue #170, the visible size was stored as two independent values:
// fontSize (11–22px) × zoom (50–300%). Collapse that legacy pair to the exact
// rounded size the old renderer displayed, then persist only fontSize.
export function migratedFontSize(raw) {
  if (raw && typeof raw === "object" && Object.prototype.hasOwnProperty.call(raw, "zoom")) {
    const base = Math.max(11, Math.min(22, Number(raw.fontSize) || DEFAULT_SETTINGS.fontSize));
    return clampFontSize(Math.round((base * clampLegacyZoom(raw.zoom)) / 100));
  }
  return clampFontSize(raw?.fontSize);
}

export function loadSettings() {
  try {
    const raw = JSON.parse(localStorage.getItem(SETTINGS_KEY) || "{}");
    const hadLegacyZoom =
      raw && typeof raw === "object" && Object.prototype.hasOwnProperty.call(raw, "zoom");
    const merged = { ...DEFAULT_SETTINGS, ...(raw && typeof raw === "object" ? raw : {}) };
    merged.fontSize = migratedFontSize(raw);
    delete merged.zoom;
    // Explorer/sidebar settings existed before PR #90's removal was completed.
    // Drop them while loading so the next settings write also cleans old data.
    delete merged.sidebar;
    delete merged.sidebarSide;
    // image mode without a stored image (cleared / failed persist) → theme default
    if (merged.bgMode === "image" && !merged.bgImage) merged.bgMode = "watercolor";
    merged.language = normalizeLanguage(merged.language);
    merged.updateCheckOnStartup = merged.updateCheckOnStartup !== false;
    merged.keymap = sanitizeKeymap(merged.keymap);
    if (hadLegacyZoom) saveSettings(merged);
    return merged;
  } catch {
    return { ...DEFAULT_SETTINGS };
  }
}

export function saveSettings(s) {
  try {
    const persisted = { ...s };
    delete persisted.zoom;
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(persisted));
    return true;
  } catch {
    return false; // private-mode / quota errors (e.g. a large bgImage)
  }
}

// Built-in themes are also defined as CSS `html[data-theme=...]` blocks in
// style.css; these JSON mirrors let the Settings JSON editor show/export them
// and act as a base for custom themes. Custom themes apply at runtime by
// setting the same CSS variables the built-ins use.
export const THEME_PRESETS = {
  "iris-light": {
    name: "Iris Light",
    type: "light",
    radius: 10,
    color: {
      paper: "#FBF8F1",
      paper2: "#FDFCF8",
      ink: "#2A2140",
      inkDim: "#6E6383",
      inkFaint: "#A99DBC",
      accent: "#7A5CC0",
      accent2: "#6A4CB0",
      gold: "#C79A2E",
      edge: "#E7E0D3",
      err: "#C0506A",
      markBg: "#FBEBB0",
      markFg: "#6B5510",
      markCur: "#E8B84B",
      markCurFg: "#2A2205",
    },
    acrylic: { tint: "rgba(255,253,248,0.72)", blur: 20 },
    background: { mode: "watercolor", solid: "#FBF8F1" },
    illustration: 0.18,
    watercolor: [
      { x: "12%", y: "84%", r: "46vh", color: "rgba(122,92,192,0.12)" },
      { x: "88%", y: "14%", r: "42vh", color: "rgba(185,139,214,0.10)" },
      { x: "70%", y: "96%", r: "30vh", color: "rgba(231,197,107,0.08)" },
    ],
  },
  "iris-mist": {
    name: "Iris Mist",
    type: "light",
    radius: 12,
    color: {
      paper: "#F7F9FC",
      paper2: "#FDFEFF",
      ink: "#26314A",
      inkDim: "#5E6E8A",
      inkFaint: "#9DAAC0",
      accent: "#5B79C9",
      accent2: "#4A68B8",
      gold: "#C9A24E",
      edge: "#DCE4EF",
      err: "#C05C74",
      markBg: "#E3ECFB",
      markFg: "#2C3E6B",
      markCur: "#7EC7C0",
      markCurFg: "#0F2A28",
    },
    acrylic: { tint: "rgba(250,252,255,0.68)", blur: 24 },
    background: { mode: "watercolor", solid: "#F7F9FC" },
    illustration: 0.22,
    watercolor: [
      { x: "14%", y: "82%", r: "44vh", color: "rgba(91,121,201,0.12)" },
      { x: "86%", y: "16%", r: "42vh", color: "rgba(143,182,224,0.10)" },
      { x: "74%", y: "96%", r: "30vh", color: "rgba(126,199,192,0.08)" },
    ],
  },
  "iris-dawn": {
    name: "Iris Dawn",
    type: "light",
    radius: 10,
    color: {
      paper: "#FDF6EE",
      paper2: "#FFFBF7",
      ink: "#3A2438",
      inkDim: "#7A5A6E",
      inkFaint: "#B79AA6",
      accent: "#A65CB0",
      accent2: "#944EA0",
      gold: "#E0A94E",
      edge: "#EFE0D6",
      err: "#D96A86",
      markBg: "#FBE7C8",
      markFg: "#7A4A16",
      markCur: "#F0B85A",
      markCurFg: "#3A2205",
    },
    acrylic: { tint: "rgba(255,250,244,0.70)", blur: 20 },
    background: { mode: "watercolor", solid: "#FDF6EE" },
    illustration: 0.22,
    watercolor: [
      { x: "12%", y: "84%", r: "46vh", color: "rgba(166,92,176,0.13)" },
      { x: "84%", y: "16%", r: "42vh", color: "rgba(224,169,78,0.11)" },
      { x: "70%", y: "96%", r: "30vh", color: "rgba(227,154,176,0.10)" },
    ],
  },
  "sumi-light": {
    name: "Sumi Light",
    type: "light",
    radius: 10,
    color: {
      paper: "#FAFAF8",
      paper2: "#FFFFFF",
      ink: "#222024",
      inkDim: "#63616A",
      inkFaint: "#A7A4AE",
      accent: "#7A5CC0",
      accent2: "#6A4CB0",
      gold: "#B7912F",
      edge: "#E6E4DE",
      err: "#B24A5E",
      markBg: "#ECE6FA",
      markFg: "#3E2E63",
      markCur: "#7A5CC0",
      markCurFg: "#FFFFFF",
    },
    acrylic: { tint: "rgba(252,252,250,0.74)", blur: 22 },
    background: { mode: "watercolor", solid: "#FAFAF8" },
    illustration: 0.16,
    watercolor: [
      { x: "16%", y: "82%", r: "40vh", color: "rgba(122,92,192,0.07)" },
      { x: "84%", y: "20%", r: "34vh", color: "rgba(40,36,48,0.03)" },
    ],
  },
  "mono-paper": {
    name: "Mono Paper",
    type: "light",
    radius: 10,
    color: {
      paper: "#F5F3ED",
      paper2: "#FBFAF5",
      ink: "#24231F",
      inkDim: "#6C6A63",
      inkFaint: "#A9A69D",
      accent: "#6F6B79",
      accent2: "#605C6C",
      gold: "#7A7568",
      edge: "#E2DFD6",
      err: "#9A6A6A",
      markBg: "#E7E4EC",
      markFg: "#3A3745",
      markCur: "#6F6B79",
      markCurFg: "#FFFFFF",
    },
    acrylic: { tint: "rgba(245,243,237,0.92)", blur: 8 },
    background: { mode: "solid", solid: "#F4F2EC" },
    illustration: 0,
    watercolor: [],
  },
};

// CSS variables a custom/JSON theme drives (cleared when switching back to a
// built-in data-theme so its CSS block wins).
export const THEME_VARS = [
  "--bg",
  "--bg-elevated",
  "--bg-toolbar",
  "--bg-active-line",
  "--gutter-bg",
  "--edit-bg",
  "--fg",
  "--fg-dim",
  "--fg-faint",
  "--border",
  "--accent",
  "--accent-bright",
  "--status",
  "--status-fg",
  "--gutter-fg",
  "--mark-bg",
  "--mark-fg",
  "--mark-active-bg",
  "--mark-active-fg",
  "--danger",
  "--gold",
  "--desk",
  "--illus",
  "--radius",
  "--acrylic-blur",
];

export function clearCustomVars() {
  const r = document.documentElement.style;
  THEME_VARS.forEach((v) => r.removeProperty(v));
}

export function deskFrom(t) {
  const bg = t.background || { mode: "watercolor" };
  if (bg.mode === "solid") return bg.solid || t.color.paper2 || t.color.paper;
  const layers = (t.watercolor || []).map(
    (b) => `radial-gradient(${b.r} ${b.r} at ${b.x} ${b.y}, ${b.color}, transparent 62%)`,
  );
  layers.push(t.color.paper);
  return layers.join(", ");
}

export function applyCustomVars(t) {
  const r = document.documentElement.style,
    c = t.color || {};
  const S = (k, v) => v != null && r.setProperty(k, v);
  S("--bg", c.paper);
  S("--bg-elevated", c.paper2 || c.paper);
  S("--bg-toolbar", (t.acrylic && t.acrylic.tint) || c.paper);
  S("--bg-active-line", `color-mix(in srgb, ${c.accent} 14%, ${c.paper})`);
  S("--gutter-bg", c.paper);
  S("--edit-bg", c.paper2 || c.paper);
  S("--fg", c.ink);
  S("--fg-dim", c.inkDim);
  S("--fg-faint", c.inkFaint);
  S("--border", c.edge);
  S("--accent", c.accent);
  S("--accent-bright", c.accent2 || c.accent);
  S("--status", (t.acrylic && t.acrylic.tint) || c.paper);
  S("--status-fg", c.inkDim);
  S("--gutter-fg", c.inkFaint);
  S("--mark-bg", c.markBg);
  S("--mark-fg", c.markFg);
  S("--mark-active-bg", c.markCur);
  S("--mark-active-fg", c.markCurFg);
  S("--danger", c.err);
  S("--gold", c.gold);
  S("--radius", (t.radius || 10) + "px");
  S("--acrylic-blur", ((t.acrylic && t.acrylic.blur) ?? 20) + "px");
  S("--desk", deskFrom(t));
  S("--illus", String(t.illustration ?? 0.2));
}

// True when the OS asks for a dark UI (guarded for non-browser test/build env).
function prefersDark() {
  return typeof matchMedia !== "undefined" && matchMedia("(prefers-color-scheme: dark)").matches;
}

// Resolve a stored theme setting to a concrete built-in id. "auto" (or an unset
// value) follows the OS preference; any explicit id is returned unchanged (#153).
export function resolvedThemeId(theme) {
  if (!theme || theme === "auto") return prefersDark() ? "dark" : "iris-light";
  return theme;
}

// Keep the mobile browser chrome (address bar / status bar) in step with the
// active theme by mirroring the computed --bg into <meta name="theme-color">.
function updateThemeColorMeta(root) {
  const meta = document.querySelector('meta[name="theme-color"]');
  if (!meta) return;
  const bg = getComputedStyle(root).getPropertyValue("--bg").trim();
  if (bg) meta.setAttribute("content", bg);
}

export function applySettings(s) {
  const root = document.documentElement;
  // ---- theme (built-in CSS block, or a custom JSON theme at runtime) ----
  clearCustomVars();
  if (s.theme && s.theme.startsWith("custom:")) {
    const t = (s.customThemes || {})[s.theme.slice(7)];
    root.dataset.theme = "custom";
    if (t) applyCustomVars(t);
  } else {
    // iris-* | dark | black | "auto" (→ OS preference). Unknown → :root default.
    root.dataset.theme = resolvedThemeId(s.theme);
  }
  // ---- whitespace glyphs: swap the zenkaku-space box for an underline ----
  root.classList.toggle("zenkaku-underline", !!s.zenkakuUnderline);
  // ---- background mode + illustration (user overrides on top of the theme) ----
  delete root.dataset.bg;
  if (s.bgMode === "solid") {
    const flat = getComputedStyle(root).getPropertyValue("--bg").trim() || "#FBF8F1";
    root.style.setProperty("--desk", flat);
  } else if (s.bgMode === "image" && s.bgImage) {
    // Custom image: show it as-is in the editor background layer (#ambient),
    // not cropped into the あやめ default's fixed, masked bottom-left corner.
    // The default (watercolor) mode keeps the あやめ illustration there.
    root.dataset.bg = "image";
    root.style.setProperty("--ambient-img", `url("${s.bgImage}")`);
  }
  if (typeof s.illus === "number") root.style.setProperty("--illus", String(s.illus));
  // ---- font / size ----
  root.style.setProperty("--mono", FONT_STACKS[s.font] || FONT_STACKS.mono);
  const fs = clampFontSize(s.fontSize);
  root.style.setProperty("--font-size", `${fs}px`);
  const lh = fs + 6;
  root.style.setProperty("--line-height", `${lh}px`);
  setLineHeight(lh); // keep virtualization math in sync with the CSS
  const fontSizeEl = document.getElementById("st-fontsize");
  if (fontSizeEl) {
    fontSizeEl.textContent = `${fs}px`;
    fontSizeEl.setAttribute("aria-label", t("status.fontSizeValue", { value: `${fs}px` }));
    fontSizeEl.classList.toggle("dim", fs === DEFAULT_SETTINGS.fontSize);
  }
  invalidateFontMetrics(); // font metrics changed → remeasure + rebuild the ruler
  // ---- long-line wrapping (折り返し) ----
  // Purely a CSS switch on #content: rows go white-space:pre-wrap and grow past
  // one LINE_HEIGHT so long lines wrap instead of scrolling horizontally. The
  // virtual scroll still steps one *logical* line per row, so files whose lines
  // fit the viewport render (and caret/select) exactly as before. See style.css
  // #content.wrap for the documented limitations on genuinely wrapped lines.
  $("content").classList.toggle("wrap", !!s.wordWrap);
  updateThemeColorMeta(root); // reflect the resolved theme in the browser chrome
  scheduleRender();
}

export function updateSetting(key, value) {
  if (key === "language") value = normalizeLanguage(value);
  if (key === "fontSize") value = clampFontSize(value);
  state.settings = { ...state.settings, [key]: value };
  applySettings(state.settings);
  saveSettings(state.settings);
  if (key === "language") {
    applyLocale();
    postNativeMessage(`ayame:language:${state.settings.language}`);
  }
  if (key === "updateCheckOnStartup") notifyNativeUpdateCheckSetting();
}

function syncFontSizeControls(value) {
  const px = clampFontSize(value);
  const range = document.getElementById("set-fontsize") as HTMLInputElement | null;
  const number = document.getElementById("set-fontsize-number") as HTMLInputElement | null;
  if (range) range.value = String(px);
  if (number) number.value = String(px);
}

export function setFontSize(px) {
  const next = clampFontSize(px);
  updateSetting("fontSize", next);
  syncFontSizeControls(next);
}

export function adjustFontSize(delta) {
  setFontSize(clampFontSize(state.settings.fontSize) + delta);
}

export function settingsVisible() {
  return !$("settings").classList.contains("hidden");
}

export function showSettings() {
  setModalOpen($("settings"), true);
  // Move focus into the panel so it is keyboard-operable the moment it opens
  // (#174); skip the corner ✕ in favor of the first real control.
  queueMicrotask(() => {
    const panel = $("settings").querySelector(".modal-panel");
    const first = panel?.querySelector<HTMLElement>(
      'select, input, button:not(.modal-x), [tabindex]:not([tabindex="-1"])',
    );
    (first || $("settings-close")).focus();
  });
}

export function hideSettings() {
  setModalOpen($("settings"), false);
  focusEditor();
}

// ---- theme JSON editor (in Settings) --------------------------------------

export function themeJSONFor(id) {
  if (id && id.startsWith("custom:"))
    return (state.settings.customThemes || {})[id.slice(7)] || null;
  return THEME_PRESETS[id] || null;
}

export function themeIllusPct(id) {
  const t = themeJSONFor(resolvedThemeId(id));
  return Math.round(((t && t.illustration) ?? 0) * 100);
}

// Build the language <select> from the available locales (Object.keys(MESSAGES)
// plus "auto") so a newly added MESSAGES block appears with no markup change.
export function populateLanguageSelect() {
  const sel = $("set-language");
  if (!sel) return;
  sel.replaceChildren();
  for (const code of ["auto", ...availableLocales()]) {
    const opt = document.createElement("option");
    opt.value = code;
    opt.textContent = localeLabel(code);
    sel.appendChild(opt);
  }
  sel.value = normalizeLanguage(state.settings.language);
}

export function populateThemeSelect() {
  const sel = $("set-theme");
  [...sel.querySelectorAll("option[data-custom]")].forEach((o) => o.remove());
  for (const name of Object.keys(state.settings.customThemes || {})) {
    const o = document.createElement("option");
    o.value = "custom:" + name;
    o.textContent = "★ " + name;
    o.dataset.custom = "1";
    sel.appendChild(o);
  }
}

export function persistCustomTheme(t) {
  const customs = { ...state.settings.customThemes };
  customs[t.name] = t;
  state.settings = {
    ...state.settings,
    customThemes: customs,
    theme: "custom:" + t.name,
    illus: null,
    bgMode: (t.background && t.background.mode) || "watercolor",
  };
  saveSettings(state.settings);
  populateThemeSelect();
  if ($("set-theme")) $("set-theme").value = "custom:" + t.name;
}

// Open the current theme's JSON as an ordinary editor tab, so it can be edited
// like any text file (edit / undo / Ctrl+S), then applied with テーマ適用.
export async function openThemeJsonDoc() {
  const id = state.settings.theme;
  const preset = themeJSONFor(id) || THEME_PRESETS["iris-light"];
  const jsonText = JSON.stringify(preset, null, 2);
  const base = (id ? id.replace(/^custom:/, "") : "theme") || "theme";
  hideSettings();
  try {
    await settleEditQueue();
    const r = await fetch("/api/upload?name=" + encodeURIComponent(base + ".ayame-theme.json"), {
      method: "POST",
      body: jsonText,
    });
    if (!r.ok) throw new Error(await r.text());
    onDocumentOpened(await r.json());
  } catch (e) {
    flashCount(t("theme.cannotOpen"));
    console.error(e);
  }
}

// Apply the theme JSON in the active buffer (a *.ayame-theme.json tab).
export async function applyThemeFromBuffer() {
  try {
    const count = Math.min(state.total, MAX_COPY_LINES);
    const r = await api<LinesResponse>(`/api/lines?start=0&count=${count}`);
    const text = r.lines.map((l) => l.text).join("\n");
    const theme = JSON.parse(text);
    if (!theme.color) return flashCount(t("theme.missingColor"));
    document.documentElement.dataset.theme = "custom";
    clearCustomVars();
    applyCustomVars(theme);
    if (theme.name) persistCustomTheme(theme);
    flashCount(theme.name ? t("theme.applied", { name: theme.name }) : t("toolbar.applyTheme"));
  } catch (e) {
    flashCount(t("theme.jsonError"));
    console.error(e);
  }
}

export function isThemeDoc(path) {
  return !!path && /\.ayame-theme\.json$/i.test(path);
}

export function keymapJSONForEditor() {
  const out = {};
  for (const [action] of KEYMAP_ACTIONS) {
    out[action] = Object.prototype.hasOwnProperty.call(state.settings.keymap || {}, action)
      ? state.settings.keymap[action]
      : DEFAULT_KEYMAP[action];
  }
  return out;
}

export async function openKeymapJsonDoc() {
  hideKeymap();
  try {
    await settleEditQueue();
    const r = await fetch("/api/upload?name=" + encodeURIComponent("keymap.ayame-keys.json"), {
      method: "POST",
      body: JSON.stringify(keymapJSONForEditor(), null, 2),
    });
    if (!r.ok) throw new Error(await r.text());
    onDocumentOpened(await r.json());
  } catch (e) {
    flashCount(t("keymap.cannotOpen"));
    console.error(e);
  }
}

export async function applyKeymapFromBuffer() {
  try {
    const count = Math.min(state.total, MAX_COPY_LINES);
    const r = await api<LinesResponse>(`/api/lines?start=0&count=${count}`);
    const text = r.lines.map((l) => l.text).join("\n");
    const parsed = JSON.parse(text);
    const clean = sanitizeKeymap(parsed);
    state.settings = { ...state.settings, keymap: clean };
    saveSettings(state.settings);
    updateKeyHints();
    renderKeymapRows();
    flashCount(t("toolbar.applyKeymap"));
  } catch (e) {
    flashCount(t("keymap.jsonError"));
    console.error(e);
  }
}

export function isKeymapDoc(path) {
  return !!path && /\.ayame-keys\.json$/i.test(path);
}

function notifyNativeUpdateCheckSetting() {
  postNativeMessage(
    `ayame:update-check-startup:${state.settings.updateCheckOnStartup === false ? "off" : "on"}`,
  );
}

export function initSettings() {
  state.settings = loadSettings();
  applySettings(state.settings);
  // Follow live OS light/dark changes while the theme is on "auto" (#153).
  if (typeof matchMedia !== "undefined") {
    matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
      const theme = state.settings.theme;
      if (!theme || theme === "auto") applySettings(state.settings);
    });
  }
  notifyNativeUpdateCheckSetting();
  populateThemeSelect();
  $("set-theme").value = state.settings.theme;
  $("set-bg").value = state.settings.bgMode || "watercolor";
  populateLanguageSelect();
  const illusPct =
    state.settings.illus == null
      ? themeIllusPct(state.settings.theme)
      : Math.round(state.settings.illus * 100);
  $("set-illus").value = String(illusPct);
  $("set-illus-val").textContent = illusPct + "%";
  $("set-font").value = state.settings.font;
  syncFontSizeControls(state.settings.fontSize);

  $("set-theme").addEventListener("change", () => {
    const id = $("set-theme").value;
    state.settings = { ...state.settings, theme: id, illus: null };
    saveSettings(state.settings);
    applySettings(state.settings);
    const pct = themeIllusPct(id);
    $("set-illus").value = String(pct);
    $("set-illus-val").textContent = pct + "%";
  });
  // ---- background: デフォルト / 単色 / カスタム画像 ----
  // The wallpaper persists as a data: URL inside settings; cap the source file
  // so the JSON stays within typical localStorage quotas (~5MB).
  const MAX_BG_IMAGE_BYTES = 4 * 1024 * 1024;
  const syncBgImageRow = () => {
    $("set-bg-image-row").classList.toggle("hidden", state.settings.bgMode !== "image");
    $("set-bg-image-name").textContent = state.settings.bgImageName || "";
  };
  syncBgImageRow();
  $("set-bg").addEventListener("change", () => {
    const mode = $("set-bg").value;
    if (mode === "image" && !state.settings.bgImage) {
      // Nothing stored yet: ask for a file first; the mode flips once it loads
      // (and stays put if the picker is cancelled).
      $("set-bg").value = state.settings.bgMode || "watercolor";
      $("set-bg-image-file").click();
      return;
    }
    updateSetting("bgMode", mode);
    syncBgImageRow();
  });
  $("set-bg-image-pick").addEventListener("click", () => $("set-bg-image-file").click());
  $("set-bg-image-file").addEventListener("change", () => {
    const file = $("set-bg-image-file").files?.[0];
    $("set-bg-image-file").value = ""; // so re-picking the same file fires again
    if (!file) return;
    if (file.size > MAX_BG_IMAGE_BYTES) {
      flashCount(t("settings.bgImageTooLarge"));
      return;
    }
    const reader = new FileReader();
    reader.onload = () => {
      state.settings = {
        ...state.settings,
        bgMode: "image",
        bgImage: String(reader.result),
        bgImageName: file.name,
      };
      if (!saveSettings(state.settings)) flashCount(t("settings.bgImagePersistError"));
      applySettings(state.settings);
      $("set-bg").value = "image";
      syncBgImageRow();
    };
    reader.onerror = () => flashCount(t("settings.bgImageError"));
    reader.readAsDataURL(file);
  });
  $("set-language").addEventListener("change", () =>
    updateSetting("language", $("set-language").value),
  );
  $("set-illus").addEventListener("input", () => {
    const v = Number($("set-illus").value);
    $("set-illus-val").textContent = v + "%";
    updateSetting("illus", v / 100);
  });
  $("set-font").addEventListener("change", () => updateSetting("font", $("set-font").value));
  $("set-fontsize").addEventListener("input", () => {
    setFontSize($("set-fontsize").value);
  });
  const fontSizeNumber = $("set-fontsize-number") as HTMLInputElement;
  fontSizeNumber.addEventListener("input", () => {
    const value = fontSizeNumber.valueAsNumber;
    if (Number.isFinite(value) && value >= FONT_SIZE_MIN && value <= FONT_SIZE_MAX) {
      setFontSize(value);
    }
  });
  fontSizeNumber.addEventListener("change", () => setFontSize(fontSizeNumber.valueAsNumber));
  $("set-ruler").checked = !!state.settings.ruler;
  $("set-ruler").addEventListener("change", () => updateSetting("ruler", $("set-ruler").checked));
  $("set-line-commas").checked = state.settings.lineNumberCommas !== false;
  $("set-line-commas").addEventListener("change", () =>
    updateSetting("lineNumberCommas", $("set-line-commas").checked),
  );
  $("set-show-whitespace").checked = !!state.settings.showWhitespace;
  $("set-show-whitespace").addEventListener("change", () =>
    updateSetting("showWhitespace", $("set-show-whitespace").checked),
  );
  $("set-syntax-highlight").checked = state.settings.syntaxHighlight !== false;
  $("set-syntax-highlight").addEventListener("change", () =>
    updateSetting("syntaxHighlight", $("set-syntax-highlight").checked),
  );
  $("set-zenkaku-underline").checked = !!state.settings.zenkakuUnderline;
  $("set-zenkaku-underline").addEventListener("change", () =>
    updateSetting("zenkakuUnderline", $("set-zenkaku-underline").checked),
  );
  $("set-word-wrap").checked = !!state.settings.wordWrap;
  $("set-word-wrap").addEventListener("change", () =>
    updateSetting("wordWrap", $("set-word-wrap").checked),
  );
  $("set-restore-session").checked = state.settings.restoreSession !== false;
  $("set-restore-session").addEventListener("change", () =>
    updateSetting("restoreSession", $("set-restore-session").checked),
  );
  $("set-update-check-startup").checked = state.settings.updateCheckOnStartup !== false;
  $("set-update-check-startup").addEventListener("change", () =>
    updateSetting("updateCheckOnStartup", $("set-update-check-startup").checked),
  );
  $("set-confirm-last-tab-close").checked = state.settings.confirmLastTabClose !== false;
  $("set-confirm-last-tab-close").addEventListener("change", () =>
    updateSetting("confirmLastTabClose", $("set-confirm-last-tab-close").checked),
  );
  $("set-memo-name").value = state.settings.memoName || DEFAULT_SETTINGS.memoName;
  $("set-memo-name").addEventListener("input", () =>
    updateSetting("memoName", $("set-memo-name").value),
  );
  $("theme-json-edit").addEventListener("click", openThemeJsonDoc);
  $("keymap-open").addEventListener("click", showKeymap);
  $("keymap-close").addEventListener("click", hideKeymap);
  $("keymap-done").addEventListener("click", hideKeymap);
  $("keymap-reset").addEventListener("click", async () => {
    if (await askConfirm(t("keymap.reset"), t("keymap.resetConfirm"), { danger: true })) {
      resetKeymap();
    }
  });
  $("keymap-json-edit").addEventListener("click", openKeymapJsonDoc);
  $("keymap-modal").addEventListener("click", (e) => {
    if (e.target === $("keymap-modal")) hideKeymap();
  });

  $("settings-close").addEventListener("click", hideSettings);
  $("settings").addEventListener("click", (e) => {
    if (e.target === $("settings")) hideSettings();
  });
  applyLocale();
}
