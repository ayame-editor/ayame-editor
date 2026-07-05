import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/selection.js", () => ({
  hasSelection: vi.fn(() => false),
  renderSelection: vi.fn(),
}));
vi.mock("../src/menus.js", () => ({ updateStatusPos: vi.fn() }));
vi.mock("../src/input.js", () => ({ anyModalOpen: vi.fn(() => false) }));

import { ensureData } from "../src/editor.js";
import { state } from "../src/state.js";

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

async function flushPromises() {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

describe("editor load generation", () => {
  beforeEach(() => {
    state.total = 1000;
    state.cache = { start: 0, lines: [] };
    state.loadToken = 0;
    vi.stubGlobal("fetch", vi.fn());
  });

  it("ignores stale line fetches after a newer loadToken supersedes them", async () => {
    const resolves: ((value: Response) => void)[] = [];
    vi.mocked(fetch).mockImplementation(
      () =>
        new Promise<Response>((resolve) => {
          resolves.push(resolve);
        }),
    );

    ensureData(0, 1);
    ensureData(700, 1);

    expect(state.loadToken).toBe(2);
    resolves[0](jsonResponse({ lines: [{ number: 0, text: "stale" }], total: 1000 }));
    await flushPromises();
    expect(state.cache.lines).toEqual([]);

    state.loadToken++;
    resolves[1](jsonResponse({ lines: [{ number: 700, text: "also stale" }], total: 1000 }));
    await flushPromises();
    expect(state.cache.lines).toEqual([]);
  });
});
