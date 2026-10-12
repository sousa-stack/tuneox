# Architecture

TuneOx is a headless Rust core with thin front ends on top. Everything that touches audio, files or the database lives in the core. The desktop UI, the `tuneox` CLI and the MCP server are all clients of the same command layer, so anything a person can do in the UI, an LLM agent or a script can do too. That split is also what lets us hit the latency goals, and it keeps the UI replaceable.

```
 apps/desktop (Tauri 2)             tuneox CLI                 tuneox mcp
 React + TS UI, GPU waveforms       scripts, CI, power users   LLM agents (Claude etc.)
          │ Tauri IPC                      │                          │ MCP over stdio
          └───────────────────┬────────────┴──────────────────────────┘
                              ▼
┌──────────────────────────────── Rust core (crates/) ───────────────────────────┐
│  tuneox-commands   every user-facing operation as a typed command: schema,     │
│                    description, effect class, undo, dry run, progress          │
│  tuneox-core       domain types: Track, Cue, Loop, BeatGrid, Playlist, Waveform│
│  tuneox-library    SQLite library DB, FTS5 search, write-behind persistence    │
│  tuneox-analysis   decode, BPM + beatgrid, key, waveform generation            │
│  tuneox-audio      playback engine: decode, two decks, sync, mixer, output     │
│  tuneox-pioneer    rekordbox/Pioneer formats: master.db, ANLZ, PDB, OneLibrary │
│  tuneox-export     USB sync: diff library vs device, write files, copy audio   │
│  tuneox-mcp        MCP server over the command registry (stdio)                │
└────────────────────────────────────────────────────────────────────────────────┘
```

Crates are added as each piece gets built; this is the target layout.

## Automation: commands, CLI and MCP

TuneOx is built to be driven by an LLM as well as by hand, for requests like "build a 90-minute peak-time set from my tech house at 124–128 BPM in harmonic order", "put a memory cue on the first downbeat of every track in this playlist", or "export my Saturday playlists to the stick in the XDJ-AZ".

1. **Every feature is a command first.** A feature lands as a command in `tuneox-commands`: typed request and response (serde), a JSON Schema, and a description written for both people and models. The UI, the CLI and the MCP server are generated from or mapped onto that one registry. A feature that exists only in the UI is a bug.
2. **Three front doors, one behavior.** Tauri IPC for the desktop app, `tuneox <command> --json` for scripts and tests, `tuneox mcp` for agents. MCP tools come from the command registry; the library, playlists and tracks are also exposed as MCP resources.
3. **One process owns the library.** When the desktop app is running, it owns the database and serves the command layer on a local socket; the CLI and MCP server connect to it, so an agent's changes show up in the UI live and nothing is written twice. When the app isn't running, the CLI or MCP server opens the library itself (headless) and holds the lock.
4. **Agents get guard rails, not a separate API.** Every command declares an effect class: read, edit, destructive or device write. Edits are undoable and recorded in a history the UI shows ("Claude changed 42 cues — undo"). Destructive commands and device writes support `dry_run`, which returns exactly what would change, and require an explicit `confirm`.
5. **Built for bulk work.** Commands take filters and ID lists, so "every track in playlist X" is one call, not hundreds. Results are compact and structured with stable IDs. Long jobs (analysis, export) return a job ID with progress events.

## Latency rules

These are the reasons TuneOx should feel faster than rekordbox. Treat them as requirements.

1. **The UI never waits on disk.** Edits (cue set, rating, playlist reorder) apply to in-memory state and render on the next frame. A background write-behind queue persists them to SQLite (WAL mode). If a write fails, the UI is told and rolls back; it is never blocked waiting for one.
2. **Waveforms are computed once.** Analysis stores waveform data as compact binary at several zoom levels. The UI receives it once per track as raw bytes (no JSON) and keeps it on the GPU. Zooming, scrolling and drawing cue markers are pure redraws with no round trip to the core.
3. **The playhead is a clock, not a stream of positions.** The audio engine publishes `(sample_position, host_time, rate)` whenever playback state changes. The UI extrapolates the position every animation frame, so the waveform scrolls smoothly regardless of IPC timing.
4. **Cue points are snapped by the engine.** When the user sets a cue during playback, the UI draws it immediately at its extrapolated position and the engine returns the sample-accurate (and beat-snapped, if quantize is on) position, which replaces it within a frame.
5. **The audio thread never blocks.** No allocation, locks or I/O on the real-time thread. It talks to the rest of the core through lock-free queues.
6. **Library views are virtualized and paged.** Only visible rows are rendered or fetched. Search uses SQLite FTS5 plus in-memory indexes, so it stays instant at 100k+ tracks.
7. **Analysis uses every core** and is prioritized: the loaded track first, then tracks visible on screen, then the rest of the collection. It can be cancelled.

### Audio engine threads

`tuneox-audio` keeps the real-time path separate from everything else:

| Thread | Does | Talks through |
| --- | --- | --- |
| Audio callback (cpal) | Applies queued commands, renders both decks (cubic resampling, tempo, loops, scrubbing) and the browser's preview voice, sync, mixer. No allocation, locks, logging or I/O. | Pops commands from an `rtrb` SPSC queue; writes a state snapshot to a `triple_buffer` once per block (plain numbers only: cue colours and comments are joined in when the non-real-time side builds `DeckState`); pushes replaced tracks onto a "garbage" queue so they're freed elsewhere. |
| Output (`tuneox-audio-out`) | Opens the default device and owns the cpal stream (so the engine handle is `Send + Sync`), reopens it if the device disappears, or drives the core silently on a timer when there's no device. | |
| Events (`tuneox-audio-events`) | Every 5 ms: turns the latest snapshot into `DeckState`/`MixerState` events (on change, ~20 Hz while playing), frees garbage. | Engine event handler (the desktop app re-emits as `tuneox://deck` / `tuneox://mixer`). |
| Decoder (`tuneox-decode`, one per load) | symphonia decode into 16-bit stereo chunks (4 bytes/frame, ~10.6 MB per minute at 44.1 kHz) that the audio thread reads while decoding continues. | Atomics: chunks are published through `OnceLock`, the decoded frame count with release/acquire. |
| Callers (commands) | `Engine` methods queue a command and return once the audio thread has applied it (one device buffer, ~1–6 ms), with the resulting state. | The command queue (behind a mutex on the non-real-time side) and the snapshot. |

The audio callback reaches the DSP core through a `Mutex` it only ever `try_lock`s: in real-time mode nothing else takes that lock while a stream runs, so it's one atomic swap, and if it were ever contended the callback would output silence rather than wait. The offline engine (`Engine::offline`) runs the same core on the caller's thread, with time advanced by `render`, which is how the tests drive it.

Positions in the clock (`position_us` at `at_ms`) are what's heard: the callback adds the device's reported output latency to the time a block is rendered.

Commands that act on the engine (decks, mixer, preview player, player settings) set `Command::PLAYER`. The desktop app runs those on a second `Context` with its own library connection, so an import never delays a deck; it routes by that flag rather than by effect class, because some of them also save to the library (a hot cue stored from a pad, an A-GAIN trim) and are therefore effect `edit`.

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
