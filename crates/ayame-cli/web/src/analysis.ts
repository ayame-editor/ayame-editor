// Progressive multi-rule log analysis controller (issue #242).
//
// The server owns the scan and exact counts. The browser retains only one
// fixed histogram per rule, one bounded result page, and matchers for visible
// viewport rows.

import { api, apiPost, type LineByteResponse, type MarkerBulkResponse } from "./api.js";
import { $, commas, modalVisible, setModalOpen } from "./dom.js";
import { focusEditor, render, revealCaret, scheduleRender, setCaret } from "./editor.js";
import { reloadViewport, settleEditQueue } from "./edits.js";
import { serverMessage, t } from "./i18n.js";
import {
  ANALYSIS_COLOR_TOKENS,
  ANALYSIS_MAX_PROFILES,
  ANALYSIS_MAX_RULES,
  analysisProfileForPath,
  compileAnalysisMatchers,
  defaultAnalysisProfile,
  normalizeAnalysisProfile,
  normalizeAnalysisProfiles,
} from "./analysis-model.js";
import { loadAnalysisProfilesShared, saveAnalysisProfilesShared } from "./persistence.js";
import { flashCount } from "./search.js";
import { grepRuleToFile } from "./save.js";
import { askConfirm, askPrompt, showMessage } from "./dialogs.js";
import { state } from "./state.js";
import type {
  AnalysisHit,
  AnalysisHitsResponse,
  AnalysisNavigateResponse,
  AnalysisProfile,
  AnalysisRuleConfig,
  AnalysisStartRequest,
  AnalysisStatus,
} from "./types/api.js";

const STATUS_POLL_MS = 180;
const HIT_PAGE = 100;
let pollEpoch = 0;
let hitOffset = 0;

function operationActive() {
  return ["scanning", "updating"].includes(state.analysisStatus?.phase);
}

function activeProfile(): AnalysisProfile | null {
  return (
    state.analysisProfiles.find((profile) => profile.id === state.activeAnalysisProfile) ||
    state.analysisProfiles[0] ||
    null
  );
}

function id(prefix: string) {
  const suffix =
    typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
      ? crypto.randomUUID()
      : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  return `${prefix}-${suffix}`;
}

function cloneProfile(profile: AnalysisProfile): AnalysisProfile {
  return JSON.parse(JSON.stringify(profile));
}

function setProfile(profile: AnalysisProfile) {
  state.activeAnalysisProfile = profile.id;
  state.analysisMatchers = compileAnalysisMatchers(profile);
  state.analysisVisibleRuleIds = new Set(
    profile.rules.filter((rule) => rule.enabled).map((rule) => rule.id),
  );
  if (
    !state.analysisSelectedRule ||
    !profile.rules.some((rule) => rule.id === state.analysisSelectedRule)
  ) {
    state.analysisSelectedRule = profile.rules.find((rule) => rule.enabled)?.id || null;
  }
}

function persistProfiles() {
  saveAnalysisProfilesShared(state.analysisProfiles, state.activeAnalysisProfile);
}

function option(value: string, text: string) {
  const element = document.createElement("option");
  element.value = value;
  element.textContent = text;
  return element;
}

function renderProfileSelect() {
  const select = $("analysis-profile-select");
  select.textContent = "";
  for (const profile of state.analysisProfiles) {
    select.append(option(profile.id, profile.name));
  }
  select.value = activeProfile()?.id || "";
  $("analysis-profile-new").disabled = state.analysisProfiles.length >= ANALYSIS_MAX_PROFILES;
}

function checkbox(className: string, checked: boolean, label: string) {
  const wrapper = document.createElement("label");
  const input = document.createElement("input");
  input.type = "checkbox";
  input.className = className;
  input.checked = checked;
  const text = document.createElement("span");
  text.textContent = label;
  wrapper.append(input, text);
  return wrapper;
}

function ruleInput(className: string, value: string, ariaLabel: string) {
  const input = document.createElement("input");
  input.type = "text";
  input.className = `input-control ${className}`;
  input.value = value;
  input.maxLength = className.includes("pattern") ? 4096 : 120;
  input.setAttribute("aria-label", ariaLabel);
  return input;
}

function renderRuleRow(rule: AnalysisRuleConfig) {
  const row = document.createElement("div");
  row.className = "analysis-rule-row";
  row.dataset.ruleId = rule.id;
  row.dataset.analysisColor = rule.color;

  const enabled = document.createElement("input");
  enabled.type = "checkbox";
  enabled.className = "analysis-rule-enabled";
  enabled.checked = rule.enabled;
  enabled.setAttribute("aria-label", t("analysis.enabled"));

  const name = ruleInput("analysis-rule-name", rule.name, t("analysis.ruleName"));
  const pattern = ruleInput("analysis-rule-pattern", rule.pattern, t("analysis.pattern"));

  const color = document.createElement("select");
  color.className = "input-control analysis-rule-color";
  color.setAttribute("aria-label", t("analysis.color"));
  for (const token of ANALYSIS_COLOR_TOKENS)
    color.append(option(token, t(`analysis.color.${token}`)));
  color.value = rule.color;
  color.addEventListener("change", () => {
    row.dataset.analysisColor = color.value;
  });

  const options = document.createElement("div");
  options.className = "analysis-rule-options";
  options.append(
    checkbox("analysis-rule-regex", rule.regex, t("analysis.regex")),
    checkbox("analysis-rule-case", rule.case_sensitive, t("analysis.case")),
    checkbox("analysis-rule-word", rule.whole_word, t("analysis.word")),
  );

  const remove = document.createElement("button");
  remove.type = "button";
  remove.className = "ico analysis-rule-remove";
  remove.setAttribute("aria-label", t("analysis.removeRule"));
  remove.title = t("analysis.removeRule");
  remove.textContent = "×";
  remove.addEventListener("click", () => {
    if ($("analysis-rules").children.length <= 1) {
      flashCount(t("analysis.oneRuleRequired"), "error");
      return;
    }
    row.remove();
    updateRuleAddState();
  });

  row.append(enabled, name, pattern, color, options, remove);
  return row;
}

function updateRuleAddState() {
  $("analysis-rule-add").disabled = $("analysis-rules").children.length >= ANALYSIS_MAX_RULES;
}

function renderProfileForm() {
  const profile = activeProfile();
  if (!profile) return;
  renderProfileSelect();
  $("analysis-profile-name").value = profile.name;
  $("analysis-file-glob").value = profile.file_glob || "";
  const rules = $("analysis-rules");
  rules.textContent = "";
  for (const rule of profile.rules) rules.append(renderRuleRow(rule));
  updateRuleAddState();
  renderModalStatus();
}

function readProfileForm(): AnalysisProfile {
  const current = activeProfile();
  if (!current) throw new Error(t("analysis.noProfile"));
  const rules = [...$("analysis-rules").querySelectorAll<HTMLElement>(".analysis-rule-row")].map(
    (row) => {
      const pattern = row.querySelector<HTMLInputElement>(".analysis-rule-pattern")!.value;
      const regex = row.querySelector<HTMLInputElement>(".analysis-rule-regex")!.checked;
      if (!pattern) throw new Error(t("analysis.patternRequired"));
      if (pattern.includes("\r") || pattern.includes("\n") || pattern.includes("\0")) {
        throw new Error(t("analysis.lineLocal"));
      }
      if (regex) {
        try {
          new RegExp(pattern, "u");
        } catch (error) {
          throw new Error(t("analysis.regexInvalid", { msg: error.message }));
        }
      }
      return {
        id: row.dataset.ruleId,
        name:
          row.querySelector<HTMLInputElement>(".analysis-rule-name")!.value.trim() ||
          pattern.slice(0, 40),
        pattern,
        regex,
        case_sensitive: row.querySelector<HTMLInputElement>(".analysis-rule-case")!.checked,
        whole_word: row.querySelector<HTMLInputElement>(".analysis-rule-word")!.checked,
        color: row.querySelector<HTMLSelectElement>(".analysis-rule-color")!.value,
        enabled: row.querySelector<HTMLInputElement>(".analysis-rule-enabled")!.checked,
      };
    },
  );
  if (!rules.some((rule) => rule.enabled)) throw new Error(t("analysis.enabledRequired"));
  const profile = normalizeAnalysisProfile({
    id: current.id,
    name: $("analysis-profile-name").value,
    file_glob: $("analysis-file-glob").value || null,
    rules,
  });
  if (!profile) throw new Error(t("analysis.invalidProfile"));
  return profile;
}

function invalidateCurrentOperation(message = "") {
  pollEpoch++;
  const operationId = state.analysisOperationId;
  if (operationId && operationActive()) {
    void apiPost("/api/analysis/cancel", { id: operationId }).catch(() => {});
  }
  if (state.analysisStatus) {
    state.analysisStatus = {
      ...state.analysisStatus,
      phase: "stale",
      message: message || t("analysis.stale"),
    };
  }
  renderAnalysisStatus();
}

function saveProfileFromForm() {
  try {
    const profile = readProfileForm();
    const index = state.analysisProfiles.findIndex((item) => item.id === profile.id);
    if (index >= 0) state.analysisProfiles[index] = profile;
    else state.analysisProfiles.push(profile);
    setProfile(profile);
    persistProfiles();
    invalidateCurrentOperation(t("analysis.profileChanged"));
    renderProfileForm();
    flashCount(t("analysis.profileSaved"));
    return profile;
  } catch (error) {
    flashCount(error.message, "error");
    return null;
  }
}

function newRule(): AnalysisRuleConfig {
  const used = new Set(
    [...$("analysis-rules").querySelectorAll<HTMLElement>(".analysis-rule-row")].map(
      (row) => row.dataset.ruleId,
    ),
  );
  let ruleId = id("rule");
  while (used.has(ruleId)) ruleId = id("rule");
  return {
    id: ruleId,
    name: t("analysis.newRule"),
    pattern: "INFO",
    regex: false,
    case_sensitive: true,
    whole_word: true,
    color: "accent",
    enabled: true,
  };
}

async function deleteActiveProfile() {
  if (state.analysisProfiles.length <= 1) {
    flashCount(t("analysis.oneProfileRequired"), "error");
    return;
  }
  const profile = activeProfile();
  if (!profile) return;
  if (
    !(await askConfirm(
      t("analysis.deleteProfile"),
      t("analysis.deleteConfirm", { name: profile.name }),
      {
        okLabel: t("analysis.deleteProfile"),
        danger: true,
      },
    ))
  ) {
    return;
  }
  state.analysisProfiles = state.analysisProfiles.filter((item) => item.id !== profile.id);
  setProfile(state.analysisProfiles[0]);
  persistProfiles();
  invalidateCurrentOperation(t("analysis.profileChanged"));
  renderProfileForm();
}

function addProfile() {
  if (state.analysisProfiles.length >= ANALYSIS_MAX_PROFILES) {
    flashCount(t("analysis.maxProfiles", { count: ANALYSIS_MAX_PROFILES }), "error");
    return;
  }
  const profile = cloneProfile(defaultAnalysisProfile());
  profile.id = id("profile");
  profile.name = t("analysis.newProfileName");
  profile.file_glob = null;
  profile.rules.forEach((rule) => {
    rule.id = id("rule");
  });
  state.analysisProfiles.push(profile);
  setProfile(profile);
  persistProfiles();
  renderProfileForm();
  queueMicrotask(() => {
    $("analysis-profile-name").focus();
    $("analysis-profile-name").select();
  });
}

function phaseText(status: AnalysisStatus) {
  const key = `analysis.phase.${status.phase}`;
  if (status.phase === "scanning" || status.phase === "updating") {
    return t(key, { percent: status.percent.toFixed(1) });
  }
  return t(key);
}

function renderStrip() {
  const status = state.analysisStatus as AnalysisStatus | null;
  const strip = $("analysis-strip");
  strip.classList.toggle("hidden", !status);
  if (!status) return;
  const profile =
    state.analysisProfiles.find((item) => item.id === status.profile_id) || activeProfile();
  $("analysis-manage").textContent = profile?.name || t("analysis.title");
  const chips = $("analysis-chips");
  chips.textContent = "";
  for (const rule of status.rules) {
    const chip = document.createElement("div");
    chip.className = "analysis-chip";
    chip.dataset.analysisColor = rule.color;
    const visible = state.analysisVisibleRuleIds.has(rule.id);
    chip.classList.toggle("off", !visible);

    const toggle = document.createElement("button");
    toggle.type = "button";
    toggle.className = "analysis-chip-toggle";
    toggle.dataset.analysisColor = rule.color;
    toggle.setAttribute("aria-pressed", String(visible));
    toggle.setAttribute(
      "aria-label",
      t("analysis.toggleRule", { name: rule.name, count: commas(rule.count) }),
    );
    const count = document.createElement("span");
    count.className = "analysis-chip-count";
    count.textContent = commas(rule.count);
    toggle.append(document.createTextNode(rule.name), count);
    toggle.addEventListener("click", () => {
      if (state.analysisVisibleRuleIds.has(rule.id)) state.analysisVisibleRuleIds.delete(rule.id);
      else state.analysisVisibleRuleIds.add(rule.id);
      renderStrip();
      scheduleRender();
    });

    const previous = document.createElement("button");
    previous.type = "button";
    previous.className = "analysis-chip-nav";
    previous.textContent = "↑";
    previous.title = t("analysis.previousRule", { name: rule.name });
    previous.setAttribute("aria-label", previous.title);
    previous.disabled = status.phase !== "complete" || !rule.enabled || rule.count === 0;
    previous.addEventListener("click", () => navigateRule(rule.id, "prev"));

    const next = document.createElement("button");
    next.type = "button";
    next.className = "analysis-chip-nav";
    next.textContent = "↓";
    next.title = t("analysis.nextRule", { name: rule.name });
    next.setAttribute("aria-label", next.title);
    next.disabled = status.phase !== "complete" || !rule.enabled || rule.count === 0;
    next.addEventListener("click", () => navigateRule(rule.id, "next"));
    if (rule.truncated)
      chip.title = t("analysis.positionsCapped", { count: commas(rule.stored_hits) });
    chip.append(toggle, previous, next);
    chips.append(chip);
  }
  const progress = $("analysis-progress") as HTMLProgressElement;
  progress.value = Math.max(0, Math.min(100, status.percent));
  $("analysis-progress-label").textContent = status.message || phaseText(status);
  $("analysis-cancel-strip").classList.toggle("hidden", !operationActive());
}

function renderModalStatus() {
  const status = state.analysisStatus as AnalysisStatus | null;
  const summary = $("analysis-summary");
  const results = $("analysis-results");
  if (!status) {
    summary.textContent = t("analysis.ready");
    results.classList.add("hidden");
    return;
  }
  const counts = status.rules
    .filter((rule) => rule.enabled)
    .map((rule) => `${rule.name}: ${commas(rule.count)}`)
    .join(" · ");
  summary.textContent = `${phaseText(status)}${counts ? ` — ${counts}` : ""}`;
  results.classList.toggle("hidden", status.phase !== "complete");
  const select = $("analysis-result-rule") as HTMLSelectElement;
  const previous = select.value || state.analysisSelectedRule;
  select.textContent = "";
  for (const rule of status.rules.filter((item) => item.enabled)) {
    select.append(option(rule.id, `${rule.name} (${commas(rule.count)})`));
  }
  if (status.rules.some((rule) => rule.id === previous && rule.enabled)) select.value = previous;
  state.analysisSelectedRule = select.value || null;
}

function renderAnalysisStatus() {
  renderStrip();
  if (modalVisible("analysis-modal")) renderModalStatus();
  scheduleRender();
}

function applyStatus(status: AnalysisStatus) {
  if (status.id !== state.analysisOperationId) return;
  state.analysisStatus = status;
  renderAnalysisStatus();
}

async function pollStatus(epoch: number) {
  while (epoch === pollEpoch && state.analysisOperationId && operationActive()) {
    await new Promise((resolve) => setTimeout(resolve, STATUS_POLL_MS));
    if (epoch !== pollEpoch || !state.analysisOperationId) return;
    try {
      const operationId = state.analysisOperationId;
      const status = await api<AnalysisStatus>(
        `/api/analysis/status?id=${encodeURIComponent(operationId)}`,
      );
      if (epoch !== pollEpoch || operationId !== state.analysisOperationId) return;
      applyStatus(status);
      if (status.phase === "complete" && modalVisible("analysis-modal")) {
        await loadHits(true);
      }
    } catch (error) {
      if (epoch !== pollEpoch) return;
      state.analysisStatus = {
        ...state.analysisStatus,
        phase: "error",
        message: serverMessage(error),
      };
      renderAnalysisStatus();
      return;
    }
  }
}

export async function runAnalysis() {
  if (!state.stat?.open) {
    flashCount(t("analysis.noDocument"), "error");
    return;
  }
  const profile = saveProfileFromForm();
  if (!profile) return;
  await settleEditQueue();
  pollEpoch++;
  const epoch = pollEpoch;
  state.analysisLastHits = new Map();
  state.analysisSelectedRule = profile.rules.find((rule) => rule.enabled)?.id || null;
  setProfile(profile);
  try {
    const status = await apiPost<AnalysisStatus, AnalysisStartRequest>("/api/analysis/start", {
      profile,
      max_hits_per_rule: null,
    });
    if (epoch !== pollEpoch) {
      void apiPost("/api/analysis/cancel", { id: status.id }).catch(() => {});
      return;
    }
    state.analysisOperationId = status.id;
    state.analysisStatus = status;
    renderAnalysisStatus();
    void pollStatus(epoch);
  } catch (error) {
    flashCount(t("analysis.startError"), "error");
    showMessage(t("analysis.startError"), serverMessage(error));
  }
}

export async function cancelAnalysis() {
  if (!state.analysisOperationId) return;
  pollEpoch++;
  try {
    applyStatus(
      await apiPost("/api/analysis/cancel", {
        id: state.analysisOperationId,
      }),
    );
  } catch {
    // The operation may already have completed/expired.
  }
}

async function currentByteAnchor() {
  const response = await api<LineByteResponse>(
    `/api/linebyte?line=${state.caret.line}&col=${state.caret.col}`,
  );
  return response.byte || 0;
}

export async function navigateRule(
  ruleId = state.analysisSelectedRule,
  direction: "next" | "prev" = "next",
) {
  const status = state.analysisStatus as AnalysisStatus | null;
  if (!status || status.phase !== "complete" || !ruleId) return;
  if (status.tail_pending) await refreshAnalysisTail();
  if (state.analysisStatus?.phase !== "complete" || !state.analysisOperationId) return;
  const operationId = state.analysisOperationId;
  const last = state.analysisLastHits.get(ruleId) as AnalysisHit | undefined;
  let from = last ? last.byte : await currentByteAnchor();
  if (last && direction === "next") from += Math.max(1, last.byte_len);
  try {
    const query = new URLSearchParams({
      id: operationId,
      rule: ruleId,
      direction,
      from: String(from),
    });
    const response = await api<AnalysisNavigateResponse>(`/api/analysis/navigate?${query}`);
    if (operationId !== state.analysisOperationId || state.analysisStatus?.phase !== "complete") {
      return;
    }
    if (!response.hit) {
      flashCount(t("analysis.noMatches"), "error");
      return;
    }
    state.analysisSelectedRule = ruleId;
    state.analysisLastHits.set(ruleId, response.hit);
    setCaret(response.hit.line, response.hit.column, response.hit.column);
    revealCaret();
    await reloadViewport();
    revealCaret();
    render();
    focusEditor();
    if (response.wrapped) flashCount(t("analysis.wrapped"));
  } catch (error) {
    flashCount(t("analysis.error", { msg: serverMessage(error) }), "error");
  }
}

export function nextAnalysisMatch() {
  return navigateRule(state.analysisSelectedRule, "next");
}

export function previousAnalysisMatch() {
  return navigateRule(state.analysisSelectedRule, "prev");
}

function appendHitRow(hit: AnalysisHit) {
  const row = document.createElement("button");
  row.type = "button";
  row.className = "analysis-hit-row";
  row.setAttribute("role", "option");
  const line = document.createElement("span");
  line.className = "analysis-hit-line";
  line.textContent = commas(hit.line + 1);
  const text = document.createElement("span");
  text.className = "analysis-hit-text";
  text.textContent = `${hit.text}${hit.text_truncated ? "…" : ""}`;
  row.append(line, text);
  row.addEventListener("click", async () => {
    state.analysisLastHits.set(state.analysisSelectedRule, hit);
    setCaret(hit.line, hit.column, hit.column);
    revealCaret();
    await reloadViewport();
    revealCaret();
    render();
    closeAnalysis();
    focusEditor();
  });
  $("analysis-hit-list").append(row);
}

async function loadHits(reset = false) {
  const status = state.analysisStatus as AnalysisStatus | null;
  const rule = ($("analysis-result-rule") as HTMLSelectElement).value;
  const operationId = state.analysisOperationId;
  if (!status || status.phase !== "complete" || !rule || !operationId) return;
  if (reset) {
    hitOffset = 0;
    $("analysis-hit-list").textContent = "";
  }
  try {
    const query = new URLSearchParams({
      id: operationId,
      rule,
      start: String(hitOffset),
      limit: String(HIT_PAGE),
    });
    const response = await api<AnalysisHitsResponse>(`/api/analysis/hits?${query}`);
    if (operationId !== state.analysisOperationId || state.analysisStatus?.phase !== "complete") {
      return;
    }
    for (const hit of response.hits) appendHitRow(hit);
    hitOffset += response.hits.length;
    $("analysis-hit-more").classList.toggle("hidden", hitOffset >= response.stored_hits);
    if (response.truncated) {
      $("analysis-summary").textContent += ` — ${t("analysis.positionsCapped", {
        count: commas(response.stored_hits),
      })}`;
    }
  } catch (error) {
    flashCount(t("analysis.error", { msg: serverMessage(error) }), "error");
  }
}

async function bookmarkSelectedRule() {
  const status = state.analysisStatus as AnalysisStatus | null;
  const ruleId = ($("analysis-result-rule") as HTMLSelectElement).value;
  const rule = status?.rules.find((item) => item.id === ruleId);
  if (!status || status.phase !== "complete" || !rule) return;
  if (
    rule.truncated &&
    !(await askConfirm(
      t("analysis.bookmarkMatches"),
      t("analysis.bookmarkCapped", { count: commas(rule.stored_hits), total: commas(rule.count) }),
      { okLabel: t("analysis.bookmarkMatches") },
    ))
  ) {
    return;
  }
  let start = 0;
  let added = 0;
  try {
    while (start < rule.stored_hits) {
      const query = new URLSearchParams({
        id: state.analysisOperationId,
        rule: ruleId,
        start: String(start),
        limit: "200",
      });
      const page = await api<AnalysisHitsResponse>(`/api/analysis/hits?${query}`);
      if (!page.hits.length) break;
      const lines = [...new Set(page.hits.map((hit) => hit.line))];
      const mutation = await apiPost<MarkerBulkResponse, { kind: string; lines: number[] }>(
        "/api/markers/add",
        { kind: "bookmark", lines },
      );
      added += mutation.added || 0;
      state.bookmarkCount = mutation.count || state.bookmarkCount;
      start += page.hits.length;
      if (mutation.limit_reached) break;
    }
    await reloadViewport();
    render();
    flashCount(t("analysis.bookmarked", { count: commas(added) }));
  } catch (error) {
    flashCount(t("analysis.error", { msg: serverMessage(error) }), "error");
  }
}

async function saveSelectedRuleMatches() {
  const profile = activeProfile();
  const ruleId = ($("analysis-result-rule") as HTMLSelectElement).value;
  const rule = profile?.rules.find((item) => item.id === ruleId);
  if (!rule) return;
  closeAnalysis();
  await grepRuleToFile(rule);
}

async function exportProfiles() {
  const json = JSON.stringify(state.analysisProfiles, null, 2);
  try {
    await navigator.clipboard.writeText(json);
    flashCount(t("analysis.exported"));
  } catch {
    await showMessage(t("analysis.export"), json);
  }
}

async function importProfiles() {
  const value = await askPrompt(t("analysis.import"), t("analysis.importPrompt"), "");
  if (value == null) return;
  try {
    const parsed = JSON.parse(value);
    const incoming = normalizeAnalysisProfiles(Array.isArray(parsed) ? parsed : [parsed]);
    if (!incoming.length) throw new Error(t("analysis.invalidProfile"));
    const existingIds = new Set(state.analysisProfiles.map((profile) => profile.id));
    const combined = normalizeAnalysisProfiles([...state.analysisProfiles, ...incoming]);
    const added = combined.filter((profile) => !existingIds.has(profile.id));
    if (!added.length) {
      throw new Error(t("analysis.maxProfiles", { count: ANALYSIS_MAX_PROFILES }));
    }
    state.analysisProfiles = combined;
    setProfile(added[0]);
    persistProfiles();
    invalidateCurrentOperation(t("analysis.profileChanged"));
    renderProfileForm();
    flashCount(t("analysis.imported", { count: commas(added.length) }));
  } catch (error) {
    flashCount(t("analysis.importError", { msg: error.message }), "error");
  }
}

export function openAnalysis() {
  if (!state.analysisProfiles.length) {
    state.analysisProfiles = [defaultAnalysisProfile()];
    setProfile(state.analysisProfiles[0]);
    persistProfiles();
  }
  renderProfileForm();
  setModalOpen($("analysis-modal"), true);
  queueMicrotask(() => $("analysis-profile-select").focus());
  if (state.analysisStatus?.phase === "complete") void loadHits(true);
}

export function closeAnalysis() {
  setModalOpen($("analysis-modal"), false);
  focusEditor();
}

export function analysisVisible() {
  return modalVisible("analysis-modal");
}

export function invalidateAnalysisForEdit() {
  if (!state.analysisStatus) return;
  invalidateCurrentOperation(t("analysis.editedStale"));
}

export function handleAnalysisDocumentOpened(path: string) {
  pollEpoch++;
  state.analysisOperationId = null;
  state.analysisStatus = null;
  state.analysisLastHits = new Map();
  const associated = analysisProfileForPath(state.analysisProfiles, path);
  const profile = associated || activeProfile();
  if (profile) {
    setProfile(profile);
    if (associated) persistProfiles();
  } else {
    state.analysisMatchers = [];
    state.analysisVisibleRuleIds = new Set();
  }
  renderAnalysisStatus();
}

export function handleAnalysisFileChanged() {
  if (state.analysisStatus) invalidateCurrentOperation(t("analysis.fileChanged"));
}

export async function refreshAnalysisTail() {
  if (!state.analysisOperationId || state.analysisStatus?.phase !== "complete") return;
  try {
    applyStatus(
      await apiPost("/api/analysis/tail", {
        id: state.analysisOperationId,
      }),
    );
  } catch (error) {
    state.analysisStatus = {
      ...state.analysisStatus,
      phase: "stale",
      message: serverMessage(error),
    };
    renderAnalysisStatus();
  }
}

export function initAnalysis() {
  const persisted = loadAnalysisProfilesShared();
  state.analysisProfiles = normalizeAnalysisProfiles(
    state.analysisProfiles.length ? state.analysisProfiles : persisted.profiles,
  );
  if (!state.analysisProfiles.length) state.analysisProfiles = [defaultAnalysisProfile()];
  state.activeAnalysisProfile =
    state.analysisProfiles.find((profile) => profile.id === state.activeAnalysisProfile)?.id ||
    state.analysisProfiles.find((profile) => profile.id === persisted.active)?.id ||
    state.analysisProfiles[0].id;
  setProfile(activeProfile());
  persistProfiles();

  $("analysis-manage").addEventListener("click", openAnalysis);
  $("analysis-close").addEventListener("click", closeAnalysis);
  $("analysis-modal").addEventListener("mousedown", (event) => {
    if (event.target === $("analysis-modal")) closeAnalysis();
  });
  $("analysis-profile-select").addEventListener("change", () => {
    const profile = state.analysisProfiles.find(
      (item) => item.id === $("analysis-profile-select").value,
    );
    if (!profile) return;
    setProfile(profile);
    persistProfiles();
    renderProfileForm();
  });
  $("analysis-profile-new").addEventListener("click", addProfile);
  $("analysis-profile-delete").addEventListener("click", deleteActiveProfile);
  $("analysis-rule-add").addEventListener("click", () => {
    if ($("analysis-rules").children.length >= ANALYSIS_MAX_RULES) return;
    $("analysis-rules").append(renderRuleRow(newRule()));
    updateRuleAddState();
  });
  $("analysis-save-profile").addEventListener("click", saveProfileFromForm);
  $("analysis-run").addEventListener("click", runAnalysis);
  $("analysis-cancel-strip").addEventListener("click", cancelAnalysis);
  $("analysis-import").addEventListener("click", importProfiles);
  $("analysis-export").addEventListener("click", exportProfiles);
  $("analysis-result-rule").addEventListener("change", () => {
    state.analysisSelectedRule = $("analysis-result-rule").value;
    void loadHits(true);
  });
  $("analysis-hit-more").addEventListener("click", () => loadHits(false));
  $("analysis-bookmark").addEventListener("click", bookmarkSelectedRule);
  $("analysis-save-matches").addEventListener("click", saveSelectedRuleMatches);
}
