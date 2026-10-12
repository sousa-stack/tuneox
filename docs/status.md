# Status

Where TuneOx stands. Update this in the same commit as the change it describes.

_Last updated: 2026-10-12_

## Done

**Foundation**
- Rust workspace + Tauri 2 desktop app, GPL-3.0, CI on macOS (fmt, clippy `-D warnings`, tests, frontend build).
- rekordbox database keys compiled in at build time from env / `.keys.env` / GitHub secrets, never in the repo.
- REA + Ghidra set up for reverse engineering rekordbox 7 (7.2.9 and 7.2.19 binaries studied).

**rekordbox import** (`import_rekordbox`)
- Reads rekordbox 6/7 `master.db` (SQLCipher) read-only from a temporary copy: tracks with full metadata and Auto Gain (`djmdMixerParam`), playlists and folders, smart playlists (rules stored, not evaluated), cues (memory, hot A–H, loops, colors, comments), beat grids from ANLZ `PQTZ`.
- Re-imports update instead of duplicating; duplicate files merge (first entry wins); dry run supported.
- Verified on a real 10,361-track library: every track, playlist and cue imports; cues and beat grids match what rekordbox 7.2.19 writes to a USB export. Import ~2.8 s, re-import ~1.2 s.

**Library** (`tuneox-library`)
- SQLite schema v2 (migrations from older versions run on open) with FTS5 search; search by text, BPM range, key and harmonic compatibility (Camelot neighbors).

**Commands, CLI and MCP**
- Command registry: `library_summary`, `list_playlists`, `get_playlist`, `search_tracks`, `list_tracks`, `get_track`, `get_waveform`, `import_rekordbox`.
- `tuneox` CLI over the registry; `tuneox mcp` serves them as MCP tools (protocol 2026-07-28 and legacy `initialize` clients), with progress notifications.

**Audio engine** (`tuneox-audio`; spec part C: [specs/decks.md](specs/decks.md))
- Two decks and a mixer for testing transitions (rekordbox's DUAL PLAYER): symphonia decoding of MP3, M4A (AAC-LC/ALAC), FLAC, WAV and AIFF into 16-bit stereo in memory (~10.6 MB per minute), playable while it decodes; all 113 files in the maintainers' test folder decode, at 1,000–4,000× real time.
- Transport, tempo and sync follow rekordbox 7's EXPORT-mode players (research part A): CUE, hot cues that jump and play (gate mode optional), memory cue calls, auto beat loops and manual loops with reloop/resize (sample-accurate wraps), beat jump, per-deck quantize, tempo ±6/10/16/100 % within 40–499 BPM with typed BPM and reset, pitch bend, beat/BPM sync with master handover and locked slave tempo; cubic resampling (vinyl mode).
- Mixer: the DUAL PLAYER's per-band kills, auto gain and crossfader, plus channel faders, isolator EQ and master volume as extras.
- cpal output on its own thread (follows the default device; runs silently on a timer without one), real-time-safe core (a counting allocator test proves the render path doesn't allocate), offline engine for tests.
- Commands (new effect class `playback`): `deck_load`, `deck_play`, `deck_pause`, `deck_cue`, `deck_hot_cue`, `deck_memory_cue`, `deck_seek`, `deck_beat_jump`, `deck_loop`, `deck_loop_in`, `deck_loop_out`, `deck_reloop`, `deck_loop_resize`, `deck_loop_exit`, `deck_set_tempo`, `deck_set_bpm`, `deck_tempo_reset`, `deck_set_tempo_range`, `deck_pitch_bend`, `deck_sync`, `deck_set_master`, `deck_set_quantize`, `player_set_hot_cue_gate`, `mixer_set`, `player_status`. The desktop app starts the engine at launch and forwards `tuneox://deck` / `tuneox://mixer` events.

**Desktop app**
- Library view: Collection and playlist tree, virtualized track table (10k+ tracks), instant local search and sorting, Camelot keys, responsive columns, detail panel with cues and beat grid summary, first-run import with live progress.

**Waveform data** (`get_waveform`)
- All six rekordbox waveforms (blue, RGB, 3-band × overview, detail) decode from ANLZ files on demand into `tuneox_core::Waveform` (~0.2 ms for a 5-minute detail waveform), with the 3-band gains (`PWVC`).

**Research** (see `research/`)
- `master.db` schema and field encodings, cue kinds (1–3, 5–9 = hot cues A–H; 4 = slotless loop exported as a memory loop), key notations, local ANLZ files carry no cues, `.3EX` = MessagePack audio embeddings.
- Waveforms (`waveforms.md`): every ANLZ waveform layout; 3-band bytes are low/mid/high (Deep Symmetry has the order wrong); `PWVC` = 3-band gains; `PVDI` = vocal detection.
- Decks (`rekordbox-decks.md`): rekordbox's DUAL PLAYER layout and controls, CUE/hot cue/memory cue/loop/quantize/beat jump rules, tempo and sync behavior, how each waveform style is drawn and colored, the 62-color hot cue palette and per-slot defaults, the EXPORT keyboard map.
- USB formats: which players read Device Library vs OneLibrary; a fresh rekordbox 7.2.19 export dissected (cues live in ANLZ files; OneLibrary `cue` table is empty; `.DAT` holds hot cues A–C only).

## In progress

**Decks: waveforms, preview player and dual player** (spec: [specs/decks.md](specs/decks.md))
- Parts A (research), B (waveform data) and C (audio engine + deck commands) are done; see above.
- Next: deck UI (part D) in rekordbox's DUAL PLAYER layout: stacked facing detail waveforms with one playhead, overviews, transport, tempo, sync, crossfader, band kills and Auto Gain, driven by the engine's `tuneox://deck` / `tuneox://mixer` events.

## Next

1. **OneLibrary USB export** (`exportLibrary.db` + ANLZ files with cues + audio copy), verified against the TUNEOX-TEST reference export and on an XDJ-AZ.
2. **Device Library USB export** (`export.pdb`/`exportExt.pdb`) for CDJ-3000 and older players.
3. Editing: cues, playlists, tags, ratings (undoable, write-behind).
4. Own analysis: BPM/beat grid, key, waveforms for tracks without rekordbox analysis.

## Known gaps

- The desktop app doesn't notice library changes made by another process (CLI/MCP) until it's reopened.
- Smart playlists are imported with their rules but not evaluated.
- Beat grids are stored per beat (~12 bytes/beat); a 10k-track library is ~106 MB. Can be packed much smaller without losing precision.
- The desktop app and MCP server both open the library file directly; the planned "one process owns the library" socket isn't built yet.
- Waveforms exist only for tracks rekordbox analyzed (we read its ANLZ files); our own waveform generation comes with our own analysis.
- rekordbox's normal scrolling waveform is computed from the audio at load time; drawing ANLZ data gives the CDJ / MIX POINT LINK look. Matching the smoother desktop look needs our own waveform generation.
- Remaining waveform/deck unknowns (exact RGB overview layering, played-portion dimming, crossfader curve, some defaults) are listed under "Open questions" in `research/rekordbox-decks.md` and `research/waveforms.md`. `PVDI` (vocal detection) isn't decoded.
- Auto Gain values above about +15 dB occur only on short sound effects; playback should cap the gain it applies.
- Decks: no key lock / master tempo yet (pitch follows tempo; rekordbox defaults it off; time-stretching is next, see the spec's "Later"), no headphone cue output, slip, reverse or jog (rekordbox's EXPORT players have none either).
- Decks: hot cues stored from the pads live only in the engine until cue editing exists; no memory cue set/delete, hot cue delete or automatic active loops yet.
- Decks: a few behaviors are still guesses (the spec's "Open behavior questions"): pitch bend amount, crossfader curve, some defaults, and whether MP3 encoder delay should be trimmed.
- Decks: the output is always the default device (no device picker); AAC is LC only (symphonia has no HE-AAC); 24-bit sources are stored at 16 bits.
- The desktop's `tuneox://deck` forwarding is built but hasn't been exercised in a running GUI yet.

## Decision log

- **2026-10-12** Stack: Rust core + Tauri 2 (React/TS) UI. The core is headless; every feature is a command.
- **2026-10-12** License GPL-3.0.
- **2026-10-12** rekordbox DB keys are injected at build time (GitHub secrets / `.keys.env`), never committed.
- **2026-10-12** Export order: OneLibrary first (XDJ-AZ test unit reads it; plain SQLite), then Device Library (CDJ-3000).
- **2026-10-12** LLM control is a core requirement: one command registry serves the UI, CLI and MCP.
- **2026-10-12** Every rekordbox paid feature is free in TuneOx ([feature-parity.md](feature-parity.md)).
- **2026-10-12** Imported tracks keep pointing at rekordbox's ANLZ files for waveform data instead of copying 3.9 GB of analysis.
- **2026-10-12** Big features are built by parallel agents in worktrees against a written spec in `docs/specs/`, orchestrated and merged by one session.
- **2026-10-12** Audio engine crates: symphonia (decoding, incl. AIFF), cpal (output), rtrb (command queue), triple_buffer (state out). Tracks are decoded fully into memory as 16-bit stereo (4 bytes/frame) rather than streamed: seeks, hot cues and loops never touch the disk, at ~11 MB per minute per deck.
- **2026-10-12** Playback commands are effect class `playback`. In the desktop app they run on their own `Context` (second library connection, shared engine) so library work never delays a deck.

## Test setup the maintainers use

- rekordbox 7.2.19 on macOS with a ~10k-track library.
- Hardware: XDJ-AZ (OneLibrary); CDJ-3000s (Device Library) available for testing.
- A reference USB export (one playlist, four AIFF tracks) and a real gig stick, kept locally in `.re/fixtures/` (never committed).
