// Ayame Editor — bounded text retrieval for normalized selection ranges.
import { api, type LinesResponse } from "./api.js";
import { MAX_COPY_LINES } from "./state.js";
import { rangeLineCount } from "./selection-model.js";

export async function selectedTextForRange(range, maxLines = MAX_COPY_LINES) {
  const count = Math.min(rangeLineCount(range), maxLines);
  const response = await api<LinesResponse>(`/api/lines?start=${range.start.line}&count=${count}`);
  // Columns are Unicode scalar counts (the server contract); slicing UTF-16
  // units here would split surrogate pairs (emoji etc.).
  const lines = response.lines.map((line) => Array.from(line.text ?? ""));
  if (!lines.length) return "";
  const complete = count >= rangeLineCount(range);
  if (lines.length === 1) {
    const endCol =
      complete && range.start.line === range.end.line ? range.end.col : lines[0].length;
    return lines[0].slice(range.start.col, endCol).join("");
  }
  const text = [lines[0].slice(range.start.col).join("")];
  for (let i = 1; i < lines.length - 1; i++) text.push(lines[i].join(""));
  const last = lines[lines.length - 1];
  text.push(last.slice(0, complete ? range.end.col : last.length).join(""));
  return text.join("\n");
}
