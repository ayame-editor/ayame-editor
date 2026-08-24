import { describe, expect, it, vi } from "vitest";

import { LazyStructureSource, StructureScanCanceled } from "../src/structure-source.js";

function response(start: number, count: number, total = 10_000_000_000) {
  return {
    start,
    total,
    markers: [],
    lines: Array.from({ length: count }, (_, offset) => ({
      number: start + offset,
      text: `line ${start + offset}`,
      edited: false,
      inserted: false,
      original_line: start + offset,
    })),
  };
}

describe("bounded lazy structure source (#245)", () => {
  it("keeps fixed-size LRU checkpoints at ten billion lines", async () => {
    const load = vi.fn(async (start: number, count: number) => response(start, count));
    const controller = new AbortController();
    const source = new LazyStructureSource({
      total: 10_000_000_000,
      generation: 7,
      currentGeneration: () => 7,
      signal: controller.signal,
      chunkSize: 2,
      maxChunks: 2,
      load,
    });

    await source.get(0);
    await source.get(2);
    await source.get(4);
    expect(source.checkpointCount).toBe(2);
    expect(load).toHaveBeenCalledTimes(3);
    await source.get(0);
    expect(load).toHaveBeenCalledTimes(4);
  });

  it("stops on cancellation and document-generation changes", async () => {
    let generation = 1;
    const controller = new AbortController();
    const source = new LazyStructureSource({
      total: 10,
      generation,
      currentGeneration: () => generation,
      signal: controller.signal,
      load: async (start, count) => response(start, count, 10),
    });
    generation++;
    await expect(source.get(0)).rejects.toBeInstanceOf(StructureScanCanceled);

    const canceled = new AbortController();
    canceled.abort();
    const canceledSource = new LazyStructureSource({
      total: 10,
      generation: 1,
      currentGeneration: () => 1,
      signal: canceled.signal,
    });
    await expect(canceledSource.get(0)).rejects.toBeInstanceOf(StructureScanCanceled);
  });
});
