# Spec: waveforms, preview player and dual player

Status: **in progress** (2026-10-12). Owners are listed per part. If the implementation has to deviate from this contract, update this file in the same commit.

## Goal

Load any library track onto one of two decks, see its waveform the way rekordbox draws it, play it, set and trigger cues, change tempo, sync the two decks and mix between them with faders and a crossfader, so a DJ can test transitions on a laptop like rekordbox's **DUAL PLAYER**. It must feel tight: controls act on the next frame and waveforms scroll smoothly in lockstep with what you hear.

Out of scope for this round: headphone cueing on a separate output, effects, recording, controllers/MIDI, key lock (time-stretching; planned next, see "Later").

## Parts and owners

| Part | Owner | Files |
| --- | --- | --- |
| A. rekordbox behavior research | research agent | `research/rekordbox-decks.md` |
| B. Waveform data | waveform agent | `crates/tuneox-core/src/waveform.rs`, `crates/tuneox-pioneer/src/anlz.rs` (+ `anlz/` submodules), `crates/tuneox-commands/src/waveform.rs`, `research/waveforms.md` |
| C. Audio engine + deck commands | engine agent | `crates/tuneox-audio/` (new), `crates/tuneox-commands/src/decks.rs`, `Context`/`Effect` in `crates/tuneox-commands/src/lib.rs`, annotations in `crates/tuneox-mcp/src/lib.rs`, event forwarding in `apps/desktop/src-tauri/src/lib.rs` |
| D. Deck UI | UI agent (after A–C land) | `apps/desktop/src/` |

Shared edits that will conflict (orchestrator merges): the `registry![…]` list and `mod` lines in `crates/tuneox-commands/src/lib.rs`, `[workspace.dependencies]` in the root `Cargo.toml`, `docs/status.md`.

## B. Waveform data

Status: **done** (2026-10-12). Formats, evidence and verification: [research/waveforms.md](../../research/waveforms.md).

rekordbox's analysis files hold three waveform styles, each at two resolutions (see Deep Symmetry's DJ Link analysis for the tags):

| Style | Preview (whole track) | Detail (scrolling, 150 columns/s) |
| --- | --- | --- |
| `blue` (monochrome) | `PWAV` in `.DAT`, 400 columns | `PWV3` in `.EXT` |
| `rgb` (color) | `PWV4` in `.EXT`, 1200 columns | `PWV5` in `.EXT` |
| `three_band` (CDJ-3000 style) | `PWV6` in `.2EX`, 1200 columns | `PWV7` in `.2EX` |

`PWVC` (in `.2EX`) is rekordbox's "wave normalize coefficient": the gain applied to each 3-band band (low, mid, high; u16 × 100), already baked into `PWV6`/`PWV7`. It comes along as `band_gains` in case the renderer needs it (see research for why that's not fully settled). The 3-band entries are stored **low, mid, high**, not mid, high, low as Deep Symmetry documents.

**Core type** (`tuneox-core`): a renderer-friendly, Pioneer-independent waveform that our own analysis will also produce later:

```rust
pub enum WaveformStyle { Blue, Rgb, ThreeBand }
pub enum WaveformResolution { Preview, Detail }
pub struct Waveform {
    pub style: WaveformStyle,
    /// Columns per second of audio for detail waveforms; None for previews,
    /// whose columns span the whole track.
    pub columns_per_second: Option<f64>,
    pub columns: u32,
    /// Fixed-size records, one per column (layouts below).
    pub data: Vec<u8>,
    /// three_band only: the gain the analysis applied to each band.
    pub band_gains: Option<BandGains>,   // { low, mid, high }: f64
}
```

Records. Every value is a byte where 255 is the full scale of the source field; narrower Pioneer fields are widened with `round(v × 255 / max)`, which `round(x × max / 255)` reverses exactly (`tuneox_pioneer::anlz::{widen, narrow}`).

| Style | Resolution | Bytes | Record |
| --- | --- | --- | --- |
| `blue` | both | 2 | height (from 0–31), whiteness (from 0–7; rekordbox draws 8 tints, index = `round(w × 7 / 255)`) |
| `rgb` | detail | 4 | red, green, blue (from 0–7 each), height (from 0–31) |
| `rgb` | preview | 6 | `PWV4`'s six bytes unchanged: [0] max amplitude, [1] min of a full-band measure, [2] another amplitude measure, [3] low, [4] mid, [5] high band maxima (mostly 0–127; how rekordbox combines them is open) |
| `three_band` | both | 3 | low, mid, high (from 0–127; final levels, already smoothed, gain-adjusted and, for high, curved by rekordbox) |

Decoders: `AnlzFile::waveform(style, resolution) -> Result<Option<Waveform>>` (missing section → `None`, corrupt → `Error::Corrupt`), `AnlzFile::three_band_gains()`, `anlz::waveform_file_extension(style, resolution)`.

**Command** `get_waveform` (effect `read`):

```jsonc
// input
{ "track_id": 123, "style": "rgb", "resolution": "detail" }   // resolution: "preview" | "detail"
// output
{ "track_id": 123, "style": "rgb", "resolution": "detail",
  "columns": 45909, "columns_per_second": 150.0,
  "bytes_per_column": 4, "data_base64": "…",
  "band_gains": null }   // three_band: { "low": 0.83, "mid": 1.13, "high": 1.03 }
```

Reads the track's `ANLZ0000.DAT`/`.EXT`/`.2EX` from `analysis_dir` on demand (nothing new stored in the library). An unknown track, a track without analysis, a missing file or a missing section each return `NotFound` with a message saying which; a corrupt file is an `InvalidData` I/O error. Measured on a 306 s track (release build): 0.15–0.21 ms per detail waveform, 0.03 ms per preview, under 1 ms on a cold first read.

**Done when:** decoders are unit-tested against synthetic files built in the tests, cross-checked by hand against real files in `.re/fixtures/` (decoded heights/colors plausible, column counts match track length × 150), and `research/waveforms.md` documents every layout we rely on. All met.

## C. Audio engine and deck commands

Status: **built** (engine agent, 2026-10-12), with transport, sync and mixer behavior updated to rekordbox 7's EXPORT-mode players from research part A ([research/rekordbox-decks.md](../../research/rekordbox-decks.md) §2–3). Follow-ups the same day: cue colours and comments in `DeckState`, cue edits saved to the library (hot cue store/delete, memory cue set/delete, A-GAIN trims), smooth scrubbing, the browser preview voice, the quantize resolution and bar sync. What's still open is listed under "Open behavior questions".

### Engine (`crates/tuneox-audio`)

- **Two decks** and a **mixer**: per-deck band kills, auto gain, volume (channel fader) and 3-band EQ (low/mid/high, −∞ to +6 dB, kill at minimum), crossfader, master volume. One stereo output (the default device).
- **Decoding** with `symphonia` (MP3, AAC/M4A, FLAC, WAV, **AIFF**; the test library is mostly MP3 and AIFF). Decode on a background thread into memory in a compact form; playback can start as soon as the first second is decoded. Report decode errors as deck state, never panic.
- **Tempo** (vinyl mode for now: pitch follows tempo): fractional read position with good interpolation (at least 4-point cubic), resampling track rate → device rate in the same step. Pitch fader range ±6, ±10, ±16 % or WIDE (±100 %); effective tempo 40–499 BPM.
- **Sync** using the library beat grid: matches the master deck's effective BPM and (beat sync) aligns beat phase. One deck is master (the first one to start playing, or chosen explicitly). Sync stays engaged until turned off.
- **Quantize** (per deck, on by default, 1 beat): cue set, loop in/out, hot cue store, auto loop start and hot cue jumps snap to the beat grid.
- **Transport** semantics follow rekordbox's EXPORT-mode players (research part A §2).
- **Real-time rules:** the audio callback never allocates, locks, logs or does I/O. Control goes in through a lock-free queue; state comes out through lock-free snapshots. The cpal stream lives on a dedicated thread, so the engine handle is `Send + Sync`.
- **Testable offline:** the DSP core renders into a buffer without an audio device (`render(&mut self, out: &mut [f32])`), so tests can load synthetic audio (generated in the test: clicks on a known beat grid, tones), drive transport and assert on samples, positions, tempo and phase alignment.

#### As built

- **Crates:** `symphonia` 0.6 (MPL-2.0; features `mp3 aac isomp4 alac flac pcm wav aiff all-meta`; AIFF is native in 0.6, so no custom reader), `cpal` 0.18 (Apache-2.0; follows the macOS default output when it changes), `rtrb` 0.4 (MIT/Apache-2.0; wait-free SPSC queue), `triple_buffer` 9 (MPL-2.0; latest-value state hand-off). All GPL-3.0-compatible.
- **PCM in memory:** 16-bit stereo, one frame packed in an `AtomicU32` (4 bytes/frame): **10.6 MB per minute at 44.1 kHz, 11.5 MB at 48 kHz**. Stored in 32 768-frame chunks the decoder publishes through `OnceLock`, plus a release/acquire frame counter, so the audio thread plays a track while it decodes (no locks, no `unsafe`). Decoding runs at 1,000–4,000× real time (a 5-minute AIFF in ~60 ms, an MP3 in ~250 ms on an M-series Mac); all 113 files in the maintainer's test folder (MP3, AIFF, WAV, M4A) decode. Loading the track that's on the other deck reuses its audio (instant double).
- **Read head:** position in seconds (`f64`), advanced `rate / device_rate` per output sample, read with 4-point Catmull-Rom interpolation. Loop wraps keep the fractional overshoot, so loops are exact to the sample at any rate. Jumps, starts and stops fade over 128 samples (~3 ms); loop wraps don't.
- **Threads:** audio callback (DSP core), `tuneox-audio-out` (owns the cpal stream; reopens it if the device disappears; with no device it drives the core silently on a timer so decks keep moving), `tuneox-audio-events` (every 5 ms: events, freeing replaced tracks), `tuneox-decode` (one per load). See "Audio engine threads" in [architecture.md](../architecture.md).
- **Commands are synchronous but short:** an `Engine` call queues the command and returns once the audio thread has applied it, with the resulting state: at most one device buffer (256 frames requested, ~6 ms; measured 1–4 ms on MacBook Pro speakers). The UI should still update optimistically and not wait for the reply on interactive paths. `Engine::start()` returns immediately; the device opens in the background (~150–250 ms) and the first command waits for it.
- **Offline engine:** `Engine::offline(sample_rate)` runs the same core on the caller's thread; `render(&mut [f32])` (interleaved stereo) advances its clock; `poll_events()` returns the events the real engine would send.
- **Desktop:** deck, mixer, preview and player commands (the registry marks them `player: true`, `Command::PLAYER`) run on a second `Context` with its own library connection and the same engine, so a long import never holds up a deck. Routing is by that flag, not the effect class, because some player commands also save to the library (effect `edit`). The engine starts at launch; the UI should call `player_status` on mount for the initial state, then follow events. DUAL CONTROL is the UI's job: it sends the same command to both decks.
- **Cue colours and comments** stay off the audio thread: each loaded library cue carries only a small tag (index + 1 into a colour/comment table the engine keeps with the deck's metadata), and `DeckState` is assembled with them on the non-real-time side. Cues stored on a deck have tag 0 (no colour, no comment).
- **Scrubbing** (`deck_scrub`): the read head chases the dragged position like a record moved by hand. Its speed is the distance to the target over 40 ms, smoothed with a 10 ms time constant (critically damped: no overshoot, settles in ~100 ms; at a steady drag it trails by about speed × 40 ms), clamped to ±32× and read with the same cubic interpolation, so no move fades. The level follows the speed (silent below 0.01×, full from 0.25×), so a held record is silent rather than a DC offset, and the start and end are clickless. A target more than 2 s away is jumped to with the usual fade. On `end` a deck with PLAY latched ramps to its tempo and hands over to normal playback once within 0.2 %; a paused deck comes to rest exactly on the final target. Dragging out of an active loop ends it; sync doesn't lock a scrubbed deck (or follow a scrubbed master). A held scrub that gets no command for 1.5 s lets go by itself (`SCRUB_IDLE_RELEASE_SECONDS`), so a lost pointer-up or a controller that went away can't leave a deck held; the UI re-sends its position every 250 ms while the pointer is down. Constants: `deck::SCRUB_*`.
- **Preview player** (`preview.rs`): a third voice with its own track (decoded like a deck's, reusing a deck's audio when it's the same file, and vice versa), read at the track's speed with cubic resampling at the track's auto gain, mixed into the master bus before the master volume (so `TUNEOX_MUTED` silences it too). Starting it pauses every moving deck; a deck starting to move stops it (one output; see open question 9). A replaced track fades out from its own audio and is then freed off the audio thread.

### Clock and events

The UI animates the playhead itself; the engine only says where a deck was and how fast it's moving:

```jsonc
// DeckState: returned by every deck command and sent as an event
{ "deck": 1, "track_id": 123, "loading": false, "error": null,
  "playing": true,               // PLAY on, or CUE / a gated hot cue held from pause
  "position_us": 61234000,       // where the deck was (as heard) at `at_ms`
  "at_ms": 912345.25,            // engine clock (monotonic ms)
  "rate": 1.04,                  // track µs per real µs, incl. a held pitch bend; 0 when paused
                                 // or waiting for the decoder
  "duration_us": 306000000,      // an estimate while `loading`
  "tempo_percent": 4.0, "tempo_range": 10.0,
  "bpm": 131.04, "original_bpm": 126.0,   // null without a grid or BPM; bpm excludes pitch bend
  "sync_mode": "beat",           // "off" | "beat" (tempo + phase) | "bpm" (tempo only; also the
                                 // fallback after a pitch bend, shown yellow/orange in rekordbox)
  "master": false,
  "quantize": true,
  "pitch_bend": null,            // "back" | "forward" while held
  "cue_us": 24000,               // null until a cue is set (fresh loads have none)
  "loop": { "start_us": 60000000, "end_us": 61904000, "beats": 4.0 },  // active loop, or null
  "last_loop": { "start_us": 60000000, "end_us": 61904000, "beats": 4.0 }, // RELOOP target, or null
  "loop_in_us": null,            // set by deck_loop_in until deck_loop_out
  "scrubbing": false,            // deck_scrub in progress (or settling after `end`); `rate` is
                                 // then the drag speed (negative backwards) and `playing` says
                                 // whether it plays on when let go
  "hot_cues": [                  // stored pads only, in slot order (incl. pads stored since loading)
    { "slot": 0, "start_us": 24000, "end_us": null, "color": 22, "comment": "drop" },
    { "slot": 2, "start_us": 92000000, "end_us": 93904000, "color": null, "comment": null } ],
  "memory_cues": [               // position order; `index` is what deck_memory_cue calls
    { "index": 0, "start_us": 24000, "end_us": null, "color": null, "comment": null } ]
}
// `color`: the library cue's palette index (hot cues: the 62-colour palette, research §5.1);
// null = the colour scheme's default (cues stored on the deck have none). `comment` from the
// library, null if none.
```

- Events carry `sent_at_ms` (engine clock at send time). The UI estimates `offset = performance.now() − sent_at_ms` and keeps the **minimum** offset seen (the least-delayed delivery), then shows `position_us + (performance.now() − offset − at_ms) × 1000 × rate`.
- The engine emits a `DeckState` event on every change and about 20 times a second while a deck plays, so drift stays small. Every loop wrap is a change, so it's sent too; between events the UI should wrap its extrapolated position inside an active `loop` itself.
- Reported positions account for output latency where the device reports it (what you hear, not what was rendered): `at_ms` is when the block's first sample reaches the speakers, so it can be a few ms ahead of `sent_at_ms`.
- Desktop: `apps/desktop/src-tauri` forwards engine events to the window as `tuneox://deck` (payload: `DeckState` + `sent_at_ms`), `tuneox://mixer` (`MixerState`) and `tuneox://preview` (`PreviewState` + `sent_at_ms`; on change and ~20 Hz while it plays). A scrubbing deck sends the periodic ~20 Hz update rather than one per block.

```jsonc
// MixerState
{ "channels": [
    { "deck": 1, "kill_low": false, "kill_mid": false, "kill_high": false, "auto_gain_db": 0.0,
      "volume": 1.0, "eq_low": 0.0, "eq_mid": 0.0, "eq_high": 0.0 },
    { "deck": 2, … } ],
  "crossfader": 0.0,             // −1 = deck 1 only … 1 = deck 2 only
  "master_volume": 1.0 }
// rekordbox's DUAL PLAYER has only the kills, auto gain (A-GAIN) and crossfader; volume, EQ and
// master volume are extras. Kills are independent of the EQ gains. auto_gain_db −24…+24 (linear
// gain inside), volume/master_volume linear 0–1, EQ dB −80 (= −∞) to +6.

// PlayerStatus
{ "running": true,               // false until a deck command starts the engine
  "decks": [ /* DeckState 1 */, /* DeckState 2 */ ],
  "mixer": { /* MixerState */ },
  "hot_cue_gate": false,         // rekordbox's "Gate playback during Pause" preference
  "quantize_beats": 1.0,         // quantize resolution, both decks: 0.125 | 0.25 | 0.5 | 1
  "sync_phase": "beat",          // what beat sync aligns: "beat" (rekordbox) | "bar" (TuneOx extra)
  "preview": { /* PreviewState */ },
  "output": { "device": "MacBook Pro Speakers", "sample_rate": 44100, "channels": 2,
              "latency_ms": 8.96, "error": null } }   // null before the device opens and offline;
                                                      // device null + error when no device could be opened

// PreviewState: the browser's preview player, a clock like DeckState
{ "track_id": 123,               // the track previewed last (kept after it stops), or null
  "playing": true, "loading": false, "error": null,
  "position_us": 61234000, "at_ms": 912345.25,
  "rate": 1.0,                   // 1 while playing, 0 when stopped or waiting for the decoder
  "duration_us": 306000000,      // an estimate while `loading`
  "auto_gain_db": -1.5 }         // the gain it plays at (the track's auto gain, capped ±12 dB)
```

### Commands (`crates/tuneox-commands/src/decks.rs`)

New effect class **`playback`**: controls audio, doesn't change the library. MCP annotations: `readOnlyHint: false, destructiveHint: false, idempotentHint: false, openWorldHint: false`. All return `DeckState` unless noted.

**Effect classes.** A command that can write the library is **`edit`**, even if it also plays: `deck_hot_cue` (an empty pad saves the hot cue it stores), `deck_hot_cue_delete`, `deck_memory_cue_set`, `deck_memory_cue_delete` and `mixer_set` (an `auto_gain_db` change on a loaded deck saves the trim). The rest are `playback`; `player_status` is `read`. The MCP annotations for `edit` and `playback` are the same. Every one of them is a player command (`player: true`), which is what the desktop routes on.

**Saving cue edits** (as rekordbox does): the edit goes to the engine first, then to the deck's track in the library through single-cue calls that leave the track's other cues alone (`Library::add_cue`, `delete_hot_cue`, `delete_memory_cue`; imports keep `replace_cues`). A hot cue stored on a pad replaces the slot's library cue; a memory cue is deleted by its start (and end). A-GAIN trims are saved as the track's `auto_gain_db`, the same field rekordbox's analysed gain imports into (rekordbox also overwrites its gain with the trim), so a reload or a preview plays at the trim. A later re-import from rekordbox overwrites both (known gap). The write is synchronous (a single-row SQLite write); the UI shows edits before the reply.

| Command | Input | Does |
| --- | --- | --- |
| `deck_load` | `deck` (1 or 2), `track_id`, optional `auto_gain_db` | Loads with grid, hot cues and memory cues; parks at 0:00, paused, no cue, no loop. Master passes to the other deck if this one was master. Channel auto gain = `auto_gain_db` or the track's (saved trims included, capped ±12 dB) or 0 dB. |
| `deck_play` / `deck_pause` | `deck` | |
| `deck_cue` | `deck`, `action`: `press` / `release` | §2.1 rules below. |
| `deck_hot_cue` | `deck`, `slot` (0 = A … 7 = H), `action` | Stored pad: jump and play (also from pause). Empty pad: store position or playing loop, **and save it to the track** (effect `edit`). |
| `deck_hot_cue_delete` | `deck`, `slot` | Clears the pad and deletes the track's hot cue (effect `edit`). An empty pad is left alone. |
| `deck_memory_cue` | `deck`, `action`: `call`, `target`: index / `"next"` / `"prev"` | Jump and pause; becomes the cue point; a memory loop is armed. |
| `deck_memory_cue_set` | `deck` | rekordbox's MEMORY (effect `edit`): saves the active loop as a memory loop; else the last loop if it was set after the cue point ("just after loop play"); else the cue point. In position order, up to 10 per track (refused beyond); the same cue again changes nothing; refused with nothing to save. |
| `deck_memory_cue_delete` | `deck`, `index` | Deletes that memory cue/loop from the deck and the track (effect `edit`); later ones move up an index. |
| `deck_seek` | `deck`, `position_us` | Keeps play state; leaving a loop ends it. |
| `deck_scrub` | `deck`, `action`: `start` / `move` / `end`, `position_us` (required for `move`; optional for `start`, the deck's position, and `end`, the last target) | Drags the deck like a record (see "As built"): the audio follows smoothly, no per-move fades; `end` plays on if PLAY is latched, else rests on the target. `move` without `start` starts. A held scrub with no command for 1.5 s ends as if let go (re-send `move` to hold longer). `scrubbing` is true meanwhile. |
| `deck_beat_jump` | `deck`, `beats` (negative jumps back) | Exact beats, phase kept; moves an active loop with it. |
| `deck_loop` | `deck`, `beats` (1/64–512) | Auto beat loop from the (snapped) position; same size again exits; another size resizes from loop-in. |
| `deck_loop_in` / `deck_loop_out` | `deck` | Manual loop; IN also sets the cue; OUT starts looping (or moves an active loop's end). |
| `deck_reloop` | `deck` | Back to `last_loop`'s start, looping. |
| `deck_loop_resize` | `deck`, `factor`: `double` / `half` | Anchored at loop-in, 1/64–512 beats. |
| `deck_loop_exit` | `deck` | Playback runs past loop-out. |
| `deck_set_tempo` | `deck`, `percent` | Clamped to the range and 40–499 BPM. Refused on a synced slave. |
| `deck_set_bpm` | `deck`, `bpm` | Typed BPM; must be 40–499 and within the range, else an error. Refused on a synced slave. |
| `deck_tempo_reset` | `deck` | 0 %. Refused on a synced slave. |
| `deck_set_tempo_range` | `deck`, `range`: 6 / 10 / 16 / 100 | Slider steps 0.02 / 0.04 / 0.04 / 0.5 % (the engine accepts any value; the UI steps). |
| `deck_pitch_bend` | `deck`, `direction`: `back` / `forward`, `action` | ±4 % while held, playing only; a beat-synced slave drops to BPM sync. |
| `deck_sync` | `deck`, `on`, optional `mode`: `beat` (default) / `bpm` | |
| `deck_set_master` | `deck` | |
| `deck_set_quantize` | `deck`, `on` | Replaces the global `player_set_quantize`. |
| `player_set_hot_cue_gate` | `on` | → `PlayerStatus`. |
| `player_set_quantize_value` | `beats`: 0.125 / 0.25 / 0.5 / 1 (default 1) | rekordbox EXPORT's "Quantize Beat Value", both decks. With Q on, cue set, loop in/out, hot cue store and auto loop starts snap to that resolution; hot cue jumps keep the phase at it. Beat jumps unchanged. → `PlayerStatus`. |
| `player_set_sync_phase` | `unit`: `beat` (default) / `bar` | What beat sync aligns. `bar` is a **TuneOx extra** (rekordbox's BEAT SYNC aligns beats only): the synced deck's place in the bar matches the master's, downbeat to downbeat, from the grids' `bar_position`, moving it at most half a bar. BPM-only sync and half/double-time sync keep beat alignment; so does a loop that isn't whole bars. → `PlayerStatus`. |
| `mixer_set` | any of `deck` + `kill_low`/`kill_mid`/`kill_high` (bool), `auto_gain_db`, `volume` (0–1), `eq_low`/`eq_mid`/`eq_high` (dB); `crossfader` (−1 … 1); `master_volume` (0–1) | → `MixerState`. An `auto_gain_db` change on a deck with a library track saves it as the track's auto gain (effect `edit`). |
| `preview_play` | `track_id`, optional `position_us` (default 0) | The browser's preview player: plays the track from there at its own auto gain through the master; pauses any moving deck. The same track again only moves the playhead. → `PreviewState`. |
| `preview_stop` | — | Stops the preview (short fade); paused decks stay paused. → `PreviewState` (idle without starting the engine). |
| `player_status` | — | → `PlayerStatus` (effect `read`; never starts the engine). |

`deck_load` looks up the track's file, beat grid and cues in the library and hands them to the engine; hot cues and memory cues come from the library. The engine starts lazily on the first deck command (the CLI/MCP server don't open an audio device unless asked); the desktop app starts it at launch so it can subscribe to events. Tests use an offline engine.

As built: `deck_load` returns `NotFound` for an unknown track or a missing file; decode problems arrive later as `error` in the state. Beat-based commands (`deck_beat_jump`, `deck_loop`, `deck_set_bpm`, `deck_sync` on) need a beat grid or at least a library BPM (then a steady grid from 0 s is assumed); without one they fail with a clear error, like rekordbox refusing SYNC on unanalysed tracks. Tempo commands on a synced slave fail with "deck 2 is synced to the master, deck 1: change the tempo on deck 1 or turn sync off on deck 2". Mixer values are clamped to their ranges; channel settings without `deck` are an error.

**Done when:** offline engine tests cover load, play/pause, CUE semantics, hot cues, seek, beat jump, loops (sample-accurate wrap), tempo (rate), sync (tempo and phase within 1 ms on synthetic grids), quantize, crossfader and EQ gains; deck commands are tested through `run_json` with an offline engine; the desktop app receives `tuneox://deck` events; `cargo clippy -D warnings` is clean.

All covered (`crates/tuneox-audio/src/tests.rs`, `crates/tuneox-commands/src/decks.rs`), plus memory cues, manual loops/reloop/resize, per-deck quantize, typed BPM and limits, pitch bend (and its sync fallback), BPM-only sync, kills and auto gain, decoding synthetic WAV/AIFF files, sync on millisecond-rounded (rekordbox-style) grids, half/double-time sync, events at ~20 Hz, and a counting allocator proving the render path (every kind of command, sync, loops, EQ, a track load) doesn't allocate or free. The desktop forwarding is built but only checked by compiling: the GUI wasn't run.

Follow-ups (2026-10-12), also covered: cue colours/comments in `DeckState`; hot cue delete, memory cue set (cue point, active loop, last loop, duplicates, the 10 limit) and delete; saving pad cues, cue deletes, memory cues and A-GAIN trims to the library and getting them back on reload (through `run_json`, plus `tuneox-library` tests for the single-cue calls and `set_auto_gain`); scrubbing a paused and a playing deck along UI-style moves every 8 ms (the read head trails the drag by ~36 ms at 1×, never a sample step above what the tone allows, a steady level with no per-move dips, and it rests exactly on the final target or plays on at the tempo); far scrub targets and loops; the preview voice (its gain, pausing the decks, a deck stopping it, the master volume muting it, stopping with a fade, the track end, reusing decoded files, events); the quantize resolution (cue, loop in/out, hot cue store, auto loop start, hot cue phase) and bar sync (bar positions equal and phase within 1 ms under `bar`, beat-only alignment under `beat`, never moving more than half a bar, re-aligning after the master jumps). The allocation test now also covers scrubbing, cue edits, the preview voice (track changes cross-fading and freeing off-thread) and the new settings. The real-device test (`--ignored`, silent) passes.

### Transport, tempo and sync rules (from research part A)

Each rule is one small function in `crates/tuneox-audio` (named in the right column), so a correction stays local.

| Rule | Where |
| --- | --- |
| **CUE** paused away from the cue → set it here (snapped with Q) and move there. Playing → back to the cue (the start if none) and pause. Paused at the cue, held → plays while held, returns and pauses on release; **PLAY while held keeps playing**. | `Deck::cue_press_while_*`, `Deck::play` |
| **Hot cue**, stored slot → **jump and play, also from pause**; while playing with Q the landing keeps the beat phase (immediate, not next beat); a hot loop jumps in and loops. Empty slot → store the position (snapped with Q) or the playing loop as a hot loop. Gate mode (`player_set_hot_cue_gate`, off by default): from pause, play only while held; not for hot loops. Smart Cue (off in rekordbox) isn't implemented. | `Deck::hot_cue_*` |
| **Memory cue call** → jump and pause; it becomes the cue point; a memory loop is armed so PLAY loops it. `next`/`prev` are relative to the playhead. | `Deck::memory_cue_call` |
| **Load** → park at 0:00 with no cue and no loop (EXPORT mode doesn't jump to a memory cue). Tempo, range, sync and quantize carry over. Loading onto a playing deck stops it. | `Deck::load` |
| **Beat jump** → exact beats on the grid, phase kept; during a loop the loop moves too. Clamped at 0 s. | `Deck::beat_jump` |
| **Auto beat loop** → from the position snapped with Q; same size again exits; another size resizes from loop-in. **Manual loop**: IN (also sets the cue) / OUT (snapped with Q; OUT snapped onto IN makes a 1-beat loop) / EXIT runs past loop-out / RELOOP returns to the last loop-in. **Resize** ×2/÷2 anchored at loop-in, 1/64–512 beats. A shorter loop wraps the playhead into it in phase. Seeking outside a loop ends it. | `Deck::beat_loop`, `loop_in`, `loop_out`, `reloop`, `loop_resize`, `jump_to` |
| **Tempo**: clamped to the range and to 40–499 BPM; typed BPM must fit both or is refused; reset to 0 %; range changes keep the tempo where it fits. | `Deck::set_tempo`, `Engine::set_bpm` |
| **Pitch bend**: ±4 % while held, playing only (rekordbox's amount is unknown, see below). | `deck::PITCH_BEND`, `Deck::pitch_bend` |
| **Sync**: tempo = the master's effective BPM (grid BPM × tempo), half/double time when closer to the track's own tempo (always on). Beat sync phase: a jump when off by > 10 ms, otherwise ≤ ±1 % tempo correction above a 0.5 ms dead band; no phase lock while previewing, pitch-bending or in a loop that isn't whole beats. Pitch-bending a beat-synced slave drops it to BPM sync until SYNC is toggled. Sync off keeps the synced tempo. | `Core::update_rates`, `sync_target`, `lock_phase`, `SYNC_*` |
| **Master**: the first deck to start playing; `deck_set_master` hands over; when the master's track is replaced, master passes to the other deck (which keeps its tempo). Pausing the master doesn't hand over. Sync on with no master makes the other deck master if it's playing, otherwise this one. **Slaves can't change or reset their tempo.** | `Core::on_started`, `on_track_changed`, `set_sync`, `Engine::check_tempo_is_free` |
| **Mixer**: kills per band, independent of EQ gains; auto gain in dB per channel, reset to the track's gain (or 0 dB) on load. | `ChannelSettings::band_gains`, `Core::apply` |
| **Quantize resolution** (rekordbox EXPORT's "Quantize Beat Value", one value for both decks): with Q on, everything that snaps snaps to the nearest 1/8, 1/4, 1/2 or 1 beat; a hot cue jump while playing keeps the read head's offset from the nearest grid point at that resolution. | `Grid::nearest`, `Deck::snap`, `Deck::hot_cue_press_while_playing` |
| **Bar sync** (TuneOx extra, `player_set_sync_phase` `bar`): beat sync's phase error is measured within the bar (from the grid's `bar_position`, 4/4) and wrapped to ±2 beats, so the slave jumps (or is pulled, for small errors) to the nearest position with the master's place in the bar. Falls back to beat phase for half/double-time matches and loops that aren't whole bars. | `Core::sync_target`, `Grid::bar_beat_at`, `loop_keeps_bars` |
| **Memory cue set** (MEMORY): active loop → memory loop; else the last loop if set after the cue point; else the cue point; no duplicates; 10 per track. **Delete**: by index. **Hot cue delete**: clears the pad. | `Deck::memory_cue_candidate`, `memory_cue_set`, `memory_cue_delete`, `hot_cue_delete` |
| **Scrub**: the read head chases the dragged target (critically damped), level follows speed, no per-move fades; `end` hands over to playback (PLAY latched) or rests on the target. | `Deck::scrub`, `Deck::render_scrub_sample` |
| **Preview player**: pauses moving decks when it starts; a deck starting stops it; stops at the track end. | `Core::apply` (`PreviewPlay`), `Core::process_commands`, `PreviewVoice` |

### Open behavior questions

| # | Now | Where |
| --- | --- | --- |
| 1 | **Pitch bend amount**: ±4 %. rekordbox nudges via `setJogRingSpeed(±0.1)`, whose percent is unknown (research open question 5). | `deck::PITCH_BEND` |
| 2 | **Crossfader curve**: constant power (both −3 dB in the middle); rekordbox's EXPORT curve is unknown (open question 6). | `mixer::crossfader_gains` |
| 3 | **Defaults**: quantize on, half/double sync always on, hot cue jumps immediate with phase kept (not "wait for next beat"); rekordbox's defaults weren't decoded (open question 7). | `Deck::default`, `Core::sync_target` |
| 4 | **Memory cue call sets the cue point** to the called cue (CDJ convention); the research says jump and pause but not whether the cue moves. | `Deck::memory_cue_call` |
| 5 | **Auto gain range** −24…+24 dB in the engine: rekordbox's knob looks like ±10 dB but stored gains reach +14.4 dB (open question 6). A track's stored gain (imported or a saved trim) applies on load and in the preview, capped at ±12 dB; the UI's knob is ±12 dB. | `mixer::AUTO_GAIN_*`, `decks::MAX_TRACK_GAIN_DB` |
| 6 | **Resolved (2026-10-12):** rekordbox doesn't trim MP3 encoder delay, but it plays a non-FFmpeg Xing/Info header frame as one frame of silence. TuneOx now does the same (`mp3_lead_in_frames`), so all 40 MP3s measured line up with rekordbox's decode within a few ms. See [research/mp3-timing.md](../research/mp3-timing.md). | `decode::mp3_lead_in_frames` |
| 7 | **Extras' details**: the channel fader and master volume are linear gain; the EQ is an isolator (Linkwitz-Riley 24 dB/oct, crossovers 250 Hz / 2.5 kHz). Not part of the DUAL PLAYER, so no rekordbox reference. | `Mixer::process`, `mixer::LOW_MID_HZ` |
| 8 | **Track end**: the deck stops at the end; PLAY there does nothing until the deck is moved. | `Deck::render`, `Deck::play` |
| 9 | **A deck starting stops the preview.** The research only says previewing pauses the decks; with one output, the reverse seemed the least surprising. | `Core::process_commands` |
| 10 | **MEMORY "just after loop play"** is read as "the last loop was set after the cue point". rekordbox's exact rule (does moving away end "just after"?) isn't known. | `Deck::memory_cue_candidate` |
| 11 | **Scrub feel**: follow 40 ms / smoothing 10 ms / max 32× / level ramp 0.01–0.25× are tuned by ear-free tests, not against rekordbox's drag. | `deck::SCRUB_*` |
| 12 | **Quantize resolution is global** (rekordbox EXPORT has one preference); PERFORMANCE's separate hot cue / loop values aren't modelled, nor its rule that fractional values disable BEAT SYNC (research §2.5; whether EXPORT does the same is unknown, so TuneOx keeps sync on). The default (1 beat) matches the research's reading of `getCurrentDeckQuantize`. | `Core::apply` (`QuantizeBeats`) |

Not built yet (later tasks): active loops that engage automatically, "Fine" beat jump (5 ms; the UI uses `deck_seek`), key lock / master tempo (rekordbox defaults it off), Smart Cue, editing cue colours and comments (the library carries them; there's no command yet).

## D. Deck UI (after A–C)

Build the UI from [research/rekordbox-decks.md](../research/rekordbox-decks.md); its "Checklist → For the UI agent" is the acceptance list. Highlights:

**Layout: rekordbox's DUAL PLAYER above the library browser.**
- Deck A on top, deck B below. Per deck: track info row (artwork if any, title/artist, BPM with two decimals, key, remaining time large with tenths `-02:54.4`, elapsed grey), BEAT SYNC (accent blue; **yellow/orange when `sync_mode` is `bpm`**), MASTER, the overview waveform, and a function row (MEMORY, AT/MT loop mode with `< N >`, `− BPM +` with double-click to type, pitch bend ◄◄ ►►, RST, Q).
- **The two scrolling detail waveforms sit stacked between the decks, as half-waveforms facing each other** (A grows up from its baseline, B grows down), sharing one center playhead line, with each deck's bar counter (`N.bBars`, accent blue) beside the playhead.
- Left operation column per deck: beat jump `< >` with size dropdown (Fine, 4/8/16 beats, 8/16/32 bars; default 4 beats), CUE (#FFB937 ring), PLAY/PAUSE. DUAL CONTROL toggle between the decks: when on, PLAY, CUE and beat jump go to both decks (the UI sends the command to each).
- Mixer column: per-deck A-GAIN knob with dB readout (from the track's `auto_gain_db`; apply at most ±12 dB), HIGH/MID/LOW **kill** pills (#1473EB outline when on), vertical crossfader (A top, B bottom; double-click centers). TuneOx extras, collapsed by default behind a "Mixer" toggle: channel faders and EQ knobs.
- Right side per deck: MEMORY / HOT CUE list (click a hot cue = jump and play; click a memory cue = jump and pause).
- Library browser below, as today. Load a track by dragging it onto a deck, a context menu ("Load to Deck A/B"), or keyboard.

**Waveforms** (data from `get_waveform`; colors and rules in research §4–5 and `research/waveforms.md`):
- Style setting BLUE / RGB / 3Band, **default 3Band**.
- Overviews come from the 1200-column data: 3Band from `three_band` preview (stacked bars per §4.4), RGB and BLUE both from the `rgb` preview (`PWV4`; BLUE uses the 8-color overview table in §4.2). Minute ticks, hot cue letter squares above, memory cue triangles on top, current cue orange triangle bottom-left, playhead light while playing and red when paused, played part drawn dimmer, click to seek keeping play state.
- Detail waveforms from the 150/s data, drawn per research §4.2–4.5: 3Band span layering with the overlap colors and the raised-cosine high band, BLUE/RGB color tables quantized as rekordbox does; black background, beat grid (downbeats white with red edge markers, other beats 30 % white, hidden past a 120 s span), hot cue flags, memory cue triangles, loop overlay (#FFA300 at 50 %), zoom −/RST/+.
- Hot cue colors: `ColorTableIndex` > 0 → the 62-color palette; 0 → COLORFUL slot defaults (A #FF376F, B #45ACDB, C #7DC13D, D #AA72FF, E #30D26E, F #E0641B, G #305AFF, H #C3AF04; hot loops #FF8C00).
- 60 fps with both decks playing: render waveform columns to cached canvases once per track/zoom, then each frame only blits with an offset and draws the playhead and markers. The playhead position comes from the clock (part C), never from a per-frame core call.

**Data the UI needs:** `get_waveform` (waveforms), `get_beat_grid` (every beat with its bar position, for grid lines and the bar counter), `get_track` (track details), `player_status` plus the `tuneox://deck` / `tuneox://mixer` / `tuneox://preview` events (live state, cue colours and comments, and the clock), and the deck, mixer and preview commands in part C. Cues stored on a deck have no colour; draw them with the scheme default for their slot.

**Testing without sound:** start the app with `TUNEOX_MUTED=1` (master volume 0 at launch).

**Keyboard:** rekordbox's EXPORT preset (research §6): plain keys for deck A, ⇧ for deck B (Space play/pause, C cue, ←/→ beat jump, 1–3 hot cues, I/O/R loops, 4–9 beat loops, F1 sync, F3 tempo reset, F4/F5 pitch bend, F6/F7 BPM −/+, T time mode). Keys must not fire while typing in the search box.

**Browser preview player** (rekordbox's Preview column): a waveform thumbnail column; clicking it previews from that position and pauses any playing deck.

### As built

Status: **built** (UI agent, 2026-10-12; follow-ups the same day: cue editing, smooth scrubbing, the preview column, quantize resolution and bar sync), except the items under "Not built" below. Code: `apps/desktop/src/decks/` (state, logic, rendering) and `apps/desktop/src/components/decks/` (panels).

**Layout.** The DUAL PLAYER sits between the toolbar and the library, `clamp(380px, 50vh, 470px)` tall (450 px at 1440×900, 400 px at 1280×800). Columns: operation (beat jump `‹ ›` + size, CUE, PLAY/PAUSE per deck, DUAL CONTROL between) · mixer (A-GAIN knob ±12 dB with `+0.0dB` readout, HIGH/MID/LOW kill pills, vertical crossfader with A on top) · TuneOx extras behind the toolbar's **Mixer** toggle (EQ knobs, channel faders, master volume) · decks (A's info row, overview and function row; the stacked detail waveforms; B's) · HOT CUE / MEMORY lists. The toolbar also has the waveform style (BLUE / RGB / 3BAND, persisted) and a **Decks** toggle that hides the player (rekordbox's FULL BROWSER).

**Waveforms.** Painted per research §4 into `ImageData` by pure functions (`decks/waveform.ts`, unit-tested): 3Band span layering with the overlap colours and the raised-cosine high band, BLUE/RGB detail with rekordbox's 3-bit colour quantization, 3Band overviews stacked from `three_band` previews, RGB/BLUE overviews from the `rgb` preview (§4.2/§4.3 rules; PWV4 heights on a 0–127 scale). Choices where the research leaves room:

| Question | Choice |
| --- | --- |
| PWVC `band_gains` on the detail | **Not applied**: the stored PWV7 levels already include them (research/waveforms.md), and applying them again would cap the low band at 80 % on most tracks and clip mids with gains up to 3.0. One constant (`APPLY_DETAIL_BAND_GAINS`) flips it. |
| High band curve | Applied at draw time (rekordbox's `draw3BandsWaveCDJ` does), on top of the curve baked in at analysis. |
| Time axis | **Real time**: scroll speed is constant and beat spacing follows the tempo, so synced decks' beats line up across the shared playhead. rekordbox has a preference for either; its default is unknown. |
| Zoom | Visible span 2, 4, 6, 8, 12, 16, 24 or 32 s across the lane, **default 8 s** (rekordbox PERFORMANCE's default step), shared by both lanes so beats stay comparable. `−` / `RST` / `+` on the left edge; `=`, `-`, `⇧=` keys. The grid hides past 120 s, which these spans never reach. |
| Markers | A 14 px marker strip on each lane's outer edge holds hot cue flags (coloured square, dark letter) and memory cue triangles (memory loops as a bar); hot cue lines and loop overlays cover the lane; the current cue is an orange triangle on the baseline; the bar counter sits beside the playhead in the strip, left of it so upcoming flags stay visible. |
| Memory cue colour | Palette colour if the cue has `color` > 0, else red #FF0000 (§4.5's CDJ default). Colours and comments come with `DeckState`'s cues (no more matching `get_track` by position). |
| Overview played part | Drawn at 42 % brightness; the playhead is #E0E0E0 playing, #FF0000 paused. |
| A style a track lacks | Falls back to another style the track has; while a newly chosen style loads, the previous one stays on screen. |

**Rendering.** Detail waveforms are pre-rendered into tiles 1024 device pixels wide, aligned to the track and cached per track, style, scale, lane height and direction (14 per lane, LRU); a zoom or tempo change starts a new set. Overviews are pre-rendered once per track, style and size. One `requestAnimationFrame` loop (`decks/render/animator.ts`) draws both decks: it blits the visible tiles at the playhead offset, draws grid, cues, loops, playheads and bar counters, redraws an overview only when its playhead pixel or markers change, and writes the time displays straight into the DOM. React re-renders only when a deck's non-clock state, the mixer or settings change. Measured in a desktop browser against a stand-in for the engine, at 1440×900 (DPR 1) with both decks playing: about 0.8 ms of drawing per frame. Painting one Retina-sized tile (1024 × 180 device pixels, real data) takes 0.7 ms for 3Band and 0.3–0.4 ms for RGB/BLUE, so a zoom change that repaints both lanes costs one ~5 ms frame. ⌥⇧P shows a timing overlay in the app; the numbers in the real window at Retina resolution are still to be taken.

**Clock and commands.** Positions follow part C's clock (minimum `performance.now() − sent_at_ms`; a reply's `at_ms` gives a provisional offset until the first event), wrapping inside an active loop between events. States older than the latest `at_ms` are ignored. Every control updates the screen at once: play, pause, seek, load, sync, master, quantize and tempo steps patch the shown state until the command's reply replaces it; mixer changes show immediately and an engine echo never undoes one still in flight. Seeks and mixer drags coalesce (one request in flight, only the latest value waits). Errors appear as toasts.

**Controls as built.** CUE, hot cue pads and pitch bend send press on pointer down and release on up. DUAL CONTROL sends PLAY/PAUSE (decided by the pressed deck), CUE and beat jump to both loaded decks, keys included. Beat jump "Fine" is a 5 ms `deck_seek` (PERFORMANCE's FINE). The function row shows MEMORY (`deck_memory_cue_set`; disabled with no cue point or loop), the manual loop (IN, OUT, RELOOP/EXIT) and the auto loop (`‹ N ›`, default 4 beats; `‹ ›` also resize an active loop) side by side instead of rekordbox's AT/MT switch. `− BPM +` steps 0.02 %, 0.1 % once held 500 ms (auto-repeat every 50 ms after 300 ms); double-click the BPM to type one. Clicking a detail lane moves that deck's playhead there; dragging moves the deck like a record (`deck_scrub`: start once the pointer has moved 3 px, moves coalesced to one in flight, the position re-sent every 250 ms while held, end on release, or on a pointer-up anywhere in the window, lost pointer capture or the window losing focus): the waveform follows the pointer exactly while the audio follows in the engine, and a playing deck plays on when let go (the dragged position stays on screen until the engine reports the scrub settled, at most 400 ms). Clicking or dragging the overview seeks. Tracks load by dragging a row onto a deck (mouse events, not HTML5 drag and drop, which the window uses for files), the row's context menu, or double-click (deck A unless it's playing, else B).

**Cue lists.** Hovering a HOT CUE or MEMORY row shows ✕, which deletes that cue from the deck and the track (`deck_hot_cue_delete` / `deck_memory_cue_delete`, shown at once). An empty MEMORY list says MEMORY (M) saves one. The track detail panel reads the track again ~300 ms after a deck holding it changes its cues or A-GAIN, so it shows the saved edits.

**Player settings** (toolbar, persisted with the deck settings and sent to the engine at launch): `Q 1/8 · 1/4 · 1/2 · 1` (`player_set_quantize_value`; the per-deck Q buttons stay, their tooltip names the resolution) and `SYNC BEAT · BAR` (`player_set_sync_phase`).

**Preview column** (rekordbox's browser preview player, research §1.5). The track table's Preview column (116 px, after #, dropped on narrow windows before #) shows a play/stop button and the track's overview thumbnail in the deck waveform style (3Band from the `three_band` preview, RGB/BLUE from the `rgb` one, falling back to the other). Clicking the thumbnail previews from that spot (`preview_play`; the engine pauses any playing deck); the button stops the playing preview, or plays (resuming a stopped preview of the same track, else from the start). A playhead (light while playing, red when stopped) moves across the previewing row, driven by its own animation frame loop only while something previews; the state comes from `tuneox://preview` events and replies, shown optimistically. Only rows on screen exist (the table is virtualized), and each fetches its preview waveform through a queue (4 at a time, most recently shown first, skipping rows already scrolled away); waveforms (3,000) and painted thumbnails (400) are cached in LRUs, so scrolling back is free.

**Error toasts** translate the API's deck numbers into the UI's letters and its clauses into sentences (`decks/errors.ts`), e.g. "Deck B is synced to Deck A. Change the tempo on Deck A, or turn off BEAT SYNC on Deck B." The engine and MCP messages stay numeric.

**Track table scrolling.** The selection scrolls into view only when it moved by keyboard, or when a search the user typed still matches it but left it off screen; otherwise a new search or view starts at the top. Library reloads, sorting, clicks and clearing the search never jump to the selection.

**Keyboard** (EXPORT preset; plain = deck A, ⇧ = deck B; ignored while typing in a text field; ⌘/Ctrl/⌥ combinations are left to the app, so ⌘F still searches, except ⌘1–3): Space play/pause · C cue (press/release) · ← → beat jump · 1 2 3 hot cues A–C (press/release) · ⌘1 ⌘2 ⌘3 clear hot cues A–C (Ctrl on other systems; ⌘⇧ for deck B) · I / O / R loop in / out / exit-reloop · 4–9 beat loops 1–32 · `/` halve the loop size · A S D F G H J K L ; call memory cues 1–10 · N / B next / previous memory cue · M set memory cue · X delete the memory cue the deck is parked on (call one first; a toast says so otherwise) · Q quantize · T time mode · F1 sync · F3 tempo reset · F4 / F5 pitch bend (press/release) · F6 / F7 BPM −/+ (hold to repeat). Shared: `=` / `-` zoom, `⇧=` reset zoom.

**Not built:** key shift, KEY SYNC and MT (no key lock), the INFO tab, phrase bars, the Matching button, artwork, ⌥¥ (loop ×2 on JIS keyboards), preview keys (⌘P play/stop, ⌘[ skip) and the artwork-hover preview, cue markers on the preview thumbnails, editing cue colours and comments.

**Backend requests** from the first UI round (cue colours and comments in `DeckState`, memory cue set/delete and hot cue delete, a scrub command, saving A-GAIN trims) are all built; see part C.

**Testing.** `cd apps/desktop && bun test` runs the unit tests (clock, colours and palette, waveform decoding and painters, grid and bar counter, key map, cues from the deck state, error wording, the thumbnail fetch queue and LRU); CI runs it after the frontend build.

## Later

- Key lock / master tempo (time-stretching; candidates: Rubber Band (GPL), Signalsmith Stretch (MIT)).
- Keeping cue and A-GAIN edits made in TuneOx through a re-import from rekordbox (today the import replaces them), and a write-behind queue so a deck edit never waits on an import's write lock.
- Active loops, Smart Cue, cue colour/comment editing, preview keys and cue markers on the preview thumbnails.
- Headphone cue on a second output, effects, recording, controllers.
- Own waveform generation for tracks without rekordbox analysis.
