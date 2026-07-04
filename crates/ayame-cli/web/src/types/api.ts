// UI-facing aliases for the generated Rust serve API declarations.
//
// Source of truth: `web/types/api.d.ts`, emitted by `cargo xtask typegen`.
// typeship represents Rust integer fields as `bigint`; JSON responses arrive as
// ordinary JavaScript numbers, and the editor does line/column/byte math with
// `number`, so this adapter maps generated `bigint` fields to `number`.

import type * as Wire from "../../types/api";

type JsonWire<T> = T extends bigint
  ? number
  : T extends Array<infer U>
    ? Array<JsonWire<U>>
    : T extends object
      ? { [K in keyof T]: JsonWire<T[K]> }
      : T;

export type OpenRequest = JsonWire<Wire.OpenRequest>;
export type BrowseEntry = JsonWire<Wire.BrowseEntry>;
export type BrowseResponse = JsonWire<Wire.BrowseResponse>;
export type ReplaceRangeRequest = JsonWire<Wire.ReplaceRangeRequest>;
export type CaretPosition = JsonWire<Wire.CaretPosition>;
export type RecoverRequest = JsonWire<Wire.RecoverRequest>;
export type SelectionSaveRequest = JsonWire<Wire.SelectionSaveRequest>;
export type SelectionSaveResponse = JsonWire<Wire.SelectionSaveResponse>;
export type ArtifactResponse = JsonWire<Wire.ArtifactResponse>;
export type SortSaveRequest = JsonWire<Wire.SortSaveRequest>;
export type ReplaceSaveRequest = JsonWire<Wire.ReplaceSaveRequest>;
export type CaseSaveRequest = JsonWire<Wire.CaseSaveRequest>;
