// Ayame Editor — transient opener dialog state.
//
// This state deliberately lives beside the opener controller instead of in the
// application-wide AppState. A pending Promise resolver is meaningful only for
// the current dialog invocation and must not be observable or mutable by
// unrelated editor features.
import type { OpenerMode } from "./state.js";

export type SaveDialogTarget = { path: string; overwrite: boolean };
export type OpenerResult = string | SaveDialogTarget | null;

let mode: OpenerMode = "open";
let resolver: ((value: OpenerResult) => void) | null = null;

export function currentOpenerMode(): OpenerMode {
  return mode;
}

export function setOpenerMode(next: OpenerMode) {
  mode = next;
}

export function setOpenerResolver<T extends OpenerResult>(next: ((value: T) => void) | null) {
  // The mode-specific controller installs and resolves this callback as one
  // transaction. Store the narrow Promise resolver behind the shared union.
  resolver = next as ((value: OpenerResult) => void) | null;
}

export function resolveOpener(value: OpenerResult) {
  const current = resolver;
  resolver = null;
  if (current) current(value);
}
