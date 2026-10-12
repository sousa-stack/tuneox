//! The dual player: two decks and a mixer, for testing transitions like
//! rekordbox's DUAL PLAYER (research/rekordbox-decks.md).
//!
//! Every command returns the state it leaves behind. Positions are
//! microseconds from the start of the track; `DeckState` is a clock
//! (`position_us` at `at_ms`, moving at `rate`) rather than a snapshot, so
//! front ends animate the playhead themselves. The audio engine starts on
//! the first command that needs it.
//!
//! Cue edits made on a deck are saved to the deck's track in the library,
//! as rekordbox does: a hot cue stored from an empty pad, hot cue and memory
//! cue deletes, memory cues set with MEMORY, and A-GAIN trims. Those
//! commands are effect class `edit`; the rest are `playback`.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::Deserialize;
use tuneox_audio::{
    BendDirection, DeckState, Engine, LoadRequest, MemoryCueTarget, MixerSettingsUpdate,
    MixerState, PlayerStatus, PreviewRequest, PreviewState, ScrubAction, Source, SyncMode,
    SyncPhase,
};
use tuneox_core::{Cue, TrackId, TrackInfo};

use crate::library::NoInput;
use crate::{Command, Context, Effect, Error, Result};

/// A deck, 1 or 2.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckInput {
    /// 1 or 2.
    pub deck: u8,
}

/// A button going down or coming back up. CUE, gated hot cues and pitch
/// bend act while held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ButtonAction {
    Press,
    Release,
}

impl ButtonAction {
    fn pressed(self) -> bool {
        self == ButtonAction::Press
    }
}

fn check_deck(deck: u8) -> Result<u8> {
    match deck {
        1 | 2 => Ok(deck),
        _ => Err(Error::InvalidInput(format!(
            "deck must be 1 or 2, not {deck}"
        ))),
    }
}

/// Runs `f` on the engine (starting it if needed) for a valid deck.
fn on_deck<T>(
    ctx: &mut Context,
    deck: u8,
    f: impl FnOnce(&Engine, u8) -> tuneox_audio::Result<T>,
) -> Result<T> {
    let deck = check_deck(deck)?;
    Ok(f(ctx.engine()?, deck)?)
}

/// Declares a command that takes only a deck.
macro_rules! deck_command {
    ($command:ident, $name:literal, $description:literal, $method:ident) => {
        pub struct $command;

        impl Command for $command {
            const NAME: &'static str = $name;
            const DESCRIPTION: &'static str = $description;
            const EFFECT: Effect = Effect::Playback;
            const PLAYER: bool = true;
            type Input = DeckInput;
            type Output = DeckState;

            fn run(ctx: &mut Context, input: DeckInput) -> Result<DeckState> {
                on_deck(ctx, input.deck, |e, d| e.$method(d))
            }
        }
    };
}

pub struct DeckLoad;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckLoadInput {
    /// 1 or 2.
    pub deck: u8,
    pub track_id: TrackId,
    /// Auto gain in dB. Defaults to the track's own (imported from
    /// rekordbox), capped at ±12 dB, or 0 dB if it has none.
    pub auto_gain_db: Option<f32>,
}

/// The most auto gain a track's stored value can apply. rekordbox stores
/// gains above +15 dB only for short sound effects.
const MAX_TRACK_GAIN_DB: f64 = 12.0;

/// The auto gain a library track plays at: its stored gain (rekordbox's
/// analysis or a saved A-GAIN trim), capped.
fn track_gain_db(info: &TrackInfo) -> Option<f32> {
    info.auto_gain_db
        .map(|db| db.clamp(-MAX_TRACK_GAIN_DB, MAX_TRACK_GAIN_DB) as f32)
}

/// A library track and its audio file.
fn track_file(ctx: &Context, id: TrackId) -> Result<(TrackInfo, PathBuf)> {
    let track = ctx
        .library
        .track(id)?
        .ok_or_else(|| Error::NotFound(format!("track {id}")))?;
    let path = PathBuf::from(&track.info.path);
    if !path.is_file() {
        return Err(Error::NotFound(format!(
            "the audio file of track {id}: {}",
            path.display()
        )));
    }
    Ok((track.info, path))
}

impl Command for DeckLoad {
    const NAME: &'static str = "deck_load";
    const DESCRIPTION: &'static str = "Loads a library track onto deck 1 or 2 with its beat grid, hot cues (pads A–H) and memory cues. The deck parks at 0:00, paused, with no cue point and no loop; if it was sync master, master passes to the other deck. Returns at once while the file decodes in the background (`loading`); it can play as soon as audio is ready.";
    const EFFECT: Effect = Effect::Playback;
    const PLAYER: bool = true;
    type Input = DeckLoadInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckLoadInput) -> Result<DeckState> {
        let deck = check_deck(input.deck)?;
        let id = input.track_id;
        let (info, path) = track_file(ctx, id)?;
        let request = LoadRequest {
            track_id: Some(id),
            source: Source::File(path),
            beat_grid: ctx.library.beat_grid(id)?,
            cues: ctx.library.cues(id)?,
            bpm: info.bpm,
            duration_secs: info.duration_secs,
            auto_gain_db: input.auto_gain_db.or_else(|| track_gain_db(&info)),
        };
        Ok(ctx.engine()?.load(deck, request)?)
    }
}

deck_command!(
    DeckPlay,
    "deck_play",
    "Starts playback on a deck. While CUE is held from pause, PLAY keeps the deck playing after CUE is released.",
    play
);

deck_command!(DeckPause, "deck_pause", "Pauses a deck where it is.", pause);

pub struct DeckCue;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckCueInput {
    /// 1 or 2.
    pub deck: u8,
    pub action: ButtonAction,
}

impl Command for DeckCue {
    const NAME: &'static str = "deck_cue";
    const DESCRIPTION: &'static str = "The CUE button. Press while playing: jump back to the cue point (the start if none is set) and pause. Press while paused away from the cue point: set the cue point there (snapped to the beat with quantize). Press while paused at the cue point: play while held; release returns to the cue point and pauses, unless PLAY was pressed meanwhile.";
    const EFFECT: Effect = Effect::Playback;
    const PLAYER: bool = true;
    type Input = DeckCueInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckCueInput) -> Result<DeckState> {
        on_deck(ctx, input.deck, |e, d| e.cue(d, input.action.pressed()))
    }
}

pub struct DeckHotCue;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckHotCueInput {
    /// 1 or 2.
    pub deck: u8,
    /// Pad 0 = A … 7 = H.
    pub slot: u8,
    pub action: ButtonAction,
}

impl Command for DeckHotCue {
    const NAME: &'static str = "deck_hot_cue";
    const DESCRIPTION: &'static str = "A hot cue pad (slot 0 = A … 7 = H). A stored pad jumps there and plays, also from pause (with quantize while playing, keeping the beat phase); a hot loop starts looping. An empty pad stores the current position (snapped with quantize), or the playing loop as a hot loop, and saves it to the track in the library (as rekordbox does). With player_set_hot_cue_gate on, a pad pressed while paused plays only until released.";
    // Pressing an empty pad saves a hot cue to the library.
    const EFFECT: Effect = Effect::Edit;
    const PLAYER: bool = true;
    type Input = DeckHotCueInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckHotCueInput) -> Result<DeckState> {
        let deck = check_deck(input.deck)?;
        let engine = ctx.engine()?.clone();
        let pressed = input.action.pressed();
        let slot = input.slot;
        let was_empty = pressed && !engine.deck(deck)?.hot_cues.iter().any(|c| c.slot == slot);
        let state = engine.hot_cue(deck, slot, pressed)?;
        if was_empty
            && let (Some(track), Some(stored)) = (
                state.track_id,
                state.hot_cues.iter().find(|c| c.slot == slot),
            )
        {
            let cue = Cue {
                hot_cue: Some(slot),
                start_us: stored.start_us,
                end_us: stored.end_us,
                color: stored.color,
                comment: stored.comment.clone(),
            };
            ctx.library.add_cue(track, &cue)?;
        }
        Ok(state)
    }
}

pub struct DeckHotCueDelete;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckHotCueDeleteInput {
    /// 1 or 2.
    pub deck: u8,
    /// Pad 0 = A … 7 = H.
    pub slot: u8,
}

impl Command for DeckHotCueDelete {
    const NAME: &'static str = "deck_hot_cue_delete";
    const DESCRIPTION: &'static str = "Clears a hot cue pad (slot 0 = A … 7 = H) on a deck and deletes that hot cue from the track in the library. An empty pad is left alone.";
    const EFFECT: Effect = Effect::Edit;
    const PLAYER: bool = true;
    type Input = DeckHotCueDeleteInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckHotCueDeleteInput) -> Result<DeckState> {
        let deck = check_deck(input.deck)?;
        let (state, removed) = ctx.engine()?.hot_cue_delete(deck, input.slot)?;
        if let (Some(track), Some(_)) = (state.track_id, removed) {
            ctx.library.delete_hot_cue(track, input.slot)?;
        }
        Ok(state)
    }
}

pub struct DeckMemoryCue;

/// What a memory cue command does. Only calling exists for now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCueAction {
    Call,
}

/// A memory cue by index (0 = the earliest), or relative to the playhead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum MemoryCueTargetInput {
    Index(u8),
    Step(MemoryCueStep),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCueStep {
    Next,
    Prev,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckMemoryCueInput {
    /// 1 or 2.
    pub deck: u8,
    pub action: MemoryCueAction,
    /// An index into the deck's `memory_cues` (0 = the earliest), or
    /// "next" / "prev" for the one after / before the playhead.
    pub target: MemoryCueTargetInput,
}

impl Command for DeckMemoryCue {
    const NAME: &'static str = "deck_memory_cue";
    const DESCRIPTION: &'static str = "Calls a memory cue or memory loop: jumps there and pauses, and it becomes the cue point. A memory loop is armed, so deck_play loops it. Target an index in the deck's `memory_cues` (0 = earliest) or \"next\"/\"prev\" relative to the playhead.";
    const EFFECT: Effect = Effect::Playback;
    const PLAYER: bool = true;
    type Input = DeckMemoryCueInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckMemoryCueInput) -> Result<DeckState> {
        let MemoryCueAction::Call = input.action;
        let target = match input.target {
            MemoryCueTargetInput::Index(i) => MemoryCueTarget::Index(i),
            MemoryCueTargetInput::Step(MemoryCueStep::Next) => MemoryCueTarget::Next,
            MemoryCueTargetInput::Step(MemoryCueStep::Prev) => MemoryCueTarget::Previous,
        };
        on_deck(ctx, input.deck, |e, d| e.memory_cue(d, target))
    }
}

pub struct DeckMemoryCueSet;

impl Command for DeckMemoryCueSet {
    const NAME: &'static str = "deck_memory_cue_set";
    const DESCRIPTION: &'static str = "rekordbox's MEMORY button: saves a memory cue on the deck and to the track in the library. During a loop it saves the loop as a memory loop; just after one (the loop was set after the cue point) the last loop; otherwise the cue point (set it with deck_cue while paused). Up to 10 per track, as in rekordbox; saving one that's already there changes nothing. Fails if there's no cue point or loop to save.";
    const EFFECT: Effect = Effect::Edit;
    const PLAYER: bool = true;
    type Input = DeckInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckInput) -> Result<DeckState> {
        let deck = check_deck(input.deck)?;
        let (state, added) = ctx.engine()?.memory_cue_set(deck)?;
        if let (Some(track), Some(cue)) = (state.track_id, added) {
            ctx.library.add_cue(track, &cue)?;
        }
        Ok(state)
    }
}

pub struct DeckMemoryCueDelete;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckMemoryCueDeleteInput {
    /// 1 or 2.
    pub deck: u8,
    /// The memory cue's `index` in the deck's `memory_cues` (0 = the
    /// earliest).
    pub index: u8,
}

impl Command for DeckMemoryCueDelete {
    const NAME: &'static str = "deck_memory_cue_delete";
    const DESCRIPTION: &'static str = "Deletes a memory cue or memory loop, by its index in the deck's `memory_cues` (0 = the earliest), from the deck and from the track in the library. The later ones move up an index.";
    const EFFECT: Effect = Effect::Edit;
    const PLAYER: bool = true;
    type Input = DeckMemoryCueDeleteInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckMemoryCueDeleteInput) -> Result<DeckState> {
        let deck = check_deck(input.deck)?;
        let (state, removed) = ctx.engine()?.memory_cue_delete(deck, input.index)?;
        if let Some(track) = state.track_id {
            ctx.library
                .delete_memory_cue(track, removed.start_us, removed.end_us)?;
        }
        Ok(state)
    }
}

pub struct DeckSeek;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckSeekInput {
    /// 1 or 2.
    pub deck: u8,
    /// Microseconds from the start of the track.
    pub position_us: i64,
}

impl Command for DeckSeek {
    const NAME: &'static str = "deck_seek";
    const DESCRIPTION: &'static str = "Moves a deck's playhead (like needle search), keeping it playing or paused. Leaving an active loop ends it.";
    const EFFECT: Effect = Effect::Playback;
    const PLAYER: bool = true;
    type Input = DeckSeekInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckSeekInput) -> Result<DeckState> {
        on_deck(ctx, input.deck, |e, d| e.seek(d, input.position_us))
    }
}

pub struct DeckScrub;

/// A phase of a scrub (dragging the waveform like a record).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScrubActionInput {
    /// Hand on the record (at `position_us`, or where the deck is).
    Start,
    /// Drag to `position_us`.
    Move,
    /// Let go (at `position_us`, or the last position dragged to).
    End,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckScrubInput {
    /// 1 or 2.
    pub deck: u8,
    pub action: ScrubActionInput,
    /// Microseconds from the start of the track. Required for "move".
    pub position_us: Option<i64>,
}

impl Command for DeckScrub {
    const NAME: &'static str = "deck_scrub";
    const DESCRIPTION: &'static str = "Scrubs a deck like a record moved by hand (dragging the enlarged waveform): \"start\" puts the hand on it (a playing deck slows to a stop unless dragged), \"move\" drags it to `position_us`, \"end\" lets go. The audio follows the drag smoothly at the speed it moves (backwards too) without per-move fades; on \"end\" the deck plays on from there if it was playing, otherwise it rests there paused. A \"move\" without a \"start\" starts one. `scrubbing` is true in the state meanwhile.";
    const EFFECT: Effect = Effect::Playback;
    const PLAYER: bool = true;
    type Input = DeckScrubInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckScrubInput) -> Result<DeckState> {
        let action = match input.action {
            ScrubActionInput::Start => ScrubAction::Start,
            ScrubActionInput::Move => ScrubAction::Move,
            ScrubActionInput::End => ScrubAction::End,
        };
        if action == ScrubAction::Move && input.position_us.is_none() {
            return Err(Error::InvalidInput(
                "a scrub \"move\" needs position_us".into(),
            ));
        }
        on_deck(ctx, input.deck, |e, d| {
            e.scrub(d, action, input.position_us)
        })
    }
}

pub struct DeckBeatJump;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckBeatJumpInput {
    /// 1 or 2.
    pub deck: u8,
    /// Beats to jump; negative jumps back. rekordbox offers 4, 8, 16, 32
    /// (8 bars), 64 and 128 beats; fractions work too.
    pub beats: f64,
}

impl Command for DeckBeatJump {
    const NAME: &'static str = "deck_beat_jump";
    const DESCRIPTION: &'static str = "Jumps a deck by a number of beats on its beat grid, keeping the beat phase. During a loop, the loop moves with it.";
    const EFFECT: Effect = Effect::Playback;
    const PLAYER: bool = true;
    type Input = DeckBeatJumpInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckBeatJumpInput) -> Result<DeckState> {
        on_deck(ctx, input.deck, |e, d| e.beat_jump(d, input.beats))
    }
}

pub struct DeckLoop;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckLoopInput {
    /// 1 or 2.
    pub deck: u8,
    /// Loop length in beats, 1/64 to 512.
    pub beats: f64,
}

impl Command for DeckLoop {
    const NAME: &'static str = "deck_loop";
    const DESCRIPTION: &'static str = "Auto beat loop: loops `beats` beats (1/64–512) from the current position, snapped to the beat with quantize. The same size again exits the loop; another size resizes the active loop from its start.";
    const EFFECT: Effect = Effect::Playback;
    const PLAYER: bool = true;
    type Input = DeckLoopInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckLoopInput) -> Result<DeckState> {
        on_deck(ctx, input.deck, |e, d| e.beat_loop(d, input.beats))
    }
}

deck_command!(
    DeckLoopIn,
    "deck_loop_in",
    "Manual loop IN: marks loop-in at the playhead (snapped to the beat with quantize) and makes it the cue point. deck_loop_out then closes the loop.",
    loop_in
);

deck_command!(
    DeckLoopOut,
    "deck_loop_out",
    "Manual loop OUT: loops from loop-in to the playhead (snapped with quantize) and starts looping. With a loop already playing, moves its end to the playhead.",
    loop_out
);

deck_command!(
    DeckReloop,
    "deck_reloop",
    "RELOOP: jumps back to the start of the last loop (`last_loop`) and loops it again.",
    reloop
);

deck_command!(
    DeckLoopExit,
    "deck_loop_exit",
    "Ends a deck's active loop; playback runs on past loop-out. deck_reloop brings it back.",
    loop_exit
);

pub struct DeckLoopResize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LoopFactor {
    Double,
    Half,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckLoopResizeInput {
    /// 1 or 2.
    pub deck: u8,
    pub factor: LoopFactor,
}

impl Command for DeckLoopResize {
    const NAME: &'static str = "deck_loop_resize";
    const DESCRIPTION: &'static str =
        "Doubles or halves the active loop, keeping its start, within 1/64–512 beats.";
    const EFFECT: Effect = Effect::Playback;
    const PLAYER: bool = true;
    type Input = DeckLoopResizeInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckLoopResizeInput) -> Result<DeckState> {
        on_deck(ctx, input.deck, |e, d| {
            e.loop_resize(d, input.factor == LoopFactor::Double)
        })
    }
}

pub struct DeckSetTempo;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckSetTempoInput {
    /// 1 or 2.
    pub deck: u8,
    /// Tempo change in percent, clamped to the deck's tempo range and to
    /// 40–499 BPM.
    pub percent: f64,
}

impl Command for DeckSetTempo {
    const NAME: &'static str = "deck_set_tempo";
    const DESCRIPTION: &'static str = "Sets a deck's tempo in percent (pitch follows tempo), clamped to its range (slider steps: 0.02 % at ±6, 0.04 % at ±10/±16, 0.5 % at WIDE; rekordbox's −/+ buttons step 0.02 %, 0.1 % when held) and to 40–499 BPM. Refused on a deck that's synced to another deck: only the master's tempo can change.";
    const EFFECT: Effect = Effect::Playback;
    const PLAYER: bool = true;
    type Input = DeckSetTempoInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckSetTempoInput) -> Result<DeckState> {
        on_deck(ctx, input.deck, |e, d| e.set_tempo(d, input.percent))
    }
}

pub struct DeckSetBpm;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckSetBpmInput {
    /// 1 or 2.
    pub deck: u8,
    /// The tempo to play at, 40–499 BPM.
    pub bpm: f64,
}

impl Command for DeckSetBpm {
    const NAME: &'static str = "deck_set_bpm";
    const DESCRIPTION: &'static str = "Sets a deck's tempo by BPM, like typing it into rekordbox's BPM field. It must be 40–499 BPM and reachable within the deck's tempo range. Refused on a deck synced to another deck.";
    const EFFECT: Effect = Effect::Playback;
    const PLAYER: bool = true;
    type Input = DeckSetBpmInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckSetBpmInput) -> Result<DeckState> {
        on_deck(ctx, input.deck, |e, d| e.set_bpm(d, input.bpm))
    }
}

deck_command!(
    DeckTempoReset,
    "deck_tempo_reset",
    "Returns a deck to its track's own tempo (0 %). Refused on a deck synced to another deck.",
    tempo_reset
);

pub struct DeckSetTempoRange;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckSetTempoRangeInput {
    /// 1 or 2.
    pub deck: u8,
    /// ±6, ±10, ±16 or ±100 (WIDE) percent.
    pub range: f64,
}

impl Command for DeckSetTempoRange {
    const NAME: &'static str = "deck_set_tempo_range";
    const DESCRIPTION: &'static str = "Sets a deck's tempo fader range: 6, 10, 16 or 100 (WIDE) percent. The tempo is clamped to the new range.";
    const EFFECT: Effect = Effect::Playback;
    const PLAYER: bool = true;
    type Input = DeckSetTempoRangeInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckSetTempoRangeInput) -> Result<DeckState> {
        on_deck(ctx, input.deck, |e, d| e.set_tempo_range(d, input.range))
    }
}

pub struct DeckPitchBend;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckPitchBendInput {
    /// 1 or 2.
    pub deck: u8,
    pub direction: BendDirection,
    pub action: ButtonAction,
}

impl Command for DeckPitchBend {
    const NAME: &'static str = "deck_pitch_bend";
    const DESCRIPTION: &'static str = "Pitch bend: while pressed, a playing deck runs 4 % slower (\"back\") or faster (\"forward\") to nudge it into phase; release ends it. Bending a beat-synced deck drops it to BPM-only sync (sync_mode \"bpm\") until sync is turned off and on again.";
    const EFFECT: Effect = Effect::Playback;
    const PLAYER: bool = true;
    type Input = DeckPitchBendInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckPitchBendInput) -> Result<DeckState> {
        on_deck(ctx, input.deck, |e, d| {
            e.pitch_bend(d, input.direction, input.action.pressed())
        })
    }
}

pub struct DeckSync;

/// How sync follows the master when turned on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SyncType {
    /// Tempo and beat phase (rekordbox's default).
    Beat,
    /// Tempo only.
    Bpm,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckSyncInput {
    /// 1 or 2.
    pub deck: u8,
    pub on: bool,
    /// When turning sync on: "beat" (default) or "bpm".
    pub mode: Option<SyncType>,
}

impl Command for DeckSync {
    const NAME: &'static str = "deck_sync";
    const DESCRIPTION: &'static str = "Turns sync on or off for a deck. A synced deck matches the master deck's BPM (or half/double time) and, in beat mode, its beat phase; its own tempo controls are locked until sync is turned off, which keeps the synced tempo. With no master yet, the other deck becomes master if it's playing, otherwise this one.";
    const EFFECT: Effect = Effect::Playback;
    const PLAYER: bool = true;
    type Input = DeckSyncInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckSyncInput) -> Result<DeckState> {
        let mode = match (input.on, input.mode) {
            (false, _) => SyncMode::Off,
            (true, Some(SyncType::Bpm)) => SyncMode::Bpm,
            (true, _) => SyncMode::Beat,
        };
        on_deck(ctx, input.deck, |e, d| e.sync(d, mode))
    }
}

deck_command!(
    DeckSetMaster,
    "deck_set_master",
    "Makes a deck the sync master that synced decks follow. Otherwise the first deck to start playing becomes master, and master passes to the other deck when the master's track is replaced.",
    set_master
);

pub struct DeckSetQuantize;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckSetQuantizeInput {
    /// 1 or 2.
    pub deck: u8,
    pub on: bool,
}

impl Command for DeckSetQuantize {
    const NAME: &'static str = "deck_set_quantize";
    const DESCRIPTION: &'static str = "Turns quantize on or off for one deck (on by default). With quantize, setting the cue, loop in/out, storing hot cues and auto loop starts snap to the nearest beat, and hot cue jumps while playing keep the beat phase.";
    const EFFECT: Effect = Effect::Playback;
    const PLAYER: bool = true;
    type Input = DeckSetQuantizeInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckSetQuantizeInput) -> Result<DeckState> {
        on_deck(ctx, input.deck, |e, d| e.set_quantize(d, input.on))
    }
}

pub struct PlayerSetHotCueGate;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlayerSetHotCueGateInput {
    pub on: bool,
}

impl Command for PlayerSetHotCueGate {
    const NAME: &'static str = "player_set_hot_cue_gate";
    const DESCRIPTION: &'static str = "rekordbox's \"Gate playback during Pause\" preference, for both decks (off by default). On: a hot cue pressed while paused plays only while held, then returns and pauses (hot loops still latch). Off: it jumps and plays.";
    const EFFECT: Effect = Effect::Playback;
    const PLAYER: bool = true;
    type Input = PlayerSetHotCueGateInput;
    type Output = PlayerStatus;

    fn run(ctx: &mut Context, input: PlayerSetHotCueGateInput) -> Result<PlayerStatus> {
        Ok(ctx.engine()?.set_hot_cue_gate(input.on)?)
    }
}

pub struct PlayerSetQuantizeValue;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlayerSetQuantizeValueInput {
    /// 0.125, 0.25, 0.5 or 1 (beats).
    pub beats: f64,
}

impl Command for PlayerSetQuantizeValue {
    const NAME: &'static str = "player_set_quantize_value";
    const DESCRIPTION: &'static str = "rekordbox's \"Quantize Beat Value\" preference, for both decks: 1/8, 1/4, 1/2 or 1 beat (`beats`: 0.125, 0.25, 0.5 or 1; default 1). On a deck with quantize on, setting the cue, loop in/out, storing a hot cue and auto loop starts snap to that resolution, and hot cue jumps while playing keep the phase at it. Beat jump sizes don't change.";
    const EFFECT: Effect = Effect::Playback;
    const PLAYER: bool = true;
    type Input = PlayerSetQuantizeValueInput;
    type Output = PlayerStatus;

    fn run(ctx: &mut Context, input: PlayerSetQuantizeValueInput) -> Result<PlayerStatus> {
        Ok(ctx.engine()?.set_quantize_beats(input.beats)?)
    }
}

pub struct PlayerSetSyncPhase;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlayerSetSyncPhaseInput {
    pub unit: SyncPhase,
}

impl Command for PlayerSetSyncPhase {
    const NAME: &'static str = "player_set_sync_phase";
    const DESCRIPTION: &'static str = "What beat sync aligns, for both decks: \"beat\" (default; rekordbox's BEAT SYNC aligns the beat phase only) or \"bar\" (a TuneOx extra: the synced deck's place in the bar matches the master's, downbeat to downbeat, by the beat grid's bar positions, moving it at most half a bar). BPM-only sync and half/double-time sync are unaffected (they keep beat alignment).";
    const EFFECT: Effect = Effect::Playback;
    const PLAYER: bool = true;
    type Input = PlayerSetSyncPhaseInput;
    type Output = PlayerStatus;

    fn run(ctx: &mut Context, input: PlayerSetSyncPhaseInput) -> Result<PlayerStatus> {
        Ok(ctx.engine()?.set_sync_phase(input.unit)?)
    }
}

pub struct MixerSet;

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MixerSetInput {
    /// The channel (1 or 2) that the channel settings below apply to.
    pub deck: Option<u8>,
    /// Channel fader, 0–1.
    pub volume: Option<f32>,
    /// Low EQ in dB, −80 (kill) to +6.
    pub eq_low: Option<f32>,
    /// Mid EQ in dB, −80 (kill) to +6.
    pub eq_mid: Option<f32>,
    /// High EQ in dB, −80 (kill) to +6.
    pub eq_high: Option<f32>,
    /// Low band kill (rekordbox DUAL PLAYER's LOW button).
    pub kill_low: Option<bool>,
    /// Mid band kill.
    pub kill_mid: Option<bool>,
    /// High band kill.
    pub kill_high: Option<bool>,
    /// Auto gain in dB (rekordbox's A-GAIN), −24 to +24.
    pub auto_gain_db: Option<f32>,
    /// −1 = deck 1 only, 0 = both, 1 = deck 2 only.
    pub crossfader: Option<f32>,
    /// 0–1.
    pub master_volume: Option<f32>,
}

impl Command for MixerSet {
    const NAME: &'static str = "mixer_set";
    const DESCRIPTION: &'static str = "Changes any of: a channel's (`deck` 1 or 2) low/mid/high kills (`kill_low`/`kill_mid`/`kill_high`, independent of the EQ), auto gain (`auto_gain_db`), fader (`volume`, 0–1) and 3-band EQ (`eq_low`/`eq_mid`/`eq_high` in dB, −80 kills, up to +6); the crossfader (−1 = deck 1 … 1 = deck 2); the master volume (0–1). Settings left out don't change; values are clamped to their ranges. Like rekordbox, an `auto_gain_db` change is saved as the loaded track's auto gain in the library, so the track loads with it next time. rekordbox's DUAL PLAYER has only the kills, auto gain and crossfader.";
    // An auto gain change on a loaded deck saves the trim to the library.
    const EFFECT: Effect = Effect::Edit;
    const PLAYER: bool = true;
    type Input = MixerSetInput;
    type Output = MixerState;

    fn run(ctx: &mut Context, input: MixerSetInput) -> Result<MixerState> {
        let deck = input.deck.map(check_deck).transpose()?;
        let update = MixerSettingsUpdate {
            deck,
            volume: input.volume,
            eq_low: input.eq_low,
            eq_mid: input.eq_mid,
            eq_high: input.eq_high,
            kill_low: input.kill_low,
            kill_mid: input.kill_mid,
            kill_high: input.kill_high,
            auto_gain_db: input.auto_gain_db,
            crossfader: input.crossfader,
            master_volume: input.master_volume,
        };
        let engine = ctx.engine()?.clone();
        let mixer = engine.set_mixer(update)?;
        // rekordbox saves a manual A-GAIN adjustment with the track.
        if let (Some(deck), Some(_)) = (deck, input.auto_gain_db)
            && let Some(track) = engine.deck(deck)?.track_id
        {
            let db = mixer.channels[deck as usize - 1].auto_gain_db as f64;
            ctx.library
                .set_auto_gain(track, Some((db * 100.0).round() / 100.0))?;
        }
        Ok(mixer)
    }
}

pub struct PreviewPlay;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewPlayInput {
    pub track_id: TrackId,
    /// Where to start, in microseconds from the start of the track
    /// (default 0).
    pub position_us: Option<i64>,
}

impl Command for PreviewPlay {
    const NAME: &'static str = "preview_play";
    const DESCRIPTION: &'static str = "The browser's preview player (rekordbox's Preview column): plays a library track from `position_us` at its own auto gain through the master output, and pauses any deck that's playing. No tempo, sync, cues or loops. Previewing the track that's already in the player only moves its playhead. A deck starting to play stops the preview. Returns the preview's state (a clock like a deck's).";
    const EFFECT: Effect = Effect::Playback;
    const PLAYER: bool = true;
    type Input = PreviewPlayInput;
    type Output = PreviewState;

    fn run(ctx: &mut Context, input: PreviewPlayInput) -> Result<PreviewState> {
        let id = input.track_id;
        let (info, path) = track_file(ctx, id)?;
        let request = PreviewRequest {
            track_id: Some(id),
            source: Source::File(path),
            position_us: input.position_us.unwrap_or(0).max(0),
            duration_secs: info.duration_secs,
            auto_gain_db: track_gain_db(&info),
        };
        Ok(ctx.engine()?.preview_play(request)?)
    }
}

pub struct PreviewStop;

impl Command for PreviewStop {
    const NAME: &'static str = "preview_stop";
    const DESCRIPTION: &'static str =
        "Stops the browser's preview player (with a short fade). Decks it paused stay paused.";
    const EFFECT: Effect = Effect::Playback;
    const PLAYER: bool = true;
    type Input = NoInput;
    type Output = PreviewState;

    fn run(ctx: &mut Context, _: NoInput) -> Result<PreviewState> {
        Ok(match ctx.running_engine() {
            Some(engine) => engine.preview_stop()?,
            None => PreviewState::idle(),
        })
    }
}

pub struct GetPlayerStatus;

impl Command for GetPlayerStatus {
    const NAME: &'static str = "player_status";
    const DESCRIPTION: &'static str = "Both decks, the mixer, the hot cue gate setting, the browser's preview player and the audio output. Doesn't start the audio engine: before any deck command, it reports `running: false` and empty decks.";
    const EFFECT: Effect = Effect::Read;
    const PLAYER: bool = true;
    type Input = NoInput;
    type Output = PlayerStatus;

    fn run(ctx: &mut Context, _: NoInput) -> Result<PlayerStatus> {
        Ok(match ctx.running_engine() {
            Some(engine) => engine.status(),
            None => PlayerStatus::idle(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use serde_json::{Value, json};
    use tuneox_audio::Engine;
    use tuneox_core::{Beat, BeatGrid, Cue, TrackInfo};
    use tuneox_library::Library;

    use crate::{Context, Effect, Error, catalog, run_json};

    const RATE: u32 = 48_000;

    /// A 16-bit stereo WAV of a click on every beat at 120 BPM.
    fn write_click_wav(path: &Path, seconds: usize) {
        let frames = seconds * RATE as usize;
        let mut data = Vec::with_capacity(frames * 4);
        for i in 0..frames {
            let since_beat = i % (RATE as usize / 2);
            let v: i16 = if since_beat < 32 { 20_000 } else { 0 };
            data.extend_from_slice(&v.to_le_bytes());
            data.extend_from_slice(&v.to_le_bytes());
        }
        let mut file = Vec::new();
        file.extend_from_slice(b"RIFF");
        file.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        file.extend_from_slice(b"WAVEfmt ");
        file.extend_from_slice(&16u32.to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&2u16.to_le_bytes());
        file.extend_from_slice(&RATE.to_le_bytes());
        file.extend_from_slice(&(RATE * 4).to_le_bytes());
        file.extend_from_slice(&4u16.to_le_bytes());
        file.extend_from_slice(&16u16.to_le_bytes());
        file.extend_from_slice(b"data");
        file.extend_from_slice(&(data.len() as u32).to_le_bytes());
        file.extend_from_slice(&data);
        std::fs::write(path, file).unwrap();
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        ctx: Context,
        engine: Engine,
        /// 20 s at 120 BPM with a grid, memory cues at 1 s and 6 s and hot
        /// cue A at 4 s.
        track: i64,
        /// The same audio without a beat grid or BPM.
        no_grid: i64,
        /// A track whose file is gone.
        missing: i64,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("clicks.wav");
        write_click_wav(&wav, 20);
        let copy = dir.path().join("clicks copy.wav");
        std::fs::copy(&wav, &copy).unwrap();

        let mut library = Library::open_in_memory().unwrap();
        let import = library.import("test", "lib").unwrap();
        let info = |path: &Path, bpm: Option<f64>| TrackInfo {
            path: path.display().to_string(),
            title: "Clicks".into(),
            bpm,
            duration_secs: Some(20.0),
            ..Default::default()
        };
        let track = import
            .upsert_track("1", &info(&wav, Some(120.0)), None)
            .unwrap()
            .id;
        let no_grid = import
            .upsert_track("2", &info(&copy, None), None)
            .unwrap()
            .id;
        let missing = import
            .upsert_track("3", &info(&dir.path().join("gone.wav"), Some(120.0)), None)
            .unwrap()
            .id;
        let beats = (0..40)
            .map(|i| Beat {
                time_us: i * 500_000,
                bar_position: (i % 4) as u8 + 1,
                bpm_centi: 12_000,
            })
            .collect();
        import.set_beat_grid(track, &BeatGrid { beats }).unwrap();
        let cue = |hot_cue: Option<u8>, start_us: i64| Cue {
            hot_cue,
            start_us,
            end_us: None,
            color: None,
            comment: None,
        };
        import
            .replace_cues(
                track,
                &[
                    (cue(None, 6_000_000), None),
                    (cue(None, 1_000_000), None),
                    (cue(Some(0), 4_000_000), None),
                ],
            )
            .unwrap();
        import.commit().unwrap();

        let engine = Engine::offline(RATE);
        let ctx = Context::new(library, None).with_engine(engine.clone());
        Fixture {
            _dir: dir,
            ctx,
            engine,
            track,
            no_grid,
            missing,
        }
    }

    fn run(ctx: &mut Context, name: &str, input: Value) -> Value {
        run_json(ctx, name, input).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    fn assert_close(value: &Value, expected: f64) {
        let actual = value.as_f64().expect("a number");
        assert!((actual - expected).abs() < 1e-9, "{actual} vs {expected}");
    }

    fn render(engine: &Engine, seconds: f64) {
        let mut out = vec![0.0; (seconds * RATE as f64) as usize * 2];
        engine.render(&mut out);
    }

    #[test]
    fn deck_load_applies_the_tracks_auto_gain_capped() {
        let Fixture {
            _dir,
            mut ctx,
            engine,
            no_grid,
            ..
        } = fixture();
        let mut info = ctx.library.track(no_grid).unwrap().unwrap().info;
        info.auto_gain_db = Some(20.0);
        let import = ctx.library.import("test", "lib").unwrap();
        import.upsert_track("2", &info, None).unwrap();
        import.commit().unwrap();

        run(
            &mut ctx,
            "deck_load",
            json!({ "deck": 2, "track_id": no_grid }),
        );
        assert_eq!(engine.mixer().channels[1].auto_gain_db, 12.0);
        run(
            &mut ctx,
            "deck_load",
            json!({ "deck": 2, "track_id": no_grid, "auto_gain_db": -3 }),
        );
        assert_eq!(
            engine.mixer().channels[1].auto_gain_db,
            -3.0,
            "explicit gain wins"
        );
    }

    #[test]
    fn deck_commands_drive_the_engine() {
        let Fixture {
            _dir,
            mut ctx,
            engine,
            track,
            ..
        } = fixture();
        let ctx = &mut ctx;

        let state = run(
            ctx,
            "deck_load",
            json!({ "deck": 1, "track_id": track, "auto_gain_db": 2.5 }),
        );
        assert_eq!(state["deck"], 1);
        assert_eq!(state["track_id"], track);
        for key in [
            "loading",
            "error",
            "playing",
            "position_us",
            "at_ms",
            "rate",
            "duration_us",
            "tempo_percent",
            "tempo_range",
            "bpm",
            "original_bpm",
            "sync_mode",
            "master",
            "quantize",
            "pitch_bend",
            "cue_us",
            "loop",
            "last_loop",
            "loop_in_us",
            "hot_cues",
            "memory_cues",
        ] {
            assert!(state.get(key).is_some(), "DeckState has {key}");
        }
        let state = engine
            .wait_until_loaded(1, Duration::from_secs(10))
            .unwrap();
        assert!(!state.loading);
        assert_eq!(state.error, None);
        assert_eq!(state.duration_us, 20_000_000);
        assert_eq!(state.cue_us, None, "no cue after loading");
        assert_eq!(state.position_us, 0);
        assert_eq!(state.bpm, Some(120.0));
        assert_eq!(state.hot_cues[0].start_us, 4_000_000);
        assert_eq!(state.memory_cues.len(), 2);
        assert_eq!(engine.mixer().channels[0].auto_gain_db, 2.5);

        // A hot cue from pause jumps and plays.
        let state = run(
            ctx,
            "deck_hot_cue",
            json!({ "deck": 1, "slot": 0, "action": "press" }),
        );
        assert_eq!(state["playing"], true);
        assert_eq!(state["position_us"], 4_000_000);
        run(
            ctx,
            "deck_hot_cue",
            json!({ "deck": 1, "slot": 0, "action": "release" }),
        );
        render(&engine, 0.2);

        // CUE while playing with no cue set: back to the start, paused.
        let state = run(ctx, "deck_cue", json!({ "deck": 1, "action": "press" }));
        assert_eq!(state["playing"], false);
        assert_eq!(state["position_us"], 0);
        run(ctx, "deck_cue", json!({ "deck": 1, "action": "release" }));

        // Memory cues: call by index or next/prev; it pauses there.
        let state = run(
            ctx,
            "deck_memory_cue",
            json!({ "deck": 1, "action": "call", "target": 1 }),
        );
        assert_eq!(state["position_us"], 6_000_000);
        assert_eq!(state["cue_us"], 6_000_000);
        let state = run(
            ctx,
            "deck_memory_cue",
            json!({ "deck": 1, "action": "call", "target": "prev" }),
        );
        assert_eq!(state["position_us"], 1_000_000);

        let state = run(
            ctx,
            "deck_seek",
            json!({ "deck": 1, "position_us": 2_000_000 }),
        );
        assert_eq!(state["position_us"], 2_000_000);
        let state = run(ctx, "deck_beat_jump", json!({ "deck": 1, "beats": 4 }));
        assert_eq!(state["position_us"], 4_000_000);
        let state = run(ctx, "deck_loop", json!({ "deck": 1, "beats": 2 }));
        assert_eq!(
            state["loop"],
            json!({ "start_us": 4_000_000, "end_us": 5_000_000, "beats": 2.0 })
        );
        let state = run(ctx, "deck_loop", json!({ "deck": 1, "beats": 2 }));
        assert_eq!(state["loop"], Value::Null, "same size exits");
        let state = run(ctx, "deck_reloop", json!({ "deck": 1 }));
        assert_eq!(state["loop"]["start_us"], 4_000_000);
        let state = run(
            ctx,
            "deck_loop_resize",
            json!({ "deck": 1, "factor": "double" }),
        );
        assert_eq!(state["loop"]["end_us"], 6_000_000);
        let state = run(ctx, "deck_loop_exit", json!({ "deck": 1 }));
        assert_eq!(state["loop"], Value::Null);
        assert_eq!(state["last_loop"]["end_us"], 6_000_000);
        run(
            ctx,
            "deck_seek",
            json!({ "deck": 1, "position_us": 8_200_000 }),
        );
        let state = run(ctx, "deck_loop_in", json!({ "deck": 1 }));
        assert_eq!(state["loop_in_us"], 8_000_000);
        run(
            ctx,
            "deck_seek",
            json!({ "deck": 1, "position_us": 9_000_000 }),
        );
        let state = run(ctx, "deck_loop_out", json!({ "deck": 1 }));
        assert_eq!(
            state["loop"],
            json!({ "start_us": 8_000_000, "end_us": 9_000_000, "beats": 2.0 })
        );
        run(ctx, "deck_loop_exit", json!({ "deck": 1 }));

        let state = run(
            ctx,
            "deck_set_tempo_range",
            json!({ "deck": 1, "range": 16 }),
        );
        assert_eq!(state["tempo_range"], 16.0);
        let state = run(ctx, "deck_set_tempo", json!({ "deck": 1, "percent": 25 }));
        assert_eq!(state["tempo_percent"], 16.0);
        assert_close(&state["bpm"], 139.2);
        let state = run(ctx, "deck_set_bpm", json!({ "deck": 1, "bpm": 126 }));
        assert_close(&state["bpm"], 126.0);
        let state = run(ctx, "deck_tempo_reset", json!({ "deck": 1 }));
        assert_eq!(state["tempo_percent"], 0.0);
        let state = run(ctx, "deck_set_quantize", json!({ "deck": 1, "on": false }));
        assert_eq!(state["quantize"], false);

        // Deck 2 syncs to deck 1 (master: it started playing first).
        run(ctx, "deck_load", json!({ "deck": 2, "track_id": track }));
        engine
            .wait_until_loaded(2, Duration::from_secs(10))
            .unwrap();
        run(ctx, "deck_set_bpm", json!({ "deck": 1, "bpm": 125 }));
        run(ctx, "deck_play", json!({ "deck": 1 }));
        let state = run(ctx, "deck_sync", json!({ "deck": 2, "on": true }));
        assert_eq!(state["sync_mode"], "beat");
        assert_eq!(state["master"], false);
        assert_close(&state["bpm"], 125.0);
        let err = run_json(ctx, "deck_set_tempo", json!({ "deck": 2, "percent": 1 })).unwrap_err();
        assert!(err.to_string().contains("deck 1"), "{err}");
        let state = run(
            ctx,
            "deck_sync",
            json!({ "deck": 2, "on": true, "mode": "bpm" }),
        );
        assert_eq!(state["sync_mode"], "bpm");
        run(ctx, "deck_play", json!({ "deck": 2 }));
        let state = run(
            ctx,
            "deck_pitch_bend",
            json!({ "deck": 2, "direction": "forward", "action": "press" }),
        );
        assert_eq!(state["pitch_bend"], "forward");
        let state = run(
            ctx,
            "deck_pitch_bend",
            json!({ "deck": 2, "direction": "forward", "action": "release" }),
        );
        assert_eq!(state["pitch_bend"], Value::Null);
        let state = run(ctx, "deck_set_master", json!({ "deck": 2 }));
        assert_eq!(state["master"], true);
        run(ctx, "deck_pause", json!({ "deck": 1 }));

        let status = run(ctx, "player_set_hot_cue_gate", json!({ "on": true }));
        assert_eq!(status["hot_cue_gate"], true);
        assert_eq!(status["running"], true);

        let mixer = run(
            ctx,
            "mixer_set",
            json!({ "deck": 2, "volume": 0.5, "eq_low": -80, "kill_high": true,
                    "auto_gain_db": -3, "crossfader": 1 }),
        );
        assert_eq!(mixer["channels"][1]["volume"], 0.5);
        assert_eq!(mixer["channels"][1]["eq_low"], -80.0);
        assert_eq!(mixer["channels"][1]["kill_high"], true);
        assert_eq!(mixer["channels"][1]["kill_low"], false);
        assert_eq!(mixer["channels"][1]["auto_gain_db"], -3.0);
        assert_eq!(mixer["crossfader"], 1.0);
        assert_eq!(mixer["master_volume"], 1.0);

        let status = run(ctx, "player_status", json!({}));
        assert_eq!(status["decks"][0]["deck"], 1);
        assert_eq!(status["decks"][1]["master"], true);
        assert_eq!(status["mixer"], mixer);
        assert_eq!(
            status["output"],
            Value::Null,
            "offline engines have no output"
        );
    }

    #[test]
    fn deck_commands_reject_bad_input() {
        let Fixture {
            _dir,
            mut ctx,
            engine,
            no_grid,
            missing,
            ..
        } = fixture();
        let ctx = &mut ctx;
        let err = |ctx: &mut Context, name: &str, input: Value| {
            run_json(ctx, name, input).expect_err(name)
        };
        assert!(matches!(
            err(ctx, "deck_play", json!({ "deck": 3 })),
            Error::InvalidInput(_)
        ));
        assert!(matches!(
            err(ctx, "deck_load", json!({ "deck": 1, "track_id": 999 })),
            Error::NotFound(_)
        ));
        assert!(matches!(
            err(ctx, "deck_load", json!({ "deck": 1, "track_id": missing })),
            Error::NotFound(_)
        ));
        assert!(matches!(
            err(ctx, "deck_play", json!({ "deck": 1, "speed": 2 })),
            Error::InvalidInput(_)
        ));
        assert!(matches!(
            err(ctx, "deck_cue", json!({ "deck": 1, "action": "tap" })),
            Error::InvalidInput(_)
        ));
        assert!(matches!(
            err(
                ctx,
                "deck_memory_cue",
                json!({ "deck": 1, "action": "call", "target": "sideways" })
            ),
            Error::InvalidInput(_)
        ));
        assert!(matches!(
            err(ctx, "deck_play", json!({ "deck": 1 })),
            Error::Audio(tuneox_audio::Error::NoTrack(1))
        ));
        run(ctx, "deck_load", json!({ "deck": 1, "track_id": no_grid }));
        engine
            .wait_until_loaded(1, Duration::from_secs(10))
            .unwrap();
        assert!(matches!(
            err(ctx, "deck_beat_jump", json!({ "deck": 1, "beats": 4 })),
            Error::Audio(tuneox_audio::Error::NoBeatGrid(1))
        ));
        assert!(matches!(
            err(ctx, "deck_set_bpm", json!({ "deck": 1, "bpm": 120 })),
            Error::Audio(tuneox_audio::Error::NoBeatGrid(1))
        ));
        assert!(matches!(
            err(ctx, "deck_loop", json!({ "deck": 1, "beats": 0 })),
            Error::Audio(tuneox_audio::Error::InvalidInput(_))
        ));
        assert!(matches!(
            err(
                ctx,
                "deck_memory_cue",
                json!({ "deck": 1, "action": "call", "target": 0 })
            ),
            Error::Audio(tuneox_audio::Error::InvalidInput(_))
        ));
        assert!(matches!(
            err(
                ctx,
                "deck_set_tempo_range",
                json!({ "deck": 1, "range": 8 })
            ),
            Error::Audio(tuneox_audio::Error::InvalidInput(_))
        ));
        assert!(matches!(
            err(ctx, "mixer_set", json!({ "kill_low": true })),
            Error::Audio(tuneox_audio::Error::InvalidInput(_))
        ));
    }

    /// (slot, start µs, end µs) of a track's cues in the library.
    fn library_cues(ctx: &Context, track: i64) -> Vec<(Option<u8>, i64, Option<i64>)> {
        ctx.library
            .cues(track)
            .unwrap()
            .into_iter()
            .map(|c| (c.hot_cue, c.start_us, c.end_us))
            .collect()
    }

    #[test]
    fn cue_edits_on_a_deck_are_saved_to_the_library() {
        let Fixture {
            _dir,
            mut ctx,
            engine,
            track,
            ..
        } = fixture();
        let ctx = &mut ctx;
        run(ctx, "deck_load", json!({ "deck": 1, "track_id": track }));
        engine
            .wait_until_loaded(1, Duration::from_secs(10))
            .unwrap();
        let state = run(
            ctx,
            "deck_seek",
            json!({ "deck": 1, "position_us": 2_000_000 }),
        );
        // Colours and comments come with the cues (none in this fixture).
        assert_eq!(state["hot_cues"][0]["color"], Value::Null);
        assert!(state["hot_cues"][0].get("comment").is_some());
        assert!(state["memory_cues"][0].get("color").is_some());

        // An empty pad stores and saves.
        let state = run(
            ctx,
            "deck_hot_cue",
            json!({ "deck": 1, "slot": 1, "action": "press" }),
        );
        assert_eq!(state["hot_cues"][1]["start_us"], 2_000_000);
        run(
            ctx,
            "deck_hot_cue",
            json!({ "deck": 1, "slot": 1, "action": "release" }),
        );
        // A stored pad only plays.
        run(
            ctx,
            "deck_hot_cue",
            json!({ "deck": 1, "slot": 0, "action": "press" }),
        );
        run(ctx, "deck_pause", json!({ "deck": 1 }));
        assert_eq!(
            library_cues(ctx, track),
            [
                (None, 1_000_000, None),
                (None, 6_000_000, None),
                (Some(0), 4_000_000, None),
                (Some(1), 2_000_000, None),
            ]
        );

        // Hot cue delete.
        let state = run(ctx, "deck_hot_cue_delete", json!({ "deck": 1, "slot": 0 }));
        assert_eq!(state["hot_cues"].as_array().unwrap().len(), 1);
        run(ctx, "deck_hot_cue_delete", json!({ "deck": 1, "slot": 5 }));

        // MEMORY saves the cue point (set by CUE while paused).
        run(
            ctx,
            "deck_seek",
            json!({ "deck": 1, "position_us": 8_000_000 }),
        );
        run(ctx, "deck_cue", json!({ "deck": 1, "action": "press" }));
        run(ctx, "deck_cue", json!({ "deck": 1, "action": "release" }));
        let state = run(ctx, "deck_memory_cue_set", json!({ "deck": 1 }));
        assert_eq!(state["memory_cues"][2]["start_us"], 8_000_000);
        // And a loop as a memory loop.
        run(ctx, "deck_loop", json!({ "deck": 1, "beats": 4 }));
        let state = run(ctx, "deck_memory_cue_set", json!({ "deck": 1 }));
        assert_eq!(state["memory_cues"][3]["end_us"], 10_000_000);
        // Memory cue delete (the earliest).
        let state = run(
            ctx,
            "deck_memory_cue_delete",
            json!({ "deck": 1, "index": 0 }),
        );
        assert_eq!(state["memory_cues"][0]["start_us"], 6_000_000);
        assert_eq!(
            library_cues(ctx, track),
            [
                (None, 6_000_000, None),
                (None, 8_000_000, None),
                (None, 8_000_000, Some(10_000_000)),
                (Some(1), 2_000_000, None),
            ]
        );

        // Reloading brings the edits back from the library.
        run(ctx, "deck_load", json!({ "deck": 2, "track_id": track }));
        let state = engine.deck(2).unwrap();
        assert_eq!(state.hot_cues.len(), 1);
        assert_eq!(state.hot_cues[0].slot, 1);
        let starts: Vec<_> = state
            .memory_cues
            .iter()
            .map(|c| (c.start_us, c.end_us))
            .collect();
        assert_eq!(
            starts,
            [
                (6_000_000, None),
                (8_000_000, None),
                (8_000_000, Some(10_000_000))
            ]
        );

        // Errors: no cue point or loop to save, an index that isn't there.
        run(ctx, "deck_load", json!({ "deck": 1, "track_id": track }));
        assert!(matches!(
            run_json(ctx, "deck_memory_cue_set", json!({ "deck": 1 })),
            Err(Error::Audio(tuneox_audio::Error::InvalidInput(_)))
        ));
        assert!(matches!(
            run_json(
                ctx,
                "deck_memory_cue_delete",
                json!({ "deck": 1, "index": 7 })
            ),
            Err(Error::Audio(tuneox_audio::Error::InvalidInput(_)))
        ));
    }

    #[test]
    fn a_gain_trims_are_saved_and_come_back() {
        let Fixture {
            _dir,
            mut ctx,
            engine,
            track,
            ..
        } = fixture();
        let ctx = &mut ctx;
        let gain = |ctx: &Context| ctx.library.track(track).unwrap().unwrap().info.auto_gain_db;
        // Nothing loaded: nothing to save.
        run(ctx, "mixer_set", json!({ "deck": 1, "auto_gain_db": 3 }));
        assert_eq!(gain(ctx), None);
        run(ctx, "deck_load", json!({ "deck": 1, "track_id": track }));
        assert_eq!(engine.mixer().channels[0].auto_gain_db, 0.0);
        run(ctx, "mixer_set", json!({ "deck": 1, "auto_gain_db": -4.3 }));
        assert_eq!(gain(ctx), Some(-4.3));
        // Other mixer changes don't touch it.
        run(
            ctx,
            "mixer_set",
            json!({ "deck": 1, "kill_low": true, "crossfader": 0.5 }),
        );
        assert_eq!(gain(ctx), Some(-4.3));
        // Reloading the track brings the trim back.
        run(ctx, "deck_load", json!({ "deck": 2, "track_id": track }));
        assert!((engine.mixer().channels[1].auto_gain_db + 4.3).abs() < 1e-6);
        // So does previewing it.
        let preview = run(ctx, "preview_play", json!({ "track_id": track }));
        assert!((preview["auto_gain_db"].as_f64().unwrap() + 4.3).abs() < 1e-6);
    }

    #[test]
    fn deck_scrub_drags_and_lets_go() {
        let Fixture {
            _dir,
            mut ctx,
            engine,
            track,
            ..
        } = fixture();
        let ctx = &mut ctx;
        run(ctx, "deck_load", json!({ "deck": 1, "track_id": track }));
        engine
            .wait_until_loaded(1, Duration::from_secs(10))
            .unwrap();
        let state = run(
            ctx,
            "deck_scrub",
            json!({ "deck": 1, "action": "start", "position_us": 0 }),
        );
        assert_eq!(state["scrubbing"], true);
        assert_eq!(state["playing"], false);
        for step in 1..=50 {
            run(
                ctx,
                "deck_scrub",
                json!({ "deck": 1, "action": "move", "position_us": step * 10_000 }),
            );
            render(&engine, 0.01);
        }
        let state = run(ctx, "deck_scrub", json!({ "deck": 1, "action": "end" }));
        assert_eq!(state["scrubbing"], true, "settling");
        render(&engine, 0.5);
        let state = engine.deck(1).unwrap();
        assert!(!state.scrubbing && !state.playing);
        assert!(
            (state.position_us - 500_000).abs() <= 1,
            "{}",
            state.position_us
        );
        assert!(matches!(
            run_json(ctx, "deck_scrub", json!({ "deck": 1, "action": "move" })),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            run_json(ctx, "deck_scrub", json!({ "deck": 1, "action": "fling" })),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn preview_plays_a_library_track_and_pauses_the_decks() {
        let Fixture {
            _dir,
            mut ctx,
            engine,
            track,
            no_grid,
            missing,
        } = fixture();
        let ctx = &mut ctx;
        // Stopping before anything started doesn't start the engine.
        let mut idle = Context::new(Library::open_in_memory().unwrap(), None);
        let state = run(&mut idle, "preview_stop", json!({}));
        assert_eq!(state["playing"], false);
        assert!(idle.running_engine().is_none());

        run(ctx, "deck_load", json!({ "deck": 1, "track_id": track }));
        engine
            .wait_until_loaded(1, Duration::from_secs(10))
            .unwrap();
        run(ctx, "deck_play", json!({ "deck": 1 }));
        let state = run(
            ctx,
            "preview_play",
            json!({ "track_id": no_grid, "position_us": 5_000_000 }),
        );
        assert_eq!(state["track_id"], no_grid);
        assert_eq!(state["playing"], true);
        assert_eq!(state["position_us"], 5_000_000);
        assert!(!engine.deck(1).unwrap().playing, "the deck paused");
        engine
            .wait_until_preview_loaded(Duration::from_secs(10))
            .unwrap();
        render(&engine, 0.25);
        let status = run(ctx, "player_status", json!({}));
        assert_eq!(status["preview"]["playing"], true);
        assert_eq!(status["preview"]["position_us"], 5_250_000);
        assert_eq!(status["preview"]["duration_us"], 20_000_000);
        let state = run(ctx, "preview_stop", json!({}));
        assert_eq!(state["playing"], false);
        assert_eq!(state["track_id"], no_grid);
        assert!(matches!(
            run_json(ctx, "preview_play", json!({ "track_id": missing })),
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            run_json(ctx, "preview_play", json!({ "track_id": 999 })),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn quantize_value_and_sync_phase_are_player_settings() {
        let Fixture {
            _dir,
            mut ctx,
            track,
            ..
        } = fixture();
        let ctx = &mut ctx;
        let status = run(ctx, "player_status", json!({}));
        assert_eq!(status["quantize_beats"], 1.0);
        assert_eq!(status["sync_phase"], "beat");
        let status = run(ctx, "player_set_quantize_value", json!({ "beats": 0.25 }));
        assert_eq!(status["quantize_beats"], 0.25);
        let status = run(ctx, "player_set_sync_phase", json!({ "unit": "bar" }));
        assert_eq!(status["sync_phase"], "bar");
        assert_eq!(status["quantize_beats"], 0.25);
        // With ¼ beat quantize, CUE snaps to the nearest 125 ms at 120 BPM.
        run(ctx, "deck_load", json!({ "deck": 1, "track_id": track }));
        run(
            ctx,
            "deck_seek",
            json!({ "deck": 1, "position_us": 2_130_000 }),
        );
        let state = run(ctx, "deck_cue", json!({ "deck": 1, "action": "press" }));
        assert_eq!(state["cue_us"], 2_125_000);
        assert!(matches!(
            run_json(ctx, "player_set_quantize_value", json!({ "beats": 2 })),
            Err(Error::Audio(tuneox_audio::Error::InvalidInput(_)))
        ));
        assert!(matches!(
            run_json(ctx, "player_set_sync_phase", json!({ "unit": "phrase" })),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn player_status_does_not_start_the_engine() {
        let library = Library::open_in_memory().unwrap();
        let mut ctx = Context::new(library, None);
        let status = run(&mut ctx, "player_status", json!({}));
        assert_eq!(status["running"], false);
        assert_eq!(status["hot_cue_gate"], false);
        assert_eq!(status["decks"][1]["deck"], 2);
        assert_eq!(status["decks"][1]["sync_mode"], "off");
        assert!(ctx.running_engine().is_none());
    }

    #[test]
    fn deck_commands_are_playback_commands() {
        let catalog = catalog();
        let effect = |name: &str| catalog.iter().find(|c| c.name == name).unwrap().effect;
        // Commands that can save to the library are edits.
        for name in [
            "deck_hot_cue",
            "deck_hot_cue_delete",
            "deck_memory_cue_set",
            "deck_memory_cue_delete",
            "mixer_set",
        ] {
            assert_eq!(effect(name), Effect::Edit, "{name}");
        }
        // Every deck, mixer, preview and player command runs on the player.
        for info in &catalog {
            let player = ["deck_", "mixer_", "player_", "preview_"]
                .iter()
                .any(|prefix| info.name.starts_with(prefix));
            assert_eq!(info.player, player, "{}", info.name);
        }
        for name in [
            "deck_load",
            "deck_play",
            "deck_pause",
            "deck_cue",
            "deck_memory_cue",
            "deck_seek",
            "deck_scrub",
            "preview_play",
            "preview_stop",
            "deck_beat_jump",
            "deck_loop",
            "deck_loop_in",
            "deck_loop_out",
            "deck_reloop",
            "deck_loop_resize",
            "deck_loop_exit",
            "deck_set_tempo",
            "deck_set_bpm",
            "deck_tempo_reset",
            "deck_set_tempo_range",
            "deck_pitch_bend",
            "deck_sync",
            "deck_set_master",
            "deck_set_quantize",
            "player_set_hot_cue_gate",
            "player_set_quantize_value",
            "player_set_sync_phase",
        ] {
            assert_eq!(effect(name), Effect::Playback, "{name}");
        }
        assert_eq!(effect("player_status"), Effect::Read);
        let load = catalog.iter().find(|c| c.name == "deck_load").unwrap();
        let properties = &load.output_schema["properties"];
        for key in [
            "position_us",
            "at_ms",
            "rate",
            "loop",
            "last_loop",
            "sync_mode",
            "hot_cues",
            "memory_cues",
        ] {
            assert!(properties[key].is_object(), "schema has {key}");
        }
    }
}
