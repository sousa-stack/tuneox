// Local library model: the UI keeps every track in memory so search and
// sorting answer on the next frame, with no round trip to the core.

import type { Track, TrackColor } from "./api";

export interface TrackRow extends Track {
  /** Normalized searchable text, words separated by single spaces. */
  haystack: string;
  /** Camelot notation, e.g. "8A". */
  camelot: string | null;
}

export function toRow(track: Track): TrackRow {
  const text = [
    track.title,
    track.artist,
    track.album,
    track.genre,
    track.label,
    track.remixer,
    track.comment,
  ]
    .filter(Boolean)
    .join(" ");
  return {
    ...track,
    haystack: " " + words(text).join(" "),
    camelot: camelot(track.key),
  };
}

function words(text: string): string[] {
  return text
    .normalize("NFD")
    .replace(/\p{Diacritic}/gu, "")
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter(Boolean);
}

/** Every word of the query must start a word in the track's text. */
export function matcher(query: string): (row: TrackRow) => boolean {
  const terms = words(query);
  if (terms.length === 0) return () => true;
  return (row) => terms.every((term) => row.haystack.includes(" " + term));
}

const TONICS: Record<string, number> = {
  C: 0, "C#": 1, Db: 1, D: 2, "D#": 3, Eb: 3, E: 4, F: 5, "F#": 6, Gb: 6,
  G: 7, "G#": 8, Ab: 8, A: 9, "A#": 10, Bb: 10, B: 11,
};

/** "Ebm" → "2A", "C" → "8B". Mirrors MusicalKey::camelot in tuneox-core. */
export function camelot(key: string | null): string | null {
  if (!key) return null;
  const minor = key.endsWith("m");
  const tonic = TONICS[minor ? key.slice(0, -1) : key];
  if (tonic === undefined) return null;
  const base = minor ? 9 : 0;
  const steps = (((7 * (tonic - base)) % 12) + 12) % 12;
  return `${((steps + 7) % 12) + 1}${minor ? "A" : "B"}`;
}

export function formatDuration(secs: number | null): string {
  if (secs == null) return "";
  const s = Math.round(secs);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Microseconds → "1:05.739". */
export function formatPosition(us: number): string {
  const ms = Math.round(us / 1000);
  const minutes = Math.floor(ms / 60000);
  const seconds = Math.floor((ms % 60000) / 1000);
  return `${minutes}:${String(seconds).padStart(2, "0")}.${String(ms % 1000).padStart(3, "0")}`;
}

export const TRACK_COLORS: Record<TrackColor, string> = {
  pink: "#f06bbf",
  red: "#f04a4a",
  orange: "#f28c28",
  yellow: "#e8cc2e",
  green: "#3fc26b",
  aqua: "#2ec4c4",
  blue: "#4f8cff",
  purple: "#9a6bf0",
};

export type ColumnId =
  | "position"
  | "title"
  | "artist"
  | "album"
  | "genre"
  | "bpm"
  | "key"
  | "duration"
  | "rating"
  | "added";

export interface Column {
  id: ColumnId;
  label: string;
  /** CSS grid track size. */
  width: string;
  /** Narrowest the column can get, in px. */
  min: number;
  /** Columns with a drop rank are hidden, lowest rank first, when space runs out. */
  drop?: number;
  align?: "right" | "center";
}

export const COLUMNS: Column[] = [
  { id: "position", label: "#", width: "40px", min: 40, drop: 5, align: "right" },
  { id: "title", label: "Title", width: "minmax(140px, 3fr)", min: 140 },
  { id: "artist", label: "Artist", width: "minmax(110px, 2fr)", min: 110 },
  { id: "album", label: "Album", width: "minmax(90px, 1.4fr)", min: 90, drop: 1 },
  { id: "genre", label: "Genre", width: "minmax(80px, 1fr)", min: 80, drop: 2 },
  { id: "bpm", label: "BPM", width: "56px", min: 56, align: "right" },
  { id: "key", label: "Key", width: "40px", min: 40, align: "center" },
  { id: "duration", label: "Time", width: "44px", min: 44, align: "right" },
  { id: "rating", label: "Rating", width: "60px", min: 60, drop: 4 },
  { id: "added", label: "Added", width: "80px", min: 80, drop: 3 },
];

const COLUMN_GAP = 10;
/** Color strip, row padding and a scrollbar. */
const ROW_CHROME = 10 + 8 + 16 + 15;

/** The columns that fit in `width` px, dropping the least important first. */
export function fitColumns(width: number): Column[] {
  let columns = COLUMNS;
  const needed = (cols: Column[]) =>
    ROW_CHROME + cols.reduce((sum, c) => sum + c.min, 0) + COLUMN_GAP * cols.length;
  const droppable = COLUMNS.filter((c) => c.drop != null).sort((a, b) => a.drop! - b.drop!);
  for (const victim of droppable) {
    if (width === 0 || needed(columns) <= width) break;
    columns = columns.filter((c) => c !== victim);
  }
  return columns;
}

export interface Sort {
  column: ColumnId;
  descending: boolean;
}

const collator = new Intl.Collator(undefined, {
  numeric: true,
  sensitivity: "base",
});

/** Camelot wheel order: 1A, 1B, 2A, … 12B. */
function keyOrder(row: TrackRow): number | null {
  if (!row.camelot) return null;
  return parseInt(row.camelot, 10) * 2 + (row.camelot.endsWith("B") ? 1 : 0);
}

type Value = string | number | null;

function sortValue(item: Item, column: ColumnId): Value {
  const row = item.row;
  switch (column) {
    case "position":
      return item.position;
    case "title":
      return row.title || null;
    case "artist":
      return row.artist;
    case "album":
      return row.album;
    case "genre":
      return row.genre;
    case "bpm":
      return row.bpm;
    case "key":
      return keyOrder(row);
    case "duration":
      return row.duration_secs;
    case "rating":
      return row.rating;
    case "added":
      return row.date_added;
  }
}

/** A row as shown in a view: the track plus its position in that view. */
export interface Item {
  row: TrackRow;
  /** 1-based position in the playlist, or in the collection's default order. */
  position: number;
}

/** Sorts in place. Empty values always go last. */
export function sortItems(items: Item[], sort: Sort): Item[] {
  const direction = sort.descending ? -1 : 1;
  return items.sort((a, b) => {
    const x = sortValue(a, sort.column);
    const y = sortValue(b, sort.column);
    if (x == null || y == null) {
      if (x == null && y == null) return a.position - b.position;
      return x == null ? 1 : -1;
    }
    const order =
      typeof x === "number" && typeof y === "number"
        ? x - y
        : collator.compare(String(x), String(y));
    return order !== 0 ? order * direction : a.position - b.position;
  });
}
