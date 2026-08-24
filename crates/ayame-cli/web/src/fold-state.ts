import {
  FoldMap,
  type FoldDocumentState,
  type FoldSummary,
  type StructureBlock,
} from "./fold-map.js";
import { state } from "./state.js";

function documentKey(path = state.doc.stat?.path || "") {
  return String(path);
}

function documentState(create = true): FoldDocumentState | null {
  const key = documentKey();
  if (!key) return null;
  let document = state.folds.documents.get(key) || null;
  if (!document && create) {
    document = { map: new FoldMap(), current: null, blocks: new Map(), summaries: new Map() };
    state.folds.documents.set(key, document);
  }
  return document;
}

function changed() {
  state.folds.revision++;
}

function reconcileStoredBlocks(document: FoldDocumentState) {
  for (const start of document.blocks.keys()) {
    if (!document.map.collapsedAt(start)) {
      document.blocks.delete(start);
      document.summaries.delete(start);
    }
  }
}

export function activeFoldMap() {
  return documentState(false)?.map || EMPTY_FOLD_MAP;
}

const EMPTY_FOLD_MAP = new FoldMap();

export function currentStructureBlock() {
  return documentState(false)?.current || null;
}

export function setCurrentStructureBlock(block: StructureBlock | null) {
  const document = documentState(!!block);
  if (!document || JSON.stringify(document.current) === JSON.stringify(block)) return;
  document.current = block;
  changed();
}

export function collapseBlock(block: StructureBlock) {
  const document = documentState();
  if (!document) return false;
  const didChange = document.map.collapse(block.start, block.end);
  document.current = block;
  document.blocks.set(block.start, block);
  reconcileStoredBlocks(document);
  if (didChange) {
    state.view.first = document.map.logicalAtVisible(
      document.map.visibleIndex(state.view.first, state.view.total),
      state.view.total,
    );
    changed();
  }
  return didChange;
}

export function toggleBlock(block: StructureBlock) {
  const document = documentState();
  if (!document) return null;
  const result = document.map.toggle(block.start, block.end);
  document.current = block;
  if (result === "expanded") {
    document.blocks.delete(block.start);
    document.summaries.delete(block.start);
  } else {
    document.blocks.set(block.start, block);
    state.view.first = document.map.logicalAtVisible(
      document.map.visibleIndex(state.view.first, state.view.total),
      state.view.total,
    );
  }
  reconcileStoredBlocks(document);
  changed();
  return result;
}

export function expandFoldStartingAt(line: number) {
  const document = documentState(false);
  if (!document || !document.map.expandStartingAt(line)) return false;
  document.summaries.delete(line);
  document.blocks.delete(line);
  changed();
  return true;
}

export function expandFoldsForLine(line: number) {
  const document = documentState(false);
  const interval = document?.map.hiddenIntervalContaining(line);
  if (!document || !interval) return false;
  document.map.expandStartingAt(interval.start);
  document.summaries.delete(interval.start);
  document.blocks.delete(interval.start);
  changed();
  return true;
}

export function clearActiveFoldsForEdit() {
  const document = documentState(false);
  if (!document || (!document.map.size && !document.current)) return false;
  document.map.clear();
  document.current = null;
  document.blocks.clear();
  document.summaries.clear();
  changed();
  return true;
}

export function clearAllActiveFolds() {
  return clearActiveFoldsForEdit();
}

export function reconcileActiveFolds() {
  const document = documentState(false);
  if (!document || !document.map.reconcile(state.view.total)) return false;
  reconcileStoredBlocks(document);
  document.summaries.clear();
  changed();
  return true;
}

export function setFoldSummary(start: number, summary: FoldSummary) {
  const document = documentState(false);
  if (!document?.map.collapsedAt(start)) return;
  document.summaries.set(start, summary);
  changed();
}

export function foldSummary(start: number) {
  return documentState(false)?.summaries.get(start) || null;
}

export function collapsedStructureBlocks() {
  return [...(documentState(false)?.blocks.values() || [])];
}

export function migrateFoldDocument(oldPath: string, newPath: string) {
  if (!oldPath || !newPath || oldPath === newPath) return false;
  const document = state.folds.documents.get(oldPath);
  if (!document) return false;
  state.folds.documents.delete(oldPath);
  state.folds.documents.set(newPath, document);
  changed();
  return true;
}

export function visibleDocumentLineCount() {
  return activeFoldMap().visibleLineCount(state.view.total);
}

export function visibleIndexForLine(line: number) {
  return activeFoldMap().visibleIndex(line, state.view.total);
}

export function logicalLineAtVisible(visible: number) {
  return activeFoldMap().logicalAtVisible(visible, state.view.total);
}

export function visibleLinesFrom(first: number, count: number) {
  return activeFoldMap().visibleWindow(first, count, state.view.total);
}
