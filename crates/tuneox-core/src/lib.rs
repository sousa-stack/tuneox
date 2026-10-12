//! TuneOx domain types, shared by every crate and front end.
//!
//! Positions inside a track are integer microseconds (`*_us`). Tempos stored
//! per beat are BPM × 100, the resolution Pioneer formats use.

mod beatgrid;
mod cue;
mod key;
mod playlist;
mod track;

pub use beatgrid::{Beat, BeatGrid};
pub use cue::Cue;
pub use key::{Mode, MusicalKey};
pub use playlist::{Playlist, PlaylistId, PlaylistKind};
pub use track::{FileType, Track, TrackColor, TrackId, TrackInfo};
