# rekordbox `master.db` (local library)

Observed on rekordbox 7.2.9 (macOS), against a real library of 10,361 tracks. Read with `crates/tuneox-pioneer/examples/rbdb.rs`.

## Location and encryption

- macOS: `~/Library/Pioneer/rekordbox/master.db`. rekordbox keeps rolling copies next to it (`master.backup.db`, `master.backup2.db`, `master.backup3.db`).
- SQLCipher 4 with default settings. A single fixed key; the same key is used by rekordbox 6 and 7. Setting `PRAGMA key` and nothing else unlocks it.
- No `-wal` / `-shm` files next to it while rekordbox is closed.
- The key is **not** in this repo. It's compiled in at build time from `TUNEOX_MASTERDB_KEY` (GitHub Actions secret, or the gitignored `.keys.env` locally). See `crates/tuneox-pioneer/src/keys.rs`.

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
- `djmdCue.Color` = `255` with `ColorTableIndex` = `0` seen on default-colored hot cues. TuneOx imports `ColorTableIndex` (when > 0) as the cue's palette color.
- `djmdContent.FileType`: 1 = MP3, 4 = M4A, 11 = WAV, 12 = AIFF (observed); 5 = FLAC per pyrekordbox.
- `djmdContent.ColorID`: 0 = none, 1–8 = Pink, Red, Orange, Yellow, Green, Aqua, Blue, Purple (`djmdColor.Commnt`).
- `djmdContent.Rating`: 0–5 stars.
- `djmdContent.FolderPath`: the full path of the audio file, including the file name.
- `djmdContent.Analysed`: 105 or 121 on every track (a bitmask; not decoded yet).
- `djmdContent.DJPlayCount` is sometimes stored as text, so read numbers type-agnostically.
- `djmdKey.ScaleName` mixes notations in one library: Camelot (`5A`, `11B`), classic names (`Am`, `Amin`, `Dmin`, `Em`), and a placeholder `All`. 787 of 10,361 tracks have no key.
- `djmdPlaylist.ParentID` is the literal string `root` for top-level playlists. `Seq` orders siblings starting at 1.
- `djmdProperty.DBID` identifies the library; USB exports carry it as OneLibrary `content.masterDbId`.

## Cue kinds

| `Kind` | Meaning | Evidence |
| --- | --- | --- |
| 0 | Memory cue or memory loop | 442 rows, 208 of them loops |
| 1, 2, 3 | Hot cues A, B, C | rekordbox 7.2.19 export: ANLZ PCO2 slots 1–3 at the same times |
| 5, 6, 7, 8, 9 | Hot cues D, E, F, G, H | Same export: a track with kinds 1, 2, 3, 5, 6, 7, 8 has PCO2 hot cues A–G |
| 4 | A loop with no hot cue slot | All 65 rows are loops, none colored, about 84% of the way into the track; 30 of the 65 tracks also have a kind 5 (D), so 4 isn't D |

Kind 1 cues are mostly rekordbox's automatic first-downbeat cue: 8,962 of 9,037 have a comment such as `1.1Bars`, at position 0.

How rekordbox exports kind 4 is still unknown. 64 of the 65 tracks are on the `ROCKITTRAX6` stick, so reading its ANLZ files will answer it. Until then TuneOx imports kind 4 as a memory loop and keeps the raw kind in `cue.source_kind`.

## Local analysis files

rekordbox keeps per-track analysis files under `~/Library/Pioneer/rekordbox/share/PIONEER/USBANLZ/<3 hex>/<uuid fragment>/` (3.9 GB for 10,361 tracks):

- `ANLZ0000.DAT`, `.EXT`, `.2EX`: same formats as on a USB export. The cue sections exist but are empty locally; cues live only in `djmdCue` and are written into ANLZ files at export time.
- `ANLZ0000.3EX` (10,353 files): not a PMAI file. It's MessagePack starting with `{"embedding": …}`, an audio embedding vector, presumably for track suggestions and related tracks. It never appears on USB exports.

## Open questions

- **`djmdPlaylist.Attribute`.** Every playlist in the sample library is `0`. TuneOx assumes 1 = folder and 4 = smart playlist (per pyrekordbox); confirm with a library that has them.
- **`contentCue` vs `djmdCue`.** Which one wins if they disagree, and does rekordbox 7 still read `djmdCue` at all? TuneOx reads `djmdCue`; its counts match the ANLZ files in a fresh export.
- **`djmdContent.Analysed`** bitmask meaning.
