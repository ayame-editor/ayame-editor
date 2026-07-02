// Ayame Editor Figma generation script.
// Target file: https://www.figma.com/design/2hMLmr5GKAg9t8VQ8XVTUl/Ayame-Editor-Design
// Paste this file as the `code` value for the Figma `use_figma` tool.

const GENERATED_PREFIX = "Ayame Editor - ";
const created = [];

const C = {
  bg: "#FBF8F1",
  panel: "#FDFCF8",
  panel2: "#F6F1E7",
  ink: "#2A2140",
  muted: "#6E6383",
  border: "#E7E0D3",
  borderStrong: "#D4C6B8",
  accent: "#7A5CC0",
  accent2: "#6A4CB0",
  accentSoft: "#EFE7FF",
  gold: "#C79A2E",
  goldSoft: "#FBEBB0",
  danger: "#C0506A",
  dangerSoft: "#F8DDE4",
  green: "#3F8F6B",
  greenSoft: "#DFF3E9",
  blue: "#3979A8",
  blueSoft: "#E1EFF7",
  code: "#211B2C",
  gutter: "#EEE6D8",
  selection: "#DCD0F8",
};

// Uploaded from crates/ayame-cli/web/iris-watercolor.png into the target Figma file.
const IRIS_WATERCOLOR_IMAGE_HASH = "0adce43423e9ab57c07810b431dbee4a37f795e6";

function rgb(hex) {
  const n = parseInt(hex.replace("#", ""), 16);
  return { r: ((n >> 16) & 255) / 255, g: ((n >> 8) & 255) / 255, b: (n & 255) / 255 };
}

function paint(hex, opacity = 1) {
  return { type: "SOLID", color: rgb(hex), opacity };
}

function track(node) {
  created.push({ id: node.id, name: node.name, type: node.type });
  return node;
}

function add(parent, node) {
  parent.appendChild(node);
  return track(node);
}

function box(parent, name, x, y, w, h, fill, stroke = null, radius = 8, opacity = 1) {
  const node = figma.createRectangle();
  node.name = name;
  node.x = x;
  node.y = y;
  node.resize(w, h);
  node.fills = fill ? [paint(fill, opacity)] : [];
  node.strokes = stroke ? [paint(stroke)] : [];
  node.strokeWeight = stroke ? 1 : 0;
  node.cornerRadius = radius;
  return add(parent, node);
}

function oval(parent, name, x, y, w, h, fill, opacity = 1) {
  const node = figma.createEllipse();
  node.name = name;
  node.x = x;
  node.y = y;
  node.resize(w, h);
  node.fills = fill ? [paint(fill, opacity)] : [];
  node.strokes = [];
  return add(parent, node);
}

function frame(parent, name, x, y, w, h, fill, stroke = null, radius = 8) {
  const node = figma.createFrame();
  node.name = name;
  node.x = x;
  node.y = y;
  node.resize(w, h);
  node.fills = fill ? [paint(fill)] : [];
  node.strokes = stroke ? [paint(stroke)] : [];
  node.strokeWeight = stroke ? 1 : 0;
  node.cornerRadius = radius;
  node.clipsContent = false;
  return add(parent, node);
}

async function label(parent, name, text, x, y, w, h, size, color = C.ink, font = uiRegular, align = "LEFT") {
  const node = figma.createText();
  node.name = name;
  node.x = x;
  node.y = y;
  node.resize(w, h);
  node.fontName = font;
  node.fontSize = size;
  node.lineHeight = { unit: "PIXELS", value: Math.max(size + 5, Math.round(size * 1.35)) };
  node.textAlignHorizontal = align;
  node.textAlignVertical = "CENTER";
  node.characters = text;
  node.fills = [paint(color)];
  return add(parent, node);
}

async function button(parent, name, text, x, y, w, h, selected = false) {
  box(parent, `${name} bg`, x, y, w, h, selected ? C.accentSoft : C.panel, selected ? C.accent : C.border, 7);
  await label(parent, name, text, x + 10, y + 1, w - 20, h - 2, 12, selected ? C.accent2 : C.ink, uiMedium, "CENTER");
}

async function chip(parent, name, text, x, y, w, h, tone = "neutral") {
  const tones = {
    neutral: [C.panel2, C.border, C.muted],
    purple: [C.accentSoft, C.accent, C.accent2],
    gold: [C.goldSoft, C.gold, C.ink],
    green: [C.greenSoft, C.green, C.green],
    red: [C.dangerSoft, C.danger, C.danger],
    blue: [C.blueSoft, C.blue, C.blue],
  };
  const [fill, stroke, ink] = tones[tone] || tones.neutral;
  box(parent, `${name} bg`, x, y, w, h, fill, stroke, 999);
  await label(parent, name, text, x + 10, y, w - 20, h, 11, ink, uiMedium, "CENTER");
}

async function menuRow(parent, text, shortcut, x, y, w, disabled = false) {
  await label(parent, `${text} label`, text, x + 12, y, w - 102, 28, 12, disabled ? C.borderStrong : C.ink, uiRegular);
  await label(parent, `${text} shortcut`, shortcut, x + w - 92, y, 80, 28, 11, disabled ? C.borderStrong : C.muted, uiRegular, "RIGHT");
}

async function treeRow(parent, depth, name, meta, x, y, w, selected = false) {
  if (selected) box(parent, `${name} selected`, x + 8, y + 2, w - 16, 26, C.accentSoft, null, 6);
  icon(parent, `${name} icon`, depth === 0 ? ICONS.chevronDown : ICONS.fileText, x + 19 + depth * 16, y + 8, 12, selected ? C.accent2 : C.muted, 0.9);
  await label(parent, `${name} name`, name, x + 38 + depth * 16, y, w - 110 - depth * 16, 28, 12, selected ? C.accent2 : C.ink, selected ? uiMedium : uiRegular);
  if (meta) {
    const metaNode = await label(parent, `${name} meta`, meta, x + w - 74, y, 52, 28, 9, C.muted, uiRegular, "RIGHT");
    metaNode.opacity = 0.5;
  }
}

async function sidebarSection(parent, name, text, y) {
  await label(parent, name, text, 16, y, 128, 18, 10, C.muted, uiMedium);
}

async function explorerRow(parent, name, textValue, y, options = {}) {
  const x = options.x ?? 16;
  const w = options.w ?? 246;
  const selected = Boolean(options.selected);
  const muted = Boolean(options.muted);
  if (selected) box(parent, `${name} selected`, 8, y + 1, 254, 30, C.accentSoft, null, 6, 0.82);
  if (options.hover) box(parent, `${name} hover`, 8, y + 1, 254, 30, C.panel, C.border, 6, 0.92);
  if (options.chevron) icon(parent, `${name} chevron`, options.chevron === "▾" ? ICONS.chevronDown : ICONS.chevronRight, x + 1, y + 11, 11, muted ? C.muted : C.accent2);
  const rowIcon = options.icon === "folder" ? ICONS.folder : options.icon === "file" ? ICONS.fileText : null;
  const iconShift = rowIcon ? 19 : 0;
  if (rowIcon) icon(parent, `${name} type icon`, rowIcon, x + (options.chevron ? 18 : 0), y + 9, 13, selected ? C.accent2 : C.muted, muted ? 0.7 : 0.9);
  await label(parent, `${name} label`, textValue, x + (options.chevron ? 18 : 0) + iconShift, y, w - (options.chevron ? 64 : 44) - iconShift, 32, 12, selected ? C.accent2 : muted ? C.muted : C.ink, selected ? uiMedium : uiRegular);
  if (options.meta) {
    const metaNode = await label(parent, `${name} meta`, options.meta, 214, y, 40, 32, 9, C.muted, uiRegular, "RIGHT");
    metaNode.opacity = 0.5;
  }
}

async function editorLine(parent, no, text, y, options = {}) {
  const lineH = 26;
  const gutterW = 58;
  if (options.current) box(parent, `line ${no} current`, gutterW, y, 1084, lineH, C.accentSoft, null, 0, 0.55);
  if (options.deleted) box(parent, `line ${no} deleted`, gutterW, y, 1084, lineH, C.dangerSoft, null, 0);
  if (options.added) box(parent, `line ${no} added`, gutterW, y, 1084, lineH, C.greenSoft, null, 0);
  await label(parent, `line ${no} number`, String(no), 10, y, 36, lineH, 12, C.muted, monoRegular, "RIGHT");
  await label(parent, `line ${no} text`, text, 72, y, 960, lineH, 13, options.dim ? C.muted : C.code, monoRegular);
  if (options.searchX) box(parent, `line ${no} search mark`, options.searchX, y + 4, options.searchW || 72, 18, C.goldSoft, C.gold, 3);
  if (options.caretX) box(parent, `line ${no} caret`, options.caretX, y + 4, 2, 18, C.accent2, null, 0);
}

async function statusItem(parent, text, x, y, w) {
  box(parent, `${text} status`, x, y, w, 22, C.panel, C.border, 5);
  await label(parent, `${text} status label`, text, x + 8, y, w - 16, 22, 11, C.muted, uiRegular, "CENTER");
}

function separator(parent, x, y, h) {
  box(parent, "separator", x, y, 1, h, C.border, null, 0);
}

function brandLogoMark(parent, x, y, size) {
  const node = figma.createNodeFromSvg(`
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">
      <defs>
        <linearGradient id="ayameMark" x1="14" y1="8" x2="50" y2="56" gradientUnits="userSpaceOnUse">
          <stop offset="0" stop-color="#B6A0E8"/><stop offset=".48" stop-color="#8B70CF"/><stop offset="1" stop-color="#6F56B8"/>
        </linearGradient>
        <linearGradient id="ayameMarkDark" x1="26" y1="16" x2="43" y2="50" gradientUnits="userSpaceOnUse">
          <stop offset="0" stop-color="#8E76D0"/><stop offset="1" stop-color="#6048A8"/>
        </linearGradient>
      </defs>
      <g fill="url(#ayameMark)">
        <path d="M32 6c5.2 8.3 5.2 16.4 0 24.2C26.8 22.4 26.8 14.3 32 6Z"/>
        <path d="M18.4 16.6c6.8 2.3 11.6 7.5 13.2 15.5-7.5 1.2-13.4-2.7-17.2-11.6 1-1.8 2.3-3.1 4-3.9Z"/>
        <path d="M45.6 16.6c-6.8 2.3-11.6 7.5-13.2 15.5 7.5 1.2 13.4-2.7 17.2-11.6-1-1.8-2.3-3.1-4-3.9Z"/>
        <path d="M24.7 31.5C20.2 36 17.6 42.1 16.8 50c6.5-.8 11.4-4.4 14.1-10.6-1.1-2.8-3.1-5.4-6.2-7.9Z"/>
        <path d="M39.3 31.5C43.8 36 46.4 42.1 47.2 50c-6.5-.8-11.4-4.4-14.1-10.6 1.1-2.8 3.1-5.4 6.2-7.9Z"/>
      </g>
      <path d="M32 28.8c4.7 4.7 5.2 10.6 0 18.7-5.2-8.1-4.7-14 0-18.7Z" fill="url(#ayameMarkDark)"/>
    </svg>
  `);
  node.name = "brand logo mark";
  node.x = x;
  node.y = y;
  node.resize(size, size);
  return add(parent, node);
}

// Stroke icon set (Lucide-style outlines): crisp, consistent, avoids the
// "text pretending to be an icon" cheap look while staying simple.
const ICONS = {
  panelLeft: '<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M9 3v18"/>',
  undo: '<path d="M3 7v6h6"/><path d="M21 17a9 9 0 0 0-9-9 9 9 0 0 0-6 2.3L3 13"/>',
  redo: '<path d="M21 7v6h-6"/><path d="M3 17a9 9 0 0 1 9-9 9 9 0 0 1 6 2.3L21 13"/>',
  wrench: '<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>',
  search: '<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>',
  close: '<path d="M18 6 6 18"/><path d="m6 6 12 12"/>',
  folder: '<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>',
  fileText: '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/>',
  chevronDown: '<path d="m6 9 6 6 6-6"/>',
  chevronRight: '<path d="m9 18 6-6-6-6"/>',
  plus: '<path d="M5 12h14"/><path d="M12 5v14"/>',
};

function icon(parent, name, body, x, y, size, color = C.ink, opacity = 1) {
  const node = figma.createNodeFromSvg(`
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none"
      stroke="${color}" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${body}</svg>
  `);
  node.name = name;
  node.x = x;
  node.y = y;
  node.resize(size, size);
  node.opacity = opacity;
  return add(parent, node);
}

async function iconButton(parent, name, iconBody, text, x, y, w, h, selected = false) {
  box(parent, `${name} bg`, x, y, w, h, selected ? C.accentSoft : C.panel, selected ? C.accent : C.border, 7);
  icon(parent, `${name} icon`, iconBody, x + 10, y + (h - 16) / 2, 16, selected ? C.accent2 : C.ink);
  await label(parent, name, text, x + 32, y + 1, w - 40, h - 2, 12, selected ? C.accent2 : C.ink, uiMedium);
}

const fonts = await figma.listAvailableFontsAsync();
function findFont(families, styles) {
  for (const family of families) {
    for (const style of styles) {
      const found = fonts.find((f) => f.fontName.family === family && f.fontName.style === style);
      if (found) return found.fontName;
    }
  }
  return null;
}

const uiRegular = findFont(["Noto Sans JP", "Hiragino Kaku Gothic ProN", "Segoe UI", "Inter", "Arial"], ["Regular"]) || { family: "Inter", style: "Regular" };
const uiMedium = findFont([uiRegular.family], ["Medium", "Semi Bold", "Bold", "Regular"]) || uiRegular;
const monoRegular = findFont(["Consolas", "SF Mono", "Menlo", "Monaco", "Roboto Mono", "Inter"], ["Regular"]) || uiRegular;
const monoMedium = findFont([monoRegular.family], ["Medium", "Semi Bold", "Bold", "Regular"]) || monoRegular;
const uniqueFonts = [uiRegular, uiMedium, monoRegular, monoMedium]
  .filter((font, index, arr) => arr.findIndex((x) => `${x.family}/${x.style}` === `${font.family}/${font.style}`) === index);
await Promise.all(uniqueFonts.map((font) => figma.loadFontAsync(font)));

let page = figma.root.children.find((p) => p.name === "Ayame Editor Design");
if (!page) page = figma.createPage();
page.name = "Ayame Editor Design";
await figma.setCurrentPageAsync(page);

for (const child of [...page.children]) {
  if (child.name.startsWith(GENERATED_PREFIX)) child.remove();
}

const main = frame(page, "Ayame Editor - Main Workspace", 0, 0, 1440, 960, C.bg, C.borderStrong, 8);
main.clipsContent = true;
box(main, "window top", 0, 0, 1440, 34, C.panel, C.border, 0);
brandLogoMark(main, 7, -4, 42);
await label(main, "brand logo text", "Ayame Editor", 48, 0, 190, 34, 18, "#101014", uiMedium);
// Menus sit right-aligned in the title bar; ツール is a toolbar dropdown below.
const menuNames = ["ファイル", "編集", "選択", "表示"];
for (let i = 0; i < menuNames.length; i++) {
  await label(main, `menubar ${menuNames[i]}`, menuNames[i], 1136 + i * 72, 0, 64, 34, 12, C.ink, uiRegular, "CENTER");
}

// Toolbar stays minimal: explorer is only an open/close toggle, sort/diff/replace
// live behind the ツール entry, and search floats inside the editor viewport.
box(main, "toolbar", 0, 34, 1440, 52, C.panel2, C.border, 0);
await iconButton(main, "tool explorer", ICONS.panelLeft, "Explorer", 14, 44, 116, 32, true);
await iconButton(main, "tool undo", ICONS.undo, "Undo", 142, 44, 88, 32);
await iconButton(main, "tool redo", ICONS.redo, "Redo", 238, 44, 88, 32);
await iconButton(main, "tool tools", ICONS.wrench, "ツール", 334, 44, 96, 32);

box(main, "tabs", 0, 86, 1440, 36, C.panel, C.border, 0);
box(main, "active tab", 14, 92, 188, 30, C.panel, C.accent, 7);
icon(main, "tab active icon", ICONS.fileText, 26, 100, 13, C.accent2);
await label(main, "tab active label", "huge-file.log  *", 46, 92, 138, 30, 12, C.accent2, uiMedium);
box(main, "inactive tab", 208, 92, 184, 30, C.panel2, C.border, 7);
icon(main, "tab inactive icon", ICONS.fileText, 220, 100, 13, C.muted);
await label(main, "tab inactive label", "memo-20260702.txt", 240, 92, 134, 30, 12, C.muted, uiRegular);
box(main, "tab new bg", 402, 94, 34, 26, C.panel, C.border, 7);
icon(main, "tab new", ICONS.plus, 412, 100, 14, C.ink);

const contentY = 122;
const statusY = 928;
box(main, "sidebar", 0, contentY, 278, statusY - contentY, C.panel2, C.border, 0);
const irisPurpleWash = oval(main, "ambient iris wash purple", -150, contentY + 444, 430, 330, C.accent, 0.09);
irisPurpleWash.opacity = 0.62;
const irisGoldWash = oval(main, "ambient iris wash gold", 118, contentY + 582, 280, 230, C.gold, 0.08);
irisGoldWash.opacity = 0.5;
const irisSlot = figma.createRectangle();
irisSlot.name = "iris watercolor art";
irisSlot.x = -10;
irisSlot.y = contentY + 426;
irisSlot.resize(282, 330);
irisSlot.fills = [{ type: "IMAGE", imageHash: IRIS_WATERCOLOR_IMAGE_HASH, scaleMode: "FIT" }];
irisSlot.strokes = [];
irisSlot.opacity = 0.34;
add(main, irisSlot);
const irisMotif = figma.createNodeFromSvg(`
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 300 460">
    <path d="M84 470 C 52 404 78 360 112 330 C 142 304 124 262 94 242"
      fill="none" stroke="#7A5CC0" stroke-width="30" stroke-linecap="round" stroke-linejoin="round" opacity=".9"/>
    <path d="M172 470 C 158 416 190 390 174 348 C 162 318 186 296 172 268"
      fill="none" stroke="#3F8F6B" stroke-width="20" stroke-linecap="round" stroke-linejoin="round" opacity=".82"/>
    <path d="M214 300 C 216 314 220 318 234 320 C 220 322 216 326 214 340
      C 212 326 208 322 194 320 C 208 318 212 314 214 300 Z"
      fill="#C79A2E" opacity=".85"/>
    <circle cx="126" cy="286" r="6" fill="#C79A2E" opacity=".8"/>
  </svg>
`);
irisMotif.name = "iris paper-cut motif";
irisMotif.x = 0;
irisMotif.y = contentY + 334;
irisMotif.resize(278, 426);
irisMotif.opacity = 0.18;
add(main, irisMotif);
box(main, "sidebar title", 0, contentY, 278, 44, C.panel, C.border, 0);
await label(main, "sidebar title text", "エクスプローラー", 16, contentY, 150, 44, 12, C.ink, uiMedium);
await sidebarSection(main, "explorer project label", "PROJECT", contentY + 54);
box(main, "explorer root row", 8, contentY + 76, 254, 30, C.panel, C.border, 6, 0.92);
await explorerRow(main, "explorer root", "ayame-editor", contentY + 76, { chevron: "▾", x: 18, w: 236, icon: "folder" });
box(main, "explorer guide root", 30, contentY + 112, 1, 130, C.borderStrong, null, 0, 0.7);
await explorerRow(main, "explorer crates", "crates", contentY + 114, { chevron: "▾", x: 42, w: 210, icon: "folder" });
box(main, "explorer guide crates", 54, contentY + 150, 1, 54, C.borderStrong, null, 0, 0.55);
await explorerRow(main, "explorer ayame-core", "ayame-core", contentY + 148, { x: 66, w: 190, icon: "folder" });
await explorerRow(main, "explorer ayame-cli", "ayame-cli", contentY + 180, { x: 66, w: 190, icon: "folder" });
await explorerRow(main, "explorer docs", "docs", contentY + 216, { chevron: "▸", x: 42, w: 210, muted: true, icon: "folder" });
await sidebarSection(main, "explorer files label", "FILES", contentY + 264);
await explorerRow(main, "explorer huge-file", "huge-file.log", contentY + 288, { x: 28, w: 222, selected: true, meta: "9.8TB", icon: "file" });
await explorerRow(main, "explorer compare-file", "compare.sorted.csv", contentY + 322, { x: 28, w: 222, meta: "4.1TB", icon: "file" });

const editor = frame(main, "editor viewport", 278, contentY, 1162, statusY - contentY, C.panel, C.border, 0);
editor.clipsContent = true;
box(editor, "ruler", 0, 0, 1162, 32, C.panel2, C.border, 0);
await label(editor, "ruler text", "1        10        20        30        40        50        60        70        80        90       100", 72, 0, 870, 32, 11, C.muted, monoRegular);
box(editor, "gutter", 0, 32, 58, 774, C.gutter, C.border, 0);
await editorLine(editor, 1, "2026-07-02T09:41:15Z INFO sort: phase=scan rows=10,000,000,000", 44, { searchX: 572, searchW: 52 });
await editorLine(editor, 2, "user_id,region,amount,status,updated_at", 70, { searchX: 304, searchW: 48 });
await editorLine(editor, 3, "0001042,ap-northeast-1,1200,active,2026-07-02T09:41:16Z", 96, { searchX: 356, searchW: 48 });
await editorLine(editor, 4, "0001043,us-east-1,980,pending,2026-07-02T09:41:17Z", 122);
await editorLine(editor, 5, "0001044,eu-west-1,1750,active,2026-07-02T09:41:18Z", 148, { current: true, caretX: 520, searchX: 330, searchW: 48 });
box(editor, "range selection", 300, 174, 420, 24, C.selection, C.accent, 3, 0.9);
await editorLine(editor, 6, "0001045,ap-northeast-1,2400,active,2026-07-02T09:41:19Z", 174);
const colSel = box(editor, "rectangle selection", 330, 205, 182, 104, C.accentSoft, C.accent, 3, 0.45);
colSel.dashPattern = [6, 4];
await editorLine(editor, 7, "0001046,ap-southeast-2,510,disabled,2026-07-02T09:41:20Z", 200);
await editorLine(editor, 8, "0001047,ap-southeast-2,510,active,2026-07-02T09:41:21Z", 226, { searchX: 354, searchW: 48 });
await editorLine(editor, 9, "0001048,us-west-2,1120,active,2026-07-02T09:41:22Z", 252, { searchX: 306, searchW: 48 });
await editorLine(editor, 10, "... virtualized rows, mmap window 18,432-18,688, edit overlay dirty pages 3", 278, { dim: true });
box(editor, "minimap", 1132, 44, 16, 720, C.panel2, C.border, 8);
box(editor, "minimap window", 1134, 210, 12, 84, C.accent, null, 6, 0.55);
// Floating in-editor search (VS Code style), overlaying the top-right of the viewport.
box(editor, "editor search panel", 764, 42, 356, 40, C.panel, C.borderStrong, 8);
icon(editor, "editor search icon", ICONS.search, 778, 55, 14, C.muted);
await label(editor, "editor search text", "status=active", 800, 46, 136, 32, 12, C.ink, uiRegular);
await chip(editor, "case toggle", "Aa", 946, 52, 34, 20, "purple");
await chip(editor, "regex toggle", ".*", 984, 52, 34, 20, "neutral");
await label(editor, "editor search count", "3/5", 1024, 46, 34, 32, 11, C.muted, uiRegular, "CENTER");
icon(editor, "editor search close", ICONS.close, 1066, 56, 13, C.muted);

box(main, "status bar", 0, statusY, 1440, 32, C.panel2, C.border, 0);
await statusItem(main, "Ln 18,452 Col 37", 14, statusY + 5, 132);
await statusItem(main, "UTF-8", 154, statusY + 5, 66);
await statusItem(main, "LF", 228, statusY + 5, 46);
await statusItem(main, "DIRTY", 282, statusY + 5, 62);
await statusItem(main, "index ready", 352, statusY + 5, 92);
await label(main, "status right", "autosave off", 1290, statusY + 5, 120, 22, 11, C.muted, uiRegular, "RIGHT");

const menus = frame(page, "Ayame Editor - Complete Menu Map", 0, 1040, 1440, 560, C.bg, C.borderStrong, 8);
await label(menus, "menu map title", "Ayame Editor complete menu map", 32, 20, 520, 34, 24, C.ink, uiMedium);
await label(menus, "menu map subtitle", "Native text editor actions: open, edit, undo, range selection, rectangle selection, replace, sort, compare, save.", 34, 56, 940, 26, 13, C.muted, uiRegular);
// Mirrors the web UI exactly: 4 right-aligned menubar menus + the toolbar
// ツール ▾ dropdown. Gestures (Alt+Drag) are listed once, in 選択.
const menuGroups = [
  ["ファイル", [["新規テキスト", "Ctrl+N"], ["開く...", "Ctrl+O"], ["保存", "Ctrl+S"], ["別名で保存...", "Ctrl+Shift+S"]]],
  ["編集", [["元に戻す", "Ctrl+Z"], ["やり直す", "Ctrl+Y"], ["検索", "Ctrl+F"], ["行へ移動", "Ctrl+G"]]],
  ["選択", [["すべて選択", "Ctrl+A"], ["コピー", "Ctrl+C"], ["切り取り", "Ctrl+X"], ["矩形選択", "Alt+Drag"]]],
  ["表示", [["エクスプローラー", "Ctrl+B"], ["検索バー", "Ctrl+F"], ["設定", ""]]],
  ["ツール ▾", [["ソート", ""], ["置換して保存", ""], ["2ファイル差分", ""], ["大文字化して保存", ""], ["小文字化して保存", ""], ["キー設定", ""]]],
];
for (let i = 0; i < menuGroups.length; i++) {
  const x = 32 + i * 276;
  box(menus, `${menuGroups[i][0]} panel`, x, 110, 246, 352, C.panel, C.border, 8);
  await label(menus, `${menuGroups[i][0]} title`, menuGroups[i][0], x + 18, 126, 150, 26, 16, C.ink, uiMedium);
  for (let j = 0; j < menuGroups[i][1].length; j++) {
    const [title, key] = menuGroups[i][1][j];
    await menuRow(menus, title, key, x + 12, 166 + j * 38, 222);
  }
}
await chip(menus, "command palette chip", "Command palette mirrors every menu item and user keymap override", 32, 492, 470, 28, "purple");
await chip(menus, "diff chip", "Diff view inspired by delta / WinMerge", 520, 492, 260, 28, "blue");
await chip(menus, "sakura chip", "Huge-file editing model: mmap read view + sparse edit overlay", 798, 492, 392, 28, "gold");

const dialogs = frame(page, "Ayame Editor - Dialogs and Detailed States", 1480, 0, 1440, 1780, C.bg, C.borderStrong, 8);
await label(dialogs, "dialogs title", "Dialogs and detailed states", 32, 20, 440, 34, 24, C.ink, uiMedium);

const openDlg = frame(dialogs, "open file dialog", 40, 80, 620, 440, C.panel, C.borderStrong, 8);
await label(openDlg, "open title", "ファイルを開く", 24, 18, 180, 32, 18, C.ink, uiMedium);
box(openDlg, "path input", 24, 66, 430, 36, C.bg, C.border, 7);
await label(openDlg, "path input text", "/data/logs/${yyyy}${mm}${dd}/huge-file.log", 38, 66, 390, 36, 12, C.ink, monoRegular);
await button(openDlg, "open browse", "参照", 466, 66, 58, 36);
await button(openDlg, "open submit", "開く", 532, 66, 58, 36, true);
await chip(openDlg, "open template chip", "default name templates: memo-${date}.txt, scratch-${hhmm}.log", 24, 116, 360, 24, "blue");
for (let i = 0; i < 7; i++) {
  const rows = ["../", "20260702/", "huge-file.log", "compare.sorted.csv", "replace-rules.tsv", "bench-300m.log", "untitled-${date}.txt"];
  box(openDlg, `open row ${i}`, 24, 156 + i * 34, 572, 30, i === 2 ? C.accentSoft : C.panel2, i === 2 ? C.accent : C.border, 5);
  await label(openDlg, `open row ${i} label`, rows[i], 42, 156 + i * 34, 400, 30, 12, i === 2 ? C.accent2 : C.ink, i === 2 ? uiMedium : uiRegular);
  await label(openDlg, `open row ${i} meta`, i < 2 ? "folder" : ["", "", "9.8TB", "4.1TB", "12KB", "18GB", "new"][i], 470, 156 + i * 34, 100, 30, 11, C.muted, uiRegular, "RIGHT");
}

const settingsDlg = frame(dialogs, "settings dialog", 700, 80, 680, 520, C.panel, C.borderStrong, 8);
await label(settingsDlg, "settings title", "設定", 24, 18, 120, 32, 18, C.ink, uiMedium);
const settingsRows = [
  ["テーマ", "iris-light / sumi-light / mono-paper"],
  ["背景画像", "iris-watercolor.png / off"],
  ["フォント", "UI: system, editor: Consolas"],
  ["行番号", "on / relative / off"],
  ["検索", "case, word, regex, selection only"],
  ["保存", "atomic write, backup, newline policy"],
  ["巨大ファイル", "mmap window, index shards, edit overlay"],
  ["キー設定", "VS Code / Sakura / custom JSON"],
];
for (let i = 0; i < settingsRows.length; i++) {
  const y = 70 + i * 48;
  await label(settingsDlg, `settings ${i} key`, settingsRows[i][0], 26, y, 130, 32, 12, C.ink, uiMedium);
  box(settingsDlg, `settings ${i} input`, 162, y, 480, 32, C.bg, C.border, 7);
  await label(settingsDlg, `settings ${i} value`, settingsRows[i][1], 176, y, 430, 32, 12, C.muted, uiRegular);
}
await button(settingsDlg, "settings cancel", "キャンセル", 440, 460, 88, 34);
await button(settingsDlg, "settings save", "保存", 542, 460, 100, 34, true);

const diffDlg = frame(dialogs, "diff dialog", 40, 640, 1340, 620, C.panel, C.borderStrong, 8);
await label(diffDlg, "diff title", "2ファイル差分: huge-file.log ↔ compare.sorted.csv", 24, 18, 560, 32, 18, C.ink, uiMedium);
await chip(diffDlg, "diff dirty chip", "2ファイル", 620, 24, 112, 24, "gold");
await chip(diffDlg, "diff summary chip", "+2  -2  3 hunks", 744, 24, 112, 24, "purple");
await button(diffDlg, "diff prev", "前ハンク", 1052, 22, 72, 32);
await button(diffDlg, "diff next", "次ハンク", 1134, 22, 72, 32);
await button(diffDlg, "diff apply", "左右入替", 1216, 22, 100, 32, true);
box(diffDlg, "diff left header", 24, 72, 630, 34, C.panel2, C.border, 6);
box(diffDlg, "diff right header", 686, 72, 630, 34, C.panel2, C.border, 6);
await label(diffDlg, "diff left header text", "左ファイル: huge-file.log", 42, 72, 180, 34, 12, C.ink, uiMedium);
await label(diffDlg, "diff right header text", "右ファイル: compare.sorted.csv", 704, 72, 180, 34, 12, C.ink, uiMedium);
const diffRows = [
  [" 1042  active    1200", " 1042  active    1200", ""],
  ["-1043  pending    980", "+1043  active     980", "change"],
  [" 1044  active    1750", " 1044  active    1750", ""],
  ["-1045  active    2400", "+1045  disabled  2400", "change"],
  [" 1046  disabled   510", " 1046  disabled   510", ""],
  ["+1047  active     510", " 1047  active     510", "add"],
];
for (let i = 0; i < diffRows.length; i++) {
  const y = 124 + i * 42;
  const tone = diffRows[i][2];
  const leftFill = tone === "change" ? C.dangerSoft : tone === "add" ? C.greenSoft : C.bg;
  const rightFill = tone === "change" ? C.greenSoft : C.bg;
  box(diffDlg, `diff left row ${i}`, 24, y, 630, 36, leftFill, C.border, 4);
  box(diffDlg, `diff right row ${i}`, 686, y, 630, 36, rightFill, C.border, 4);
  await label(diffDlg, `diff left ${i}`, diffRows[i][0], 44, y, 540, 36, 13, C.code, monoRegular);
  await label(diffDlg, `diff right ${i}`, diffRows[i][1], 706, y, 540, 36, 13, C.code, monoRegular);
}
box(diffDlg, "diff mini map", 24, 430, 1292, 58, C.panel2, C.border, 6);
await label(diffDlg, "diff mini map text", "Hunk map:  |-- equal --| change |-- equal --| change | add |  virtualized diff slices keep memory bounded", 44, 430, 1010, 58, 12, C.muted, uiRegular);
await chip(diffDlg, "diff engine chip", "Two-file streaming compare for huge text files", 44, 520, 306, 28, "green");
await chip(diffDlg, "diff view chip", "Side-by-side file diff with hunk navigation", 366, 520, 330, 28, "blue");

const keymapDlg = frame(dialogs, "keymap dialog", 40, 1300, 760, 420, C.panel, C.borderStrong, 8);
await label(keymapDlg, "keymap title", "キー設定", 24, 18, 180, 32, 18, C.ink, uiMedium);
const keyRows = [
  ["file.open", "Ctrl+O", "開く..."],
  ["file.save", "Ctrl+S", "保存"],
  ["edit.undo", "Ctrl+Z", "元に戻す"],
  ["edit.redo", "Ctrl+Y", "やり直す"],
  ["selection.rectangle", "Alt+Drag", "矩形選択"],
  ["tools.sort", "", "ソート"],
  ["tools.diff", "", "2ファイル差分"],
  ["tools.case.upper", "", "大文字化して保存"],
];
for (let i = 0; i < keyRows.length; i++) {
  const y = 72 + i * 38;
  box(keymapDlg, `keymap row ${i}`, 24, y, 712, 32, i % 2 ? C.panel2 : C.bg, C.border, 5);
  await label(keymapDlg, `keymap command ${i}`, keyRows[i][0], 40, y, 230, 32, 12, C.code, monoRegular);
  await label(keymapDlg, `keymap key ${i}`, keyRows[i][1] || "未割当", 286, y, 130, 32, 12, keyRows[i][1] ? C.accent2 : C.muted, uiMedium);
  await label(keymapDlg, `keymap label ${i}`, keyRows[i][2], 432, y, 220, 32, 12, C.ink, uiRegular);
}

const caseDlg = frame(dialogs, "case replace popover", 840, 1300, 500, 420, C.panel, C.borderStrong, 8);
await label(caseDlg, "case title", "変換と置換", 24, 18, 180, 32, 18, C.ink, uiMedium);
await label(caseDlg, "case desc", "Selection-aware operations for range and rectangle selections.", 24, 54, 360, 26, 12, C.muted, uiRegular);
for (let i = 0; i < 7; i++) {
  const labels = ["大文字化", "小文字化", "Title Case", "snake_case", "camelCase", "選択範囲置換", "正規表現置換"];
  await button(caseDlg, `case ${i}`, labels[i], 24 + (i % 2) * 228, 104 + Math.floor(i / 2) * 58, 204, 38, i === 5);
}

const components = frame(page, "Ayame Editor - Components and Tokens", 0, 1640, 1440, 760, C.bg, C.borderStrong, 8);
await label(components, "components title", "Components and tokens", 32, 20, 360, 34, 24, C.ink, uiMedium);
await label(components, "components subtitle", "Reusable pieces for the native editor UI: compact controls, dense tables, stable dimensions, readable huge-file views.", 34, 56, 900, 26, 13, C.muted, uiRegular);
const swatches = [
  ["bg", C.bg],
  ["panel", C.panel],
  ["ink", C.ink],
  ["accent", C.accent],
  ["gold", C.gold],
  ["green", C.green],
  ["danger", C.danger],
  ["blue", C.blue],
];
for (let i = 0; i < swatches.length; i++) {
  const x = 36 + i * 120;
  box(components, `swatch ${swatches[i][0]}`, x, 110, 72, 54, swatches[i][1], C.borderStrong, 8);
  await label(components, `swatch label ${swatches[i][0]}`, swatches[i][0], x, 172, 72, 22, 11, C.muted, uiRegular, "CENTER");
}
await label(components, "buttons label", "Toolbar buttons", 36, 230, 220, 28, 16, C.ink, uiMedium);
await button(components, "component button normal", "Normal", 36, 274, 92, 34);
await button(components, "component button active", "Active", 144, 274, 92, 34, true);
await chip(components, "component chip neutral", "Neutral", 252, 278, 84, 26);
await chip(components, "component chip warning", "Dirty", 352, 278, 72, 26, "gold");
await label(components, "tabs label", "Tabs", 36, 348, 140, 28, 16, C.ink, uiMedium);
box(components, "component tab active", 36, 392, 180, 34, C.panel, C.accent, 7);
await label(components, "component tab active text", "active.log  *", 52, 392, 120, 34, 12, C.accent2, uiMedium);
box(components, "component tab inactive", 228, 392, 180, 34, C.panel2, C.border, 7);
await label(components, "component tab inactive text", "compare.csv", 244, 392, 120, 34, 12, C.muted, uiRegular);
await label(components, "diff label", "Diff row states", 540, 230, 220, 28, 16, C.ink, uiMedium);
box(components, "component diff delete", 540, 274, 420, 34, C.dangerSoft, C.danger, 5);
await label(components, "component diff delete text", "- pending -> active", 558, 274, 240, 34, 12, C.code, monoRegular);
box(components, "component diff add", 540, 318, 420, 34, C.greenSoft, C.green, 5);
await label(components, "component diff add text", "+ disabled -> active", 558, 318, 240, 34, 12, C.code, monoRegular);
await label(components, "tree label", "Explorer rows", 540, 392, 220, 28, 16, C.ink, uiMedium);
await treeRow(components, 0, "workspace", "", 520, 432, 430, true);
await treeRow(components, 1, "huge-file.log", "9.8TB", 520, 466, 430);
await treeRow(components, 1, "compare.sorted.csv", "4.1TB", 520, 500, 430);
await label(components, "behavior label", "Required editor behavior", 1010, 230, 260, 28, 16, C.ink, uiMedium);
const reqs = ["Open/edit/save native files", "Undo / redo", "Range and rectangle selection", "Selection replace", "Sort and compare", "Case conversion", "Cross-platform: Linux / macOS / Windows"];
for (let i = 0; i < reqs.length; i++) {
  await chip(components, `requirement ${i}`, reqs[i], 1010, 274 + i * 42, 330, 28, i % 3 === 0 ? "purple" : i % 3 === 1 ? "blue" : "green");
}

for (const node of [main, menus, dialogs, components]) {
  node.setSharedPluginData("ayame_editor", "generatedAt", "2026-07-02");
  node.setSharedPluginData("ayame_editor", "source", "design/figma/ayame-editor-design.use-figma.js");
}

figma.viewport.scrollAndZoomIntoView([main, dialogs, menus, components]);

return {
  ok: true,
  pageId: page.id,
  frames: {
    main: main.id,
    menus: menus.id,
    dialogs: dialogs.id,
    components: components.id,
  },
  fonts: {
    uiRegular,
    uiMedium,
    monoRegular,
    monoMedium,
  },
  createdCount: created.length,
  created: created.slice(0, 40),
};
