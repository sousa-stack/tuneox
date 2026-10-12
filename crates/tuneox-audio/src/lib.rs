//! The TuneOx audio engine: two decks and a mixer, like rekordbox's DUAL
//! PLAYER.
//!
//! # Threads
//!
//! - **Audio thread** (cpal's callback, on the output thread's stream): runs
//!   the [`core`] DSP: applies queued commands, renders both decks with
//!   cubic resampling, applies sync and the mixer. It never allocates,
//!   locks, logs or does I/O.
//! - **Output thread** (`tuneox-audio-out`): opens the default device and
//!   owns the stream, reopening it if the device goes away, or drives the
//!   core silently on a timer when there's no device.
//! - **Event thread** (`tuneox-audio-events`): every 5 ms turns the latest
//!   state into events (on change, and ~20 Hz while playing) and frees
//!   replaced tracks.
//! - **Decoder threads** (`tuneox-decode`, one per load): decode a file into
//!   a [`pcm::TrackPcm`] the audio thread can already play from.
//! - **Callers** (any thread) hold an [`Engine`]: commands go into a
//!   lock-free queue and the call returns once the audio thread has applied
//!   them (within one device buffer), with the resulting [`DeckState`].
//!
//! The offline engine ([`Engine::offline`]) runs the same core on the
//! caller's thread, with time advanced by [`Engine::render`], for tests.

mod core;
mod deck;
mod decode;
mod engine;
mod events;
mod grid;
mod mixer;
mod output;
mod pcm;
mod state;

#[cfg(test)]
mod tests;

pub use engine::{
    Engine, LoadRequest, MixerSettingsUpdate, OFFLINE_SAMPLE_RATE, Source, TEMPO_RANGES,
};
pub use mixer::{EQ_KILL_DB, EQ_MAX_DB};
pub use state::{
    ChannelState, DeckEvent, DeckState, EngineEvent, HotCueState, LoopState, MixerState,
    OutputInfo, PlayerStatus,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("there are two decks, 1 and 2; there's no deck {0}")]
    InvalidDeck(u8),
    #[error("deck {0} has no track loaded")]
    NoTrack(u8),
    #[error("the track on deck {0} has no beat grid or BPM")]
    NoBeatGrid(u8),
    #[error("{0}")]
    InvalidInput(String),
    #[error("the audio engine is busy; try again")]
    Busy,
    #[error("the audio engine isn't responding")]
    NotResponding,
    #[error("audio output: {0}")]
    Output(String),
}

pub type Result<T> = std::result::Result<T, Error>;
