// Pure analysis profile/matcher helpers. Kept DOM-free so persistence, the
// editor renderer, and the analysis controller share one validation model.

import type { AnalysisProfile, AnalysisRuleConfig } from "./types/api.js";

export const ANALYSIS_MAX_PROFILES = 32;
export const ANALYSIS_MAX_RULES = 12;
export const ANALYSIS_COLOR_TOKENS = [
  "accent",
  "danger",
  "warn",
  "string",
  "number",
  "literal",
  "function",
  "link",
] as const;

const WORD = /[\p{L}\p{N}_]/u;

function cleanText(value, max, fallback = "") {
  const text = Array.from(String(value ?? ""))
    .filter((character) => {
      const code = character.codePointAt(0) || 0;
      return code >= 0x20 && code !== 0x7f;
    })
    .join("")
    .trim();
  return Array.from(text).slice(0, max).join("") || fallback;
}

function uniqueId(prefix, used: Set<string>) {
  let id = `${prefix}-${Date.now().toString(36)}`;
  let suffix = 1;
  while (used.has(id)) id = `${prefix}-${Date.now().toString(36)}-${suffix++}`;
  used.add(id);
  return id;
}

export function defaultAnalysisProfile(): AnalysisProfile {
  return {
    id: "log-basics",
    name: "Log basics",
    file_glob: "*.log",
    rules: [
      {
        id: "error",
        name: "ERROR",
        pattern: "ERROR",
        regex: false,
        case_sensitive: true,
        whole_word: true,
        color: "danger",
        enabled: true,
      },
      {
        id: "warning",
        name: "WARN",
        pattern: "WARN",
        regex: false,
        case_sensitive: true,
        whole_word: true,
        color: "warn",
        enabled: true,
      },
      {
        id: "request-id",
        name: "Request / trace ID",
        pattern: "\\b(?:request[_-]?id|trace[_-]?id)[=: ]+\\S+",
        regex: true,
        case_sensitive: false,
        whole_word: false,
        color: "link",
        enabled: true,
      },
    ],
  };
}

function normalizeRule(value, index, used: Set<string>): AnalysisRuleConfig | null {
  if (!value || typeof value !== "object") return null;
  let id = cleanText(value.id, 120);
  if (!id || used.has(id)) id = uniqueId(`rule-${index + 1}`, used);
  else used.add(id);
  const pattern = Array.from(String(value.pattern ?? ""))
    .filter((character) => character !== "\r" && character !== "\n" && character !== "\0")
    .join("")
    .slice(0, 4096);
  if (!pattern) return null;
  const color = ANALYSIS_COLOR_TOKENS.includes(value.color) ? value.color : "accent";
  return {
    id,
    name: cleanText(value.name, 120, pattern.slice(0, 40)),
    pattern,
    regex: !!value.regex,
    case_sensitive: !!value.case_sensitive,
    whole_word: !!value.whole_word,
    color,
    enabled: value.enabled !== false,
  };
}

export function normalizeAnalysisProfile(
  value,
  index = 0,
  profileIds = new Set<string>(),
): AnalysisProfile | null {
  if (!value || typeof value !== "object") return null;
  let id = cleanText(value.id, 120);
  if (!id || profileIds.has(id)) id = uniqueId(`profile-${index + 1}`, profileIds);
  else profileIds.add(id);
  const ruleIds = new Set<string>();
  const rules = (Array.isArray(value.rules) ? value.rules : [])
    .slice(0, ANALYSIS_MAX_RULES)
    .map((rule, ruleIndex) => normalizeRule(rule, ruleIndex, ruleIds))
    .filter(Boolean) as AnalysisRuleConfig[];
  if (!rules.length) return null;
  return {
    id,
    name: cleanText(value.name, 120, `Profile ${index + 1}`),
    file_glob: cleanText(value.file_glob, 1024) || null,
    rules,
  };
}

export function normalizeAnalysisProfiles(values): AnalysisProfile[] {
  const profileIds = new Set<string>();
  return (Array.isArray(values) ? values : [])
    .slice(0, ANALYSIS_MAX_PROFILES)
    .map((profile, index) => normalizeAnalysisProfile(profile, index, profileIds))
    .filter(Boolean) as AnalysisProfile[];
}

export function analysisProfileForPath(profiles: AnalysisProfile[], path: string) {
  return profiles.find((profile) => profile.file_glob && globMatches(profile.file_glob, path));
}

export function globMatches(glob: string, path: string) {
  const normalized = String(path || "").replace(/\\/g, "/");
  const basename = normalized.split("/").pop() || normalized;
  const source = String(glob || "")
    .split(/[,\s]+/)
    .filter(Boolean)
    .map((part) =>
      part
        .replace(/[.+^${}()|[\]\\]/g, "\\$&")
        .replace(/\*/g, ".*")
        .replace(/\?/g, "."),
    )
    .map((part) => `^(?:${part})$`)
    .join("|");
  if (!source) return false;
  try {
    const matcher = new RegExp(source, "i");
    return matcher.test(normalized) || matcher.test(basename);
  } catch {
    return false;
  }
}

export type AnalysisMatcher = {
  id: string;
  color: string;
  priority: number;
  wholeWord: boolean;
  regex: RegExp;
};

export function compileAnalysisMatchers(profile: AnalysisProfile | null): AnalysisMatcher[] {
  if (!profile) return [];
  const matchers: AnalysisMatcher[] = [];
  profile.rules.forEach((rule, priority) => {
    if (!rule.enabled || !rule.pattern) return;
    try {
      const source = rule.regex
        ? rule.pattern
        : rule.pattern.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      matchers.push({
        id: rule.id,
        color: rule.color,
        priority,
        wholeWord: !!rule.whole_word,
        regex: new RegExp(source, rule.case_sensitive ? "gu" : "giu"),
      });
    } catch {
      // The server reports the authoritative Rust-regex error on Run. An
      // invalid browser-side matcher simply stays out of visible highlighting.
    }
  });
  return matchers;
}

export type AnalysisRange = {
  start: number;
  end: number;
  color: string;
  overlap: boolean;
  ruleIds: string[];
};

function codePointBefore(text: string, index: number) {
  if (index <= 0) return "";
  const trailing = text.charCodeAt(index - 1);
  if (trailing >= 0xdc00 && trailing <= 0xdfff && index > 1) {
    const leading = text.charCodeAt(index - 2);
    if (leading >= 0xd800 && leading <= 0xdbff) return text.slice(index - 2, index);
  }
  return text[index - 1];
}

function codePointAt(text: string, index: number) {
  if (index >= text.length) return "";
  const leading = text.charCodeAt(index);
  if (leading >= 0xd800 && leading <= 0xdbff && index + 1 < text.length) {
    const trailing = text.charCodeAt(index + 1);
    if (trailing >= 0xdc00 && trailing <= 0xdfff) return text.slice(index, index + 2);
  }
  return text[index];
}

function wordBoundaryOk(text: string, start: number, end: number) {
  return !WORD.test(codePointBefore(text, start)) && !WORD.test(codePointAt(text, end));
}

/// Non-overlapping display segments. Earlier rules own the background; where
/// two or more rules overlap the segment gets the explicit overlap underline.
export function analysisRanges(
  text: string,
  matchers: AnalysisMatcher[],
  visibleRuleIds?: Set<string>,
): AnalysisRange[] {
  type LineMatch = { start: number; end: number; matcher: AnalysisMatcher };
  const matches: LineMatch[] = [];
  for (const matcher of matchers) {
    if (visibleRuleIds && !visibleRuleIds.has(matcher.id)) continue;
    matcher.regex.lastIndex = 0;
    let match;
    while ((match = matcher.regex.exec(text)) !== null) {
      if (!match[0]) {
        matcher.regex.lastIndex++;
        continue;
      }
      const start = match.index;
      const end = start + match[0].length;
      if (!matcher.wholeWord || wordBoundaryOk(text, start, end)) {
        matches.push({ start, end, matcher });
      }
    }
  }
  if (!matches.length) return [];
  const events = new Map<number, { starts: LineMatch[]; ends: LineMatch[] }>();
  const eventAt = (position: number) => {
    let event = events.get(position);
    if (!event) {
      event = { starts: [], ends: [] };
      events.set(position, event);
    }
    return event;
  };
  for (const match of matches) {
    eventAt(match.start).starts.push(match);
    eventAt(match.end).ends.push(match);
  }
  const boundaries = [...events.keys()].sort((a, b) => a - b);
  const ranges: AnalysisRange[] = [];
  let active: LineMatch[] = [];
  for (let index = 0; index + 1 < boundaries.length; index++) {
    const start = boundaries[index];
    const end = boundaries[index + 1];
    const event = events.get(start);
    if (event?.ends.length) {
      const ending = new Set(event.ends);
      active = active.filter((match) => !ending.has(match));
    }
    if (event?.starts.length) {
      active.push(...event.starts);
      active.sort((a, b) => a.matcher.priority - b.matcher.priority);
    }
    if (!active.length) continue;
    const range = {
      start,
      end,
      color: active[0].matcher.color,
      overlap: active.length > 1,
      ruleIds: active.map((match) => match.matcher.id),
    };
    const previous = ranges[ranges.length - 1];
    if (
      previous &&
      previous.end === range.start &&
      previous.color === range.color &&
      previous.overlap === range.overlap &&
      previous.ruleIds.join("\0") === range.ruleIds.join("\0")
    ) {
      previous.end = range.end;
    } else {
      ranges.push(range);
    }
  }
  return ranges;
}
