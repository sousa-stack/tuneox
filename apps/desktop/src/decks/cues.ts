// The engine reports hot cues and memory cues by position only; colours and
// comments come from the library's cues (get_track). Pads stored from the
// decks have no library cue yet and use their slot's default colour.

import type { Cue, TrackDetails } from "../api";
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

/** Library cues match engine cues within a millisecond (the library stores ms). */
const MATCH_US = 1500;

function find(cues: Cue[], hot: number | null, startUs: number): Cue | undefined {
  return cues.find((c) => c.hot_cue === hot && Math.abs(c.start_us - startUs) <= MATCH_US);
}

/** Hot cues A–H in slot order; null for an empty slot. */
export function hotCues(state: DeckState, details: TrackDetails | null): (ShownCue | null)[] {
  const library = details && details.track.id === state.track_id ? details.cues : [];
  const slots: (ShownCue | null)[] = new Array(8).fill(null);
  for (const cue of state.hot_cues) {
    if (cue.slot < 0 || cue.slot > 7) continue;
    const match = find(library, cue.slot, cue.start_us);
    slots[cue.slot] = {
      id: cue.slot,
      start_us: cue.start_us,
      end_us: cue.end_us,
      color: hotCueColor(cue.slot, match?.color, cue.end_us != null),
      comment: match?.comment ?? null,
    };
  }
  return slots;
}

export function memoryCues(state: DeckState, details: TrackDetails | null): ShownCue[] {
  const library = details && details.track.id === state.track_id ? details.cues : [];
  return state.memory_cues.map((cue) => {
    const match = find(library, null, cue.start_us);
    return {
      id: cue.index,
      start_us: cue.start_us,
      end_us: cue.end_us,
      color: memoryCueColor(match?.color),
      comment: match?.comment ?? null,
    };
  });
}
