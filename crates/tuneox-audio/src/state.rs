//! What the engine reports: deck and mixer state, events, status. These are
//! the JSON shapes the command layer returns and the desktop app forwards.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One deck, as of `at_ms` on the engine clock.
///
/// The playhead is a clock, not a stream of positions: the deck was at
/// `position_us` at `at_ms` and moves `rate` track microseconds per real
/// microsecond, so the UI extrapolates
/// `position_us + (now_ms − at_ms) × 1000 × rate` every frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeckState {
    /// 1 or 2.
    pub deck: u8,
    pub track_id: Option<i64>,
    /// The track is still being decoded. Playback can start before it ends.
    pub loading: bool,
    /// Why the track couldn't be loaded or decoded.
    pub error: Option<String>,
    /// Audio is moving: PLAY is on, or CUE/a hot cue is held from pause.
    pub playing: bool,
    /// Where the deck was (as heard, after output latency) at `at_ms`.
    pub position_us: i64,
    /// Engine clock (monotonic milliseconds) when `position_us` was heard.
    pub at_ms: f64,
    /// Track microseconds per real microsecond; 0 when paused or waiting for
    /// the decoder.
    pub rate: f64,
    /// Track length; an estimate while `loading`.
    pub duration_us: i64,
    /// Effective tempo change in percent (with sync on, what sync chose).
    pub tempo_percent: f64,
    /// Tempo fader range in percent: 6, 10, 16 or 100 (WIDE).
    pub tempo_range: f64,
    /// Effective tempo here: the grid's BPM × the tempo factor (without a
    /// pitch bend), 40–499.
    pub bpm: Option<f64>,
    /// The beat grid's BPM at the playhead.
    pub original_bpm: Option<f64>,
    /// `beat` (tempo and phase), `bpm` (tempo only; also what beat sync
    /// falls back to after a pitch bend, until SYNC is toggled) or `off`.
    pub sync_mode: SyncMode,
    /// This deck is the sync master.
    pub master: bool,
    /// Quantize for this deck: cue/loop/hot cue set and hot cue jumps snap
    /// to the beat.
    pub quantize: bool,
    /// A pitch bend button held right now.
    pub pitch_bend: Option<BendDirection>,
    /// The cue point; none until one is set (a freshly loaded deck has none).
    pub cue_us: Option<i64>,
    /// The active loop, if any.
    #[serde(rename = "loop")]
    pub active_loop: Option<LoopState>,
    /// The most recent loop, active or exited: where RELOOP goes.
    pub last_loop: Option<LoopState>,
    /// Loop in set with `deck_loop_in`, waiting for `deck_loop_out`.
    pub loop_in_us: Option<i64>,
    /// The deck is being scrubbed (`deck_scrub`): the read head follows a
    /// target position, `rate` is its current speed (negative backwards),
    /// and `playing` says whether it plays on when the scrub ends.
    pub scrubbing: bool,
    /// Hot cues A–H that are set, in slot order. Includes cues set from the
    /// pads since the track was loaded.
    pub hot_cues: Vec<HotCueState>,
    /// Memory cues and loops in position order; `index` is what
    /// `deck_memory_cue` calls.
    pub memory_cues: Vec<MemoryCueState>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    #[default]
    Off,
    /// Tempo and beat phase.
    Beat,
    /// Tempo only.
    Bpm,
}

/// What beat sync aligns: the beat (rekordbox) or, a TuneOx extra, the bar
/// (downbeat to downbeat).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SyncPhase {
    #[default]
    Beat,
    Bar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BendDirection {
    Back,
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LoopState {
    pub start_us: i64,
    pub end_us: i64,
    /// Length in beats on the grid, if the track has one.
    pub beats: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryCueState {
    /// 0 = the earliest.
    pub index: u8,
    pub start_us: i64,
    /// End of a memory loop.
    pub end_us: Option<i64>,
    /// The cue's colour from the library; `None` = the default (memory
    /// cues stored on the deck have none).
    pub color: Option<u8>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HotCueState {
    /// 0 = A … 7 = H.
    pub slot: u8,
    pub start_us: i64,
    /// End of a hot loop.
    pub end_us: Option<i64>,
    /// Index into rekordbox's 62-colour hot cue palette; `None` = the
    /// colour scheme's default for the slot (pads stored on the deck have
    /// none).
    pub color: Option<u8>,
    pub comment: Option<String>,
}

impl DeckState {
    /// A deck with nothing loaded.
    pub fn empty(deck: u8) -> Self {
        Self {
            deck,
            track_id: None,
            loading: false,
            error: None,
            playing: false,
            position_us: 0,
            at_ms: 0.0,
            rate: 0.0,
            duration_us: 0,
            tempo_percent: 0.0,
            tempo_range: 10.0,
            bpm: None,
            original_bpm: None,
            sync_mode: SyncMode::Off,
            master: false,
            quantize: true,
            pitch_bend: None,
            cue_us: None,
            active_loop: None,
            last_loop: None,
            loop_in_us: None,
            scrubbing: false,
            hot_cues: Vec::new(),
            memory_cues: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChannelState {
    /// 1 or 2.
    pub deck: u8,
    /// Channel fader, 0–1 (linear gain).
    pub volume: f32,
    /// EQ bands in dB, −80 (kill) to +6.
    pub eq_low: f32,
    pub eq_mid: f32,
    pub eq_high: f32,
    /// Band kills (rekordbox DUAL PLAYER's LOW/MID/HIGH), independent of
    /// the EQ gains.
    pub kill_low: bool,
    pub kill_mid: bool,
    pub kill_high: bool,
    /// Per-track loudness trim in dB (rekordbox's A-GAIN).
    pub auto_gain_db: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MixerState {
    /// Channels for decks 1 and 2.
    pub channels: [ChannelState; 2],
    /// −1 = deck 1 only, 0 = both, 1 = deck 2 only (constant-power curve).
    pub crossfader: f32,
    /// 0–1 (linear gain).
    pub master_volume: f32,
}

/// The audio output the engine plays through.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OutputInfo {
    /// Output device name; `None` when no device could be opened, in which
    /// case the engine runs silently on a timer so decks keep moving.
    pub device: Option<String>,
    pub sample_rate: u32,
    pub channels: u16,
    /// Delay from rendering to hearing, as reported by the device.
    pub latency_ms: f64,
    /// Why no device is in use, if one couldn't be opened.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlayerStatus {
    /// False until a deck command starts the engine.
    pub running: bool,
    /// Decks 1 and 2.
    pub decks: [DeckState; 2],
    pub mixer: MixerState,
    /// "Gate playback during Pause": a hot cue pressed while paused plays
    /// only while held. Off by default (it jumps and plays).
    pub hot_cue_gate: bool,
    /// Quantize resolution in beats for both decks: 0.125, 0.25, 0.5 or 1
    /// (rekordbox EXPORT's "Quantize Beat Value"; default 1).
    pub quantize_beats: f64,
    /// What beat sync aligns: `beat` (rekordbox) or `bar` (a TuneOx extra).
    pub sync_phase: SyncPhase,
    /// The browser's preview player.
    pub preview: PreviewState,
    pub output: Option<OutputInfo>,
}

impl PlayerStatus {
    /// Status before the engine starts.
    pub fn idle() -> Self {
        Self {
            running: false,
            decks: [DeckState::empty(1), DeckState::empty(2)],
            mixer: crate::Engine::default_mixer_state(),
            hot_cue_gate: false,
            quantize_beats: 1.0,
            sync_phase: SyncPhase::Beat,
            preview: PreviewState::idle(),
            output: None,
        }
    }
}

/// The browser's preview player (rekordbox's Preview column): one track at
/// a time from a position, at its own auto gain, through the master output.
/// No tempo, sync, cues or loops. A clock like [`DeckState`]: it was at
/// `position_us` at `at_ms` and moves at `rate`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PreviewState {
    /// The track previewed last (kept after it stops), if any.
    pub track_id: Option<i64>,
    /// Audio is moving.
    pub playing: bool,
    /// The track is still being decoded.
    pub loading: bool,
    /// Why the track couldn't be decoded.
    pub error: Option<String>,
    /// Where the preview was (as heard) at `at_ms`.
    pub position_us: i64,
    /// Engine clock (monotonic milliseconds) when `position_us` was heard.
    pub at_ms: f64,
    /// 1 while playing, 0 when stopped or waiting for the decoder.
    pub rate: f64,
    /// Track length; an estimate while `loading`.
    pub duration_us: i64,
    /// The gain it plays at: the track's auto gain.
    pub auto_gain_db: f32,
}

impl PreviewState {
    /// Nothing previewed yet.
    pub fn idle() -> Self {
        Self {
            track_id: None,
            playing: false,
            loading: false,
            error: None,
            position_us: 0,
            at_ms: 0.0,
            rate: 0.0,
            duration_us: 0,
            auto_gain_db: 0.0,
        }
    }
}

/// A deck changed, or a playing deck's periodic clock update.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeckEvent {
    #[serde(flatten)]
    pub state: DeckState,
    /// Engine clock when the event was sent. The UI keeps the smallest
    /// `performance.now() − sent_at_ms` it has seen as its clock offset.
    pub sent_at_ms: f64,
}

/// The preview player changed, or its periodic clock update while playing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PreviewEvent {
    #[serde(flatten)]
    pub state: PreviewState,
    /// Engine clock when the event was sent.
    pub sent_at_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EngineEvent {
    Deck(Box<DeckEvent>),
    Mixer(MixerState),
    Preview(Box<PreviewEvent>),
}
