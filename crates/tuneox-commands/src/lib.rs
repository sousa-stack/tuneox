//! Every TuneOx operation as a typed command.
//!
//! The desktop app, the `tuneox` CLI and the MCP server are all thin front
//! ends over this registry, so anything one of them can do, all of them can
//! do. Each command has a JSON Schema for its input and output, a description
//! written for both people and LLMs, and an [`Effect`] class that front ends
//! use to decide what needs confirmation.

mod decks;
mod import_rekordbox;
mod library;
mod waveform;

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tuneox_audio::Engine;
use tuneox_library::Library;

pub use decks::{
    ButtonAction, DeckBeatJump, DeckBeatJumpInput, DeckCue, DeckCueInput, DeckHotCue,
    DeckHotCueDelete, DeckHotCueDeleteInput, DeckHotCueInput, DeckInput, DeckLoad, DeckLoadInput,
    DeckLoop, DeckLoopExit, DeckLoopIn, DeckLoopInput, DeckLoopOut, DeckLoopResize,
    DeckLoopResizeInput, DeckMemoryCue, DeckMemoryCueDelete, DeckMemoryCueDeleteInput,
    DeckMemoryCueInput, DeckMemoryCueSet, DeckPause, DeckPitchBend, DeckPitchBendInput, DeckPlay,
    DeckReloop, DeckScrub, DeckScrubInput, DeckSeek, DeckSeekInput, DeckSetBpm, DeckSetBpmInput,
    DeckSetMaster, DeckSetQuantize, DeckSetQuantizeInput, DeckSetTempo, DeckSetTempoInput,
    DeckSetTempoRange, DeckSetTempoRangeInput, DeckSync, DeckSyncInput, DeckTempoReset,
    GetPlayerStatus, LoopFactor, MemoryCueAction, MemoryCueStep, MemoryCueTargetInput, MixerSet,
    MixerSetInput, PlayerSetHotCueGate, PlayerSetHotCueGateInput, PlayerSetQuantizeValue,
    PlayerSetQuantizeValueInput, PlayerSetSyncPhase, PlayerSetSyncPhaseInput, PreviewPlay,
    PreviewPlayInput, PreviewStop, ScrubActionInput, SyncType,
};
pub use import_rekordbox::{ImportCounts, ImportRekordbox, ImportRekordboxInput, ImportReport};
pub use library::{
    GetBeatGrid, GetBeatGridInput, GetBeatGridOutput, GetPlaylist, GetPlaylistInput,
    GetPlaylistOutput, GetTrack, GetTrackInput, GetTrackOutput, GridSummary, LibrarySummary,
    LibrarySummaryOutput, ListPlaylists, ListPlaylistsOutput, ListTracks, ListTracksInput,
    ListTracksOutput, NoInput, SearchTracks, SearchTracksInput, SearchTracksOutput,
};
pub use waveform::{GetWaveform, GetWaveformInput, GetWaveformOutput};

/// What running a command can change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// Only reads.
    Read,
    /// Changes the library in ways that can be undone.
    Edit,
    /// Deletes or overwrites data. Needs explicit confirmation.
    Destructive,
    /// Writes to a USB drive or other device. Needs explicit confirmation.
    DeviceWrite,
    /// Controls audio playback (decks, mixer, preview); doesn't change the
    /// library. A player command that can also save to the library (a hot
    /// cue stored from an empty pad, an A-GAIN trim) is `Edit` instead.
    Playback,
}

pub trait Command {
    /// snake_case name, used as the MCP tool name and `tuneox run` argument.
    const NAME: &'static str;
    const DESCRIPTION: &'static str;
    const EFFECT: Effect;
    /// Acts on the audio engine (decks, mixer, preview player), whatever
    /// its effect class. The desktop app runs these on their own context so
    /// library work (an import) never delays a deck.
    const PLAYER: bool = false;
    type Input: DeserializeOwned + JsonSchema;
    type Output: Serialize + JsonSchema;

    fn run(ctx: &mut Context, input: Self::Input) -> Result<Self::Output>;
}

/// Progress of a long-running command.
#[derive(Debug, Clone, Serialize)]
pub struct Progress<'a> {
    pub stage: &'a str,
    pub done: u64,
    pub total: u64,
}

/// What a command runs against.
pub struct Context {
    pub library: Library,
    /// Where the library lives on disk, if it isn't in memory.
    pub library_path: Option<PathBuf>,
    progress: Box<dyn FnMut(&Progress) + Send>,
    /// The audio engine, once a deck command has started it.
    engine: Option<Engine>,
}

impl Context {
    pub fn new(library: Library, library_path: Option<PathBuf>) -> Self {
        Self {
            library,
            library_path,
            progress: Box::new(|_| {}),
            engine: None,
        }
    }

    /// Uses `engine` for deck commands instead of opening the default audio
    /// output on first use: an offline engine in tests, or one shared with
    /// another context.
    pub fn with_engine(mut self, engine: Engine) -> Self {
        self.engine = Some(engine);
        self
    }

    /// The audio engine, started on the default output device the first
    /// time it's needed, so the CLI and MCP server only open a device when
    /// asked to play.
    pub fn engine(&mut self) -> Result<&Engine> {
        if self.engine.is_none() {
            self.engine = Some(Engine::start()?);
        }
        Ok(self.engine.as_ref().expect("just started"))
    }

    /// The audio engine if it's running.
    pub fn running_engine(&self) -> Option<&Engine> {
        self.engine.as_ref()
    }

    pub fn on_progress(mut self, report: impl FnMut(&Progress) + Send + 'static) -> Self {
        self.set_progress(report);
        self
    }

    /// Replaces the progress handler, e.g. to route one call's progress to
    /// the client that made it.
    pub fn set_progress(&mut self, report: impl FnMut(&Progress) + Send + 'static) {
        self.progress = Box::new(report);
    }

    fn progress(&mut self, stage: &str, done: u64, total: u64) {
        (self.progress)(&Progress { stage, done, total });
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unknown command: {0}")]
    UnknownCommand(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Library(#[from] tuneox_library::Error),
    #[error(transparent)]
    MasterDb(#[from] tuneox_pioneer::masterdb::Error),
    #[error(transparent)]
    Audio(#[from] tuneox_audio::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// A command's name, description, effect and schemas, as the CLI and MCP
/// server publish them.
#[derive(Debug, Clone, Serialize)]
pub struct CommandInfo {
    pub name: &'static str,
    pub description: &'static str,
    pub effect: Effect,
    /// Acts on the audio engine (see [`Command::PLAYER`]).
    pub player: bool,
    pub input_schema: Value,
    pub output_schema: Value,
}

fn info<C: Command>() -> CommandInfo {
    CommandInfo {
        name: C::NAME,
        description: C::DESCRIPTION,
        effect: C::EFFECT,
        player: C::PLAYER,
        input_schema: schema::<C::Input>(),
        output_schema: schema::<C::Output>(),
    }
}

/// JSON Schema 2020-12 with every definition inlined: some MCP clients and
/// models handle `$ref` poorly.
fn schema<T: JsonSchema>() -> Value {
    let schema = schemars::generate::SchemaSettings::draft2020_12()
        .with(|s| s.inline_subschemas = true)
        .into_generator()
        .into_root_schema_for::<T>();
    serde_json::to_value(schema).expect("schemas serialize")
}

fn run_typed<C: Command>(ctx: &mut Context, input: Value) -> Result<Value> {
    let input = match input {
        Value::Null => Value::Object(Default::default()),
        other => other,
    };
    let input: C::Input =
        serde_json::from_value(input).map_err(|e| Error::InvalidInput(e.to_string()))?;
    let output = C::run(ctx, input)?;
    Ok(serde_json::to_value(output).expect("outputs serialize"))
}

macro_rules! registry {
    ($($command:ty),* $(,)?) => {
        /// Every command, in the order front ends should list them.
        pub fn catalog() -> Vec<CommandInfo> {
            vec![$(info::<$command>()),*]
        }

        /// Runs a command by name with JSON input and returns JSON output.
        pub fn run_json(ctx: &mut Context, name: &str, input: Value) -> Result<Value> {
            $(
                if name == <$command as Command>::NAME {
                    return run_typed::<$command>(ctx, input);
                }
            )*
            Err(Error::UnknownCommand(name.to_string()))
        }
    };
}

registry![
    LibrarySummary,
    ListPlaylists,
    GetPlaylist,
    SearchTracks,
    ListTracks,
    GetTrack,
    GetBeatGrid,
    GetWaveform,
    ImportRekordbox,
    DeckLoad,
    DeckPlay,
    DeckPause,
    DeckCue,
    DeckHotCue,
    DeckHotCueDelete,
    DeckMemoryCue,
    DeckMemoryCueSet,
    DeckMemoryCueDelete,
    DeckSeek,
    DeckScrub,
    DeckBeatJump,
    DeckLoop,
    DeckLoopIn,
    DeckLoopOut,
    DeckReloop,
    DeckLoopResize,
    DeckLoopExit,
    DeckSetTempo,
    DeckSetBpm,
    DeckTempoReset,
    DeckSetTempoRange,
    DeckPitchBend,
    DeckSync,
    DeckSetMaster,
    DeckSetQuantize,
    PlayerSetHotCueGate,
    PlayerSetQuantizeValue,
    PlayerSetSyncPhase,
    MixerSet,
    PreviewPlay,
    PreviewStop,
    GetPlayerStatus,
];
