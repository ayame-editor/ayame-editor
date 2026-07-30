import { beforeEach, describe, expect, it, vi } from "vitest";

// Mirror workspace.test.ts's mocks of the side-effecting graph, plus api.js so
// newUntitled's POST is observable without touching the network. apiPost never
// resolves here, so onDocumentOpened does not run — we assert only the
// synchronous "did the dead end open up?" behavior.
vi.mock("../src/editor.js", () => ({
  clearLineCache: vi.fn(),
  focusEditor: vi.fn(),
  render: vi.fn(),
  scheduleRender: vi.fn(),
  setActiveLine: vi.fn(),
  setCaret: vi.fn(),
  setSearchHits: vi.fn(),
  setSelection: vi.fn(),
}));
vi.mock("../src/save.js", () => ({
  expectWalHandoff: vi.fn(),
  maybeOfferWalRecovery: vi.fn(),
  noteWalError: vi.fn(),
  setSaveWorkspaceService: vi.fn(),
  savingCount: 0,
}));
vi.mock("../src/menu-surface.js", () => ({
  fileMenuVisible: vi.fn(() => false),
  hideFileMenu: vi.fn(),
}));
vi.mock("../src/status.js", () => ({
  updateStatusMeta: vi.fn(),
}));
vi.mock("../src/edits.js", () => ({
  setFollowTail: vi.fn(),
  settleEditQueue: vi.fn(() => Promise.resolve()),
}));
vi.mock("../src/notifications.js", () => ({ flashCount: vi.fn() }));
vi.mock("../src/app.js", () => ({
  confirmCloseLastTab: vi.fn(),
  isNativeApp: vi.fn(() => false),
  nativeOpenDialog: vi.fn(),
  nativeSaveDialog: vi.fn(),
  openNewWindow: vi.fn(),
  requestEditorClose: vi.fn(),
}));
vi.mock("../src/i18n.js", () => ({
  t: (key: string) => key,
  currentLocale: () => "en-US",
  applyStaticI18n: vi.fn(),
}));
vi.mock("../src/api.js", () => ({
  api: vi.fn(() => new Promise(() => {})),
  apiPost: vi.fn(() => new Promise(() => {})),
}));

import { apiPost } from "../src/api.js";
import { focusEditor } from "../src/editor.js";
import { state } from "../src/state.js";
import { setOpenerMode } from "../src/opener-state.js";
import { configureOpener, hideOpener } from "../src/workspace.js";
import { setModalOpen } from "../src/dom.js";

function openerDom() {
  document.body.innerHTML = `
    <div id="app"></div>
    <div id="opener" class="modal hidden" aria-hidden="true">
      <div class="modal-panel">
        <span id="opener-title"></span>
        <label id="opener-input-label"></label>
        <input id="opener-input" />
        <button id="opener-new" class="cmd"></button>
        <button id="opener-open" class="cmd"></button>
        <div id="opener-recent" class="opener-recent hidden" role="listbox"></div>
        <div id="opener-cwd"></div>
        <div id="opener-list" class="opener-list" role="listbox"></div>
        <div id="opener-msg"></div>
        <div id="opener-hint"></div>
      </div>
    </div>`;
}

describe("welcome opener escape hatch (#174)", () => {
  beforeEach(() => {
    openerDom();
    setOpenerMode("open");
    vi.clearAllMocks();
  });

  it("closes and starts a fresh buffer instead of trapping when nothing is open", async () => {
    state.doc.stat = { open: false } as never;
    setModalOpen($("opener"), true);
    expect($("opener").classList.contains("hidden")).toBe(false);
    hideOpener();
    // No longer a dead end: the dialog closes synchronously...
    expect($("opener").classList.contains("hidden")).toBe(true);
    // ...and a new untitled buffer starts (after newUntitled's settle await).
    await vi.waitFor(() => expect(apiPost).toHaveBeenCalledWith("/api/new", {}));
  });

  it("closes normally and returns focus to the editor when a document is open", () => {
    state.doc.stat = { open: true } as never;
    setModalOpen($("opener"), true);
    hideOpener();
    expect($("opener").classList.contains("hidden")).toBe(true);
    expect(apiPost).not.toHaveBeenCalled();
    expect(focusEditor).toHaveBeenCalled();
  });

  it("offers New File only on the open/welcome screen, not save or folder mode", () => {
    configureOpener("open");
    expect($("opener-new").classList.contains("hidden")).toBe(false);
    configureOpener("save");
    expect($("opener-new").classList.contains("hidden")).toBe(true);
    configureOpener("folder");
    expect($("opener-new").classList.contains("hidden")).toBe(true);
  });
});

// Helper: dom.js `$` throws on missing ids, so this mirrors the app's accessor.
function $(id: string): HTMLElement {
  return document.getElementById(id)!;
}
