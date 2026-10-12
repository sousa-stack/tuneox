import { describe, expect, test } from "bun:test";
import { EngineClock, positionAt, tempoFactor, type ClockState } from "./clock";

const state = (s: Partial<ClockState>): ClockState => ({
  position_us: 10_000_000,
  at_ms: 1000,
  rate: 1,
  duration_us: 300_000_000,
  loop: null,
  ...s,
});

describe("EngineClock", () => {
  test("keeps the smallest offset seen (the least delayed event)", () => {
    const clock = new EngineClock();
    clock.observeEvent(1000, 5003); // 3 ms late
    clock.observeEvent(1050, 5051); // 1 ms late
    clock.observeEvent(1100, 5110); // 10 ms late
    expect(clock.offset).toBe(4001);
    expect(clock.engineNow(6001)).toBe(2000);
  });

  test("a state's at_ms gives a provisional offset until the first event", () => {
    const clock = new EngineClock();
    expect(clock.offset).toBeNull();
    clock.observeState(1000, 5000);
    expect(clock.offset).toBe(4000);
    expect(clock.synced).toBe(false);
    clock.observeEvent(1000, 5010);
    expect(clock.offset).toBe(4010);
    expect(clock.synced).toBe(true);
  });
});

describe("positionAt", () => {
  test("extrapolates position + elapsed × rate", () => {
    expect(positionAt(state({}), 1500)).toBe(10_500_000);
    expect(positionAt(state({ rate: 1.04 }), 1500)).toBeCloseTo(10_520_000, 6);
  });

  test("a paused deck stays put", () => {
    expect(positionAt(state({ rate: 0 }), 99_999)).toBe(10_000_000);
  });

  test("before at_ms (output latency) the heard position is earlier", () => {
    expect(positionAt(state({}), 990)).toBe(9_990_000);
  });

  test("wraps inside an active loop the deck was in", () => {
    const loop = { start_us: 9_000_000, end_us: 11_000_000, beats: 4 };
    expect(positionAt(state({ loop }), 2500)).toBe(9_500_000); // 11.5 s → wrapped
    expect(positionAt(state({ loop }), 6500)).toBe(9_500_000); // several wraps
  });

  test("doesn't wrap a loop the deck was already past", () => {
    const loop = { start_us: 1_000_000, end_us: 2_000_000, beats: 4 };
    expect(positionAt(state({ loop }), 2000)).toBe(11_000_000);
  });

  test("clamps to the track", () => {
    expect(positionAt(state({ position_us: 299_900_000 }), 2000)).toBe(300_000_000);
    expect(positionAt(state({ position_us: 0 }), 500)).toBe(0);
  });
});

test("tempoFactor", () => {
  expect(tempoFactor(0)).toBe(1);
  expect(tempoFactor(4)).toBeCloseTo(1.04);
  expect(tempoFactor(-100)).toBeGreaterThan(0);
});
