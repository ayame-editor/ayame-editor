import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/dialogs.js", () => ({ askConfirm: vi.fn() }));
vi.mock("../src/i18n.js", () => ({
  currentLocale: () => "en-US",
  t: (key: string, values?: Record<string, string>) =>
    values?.name ? `${key}:${values.name}` : key,
}));

import { askConfirm } from "../src/dialogs.js";
import { withOverwriteRetry } from "../src/saveflow.js";

describe("overwrite retry flow", () => {
  beforeEach(() => vi.clearAllMocks());

  it("returns the first successful attempt without prompting", async () => {
    const attempt = vi.fn(async (overwrite: boolean) => (overwrite ? "overwrite" : "create"));

    await expect(withOverwriteRetry("new.txt", attempt)).resolves.toBe("create");
    expect(attempt).toHaveBeenCalledOnce();
    expect(askConfirm).not.toHaveBeenCalled();
  });

  it("confirms an existing target and retries exactly once with overwrite", async () => {
    const exists = Object.assign(new Error("conflict"), { code: "exists" });
    const attempt = vi
      .fn<(overwrite: boolean) => Promise<string>>()
      .mockRejectedValueOnce(exists)
      .mockResolvedValueOnce("saved");
    vi.mocked(askConfirm).mockResolvedValue(true);

    await expect(withOverwriteRetry("\\\\?\\C:\\work\\memo.txt", attempt)).resolves.toBe("saved");
    expect(attempt.mock.calls).toEqual([[false], [true]]);
    expect(askConfirm).toHaveBeenCalledWith(
      "dialog.overwrite.title",
      "dialog.overwrite.ask:C:\\work\\memo.txt",
      { okLabel: "dialog.overwrite.ok", danger: true },
    );
  });

  it("returns null when overwrite is declined", async () => {
    const exists = Object.assign(new Error("conflict"), { code: "exists" });
    const attempt = vi.fn(async () => {
      throw exists;
    });
    vi.mocked(askConfirm).mockResolvedValue(false);

    await expect(withOverwriteRetry("memo.txt", attempt)).resolves.toBeNull();
    expect(attempt).toHaveBeenCalledOnce();
  });

  it("does not prompt or loop for unrelated and forced-overwrite failures", async () => {
    const unrelated = Object.assign(new Error("denied"), { code: "permission_denied" });
    await expect(
      withOverwriteRetry("memo.txt", async () => Promise.reject(unrelated)),
    ).rejects.toBe(unrelated);

    const exists = Object.assign(new Error("still exists"), { code: "exists" });
    const forced = vi.fn(async () => Promise.reject(exists));
    await expect(withOverwriteRetry("memo.txt", forced, true)).rejects.toBe(exists);
    expect(forced).toHaveBeenCalledOnce();
    expect(forced).toHaveBeenCalledWith(true);
    expect(askConfirm).not.toHaveBeenCalled();
  });
});
