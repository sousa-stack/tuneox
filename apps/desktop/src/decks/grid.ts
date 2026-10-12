// Beat grid helpers: grid lines, the bar counter and beat jump sizes.

import type { Beat } from "./types";

export interface BeatGrid {
  /** Beat times in µs, ascending. */
  times: Float64Array;
  /** 1 = downbeat … 4. */
  bars: Uint8Array;
  /** Index of the first downbeat, or -1. */
  firstDownbeat: number;
}

export function toBeatGrid(beats: Beat[]): BeatGrid {
  const times = new Float64Array(beats.length);
  const bars = new Uint8Array(beats.length);
  let firstDownbeat = -1;
  beats.forEach((beat, i) => {
    times[i] = beat.time_us;
    bars[i] = beat.bar_position;
    if (firstDownbeat < 0 && beat.bar_position === 1) firstDownbeat = i;
  });
  return { times, bars, firstDownbeat };
}

/** Index of the first beat at or after `time_us` (binary search). */
export function lowerBound(times: Float64Array, timeUs: number): number {
  let lo = 0;
  let hi = times.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (times[mid] < timeUs) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/** Beats closer than this count as reached (the engine snaps onto beats exactly). */
const ON_BEAT_US = 2000;

/**
 * The beat the playhead is in, as an index into the grid. Before the first
 * beat it goes negative, and past the last it keeps counting, extrapolated
 * with the first or last beat interval.
 */
export function beatIndexAt(grid: BeatGrid, positionUs: number): number | null {
  const { times } = grid;
  const n = times.length;
  if (n === 0) return null;
  const at = positionUs + ON_BEAT_US;
  if (at < times[0]) {
    const interval = n > 1 ? times[1] - times[0] : 500000;
    return -Math.ceil((times[0] - at) / interval);
  }
  const index = lowerBound(times, at + 1) - 1;
  if (index === n - 1 && n > 1) {
    const interval = times[n - 1] - times[n - 2];
    return index + Math.floor((at - times[n - 1]) / interval);
  }
  return index;
}

/**
 * rekordbox's bar counter, `bar.beat` with 4 beats to the bar counted from the
 * first downbeat (`1.1`), negative before it (`-1.4` is the beat before 1.1).
 */
export function barBeat(grid: BeatGrid, positionUs: number): [number, number] | null {
  const index = beatIndexAt(grid, positionUs);
  if (index === null) return null;
  const origin = grid.firstDownbeat >= 0 ? grid.firstDownbeat : (5 - grid.bars[0]) % 4;
  const b = index - origin;
  if (b >= 0) return [Math.floor(b / 4) + 1, (b % 4) + 1];
  const back = -b - 1;
  return [-(Math.floor(back / 4) + 1), 4 - (back % 4)];
}

export function formatBars(grid: BeatGrid | null, positionUs: number): string {
  const bb = grid && barBeat(grid, positionUs);
  return bb ? `${bb[0]}.${bb[1]}Bars` : "";
}

/** Beat jump sizes of rekordbox's EXPORT players (§2.6); default 4 beats. */
export const JUMP_SIZES = [
  { id: "fine", label: "Fine", beats: 0 },
  { id: "4beats", label: "4 Beats", beats: 4 },
  { id: "8beats", label: "8 Beats", beats: 8 },
  { id: "16beats", label: "16 Beats", beats: 16 },
  { id: "8bars", label: "8 Bars", beats: 32 },
  { id: "16bars", label: "16 Bars", beats: 64 },
  { id: "32bars", label: "32 Bars", beats: 128 },
] as const;

export type JumpSizeId = (typeof JUMP_SIZES)[number]["id"];
export const DEFAULT_JUMP: JumpSizeId = "4beats";

/** "Fine" isn't a beat jump: it nudges by 5 ms (PERFORMANCE's FINE) with deck_seek. */
export const FINE_JUMP_US = 5000;

export function jumpBeats(id: JumpSizeId): number {
  return JUMP_SIZES.find((s) => s.id === id)?.beats ?? 4;
}

/** Auto beat loop sizes, 1/64–512 beats. */
export const LOOP_SIZES = [1 / 64, 1 / 32, 1 / 16, 1 / 8, 1 / 4, 1 / 2, 1, 2, 4, 8, 16, 32, 64, 128, 256, 512];
export const DEFAULT_LOOP_BEATS = 4;

export function formatBeats(beats: number): string {
  return beats >= 1 ? String(beats) : `1/${Math.round(1 / beats)}`;
}
