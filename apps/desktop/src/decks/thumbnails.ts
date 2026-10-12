// Overview thumbnails for the browser's Preview column: the same 1200-column
// preview waveform the decks' overviews use (research §1.5), fetched only for
// rows on screen, a few at a time with the most recently shown rows first,
// and cached (waveforms and painted pixels) so scrolling back is free.

import { FetchQueue, Lru } from "./cache";
import { commands } from "./commands";
import { overviewSource, paintOverview, toWaveform, type Waveform } from "./waveform";
import type { WaveformStyle } from "./types";

/** A thumbnail's data: the waveform and the style to paint it in, or null without analysis. */
export interface Thumbnail {
  wf: Waveform;
  style: WaveformStyle;
}

type Source = "three_band" | "rgb";
const SOURCES: Source[] = ["three_band", "rgb"];

const STYLES: WaveformStyle[] = ["three_band", "rgb", "blue"];

/** `${trackId}/${source}` → waveform, or null when the track has none in that style. */
const waveforms = new Lru<string, Waveform | null>(3000);
/** `${trackId}|${style}` → what to draw. */
const resolved = new Lru<string, Thumbnail | null>(3000);
const listeners = new Map<number, Set<() => void>>();

const sourceKey = (trackId: number, source: Source) => `${trackId}/${source}`;

/** The sources to try for a style: its own, then the other (a track may lack one). */
function sourcesFor(first: Source): Source[] {
  return [first, ...SOURCES.filter((s) => s !== first)];
}

/** The thumbnail to draw for `style`; undefined while still unknown. */
function resolve(trackId: number, style: WaveformStyle): Thumbnail | null | undefined {
  const first = overviewSource(style) as Source;
  for (const source of sourcesFor(first)) {
    if (!waveforms.has(sourceKey(trackId, source))) return undefined;
    const wf = waveforms.get(sourceKey(trackId, source));
    if (wf) return { wf, style: source === first ? style : source };
  }
  return null;
}

/** Fetch keys: `${trackId}|${first source}`. */
const queue = new FetchQueue<string>(
  4,
  async (key) => {
    const [id, first] = key.split("|");
    const trackId = Number(id);
    for (const source of sourcesFor(first as Source)) {
      const k = sourceKey(trackId, source);
      if (!waveforms.has(k)) {
        const wf = await commands
          .waveform(trackId, source, "preview")
          .then(toWaveform)
          .catch(() => null);
        waveforms.set(k, wf);
      }
      if (waveforms.get(k)) break;
    }
    for (const style of STYLES) resolved.delete(`${trackId}|${style}`);
    for (const listener of listeners.get(trackId) ?? []) listener();
  },
  (key) => {
    const [id, first] = key.split("|");
    return resolve(Number(id), first as Source) !== undefined;
  },
);

/**
 * Interest in a track's thumbnail in `style` while a row shows it;
 * `onChange` fires when its data arrives.
 */
export function watchThumbnail(trackId: number, style: WaveformStyle, onChange: () => void): () => void {
  let set = listeners.get(trackId);
  if (!set) listeners.set(trackId, (set = new Set()));
  set.add(onChange);
  const release = queue.want(`${trackId}|${overviewSource(style)}`);
  return () => {
    release();
    set.delete(onChange);
    if (set.size === 0) listeners.delete(trackId);
  };
}

/** The current thumbnail data for a track (a stable object until new data arrives). */
export function thumbnailData(trackId: number, style: WaveformStyle): Thumbnail | null | undefined {
  const key = `${trackId}|${style}`;
  if (resolved.has(key)) return resolved.get(key);
  const value = resolve(trackId, style);
  if (value !== undefined) resolved.set(key, value);
  return value;
}

const images = new Lru<string, ImageData>(400);

/** Painted pixels for a thumbnail at a size, cached. */
export function thumbnailImage(trackId: number, thumb: Thumbnail, width: number, height: number): ImageData {
  const key = `${trackId}|${thumb.style}|${width}x${height}|${thumb.wf.columns}`;
  const cached = images.get(key);
  if (cached) return cached;
  const image = new ImageData(width, height);
  paintOverview(thumb.wf, thumb.style, width, height, new Uint32Array(image.data.buffer));
  images.set(key, image);
  return image;
}
