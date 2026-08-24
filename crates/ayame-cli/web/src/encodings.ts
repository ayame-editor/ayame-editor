// UI metadata for every encoding exposed by the server. Core stat values use
// kebab-case enum serialization (notably "utf8"), while conversion requests
// use the parser-friendly value in `value`.
import { el } from "./dom.js";

export const ENCODINGS = [
  {
    value: "utf-8",
    aliases: ["utf8", "utf-8"],
    label: "UTF-8",
    supportsBom: true,
    canConvert: true,
  },
  {
    value: "utf-16le",
    aliases: ["utf16-le", "utf-16le"],
    label: "UTF-16 LE",
    supportsBom: true,
    canConvert: true,
  },
  {
    value: "utf-16be",
    aliases: ["utf16-be", "utf-16be"],
    label: "UTF-16 BE",
    supportsBom: true,
    canConvert: true,
  },
  {
    value: "shift-jis",
    aliases: ["shift-jis"],
    label: "Shift_JIS",
    supportsBom: false,
    canConvert: true,
  },
  {
    value: "euc-jp",
    aliases: ["euc-jp"],
    label: "EUC-JP",
    supportsBom: false,
    canConvert: true,
  },
  {
    value: "ascii",
    aliases: ["ascii"],
    label: "ASCII",
    supportsBom: false,
    canConvert: false,
  },
  {
    value: "iso-2022-jp",
    aliases: ["iso-2022-jp"],
    label: "ISO-2022-JP",
    supportsBom: false,
    canConvert: false,
  },
] as const;

const ENCODING_BY_ALIAS = new Map(
  ENCODINGS.flatMap((encoding) => encoding.aliases.map((alias) => [alias, encoding] as const)),
);

export function encodingInfo(value: unknown) {
  return ENCODING_BY_ALIAS.get(String(value || "").toLowerCase());
}

export function encodingLabel(value: unknown) {
  return encodingInfo(value)?.label || String(value);
}

export function conversionEncodingValue(value: unknown) {
  const encoding = encodingInfo(value);
  return encoding?.canConvert ? encoding.value : "utf-8";
}

export function encodingSupportsBom(value: unknown) {
  return encodingInfo(value)?.supportsBom === true;
}

export function populateEncodingSelect(select: HTMLSelectElement) {
  select.replaceChildren();
  for (const encoding of ENCODINGS) {
    if (!encoding.canConvert) continue;
    const option = el("option", "", encoding.label);
    option.value = encoding.value;
    select.appendChild(option);
  }
}
