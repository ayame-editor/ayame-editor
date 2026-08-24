import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { beforeEach, describe, expect, it } from "vitest";

import {
  conversionEncodingValue,
  ENCODINGS,
  encodingLabel,
  encodingSupportsBom,
  populateEncodingSelect,
} from "../src/encodings.js";

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

describe("encoding UI metadata", () => {
  beforeEach(() => {
    document.body.innerHTML = '<select id="encoding"></select>';
  });

  it("covers every core encoding and its serialized aliases", () => {
    expect(ENCODINGS.map((encoding) => encoding.value)).toEqual([
      "utf-8",
      "utf-16le",
      "utf-16be",
      "shift-jis",
      "euc-jp",
      "ascii",
      "iso-2022-jp",
    ]);
    expect(encodingLabel("utf8")).toBe("UTF-8");
    expect(encodingLabel("iso-2022-jp")).toBe("ISO-2022-JP");
    expect(encodingLabel("future-encoding")).toBe("future-encoding");
  });

  it("drives the conversion picker and BOM policy from the same rows", () => {
    const select = document.getElementById("encoding") as HTMLSelectElement;
    populateEncodingSelect(select);
    expect([...select.options].map((option) => [option.value, option.textContent])).toEqual(
      ENCODINGS.filter((encoding) => encoding.canConvert).map((encoding) => [
        encoding.value,
        encoding.label,
      ]),
    );
    expect(conversionEncodingValue("utf8")).toBe("utf-8");
    expect(conversionEncodingValue("ascii")).toBe("utf-8");
    expect(encodingSupportsBom("utf-8")).toBe(true);
    expect(encodingSupportsBom("utf-16be")).toBe(true);
    expect(encodingSupportsBom("shift-jis")).toBe(false);
  });

  it("keeps encoding options out of static HTML", () => {
    const html = readFileSync(path.join(webRoot, "index.html"), "utf8");
    const select = html.match(/<select id="convert-enc"[^>]*>([\s\S]*?)<\/select>/)?.[1];
    expect(select).toBe("");
  });
});
