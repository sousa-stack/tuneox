//! The dual player: two decks and a mixer, for testing transitions.
//!
//! Every command returns the state it leaves behind. Positions are
//! microseconds from the start of the track; `DeckState` is a clock
//! (`position_us` at `at_ms`, moving at `rate`) rather than a snapshot, so
//! front ends animate the playhead themselves. The audio engine starts on
//! the first command that needs it.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::Deserialize;
use tuneox_audio::{DeckState, LoadRequest, MixerSettingsUpdate, MixerState, PlayerStatus, Source};
use tuneox_core::TrackId;

use crate::library::NoInput;
use crate::{Command, Context, Effect, Error, Result};

/// A deck, 1 or 2.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckInput {
    /// 1 or 2.
    pub deck: u8,
}

/// A button going down or coming back up. CUE and hot cue pads behave
/// differently while held, like on a CDJ.
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
    f: impl FnOnce(&tuneox_audio::Engine, u8) -> tuneox_audio::Result<T>,
) -> Result<T> {
    let deck = check_deck(deck)?;
    Ok(f(ctx.engine()?, deck)?)
}

pub struct DeckLoad;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckLoadInput {
    /// 1 or 2.
    pub deck: u8,
    pub track_id: TrackId,
}

impl Command for DeckLoad {
    const NAME: &'static str = "deck_load";
    const DESCRIPTION: &'static str = "Loads a library track onto deck 1 or 2 with its beat grid, hot cues (onto pads A–H) and memory cues; the deck waits paused at the first memory cue. Returns at once while the file decodes in the background (`loading`); it can play as soon as audio is ready.";
    const EFFECT: Effect = Effect::Playback;
    type Input = DeckLoadInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckLoadInput) -> Result<DeckState> {
        let deck = check_deck(input.deck)?;
        let id = input.track_id;
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
        let request = LoadRequest {
            track_id: Some(id),
            source: Source::File(path),
            beat_grid: ctx.library.beat_grid(id)?,
            cues: ctx.library.cues(id)?,
            bpm: track.info.bpm,
            duration_secs: track.info.duration_secs,
        };
        Ok(ctx.engine()?.load(deck, request)?)
    }
}

pub struct DeckPlay;

impl Command for DeckPlay {
    const NAME: &'static str = "deck_play";
    const DESCRIPTION: &'static str = "Starts playback on a deck. While CUE or a hot cue is held from pause, PLAY keeps the deck playing after release.";
    const EFFECT: Effect = Effect::Playback;
    type Input = DeckInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckInput) -> Result<DeckState> {
        on_deck(ctx, input.deck, |e, d| e.play(d))
    }
}

pub struct DeckPause;

impl Command for DeckPause {
    const NAME: &'static str = "deck_pause";
    const DESCRIPTION: &'static str = "Pauses a deck where it is.";
    const EFFECT: Effect = Effect::Playback;
    type Input = DeckInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckInput) -> Result<DeckState> {
        on_deck(ctx, input.deck, |e, d| e.pause(d))
    }
}

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
    const DESCRIPTION: &'static str = "The CUE button, as on a CDJ. Press while playing: jump back to the cue point and pause. Press while paused elsewhere: set the cue point there (snapped to the beat with quantize). Press while paused at the cue point: play while held; release returns to the cue point and pauses.";
    const EFFECT: Effect = Effect::Playback;
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
    const DESCRIPTION: &'static str = "A hot cue pad (slot 0 = A … 7 = H). Empty pad: store the current position (snapped with quantize). While playing: jump there and keep playing (with quantize, keeping the beat phase). While paused: play from it while held, return on release. A hot loop also starts looping.";
    const EFFECT: Effect = Effect::Playback;
    type Input = DeckHotCueInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckHotCueInput) -> Result<DeckState> {
        on_deck(ctx, input.deck, |e, d| {
            e.hot_cue(d, input.slot, input.action.pressed())
        })
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
    type Input = DeckSeekInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckSeekInput) -> Result<DeckState> {
        on_deck(ctx, input.deck, |e, d| e.seek(d, input.position_us))
    }
}

pub struct DeckBeatJump;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckBeatJumpInput {
    /// 1 or 2.
    pub deck: u8,
    /// Beats to jump; negative jumps back. Fractions are allowed.
    pub beats: f64,
}

impl Command for DeckBeatJump {
    const NAME: &'static str = "deck_beat_jump";
    const DESCRIPTION: &'static str = "Jumps a deck by a number of beats on its beat grid, keeping the beat phase. Inside an active loop, the loop moves too.";
    const EFFECT: Effect = Effect::Playback;
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
    /// Loop length in beats, e.g. 1/4, 1, 4, 16.
    pub beats: f64,
}

impl Command for DeckLoop {
    const NAME: &'static str = "deck_loop";
    const DESCRIPTION: &'static str = "Auto beat loop: loops `beats` beats from the current beat (the nearest beat with quantize). With a loop already active, resizes it from its start.";
    const EFFECT: Effect = Effect::Playback;
    type Input = DeckLoopInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckLoopInput) -> Result<DeckState> {
        if !(input.beats > 0.0 && input.beats <= 512.0) {
            return Err(Error::InvalidInput(
                "loop length must be more than 0 and at most 512 beats".into(),
            ));
        }
        on_deck(ctx, input.deck, |e, d| e.beat_loop(d, input.beats))
    }
}

pub struct DeckLoopExit;

impl Command for DeckLoopExit {
    const NAME: &'static str = "deck_loop_exit";
    const DESCRIPTION: &'static str =
        "Ends a deck's active loop; playback continues past the loop end.";
    const EFFECT: Effect = Effect::Playback;
    type Input = DeckInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckInput) -> Result<DeckState> {
        on_deck(ctx, input.deck, |e, d| e.loop_exit(d))
    }
}

pub struct DeckSetTempo;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckSetTempoInput {
    /// 1 or 2.
    pub deck: u8,
    /// Tempo change in percent, clamped to the deck's tempo range.
    pub percent: f64,
}

impl Command for DeckSetTempo {
    const NAME: &'static str = "deck_set_tempo";
    const DESCRIPTION: &'static str = "Sets a deck's tempo fader in percent (pitch follows tempo). Clamped to the deck's range. Has no effect while the deck is synced to another deck.";
    const EFFECT: Effect = Effect::Playback;
    type Input = DeckSetTempoInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckSetTempoInput) -> Result<DeckState> {
        on_deck(ctx, input.deck, |e, d| e.set_tempo(d, input.percent))
    }
}

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
    type Input = DeckSetTempoRangeInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckSetTempoRangeInput) -> Result<DeckState> {
        on_deck(ctx, input.deck, |e, d| e.set_tempo_range(d, input.range))
    }
}

pub struct DeckSync;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeckSyncInput {
    /// 1 or 2.
    pub deck: u8,
    pub on: bool,
}

impl Command for DeckSync {
    const NAME: &'static str = "deck_sync";
    const DESCRIPTION: &'static str = "Turns beat sync on or off for a deck. A synced deck matches the master deck's BPM (or half/double time) and beat phase until sync is turned off, then keeps that tempo. With no master yet, the other deck becomes master if it's playing, otherwise this one.";
    const EFFECT: Effect = Effect::Playback;
    type Input = DeckSyncInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckSyncInput) -> Result<DeckState> {
        on_deck(ctx, input.deck, |e, d| e.sync(d, input.on))
    }
}

pub struct DeckSetMaster;

impl Command for DeckSetMaster {
    const NAME: &'static str = "deck_set_master";
    const DESCRIPTION: &'static str = "Makes a deck the sync master that synced decks follow. Otherwise the first deck to play is master, and the role passes to the other deck when the master stops while it plays.";
    const EFFECT: Effect = Effect::Playback;
    type Input = DeckInput;
    type Output = DeckState;

    fn run(ctx: &mut Context, input: DeckInput) -> Result<DeckState> {
        on_deck(ctx, input.deck, |e, d| e.set_master(d))
    }
}

pub struct PlayerSetQuantize;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlayerSetQuantizeInput {
    pub on: bool,
}

impl Command for PlayerSetQuantize {
    const NAME: &'static str = "player_set_quantize";
    const DESCRIPTION: &'static str = "Turns quantize on or off for both decks (on by default). With quantize, setting cues, triggering hot cues and starting loops snap to the beat grid.";
    const EFFECT: Effect = Effect::Playback;
    type Input = PlayerSetQuantizeInput;
    type Output = PlayerStatus;

    fn run(ctx: &mut Context, input: PlayerSetQuantizeInput) -> Result<PlayerStatus> {
        Ok(ctx.engine()?.set_quantize(input.on)?)
    }
}

pub struct MixerSet;

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MixerSetInput {
    /// The channel (1 or 2) that `volume` and the EQ settings apply to.
    pub deck: Option<u8>,
    /// Channel fader, 0–1.
    pub volume: Option<f32>,
    /// Low EQ in dB, −80 (kill) to +6.
    pub eq_low: Option<f32>,
    /// Mid EQ in dB, −80 (kill) to +6.
    pub eq_mid: Option<f32>,
    /// High EQ in dB, −80 (kill) to +6.
    pub eq_high: Option<f32>,
    /// −1 = deck 1 only, 0 = both, 1 = deck 2 only.
    pub crossfader: Option<f32>,
    /// 0–1.
    pub master_volume: Option<f32>,
}

impl Command for MixerSet {
    const NAME: &'static str = "mixer_set";
    const DESCRIPTION: &'static str = "Changes any of: a channel's fader (`deck` + `volume`, 0–1) and 3-band EQ (`deck` + `eq_low`/`eq_mid`/`eq_high` in dB, −80 kills the band, up to +6), the crossfader (−1 = deck 1 … 1 = deck 2) and the master volume (0–1). Settings left out don't change; values are clamped to their ranges.";
    const EFFECT: Effect = Effect::Playback;
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
            crossfader: input.crossfader,
            master_volume: input.master_volume,
        };
        Ok(ctx.engine()?.set_mixer(update)?)
    }
}

pub struct GetPlayerStatus;

impl Command for GetPlayerStatus {
    const NAME: &'static str = "player_status";
    const DESCRIPTION: &'static str = "Both decks, the mixer, quantize and the audio output. Doesn't start the audio engine: before any deck command, it reports `running: false` and empty decks.";
    const EFFECT: Effect = Effect::Read;
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
        /// 20 s at 120 BPM with a grid, a memory cue at 1 s and hot cue A
        /// at 4 s.
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

    fn render(engine: &Engine, seconds: f64) {
        let mut out = vec![0.0; (seconds * RATE as f64) as usize * 2];
        engine.render(&mut out);
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

        let state = run(ctx, "deck_load", json!({ "deck": 1, "track_id": track }));
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
            "sync",
            "master",
            "cue_us",
            "loop",
            "hot_cues",
        ] {
            assert!(state.get(key).is_some(), "DeckState has {key}");
        }
        let state = engine
            .wait_until_loaded(1, Duration::from_secs(10))
            .unwrap();
        assert!(!state.loading);
        assert_eq!(state.error, None);
        assert_eq!(state.duration_us, 20_000_000);
        assert_eq!(state.cue_us, 1_000_000, "first memory cue");
        assert_eq!(state.position_us, 1_000_000);
        assert_eq!(state.bpm, Some(120.0));
        assert_eq!(state.hot_cues[0].start_us, 4_000_000);

        let state = run(ctx, "deck_play", json!({ "deck": 1 }));
        assert_eq!(state["playing"], true);
        assert_eq!(state["rate"], 1.0);
        render(&engine, 0.2);

        // Quantized hot cue jump keeps the 0.2 s offset from the beat.
        let state = run(
            ctx,
            "deck_hot_cue",
            json!({ "deck": 1, "slot": 0, "action": "press" }),
        );
        assert_eq!(state["position_us"], 4_200_000);
        run(
            ctx,
            "deck_hot_cue",
            json!({ "deck": 1, "slot": 0, "action": "release" }),
        );

        // CUE while playing: back to the cue point, paused.
        let state = run(ctx, "deck_cue", json!({ "deck": 1, "action": "press" }));
        assert_eq!(state["playing"], false);
        assert_eq!(state["position_us"], 1_000_000);
        run(ctx, "deck_cue", json!({ "deck": 1, "action": "release" }));

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
            json!({ "start_us": 4_000_000, "end_us": 5_000_000 })
        );
        let state = run(ctx, "deck_loop_exit", json!({ "deck": 1 }));
        assert_eq!(state["loop"], Value::Null);

        let state = run(
            ctx,
            "deck_set_tempo_range",
            json!({ "deck": 1, "range": 16 }),
        );
        assert_eq!(state["tempo_range"], 16.0);
        let state = run(ctx, "deck_set_tempo", json!({ "deck": 1, "percent": 25 }));
        assert_eq!(state["tempo_percent"], 16.0);
        assert_eq!(state["bpm"], 139.2);

        // Deck 2 syncs to deck 1.
        run(ctx, "deck_load", json!({ "deck": 2, "track_id": track }));
        engine
            .wait_until_loaded(2, Duration::from_secs(10))
            .unwrap();
        run(ctx, "deck_play", json!({ "deck": 1 }));
        let state = run(ctx, "deck_sync", json!({ "deck": 2, "on": true }));
        assert_eq!(state["sync"], true);
        assert_eq!(state["master"], false);
        assert_eq!(state["bpm"], 139.2);
        let state = run(ctx, "deck_set_master", json!({ "deck": 2 }));
        assert_eq!(state["master"], true);
        run(ctx, "deck_pause", json!({ "deck": 1 }));

        let status = run(ctx, "player_set_quantize", json!({ "on": false }));
        assert_eq!(status["quantize"], false);
        assert_eq!(status["running"], true);

        let mixer = run(
            ctx,
            "mixer_set",
            json!({ "deck": 2, "volume": 0.5, "eq_low": -80, "crossfader": 1 }),
        );
        assert_eq!(mixer["channels"][1]["volume"], 0.5);
        assert_eq!(mixer["channels"][1]["eq_low"], -80.0);
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
            err(ctx, "deck_loop", json!({ "deck": 1, "beats": 0 })),
            Error::InvalidInput(_)
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
            err(ctx, "mixer_set", json!({ "volume": 0.5 })),
            Error::Audio(tuneox_audio::Error::InvalidInput(_))
        ));
    }

    #[test]
    fn player_status_does_not_start_the_engine() {
        let library = Library::open_in_memory().unwrap();
        let mut ctx = Context::new(library, None);
        let status = run(&mut ctx, "player_status", json!({}));
        assert_eq!(status["running"], false);
        assert_eq!(status["quantize"], true);
        assert_eq!(status["decks"][1]["deck"], 2);
        assert!(ctx.running_engine().is_none());
    }

    #[test]
    fn deck_commands_are_playback_commands() {
        let catalog = catalog();
        let effect = |name: &str| catalog.iter().find(|c| c.name == name).unwrap().effect;
        for name in [
            "deck_load",
            "deck_play",
            "deck_pause",
            "deck_cue",
            "deck_hot_cue",
            "deck_seek",
            "deck_beat_jump",
            "deck_loop",
            "deck_loop_exit",
            "deck_set_tempo",
            "deck_set_tempo_range",
            "deck_sync",
            "deck_set_master",
            "player_set_quantize",
            "mixer_set",
        ] {
            assert_eq!(effect(name), Effect::Playback, "{name}");
        }
        assert_eq!(effect("player_status"), Effect::Read);
        let load = catalog.iter().find(|c| c.name == "deck_load").unwrap();
        let properties = &load.output_schema["properties"];
        for key in ["position_us", "at_ms", "rate", "loop", "hot_cues"] {
            assert!(properties[key].is_object(), "schema has {key}");
        }
    }
}
