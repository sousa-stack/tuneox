import { describe, expect, test } from "bun:test";
import {
  BLUE_DETAIL_COLORS,
  COLORFUL_SLOT_INDEX,
  HOT_CUE_PALETTE,
  hotCueColor,
  memoryCueColor,
  pack,
  paletteColor,
  quantize3Bit,
  rgbToHex,
} from "./colors";

describe("hot cue palette", () => {
  test("has 62 colours plus black and white", () => {
    expect(HOT_CUE_PALETTE.length).toBe(65);
    expect(HOT_CUE_PALETTE[1]).toBe("#305AFF");
    expect(HOT_CUE_PALETTE[22]).toBe("#28E214");
    expect(HOT_CUE_PALETTE[36]).toBe("#FF8C00");
    expect(HOT_CUE_PALETTE[62]).toBe("#6473FF");
    expect(paletteColor(0)).toBeNull();
    expect(paletteColor(null)).toBeNull();
    expect(paletteColor(99)).toBeNull();
  });

  test("an explicit colour wins over the scheme", () => {
    expect(hotCueColor(0, 22, false)).toBe("#28E214");
    expect(hotCueColor(3, 36, true)).toBe("#FF8C00");
  });

  test("index 0 falls back to the COLORFUL slot defaults", () => {
    const expected = ["#FF376F", "#45ACDB", "#7DC13D", "#AA72FF", "#30D26E", "#E0641B", "#305AFF", "#C3AF04"];
    expect(COLORFUL_SLOT_INDEX.map((_, slot) => hotCueColor(slot, null, false))).toEqual(expected);
    expect(hotCueColor(2, 0, false)).toBe("#7DC13D");
  });

  test("hot loops without a colour are #FF8C00", () => {
    expect(hotCueColor(0, null, true)).toBe("#FF8C00");
  });

  test("memory cues without a colour are red", () => {
    expect(memoryCueColor(null)).toBe("#FF0000");
    expect(memoryCueColor(19)).toBe("#30D26E");
  });
});

describe("3-bit quantization", () => {
  test("reproduces the BLUE detail colours rekordbox draws (research §4.2)", () => {
    expect(BLUE_DETAIL_COLORS.map(rgbToHex)).toEqual([
      "#002491",
      "#0048B6",
      "#006DFF",
      "#00B6FF",
      "#48B6FF",
      "#6DB6FF",
      "#6DB6FF",
      "#DAFFFF",
    ]);
    expect(quantize3Bit([255, 255, 255])).toEqual([255, 255, 255]);
    expect(quantize3Bit([31, 31, 31])).toEqual([0, 0, 0]);
  });
});

test("pack is little-endian RGBA for ImageData", () => {
  const view = new Uint8Array(new Uint32Array([pack(1, 2, 3, 4)]).buffer);
  expect([...view]).toEqual([1, 2, 3, 4]);
});
