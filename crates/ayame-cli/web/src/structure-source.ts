import { api, type LinesResponse } from "./api.js";
import type { StructureLineSource } from "./structure.js";

export class StructureScanCanceled extends Error {
  constructor() {
    super("structure scan canceled");
    this.name = "AbortError";
  }
}

export type StructureChunkLoader = (
  start: number,
  count: number,
  signal: AbortSignal,
) => Promise<LinesResponse>;

type LazyStructureSourceOptions = {
  total: number;
  generation: number;
  currentGeneration: () => number;
  signal: AbortSignal;
  chunkSize?: number;
  maxChunks?: number;
  load?: StructureChunkLoader;
  onLine?: (visited: number, line: number) => void;
};

/**
 * Fixed-interval, bounded LRU checkpoints for a lazy structure scan.
 * No array or flag is allocated per document line, even at 10B lines.
 */
export class LazyStructureSource implements StructureLineSource {
  readonly total: number;
  readonly chunkSize: number;
  readonly maxChunks: number;
  private readonly generation: number;
  private readonly currentGeneration: () => number;
  private readonly signal: AbortSignal;
  private readonly load: StructureChunkLoader;
  private readonly onLine?: (visited: number, line: number) => void;
  private readonly chunks = new Map<number, string[]>();
  private visited = 0;

  constructor(options: LazyStructureSourceOptions) {
    this.total = Math.max(0, Math.floor(options.total));
    this.chunkSize = Math.max(1, Math.floor(options.chunkSize ?? 256));
    this.maxChunks = Math.max(1, Math.floor(options.maxChunks ?? 16));
    this.generation = options.generation;
    this.currentGeneration = options.currentGeneration;
    this.signal = options.signal;
    this.onLine = options.onLine;
    this.load =
      options.load ||
      ((start, count, signal) =>
        api<LinesResponse>(`/api/lines?start=${start}&count=${count}`, { signal }));
  }

  get checkpointCount() {
    return this.chunks.size;
  }

  async get(line: number) {
    this.assertCurrent();
    if (!Number.isSafeInteger(line) || line < 0 || line >= this.total) return null;
    this.visited++;
    this.onLine?.(this.visited, line);
    const start = Math.floor(line / this.chunkSize) * this.chunkSize;
    let lines = this.chunks.get(start);
    if (lines) {
      this.chunks.delete(start);
      this.chunks.set(start, lines);
    } else {
      const response = await this.load(
        start,
        Math.min(this.chunkSize, this.total - start),
        this.signal,
      );
      this.assertCurrent();
      lines = response.lines.map((record) => record.text);
      this.chunks.set(start, lines);
      while (this.chunks.size > this.maxChunks)
        this.chunks.delete(this.chunks.keys().next().value!);
    }
    return lines[line - start] ?? null;
  }

  private assertCurrent() {
    if (this.signal.aborted || this.currentGeneration() !== this.generation) {
      throw new StructureScanCanceled();
    }
  }
}
