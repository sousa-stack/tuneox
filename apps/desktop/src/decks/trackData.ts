// Per-track data the decks draw: details (cues with colours and comments),
// the beat grid and waveforms, fetched once per track and kept for a few
// tracks so reloading one is instant.

import { api, type TrackDetails } from "../api";
import { commands } from "./commands";
import { toBeatGrid, type BeatGrid } from "./grid";
import type { WaveformResolution, WaveformStyle } from "./types";
import { overviewSource, toWaveform, type Waveform } from "./waveform";

export interface TrackData {
  trackId: number;
  details: TrackDetails | null;
  grid: BeatGrid | null;
  /** `${style}/${resolution}` → waveform, or null when the track has none. */
  waveforms: ReadonlyMap<string, Waveform | null>;
  pending: ReadonlySet<string>;
  error: string | null;
}

type Listener = () => void;

const KEEP = 8;
const STYLES: WaveformStyle[] = ["three_band", "rgb", "blue"];
const key = (style: WaveformStyle, resolution: WaveformResolution) => `${style}/${resolution}`;

export interface Resolved {
  wf: Waveform | null;
  /** The style drawn: the chosen one, or a fallback the track has. */
  style: WaveformStyle;
  loading: boolean;
}

class TrackDataCache {
  private entries = new Map<number, TrackData>();
  private listeners = new Set<Listener>();
  /** Bumps on every change, for the animator. */
  version = 0;

  peek(trackId: number | null): TrackData | null {
    return trackId == null ? null : (this.entries.get(trackId) ?? null);
  }

  /** Starts loading a track's details and grid if needed. */
  ensure(trackId: number): TrackData {
    const existing = this.entries.get(trackId);
    if (existing) {
      // Most recently used last.
      this.entries.delete(trackId);
      this.entries.set(trackId, existing);
      return existing;
    }
    const entry: TrackData = {
      trackId,
      details: null,
      grid: null,
      waveforms: new Map(),
      pending: new Set(["details", "grid"]),
      error: null,
    };
    this.entries.set(trackId, entry);
    while (this.entries.size > KEEP) this.entries.delete(this.entries.keys().next().value!);
    api
      .track(trackId)
      .then((details) => this.change(trackId, (e) => ({ details, pending: without(e.pending, "details") })))
      .catch((err) => this.change(trackId, (e) => ({ error: String(err), pending: without(e.pending, "details") })));
    commands
      .beatGrid(trackId)
      .then((r) => this.change(trackId, (e) => ({ grid: r.beats.length ? toBeatGrid(r.beats) : null, pending: without(e.pending, "grid") })))
      .catch(() => this.change(trackId, (e) => ({ pending: without(e.pending, "grid") })));
    this.emit();
    return entry;
  }

  /** The detail waveform to draw in `style`, falling back to another style the track has. */
  detail(trackId: number, style: WaveformStyle): Resolved {
    return this.resolve(trackId, [style, ...STYLES.filter((s) => s !== style)], "detail", (s) => s);
  }

  /** The overview to draw in `style` (BLUE and RGB both come from the RGB preview). */
  preview(trackId: number, style: WaveformStyle): Resolved {
    return this.resolve(trackId, [style, ...STYLES.filter((s) => s !== style)], "preview", overviewSource);
  }

  private resolve(
    trackId: number,
    candidates: WaveformStyle[],
    resolution: WaveformResolution,
    source: (style: WaveformStyle) => WaveformStyle,
  ): Resolved {
    const entry = this.ensure(trackId);
    for (const style of candidates) {
      const k = key(source(style), resolution);
      let loading = entry.pending.has(k);
      if (!loading && !entry.waveforms.has(k)) {
        this.fetch(trackId, source(style), resolution);
        loading = true;
      }
      if (loading) {
        // Keep showing a style that's already here (e.g. right after switching styles).
        const shown = candidates.find((s) => entry.waveforms.get(key(source(s), resolution)));
        const wf = shown ? entry.waveforms.get(key(source(shown), resolution))! : null;
        return { wf, style: shown ?? style, loading: true };
      }
      const wf = entry.waveforms.get(k);
      if (wf) return { wf, style, loading: false };
    }
    return { wf: null, style: candidates[0], loading: false };
  }

  private fetch(trackId: number, style: WaveformStyle, resolution: WaveformResolution) {
    const k = key(style, resolution);
    this.change(trackId, (e) => ({ pending: new Set([...e.pending, k]) }));
    commands
      .waveform(trackId, style, resolution)
      .then((r) => toWaveform(r))
      .catch(() => null) // No analysis, or not this style: try the next.
      .then((wf) =>
        this.change(trackId, (e) => ({
          waveforms: new Map([...e.waveforms, [k, wf]]),
          pending: without(e.pending, k),
        })),
      );
  }

  private change(trackId: number, update: (entry: TrackData) => Partial<TrackData>) {
    const entry = this.entries.get(trackId);
    if (!entry) return;
    this.entries.set(trackId, { ...entry, ...update(entry) });
    this.emit();
  }

  private emit() {
    this.version++;
    for (const listener of this.listeners) listener();
  }

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
}

function without<T>(set: ReadonlySet<T>, item: T): Set<T> {
  const next = new Set(set);
  next.delete(item);
  return next;
}

export const trackData = new TrackDataCache();
