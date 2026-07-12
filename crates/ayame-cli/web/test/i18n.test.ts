import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import * as ts from "typescript";
import { afterEach, describe, expect, it } from "vitest";

import { applyStaticI18n, I18N_ATTR_MAP, MESSAGES, serverMessage } from "../src/i18n.js";
import { state } from "../src/state.js";

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

afterEach(() => {
  state.settings.language = "auto";
  document.documentElement.lang = "en";
  document.documentElement.dir = "";
});

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
    ]);
    referenced.delete("");

    for (const key of referenced) {
      for (const [locale, table] of Object.entries(MESSAGES)) {
        expect(table, `${locale} missing ${key}`).toHaveProperty(key);
      }
    }
  });
});

describe("locale behavior", () => {
  it("translates stable server error codes in every built-in locale", () => {
    state.settings.language = "ja";
    expect(serverMessage({ code: "not_found", message: "raw detail" })).toBe(
      "対象が見つかりません。",
    );
    state.settings.language = "en";
    expect(serverMessage({ code: "not_found", message: "raw detail" })).toBe(
      "The requested item was not found.",
    );
    expect(serverMessage({ code: "future_code", message: "raw detail" })).toBe("raw detail");
  });

  it("applies both language and text direction from locale data", () => {
    const tables = MESSAGES as Record<string, (typeof MESSAGES)["en"]>;
    tables.ar = { ...MESSAGES.en, "language.name": "العربية", "language.dir": "rtl" };
    try {
      state.settings.language = "ar";
      applyStaticI18n();
      expect(document.documentElement.lang).toBe("ar");
      expect(document.documentElement.dir).toBe("rtl");
    } finally {
      delete tables.ar;
    }
  });
});
