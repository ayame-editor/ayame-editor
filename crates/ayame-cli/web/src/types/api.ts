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

export type Encoding = JsonWire<Wire.Encoding>;
export type Eol = JsonWire<Wire.Eol>;
export type LineRecord = JsonWire<Wire.EditLine>;
export type LineMarker = JsonWire<Wire.LineMarker>;
export type SearchHit = JsonWire<Wire.SearchHit>;
export type GrepHit = JsonWire<Wire.GrepHit>;
export type OpenRequest = JsonWire<Wire.OpenRequest>;
export type OpenResponse = JsonWire<Wire.OpenResponse>;
export type StatResponse = JsonWire<Wire.StatResponse>;
export type TailPollResponse = JsonWire<Wire.TailPollResponse>;
export type TabIdRequest = JsonWire<Wire.TabIdRequest>;
export type TabReorderRequest = JsonWire<Wire.TabReorderRequest>;
export type BrowseEntry = JsonWire<Wire.BrowseEntry>;
export type BrowseResponse = JsonWire<Wire.BrowseResponse>;
export type LinesResponse = JsonWire<Wire.LinesResponse>;
export type PositionResolveRequest = JsonWire<Wire.PositionResolveRequest>;
export type PositionResolveResponse = JsonWire<Wire.PositionResolveResponse>;
export type RecognizeRequest = JsonWire<Wire.RecognizeRequest>;
export type RecognizedKind = JsonWire<Wire.RecognizedKind>;
export type RecognizeResponse = JsonWire<Wire.RecognizeResponse>;
export type ActionInput = JsonWire<Wire.ActionInput>;
export type ActionOutput = JsonWire<Wire.ActionOutput>;
export type ExternalActionConfig = JsonWire<Wire.ExternalActionConfig>;
export type ActionSelection = JsonWire<Wire.ActionSelection>;
export type ExternalActionRequest = JsonWire<Wire.ExternalActionRequest>;
export type ExternalActionResponse = JsonWire<Wire.ExternalActionResponse>;
export type CompletionRequest = JsonWire<Wire.CompletionRequest>;
export type CompletionResponse = JsonWire<Wire.CompletionResponse>;
export type InspectPoint = JsonWire<Wire.InspectPoint>;
export type InspectRequest = JsonWire<Wire.InspectRequest>;
export type InspectSummary = JsonWire<Wire.InspectSummary>;
export type ScalarInfo = JsonWire<Wire.ScalarInfo>;
export type ClusterInfo = JsonWire<Wire.ClusterInfo>;
export type ColorLiteral = JsonWire<Wire.ColorLiteral>;
export type InspectResponse = JsonWire<Wire.InspectResponse>;
export type ParseEscapeRequest = JsonWire<Wire.ParseEscapeRequest>;
export type ParseEscapeResponse = JsonWire<Wire.ParseEscapeResponse>;
export type FindResponse = JsonWire<Wire.FindResponse>;
export type SearchResponse = JsonWire<Wire.SearchResponse>;
export type GrepResponse = JsonWire<Wire.GrepResponse>;
export type LineByteResponse = JsonWire<Wire.LineByteResponse>;
export type ReplaceRangeRequest = JsonWire<Wire.ReplaceRangeRequest>;
export type ReplaceRectRequest = JsonWire<Wire.ReplaceRectRequest>;
export type CaretPosition = JsonWire<Wire.CaretPosition>;
export type EditSaveRequest = JsonWire<Wire.EditSaveRequest>;
export type EditSaveResponse = JsonWire<Wire.EditSaveResponse>;
export type RecoverRequest = JsonWire<Wire.RecoverRequest>;
export type ReopenRequest = JsonWire<Wire.ReopenRequest>;
export type MarkerToggleRequest = JsonWire<Wire.MarkerToggleRequest>;
export type MarkerBulkRequest = JsonWire<Wire.MarkerBulkRequest>;
export type MarkerBulkResponse = JsonWire<Wire.MarkerBulkResponse>;
export type MarkerClearRequest = JsonWire<Wire.MarkerClearRequest>;
export type MarkerSaveRequest = JsonWire<Wire.MarkerSaveRequest>;
export type MarkerSaveResponse = JsonWire<Wire.MarkerSaveResponse>;
export type MarkerMutationResponse = JsonWire<Wire.MarkerMutationResponse>;
export type MarkerListResponse = JsonWire<Wire.MarkerListResponse>;
export type MarkerRangeCountsResponse = JsonWire<Wire.MarkerRangeCountsResponse>;
export type MarkerNavigateResponse = JsonWire<Wire.MarkerNavigateResponse>;
export type MarkerPreviewResponse = JsonWire<Wire.MarkerPreviewResponse>;
export type ChangeHistoryResponse = JsonWire<Wire.ChangeHistoryResponse>;
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
export type SyntaxMapping = JsonWire<Wire.SyntaxMapping>;
export type SyntaxOverride = JsonWire<Wire.SyntaxOverride>;
export type UiState = JsonWire<Wire.UiState>;
export type TabInfo = JsonWire<Wire.TabInfo>;
export type TabsResponse = JsonWire<Wire.TabsResponse>;
export type DiskCheckResponse = JsonWire<Wire.DiskCheckResponse>;
