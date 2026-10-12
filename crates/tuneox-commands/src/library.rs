//! Read-only library commands.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tuneox_core::{Cue, MusicalKey, Playlist, PlaylistId, Track, TrackId};
use tuneox_library::SearchQuery;

use crate::{Command, Context, Effect, Error, Result};

/// Input for commands that take no arguments.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoInput {}

pub struct LibrarySummary;

#[derive(Debug, Serialize, JsonSchema)]
pub struct LibrarySummaryOutput {
    pub tracks: u64,
    /// Playlists, including smart playlists; folders are counted separately.
    pub playlists: u64,
    pub folders: u64,
    pub cues: u64,
    pub beat_grids: u64,
    pub library_path: Option<String>,
}

impl Command for LibrarySummary {
    const NAME: &'static str = "library_summary";
    const DESCRIPTION: &'static str =
        "Counts of tracks, playlists, folders, cues and beat grids in the TuneOx library.";
    const EFFECT: Effect = Effect::Read;
    type Input = NoInput;
    type Output = LibrarySummaryOutput;

    fn run(ctx: &mut Context, _: NoInput) -> Result<LibrarySummaryOutput> {
        let s = ctx.library.summary()?;
        Ok(LibrarySummaryOutput {
            tracks: s.tracks,
            playlists: s.playlists,
            folders: s.folders,
            cues: s.cues,
            beat_grids: s.beat_grids,
            library_path: ctx.library_path.as_ref().map(|p| p.display().to_string()),
        })
    }
}

pub struct ListPlaylists;

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListPlaylistsOutput {
    /// The playlist tree, flattened: parents come before their children and
    /// siblings are in order. Rebuild the tree with `parent_id`.
    pub playlists: Vec<Playlist>,
}

impl Command for ListPlaylists {
    const NAME: &'static str = "list_playlists";
    const DESCRIPTION: &'static str =
        "Lists every playlist and folder with its parent, position and track count.";
    const EFFECT: Effect = Effect::Read;
    type Input = NoInput;
    type Output = ListPlaylistsOutput;

    fn run(ctx: &mut Context, _: NoInput) -> Result<ListPlaylistsOutput> {
        Ok(ListPlaylistsOutput {
            playlists: ctx.library.playlists()?,
        })
    }
}

pub struct SearchTracks;

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchTracksInput {
    /// Words to match against title, artist, album, genre, label, remixer and
    /// comment. Each word matches as a prefix; all words must match.
    pub text: Option<String>,
    pub bpm_min: Option<f64>,
    pub bpm_max: Option<f64>,
    /// Only tracks in exactly this key.
    pub key: Option<MusicalKey>,
    /// Only tracks whose key mixes harmonically with this one: the same key,
    /// ±1 on the Camelot wheel, or its relative major/minor.
    pub compatible_with_key: Option<MusicalKey>,
    /// Exact genre name (case-insensitive).
    pub genre: Option<String>,
    /// Only tracks in this playlist.
    pub playlist_id: Option<PlaylistId>,
    /// Maximum number of results (default 50, at most 1000).
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchTracksOutput {
    pub tracks: Vec<Track>,
    /// True if more tracks matched than `limit`.
    pub truncated: bool,
}

impl Command for SearchTracks {
    const NAME: &'static str = "search_tracks";
    const DESCRIPTION: &'static str = "Finds tracks by text, BPM range, key, harmonic compatibility, genre or playlist. Text results are ranked by relevance; others are sorted by artist and title.";
    const EFFECT: Effect = Effect::Read;
    type Input = SearchTracksInput;
    type Output = SearchTracksOutput;

    fn run(ctx: &mut Context, input: SearchTracksInput) -> Result<SearchTracksOutput> {
        let limit = input.limit.unwrap_or(50).clamp(1, 1000);
        let mut tracks = ctx.library.search(&SearchQuery {
            text: input.text,
            bpm_min: input.bpm_min,
            bpm_max: input.bpm_max,
            key: input.key,
            compatible_with: input.compatible_with_key,
            genre: input.genre,
            playlist: input.playlist_id,
            limit: limit + 1,
        })?;
        let truncated = tracks.len() > limit as usize;
        tracks.truncate(limit as usize);
        Ok(SearchTracksOutput { tracks, truncated })
    }
}

pub struct GetTrack;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetTrackInput {
    pub id: TrackId,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetTrackOutput {
    pub track: Track,
    /// Memory cues first, then hot cues A–H.
    pub cues: Vec<Cue>,
    pub beat_grid: Option<GridSummary>,
}

/// The shape of a beat grid without every beat.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GridSummary {
    pub beats: u32,
    /// Microseconds from the start of the track.
    pub first_beat_us: i64,
    /// Position in the bar of the first beat, 1–4.
    pub first_beat_bar_position: u8,
    /// The tempo, if it never changes.
    pub constant_bpm: Option<f64>,
}

impl Command for GetTrack {
    const NAME: &'static str = "get_track";
    const DESCRIPTION: &'static str =
        "Full details of one track: metadata, cues and a summary of its beat grid.";
    const EFFECT: Effect = Effect::Read;
    type Input = GetTrackInput;
    type Output = GetTrackOutput;

    fn run(ctx: &mut Context, input: GetTrackInput) -> Result<GetTrackOutput> {
        let track = ctx
            .library
            .track(input.id)?
            .ok_or_else(|| Error::NotFound(format!("track {}", input.id)))?;
        let beat_grid = ctx.library.beat_grid(input.id)?.and_then(|grid| {
            let first = *grid.beats.first()?;
            Some(GridSummary {
                beats: grid.beats.len() as u32,
                first_beat_us: first.time_us,
                first_beat_bar_position: first.bar_position,
                constant_bpm: grid.constant_bpm(),
            })
        });
        Ok(GetTrackOutput {
            cues: ctx.library.cues(input.id)?,
            beat_grid,
            track,
        })
    }
}
