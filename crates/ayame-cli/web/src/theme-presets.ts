// Canonical built-in theme data lives in ../themes.json. Cargo's build script
// embeds the same JSON as a plain JavaScript module for the shipped browser;
// Vite/tsc consume this source adapter during frontend tests and type checks.
import themeSource from "../themes.json";

export const THEME_PRESETS = Object.fromEntries(
  Object.entries(themeSource).map(([id, source]) => {
    const { ui: _ui, ...preset } = source;
    return [id, preset];
  }),
);

export const BUILTIN_THEMES = Object.entries(themeSource)
  .sort(([, left], [, right]) => left.ui.order - right.ui.order)
  .map(([id, source]) => ({
    id,
    name: source.name,
    labelKey: "labelKey" in source.ui ? source.ui.labelKey : null,
  }));
