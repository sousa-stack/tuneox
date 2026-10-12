import { describe, expect, test } from "bun:test";
import { BLUE_OVERVIEW_COLORS, THREE_BAND, pack } from "./colors";
import {
  blueDetailColumn,
  blueOverviewColumn,
  cosc,
  decodeBase64,
  narrow,
  overviewSource,
  paintDetailTile,
  paintOverview,
  rgbDetailColumn,
  rgbOverviewColumn,
  threeBandDetailHeights,
  threeBandLayers,
  threeBandOverviewScale,
  threeBandOverviewTops,
  toWaveform,
  type Waveform,
} from "./waveform";

const widen = (v: number, max: number) => Math.round((v * 255) / max);

function waveform(style: Waveform["style"], records: number[][], resolution: Waveform["resolution"] = "detail"): Waveform {
  return {
    style,
    resolution,
    columns: records.length,
    columnsPerSecond: resolution === "detail" ? 150 : null,
    bytesPerColumn: records[0]?.length ?? 3,
    data: new Uint8Array(records.flat()),
    bandGains: null,
  };
}

describe("records", () => {
  test("decodes base64 and wraps a get_waveform reply", () => {
    expect([...decodeBase64("AAEC/w==")]).toEqual([0, 1, 2, 255]);
    const wf = toWaveform({
      track_id: 1,
      style: "three_band",
      resolution: "detail",
      columns: 2,
      columns_per_second: 150,
      bytes_per_column: 3,
      data_base64: "AQIDBAUG",
      band_gains: { low: 0.83, mid: 1.13, high: 1.03 },
    });
    expect(wf.columns).toBe(2);
    expect([...wf.data]).toEqual([1, 2, 3, 4, 5, 6]);
    expect(wf.bandGains?.mid).toBe(1.13);
  });

  test("narrow reverses the core's widening exactly", () => {
    for (const max of [7, 31, 127]) {
      for (let v = 0; v <= max; v++) expect(narrow(widen(v, max), max)).toBe(v);
    }
  });
});

describe("3Band", () => {
  test("the raised cosine flattens quiet highs and keeps loud ones", () => {
    expect(cosc(0)).toBe(0);
    expect(cosc(64)).toBeCloseTo(0.5);
    expect(cosc(128)).toBeCloseTo(1);
    expect(cosc(16)).toBeLessThan(16 / 128);
  });

  test("detail heights: low and mid linear on 128, high curved; gains already baked in", () => {
    const [l, m, h] = threeBandDetailHeights(widen(64, 127), widen(32, 127), widen(64, 127), { low: 0.8, mid: 3, high: 5 });
    expect(l).toBeCloseTo(0.5);
    expect(m).toBeCloseTo(0.25);
    expect(h).toBeCloseTo(0.5);
  });

  test("largest span in its band colour, middle in the pair's overlap, smallest cream", () => {
    const [h1, c1, h2, c2, h3, c3] = threeBandLayers(0.5, 0.3, 0.1);
    expect([h1, h2, h3]).toEqual([0.5, 0.3, 0.1]);
    expect(c1).toEqual(THREE_BAND.low);
    expect(c2).toEqual(THREE_BAND.lowMid);
    expect(c3).toEqual(THREE_BAND.all);
    expect(threeBandLayers(0.1, 0.6, 0.4)[1]).toEqual(THREE_BAND.mid);
    expect(threeBandLayers(0.1, 0.6, 0.4)[3]).toEqual(THREE_BAND.midHigh);
    expect(threeBandLayers(0.6, 0.1, 0.4)[3]).toEqual(THREE_BAND.lowHigh);
    expect(threeBandLayers(0.2, 0.1, 0.9)[1]).toEqual(THREE_BAND.high);
  });

  test("overview: stacked 1.2·low, + 0.6·mid, + 153.6·cosc(high), over the track scale", () => {
    const wf = waveform("three_band", [[widen(50, 127), widen(40, 127), widen(64, 127)]], "preview");
    // 50 + 40 + 128·cosc(64) = 154, clamped to 128.
    expect(threeBandOverviewScale(wf)).toBe(128);
    expect(threeBandOverviewScale(waveform("three_band", [[widen(20, 127), widen(10, 127), 0]], "preview"))).toBe(30);
    const [blue, amber, white] = threeBandOverviewTops(widen(50, 127), widen(40, 127), widen(64, 127), 128);
    expect(blue).toBeCloseTo(60 / 128);
    expect(amber).toBeCloseTo(84 / 128);
    expect(white).toBeCloseTo((84 + 76.8) / 128);
  });

  test("a silent track's overview scale is 1", () => {
    expect(threeBandOverviewScale(waveform("three_band", [[0, 0, 0]], "preview"))).toBe(1);
  });
});

describe("BLUE and RGB", () => {
  test("BLUE detail: whiteness picks the quantized colour, height on 31", () => {
    const [color, height] = blueDetailColumn(widen(31, 31), widen(7, 7));
    expect(color).toEqual([0xda, 0xff, 0xff]);
    expect(height).toBe(1);
    expect(blueDetailColumn(0, 0)[0]).toEqual([0x00, 0x24, 0x91]);
  });

  test("RGB detail widens 3-bit channels by ⌊v × 255 / 7⌋", () => {
    const [color, height] = rgbDetailColumn(widen(7, 7), widen(1, 7), widen(4, 7), widen(16, 31));
    expect(color).toEqual([255, 36, 145]);
    expect(height).toBeCloseTo(16 / 31, 2);
  });

  test("RGB overview: hue scaled to 255, height and white core on 127", () => {
    const [color, height, core] = rgbOverviewColumn(100, 50, 25);
    expect(color).toEqual([255, 128, 64]);
    expect(height).toBeCloseTo(100 / 127);
    expect(core).toBeCloseTo(25 / 127);
  });

  test("BLUE overview picks its colour from max(b0, b1) and b2", () => {
    expect(blueOverviewColumn(95, 161, 78, 60, 54, 30)[0]).toEqual(BLUE_OVERVIEW_COLORS[4]);
    expect(blueOverviewColumn(0, 0, 0, 0, 0, 0)[0]).toEqual(BLUE_OVERVIEW_COLORS[7]);
    expect(blueOverviewColumn(100, 0, 120, 0, 0, 0)[0]).toEqual(BLUE_OVERVIEW_COLORS[0]);
  });

  test("overview sources: 3Band from three_band, BLUE and RGB from rgb", () => {
    expect(overviewSource("three_band")).toBe("three_band");
    expect(overviewSource("blue")).toBe("rgb");
    expect(overviewSource("rgb")).toBe("rgb");
  });
});

describe("painters", () => {
  const BLACK = pack(0, 0, 0);
  const at = (out: Uint32Array, width: number, x: number, y: number) => out[y * width + x];

  test("detail tile: half form, up from the bottom or down from the top", () => {
    // One loud bass column, then silence.
    const wf = waveform("three_band", [[255, 0, 0], [0, 0, 0]]);
    const width = 2;
    const height = 10;
    const up = new Uint32Array(width * height);
    paintDetailTile(wf, { x0: 0, width, height, columnsPerPixel: 1, baselineAtBottom: true }, up);
    const blue = pack(...THREE_BAND.low);
    expect(at(up, width, 0, 9)).toBe(blue); // baseline
    expect(at(up, width, 0, 0)).toBe(blue); // 127/128 of 10 rounds to 10
    expect(at(up, width, 1, 9)).toBe(BLACK);
    const down = new Uint32Array(width * height);
    paintDetailTile(wf, { x0: 0, width, height, columnsPerPixel: 1, baselineAtBottom: false }, down);
    expect(at(down, width, 0, 0)).toBe(blue);
  });

  test("detail tile takes each band's maximum over the columns a pixel covers", () => {
    const wf = waveform("three_band", [[widen(64, 127), 0, 0], [0, widen(64, 127), 0]]);
    const out = new Uint32Array(1 * 4);
    paintDetailTile(wf, { x0: 0, width: 1, height: 4, columnsPerPixel: 2, baselineAtBottom: true }, out);
    // low = mid = 0.5 → 2 px of the low∩mid overlap colour.
    expect(at(out, 1, 0, 3)).toBe(pack(...THREE_BAND.lowMid));
    expect(at(out, 1, 0, 2)).toBe(pack(...THREE_BAND.lowMid));
    expect(at(out, 1, 0, 1)).toBe(BLACK);
  });

  test("detail tile past the end of the track stays black", () => {
    const wf = waveform("blue", [[255, 255]]);
    const out = new Uint32Array(4 * 2);
    paintDetailTile(wf, { x0: 10, width: 4, height: 2, columnsPerPixel: 1, baselineAtBottom: true }, out);
    expect([...out].every((v) => v === BLACK)).toBe(true);
  });

  test("overview: 3Band paints blue at the bottom, amber, then white on top", () => {
    const wf = waveform("three_band", [[widen(40, 127), widen(40, 127), widen(100, 127)]], "preview");
    const height = 40;
    const out = new Uint32Array(height);
    paintOverview(wf, "three_band", 1, height, out);
    expect(at(out, 1, 0, height - 1)).toBe(pack(...THREE_BAND.low));
    const colours = new Set([...out]);
    expect(colours.has(pack(...THREE_BAND.mid))).toBe(true);
    expect(colours.has(pack(255, 255, 255))).toBe(true);
  });
});
