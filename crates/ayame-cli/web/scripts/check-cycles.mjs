import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "src");

function sourceFiles(dir) {
  return fs
    .readdirSync(dir, { withFileTypes: true })
    .flatMap((entry) => {
      const fullPath = path.join(dir, entry.name);
      if (entry.isDirectory()) return sourceFiles(fullPath);
      return entry.isFile() && entry.name.endsWith(".ts") ? [fullPath] : [];
    })
    .sort();
}

const files = sourceFiles(root);
const knownFiles = new Set(files);
const graph = new Map(files.map((file) => [file, new Set()]));

function localTarget(from, specifier) {
  if (!specifier.startsWith(".")) return null;
  const resolved = path.resolve(path.dirname(from), specifier);
  const candidates = specifier.endsWith(".js")
    ? [resolved.slice(0, -3) + ".ts"]
    : [resolved, `${resolved}.ts`, path.join(resolved, "index.ts")];
  return candidates.find((candidate) => knownFiles.has(candidate)) ?? null;
}

for (const file of files) {
  const source = ts.createSourceFile(
    file,
    fs.readFileSync(file, "utf8"),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TS,
  );
  const add = (specifier) => {
    const target = localTarget(file, specifier);
    if (target) graph.get(file).add(target);
  };
  const visit = (node) => {
    if (
      (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) &&
      node.moduleSpecifier &&
      ts.isStringLiteral(node.moduleSpecifier)
    ) {
      add(node.moduleSpecifier.text);
    } else if (
      ts.isCallExpression(node) &&
      node.expression.kind === ts.SyntaxKind.ImportKeyword &&
      node.arguments.length === 1 &&
      ts.isStringLiteral(node.arguments[0])
    ) {
      add(node.arguments[0].text);
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
}

let nextIndex = 0;
const indices = new Map();
const lowLinks = new Map();
const stack = [];
const onStack = new Set();
const cycles = [];

function connect(file) {
  indices.set(file, nextIndex);
  lowLinks.set(file, nextIndex++);
  stack.push(file);
  onStack.add(file);

  for (const target of graph.get(file)) {
    if (!indices.has(target)) {
      connect(target);
      lowLinks.set(file, Math.min(lowLinks.get(file), lowLinks.get(target)));
    } else if (onStack.has(target)) {
      lowLinks.set(file, Math.min(lowLinks.get(file), indices.get(target)));
    }
  }

  if (lowLinks.get(file) !== indices.get(file)) return;
  const component = [];
  let member;
  do {
    member = stack.pop();
    onStack.delete(member);
    component.push(member);
  } while (member !== file);
  if (component.length > 1 || graph.get(file).has(file)) cycles.push(component.sort());
}

for (const file of files) {
  if (!indices.has(file)) connect(file);
}

const relative = (file) => path.relative(root, file).split(path.sep).join("/");

if (cycles.length) {
  console.error(`Circular imports found in ${cycles.length} component(s):`);
  for (const component of cycles) {
    const members = new Set(component);
    console.error(`\n  ${component.map(relative).join(", ")}`);
    for (const file of component) {
      const targets = [...graph.get(file)].filter((target) => members.has(target));
      if (targets.length) {
        console.error(`    ${relative(file)} -> ${targets.map(relative).join(", ")}`);
      }
    }
  }
  process.exitCode = 1;
} else {
  console.log(`No circular imports in ${files.length} frontend modules.`);
}
