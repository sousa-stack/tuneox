import { describe, expect, test } from "bun:test";
import { friendlyError } from "./errors";

describe("friendlyError", () => {
  test("a tempo change on a synced deck reads naturally with deck letters", () => {
    expect(
      friendlyError("deck 2 is synced to the master, deck 1: change the tempo on deck 1 or turn sync off on deck 2"),
    ).toBe("Deck B is synced to Deck A. Change the tempo on Deck A, or turn off BEAT SYNC on Deck B.");
    expect(
      friendlyError("deck 1 is synced to the master, deck 2: change the tempo on deck 2 or turn sync off on deck 1"),
    ).toBe("Deck A is synced to Deck B. Change the tempo on Deck B, or turn off BEAT SYNC on Deck A.");
  });

  test("deck numbers become letters and clauses become sentences", () => {
    expect(friendlyError("deck 1 has no track loaded")).toBe("Deck A has no track loaded.");
    expect(friendlyError("the track on deck 2 has no beat grid or BPM")).toBe("The track on Deck B has no beat grid or BPM.");
    expect(friendlyError("deck 1 has no cue point or loop to save: set a cue point or a loop first")).toBe(
      "Deck A has no cue point or loop to save. Set a cue point or a loop first.",
    );
    expect(
      friendlyError("deck 2's track already has 10 memory cues, the most rekordbox allows; delete one first"),
    ).toBe("Deck B's track already has 10 memory cues, the most rekordbox allows. Delete one first.");
    expect(friendlyError("deck 1's track has 3 memory cues; there's no index 5")).toBe(
      "Deck A's track has 3 memory cues. There's no index 5.",
    );
  });

  test("other messages pass through, minus API prefixes", () => {
    expect(friendlyError("Error: not found: the audio file of track 5: /Music/a.mp3")).toBe(
      "not found: the audio file of track 5: /Music/a.mp3",
    );
    expect(friendlyError("invalid input: a scrub \"move\" needs position_us")).toBe('a scrub "move" needs position_us');
    expect(friendlyError("130.00 BPM is +4.00 % from the track's 125.00 BPM, outside the ±6 % tempo range")).toBe(
      "130.00 BPM is +4.00 % from the track's 125.00 BPM, outside the ±6 % tempo range",
    );
    expect(friendlyError("there are two decks, 1 and 2; there's no deck 3")).toBe(
      "There are two decks, A and B; there's no deck 3.",
    );
  });
});
