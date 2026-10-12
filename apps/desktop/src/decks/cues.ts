// Hot cues and memory cues as the decks draw and list them. The engine's
// DeckState carries each cue's colour and comment from the library (none for
// cues stored on the deck, which use their slot's default colour).

import { hotCueColor, memoryCueColor } from "./colors";
import type { DeckState } from "./types";

export interface ShownCue {
  /** Hot cue slot 0–7, or the memory cue index. */
  id: number;
  start_us: number;
  end_us: number | null;
  color: string;
  comment: string | null;
}

/** Hot cues A–H in slot order; null for an empty slot. */
export function hotCues(state: DeckState): (ShownCue | null)[] {
  const slots: (ShownCue | null)[] = new Array(8).fill(null);
  for (const cue of state.hot_cues) {
    if (cue.slot < 0 || cue.slot > 7) continue;
    slots[cue.slot] = {
      id: cue.slot,
      start_us: cue.start_us,
      end_us: cue.end_us,
      color: hotCueColor(cue.slot, cue.color, cue.end_us != null),
      comment: cue.comment || null,
    };
  }
  return slots;
}

export function memoryCues(state: DeckState): ShownCue[] {
  return state.memory_cues.map((cue) => ({
    id: cue.index,
    start_us: cue.start_us,
    end_us: cue.end_us,
    color: memoryCueColor(cue.color),
    comment: cue.comment || null,
  }));
}

/** Positions closer than this are the same cue (the library stores µs; the UI shows ms). */
const SAME_US = 2000;

/**
 * The memory cue X deletes: the one the deck is parked on (rekordbox: call a
 * memory cue, then delete it), matched at the cue point or the playhead.
 */
export function memoryCueAt(state: DeckState, positionUs: number): number | null {
  const at = (us: number | null) => us != null && state.memory_cues.find((c) => Math.abs(c.start_us - us) <= SAME_US);
  const found = at(state.cue_us) || at(positionUs);
  return found ? found.index : null;
}

/** The deck's state with hot cue `slot` cleared (shown before the engine answers). */
export function withoutHotCue(state: DeckState, slot: number): Pick<DeckState, "hot_cues"> {
  return { hot_cues: state.hot_cues.filter((c) => c.slot !== slot) };
}

/** The deck's state with memory cue `index` removed and the later ones moved up. */
export function withoutMemoryCue(state: DeckState, index: number): Pick<DeckState, "memory_cues"> {
  return {
    memory_cues: state.memory_cues.filter((c) => c.index !== index).map((c, i) => ({ ...c, index: i })),
  };
}
