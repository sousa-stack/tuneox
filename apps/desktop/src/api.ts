// Typed access to TuneOx commands. Every call goes through the same command
// registry the CLI and MCP server use (crates/tuneox-commands).

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type TrackColor =
  | "pink"
  | "red"
  | "orange"
  | "yellow"
  | "green"
  | "aqua"
  | "blue"
  | "purple";

export interface Track {
  id: number;
  path: string;
  title: string;
  artist: string | null;
  album: string | null;
  genre: string | null;
  label: string | null;
  remixer: string | null;
  comment: string | null;
  /** Classic notation, e.g. "Am", "F#m", "Db". */
  key: string | null;
  bpm: number | null;
  duration_secs: number | null;
  rating: number;
  color: TrackColor | null;
  file_type: string | null;
  sample_rate: number | null;
  bit_depth: number | null;
  bitrate: number | null;
  file_size: number | null;
  play_count: number;
  year: number | null;
  track_number: number | null;
  date_added: string | null;
}

export type PlaylistKind = "folder" | "playlist" | "smart";

export interface Playlist {
  id: number;
  parent_id: number | null;
  name: string;
  kind: PlaylistKind;
  position: number;
  track_count: number;
}

export interface Cue {
  /** 0 = A … 7 = H; null for memory cues. */
  hot_cue: number | null;
  start_us: number;
  end_us: number | null;
  color: number | null;
  comment: string | null;
}

export interface GridSummary {
  beats: number;
  first_beat_us: number;
  first_beat_bar_position: number;
  constant_bpm: number | null;
}

export interface TrackDetails {
  track: Track;
  cues: Cue[];
  beat_grid: GridSummary | null;
}

export interface Summary {
  tracks: number;
  playlists: number;
  folders: number;
  cues: number;
  beat_grids: number;
  library_path: string | null;
}

export interface Counts {
  found: number;
  created: number;
  updated: number;
}

export interface ImportReport {
  dry_run: boolean;
  tracks: Counts;
  playlists: Counts;
  cues: number;
  beat_grids: number;
  missing_audio_files: number;
  missing_analysis_files: number;
  warnings: string[];
  elapsed_ms: number;
}

export interface Progress {
  command: string;
  stage: string;
  done: number;
  total: number;
}

function run<T>(name: string, input: object = {}): Promise<T> {
  return invoke<T>("run_command", { name, input });
}

export const api = {
  summary: () => run<Summary>("library_summary"),
  playlists: () =>
    run<{ playlists: Playlist[] }>("list_playlists").then((r) => r.playlists),
  playlistTrackIds: (id: number) =>
    run<{ track_ids: number[] }>("get_playlist", {
      id,
      include_tracks: false,
    }).then((r) => r.track_ids),
  tracksPage: (offset: number, limit: number) =>
    run<{ tracks: Track[]; total: number }>("list_tracks", { offset, limit }),
  track: (id: number) => run<TrackDetails>("get_track", { id }),
  importRekordbox: () => run<ImportReport>("import_rekordbox", {}),
};

export function onProgress(
  handler: (progress: Progress) => void,
): Promise<UnlistenFn> {
  return listen<Progress>("tuneox://progress", (event) =>
    handler(event.payload),
  );
}

/** Loads the whole library a page at a time, reporting each page as it lands. */
export async function loadAllTracks(
  onPage: (tracks: Track[], loaded: number, total: number) => void,
  pageSize = 5000,
): Promise<void> {
  let offset = 0;
  for (;;) {
    const page = await api.tracksPage(offset, pageSize);
    offset += page.tracks.length;
    onPage(page.tracks, offset, page.total);
    if (page.tracks.length < pageSize || offset >= page.total) return;
  }
}
