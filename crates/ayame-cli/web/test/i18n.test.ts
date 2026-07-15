import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import * as ts from "typescript";
import { describe, expect, it } from "vitest";

import {
  applyStaticI18n,
  I18N_ATTR_MAP,
  localeDir,
  MESSAGES,
  SERVER_CODE_KEYS,
  serverMessage,
} from "../src/i18n.js";
import { state } from "../src/state.js";

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) return sourceFiles(full);
    return entry.isFile() && entry.name.endsWith(".ts") ? [full] : [];
  });
}

function staticTKeys(file: string): string[] {
  const source = readFileSync(file, "utf8");
  const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const keys: string[] = [];
  const visit = (node: ts.Node) => {
    if (
      ts.isCallExpression(node) &&
      ts.isIdentifier(node.expression) &&
      node.expression.text === "t" &&
      ts.isStringLiteralLike(node.arguments[0])
    ) {
      keys.push(node.arguments[0].text);
    }
    ts.forEachChild(node, visit);
  };
  visit(ast);
  return keys;
}

function staticHtmlKeys(): string[] {
  const html = readFileSync(path.join(webRoot, "index.html"), "utf8");
  const doc = new DOMParser().parseFromString(html, "text/html");
  const attrs = ["data-i18n", ...I18N_ATTR_MAP.map(([dataAttr]) => dataAttr)];
  return attrs.flatMap((attr) =>
    [...doc.querySelectorAll(`[${attr}]`)].map((el) => el.getAttribute(attr) ?? ""),
  );
}

describe("i18n completeness", () => {
  it("keeps every locale aligned with the English key set", () => {
    const reference = Object.keys(MESSAGES.en)
      .filter((key) => key !== "weekday")
      .sort();

    for (const [locale, table] of Object.entries(MESSAGES)) {
      const keys = Object.keys(table)
        .filter((key) => key !== "weekday")
        .sort();
      expect(keys, `${locale} translation keys`).toEqual(reference);
      expect(table.weekday?.short, `${locale} short weekdays`).toHaveLength(7);
      expect(table.weekday?.long, `${locale} long weekdays`).toHaveLength(7);
    }
  });

  it("has translations for static t() calls and data-i18n attributes", () => {
    const referenced = new Set([
      ...sourceFiles(path.join(webRoot, "src")).flatMap(staticTKeys),
      ...staticHtmlKeys(),
      ...Object.values(SERVER_CODE_KEYS),
    ]);
    referenced.delete("");

    for (const key of referenced) {
      for (const [locale, table] of Object.entries(MESSAGES)) {
        expect(table, `${locale} missing ${key}`).toHaveProperty(key);
      }
    }
  });
});

describe("localized server errors (#176)", () => {
  it("translates stable server codes in Japanese and English", () => {
    const originalLanguage = state.settings.language;
    try {
      state.settings.language = "ja";
      expect(serverMessage({ code: "not_found", message: "operation not found" })).toBe(
        "要求された項目が見つかりません。",
      );

      state.settings.language = "en";
      expect(serverMessage({ code: "corrupted", message: "data corruption" })).toBe(
        "Data corruption was detected.",
      );
    } finally {
      state.settings.language = originalLanguage;
    }
  });

  it("keeps unknown-code details behind a localized fallback", () => {
    const originalLanguage = state.settings.language;
    try {
      state.settings.language = "ja";
      expect(serverMessage({ code: "future_code", message: "detail" })).toBe(
        "サーバエラー: detail",
      );
      state.settings.language = "en";
      expect(serverMessage({ code: "future_code", message: "detail" })).toBe(
        "Server error: detail",
      );
    } finally {
      state.settings.language = originalLanguage;
    }
  });
});

describe("writing direction (#190)", () => {
  it("declares a direction for every shipped locale", () => {
    for (const [locale, table] of Object.entries(MESSAGES)) {
      expect(["ltr", "rtl"], `${locale} language.dir`).toContain(table["language.dir"]);
    }
  });

  it("reads localeDir from the block and defaults to ltr", () => {
    expect(localeDir("ja")).toBe("ltr");
    expect(localeDir("en")).toBe("ltr");
    expect(localeDir("unknown")).toBe("ltr");
  });

  it("normalizes any non-rtl value to ltr", () => {
    const original = MESSAGES.en["language.dir"];
    try {
      // @ts-expect-error probing the defensive branch with a bogus value
      MESSAGES.en["language.dir"] = "sideways";
      expect(localeDir("en")).toBe("ltr");
      MESSAGES.en["language.dir"] = "rtl";
      expect(localeDir("en")).toBe("rtl");
    } finally {
      MESSAGES.en["language.dir"] = original;
    }
  });

  it("mirrors the active locale's direction onto <html> lang/dir", () => {
    const originalLanguage = state.settings.language;
    const originalDir = MESSAGES.ja["language.dir"];
    try {
      state.settings.language = "ja";
      applyStaticI18n();
      expect(document.documentElement.lang).toBe("ja");
      expect(document.documentElement.dir).toBe("ltr");

      // An RTL locale (declared via data alone) flips document.dir.
      MESSAGES.ja["language.dir"] = "rtl";
      applyStaticI18n();
      expect(document.documentElement.dir).toBe("rtl");
    } finally {
      MESSAGES.ja["language.dir"] = originalDir;
      state.settings.language = originalLanguage;
      applyStaticI18n();
    }
  });
});
