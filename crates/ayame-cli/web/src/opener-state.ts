// Ayame Editor — transient opener dialog state.
//
// This state deliberately lives beside the opener controller instead of in the
// application-wide AppState. A pending Promise resolver is meaningful only for
// the current dialog invocation and must not be observable or mutable by
// unrelated editor features.
import type { OpenerMode } from "./state.js";

export type OpenerResult = string | { path: string; overwrite: boolean } | null;

let mode: OpenerMode = "open";
let resolver: ((value: any) => void) | null = null;

export function currentOpenerMode(): OpenerMode {
  return mode;
}

export function setOpenerMode(next: OpenerMode) {
  mode = next;
}

export function setOpenerResolver(next: ((value: any) => void) | null) {
  resolver = next;
}

export function resolveOpener(value: OpenerResult) {
  const current = resolver;
  resolver = null;
  if (current) current(value);
}
