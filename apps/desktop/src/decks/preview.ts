// The browser's preview player (rekordbox's Preview column, research §1.5):
// its state from the engine (`tuneox://preview` events, command replies),
// play/stop with an immediate optimistic update, and the playhead that moves
// across the previewing row's thumbnail.

import { positionAt } from "./clock";
import { commands } from "./commands";
import { store } from "./store";
import { toast } from "./toast";
import { idlePreview, type PreviewState } from "./types";

type Listener = () => void;

/** What a table row needs to re-render: which track previews, and whether it plays. */
export interface PreviewView {
  trackId: number | null;
  playing: boolean;
}

class PreviewStore {
  private auth: PreviewState = idlePreview();
  private optimistic: PreviewState | null = null;
  private guessId = 0;
  private viewValue: PreviewView = { trackId: null, playing: false };
  private listeners = new Set<Listener>();

  get state(): PreviewState {
    return this.optimistic ?? this.auth;
  }

  get view(): PreviewView {
    return this.viewValue;
  }

  /** A state from an event (with `sent_at_ms`) or a reply. Older states are ignored. */
  apply(state: PreviewState, sentAtMs?: number): void {
    const now = performance.now();
    if (sentAtMs !== undefined) store.clock.observeEvent(sentAtMs, now);
    else store.clock.observeState(state.at_ms, now);
    if (state.at_ms < this.auth.at_ms) return;
    this.auth = state;
    this.changed();
  }

  /** Shows `state` until `forget(id)` (when the engine has answered). */
  guess(state: PreviewState): number {
    this.optimistic = state;
    this.changed();
    return ++this.guessId;
  }

  /** Drops a guess, unless a newer one replaced it. */
  forget(id: number): void {
    if (id !== this.guessId || !this.optimistic) return;
    this.optimistic = null;
    this.changed();
  }

  private changed(): void {
    const s = this.state;
    if (s.track_id !== this.viewValue.trackId || s.playing !== this.viewValue.playing) {
      this.viewValue = { trackId: s.track_id, playing: s.playing };
    }
    for (const listener of this.listeners) listener();
  }

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
}

export const preview = new PreviewStore();

const engineNow = () => store.clock.engineNow(performance.now());

/** Where the preview is now. */
export function previewPosition(): number {
  return positionAt({ ...preview.state, loop: null }, engineNow());
}

/** Previews a track from a position; any playing deck pauses (the engine does that). */
export function previewPlay(trackId: number, positionUs: number, durationUs: number): void {
  const position = Math.max(0, durationUs > 0 ? Math.min(positionUs, durationUs) : positionUs);
  const id = preview.guess({
    ...preview.state,
    track_id: trackId,
    playing: true,
    error: null,
    position_us: position,
    at_ms: engineNow(),
    // Moving at once on screen; the reply corrects it if the file is still decoding.
    rate: 1,
    duration_us: durationUs || preview.state.duration_us,
  });
  commands
    .previewPlay(trackId, position)
    .then((state) => preview.apply(state))
    .catch(toast)
    .finally(() => preview.forget(id));
}

export function previewStop(): void {
  const id = preview.guess({ ...preview.state, playing: false, rate: 0, position_us: previewPosition(), at_ms: engineNow() });
  commands
    .previewStop()
    .then((state) => preview.apply(state))
    .catch(toast)
    .finally(() => preview.forget(id));
}

/** The play/stop button: stops the track that's playing, else plays (resuming the stopped one). */
export function previewToggle(trackId: number, durationUs: number): void {
  const s = preview.state;
  if (s.track_id === trackId && s.playing) previewStop();
  else previewPlay(trackId, s.track_id === trackId && s.position_us < (s.duration_us || durationUs) ? s.position_us : 0, durationUs);
}

// ── Playhead ────────────────────────────────────────────────────────────────
// One element at a time (the previewing row's), moved by its own animation
// loop that runs only while something previews.

const playheads = new Map<number, HTMLElement>();
let raf = 0;

function frame() {
  raf = 0;
  const s = preview.state;
  const element = s.track_id != null ? playheads.get(s.track_id) : undefined;
  if (element && s.duration_us > 0) {
    const fraction = Math.min(1, Math.max(0, previewPosition() / s.duration_us));
    element.style.left = `${(fraction * 100).toFixed(3)}%`;
    element.style.display = "block";
  }
  if (s.playing) raf = requestAnimationFrame(frame);
}

function kick() {
  if (!raf) raf = requestAnimationFrame(frame);
}

preview.subscribe(kick);

/** A row's playhead element; hidden unless that row's track is the one previewing. */
export function attachPlayhead(trackId: number, element: HTMLElement): () => void {
  playheads.set(trackId, element);
  kick();
  return () => {
    if (playheads.get(trackId) === element) playheads.delete(trackId);
  };
}
