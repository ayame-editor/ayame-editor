import { isSchemeId, type SchemeId, type SyntaxGlobMapping } from "./syntax.js";

export const SYNTAX_PREFERENCES_KEY = "ayame.syntaxPreferences.v1";
export const DEFAULT_SYNTAX_FAVORITES: readonly SchemeId[] = [
  "json",
  "log",
  "markdown",
  "javascript",
  "rust",
];

export type SyntaxPreferences = {
  configured: boolean;
  favorites: SchemeId[];
  mappings: SyntaxGlobMapping[];
  overrides: Record<string, SchemeId>;
};

export type SyntaxPreferencesInput = {
  configured?: unknown;
  favorites?: unknown;
  mappings?: unknown;
  overrides?: unknown;
  syntax_configured?: unknown;
  syntax_favorites?: unknown;
  syntax_mappings?: unknown;
  syntax_overrides?: unknown;
};

function cleanText(value: unknown, max: number): string {
  return Array.from(String(value ?? ""))
    .filter((character) => {
      const code = character.codePointAt(0) ?? 0;
      return code >= 32 && code !== 127;
    })
    .join("")
    .trim()
    .slice(0, max);
}

export function normalizeSyntaxPreferences(
  input: SyntaxPreferencesInput = {},
): SyntaxPreferences & { invalid: number } {
  let invalid = 0;
  const favorites: SchemeId[] = [];
  const rawFavorites = input.favorites ?? input.syntax_favorites;
  if (rawFavorites != null && !Array.isArray(rawFavorites)) invalid++;
  for (const value of Array.isArray(rawFavorites) ? rawFavorites : []) {
    if (!isSchemeId(value) || favorites.includes(value) || favorites.length >= 32) {
      invalid++;
      continue;
    }
    favorites.push(value);
  }

  const mappings: SyntaxGlobMapping[] = [];
  const rawMappings = input.mappings ?? input.syntax_mappings;
  if (rawMappings != null && !Array.isArray(rawMappings)) invalid++;
  for (const value of Array.isArray(rawMappings) ? rawMappings : []) {
    const glob = cleanText((value as { glob?: unknown })?.glob, 256);
    const scheme = (value as { scheme?: unknown })?.scheme;
    if (
      !glob ||
      !isSchemeId(scheme) ||
      mappings.some((mapping) => mapping.glob === glob) ||
      mappings.length >= 64
    ) {
      invalid++;
      continue;
    }
    mappings.push({ glob, scheme });
  }

  const overrides: Record<string, SchemeId> = {};
  const rawOverrides = input.overrides ?? input.syntax_overrides;
  if (
    rawOverrides != null &&
    !Array.isArray(rawOverrides) &&
    (typeof rawOverrides !== "object" || rawOverrides === null)
  ) {
    invalid++;
  }
  const entries = Array.isArray(rawOverrides)
    ? rawOverrides.map((value) => [
        (value as { path?: unknown })?.path,
        (value as { scheme?: unknown })?.scheme,
      ])
    : Object.entries(rawOverrides && typeof rawOverrides === "object" ? rawOverrides : {});
  for (const [rawPath, scheme] of entries) {
    const path = cleanText(rawPath, 4096);
    if (!path || !isSchemeId(scheme) || path in overrides || Object.keys(overrides).length >= 256) {
      invalid++;
      continue;
    }
    overrides[path] = scheme;
  }

  return {
    configured: Boolean(input.configured ?? input.syntax_configured),
    favorites,
    mappings,
    overrides,
    invalid,
  };
}

export function defaultSyntaxPreferences(): SyntaxPreferences {
  return {
    configured: false,
    favorites: [...DEFAULT_SYNTAX_FAVORITES],
    mappings: [],
    overrides: {},
  };
}

export function syntaxPreferenceJson(preferences: SyntaxPreferences) {
  return {
    version: 1,
    favorites: [...preferences.favorites],
    mappings: preferences.mappings.map((mapping) => ({ ...mapping })),
    overrides: Object.entries(preferences.overrides).map(([path, scheme]) => ({ path, scheme })),
  };
}

export function moveSyntaxOverride(
  overrides: Readonly<Record<string, SchemeId>>,
  oldPath: string,
  newPath: string,
) {
  if (!oldPath || !newPath || oldPath === newPath || !overrides[oldPath]) return { ...overrides };
  const moved = { ...overrides, [newPath]: overrides[oldPath] };
  delete moved[oldPath];
  return moved;
}
