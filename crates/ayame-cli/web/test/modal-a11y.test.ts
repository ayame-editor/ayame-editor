import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/editor.js", () => ({ focusEditor: vi.fn() }));
vi.mock("../src/i18n.js", () => ({
  t: (key: string) => key,
  currentLocale: () => "en-US",
}));
vi.mock("../src/api.js", () => ({
  api: vi.fn(() => Promise.resolve({})),
  apiPost: vi.fn(() => Promise.resolve({})),
}));

import { activeTrapRoot, initModalFocusTrap, setModalOpen } from "../src/dom.js";
import { cancelLoading, loadingCancelable, showLoading } from "../src/dialogs.js";

// The focus trap installs one document-level listener for the whole app; do the
// same here, once, so repeated setup doesn't stack duplicate handlers.
let trapReady = false;
function ensureTrap() {
  if (!trapReady) {
    initModalFocusTrap();
    trapReady = true;
  }
}

function tab(shift = false) {
  (document.activeElement || document).dispatchEvent(
    new KeyboardEvent("keydown", { key: "Tab", shiftKey: shift, bubbles: true }),
  );
}

describe("modal inert backdrop (#160)", () => {
  beforeEach(() => {
    document.body.innerHTML = `
      <div id="app"><button id="bg-btn">bg</button></div>
      <div id="opener" class="modal hidden" aria-hidden="true"><button id="op-btn">o</button></div>
      <div id="confirm" class="modal hidden" aria-hidden="true"><button id="cf-btn">c</button></div>`;
  });

  it("makes #app inert and hidden from assistive tech while a modal is open", () => {
    const app = document.getElementById("app")!;
    expect(app.hasAttribute("inert")).toBe(false);
    setModalOpen(document.getElementById("opener")!, true);
    expect(app.hasAttribute("inert")).toBe(true);
    expect(app.getAttribute("aria-hidden")).toBe("true");
    setModalOpen(document.getElementById("opener")!, false);
    expect(app.hasAttribute("inert")).toBe(false);
    expect(app.hasAttribute("aria-hidden")).toBe(false);
  });

  it("keeps only the top-most dialog interactive when modals stack", () => {
    const opener = document.getElementById("opener")!;
    const confirm = document.getElementById("confirm")!;
    setModalOpen(opener, true);
    setModalOpen(confirm, true);
    // The lower dialog is inert behind the confirm; the confirm stays active.
    expect(opener.hasAttribute("inert")).toBe(true);
    expect(opener.getAttribute("aria-hidden")).toBe("true");
    expect(confirm.hasAttribute("inert")).toBe(false);
    expect(confirm.getAttribute("aria-hidden")).toBe("false");
    // Closing the confirm hands control back to the opener.
    setModalOpen(confirm, false);
    expect(opener.hasAttribute("inert")).toBe(false);
    expect(opener.getAttribute("aria-hidden")).toBe("false");
  });
});

describe("modal focus trap (#160)", () => {
  beforeEach(() => {
    ensureTrap();
    document.body.innerHTML = `
      <div id="app"><button id="bg-btn">bg</button></div>
      <div id="dlg" class="modal hidden" aria-hidden="true">
        <button id="a">a</button><button id="b">b</button><button id="c">c</button>
      </div>`;
    setModalOpen(document.getElementById("dlg")!, true);
  });

  it("wraps Tab from the last focusable back to the first", () => {
    document.getElementById("c")!.focus();
    tab();
    expect(document.activeElement!.id).toBe("a");
  });

  it("wraps Shift+Tab from the first focusable to the last", () => {
    document.getElementById("a")!.focus();
    tab(true);
    expect(document.activeElement!.id).toBe("c");
  });

  it("pulls focus back inside when it has escaped the dialog", () => {
    document.getElementById("bg-btn")!.focus();
    tab();
    expect(document.activeElement!.id).toBe("a");
  });
});

describe("progress overlay accessibility (#184)", () => {
  beforeEach(() => {
    ensureTrap();
    document.body.innerHTML = `
      <div id="app">
        <main id="viewport">
          <div id="overlay" class="overlay hidden" role="dialog" aria-modal="true" aria-label="busy"></div>
        </main>
      </div>`;
  });

  it("focuses Cancel and exposes it as the trap root for a cancelable op", () => {
    showLoading("Sorting", { opId: "sort:1", cancel: true });
    // Overlay is now the active trap root even though it lives inside #app.
    expect(activeTrapRoot()!.id).toBe("overlay");
    expect(loadingCancelable()).toBe(true);
    return Promise.resolve().then(() => {
      expect(document.activeElement!.id).toBe("overlay-cancel");
    });
  });

  it("Esc-driven cancel disables the button and shows the canceling state", () => {
    showLoading("Sorting", { opId: "sort:2", cancel: true });
    const cancel = document.getElementById("overlay-cancel") as HTMLButtonElement;
    expect(cancel.disabled).toBe(false);
    cancelLoading();
    expect(cancel.disabled).toBe(true);
    expect(document.getElementById("overlay-detail")!.textContent).toBe(
      "dialog.operation.canceling",
    );
  });

  it("does not treat a non-cancelable overlay as cancelable", () => {
    showLoading("Opening");
    expect(loadingCancelable()).toBe(false);
  });
});
