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
    /// Effective tempo here: the grid's BPM × the tempo factor.
    pub bpm: Option<f64>,
    /// The beat grid's BPM at the playhead.
    pub original_bpm: Option<f64>,
    pub sync: bool,
    pub master: bool,
    pub cue_us: i64,
    /// The active loop, if any.
    #[serde(rename = "loop")]
    pub active_loop: Option<LoopState>,
    /// Hot cues A–H that are set, in slot order. Includes cues set from the
    /// pads since the track was loaded.
    pub hot_cues: Vec<HotCueState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LoopState {
    pub start_us: i64,
    pub end_us: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HotCueState {
    /// 0 = A … 7 = H.
    pub slot: u8,
    pub start_us: i64,
    /// End of a hot loop.
    pub end_us: Option<i64>,
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
            sync: false,
            master: false,
            cue_us: 0,
            active_loop: None,
            hot_cues: Vec::new(),
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
    /// Global quantize: cue set, hot cues, loop in and beat jump snap to the
    /// beat grid.
    pub quantize: bool,
    pub output: Option<OutputInfo>,
}

impl PlayerStatus {
    /// Status before the engine starts.
    pub fn idle() -> Self {
        Self {
            running: false,
            decks: [DeckState::empty(1), DeckState::empty(2)],
            mixer: crate::Engine::default_mixer_state(),
            quantize: true,
            output: None,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EngineEvent {
    Deck(DeckEvent),
    Mixer(MixerState),
}
