import { describe, expect, test } from "bun:test";
import { HOT_CUE_PALETTE, MEMORY_CUE_DEFAULT } from "./colors";
import { hotCues, memoryCueAt, memoryCues, withoutHotCue, withoutMemoryCue } from "./cues";
import { emptyDeck, type DeckState } from "./types";

function deck(): DeckState {
  return {
    ...emptyDeck(1),
    track_id: 5,
    cue_us: 6_000_000,
    hot_cues: [
      { slot: 0, start_us: 1_000_000, end_us: null, color: 22, comment: "drop" },
      { slot: 2, start_us: 3_000_000, end_us: 4_000_000, color: null, comment: null },
      { slot: 3, start_us: 5_000_000, end_us: null, color: null, comment: "" },
    ],
    memory_cues: [
      { index: 0, start_us: 2_000_000, end_us: null, color: null, comment: null },
      { index: 1, start_us: 6_000_000, end_us: null, color: 3, comment: "break" },
      { index: 2, start_us: 9_000_000, end_us: 11_000_000, color: null, comment: null },
    ],
  };
}

describe("cues from the deck state", () => {
  test("hot cues take their own colour, else the slot or hot loop default", () => {
    const slots = hotCues(deck());
    expect(slots.length).toBe(8);
    expect(slots[0]).toEqual({ id: 0, start_us: 1_000_000, end_us: null, color: HOT_CUE_PALETTE[22], comment: "drop" });
    expect(slots[1]).toBeNull();
    expect(slots[2]?.color).toBe("#FF8C00"); // COLORFUL hot loop
    expect(slots[3]?.color).toBe("#AA72FF"); // COLORFUL slot D
    expect(slots[3]?.comment).toBeNull();
  });

  test("memory cues: palette colour or red", () => {
    const cues = memoryCues(deck());
    expect(cues.map((c) => c.color)).toEqual([MEMORY_CUE_DEFAULT, HOT_CUE_PALETTE[3], MEMORY_CUE_DEFAULT]);
    expect(cues[1].comment).toBe("break");
  });

  test("X finds the memory cue the deck is parked on", () => {
    const state = deck();
    expect(memoryCueAt(state, 0)).toBe(1); // at the cue point
    expect(memoryCueAt({ ...state, cue_us: null }, 9_001_000)).toBe(2); // at the playhead
    expect(memoryCueAt({ ...state, cue_us: null }, 7_000_000)).toBeNull();
  });

  test("optimistic deletes", () => {
    expect(withoutHotCue(deck(), 2).hot_cues.map((c) => c.slot)).toEqual([0, 3]);
    const { memory_cues } = withoutMemoryCue(deck(), 0);
    expect(memory_cues.map((c) => [c.index, c.start_us])).toEqual([
      [0, 6_000_000],
      [1, 9_000_000],
    ]);
  });
});
