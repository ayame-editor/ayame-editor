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
export type TabIdRequest = JsonWire<Wire.TabIdRequest>;
export type TabReorderRequest = JsonWire<Wire.TabReorderRequest>;
export type BrowseEntry = JsonWire<Wire.BrowseEntry>;
export type BrowseResponse = JsonWire<Wire.BrowseResponse>;
export type ReplaceRangeRequest = JsonWire<Wire.ReplaceRangeRequest>;
export type ReplaceRectRequest = JsonWire<Wire.ReplaceRectRequest>;
export type CaretPosition = JsonWire<Wire.CaretPosition>;
export type EditSaveRequest = JsonWire<Wire.EditSaveRequest>;
export type EditSaveResponse = JsonWire<Wire.EditSaveResponse>;
export type RecoverRequest = JsonWire<Wire.RecoverRequest>;
export type ReopenRequest = JsonWire<Wire.ReopenRequest>;
export type SelectionSaveRequest = JsonWire<Wire.SelectionSaveRequest>;
export type SelectionSaveResponse = JsonWire<Wire.SelectionSaveResponse>;
export type ArtifactResponse = JsonWire<Wire.ArtifactResponse>;
export type ArtifactOpStatus = JsonWire<Wire.ArtifactOpStatus>;
export type OperationCancelRequest = JsonWire<Wire.OperationCancelRequest>;
export type SortSaveRequest = JsonWire<Wire.SortSaveRequest>;
export type ReplaceSaveRequest = JsonWire<Wire.ReplaceSaveRequest>;
export type CaseSaveRequest = JsonWire<Wire.CaseSaveRequest>;
export type SplitSaveRequest = JsonWire<Wire.SplitSaveRequest>;
export type GrepRequest = JsonWire<Wire.GrepRequest>;
export type GrepSaveRequest = JsonWire<Wire.GrepSaveRequest>;
export type AnalysisRuleConfig = JsonWire<Wire.AnalysisRuleConfig>;
export type AnalysisProfile = JsonWire<Wire.AnalysisProfile>;
export type AnalysisStartRequest = JsonWire<Wire.AnalysisStartRequest>;
export type AnalysisCancelRequest = JsonWire<Wire.AnalysisCancelRequest>;
export type AnalysisRuleStatus = JsonWire<Wire.AnalysisRuleStatus>;
export type AnalysisStatus = JsonWire<Wire.AnalysisStatus>;
export type AnalysisHit = JsonWire<Wire.AnalysisHit>;
export type AnalysisNavigateResponse = JsonWire<Wire.AnalysisNavigateResponse>;
export type AnalysisHitsResponse = JsonWire<Wire.AnalysisHitsResponse>;
export type SessionState = JsonWire<Wire.SessionState>;
export type UiState = JsonWire<Wire.UiState>;
export type TabInfo = JsonWire<Wire.TabInfo>;
export type TabsResponse = JsonWire<Wire.TabsResponse>;
export type DiskCheckResponse = JsonWire<Wire.DiskCheckResponse>;
