// Waveform records from `get_waveform` (research/waveforms.md, "TuneOx's
// records") and how rekordbox turns them into pixels
// (research/rekordbox-decks.md §4). The painters write straight into a
// Uint32Array view of ImageData, so they are pure and testable.

import {
  BLUE_DETAIL_COLORS,
  BLUE_OVERVIEW_COLORS,
  THREE_BAND,
  pack,
  widen3,
  type RGB,
} from "./colors";
import type { BandGains, WaveformResolution, WaveformResponse, WaveformStyle } from "./types";

export interface Waveform {
  style: WaveformStyle;
  resolution: WaveformResolution;
  columns: number;
  /** Detail waveforms only (150). */
  columnsPerSecond: number | null;
  bytesPerColumn: number;
  data: Uint8Array;
  bandGains: BandGains | null;
}

export function decodeBase64(text: string): Uint8Array {
  const binary = atob(text);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

export function toWaveform(response: WaveformResponse): Waveform {
  const data = decodeBase64(response.data_base64);
  const columns = Math.min(response.columns, Math.floor(data.length / response.bytes_per_column));
  return {
    style: response.style,
    resolution: response.resolution,
    columns,
    columnsPerSecond: response.columns_per_second,
    bytesPerColumn: response.bytes_per_column,
    data,
    bandGains: response.band_gains,
  };
}

/** A 0–255 TuneOx byte back to its source field's 0–`max` range (`anlz::narrow`). */
export function narrow(value: number, max: number): number {
  return Math.round((value * max) / 255);
}

/** rekordbox's raised cosine for the high band, on its 0–128 scale: 0.5 − 0.5·cos(π·h/128). */
export function cosc(h: number): number {
  return 0.5 - 0.5 * Math.cos((Math.PI * h) / 128);
}

/**
 * Whether the detail renderer multiplies the 3-band levels by `band_gains`.
 * No: the stored PWV7 levels already include the gains (research/waveforms.md,
 * "PWVC"); applying them again would cap a band at 80 % with the common 0.80
 * low gain and clip bands with gains up to 3.0. rekordbox's CDJ-style view
 * receives the gains, but whether it applies them twice is unsettled.
 */
export const APPLY_DETAIL_BAND_GAINS = false;

// ── 3Band ───────────────────────────────────────────────────────────────────

type Band = "low" | "mid" | "high";

function overlap(a: Band, b: Band): RGB {
  if (a !== "high" && b !== "high") return THREE_BAND.lowMid;
  if (a !== "mid" && b !== "mid") return THREE_BAND.lowHigh;
  return THREE_BAND.midHigh;
}

/**
 * One 3-band detail column as three spans from the baseline, largest first:
 * the largest in its band's own colour, the middle one in the overlap colour
 * of the two larger bands, the smallest in the all-three cream (§4.4).
 * Heights are fractions of the lane (0–1).
 */
export function threeBandLayers(low: number, mid: number, high: number): [number, RGB, number, RGB, number, RGB] {
  const bands: [Band, number][] = [
    ["low", low],
    ["mid", mid],
    ["high", high],
  ];
  bands.sort((x, y) => y[1] - x[1]);
  const [[b1, h1], [b2, h2], [, h3]] = bands;
  return [h1, THREE_BAND[b1], h2, overlap(b1, b2), h3, THREE_BAND.all];
}

/** Lane heights (0–1) of a 3-band detail record (TuneOx bytes). */
export function threeBandDetailHeights(
  low: number,
  mid: number,
  high: number,
  gains: BandGains | null = null,
): [number, number, number] {
  let l = narrow(low, 127) / 128;
  let m = narrow(mid, 127) / 128;
  let h = cosc(narrow(high, 127));
  if (APPLY_DETAIL_BAND_GAINS && gains) {
    l *= gains.low;
    m *= gains.mid;
    h *= gains.high;
  }
  return [Math.min(1, l), Math.min(1, m), Math.min(1, h)];
}

/**
 * The scale of a 3-band overview: the track's largest low + mid +
 * 128·cosc(high), clamped to 1–128 (§4.4).
 */
export function threeBandOverviewScale(wf: Waveform): number {
  let max = 0;
  const d = wf.data;
  for (let c = 0; c < wf.columns; c++) {
    const o = c * 3;
    const v = narrow(d[o], 127) + narrow(d[o + 1], 127) + 128 * cosc(narrow(d[o + 2], 127));
    if (v > max) max = v;
  }
  return Math.min(128, Math.max(1, max));
}

/**
 * Tops of the stacked 3-band overview bars, as fractions of the view height
 * (may exceed 1; clip): blue = 1.2·low, amber up to 1.2·low + 0.6·mid, white
 * on top up to + 153.6·cosc(high), all ÷ the track scale (§4.4).
 */
export function threeBandOverviewTops(low: number, mid: number, high: number, scale: number): [number, number, number] {
  const l = narrow(low, 127);
  const m = narrow(mid, 127);
  const h = narrow(high, 127);
  const blue = 1.2 * l;
  const amber = blue + 0.6 * m;
  const white = amber + 153.6 * cosc(h);
  return [blue / scale, amber / scale, white / scale];
}

// ── BLUE and RGB ────────────────────────────────────────────────────────────

/** BLUE detail: colour by whiteness (3-bit quantized table) and height 0–1. */
export function blueDetailColumn(height: number, whiteness: number): [RGB, number] {
  const w = narrow(whiteness, 7);
  return [BLUE_DETAIL_COLORS[w > 7 ? 1 : w], height / 255];
}

/** RGB detail: rekordbox widens each 3-bit channel by ⌊v × 255 / 7⌋. */
export function rgbDetailColumn(r: number, g: number, b: number, height: number): [RGB, number] {
  return [[widen3(narrow(r, 7)), widen3(narrow(g, 7)), widen3(narrow(b, 7))], height / 255];
}

/** PWV4 heights are 0–127. */
const PWV4_FULL_SCALE = 127;

/**
 * RGB overview from PWV4 bytes 3–5: hue = the three band maxima scaled so the
 * largest channel is 255; bar height = their maximum; a white core whose
 * height is the high band (§4.3).
 */
export function rgbOverviewColumn(b3: number, b4: number, b5: number): [RGB, number, number] {
  const top = Math.max(b3, b4, b5);
  const color: RGB = top > 0 ? [Math.round((b3 * 255) / top), Math.round((b4 * 255) / top), Math.round((b5 * 255) / top)] : [0, 0, 0];
  return [color, top / PWV4_FULL_SCALE, b5 / PWV4_FULL_SCALE];
}

/**
 * BLUE overview from PWV4: m = max(b0, b1), colour index
 * clamp((m − b2) × 8 / m, 0, 7) (7 when m = 0), height as in RGB (§4.2).
 */
export function blueOverviewColumn(b0: number, b1: number, b2: number, b3: number, b4: number, b5: number): [RGB, number] {
  const m = Math.max(b0, b1);
  const index = m === 0 ? 7 : Math.min(7, Math.max(0, Math.trunc(((m - b2) * 8) / m)));
  return [BLUE_OVERVIEW_COLORS[index], Math.max(b3, b4, b5) / PWV4_FULL_SCALE];
}

// ── Painters ────────────────────────────────────────────────────────────────

const BLACK = pack(0, 0, 0);

function px(fraction: number, height: number): number {
  if (!(fraction > 0)) return 0;
  return Math.min(height, Math.round(fraction * height));
}

/** Which pixel row (0 = baseline) is buffer row `y` of a lane `height` tall. */
function rowIndex(baselineAtBottom: boolean, height: number, r: number): number {
  return baselineAtBottom ? height - 1 - r : r;
}

export interface DetailTile {
  /** Track pixel of the tile's first column. */
  x0: number;
  width: number;
  height: number;
  /** Waveform columns per pixel. */
  columnsPerPixel: number;
  /** Bars grow up from the bottom (deck A) or down from the top (deck B). */
  baselineAtBottom: boolean;
}

/**
 * Paints a detail waveform tile, black background, half form. Each pixel
 * column takes the maximum over the waveform columns it covers (the loudest
 * record for BLUE/RGB, each band's maximum for 3Band).
 */
export function paintDetailTile(wf: Waveform, tile: DetailTile, out: Uint32Array): void {
  const { width, height, columnsPerPixel: cpp, baselineAtBottom } = tile;
  out.fill(BLACK, 0, width * height);
  const d = wf.data;
  const n = wf.bytesPerColumn;
  for (let x = 0; x < width; x++) {
    const start = (tile.x0 + x) * cpp;
    let c0 = Math.floor(start);
    let c1 = Math.floor(start + cpp);
    if (c1 <= c0) c1 = c0 + 1;
    if (c1 <= 0 || c0 >= wf.columns) continue;
    if (c0 < 0) c0 = 0;
    if (c1 > wf.columns) c1 = wf.columns;

    if (wf.style === "three_band") {
      let l = 0;
      let m = 0;
      let h = 0;
      for (let c = c0; c < c1; c++) {
        const o = c * n;
        if (d[o] > l) l = d[o];
        if (d[o + 1] > m) m = d[o + 1];
        if (d[o + 2] > h) h = d[o + 2];
      }
      const [hl, hm, hh] = threeBandDetailHeights(l, m, h, wf.bandGains);
      const [h1, k1, h2, k2, h3] = threeBandLayers(hl, hm, hh);
      const p1 = px(h1, height);
      const p2 = px(h2, height);
      const p3 = px(h3, height);
      const c1c = pack(k1[0], k1[1], k1[2]);
      const c2c = pack(k2[0], k2[1], k2[2]);
      const c3c = pack(THREE_BAND.all[0], THREE_BAND.all[1], THREE_BAND.all[2]);
      for (let r = 0; r < p1; r++) {
        out[rowIndex(baselineAtBottom, height, r) * width + x] = r < p3 ? c3c : r < p2 ? c2c : c1c;
      }
    } else {
      // The loudest record in the range decides colour and height.
      let best = c0;
      const hOffset = wf.style === "rgb" ? 3 : 0;
      for (let c = c0 + 1; c < c1; c++) if (d[c * n + hOffset] > d[best * n + hOffset]) best = c;
      const o = best * n;
      const [rgb, fraction] = wf.style === "rgb" ? rgbDetailColumn(d[o], d[o + 1], d[o + 2], d[o + 3]) : blueDetailColumn(d[o], d[o + 1]);
      const p = px(fraction, height);
      const color = pack(rgb[0], rgb[1], rgb[2]);
      for (let r = 0; r < p; r++) out[rowIndex(baselineAtBottom, height, r) * width + x] = color;
    }
  }
}

/** Overview styles: 3Band from the `three_band` preview, RGB and BLUE from the `rgb` preview. */
export function overviewSource(style: WaveformStyle): WaveformStyle {
  return style === "three_band" ? "three_band" : "rgb";
}

/**
 * Paints an overview (1200 columns over `width` pixels), half form with the
 * baseline at the bottom, black background. `style` is the display style;
 * `wf` must be its `overviewSource`.
 */
export function paintOverview(wf: Waveform, style: WaveformStyle, width: number, height: number, out: Uint32Array): void {
  out.fill(BLACK, 0, width * height);
  if (wf.columns === 0) return;
  const d = wf.data;
  const n = wf.bytesPerColumn;
  const scale = style === "three_band" ? threeBandOverviewScale(wf) : 1;
  const set = (x: number, r: number, color: number) => {
    out[(height - 1 - r) * width + x] = color;
  };
  for (let x = 0; x < width; x++) {
    const c0 = Math.min(wf.columns - 1, Math.floor((x * wf.columns) / width));
    const c1 = Math.max(c0 + 1, Math.min(wf.columns, Math.floor(((x + 1) * wf.columns) / width)));
    if (style === "three_band") {
      let l = 0;
      let m = 0;
      let h = 0;
      for (let c = c0; c < c1; c++) {
        const o = c * n;
        if (d[o] > l) l = d[o];
        if (d[o + 1] > m) m = d[o + 1];
        if (d[o + 2] > h) h = d[o + 2];
      }
      const [blue, amber, white] = threeBandOverviewTops(l, m, h, scale);
      const pb = px(blue, height);
      const pa = px(amber, height);
      const pw = px(white, height);
      const cb = pack(THREE_BAND.low[0], THREE_BAND.low[1], THREE_BAND.low[2]);
      const ca = pack(THREE_BAND.mid[0], THREE_BAND.mid[1], THREE_BAND.mid[2]);
      const cw = pack(255, 255, 255);
      for (let r = 0; r < pw; r++) set(x, r, r < pb ? cb : r < pa ? ca : cw);
    } else {
      // The column with the tallest bar in the range.
      let best = c0;
      let bestTop = -1;
      for (let c = c0; c < c1; c++) {
        const o = c * n;
        const top = Math.max(d[o + 3], d[o + 4], d[o + 5]);
        if (top > bestTop) {
          bestTop = top;
          best = c;
        }
      }
      const o = best * n;
      if (style === "blue") {
        const [rgb, fraction] = blueOverviewColumn(d[o], d[o + 1], d[o + 2], d[o + 3], d[o + 4], d[o + 5]);
        const p = px(fraction, height);
        const color = pack(rgb[0], rgb[1], rgb[2]);
        for (let r = 0; r < p; r++) set(x, r, color);
      } else {
        const [rgb, fraction, core] = rgbOverviewColumn(d[o + 3], d[o + 4], d[o + 5]);
        const p = px(fraction, height);
        const pc = px(core, height);
        for (let r = 0; r < p; r++) {
          // Colour fading from full at the baseline to 50 % at the bar's top,
          // over a 50 % white core.
          const a = 1 - (0.5 * r) / p;
          const under = r < pc ? 127.5 : 0;
          set(
            x,
            r,
            pack(
              Math.round(rgb[0] * a + under * (1 - a)),
              Math.round(rgb[1] * a + under * (1 - a)),
              Math.round(rgb[2] * a + under * (1 - a)),
            ),
          );
        }
      }
    }
  }
}
