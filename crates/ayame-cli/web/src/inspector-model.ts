// Side-effect-free formatting helpers for the character/byte inspector (#247).

export type PreservedColor = {
  literal: string;
  format: string;
  uppercase: boolean;
};

function byteHex(value: number) {
  const finite = Number.isFinite(value) ? value : 255;
  return Math.max(0, Math.min(255, Math.round(finite)))
    .toString(16)
    .padStart(2, "0");
}

function compactPair(pair: string) {
  return pair[0] === pair[1] ? pair[0] : null;
}

export function formatPreservedColor(
  rgbHex: string,
  alpha: number,
  format: string,
  uppercase: boolean,
  originalPrefix?: string,
): PreservedColor {
  const rgb = rgbHex.replace(/^#/, "").toLowerCase();
  if (!/^[0-9a-f]{6}$/.test(rgb)) throw new Error("invalid RGB color");
  const a = byteHex(alpha);
  const shortRgb = [rgb.slice(0, 2), rgb.slice(2, 4), rgb.slice(4, 6)].map(compactPair).join("");
  const shortAlpha = compactPair(a);
  let nextFormat = format;
  const prefix = format.startsWith("0x") ? originalPrefix || "0x" : "#";
  let body: string;
  if (format === "hex3" && shortRgb.length === 3) {
    body = shortRgb;
  } else if (format === "hex4" && shortRgb.length === 3 && shortAlpha) {
    body = `${shortRgb}${shortAlpha}`;
  } else if (format === "hex3" || format === "hex6") {
    nextFormat = "hex6";
    body = rgb;
  } else if (format === "hex4" || format === "hex8") {
    nextFormat = "hex8";
    body = `${rgb}${a}`;
  } else if (format === "0x6") {
    body = rgb;
  } else {
    nextFormat = "0x8";
    body = `${rgb}${a}`;
  }
  return {
    literal: `${prefix}${uppercase ? body.toUpperCase() : body}`,
    format: nextFormat,
    uppercase,
  };
}

export function joinedHex(values: Array<string | null>) {
  if (values.some((value) => value == null)) return null;
  return values.filter(Boolean).join(" ");
}

export function hexToByteEscapes(value: string | null) {
  return value == null
    ? null
    : value
        .trim()
        .split(/\s+/)
        .filter(Boolean)
        .map((byte) => `\\x${byte}`)
        .join("");
}
