# Architecture

TuneOx is a headless Rust core with a thin desktop UI on top. Everything that touches audio, files or the database lives in the core; the UI renders state and sends commands. That split is what lets us hit the latency goals, and it keeps the UI replaceable.

```
┌──────────────────────────── apps/desktop (Tauri 2) ────────────────────────────┐
│  React + TypeScript UI                                                         │
│  - library tables (virtualized), playlists, cue editor                         │
│  - waveforms drawn on the GPU (WebGL2) from data cached in the UI process      │
│                                                                                │
│        commands ▼            ▲ events (state diffs, playhead clock)            │
│  src-tauri: thin command/event layer, binary IPC for waveform data             │
└────────────────────────────────────────┬───────────────────────────────────────┘
                                         │
┌──────────────────────────────── Rust core (crates/) ───────────────────────────┐
│  tuneox-core       domain types: Track, Cue, Loop, BeatGrid, Playlist, Waveform│
│  tuneox-library    SQLite library DB, FTS5 search, write-behind persistence    │
│  tuneox-analysis   decode, BPM + beatgrid, key, waveform generation            │
│  tuneox-audio      playback engine (preview player now, decks later)           │
│  tuneox-pioneer    rekordbox/Pioneer formats: master.db, ANLZ, PDB, OneLibrary │
│  tuneox-export     USB sync: diff library vs device, write files, copy audio   │
└────────────────────────────────────────────────────────────────────────────────┘
```

Crates are added as each piece gets built; this is the target layout.

## Latency rules

These are the reasons TuneOx should feel faster than rekordbox. Treat them as requirements.

1. **The UI never waits on disk.** Edits (cue set, rating, playlist reorder) apply to in-memory state and render on the next frame. A background write-behind queue persists them to SQLite (WAL mode). If a write fails, the UI is told and rolls back; it is never blocked waiting for one.
2. **Waveforms are computed once.** Analysis stores waveform data as compact binary at several zoom levels. The UI receives it once per track as raw bytes (no JSON) and keeps it on the GPU. Zooming, scrolling and drawing cue markers are pure redraws with no round trip to the core.
3. **The playhead is a clock, not a stream of positions.** The audio engine publishes `(sample_position, host_time, rate)` whenever playback state changes. The UI extrapolates the position every animation frame, so the waveform scrolls smoothly regardless of IPC timing.
4. **Cue points are snapped by the engine.** When the user sets a cue during playback, the UI draws it immediately at its extrapolated position and the engine returns the sample-accurate (and beat-snapped, if quantize is on) position, which replaces it within a frame.
5. **The audio thread never blocks.** No allocation, locks or I/O on the real-time thread. It talks to the rest of the core through lock-free queues.
6. **Library views are virtualized and paged.** Only visible rows are rendered or fetched. Search uses SQLite FTS5 plus in-memory indexes, so it stays instant at 100k+ tracks.
7. **Analysis uses every core** and is prioritized: the loaded track first, then tracks visible on screen, then the rest of the collection. It can be cancelled.

## Device export

Pioneer/AlphaTheta hardware reads one of two on-device library formats. TuneOx writes both so a single USB works everywhere.

| Format | Database | Read by |
| --- | --- | --- |
| Device Library | `PIONEER/rekordbox/export.pdb` + `exportExt.pdb`, a paged binary database ("DeviceSQL") | CDJ-3000, CDJ-2000NXS2 and older, XDJ-XZ, XDJ-RX3, XDJ-RR, XDJ-1000MK2, XDJ-700 |
| OneLibrary (was "Device Library Plus") | `PIONEER/rekordbox/exportLibrary.db`, SQLCipher-encrypted SQLite | CDJ-3000X, XDJ-AZ, OPUS-QUAD, OMNIS-DUO, and AlphaTheta's upcoming products |

Both formats point at the same per-track analysis files (`PIONEER/USBANLZ/**/ANLZ0000.DAT/.EXT/.2EX`) and the same audio under `Contents/`. An export writes those once, then writes the two databases. Details, including what a real rekordbox stick contains, are in [research/usb-export.md](../research/usb-export.md).

OneLibrary comes first in milestone 1: it's plain SQLite, so it's the quicker writer to get right, and it's what our main test player (an XDJ-AZ) reads. Device Library follows for the CDJ-3000 and older gear.

Export is incremental. Each track has a content hash covering its metadata, cues, grid and analysis; only tracks whose hash changed get their ANLZ files rewritten or audio re-copied. The PDB is small and is regenerated on every sync.

### Proving compatibility

"Works on all players" has to be tested, not assumed:

1. **Golden comparisons.** Export the same playlists with rekordbox and with TuneOx, parse both, and diff them field by field. Differences must be explained in `research/`.
2. **Independent readers.** TuneOx exports must parse cleanly in rekordcrate, crate-digger and pyrekordbox, not only in our own reader.
3. **rekordbox as a validator.** rekordbox can browse a USB device's library. A TuneOx export should show up there with correct tracks, cues and waveforms.
4. **Hardware matrix.** Real players, tracked per model and firmware version.

## rekordbox import

`tuneox-pioneer` reads the local rekordbox library (`master.db`, SQLCipher) plus its ANLZ files, and maps tracks, playlists, cues, beatgrids and waveforms into the TuneOx library without re-analyzing. The user's live rekordbox files are opened read-only and are never modified.
