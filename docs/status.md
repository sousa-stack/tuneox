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
- Single edits that leave the rest of a track alone: add a cue (a hot cue replaces its slot), delete a hot cue by slot or a memory cue by position, set the auto gain. Cues and gains edited in TuneOx are TuneOx's from then on (but see the re-import gap below).

**Commands, CLI and MCP**
- Command registry: `library_summary`, `list_playlists`, `get_playlist`, `search_tracks`, `list_tracks`, `get_track`, `get_beat_grid`, `get_waveform`, `import_rekordbox`, plus the deck and mixer commands (see the audio engine above).
- `tuneox` CLI over the registry; `tuneox mcp` serves them as MCP tools (protocol 2026-07-28 and legacy `initialize` clients), with progress notifications.

**Audio engine** (`tuneox-audio`; spec part C: [specs/decks.md](specs/decks.md))
- Two decks and a mixer for testing transitions (rekordbox's DUAL PLAYER): symphonia decoding of MP3, M4A (AAC-LC/ALAC), FLAC, WAV and AIFF into 16-bit stereo in memory (~10.6 MB per minute), playable while it decodes; all 113 files in the maintainers' test folder decode, at 1,000–4,000× real time.
- Transport, tempo and sync follow rekordbox 7's EXPORT-mode players (research part A): CUE, hot cues that jump and play (gate mode optional), memory cue calls, auto beat loops and manual loops with reloop/resize (sample-accurate wraps), beat jump, per-deck quantize, tempo ±6/10/16/100 % within 40–499 BPM with typed BPM and reset, pitch bend, beat/BPM sync with master handover and locked slave tempo; cubic resampling (vinyl mode).
- Mixer: the DUAL PLAYER's per-band kills, auto gain and crossfader, plus channel faders, isolator EQ and master volume as extras.
- cpal output on its own thread (follows the default device; runs silently on a timer without one), real-time-safe core (a counting allocator test proves the render path doesn't allocate), offline engine for tests.
- Cue editing saved to the library like rekordbox: hot cues stored from empty pads, hot cue delete, memory cue set (MEMORY: the active loop, the loop just played, or the cue point; up to 10 per track) and delete, and A-GAIN trims (saved as the track's auto gain, so reloads and previews use them). Cue colours and comments come with the deck state, joined off the audio thread.
- Smooth scrubbing (`deck_scrub`): the read head follows a drag like a record moved by hand (speed from the target over time, critically damped, level following speed, no per-move fades), then plays on or rests where it was let go.
- Browser preview player: a third voice that plays a track from a position at its own auto gain through the master; starting it pauses the decks, a deck starting stops it.
- Quantize resolution (rekordbox EXPORT's "Quantize Beat Value": 1/8, 1/4, 1/2, 1 beat) and, as a TuneOx extra, bar sync (beat sync aligning downbeats, at most half a bar's jump).
- Commands (effect `playback`, or `edit` when they can save to the library): `deck_load`, `deck_play`, `deck_pause`, `deck_cue`, `deck_hot_cue` (edit), `deck_hot_cue_delete` (edit), `deck_memory_cue`, `deck_memory_cue_set` (edit), `deck_memory_cue_delete` (edit), `deck_seek`, `deck_scrub`, `deck_beat_jump`, `deck_loop`, `deck_loop_in`, `deck_loop_out`, `deck_reloop`, `deck_loop_resize`, `deck_loop_exit`, `deck_set_tempo`, `deck_set_bpm`, `deck_tempo_reset`, `deck_set_tempo_range`, `deck_pitch_bend`, `deck_sync`, `deck_set_master`, `deck_set_quantize`, `player_set_hot_cue_gate`, `player_set_quantize_value`, `player_set_sync_phase`, `mixer_set` (edit), `preview_play`, `preview_stop`, `player_status`. The desktop app starts the engine at launch and forwards `tuneox://deck` / `tuneox://mixer` / `tuneox://preview` events.

**Desktop app**
- Library view: Collection and playlist tree, virtualized track table (10k+ tracks), instant local search and sorting, Camelot keys, responsive columns, detail panel with cues and beat grid summary, first-run import with live progress.
- Dual player (spec part D) above the library, after rekordbox's DUAL PLAYER: stacked facing detail waveforms with one shared playhead and bar counters, overviews, BLUE/RGB/3Band waveforms drawn with rekordbox's colours (3Band default), beat grid, hot cue/memory cue/loop markers, transport (CUE press/release, beat jump, loops, hot cues, memory cues), tempo −/+/typed BPM/reset, pitch bend, BEAT SYNC (orange when it falls back to BPM sync), MASTER, Q, DUAL CONTROL, A-GAIN, band kills, crossfader, mixer extras (EQ, channel faders, master), HOT CUE / MEMORY lists, loading by drag/context menu/double-click, rekordbox's EXPORT key map, error toasts. Cached waveform tiles and one animation loop driven by the engine clock (~1 ms of drawing per frame); frontend unit tests under `bun test`.
- Cue editing: MEMORY button, M / X (set / delete the memory cue the deck is on), ⌘1–3 (⌘⇧ for deck B) to clear hot cues A–C, ✕ on HOT CUE and MEMORY rows; cue colours and comments straight from the deck state; the detail panel refreshes after a deck edits its track.
- Dragging the enlarged waveform scrubs smoothly (`deck_scrub`); a playing deck plays on when let go.
- Preview column in the track table: overview thumbnails (only rows on screen fetch, cached) with a play/stop button; clicking previews from that spot, with a moving playhead.
- Toolbar: quantize resolution (Q 1/8 · 1/4 · 1/2 · 1) and SYNC BEAT · BAR, persisted and applied at launch. Error toasts say Deck A / Deck B in sentences. The track table only scrolls to the selection on keyboard moves or a search that still matches it.

**Waveform data** (`get_waveform`)
- All six rekordbox waveforms (blue, RGB, 3-band × overview, detail) decode from ANLZ files on demand into `tuneox_core::Waveform` (~0.2 ms for a 5-minute detail waveform), with the 3-band gains (`PWVC`).

**Research** (see `research/`)
- `master.db` schema and field encodings, cue kinds (1–3, 5–9 = hot cues A–H; 4 = slotless loop exported as a memory loop), key notations, local ANLZ files carry no cues, `.3EX` = MessagePack audio embeddings.
- MP3 timing (`mp3-timing.md`): rekordbox plays a non-FFmpeg Xing/Info header frame as a frame of silence (26.1 ms) and doesn't trim encoder delay; TuneOx's decoder matches it, so imported cues and grids line up on MP3s (verified on 50 tracks against rekordbox's own waveforms).
- Waveforms (`waveforms.md`): every ANLZ waveform layout; 3-band bytes are low/mid/high (Deep Symmetry has the order wrong); `PWVC` = 3-band gains; `PVDI` = vocal detection.
- Decks (`rekordbox-decks.md`): rekordbox's DUAL PLAYER layout and controls, CUE/hot cue/memory cue/loop/quantize/beat jump rules, tempo and sync behavior, how each waveform style is drawn and colored, the 62-color hot cue palette and per-slot defaults, the EXPORT keyboard map.
- USB formats: which players read Device Library vs OneLibrary; a fresh rekordbox 7.2.19 export dissected (cues live in ANLZ files; OneLibrary `cue` table is empty; `.DAT` holds hot cues A–C only).

## In progress

**Decks: waveforms, preview player and dual player** (spec: [specs/decks.md](specs/decks.md))
- Parts A (research), B (waveform data), C (audio engine + deck commands) and D (deck UI) are done, with the follow-ups (cue editing, scrubbing, preview player, quantize resolution, bar sync); see above.
- Next: key lock / master tempo.

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
- Cues and A-GAIN trims edited in TuneOx are overwritten by a later re-import from rekordbox (the import replaces a track's cues and gain). Keeping TuneOx's edits needs edit tracking in the library.
- Deck edits that save to the library write synchronously on the desktop's player context; during an import (which holds SQLite's write lock for ~3 s) such a save waits, and deck commands queue behind it. A write-behind queue (architecture "Latency rules" 1) isn't built yet.
- Decks: no automatic active loops, Smart Cue or cue colour/comment editing yet.
- Decks: a few behaviors are still guesses (the spec's "Open behavior questions"): pitch bend amount, crossfader curve, some defaults, a deck starting stopping the preview, MEMORY's "just after loop play", the scrub feel constants.
- Decks: the output is always the default device (no device picker); AAC is LC only (symphonia has no HE-AAC); 24-bit sources are stored at 16 bits.
- Deck UI gaps (spec part D, "Not built"): key shift / KEY SYNC / MT, INFO tab, phrase bars, artwork, the Matching button, preview keys (⌘P, ⌘[) and cue markers on preview thumbnails. How the scrub sounds hasn't been judged by ear yet.
- Hands-on QA in the real window (2026-10-12, muted): loading by double-click and drag, facing 3-band/RGB/BLUE waveforms, beat sync alignment across the shared playhead, 60 fps at ~0.3 ms drawing per frame on Retina, overview seek, CUE set, hot cue jump-and-play, palette colors, kills, mixer extras and master handover all work. Bar sync was added since as an option.
- Hands-on QA of the follow-ups (2026-10-12): the preview column and preview playback, bar sync, the quantize resolution, MEMORY (a quantized memory cue, saved to the library), ⌘3 deleting a hot cue (saved), A-GAIN trims (saved as the track's gain; double-click restores it), scrubbing a paused deck (rests where let go) and a playing one (plays on) all work. A scrub that looked stuck during QA (most likely a stale screenshot) showed that a lost pointer-up would have left the deck held; scrubs now also end on a pointer-up anywhere in the window, lost pointer capture or window blur, and the engine lets go after 1.5 s without a command. The ✕ on cue rows was `display: none` until hover, so automation couldn't reach it; it now fades in instead.
- The track detail panel shows the gain from when the library loaded, so it lags after an A-GAIN trim until the next reload.

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
- **2026-10-12** A command that can write the library is `edit` even when it also plays (`deck_hot_cue`, `mixer_set`, the cue edit commands). The desktop routes engine commands by a separate `player` flag on each command (`Command::PLAYER`), not by effect class.
- **2026-10-12** Cue edits made on a deck and A-GAIN trims save to the library at once, as rekordbox does; a trim becomes the track's `auto_gain_db` (rekordbox overwrites its gain the same way) rather than a separate column.
- **2026-10-12** Bar sync (beat sync aligning downbeats) is a TuneOx extra beyond rekordbox, off by default.
- **2026-10-12** A held scrub lets go by itself after 1.5 s without a command; the UI re-sends its position every 250 ms while the pointer is down. A lost pointer-up, a reloaded window or an agent that forgets `end` can't leave a deck held.

## Test setup the maintainers use

- rekordbox 7.2.19 on macOS with a ~10k-track library.
- Hardware: XDJ-AZ (OneLibrary); CDJ-3000s (Device Library) available for testing.
- A reference USB export (one playlist, four AIFF tracks) and a real gig stick, kept locally in `.re/fixtures/` (never committed).
