// Deck UI preferences. The waveform style, zoom and panel toggles persist in
// localStorage (per-viewer conveniences; the app works without it).

import { DEFAULT_JUMP, DEFAULT_LOOP_BEATS, JUMP_SIZES, LOOP_SIZES, type JumpSizeId } from "./grid";
import type { WaveformStyle } from "./types";

/**
 * Visible span of the detail waveforms in seconds, full width. rekordbox's
 * PERFORMANCE players step through 1–24 s with 8 s as the default; EXPORT's
 * steps aren't known (research §4.7), so TuneOx uses these, default 8 s.
 */
export const ZOOM_SPANS = [2, 4, 6, 8, 12, 16, 24, 32];
export const DEFAULT_ZOOM = 3;

export type TimeMode = "remaining" | "elapsed";

/** Quantize resolutions in beats (rekordbox EXPORT's "Quantize Beat Value"). */
export const QUANTIZE_VALUES = [0.125, 0.25, 0.5, 1] as const;
export type QuantizeValue = (typeof QUANTIZE_VALUES)[number];
/** What beat sync aligns: the beat (rekordbox) or the bar (a TuneOx extra). */
export type SyncPhase = "beat" | "bar";

export interface Settings {
  style: WaveformStyle;
  zoom: number;
  dualControl: boolean;
  jump: [JumpSizeId, JumpSizeId];
  loopBeats: [number, number];
  timeMode: [TimeMode, TimeMode];
  cueTab: ["hot" | "memory", "hot" | "memory"];
  mixerOpen: boolean;
  decksVisible: boolean;
  hud: boolean;
  quantizeBeats: QuantizeValue;
  syncPhase: SyncPhase;
}

const STORAGE_KEY = "tuneox.decks.v1";
const PERSISTED: (keyof Settings)[] = [
  "style",
  "zoom",
  "jump",
  "loopBeats",
  "timeMode",
  "cueTab",
  "mixerOpen",
  "decksVisible",
  "quantizeBeats",
  "syncPhase",
];

const defaults: Settings = {
  // rekordbox's default (kWaveColorTypeDefaultValue = 3Band).
  style: "three_band",
  zoom: DEFAULT_ZOOM,
  dualControl: false,
  jump: [DEFAULT_JUMP, DEFAULT_JUMP],
  loopBeats: [DEFAULT_LOOP_BEATS, DEFAULT_LOOP_BEATS],
  timeMode: ["remaining", "remaining"],
  cueTab: ["hot", "hot"],
  mixerOpen: false,
  decksVisible: true,
  hud: false,
  quantizeBeats: 1,
  syncPhase: "beat",
};

function valid(s: Settings): Settings {
  const styles: WaveformStyle[] = ["blue", "rgb", "three_band"];
  const jumpIds = JUMP_SIZES.map((j) => j.id as JumpSizeId);
  const pair = <T>(value: unknown, ok: (v: unknown) => boolean, fallback: [T, T]): [T, T] =>
    Array.isArray(value) && value.length === 2 && value.every(ok) ? (value as [T, T]) : fallback;
  return {
    ...s,
    style: styles.includes(s.style) ? s.style : defaults.style,
    zoom: Number.isInteger(s.zoom) && s.zoom >= 0 && s.zoom < ZOOM_SPANS.length ? s.zoom : DEFAULT_ZOOM,
    jump: pair(s.jump, (v) => jumpIds.includes(v as JumpSizeId), defaults.jump),
    loopBeats: pair(s.loopBeats, (v) => LOOP_SIZES.includes(v as number), defaults.loopBeats),
    timeMode: pair(s.timeMode, (v) => v === "remaining" || v === "elapsed", defaults.timeMode),
    cueTab: pair(s.cueTab, (v) => v === "hot" || v === "memory", defaults.cueTab),
    mixerOpen: s.mixerOpen === true,
    decksVisible: s.decksVisible !== false,
    quantizeBeats: QUANTIZE_VALUES.includes(s.quantizeBeats) ? s.quantizeBeats : 1,
    syncPhase: s.syncPhase === "bar" ? "bar" : "beat",
  };
}

function load(): Settings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return valid({ ...defaults, ...JSON.parse(raw) });
  } catch {
    // Storage unavailable or corrupt: defaults.
  }
  return { ...defaults };
}

type Listener = () => void;

class SettingsStore {
  private value: Settings = load();
  private listeners = new Set<Listener>();

  get(): Settings {
    return this.value;
  }

  update(change: Partial<Settings> | ((s: Settings) => Partial<Settings>)): void {
    const partial = typeof change === "function" ? change(this.value) : change;
    this.value = { ...this.value, ...partial };
    try {
      const persisted = Object.fromEntries(PERSISTED.map((k) => [k, this.value[k]]));
      localStorage.setItem(STORAGE_KEY, JSON.stringify(persisted));
    } catch {
      // Not persisted; fine.
    }
    for (const listener of this.listeners) listener();
  }

  /** Replaces one deck's entry of a per-deck pair. */
  updateDeck<K extends "jump" | "loopBeats" | "timeMode" | "cueTab">(key: K, deck: 1 | 2, value: Settings[K][number]): void {
    this.update((s) => {
      const pair = [...s[key]] as Settings[K];
      pair[deck - 1] = value;
      return { [key]: pair } as Partial<Settings>;
    });
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
}

export const settings = new SettingsStore();
