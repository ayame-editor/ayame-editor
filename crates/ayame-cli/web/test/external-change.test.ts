// External file changes (#163): the client half of "somebody else wrote this
// file". The server owns the baseline and the authoritative pre-overwrite
// refusal; what is checked here is when the client bothers to ask, and that a
// failing probe stays quiet instead of popping a dialog at the user.
import { beforeEach, describe, expect, it, vi } from "vitest";

const apiPost = vi.fn();

vi.mock("../src/api.js", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../src/api.js")>()),
  api: vi.fn(),
  apiPost: (...args: unknown[]) => apiPost(...args),
}));

import { diskChanged, externalChangeWatchable } from "../src/save.js";
import { state } from "../src/state.js";

function openFile(overrides: Record<string, unknown> = {}) {
  state.doc.stat = { open: true, path: "/logs/app.log", dirty: false, ...overrides } as never;
  state.doc.followTail = false;
}

describe("external change probe", () => {
  beforeEach(() => {
    apiPost.mockReset();
    openFile();
  });

  it("reports a change the server saw", async () => {
    apiPost.mockResolvedValue({ open: true, changed: true });
    await expect(diskChanged()).resolves.toBe(true);
    expect(apiPost).toHaveBeenCalledWith("/api/disk/check");
  });

  it("reports no change for an untouched file", async () => {
    apiPost.mockResolvedValue({ open: true, changed: false });
    await expect(diskChanged()).resolves.toBe(false);
  });

  it("treats a closed document as unchanged even if the server says otherwise", async () => {
    apiPost.mockResolvedValue({ open: false, changed: true });
    await expect(diskChanged()).resolves.toBe(false);
  });

  it("stays quiet when the probe itself fails", async () => {
    apiPost.mockRejectedValue(new Error("server went away"));
    await expect(diskChanged()).resolves.toBe(false);
  });

  it("does not probe at all with nothing open", async () => {
    state.doc.stat = { open: false } as never;
    await expect(diskChanged()).resolves.toBe(false);
    expect(apiPost).not.toHaveBeenCalled();
  });
});

describe("when a focus check is worth making", () => {
  beforeEach(() => {
    openFile();
  });

  it("watches an ordinary open file", () => {
    expect(externalChangeWatchable()).toBe(true);
  });

  it("ignores an empty workspace", () => {
    state.doc.stat = { open: false } as never;
    expect(externalChangeWatchable()).toBe(false);
  });

  // The scratch file behind an untitled buffer is this session's own; no other
  // process knows its path, so there is nobody to warn about.
  it("ignores untitled buffers", () => {
    openFile({ path: "/tmp/ayame-srv-untitled-1234/untitled.txt" });
    expect(externalChangeWatchable()).toBe(false);
  });

  // Tail-follow polls, reports and adopts appended bytes on its own; asking on
  // top of it would fire on every line the log gains.
  it("leaves tail-follow to its own polling", () => {
    state.doc.followTail = true;
    expect(externalChangeWatchable()).toBe(false);
  });
});
