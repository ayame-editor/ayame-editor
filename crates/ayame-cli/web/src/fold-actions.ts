import { flashCount } from "./notifications.js";
import { hideLoading, setLoadingDetail, showLoading } from "./dialogs.js";
import { moveCaret, render, scheduleRender } from "./editor.js";
import {
  activeFoldMap,
  collapseBlock,
  collapsedStructureBlocks,
  currentStructureBlock,
  expandFoldStartingAt,
  setCurrentStructureBlock,
  setFoldSummary,
  toggleBlock,
} from "./fold-state.js";
import { state } from "./state.js";
import {
  findMatchingBrace,
  findSiblingBlock,
  findStructureBlock,
  lineMayStartStructure,
  structureBlockStartingAt,
  type StructureLineSource,
} from "./structure.js";
import { LazyStructureSource, StructureScanCanceled } from "./structure-source.js";
import { resolveSyntaxScheme, schemeDefinition, type StructureProviderId } from "./syntax.js";
import { t } from "./i18n.js";
import type { StructureBlock } from "./fold-map.js";
import { api, type MarkerRangeCountsResponse } from "./api.js";

const FOREGROUND_SCAN_LINES = 512;
let scanSequence = 0;
let activeController: AbortController | null = null;
let activeOverlaySequence = 0;

export function activeStructureProvider(): StructureProviderId | null {
  const path = state.doc.stat?.path || "";
  const selection = state.syntax.overrides[path] || "auto";
  const scheme = resolveSyntaxScheme(path, selection, state.syntax.mappings);
  return scheme ? schemeDefinition(scheme).structure || null : null;
}

function localSummary(block: StructureBlock) {
  const inBody = (line: number) => line > block.start && line <= block.end;
  return {
    bookmarks: [...state.markers.bookmarks].filter(inBody).length,
    changes: new Set([...state.markers.changeSaved, ...state.markers.changeUnsaved].filter(inBody))
      .size,
    matches: (state.search.hits || []).filter((hit) => inBody(hit.line)).length,
  };
}

async function refreshSummary(block: StructureBlock) {
  const generation = state.doc.generation;
  const local = localSummary(block);
  try {
    const counts = await api<MarkerRangeCountsResponse>(
      `/api/markers/range-counts?start=${block.start + 1}&end=${block.end + 1}`,
    );
    if (generation !== state.doc.generation || !activeFoldMap().collapsedAt(block.start)) return;
    setFoldSummary(block.start, {
      bookmarks: counts.bookmarks,
      changes: counts.change_saved + counts.change_unsaved,
      matches: counts.search_rules + local.matches,
    });
    scheduleRender();
  } catch (error) {
    // The fold itself is useful even if an auxiliary badge count fails.
    console.error("fold summary failed", error);
  }
}

async function runStructureScan<T>(work: (source: StructureLineSource) => Promise<T>) {
  activeController?.abort();
  if (activeOverlaySequence) {
    hideLoading();
    activeOverlaySequence = 0;
  }
  const controller = new AbortController();
  activeController = controller;
  const sequence = ++scanSequence;
  const generation = state.doc.generation;
  const caretGeneration = state.caret.editGeneration;
  let overlay = false;
  const source = new LazyStructureSource({
    total: state.view.total,
    generation,
    currentGeneration: () => state.doc.generation,
    signal: controller.signal,
    onLine: (visited) => {
      if (visited > FOREGROUND_SCAN_LINES && !overlay) {
        overlay = true;
        activeOverlaySequence = sequence;
        showLoading(t("fold.scanning"), {
          cancel: true,
          onCancel: () => controller.abort(),
        });
      }
      if (overlay && visited % 128 === 0) {
        const percent = state.view.total ? Math.min(99, (visited / state.view.total) * 100) : null;
        setLoadingDetail(t("fold.scannedLines", { count: visited }), percent);
      }
    },
  });
  try {
    const result = await work(source);
    if (
      sequence !== scanSequence ||
      generation !== state.doc.generation ||
      caretGeneration !== state.caret.editGeneration
    ) {
      throw new StructureScanCanceled();
    }
    return result;
  } catch (error) {
    if (
      error instanceof StructureScanCanceled ||
      (error as { name?: string })?.name === "AbortError"
    ) {
      return null;
    }
    console.error("structure scan failed", error);
    flashCount(t("fold.scanError"), "error");
    return null;
  } finally {
    if (activeController === controller) activeController = null;
    if (overlay && activeOverlaySequence === sequence) {
      hideLoading();
      activeOverlaySequence = 0;
    }
  }
}

async function currentBlockAt(line = state.caret.position.line, col = state.caret.position.col) {
  const provider = activeStructureProvider();
  if (!provider) {
    flashCount(t("fold.unsupported"));
    return null;
  }
  const block = await runStructureScan((source) => findStructureBlock(provider, source, line, col));
  if (block) setCurrentStructureBlock(block);
  return block;
}

function afterFoldChange(block?: StructureBlock | null) {
  if (block && activeFoldMap().collapsedAt(block.start)) {
    setFoldSummary(block.start, localSummary(block));
    void refreshSummary(block);
  }
  render();
}

export async function toggleFoldAt(line = state.caret.position.line) {
  const collapsed = activeFoldMap().collapsedAt(line);
  if (collapsed) {
    expandFoldStartingAt(line);
    afterFoldChange();
    return;
  }
  const block = await currentBlockAt(line, 0);
  if (!block) return;
  toggleBlock(block);
  afterFoldChange(block);
}

export async function toggleCurrentFold() {
  const hidden = activeFoldMap().hiddenIntervalContaining(state.caret.position.line);
  if (hidden) {
    expandFoldStartingAt(hidden.start);
    afterFoldChange();
    return;
  }
  const block = await currentBlockAt();
  if (!block) return;
  toggleBlock(block);
  afterFoldChange(block);
}

async function allBlocks(provider: StructureProviderId, source: StructureLineSource) {
  const blocks: StructureBlock[] = [];
  for (let line = 0; line < source.total; line++) {
    const text = await source.get(line);
    if (text == null || !lineMayStartStructure(provider, text)) continue;
    const block = await structureBlockStartingAt(provider, source, line);
    if (block) blocks.push(block);
  }
  return blocks;
}

export async function foldCurrentLevel() {
  const current = await currentBlockAt();
  const provider = activeStructureProvider();
  if (!current || !provider) return;
  const blocks = await runStructureScan((source) => allBlocks(provider, source));
  if (!blocks) return;
  for (const block of blocks.filter((candidate) => candidate.level === current.level)) {
    collapseBlock(block);
    setFoldSummary(block.start, localSummary(block));
  }
  afterFoldChange();
}

export function unfoldCurrentLevel() {
  const level = currentStructureBlock()?.level;
  if (level == null) return;
  for (const block of collapsedStructureBlocks()) {
    if (block.level === level) expandFoldStartingAt(block.start);
  }
  afterFoldChange();
}

export function unfoldAll() {
  for (const interval of activeFoldMap().intervals()) expandFoldStartingAt(interval.start);
  afterFoldChange();
}

export async function foldToLevel(level: number) {
  const provider = activeStructureProvider();
  if (!provider) {
    flashCount(t("fold.unsupported"));
    return;
  }
  const blocks = await runStructureScan((source) => allBlocks(provider, source));
  if (!blocks) return;
  unfoldAll();
  const levels = [...new Set(blocks.map((block) => block.level))].sort(
    (left, right) => left - right,
  );
  const target = levels[Math.max(0, Math.min(level - 1, levels.length - 1))];
  if (target == null) return;
  for (const block of blocks.filter((candidate) => candidate.level === target))
    collapseBlock(block);
  afterFoldChange();
}

export async function goToBlockBoundary(end: boolean) {
  const block = await currentBlockAt();
  if (!block) return;
  moveCaret(end ? block.end : block.start, 0, false, 0);
}

export async function goToSibling(direction: -1 | 1) {
  const current = currentStructureBlock() || (await currentBlockAt());
  const provider = activeStructureProvider();
  if (!current || !provider) return;
  const sibling = await runStructureScan((source) =>
    findSiblingBlock(provider, source, current, direction),
  );
  if (!sibling) return;
  setCurrentStructureBlock(sibling);
  moveCaret(sibling.start, 0, false, 0);
}

export async function goToMatchingBrace(select = false) {
  if (activeStructureProvider() !== "brace") {
    flashCount(t("fold.noMatchingBrace"));
    return;
  }
  const match = await runStructureScan((source) =>
    findMatchingBrace(source, state.caret.position.line, state.caret.position.col),
  );
  if (!match) {
    flashCount(t("fold.noMatchingBrace"));
    return;
  }
  moveCaret(match.line, match.col + (select ? 1 : 0), select, match.col + 1);
}

export function initFolding() {
  const content = document.getElementById("content");
  content?.addEventListener(
    "mousedown",
    (event) => {
      if (!(event.target as HTMLElement | null)?.closest?.(".fold-toggle")) return;
      event.preventDefault();
      event.stopPropagation();
    },
    true,
  );
  content?.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement | null)?.closest?.(".fold-toggle");
    if (!button) return;
    const line = Number((button.closest(".row") as HTMLElement | null)?.dataset.line);
    if (!Number.isSafeInteger(line) || line < 0) return;
    event.preventDefault();
    event.stopPropagation();
    void toggleFoldAt(line);
  });
}
