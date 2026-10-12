# rekordbox `master.db` (local library)

Observed on rekordbox 7.2.9 (macOS), against a real library of 10,361 tracks. Read with `crates/tuneox-pioneer/examples/masterdb_schema.rs` and `masterdb_query.rs`.

## Location and encryption

- macOS: `~/Library/Pioneer/rekordbox/master.db`. rekordbox keeps rolling copies next to it (`master.backup.db`, `master.backup2.db`, `master.backup3.db`).
- SQLCipher 4 with default settings. A single fixed key; the same key is used by rekordbox 6 and 7. Setting `PRAGMA key` and nothing else unlocks it.
- No `-wal` / `-shm` files next to it while rekordbox is closed.
- The key is **not** in this repo. Code takes it from the caller (`TUNEOX_MASTERDB_KEY` in the research examples). How the shipping app obtains it is an open decision.

## Conventions shared by every table

- Every table carries sync bookkeeping: `UUID`, `rb_data_status`, `rb_local_data_status`, `rb_local_deleted`, `rb_local_synced`, `usn`, `rb_local_usn`, `created_at`, `updated_at`.
- Deletes are soft: filter on `rb_local_deleted = 0`.
- IDs are stored as text-like integers (e.g. `"239657308"`), not autoincrement.

## Tables we import (47 in total)

| Table | Rows (sample library) | What it holds |
| --- | --- | --- |
| `djmdContent` | 10,361 | One row per track: file path (`FolderPath`, `FileNameL`), tags via foreign keys (`ArtistID`, `AlbumID`, `GenreID`, `KeyID`, `LabelID`, `ColorID`), `BPM`, `Length`, `SampleRate`, `BitRate`, `Rating`, `DJPlayCount`, `AnalysisDataPath`, `Analysed` |
| `djmdPlaylist` | 120 | Tree of playlists: `ParentID`, `Seq` (order among siblings), `Attribute` (kind), `SmartList` (smart playlist rules) |
| `djmdSongPlaylist` | 14,685 | Playlist membership: `PlaylistID`, `ContentID`, `TrackNo` |
| `djmdCue` | 13,194 | Cues and loops, one row each (see below) |
| `contentCue` | 10,278 | **New in rekordbox 7.** Per-track JSON array copy of that track's `djmdCue` rows (`Cues` text column, `rb_cue_count`). Appears to exist for cloud sync. |
| `contentFile` | 47,405 | Files belonging to tracks (audio, analysis files) with `Hash`, `Size`, `rb_local_path`. Used by cloud sync. |
| `djmdArtist`, `djmdAlbum`, `djmdGenre`, `djmdLabel`, `djmdKey`, `djmdColor` | | Lookup tables referenced from `djmdContent` |
| `djmdMyTag`, `djmdSongMyTag` | | My Tag definitions and assignments |
| `djmdHistory`, `djmdSongHistory` | | Play history sessions |

## Field encodings

- `djmdContent.BPM`: integer, BPM × 100 (rekordbox's own built-in sampler entries have `BPM = 0`).
- `djmdContent.Length`: whole seconds.
- `djmdContent.AnalysisDataPath`: path of the track's `ANLZ0000.DAT`, relative to `~/Library/Pioneer/rekordbox/share`, e.g. `/PIONEER/USBANLZ/a6f/0475d-…/ANLZ0000.DAT`. The `.EXT` / `.2EX` siblings sit next to it.
- `djmdCue.InMsec` / `OutMsec`: milliseconds. `OutMsec = -1` means a cue point rather than a loop.
- `djmdCue.InFrame`: `InMsec × 150 / 1000`, truncated. 1/150 s is the CDJ "frame" unit (420 ms → 63, 1318 ms → 197).
- `djmdCue.CueMicrosec`: present but `0` on every row sampled; sub-millisecond precision may only be filled by newer analysis. Unverified.
- `djmdCue.Color` = `255` with `ColorTableIndex` = `0` seen on default-colored hot cues.

## Open questions

- **`djmdCue.Kind` values.** Observed distribution: 0 (442), 1 (9,037), 2 (1,299), 3 (1,211), 4 (65), 5 (720), 6 (314), 7 (86), 8 (18), 9 (2). 0 is a memory cue. Community docs map hot cues A–H to 1, 2, 3, 5, 6, 7, 8, 9 (skipping 4), but this library has 65 rows with `Kind = 4`. Find out what 4 is.
- **`djmdPlaylist.Attribute`.** Every playlist in the sample library is `0`, so folder and smart-list values still need confirming against a library that has them.
- **`contentCue` vs `djmdCue`.** Which one wins if they disagree, and does rekordbox 7 still read `djmdCue` at all?
