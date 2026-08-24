import { describe, expect, it } from "vitest";

import {
  defaultSyntaxPreferences,
  moveSyntaxOverride,
  normalizeSyntaxPreferences,
  syntaxPreferenceJson,
} from "../src/syntax-preference-model.js";

describe("syntax preference model (#244)", () => {
  it("preserves favorite and mapping order while isolating malformed entries", () => {
    const normalized = normalizeSyntaxPreferences({
      configured: true,
      favorites: ["rust", "missing", "json", "rust"],
      mappings: [
        { glob: "*.conf", scheme: "nginx" },
        { glob: "", scheme: "json" },
        { glob: "*.txt", scheme: "plain" },
      ],
      overrides: [
        { path: "/tmp/app.conf", scheme: "nginx" },
        { path: "/tmp/bad", scheme: "missing" },
      ],
    });

    expect(normalized.favorites).toEqual(["rust", "json"]);
    expect(normalized.mappings).toEqual([
      { glob: "*.conf", scheme: "nginx" },
      { glob: "*.txt", scheme: "plain" },
    ]);
    expect(normalized.overrides).toEqual({ "/tmp/app.conf": "nginx" });
    expect(normalized.invalid).toBe(4);
  });

  it("reports malformed preference sections without discarding valid sections", () => {
    const normalized = normalizeSyntaxPreferences({
      configured: true,
      favorites: "rust",
      mappings: [{ glob: "*.log", scheme: "log" }],
      overrides: 42,
    });

    expect(normalized.favorites).toEqual([]);
    expect(normalized.mappings).toEqual([{ glob: "*.log", scheme: "log" }]);
    expect(normalized.overrides).toEqual({});
    expect(normalized.invalid).toBe(2);
  });

  it("keeps an intentional empty configured value distinct from defaults", () => {
    expect(normalizeSyntaxPreferences({ configured: true }).configured).toBe(true);
    expect(normalizeSyntaxPreferences({ configured: true }).favorites).toEqual([]);
    expect(defaultSyntaxPreferences().favorites.length).toBeGreaterThan(0);
  });

  it("moves a tab override to its Save As path and exports deterministic arrays", () => {
    const overrides = moveSyntaxOverride({ "/tmp/old": "shell" }, "/tmp/old", "/tmp/new");
    expect(overrides).toEqual({ "/tmp/new": "shell" });
    expect(
      syntaxPreferenceJson({
        configured: true,
        favorites: ["shell"],
        mappings: [{ glob: "*.run", scheme: "shell" }],
        overrides,
      }),
    ).toEqual({
      version: 1,
      favorites: ["shell"],
      mappings: [{ glob: "*.run", scheme: "shell" }],
      overrides: [{ path: "/tmp/new", scheme: "shell" }],
    });
  });
});
