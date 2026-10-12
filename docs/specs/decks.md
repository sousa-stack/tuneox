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

### Engine (`crates/tuneox-audio`)

- **Two decks** and a **mixer**: per-deck volume (channel fader) and 3-band EQ (low/mid/high, −∞ to +6 dB, kill at minimum), crossfader, master volume. One stereo output (the default device).
- **Decoding** with `symphonia` (MP3, AAC/M4A, FLAC, WAV, **AIFF**; the test library is mostly MP3 and AIFF). Decode on a background thread into memory in a compact form; playback can start as soon as the first second is decoded. Report decode errors as deck state, never panic.
- **Tempo** (vinyl mode for now: pitch follows tempo): fractional read position with good interpolation (at least 4-point cubic), resampling track rate → device rate in the same step. Pitch fader range ±6, ±10, ±16 % or WIDE (±100 %).
- **Sync** using the library beat grid: matches the master deck's effective BPM and aligns beat phase. One deck is master (the first one playing, or chosen explicitly). Sync stays engaged until turned off.
- **Quantize** (global, on by default): cue set, hot cue trigger, loop in and beat jump snap to the nearest beat of the grid.
- **Transport** semantics follow rekordbox/CDJ behavior as documented by research part A (CUE button, hot cues while paused/playing, beat jump, auto beat loop, loop exit). Until A lands, follow CDJ conventions.
- **Real-time rules:** the audio callback never allocates, locks, logs or does I/O. Control goes in through a lock-free queue; state comes out through lock-free snapshots. The cpal stream lives on a dedicated thread, so the engine handle is `Send + Sync`.
- **Testable offline:** the DSP core renders into a buffer without an audio device (`render(&mut self, out: &mut [f32])`), so tests can load synthetic audio (generated in the test: clicks on a known beat grid, tones), drive transport and assert on samples, positions, tempo and phase alignment.

### Clock and events

The UI animates the playhead itself; the engine only says where a deck was and how fast it's moving:

```jsonc
// DeckState: returned by every deck command and sent as an event
{ "deck": 1, "track_id": 123, "loading": false, "error": null,
  "playing": true,
  "position_us": 61234000,       // where the deck was at `at_ms`
  "at_ms": 912345.25,            // engine clock (monotonic ms) when sampled
  "rate": 1.04,                  // track microseconds per real microsecond (0 when paused)
  "duration_us": 306000000,
  "tempo_percent": 4.0, "tempo_range": 10.0,
  "bpm": 131.04, "original_bpm": 126.0,
  "sync": true, "master": false,
  "cue_us": 24000,
  "loop": { "start_us": 60000000, "end_us": 61904000 }   // or null
}
```

- Events carry `sent_at_ms` (engine clock at send time). The UI estimates `offset = performance.now() − sent_at_ms` and keeps the **minimum** offset seen (the least-delayed delivery), then shows `position_us + (performance.now() − offset − at_ms) × 1000 × rate`.
- The engine emits a `DeckState` event on every change and about 20 times a second while a deck plays, so drift stays small.
- Reported positions account for output latency where the device reports it (what you hear, not what was rendered).
- Desktop: `apps/desktop/src-tauri` forwards engine events to the window as `tuneox://deck` (payload: `DeckState` + `sent_at_ms`) and `tuneox://mixer` (`MixerState`).

### Commands (`crates/tuneox-commands/src/decks.rs`)

New effect class **`playback`**: controls audio, doesn't change the library. MCP annotations: `readOnlyHint: false, destructiveHint: false, idempotentHint: false, openWorldHint: false`.

| Command | Input | Output |
| --- | --- | --- |
| `deck_load` | `deck` (1 or 2), `track_id` | `DeckState` |
| `deck_play` / `deck_pause` | `deck` | `DeckState` |
| `deck_cue` | `deck`, `action`: `press` / `release` | `DeckState` |
| `deck_hot_cue` | `deck`, `slot` (0 = A … 7 = H), `action`: `press` / `release` | `DeckState` |
| `deck_seek` | `deck`, `position_us` | `DeckState` |
| `deck_beat_jump` | `deck`, `beats` (negative jumps back) | `DeckState` |
| `deck_loop` | `deck`, `beats` (auto beat loop from the current beat) | `DeckState` |
| `deck_loop_exit` | `deck` | `DeckState` |
| `deck_set_tempo` | `deck`, `percent` (clamped to the range) | `DeckState` |
| `deck_set_tempo_range` | `deck`, `range`: 6 / 10 / 16 / 100 | `DeckState` |
| `deck_sync` | `deck`, `on` | `DeckState` |
| `deck_set_master` | `deck` | `DeckState` |
| `player_set_quantize` | `on` | `PlayerStatus` |
| `mixer_set` | any of `deck`+`volume` (0–1), `deck`+`eq_low`/`eq_mid`/`eq_high` (dB), `crossfader` (−1 = deck 1 … 1 = deck 2), `master_volume` (0–1) | `MixerState` |
| `player_status` | — | `PlayerStatus`: both `DeckState`s, `MixerState`, `quantize` |

`deck_load` looks up the track's file, beat grid and cues in the library and hands them to the engine; hot cues and memory cues come from the library. The engine starts lazily on the first deck command (the CLI/MCP server don't open an audio device unless asked); the desktop app starts it at launch so it can subscribe to events. Tests use an offline engine.

**Done when:** offline engine tests cover load, play/pause, CUE semantics, hot cues, seek, beat jump, loops (sample-accurate wrap), tempo (rate), sync (tempo and phase within 1 ms on synthetic grids), quantize, crossfader and EQ gains; deck commands are tested through `run_json` with an offline engine; the desktop app receives `tuneox://deck` events; `cargo clippy -D warnings` is clean.

## D. Deck UI (after A–C)

A rekordbox-style dual player above the library browser:

- Per deck: overview waveform (whole track, playhead, played part dimmed, cue and hot cue markers, click to seek), scrolling detail waveform (playhead fixed in the center, beat grid lines with bar downbeats emphasized, cue/hot cue/loop markers, zoom), track title/artist, elapsed/remaining time, effective BPM and key, tempo % with fader and range, SYNC and MASTER buttons, CUE and PLAY/PAUSE, hot cue pads A–H, beat jump, auto loop sizes.
- Mixer strip: two channel faders with 3-band EQ knobs, crossfader, master volume.
- Load a track from the browser by drag-and-drop onto a deck, a context menu, or keyboard shortcuts.
- Waveform style setting (blue / RGB / 3-band) with rekordbox's colors from research A/B.
- Rendering stays at 60 fps with both decks playing: draw from cached waveform data on a canvas, animate the playhead from the clock above, never wait on the core per frame.

## Later

- Key lock / master tempo (time-stretching; candidates: Rubber Band (GPL), Signalsmith Stretch (MIT)).
- Headphone cue on a second output, effects, recording, controllers.
- Own waveform generation for tracks without rekordbox analysis.
