// IME-safe, bounded word completion (#246). Automatic completion consults
// only syntax vocabulary and already-resident viewport caches. The document
// scanner is reached exclusively from the explicit command and returns words,
// never source lines, through hard server-side budgets.

import { api, type CompletionRequest, type CompletionResponse } from "./api.js";
import { $ } from "./dom.js";
import { cachedLine, caretX } from "./editor.js";
import { completionPrefix } from "./input-assist.js";
import {
  COMPLETION_MAX_DOM_ROWS,
  CompletionCandidates,
  localCompletionCandidates,
} from "./completion-model.js";
import { t } from "./i18n.js";
import { state, LINE_HEIGHT } from "./state.js";
import { completionWordsForScheme, resolveSyntaxScheme, type SchemeId } from "./syntax.js";
import { typeText } from "./edits.js";
import { visibleIndexForLine } from "./fold-state.js";

type CompletionAnchor = {
  request: number;
  docGeneration: number;
  line: number;
  col: number;
  prefix: string;
};

const CLIENT_DEADLINE_MS = 350;
let requestGeneration = 0;
let controller: AbortController | null = null;
let choices: string[] = [];
let selected = 0;
let anchor: CompletionAnchor | null = null;
let initialized = false;

function popup() {
  return $<HTMLDivElement>("completion-popup");
}

function hiddenInput() {
  return $<HTMLTextAreaElement>("hidden-input");
}

function activeScheme(): SchemeId | null {
  const path = state.doc.stat?.path || "";
  const selection = state.syntax.overrides[path] || "auto";
  return resolveSyntaxScheme(path, selection, state.syntax.mappings);
}

function completionContext() {
  if (
    !state.doc.stat?.open ||
    state.caret.composing ||
    state.caret.selection ||
    state.caret.extraCursors.length
  ) {
    return null;
  }
  const { line, col } = state.caret.position;
  const text = cachedLine(line)?.text;
  if (text == null) return null;
  const prefix = completionPrefix(text, col);
  return prefix ? { line, col, prefix } : null;
}

function localTexts(currentLine: number): string[] {
  const texts: string[] = [];
  const seen = new Set<number>();
  const current = cachedLine(currentLine);
  if (current) {
    texts.push(current.text ?? "");
    seen.add(currentLine);
  }
  for (const record of state.view.cache.lines) {
    if (seen.has(record.number)) continue;
    seen.add(record.number);
    texts.push(record.text ?? "");
  }
  for (const record of state.view.sparseCache.values()) {
    if (seen.has(record.number)) continue;
    seen.add(record.number);
    texts.push(record.text ?? "");
  }
  return texts;
}

function stillCurrent(candidate: CompletionAnchor) {
  const context = completionContext();
  return (
    candidate.request === requestGeneration &&
    candidate.docGeneration === state.doc.generation &&
    context?.line === candidate.line &&
    context.col === candidate.col &&
    context.prefix === candidate.prefix
  );
}

function retainIfCurrent(candidate: CompletionAnchor) {
  if (stillCurrent(candidate)) return true;
  if (candidate.request === requestGeneration) hideCompletion();
  return false;
}

function positionPopup() {
  if (!anchor) return;
  const row = visibleIndexForLine(anchor.line) - visibleIndexForLine(state.view.first);
  popup().style.transform = `translate(${caretX(anchor.line, anchor.col)}px, ${(row + 1) * LINE_HEIGHT}px)`;
}

function renderPopup(status: "loading" | "timedOut" | "empty" | null = null) {
  const list = popup();
  list.replaceChildren();
  choices = choices.slice(0, COMPLETION_MAX_DOM_ROWS);
  selected = Math.min(selected, Math.max(0, choices.length - 1));
  choices.forEach((word, index) => {
    const option = document.createElement("div");
    option.id = `completion-option-${index}`;
    option.className = "completion-option";
    option.setAttribute("role", "option");
    option.setAttribute("aria-selected", String(index === selected));
    option.textContent = word;
    option.addEventListener("mousedown", (event) => event.preventDefault());
    option.addEventListener("click", () => acceptCompletion(index));
    list.append(option);
  });
  if (status) {
    const message = document.createElement("div");
    message.className = "completion-status";
    message.setAttribute("role", "status");
    message.textContent = t(`completion.${status}`);
    list.append(message);
  }
  list.classList.remove("hidden");
  const input = hiddenInput();
  input.setAttribute("aria-expanded", "true");
  if (choices.length) {
    input.setAttribute("aria-activedescendant", `completion-option-${selected}`);
  } else {
    input.removeAttribute("aria-activedescendant");
  }
  positionPopup();
}

function localChoices(prefix: string, line: number) {
  return localCompletionCandidates(
    prefix,
    completionWordsForScheme(activeScheme()),
    localTexts(line),
  ).candidates;
}

function startCompletion() {
  controller?.abort();
  controller = null;
  const context = completionContext();
  if (!context || state.settings.wordCompletion === false) {
    hideCompletion();
    return null;
  }
  requestGeneration++;
  anchor = {
    request: requestGeneration,
    docGeneration: state.doc.generation,
    ...context,
  };
  selected = 0;
  choices = localChoices(context.prefix, context.line);
  return anchor;
}

export function showAutomaticCompletion() {
  const context = completionContext();
  if (!context || context.prefix.length < 2 || state.settings.wordCompletion === false) {
    hideCompletion();
    return;
  }
  const nextAnchor = startCompletion();
  if (!nextAnchor || !choices.length) {
    hideCompletion();
    return;
  }
  renderPopup();
}

export async function showCompletion() {
  const nextAnchor = startCompletion();
  if (!nextAnchor) return;
  renderPopup("loading");
  const abort = new AbortController();
  controller = abort;
  const timeout = setTimeout(() => abort.abort(), CLIENT_DEADLINE_MS);
  try {
    const response = await api<CompletionResponse>("/api/completion", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        prefix: nextAnchor.prefix,
        deadline_ms: 250,
      } satisfies CompletionRequest),
      signal: abort.signal,
    });
    if (!retainIfCurrent(nextAnchor)) return;
    const merged = new CompletionCandidates(nextAnchor.prefix);
    merged.addAll(choices);
    merged.addAll(response.candidates);
    choices = merged.sorted();
    renderPopup(
      response.timed_out || response.truncated ? "timedOut" : choices.length ? null : "empty",
    );
  } catch (error) {
    if (!retainIfCurrent(nextAnchor)) return;
    if ((error as Error).name === "AbortError") {
      renderPopup(choices.length ? "timedOut" : "empty");
    } else {
      hideCompletion();
    }
  } finally {
    clearTimeout(timeout);
    if (controller === abort) controller = null;
  }
}

export function hideCompletion() {
  requestGeneration++;
  controller?.abort();
  controller = null;
  anchor = null;
  choices = [];
  selected = 0;
  const list = document.getElementById("completion-popup");
  list?.classList.add("hidden");
  const input = document.getElementById("hidden-input");
  input?.setAttribute("aria-expanded", "false");
  input?.removeAttribute("aria-activedescendant");
}

export function completionVisible() {
  const list = document.getElementById("completion-popup");
  return !!list && !list.classList.contains("hidden");
}

export function acceptCompletion(index = selected) {
  const word = choices[index];
  const current = anchor;
  if (!word || !current || !stillCurrent(current)) {
    hideCompletion();
    return;
  }
  const suffix = Array.from(word).slice(Array.from(current.prefix).length).join("");
  hideCompletion();
  if (suffix) void typeText(suffix);
}

export function handleCompletionKey(event) {
  if (!completionVisible()) return false;
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    event.stopPropagation();
    const delta = event.key === "ArrowDown" ? 1 : -1;
    selected = (selected + delta + choices.length) % Math.max(1, choices.length);
    renderPopup();
    return true;
  }
  if ((event.key === "Enter" || event.key === "Tab") && choices.length) {
    event.preventDefault();
    event.stopPropagation();
    acceptCompletion();
    return true;
  }
  if (event.key === "Escape") {
    event.preventDefault();
    event.stopPropagation();
    hideCompletion();
    return true;
  }
  if (!event.ctrlKey && !event.metaKey && !event.altKey) hideCompletion();
  return false;
}

export function initCompletion() {
  if (initialized) return;
  initialized = true;
  const list = popup();
  const content = $("content");
  content.append(list);
  content.addEventListener("mousedown", (event) => {
    if (!list.contains(event.target as Node)) hideCompletion();
  });
  const input = hiddenInput();
  input.setAttribute("aria-autocomplete", "list");
  input.setAttribute("aria-controls", "completion-popup");
  input.setAttribute("aria-expanded", "false");
}
