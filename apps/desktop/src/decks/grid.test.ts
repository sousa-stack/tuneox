import { describe, expect, test } from "bun:test";
import { barBeat, beatIndexAt, formatBars, formatBeats, jumpBeats, lowerBound, toBeatGrid } from "./grid";
import type { Beat } from "./types";

/** A steady grid: `count` beats every `interval` µs from `first`, bar positions from `firstBar`. */
function grid(count: number, first = 24_000, interval = 476_190, firstBar = 1) {
  const beats: Beat[] = [];
  for (let i = 0; i < count; i++) {
    beats.push({ time_us: first + i * interval, bar_position: ((firstBar - 1 + i) % 4) + 1, bpm_centi: 12600 });
  }
  return toBeatGrid(beats);
}

describe("bar counter", () => {
  const g = grid(64);
  const long = grid(200);

  test("counts bar.beat from the first downbeat", () => {
    expect(barBeat(g, 24_000)).toEqual([1, 1]);
    expect(barBeat(g, 24_000 + 476_190)).toEqual([1, 2]);
    expect(barBeat(g, 24_000 + 3 * 476_190 + 1000)).toEqual([1, 4]);
    expect(barBeat(g, 24_000 + 4 * 476_190)).toEqual([2, 1]);
    expect(formatBars(long, 24_000 + 146 * 476_190 + 1000)).toBe("37.3Bars");
  });

  test("a beat counts once the playhead is within 2 ms of it", () => {
    expect(barBeat(g, 24_000 + 476_190 - 1500)).toEqual([1, 2]);
    expect(barBeat(g, 24_000 + 476_190 - 5000)).toEqual([1, 1]);
  });

  test("goes negative before the first downbeat", () => {
    expect(barBeat(g, 0)).toEqual([-1, 4]);
    expect(formatBars(g, 0)).toBe("-1.4Bars");
    expect(barBeat(g, 24_000 - 3 * 476_190)).toEqual([-1, 2]);
    expect(barBeat(g, 24_000 - 4 * 476_190)).toEqual([-1, 1]);
    expect(barBeat(g, 24_000 - 5 * 476_190)).toEqual([-2, 4]);
  });

  test("keeps counting past the last beat", () => {
    expect(barBeat(g, 24_000 + 63 * 476_190)).toEqual([16, 4]);
    expect(barBeat(g, 24_000 + 65 * 476_190 + 1000)).toEqual([17, 2]);
  });

  test("a grid that starts mid-bar counts from its first downbeat", () => {
    const late = grid(16, 1_000_000, 500_000, 3); // beats 3, 4, then 1
    expect(barBeat(late, 1_000_000)).toEqual([-1, 3]);
    expect(barBeat(late, 1_500_000)).toEqual([-1, 4]);
    expect(barBeat(late, 2_000_000)).toEqual([1, 1]);
  });

  test("no grid, no counter", () => {
    expect(formatBars(null, 1000)).toBe("");
    expect(beatIndexAt(toBeatGrid([]), 1000)).toBeNull();
  });
});

test("lowerBound finds the first beat at or after a time", () => {
  const g = grid(8, 0, 1000);
  expect(lowerBound(g.times, -5)).toBe(0);
  expect(lowerBound(g.times, 1000)).toBe(1);
  expect(lowerBound(g.times, 1001)).toBe(2);
  expect(lowerBound(g.times, 99_999)).toBe(8);
});

test("beat jump sizes and loop labels", () => {
  expect(jumpBeats("4beats")).toBe(4);
  expect(jumpBeats("8bars")).toBe(32);
  expect(jumpBeats("32bars")).toBe(128);
  expect(jumpBeats("fine")).toBe(0);
  expect(formatBeats(4)).toBe("4");
  expect(formatBeats(1 / 4)).toBe("1/4");
});
