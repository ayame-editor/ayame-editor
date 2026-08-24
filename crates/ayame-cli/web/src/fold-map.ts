export type FoldInterval = {
  /** Visible header line. */
  start: number;
  /** Inclusive final logical line hidden below the header. */
  end: number;
};

export type StructureBlock = FoldInterval & {
  level: number;
};

export type FoldSummary = {
  bookmarks: number;
  changes: number;
  matches: number;
};

export type FoldDocumentState = {
  map: FoldMap;
  current: StructureBlock | null;
  blocks: Map<number, StructureBlock>;
  summaries: Map<number, FoldSummary>;
};

function boundedLine(value: number) {
  if (!Number.isSafeInteger(value)) return null;
  return Math.max(0, value);
}

function normalizeIntervals(input: readonly FoldInterval[]): FoldInterval[] {
  const sorted = input
    .flatMap((interval) => {
      const start = boundedLine(interval.start);
      const end = boundedLine(interval.end);
      return start == null || end == null || end <= start ? [] : [{ start, end }];
    })
    .sort((left, right) => left.start - right.start || right.end - left.end);
  const out: FoldInterval[] = [];
  for (const interval of sorted) {
    const previous = out.at(-1);
    // Overlapping and nested ranges have only one visible header after folding,
    // so store their union. Adjacent ranges remain separate: both headers are
    // independently visible and independently expandable.
    if (previous && interval.start <= previous.end) {
      previous.end = Math.max(previous.end, interval.end);
    } else {
      out.push({ ...interval });
    }
  }
  return out;
}

function upperBound(values: readonly number[], value: number) {
  let low = 0;
  let high = values.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (values[middle] <= value) low = middle + 1;
    else high = middle;
  }
  return low;
}

/**
 * Sparse collapsed logical-line intervals with prefix hidden counts.
 *
 * Storage and every mapping operation depend on the number of collapsed
 * ranges, never on the document's logical line count (#245).
 */
export class FoldMap {
  private ranges: FoldInterval[] = [];
  private starts: number[] = [];
  private prefixHidden: number[] = [];

  constructor(intervals: readonly FoldInterval[] = []) {
    this.replace(intervals);
  }

  get size() {
    return this.ranges.length;
  }

  intervals(): readonly FoldInterval[] {
    return this.ranges;
  }

  replace(intervals: readonly FoldInterval[]) {
    this.ranges = normalizeIntervals(intervals);
    this.rebuild();
  }

  clear() {
    if (!this.ranges.length) return false;
    this.replace([]);
    return true;
  }

  collapse(start: number, end: number) {
    const before = JSON.stringify(this.ranges);
    this.replace([...this.ranges, { start, end }]);
    return before !== JSON.stringify(this.ranges);
  }

  expandStartingAt(start: number) {
    const next = this.ranges.filter((interval) => interval.start !== start);
    if (next.length === this.ranges.length) return false;
    this.replace(next);
    return true;
  }

  expandContaining(line: number) {
    const interval = this.hiddenIntervalContaining(line);
    return interval ? this.expandStartingAt(interval.start) : false;
  }

  toggle(start: number, end: number) {
    if (this.expandStartingAt(start)) return "expanded";
    this.collapse(start, end);
    return "collapsed";
  }

  collapsedAt(start: number): FoldInterval | null {
    const index = upperBound(this.starts, start) - 1;
    const interval = index >= 0 ? this.ranges[index] : null;
    return interval?.start === start ? interval : null;
  }

  hiddenIntervalContaining(line: number): FoldInterval | null {
    const index = upperBound(this.starts, line) - 1;
    const interval = index >= 0 ? this.ranges[index] : null;
    return interval && interval.start < line && line <= interval.end ? interval : null;
  }

  visibleLineCount(totalLogicalLines: number) {
    const total = Math.max(0, Math.floor(totalLogicalLines));
    let hidden = 0;
    for (const interval of this.ranges) {
      if (interval.start >= total) break;
      hidden += Math.max(0, Math.min(interval.end, total - 1) - interval.start);
    }
    return Math.max(0, total - hidden);
  }

  /** Visible-row index for a logical line; hidden body lines map to their header. */
  visibleIndex(logicalLine: number, totalLogicalLines: number) {
    const total = Math.max(0, Math.floor(totalLogicalLines));
    const logical = Math.max(0, Math.min(Math.floor(logicalLine), total));
    const index = upperBound(this.starts, logical) - 1;
    if (index < 0) return logical;
    const interval = this.ranges[index];
    const hiddenBefore = index > 0 ? this.prefixHidden[index - 1] : 0;
    if (logical <= interval.end) return interval.start - hiddenBefore;
    return logical - this.prefixHidden[index];
  }

  /** Logical line (or EOF = total) represented by a visible-row index. */
  logicalAtVisible(visibleRow: number, totalLogicalLines: number) {
    const total = Math.max(0, Math.floor(totalLogicalLines));
    const maxVisible = this.visibleLineCount(total);
    const target = Math.max(0, Math.min(Math.floor(visibleRow), maxVisible));
    let low = 0;
    let high = total;
    while (low < high) {
      const middle = Math.floor((low + high) / 2);
      if (this.visibleIndex(middle, total) < target) low = middle + 1;
      else high = middle;
    }
    return low;
  }

  visibleWindow(firstLogicalLine: number, count: number, totalLogicalLines: number) {
    const total = Math.max(0, Math.floor(totalLogicalLines));
    const first = this.visibleIndex(firstLogicalLine, total);
    const last = Math.min(this.visibleLineCount(total), first + Math.max(0, Math.floor(count)) - 1);
    const lines: number[] = [];
    for (let visible = first; visible <= last; visible++) {
      lines.push(this.logicalAtVisible(visible, total));
    }
    return lines;
  }

  reconcile(totalLogicalLines: number) {
    const last = Math.max(0, Math.floor(totalLogicalLines) - 1);
    const next = this.ranges.flatMap((interval) => {
      if (interval.start >= last) return [];
      return [{ start: interval.start, end: Math.min(interval.end, last) }];
    });
    const changed = JSON.stringify(next) !== JSON.stringify(this.ranges);
    if (changed) this.replace(next);
    return changed;
  }

  private rebuild() {
    this.starts = this.ranges.map((interval) => interval.start);
    this.prefixHidden = [];
    let hidden = 0;
    for (const interval of this.ranges) {
      hidden += interval.end - interval.start;
      this.prefixHidden.push(hidden);
    }
  }
}
