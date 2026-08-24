import { describe, expect, it } from "vitest";

import { formatPreservedColor, hexToByteEscapes, joinedHex } from "../src/inspector-model.js";

describe("character and color inspector model (#247)", () => {
  it("preserves color prefix, case, and alpha position", () => {
    expect(formatPreservedColor("#aabbcc", 0xdd, "hex4", true)).toEqual({
      literal: "#ABCD",
      format: "hex4",
      uppercase: true,
    });
    expect(formatPreservedColor("#123456", 0x80, "0x8", false).literal).toBe("0x12345680");
    expect(formatPreservedColor("#abcdef", 255, "0x6", true, "0x").literal).toBe("0xABCDEF");
    expect(formatPreservedColor("#abcdef", 255, "0x6", false, "0X").literal).toBe("0Xabcdef");
  });

  it("expands shorthand instead of losing color precision", () => {
    expect(formatPreservedColor("#12ab34", 255, "hex3", false)).toMatchObject({
      literal: "#12ab34",
      format: "hex6",
    });
  });

  it("does not silently omit unavailable raw byte segments", () => {
    expect(joinedHex(["41", "42 43"])).toBe("41 42 43");
    expect(joinedHex(["41", null, "43"])).toBeNull();
    expect(hexToByteEscapes("82 A0")).toBe("\\x82\\xA0");
    expect(hexToByteEscapes(null)).toBeNull();
  });
});
