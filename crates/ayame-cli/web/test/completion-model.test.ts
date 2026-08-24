import { describe, expect, it } from "vitest";

import {
  COMPLETION_MAX_CANDIDATES,
  COMPLETION_MAX_LOCAL_SOURCE_BYTES,
  CompletionCandidates,
  localCompletionCandidates,
} from "../src/completion-model.js";

describe("bounded completion candidates (#246)", () => {
  it("combines scheme and cached Unicode words without returning the prefix itself", () => {
    const result = localCompletionCandidates("日", ["日本語"], ["日 日本橋 日本語 unrelated"]);

    expect(result.candidates).toEqual(["日本橋", "日本語"]);
  });

  it("caps candidate count and retained candidate memory", () => {
    const pool = new CompletionCandidates("a", 8, 24);
    pool.addAll(Array.from({ length: 100 }, (_, index) => `alpha${index}`));

    expect(pool.words.size).toBeLessThanOrEqual(8);
    expect(pool.bytes).toBeLessThanOrEqual(24);
    expect(pool.truncated).toBe(true);
  });

  it("stops reading resident cache text at a fixed byte budget", () => {
    const result = localCompletionCandidates(
      "a",
      [],
      ["alpha ".repeat(COMPLETION_MAX_LOCAL_SOURCE_BYTES), "afterBudget"],
    );

    expect(result.sourceBytes).toBe(COMPLETION_MAX_LOCAL_SOURCE_BYTES);
    expect(result.candidates.length).toBeLessThanOrEqual(COMPLETION_MAX_CANDIDATES);
    expect(result.candidates).not.toContain("afterBudget");
    expect(result.truncated).toBe(true);
  });
});
