// Bounded, explicit character/raw-byte inspection UI (#247).
import { apiPost } from "./api.js";
import { $, el, setModalOpen } from "./dom.js";
import { applyRange } from "./edits.js";
import { focusEditor } from "./editor.js";
import { formatPreservedColor, hexToByteEscapes, joinedHex } from "./inspector-model.js";
import { serverMessage, t, type MessageKey } from "./i18n.js";
import { registerModal } from "./modal-state.js";
import { flashCount } from "./notifications.js";
import { selRange } from "./selection-model.js";
import { state, type Point } from "./state.js";
import type {
  ClusterInfo,
  InspectRequest,
  InspectResponse,
  ParseEscapeRequest,
  ParseEscapeResponse,
} from "./types/api.js";

type InspectorContext = {
  docGeneration: number;
  editGeneration: number;
  start: Point;
  end: Point;
};

const DIAGNOSTIC_KEYS: Record<string, MessageKey> = {
  "bidi-control": "inspect.diag.bidiControl",
  "zero-width-space": "inspect.diag.zeroWidthSpace",
  "zero-width-non-joiner": "inspect.diag.zeroWidthNonJoiner",
  "zero-width-joiner": "inspect.diag.zeroWidthJoiner",
  "non-breaking-space": "inspect.diag.nonBreakingSpace",
  "soft-hyphen": "inspect.diag.softHyphen",
  "variation-selector": "inspect.diag.variationSelector",
  "replacement-character": "inspect.diag.replacementCharacter",
  "control-character": "inspect.diag.controlCharacter",
  unrepresentable: "inspect.diag.unrepresentable",
  "decode-mismatch": "inspect.diag.decodeMismatch",
  "line-ending": "inspect.diag.lineEnding",
  "mixed-script-possible-confusable": "inspect.diag.mixedScript",
  "inspection-truncated": "inspect.diag.truncated",
};

let context: InspectorContext | null = null;
let response: InspectResponse | null = null;
let parsed: ParseEscapeResponse | null = null;
let requestSerial = 0;
let requestController: AbortController | null = null;

function diagnosticLabel(code: string) {
  const key = DIAGNOSTIC_KEYS[code];
  return key ? t(key) : code;
}

function contextCurrent(candidate = context) {
  return (
    !!candidate &&
    state.doc.generation === candidate.docGeneration &&
    state.caret.editGeneration === candidate.editGeneration
  );
}

function metadata(label: string, value: string) {
  const row = el("div", "inspect-meta-row");
  row.append(el("dt", "inspect-meta-label", label), el("dd", "inspect-meta-value", value));
  return row;
}

function diagnostics(codes: string[]) {
  const list = el("div", "inspect-diagnostics");
  for (const code of new Set(codes)) {
    const badge = el("span", "inspect-diagnostic", diagnosticLabel(code));
    badge.dataset.diagnostic = code;
    list.append(badge);
  }
  return list;
}

function renderCluster(cluster: ClusterInfo) {
  const card = el("article", "inspect-cluster");
  const heading = el("div", "inspect-cluster-head");
  heading.append(
    el("span", "inspect-glyph", cluster.display),
    el(
      "span",
      "inspect-position",
      t("inspect.position", { line: cluster.line + 1, col: cluster.col + 1 }),
    ),
  );
  card.append(heading);
  const facts = el("dl", "inspect-meta");
  facts.append(
    metadata(t("inspect.codePoints"), cluster.scalars.map((item) => item.code_point).join(" ")),
    metadata(t("inspect.cellWidth"), `${cluster.cell_width} / CJK ${cluster.cell_width_cjk}`),
    metadata(t("inspect.utf8Hex"), cluster.utf8_hex || "—"),
    metadata(t("inspect.utf16Hex"), cluster.utf16_hex || "—"),
    metadata(
      t("inspect.byteOffset"),
      cluster.original_byte_offset == null
        ? t("inspect.overlay")
        : String(cluster.original_byte_offset),
    ),
    metadata(t("inspect.rawHex"), cluster.raw_hex ?? t("inspect.unavailable")),
    metadata(
      t("inspect.originalEncodingHex"),
      cluster.original_encoding_hex ?? t("inspect.unrepresentable"),
    ),
  );
  card.append(facts);
  const scalarList = el("div", "inspect-scalars");
  for (const scalar of cluster.scalars) {
    const scalarRow = el("div", "inspect-scalar");
    scalarRow.append(
      el(
        "div",
        "inspect-scalar-name",
        `${scalar.code_point} · ${scalar.name || t("inspect.unassigned")}`,
      ),
      el(
        "div",
        "inspect-scalar-facts",
        `${scalar.general_category} · ${scalar.script} · Bidi ${scalar.bidi_class} · EAW ${scalar.east_asian_width}`,
      ),
    );
    if (scalar.diagnostics.length) scalarRow.append(diagnostics(scalar.diagnostics));
    scalarList.append(scalarRow);
  }
  card.append(scalarList);
  if (cluster.diagnostics.length) card.append(diagnostics(cluster.diagnostics));
  return card;
}

function copyData(data: string | null, buttonElement: HTMLButtonElement) {
  buttonElement.disabled = data == null;
  buttonElement.onclick =
    data == null
      ? null
      : async () => {
          try {
            await navigator.clipboard.writeText(data);
            flashCount(t("inspect.copied"));
          } catch {
            flashCount(t("inspect.copyError"), "error");
          }
        };
}

function configureCopyButtons(result: InspectResponse) {
  const scalars = result.clusters.flatMap((cluster) => cluster.scalars);
  const rawHex = joinedHex(result.clusters.map((cluster) => cluster.raw_hex));
  copyData(
    result.clusters.map((cluster) => cluster.text).join(""),
    $<HTMLButtonElement>("inspect-copy-text"),
  );
  copyData(
    scalars.map((scalar) => scalar.code_point).join(" "),
    $<HTMLButtonElement>("inspect-copy-codepoints"),
  );
  copyData(
    scalars.map((scalar) => `\\u{${scalar.code_point.slice(2)}}`).join(""),
    $<HTMLButtonElement>("inspect-copy-escapes"),
  );
  copyData(
    joinedHex(result.clusters.map((cluster) => cluster.utf8_hex)),
    $<HTMLButtonElement>("inspect-copy-utf8"),
  );
  copyData(
    joinedHex(result.clusters.map((cluster) => cluster.original_encoding_hex)),
    $<HTMLButtonElement>("inspect-copy-encoded"),
  );
  copyData(rawHex, $<HTMLButtonElement>("inspect-copy-raw"));
  copyData(hexToByteEscapes(rawHex), $<HTMLButtonElement>("inspect-copy-byte-escapes"));
}

function renderColor(result: InspectResponse) {
  const section = $("inspect-color-section");
  const color = result.color;
  section.classList.toggle("hidden", !color);
  if (!color) return;
  const picker = $<HTMLInputElement>("inspect-color-picker");
  const alpha = $<HTMLInputElement>("inspect-color-alpha");
  picker.value = color.rgb_hex;
  alpha.value = String(color.alpha);
  const alphaValue = () =>
    Number.isFinite(alpha.valueAsNumber)
      ? Math.max(0, Math.min(255, Math.round(alpha.valueAsNumber)))
      : color.alpha;
  const update = () => {
    const next = formatPreservedColor(
      picker.value,
      alphaValue(),
      color.format,
      color.uppercase,
      color.prefix,
    );
    $("inspect-color-preview").textContent = next.literal;
    $("inspect-color-swatch").style.backgroundColor = `${picker.value}${alphaValue()
      .toString(16)
      .padStart(2, "0")}`;
  };
  picker.oninput = update;
  alpha.oninput = update;
  update();
  $<HTMLButtonElement>("inspect-color-apply").onclick = async () => {
    if (!contextCurrent()) {
      flashCount(t("inspect.stale"), "error");
      return;
    }
    const next = formatPreservedColor(
      picker.value,
      alphaValue(),
      color.format,
      color.uppercase,
      color.prefix,
    );
    await applyRange(color.line, color.start_col, color.line, color.end_col, next.literal);
    hideInspector();
  };
}

function replacementTarget(result: InspectResponse) {
  if (!context || result.summary.truncated) return null;
  if (context.start.line !== context.end.line || context.start.col !== context.end.col) {
    return { start: context.start, end: context.end };
  }
  const cluster = result.clusters[0];
  if (!cluster || cluster.kind === "eol") return null;
  return {
    start: { line: cluster.line, col: cluster.col },
    end: { line: cluster.line, col: cluster.end_col },
  };
}

async function parseExpression() {
  const expression = $<HTMLInputElement>("inspect-expression").value;
  const preview = $("inspect-expression-preview");
  const replaceButton = $<HTMLButtonElement>("inspect-expression-replace");
  parsed = null;
  replaceButton.disabled = true;
  if (!expression) {
    preview.textContent = t("inspect.expressionHint");
    return;
  }
  const serial = ++requestSerial;
  requestController?.abort();
  requestController = new AbortController();
  try {
    const result = await apiPost<ParseEscapeResponse, ParseEscapeRequest>(
      "/api/inspect/parse",
      { expression },
      requestController.signal,
    );
    if (serial !== requestSerial || !contextCurrent()) return;
    parsed = result;
    preview.textContent = `${result.code_points} · ${JSON.stringify(result.text)} · ${result.original_encoding_hex ?? t("inspect.unrepresentable")}`;
    if (result.diagnostics.length) preview.append(diagnostics(result.diagnostics));
    replaceButton.disabled = !result.representable || !response || !replacementTarget(response);
  } catch (error) {
    if ((error as Error).name === "AbortError") return;
    preview.textContent = serverMessage(error);
  }
}

function renderResponse(result: InspectResponse) {
  response = result;
  $("inspect-summary").textContent = t("inspect.summary", {
    encoding: result.encoding,
    graphemes: result.summary.grapheme_count,
    scalars: result.summary.scalar_count,
    utf8: result.summary.utf8_bytes,
    utf16: result.summary.utf16_units,
  });
  $("inspect-file-meta").textContent = result.bom_bytes
    ? `${t("inspect.bom")}: ${result.bom_hex} (${result.bom_bytes} bytes)`
    : t("inspect.noBom");
  const diagnosticRoot = $("inspect-document-diagnostics");
  diagnosticRoot.replaceChildren();
  if (result.diagnostics.length) diagnosticRoot.append(diagnostics(result.diagnostics));
  const list = $("inspect-clusters");
  list.replaceChildren(...result.clusters.map(renderCluster));
  configureCopyButtons(result);
  renderColor(result);
  $<HTMLButtonElement>("inspect-expression-replace").disabled = true;
}

export async function showInspector() {
  if (!state.doc.stat?.open) return;
  if (state.caret.selection?.rect || state.caret.extraCursors.length) {
    flashCount(t("inspect.singleRangeOnly"), "error");
    return;
  }
  const selection = selRange();
  const start = selection?.start ?? state.caret.position;
  const end = selection?.end ?? state.caret.position;
  context = {
    docGeneration: state.doc.generation,
    editGeneration: state.caret.editGeneration,
    start: { ...start },
    end: { ...end },
  };
  response = null;
  parsed = null;
  $("inspect-summary").textContent = t("inspect.loading");
  $("inspect-clusters").replaceChildren();
  $("inspect-document-diagnostics").replaceChildren();
  $("inspect-file-meta").textContent = "";
  $("inspect-color-section").classList.add("hidden");
  $<HTMLInputElement>("inspect-expression").value = "";
  $("inspect-expression-preview").textContent = t("inspect.expressionHint");
  setModalOpen($("inspect-modal"), true);
  $<HTMLButtonElement>("inspect-close").focus();

  const serial = ++requestSerial;
  requestController?.abort();
  requestController = new AbortController();
  const request: InspectRequest = { start, end };
  try {
    const result = await apiPost<InspectResponse, InspectRequest>(
      "/api/inspect",
      request,
      requestController.signal,
    );
    if (serial !== requestSerial || !contextCurrent()) return;
    renderResponse(result);
  } catch (error) {
    if ((error as Error).name === "AbortError") return;
    $("inspect-summary").textContent = t("inspect.error", {
      msg: serverMessage(error),
    });
  }
}

export function hideInspector() {
  requestSerial++;
  requestController?.abort();
  requestController = null;
  context = null;
  response = null;
  parsed = null;
  setModalOpen($("inspect-modal"), false);
  focusEditor();
}

export function initInspector() {
  registerModal("inspect-modal", { onClose: hideInspector, closeOnBackdrop: true });
  $<HTMLButtonElement>("inspect-close").addEventListener("click", hideInspector);
  $<HTMLButtonElement>("inspect-expression-parse").addEventListener("click", () => {
    void parseExpression();
  });
  $<HTMLInputElement>("inspect-expression").addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      void parseExpression();
    }
  });
  $<HTMLButtonElement>("inspect-expression-replace").addEventListener("click", async () => {
    if (!parsed || !parsed.representable || !response || !contextCurrent()) {
      flashCount(t("inspect.stale"), "error");
      return;
    }
    const target = replacementTarget(response);
    if (!target) return;
    await applyRange(
      target.start.line,
      target.start.col,
      target.end.line,
      target.end.col,
      parsed.text,
    );
    hideInspector();
  });
}
