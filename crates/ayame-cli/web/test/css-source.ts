import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

export function readCssSource() {
  const stylesRoot = path.join(webRoot, "styles");
  return readdirSync(stylesRoot)
    .filter((name) => name.endsWith(".css"))
    .sort()
    .map((name) => readFileSync(path.join(stylesRoot, name), "utf8"))
    .join("");
}
