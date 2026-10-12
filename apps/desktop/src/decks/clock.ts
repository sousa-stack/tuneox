// The playhead is a clock, not a stream of positions (spec part C, "Clock and
// events"). The engine says where a deck was (`position_us` at `at_ms` on its
// own clock) and how fast it moves; the UI maps the engine clock onto
// `performance.now()` and extrapolates every frame.

import type { LoopState } from "./types";

/** The parts of a DeckState the clock needs. */
export interface ClockState {
  position_us: number;
  at_ms: number;
  rate: number;
  duration_us: number;
  loop: LoopState | null;
}

/**
 * Maps the engine clock onto `performance.now()`.
 *
 * Every event carries `sent_at_ms`; `performance.now() − sent_at_ms` is the
 * offset plus that event's delivery delay, so the smallest value seen is the
 * best estimate. Before the first event, a state's `at_ms` gives a rough
 * provisional offset that the first event replaces.
 */
export class EngineClock {
  private best: number | null = null;
  private provisional: number | null = null;

  /** An event sent at `sentAtMs` (engine clock) arrived at `now`. */
  observeEvent(sentAtMs: number, now: number): void {
    const offset = now - sentAtMs;
    if (this.best === null || offset < this.best) this.best = offset;
  }

  /** A state without `sent_at_ms` (a command reply or `player_status`). */
  observeState(atMs: number, now: number): void {
    if (this.best === null && this.provisional === null && atMs > 0) this.provisional = now - atMs;
  }

  get offset(): number | null {
    return this.best ?? this.provisional;
  }

  get synced(): boolean {
    return this.best !== null;
  }

  /** The engine clock now, in ms. */
  engineNow(now: number): number {
    return now - (this.offset ?? now);
  }
}

/**
 * Where a deck is at `engineMs`: `position_us + (engineMs − at_ms) × 1000 ×
 * rate`, wrapped inside an active loop the deck was in, and clamped to the
 * track.
 */
export function positionAt(state: ClockState, engineMs: number): number {
  let position = state.position_us;
  if (state.rate > 0 && state.at_ms > 0) {
    position += (engineMs - state.at_ms) * 1000 * state.rate;
    const loop = state.loop;
    if (loop && state.position_us < loop.end_us && position >= loop.end_us) {
      const length = loop.end_us - loop.start_us;
      if (length > 0) position = loop.start_us + ((position - loop.start_us) % length);
    }
  }
  if (position < 0) return 0;
  if (state.duration_us > 0 && position > state.duration_us) return state.duration_us;
  return position;
}

/** Tempo factor without a pitch bend: how much faster than the track's own tempo. */
export function tempoFactor(tempoPercent: number): number {
  const factor = 1 + tempoPercent / 100;
  return factor > 0.01 ? factor : 0.01;
}
