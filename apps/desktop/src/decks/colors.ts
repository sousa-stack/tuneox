// Every colour the deck UI takes from rekordbox (research/rekordbox-decks.md
// §4–5). Waveform colours are packed for ImageData; marker colours are CSS.

/** rekordbox's 62-colour hot cue palette, indexed by `djmdCue.ColorTableIndex` (§5.1). */
export const HOT_CUE_PALETTE: readonly string[] = [
  "", // 0 = no colour: use the scheme default
  "#305AFF", "#5073FF", "#508CFF", "#50A0FF", "#50B4FF", "#50B0F2", "#50AEE8", "#45ACDB",
  "#00E0FF", "#19DAF0", "#32D2E6", "#21B4B9", "#20AAA0", "#1FA392", "#19A08C", "#14A584",
  "#14AA7D", "#10B176", "#30D26E", "#37DE5A", "#3CEB50", "#28E214", "#7DC13D", "#8CC832",
  "#9BD723", "#A5E116", "#A5DC0A", "#AAD208", "#B4C805", "#B4BE04", "#BAB404", "#C3AF04",
  "#E1AA00", "#FFA000", "#FF9600", "#FF8C00", "#FF7500", "#E0641B", "#E0461E", "#E0301E",
  "#E02823", "#E62828", "#FF376F", "#FF2D6F", "#FF127B", "#F51E8C", "#EB2DA0", "#E637B4",
  "#DE44CF", "#DE448D", "#E630B4", "#E619DC", "#E600FF", "#DC00FF", "#CC00FF", "#B432FF",
  "#B93CFF", "#C542FF", "#AA5AFF", "#AA72FF", "#8272FF", "#6473FF",
  "#000000", // 63 = black
  "#FFFFFF", // 64 = white
];

/** The COLORFUL hot cue scheme (rekordbox's default): palette index per slot A–H. */
export const COLORFUL_SLOT_INDEX: readonly number[] = [43, 8, 23, 60, 19, 38, 1, 32];
/** COLORFUL's hot loop colour (#FF8C00). */
export const COLORFUL_HOT_LOOP_INDEX = 36;

/** A palette colour, or null for index 0 / out of range. */
export function paletteColor(index: number | null | undefined): string | null {
  if (index == null || index <= 0 || index >= HOT_CUE_PALETTE.length) return null;
  return HOT_CUE_PALETTE[index];
}

/**
 * A hot cue's colour: its own palette colour when it has one (`ColorTableIndex`
 * > 0), else the COLORFUL default for its slot, or the hot loop colour.
 */
export function hotCueColor(slot: number, colorIndex: number | null | undefined, isLoop: boolean): string {
  const own = paletteColor(colorIndex);
  if (own) return own;
  if (isLoop) return HOT_CUE_PALETTE[COLORFUL_HOT_LOOP_INDEX];
  return HOT_CUE_PALETTE[COLORFUL_SLOT_INDEX[((slot % 8) + 8) % 8]];
}

/** Memory cues with no colour are drawn red, the CDJ convention (§4.5). */
export const MEMORY_CUE_DEFAULT = "#FF0000";

export function memoryCueColor(colorIndex: number | null | undefined): string {
  return paletteColor(colorIndex) ?? MEMORY_CUE_DEFAULT;
}

/** Fixed UI colours (§4.5, §5.3). */
export const UI = {
  accent: "#1473EB",
  cueRing: "#FFB937",
  currentCue: "#F8A030",
  playheadPlaying: "#E0E0E0",
  playheadPaused: "#FF0000",
  minuteTick: "#C8C8C8",
  loop: "rgba(255, 163, 0, 0.5)", // #FFA300 at 50 %
  storedLoop: "rgba(232, 240, 255, 0.2)", // #E8F0FF at 20 %
  loopIn: "#F80000",
  downbeatMarker: "#FF0000",
} as const;

// ── Waveform colours ────────────────────────────────────────────────────────

export type RGB = readonly [number, number, number];

export function hexToRgb(hex: string): RGB {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

export function rgbToHex([r, g, b]: RGB): string {
  return "#" + ((1 << 24) | (r << 16) | (g << 8) | b).toString(16).slice(1).toUpperCase();
}

/** Packs a colour for a little-endian Uint32Array view of ImageData. */
export function pack(r: number, g: number, b: number, a = 255): number {
  return ((a << 24) | (b << 16) | (g << 8) | r) >>> 0;
}

export function packHex(hex: string): number {
  const [r, g, b] = hexToRgb(hex);
  return pack(r, g, b);
}

/** Widens a 3-bit channel the way rekordbox does: ⌊v × 255 / 7⌋. */
export function widen3(v: number): number {
  return Math.floor((v * 255) / 7);
}

/** rekordbox keeps the top 3 bits of each channel before drawing a detail waveform (§4.2). */
export function quantize3Bit([r, g, b]: RGB): RGB {
  return [widen3(r >> 5), widen3(g >> 5), widen3(b >> 5)];
}

/** BLUE detail colours by whiteness 0–7, as stored in rekordbox's table (§4.2). */
export const BLUE_DETAIL_TABLE: readonly string[] = [
  "#083C8C", "#085DBD", "#006DFF", "#08AAF7", "#52B6F7", "#73AEFF", "#73AEFF", "#CEE7FF",
];

/** …and as drawn, after the 3-bit quantization. */
export const BLUE_DETAIL_COLORS: readonly RGB[] = BLUE_DETAIL_TABLE.map((hex) => quantize3Bit(hexToRgb(hex)));

/** BLUE overview colours by index 0–7 (§4.2). */
export const BLUE_OVERVIEW_COLORS: readonly RGB[] = [
  "#106098", "#1080B8", "#0890F8", "#10B8E8", "#78C0E0", "#90C0F8", "#90C0F8", "#D0E8F8",
].map(hexToRgb);

/** 3Band colours (§4.4). */
export const THREE_BAND = {
  low: hexToRgb("#0055E1"),
  mid: hexToRgb("#FFA600"),
  high: hexToRgb("#FFFFFF"),
  lowMid: hexToRgb("#B4690A"),
  lowHigh: hexToRgb("#D2DCFA"),
  midHigh: hexToRgb("#FFF0D7"),
  all: hexToRgb("#F5EBD7"),
} as const;
