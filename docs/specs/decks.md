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

Status: **built** (engine agent, 2026-10-12), with transport, sync and mixer behavior updated to rekordbox 7's EXPORT-mode players from research part A ([research/rekordbox-decks.md](../../research/rekordbox-decks.md) §2–3). What's still open is listed under "Open behavior questions".

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
- **Desktop:** deck/mixer commands (effect `playback`) run on a second `Context` with its own library connection and the same engine, so a long import never holds up a deck. The engine starts at launch; the UI should call `player_status` on mount for the initial state, then follow events. DUAL CONTROL is the UI's job: it sends the same command to both decks.

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
  "hot_cues": [                  // stored pads only, in slot order (incl. pads stored since loading)
    { "slot": 0, "start_us": 24000, "end_us": null },
    { "slot": 2, "start_us": 92000000, "end_us": 93904000 } ],
  "memory_cues": [               // position order; `index` is what deck_memory_cue calls
    { "index": 0, "start_us": 24000, "end_us": null } ]
}
```

- Events carry `sent_at_ms` (engine clock at send time). The UI estimates `offset = performance.now() − sent_at_ms` and keeps the **minimum** offset seen (the least-delayed delivery), then shows `position_us + (performance.now() − offset − at_ms) × 1000 × rate`.
- The engine emits a `DeckState` event on every change and about 20 times a second while a deck plays, so drift stays small. Every loop wrap is a change, so it's sent too; between events the UI should wrap its extrapolated position inside an active `loop` itself.
- Reported positions account for output latency where the device reports it (what you hear, not what was rendered): `at_ms` is when the block's first sample reaches the speakers, so it can be a few ms ahead of `sent_at_ms`.
- Desktop: `apps/desktop/src-tauri` forwards engine events to the window as `tuneox://deck` (payload: `DeckState` + `sent_at_ms`) and `tuneox://mixer` (`MixerState`).

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
  "output": { "device": "MacBook Pro Speakers", "sample_rate": 44100, "channels": 2,
              "latency_ms": 8.96, "error": null } }   // null before the device opens and offline;
                                                      // device null + error when no device could be opened
```

### Commands (`crates/tuneox-commands/src/decks.rs`)

New effect class **`playback`**: controls audio, doesn't change the library. MCP annotations: `readOnlyHint: false, destructiveHint: false, idempotentHint: false, openWorldHint: false`. All return `DeckState` unless noted.

| Command | Input | Does |
| --- | --- | --- |
| `deck_load` | `deck` (1 or 2), `track_id`, optional `auto_gain_db` | Loads with grid, hot cues and memory cues; parks at 0:00, paused, no cue, no loop. Master passes to the other deck if this one was master. Channel auto gain = `auto_gain_db` or 0 dB. |
| `deck_play` / `deck_pause` | `deck` | |
| `deck_cue` | `deck`, `action`: `press` / `release` | §2.1 rules below. |
| `deck_hot_cue` | `deck`, `slot` (0 = A … 7 = H), `action` | Stored pad: jump and play (also from pause). Empty pad: store position or playing loop. |
| `deck_memory_cue` | `deck`, `action`: `call`, `target`: index / `"next"` / `"prev"` | Jump and pause; becomes the cue point; a memory loop is armed. |
| `deck_seek` | `deck`, `position_us` | Keeps play state; leaving a loop ends it. |
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
| `mixer_set` | any of `deck` + `kill_low`/`kill_mid`/`kill_high` (bool), `auto_gain_db`, `volume` (0–1), `eq_low`/`eq_mid`/`eq_high` (dB); `crossfader` (−1 … 1); `master_volume` (0–1) | → `MixerState`. |
| `player_status` | — | → `PlayerStatus` (effect `read`; never starts the engine). |

`deck_load` looks up the track's file, beat grid and cues in the library and hands them to the engine; hot cues and memory cues come from the library. The engine starts lazily on the first deck command (the CLI/MCP server don't open an audio device unless asked); the desktop app starts it at launch so it can subscribe to events. Tests use an offline engine.

As built: `deck_load` returns `NotFound` for an unknown track or a missing file; decode problems arrive later as `error` in the state. Beat-based commands (`deck_beat_jump`, `deck_loop`, `deck_set_bpm`, `deck_sync` on) need a beat grid or at least a library BPM (then a steady grid from 0 s is assumed); without one they fail with a clear error, like rekordbox refusing SYNC on unanalysed tracks. Tempo commands on a synced slave fail with "deck 2 is synced to the master, deck 1: change the tempo on deck 1 or turn sync off on deck 2". Mixer values are clamped to their ranges; channel settings without `deck` are an error.

**Done when:** offline engine tests cover load, play/pause, CUE semantics, hot cues, seek, beat jump, loops (sample-accurate wrap), tempo (rate), sync (tempo and phase within 1 ms on synthetic grids), quantize, crossfader and EQ gains; deck commands are tested through `run_json` with an offline engine; the desktop app receives `tuneox://deck` events; `cargo clippy -D warnings` is clean.

All covered (`crates/tuneox-audio/src/tests.rs`, `crates/tuneox-commands/src/decks.rs`), plus memory cues, manual loops/reloop/resize, per-deck quantize, typed BPM and limits, pitch bend (and its sync fallback), BPM-only sync, kills and auto gain, decoding synthetic WAV/AIFF files, sync on millisecond-rounded (rekordbox-style) grids, half/double-time sync, events at ~20 Hz, and a counting allocator proving the render path (every kind of command, sync, loops, EQ, a track load) doesn't allocate or free. The desktop forwarding is built but only checked by compiling: the GUI wasn't run.

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

### Open behavior questions

| # | Now | Where |
| --- | --- | --- |
| 1 | **Pitch bend amount**: ±4 %. rekordbox nudges via `setJogRingSpeed(±0.1)`, whose percent is unknown (research open question 5). | `deck::PITCH_BEND` |
| 2 | **Crossfader curve**: constant power (both −3 dB in the middle); rekordbox's EXPORT curve is unknown (open question 6). | `mixer::crossfader_gains` |
| 3 | **Defaults**: quantize on, half/double sync always on, hot cue jumps immediate with phase kept (not "wait for next beat"); rekordbox's defaults weren't decoded (open question 7). | `Deck::default`, `Core::sync_target` |
| 4 | **Memory cue call sets the cue point** to the called cue (CDJ convention); the research says jump and pause but not whether the cue moves. | `Deck::memory_cue_call` |
| 5 | **Auto gain range** −24…+24 dB: rekordbox's knob looks like ±10 dB but stored gains reach +14.4 dB (open question 6). Imported track gains aren't applied yet: `deck_load` takes `auto_gain_db` until the library stores `djmdMixerParam` gains. | `mixer::AUTO_GAIN_*` |
| 6 | **MP3 encoder delay** (LAME/Xing gapless info) is **not** trimmed, so the first decoded frame is time 0. If rekordbox trims it, every MP3 cue and grid would be ~25 ms off here: compare against a known grid. | `decode::TRIM_ENCODER_DELAY` |
| 7 | **Extras' details**: the channel fader and master volume are linear gain; the EQ is an isolator (Linkwitz-Riley 24 dB/oct, crossovers 250 Hz / 2.5 kHz). Not part of the DUAL PLAYER, so no rekordbox reference. | `Mixer::process`, `mixer::LOW_MID_HZ` |
| 8 | **Track end**: the deck stops at the end; PLAY there does nothing until the deck is moved. | `Deck::render`, `Deck::play` |

Not built yet (later tasks): saving hot cues/memory cues stored from the pads to the library, memory cue set/delete and hot cue delete, active loops that engage automatically, fractional quantize values, "Fine" beat jump (5 ms; the UI can use `deck_seek`), browser preview pausing the decks, key lock / master tempo (rekordbox defaults it off).

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

**Keyboard:** rekordbox's EXPORT preset (research §6): plain keys for deck A, ⇧ for deck B (Space play/pause, C cue, ←/→ beat jump, 1–3 hot cues, I/O/R loops, 4–9 beat loops, F1 sync, F3 tempo reset, F4/F5 pitch bend, F6/F7 BPM −/+, T time mode). Keys must not fire while typing in the search box.

**Browser preview player** (rekordbox's Preview column): a waveform thumbnail column; clicking it previews from that position and pauses any playing deck. Needs a preview voice in the engine (see "Later" if not built yet).

## Later

- Browser preview voice in the engine (`preview_play` { track_id, position_us } / `preview_stop`), if part C doesn't include it.
- Persisting cue edits (hot cues stored from the decks) to the library.
- Key lock / master tempo (time-stretching; candidates: Rubber Band (GPL), Signalsmith Stretch (MIT)).
- Headphone cue on a second output, effects, recording, controllers.
- Own waveform generation for tracks without rekordbox analysis.
