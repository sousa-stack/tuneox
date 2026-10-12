// One requestAnimationFrame loop draws both decks: the detail waveforms, the
// overviews and the time displays. Positions come from the engine clock
// (never a per-frame core call) and nothing here goes through React.

import { positionAt } from "../clock";
import { hotCues, memoryCues, type ShownCue } from "../cues";
import { settings, ZOOM_SPANS } from "../settings";
import { store } from "../store";
import { trackData, type TrackData } from "../trackData";
import { DECKS, type DeckId, type DeckState } from "../types";
import { DetailView, type LaneInput } from "./detail";
import { OverviewView } from "./overview";

interface TimeDisplay {
  big: HTMLElement;
  small: HTMLElement;
  text: [string, string];
}

interface CueCache {
  state: DeckState | null;
  details: TrackData["details"] | undefined;
  hot: (ShownCue | null)[];
  memory: ShownCue[];
}

/** mm:ss.t, truncated to tenths. */
export function formatClock(us: number): string {
  const tenths = Math.max(0, Math.floor(us / 100000));
  const minutes = Math.floor(tenths / 600);
  const seconds = Math.floor((tenths % 600) / 10);
  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}.${tenths % 10}`;
}

class Animator {
  private detail: DetailView | null = null;
  private overviews: (OverviewView | null)[] = [null, null];
  private times: (TimeDisplay | null)[] = [null, null];
  private hud: HTMLElement | null = null;
  private cues: CueCache[] = [0, 1].map(() => ({ state: null, details: undefined, hot: [], memory: [] }));
  private raf = 0;
  private dirty = true;
  private stats = { work: 0, worst: 0, frames: 0, since: 0, tiles: 0 };

  constructor() {
    store.subscribeFrames(() => (this.dirty = true));
    trackData.subscribe(() => (this.dirty = true));
    settings.subscribe(() => (this.dirty = true));
  }

  attachDetail(canvas: HTMLCanvasElement): () => void {
    const view = new DetailView(canvas);
    this.detail = view;
    const stop = this.observeSize(canvas, (w, h, dpr) => view.resize(w, h, dpr));
    this.start();
    return () => {
      stop();
      if (this.detail === view) this.detail = null;
    };
  }

  attachOverview(deck: DeckId, canvas: HTMLCanvasElement): () => void {
    const view = new OverviewView(canvas);
    this.overviews[deck - 1] = view;
    const stop = this.observeSize(canvas, (w, h, dpr) => view.resize(w, h, dpr));
    this.start();
    return () => {
      stop();
      if (this.overviews[deck - 1] === view) this.overviews[deck - 1] = null;
    };
  }

  attachTime(deck: DeckId, big: HTMLElement, small: HTMLElement): () => void {
    const display: TimeDisplay = { big, small, text: ["", ""] };
    this.times[deck - 1] = display;
    this.start();
    return () => {
      if (this.times[deck - 1] === display) this.times[deck - 1] = null;
    };
  }

  attachHud(element: HTMLElement): () => void {
    this.hud = element;
    return () => {
      if (this.hud === element) this.hud = null;
    };
  }

  get detailView(): DetailView | null {
    return this.detail;
  }

  /** The lane input the detail view drew last for a deck (for pointer math). */
  lane(deck: DeckId): LaneInput {
    return this.laneInput(deck, store.clock.engineNow(performance.now()));
  }

  private observeSize(element: HTMLElement, resize: (w: number, h: number, dpr: number) => void): () => void {
    let media: MediaQueryList | null = null;
    const apply = () => {
      const rect = element.getBoundingClientRect();
      resize(rect.width, rect.height, window.devicePixelRatio || 1);
      this.dirty = true;
      // Moving to a display with another pixel ratio doesn't resize anything.
      media?.removeEventListener("change", apply);
      media = matchMedia(`(resolution: ${window.devicePixelRatio || 1}dppx)`);
      media.addEventListener("change", apply);
    };
    const observer = new ResizeObserver(apply);
    observer.observe(element);
    apply();
    return () => {
      observer.disconnect();
      media?.removeEventListener("change", apply);
    };
  }

  private start() {
    if (!this.raf) this.raf = requestAnimationFrame(this.tick);
  }

  private cuesFor(deck: DeckId): CueCache {
    const cache = this.cues[deck - 1];
    const state = store.view(deck).state;
    const details = trackData.peek(state.track_id)?.details;
    if (cache.state !== state || cache.details !== details) {
      cache.state = state;
      cache.details = details;
      cache.hot = hotCues(state, details ?? null);
      cache.memory = memoryCues(state, details ?? null);
    }
    return cache;
  }

  private laneInput(deck: DeckId, engineMs: number): LaneInput {
    const state = store.deck(deck);
    const position = positionAt(state, engineMs);
    const cues = this.cuesFor(deck);
    if (state.track_id == null) {
      return { state, position, wf: null, message: null, grid: null, hotCues: [], memoryCues: [] };
    }
    const resolved = trackData.detail(state.track_id, settings.get().style);
    const entry = trackData.peek(state.track_id);
    const message = state.error
      ? state.error
      : resolved.wf
        ? null
        : resolved.loading
          ? "Loading waveform…"
          : "No waveform: the track hasn't been analysed by rekordbox";
    return { state, position, wf: resolved.wf, message, grid: entry?.grid ?? null, hotCues: cues.hot, memoryCues: cues.memory };
  }

  private tick = (now: number) => {
    this.raf = requestAnimationFrame(this.tick);
    this.render(now);
  };

  /** Draws one frame for time `now` (a `performance.now()` value). */
  render(now: number, force = false): void {
    if (force) this.dirty = true;
    const started = performance.now();
    const engineMs = store.clock.engineNow(now);
    const s = settings.get();
    const lanes = DECKS.map((d) => this.laneInput(d, engineMs)) as [LaneInput, LaneInput];
    const moving = lanes.some((l) => l.state.rate > 0 && l.state.track_id != null);
    let drew = false;

    if (this.detail && s.decksVisible && (moving || this.dirty)) {
      const before = this.detail.tiles[0].rendered + this.detail.tiles[1].rendered;
      this.detail.draw(lanes, ZOOM_SPANS[s.zoom], 1);
      this.stats.tiles += this.detail.tiles[0].rendered + this.detail.tiles[1].rendered - before;
      drew = true;
    }

    for (const deck of DECKS) {
      const lane = lanes[deck - 1];
      const overview = this.overviews[deck - 1];
      if (overview && s.decksVisible) {
        const view = store.view(deck).state;
        const resolved = view.track_id != null ? trackData.preview(view.track_id, s.style) : null;
        const cues = this.cuesFor(deck);
        drew =
          overview.draw({
            state: view,
            position: lane.position,
            wf: resolved?.wf ?? null,
            style: resolved?.style ?? s.style,
            hotCues: cues.hot,
            memoryCues: cues.memory,
          }) || drew;
      }
      const time = this.times[deck - 1];
      if (time) {
        const state = lane.state;
        const loaded = state.track_id != null && state.duration_us > 0;
        const elapsed = loaded ? formatClock(lane.position) : "--:--.-";
        const remaining = loaded ? "-" + formatClock(state.duration_us - lane.position) : "--:--.-";
        const text: [string, string] = s.timeMode[deck - 1] === "remaining" ? [remaining, elapsed] : [elapsed, remaining];
        if (text[0] !== time.text[0]) time.big.textContent = text[0];
        if (text[1] !== time.text[1]) time.small.textContent = text[1];
        time.text = text;
      }
    }
    this.dirty = false;

    if (drew) {
      const work = performance.now() - started;
      this.stats.work += work;
      this.stats.worst = Math.max(this.stats.worst, work);
      this.stats.frames++;
    }
    if (this.hud && now - this.stats.since > 1000) {
      const { work, worst, frames, tiles } = this.stats;
      const elapsed = (now - this.stats.since) / 1000;
      this.hud.textContent = frames
        ? `draw ${(work / frames).toFixed(2)} ms avg · ${worst.toFixed(1)} ms worst · ${Math.round(frames / elapsed)} drawn/s · ${tiles} tiles`
        : "idle";
      this.stats = { work: 0, worst: 0, frames: 0, since: now, tiles: 0 };
    }
  }
}

export const animator = new Animator();
