// Live deck and mixer state, outside React. The animation loop reads it every
// frame; React subscribes to "views" that only change when something other
// than the clock changes, so a playing deck's 20 Hz clock updates never
// re-render components.

import { EngineClock } from "./clock";
import {
  emptyDeck,
  emptyMixer,
  type DeckId,
  type DeckState,
  type MixerState,
  type OutputInfo,
  type PlayerStatus,
} from "./types";

type Listener = () => void;

export interface DeckView {
  state: DeckState;
  /** Paused at the cue point (CUE lit). */
  atCue: boolean;
}

function viewKey(state: DeckState): string {
  const atCue = isAtCue(state);
  // Everything except where a moving playhead has got to. A variable grid's
  // BPM drifts beat by beat; the display shows two decimals.
  const round = (bpm: number | null) => (bpm == null ? null : Math.round(bpm * 100));
  return JSON.stringify({
    ...state,
    position_us: state.playing ? 0 : state.position_us,
    at_ms: 0,
    rate: state.rate > 0,
    bpm: round(state.bpm),
    original_bpm: round(state.original_bpm),
    atCue,
  });
}

function isAtCue(state: DeckState): boolean {
  return !state.playing && state.cue_us != null && Math.abs(state.position_us - state.cue_us) < 2000;
}

export class DeckStore {
  readonly clock = new EngineClock();
  private auth: [DeckState, DeckState] = [emptyDeck(1), emptyDeck(2)];
  private patches: [Map<number, Partial<DeckState>>, Map<number, Partial<DeckState>>] = [new Map(), new Map()];
  private effective: [DeckState, DeckState] = [this.auth[0], this.auth[1]];
  private views: [DeckView, DeckView] = [
    { state: this.auth[0], atCue: false },
    { state: this.auth[1], atCue: false },
  ];
  private keys: [string, string] = [viewKey(this.auth[0]), viewKey(this.auth[1])];
  private deckListeners: [Set<Listener>, Set<Listener>] = [new Set(), new Set()];
  private frameListeners = new Set<(deck: DeckId) => void>();
  private mixerListeners = new Set<Listener>();
  private nextPatch = 1;

  mixer: MixerState = emptyMixer();
  hotCueGate = false;
  output: OutputInfo | null = null;
  running = false;

  /** The current state of a deck, including optimistic patches. Read every frame. */
  deck(deck: DeckId): DeckState {
    return this.effective[deck - 1];
  }

  view(deck: DeckId): DeckView {
    return this.views[deck - 1];
  }

  /** A state from an event (with `sent_at_ms`) or a command reply. Older states are ignored. */
  applyDeck(state: DeckState, sentAtMs?: number): void {
    const now = performance.now();
    if (sentAtMs !== undefined) this.clock.observeEvent(sentAtMs, now);
    else this.clock.observeState(state.at_ms, now);
    const i = state.deck - 1;
    if (i !== 0 && i !== 1) return;
    if (state.at_ms < this.auth[i].at_ms) return;
    this.auth[i] = state;
    this.recompute(i);
  }

  applyStatus(status: PlayerStatus): void {
    for (const state of status.decks) this.applyDeck(state);
    this.applyMixer(status.mixer);
    this.hotCueGate = status.hot_cue_gate;
    this.output = status.output;
    this.running = status.running;
  }

  applyMixer(mixer: MixerState): void {
    this.mixer = mixer;
    for (const listener of this.mixerListeners) listener();
  }

  /** Shows `partial` on top of the engine's state until `unpatch`. */
  patch(deck: DeckId, partial: Partial<DeckState>): number {
    const id = this.nextPatch++;
    this.patches[deck - 1].set(id, partial);
    this.recompute(deck - 1);
    return id;
  }

  unpatch(deck: DeckId, id: number): void {
    if (this.patches[deck - 1].delete(id)) this.recompute(deck - 1);
  }

  private recompute(i: number): void {
    const patches = this.patches[i];
    const state = patches.size === 0 ? this.auth[i] : Object.assign({}, this.auth[i], ...patches.values());
    this.effective[i] = state;
    const deck = (i + 1) as DeckId;
    for (const listener of this.frameListeners) listener(deck);
    const key = viewKey(state);
    if (key !== this.keys[i]) {
      this.keys[i] = key;
      this.views[i] = { state, atCue: isAtCue(state) };
      for (const listener of this.deckListeners[i]) listener();
    }
  }

  subscribeDeck(deck: DeckId, listener: Listener): () => void {
    const set = this.deckListeners[deck - 1];
    set.add(listener);
    return () => set.delete(listener);
  }

  /** Every state change, including clock-only ones (for the animator). */
  subscribeFrames(listener: (deck: DeckId) => void): () => void {
    this.frameListeners.add(listener);
    return () => this.frameListeners.delete(listener);
  }

  subscribeMixer(listener: Listener): () => void {
    this.mixerListeners.add(listener);
    return () => this.mixerListeners.delete(listener);
  }
}

export const store = new DeckStore();
