// Compatibility facade for search-related responsibilities.
export * from "./findbar.js";
export * from "./replace.js";
export * from "./grep.js";
export {
  findNextOccurrenceRange,
  promoteSelectionRange,
  selectNextOccurrence,
  selectPrimaryRange,
  wordRangeAt,
} from "./selection.js";
