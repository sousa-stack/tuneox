//! Import a rekordbox 6/7 library.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tuneox_core::{BeatGrid, PlaylistKind, TrackId};
use tuneox_pioneer::anlz::AnlzFile;
use tuneox_pioneer::masterdb::{self, MasterDb, RbPlaylist};

use crate::{Command, Context, Effect, Error, Result};

pub struct ImportRekordbox;

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportRekordboxInput {
    /// rekordbox's data folder, holding `master.db` and `share/`. Defaults to
    /// `~/Library/Pioneer/rekordbox` on macOS and `%APPDATA%\Pioneer\rekordbox`
    /// on Windows.
    pub rekordbox_dir: Option<String>,
    /// Read the library from this `master.db` instead (for example a backup).
    /// Analysis files are still read from `rekordbox_dir/share`.
    pub master_db: Option<String>,
    /// Report what would be imported without changing the TuneOx library.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ImportCounts {
    pub found: u64,
    pub created: u64,
    pub updated: u64,
}

#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct ImportReport {
    pub dry_run: bool,
    /// rekordbox's ID for the imported library (`djmdProperty.DBID`).
    pub rekordbox_library_id: String,
    pub tracks: ImportCounts,
    /// Playlists, folders and smart playlists.
    pub playlists: ImportCounts,
    /// Smart playlists are imported with their rules, but TuneOx doesn't
    /// evaluate rekordbox's rules yet, so they're empty for now.
    pub smart_playlists: u64,
    pub cues: u64,
    /// rekordbox cue kinds TuneOx doesn't map to a hot cue slot, with counts.
    /// They import as memory cues/loops and keep their raw kind.
    pub cues_with_unmapped_kind: BTreeMap<String, u64>,
    pub beat_grids: u64,
    /// Tracks whose audio file wasn't found on disk. They're still imported.
    pub missing_audio_files: u64,
    /// Tracks whose analysis file wasn't found, so no beat grid was imported.
    pub missing_analysis_files: u64,
    pub unreadable_analysis_files: u64,
    /// rekordbox tracks that point at the same audio file as another one and
    /// were merged into a single TuneOx track.
    pub merged_duplicates: u64,
    /// Playlist entries pointing at tracks that no longer exist.
    pub skipped_playlist_entries: u64,
    pub elapsed_ms: u64,
    /// The first few problems worth a human look.
    pub warnings: Vec<String>,
}

const MAX_WARNINGS: usize = 25;

impl ImportReport {
    fn warn(&mut self, message: String) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(message);
        }
    }
}

impl Command for ImportRekordbox {
    const NAME: &'static str = "import_rekordbox";
    const DESCRIPTION: &'static str = "Imports tracks, playlists, cues and beat grids from a rekordbox 6/7 library. rekordbox's files are only read, never changed. Re-running it updates previously imported tracks instead of duplicating them; rekordbox's data wins over edits made in TuneOx. Use dry_run to preview.";
    const EFFECT: Effect = Effect::Edit;
    type Input = ImportRekordboxInput;
    type Output = ImportReport;

    fn run(ctx: &mut Context, input: ImportRekordboxInput) -> Result<ImportReport> {
        let rekordbox_dir = match input.rekordbox_dir {
            Some(dir) => PathBuf::from(dir),
            None => default_rekordbox_dir().ok_or_else(|| {
                Error::InvalidInput(
                    "no default rekordbox folder on this OS; pass rekordbox_dir".into(),
                )
            })?,
        };
        let master_path = input
            .master_db
            .map(PathBuf::from)
            .unwrap_or_else(|| rekordbox_dir.join("master.db"));
        if !master_path.is_file() {
            return Err(Error::NotFound(master_path.display().to_string()));
        }
        // Work on a copy so a running rekordbox is never locked out.
        let copy = tempfile::NamedTempFile::new()?;
        std::fs::copy(&master_path, copy.path())?;
        let master = MasterDb::open(copy.path())?;
        import(ctx, &master, &rekordbox_dir.join("share"), input.dry_run)
    }
}

/// The import itself, separate from file discovery so tests can supply their
/// own `master.db`.
pub(crate) fn import(
    ctx: &mut Context,
    master: &MasterDb,
    share_dir: &Path,
    dry_run: bool,
) -> Result<ImportReport> {
    let started = Instant::now();
    ctx.progress("reading rekordbox library", 0, 1);
    let rb = master.read_library()?;
    let mut report = ImportReport {
        dry_run,
        rekordbox_library_id: rb.library_id.clone(),
        ..ImportReport::default()
    };

    // Analysis files: beat grids, and where each track's files live.
    let total = rb.tracks.len() as u64;
    let mut analysis: Vec<(Option<String>, Option<BeatGrid>)> = Vec::with_capacity(rb.tracks.len());
    for (i, track) in rb.tracks.iter().enumerate() {
        if i % 500 == 0 {
            ctx.progress("reading analysis files", i as u64, total);
        }
        if !Path::new(&track.info.path).exists() {
            report.missing_audio_files += 1;
        }
        let Some(relative) = &track.analysis_path else {
            analysis.push((None, None));
            continue;
        };
        let file = share_dir.join(relative.trim_start_matches(['/', '\\']));
        let dir = file.parent().map(|d| d.display().to_string());
        match std::fs::read(&file) {
            Err(_) => {
                report.missing_analysis_files += 1;
                analysis.push((None, None));
            }
            Ok(bytes) => match AnlzFile::parse(bytes).and_then(|a| a.beat_grid()) {
                Ok(grid) => analysis.push((dir, grid.filter(|g| !g.beats.is_empty()))),
                Err(e) => {
                    report.unreadable_analysis_files += 1;
                    report.warn(format!("{}: {e}", file.display()));
                    analysis.push((dir, None));
                }
            },
        }
    }

    let mut cues_by_track: HashMap<&str, Vec<_>> = HashMap::new();
    for rb_cue in &rb.cues {
        if !masterdb::is_known_cue_kind(rb_cue.kind) {
            *report
                .cues_with_unmapped_kind
                .entry(rb_cue.kind.to_string())
                .or_default() += 1;
        }
        cues_by_track
            .entry(rb_cue.track_id.as_str())
            .or_default()
            .push((rb_cue.cue.clone(), Some(rb_cue.kind)));
    }

    // Split the context so progress can be reported while the import holds
    // the library.
    let Context {
        library, progress, ..
    } = ctx;
    let mut progress =
        |stage: &str, done: u64, total: u64| progress(&crate::Progress { stage, done, total });
    let import = library.import("rekordbox", &rb.library_id)?;
    let mut track_ids: HashMap<&str, TrackId> = HashMap::with_capacity(rb.tracks.len());
    let mut seen: HashMap<&str, TrackId> = HashMap::with_capacity(rb.tracks.len());
    report.tracks.found = total;
    for (i, (track, (dir, grid))) in rb.tracks.iter().zip(&analysis).enumerate() {
        if i % 500 == 0 {
            progress("writing tracks", i as u64, total);
        }
        // The first rekordbox entry for a file wins; later ones are linked to
        // it so re-imports and playlists still find it.
        if let Some(&existing) = seen.get(track.info.path.as_str()) {
            import.link_source(&track.id, existing)?;
            track_ids.insert(&track.id, existing);
            report.merged_duplicates += 1;
            report.warn(format!(
                "rekordbox track {} points at the same file as another track; kept the first one: {}",
                track.id, track.info.path
            ));
            continue;
        }
        let upserted = import.upsert_track(&track.id, &track.info, dir.as_deref())?;
        if upserted.created {
            report.tracks.created += 1;
        } else {
            report.tracks.updated += 1;
        }
        seen.insert(&track.info.path, upserted.id);
        track_ids.insert(&track.id, upserted.id);

        let cues = cues_by_track.remove(track.id.as_str()).unwrap_or_default();
        report.cues += cues.len() as u64;
        import.replace_cues(upserted.id, &cues)?;
        if let Some(grid) = grid {
            import.set_beat_grid(upserted.id, grid)?;
            report.beat_grids += 1;
        }
    }

    // Playlists, parents before children, with positions renumbered from 0.
    report.playlists.found = rb.playlists.len() as u64;
    let known: HashSet<&str> = rb.playlists.iter().map(|p| p.id.as_str()).collect();
    let mut children: BTreeMap<Option<&str>, Vec<&RbPlaylist>> = BTreeMap::new();
    for playlist in &rb.playlists {
        let parent = playlist.parent_id.as_deref().filter(|p| known.contains(p));
        if playlist.parent_id.is_some() && parent.is_none() {
            report.warn(format!(
                "playlist \"{}\" has a missing parent; imported at the top level",
                playlist.name
            ));
        }
        children.entry(parent).or_default().push(playlist);
    }
    for siblings in children.values_mut() {
        siblings.sort_by(|a, b| a.seq.cmp(&b.seq).then_with(|| a.name.cmp(&b.name)));
    }
    let mut queue: Vec<(Option<&str>, Option<i64>)> = vec![(None, None)];
    while let Some((rb_parent, parent)) = queue.pop() {
        for (position, playlist) in children.get(&rb_parent).into_iter().flatten().enumerate() {
            let upserted = import.upsert_playlist(
                &playlist.id,
                parent,
                &playlist.name,
                playlist.kind,
                position as u32,
                playlist.smart_rules.as_deref(),
            )?;
            if upserted.created {
                report.playlists.created += 1;
            } else {
                report.playlists.updated += 1;
            }
            match playlist.kind {
                PlaylistKind::Smart => report.smart_playlists += 1,
                PlaylistKind::Folder => {}
                PlaylistKind::Playlist => {
                    if playlist.attribute != 0 {
                        report.warn(format!(
                            "playlist \"{}\" has unknown rekordbox attribute {}; imported as a playlist",
                            playlist.name, playlist.attribute
                        ));
                    }
                }
            }
            let mut entries = Vec::with_capacity(playlist.track_ids.len());
            for rb_track in &playlist.track_ids {
                match track_ids.get(rb_track.as_str()) {
                    Some(&id) => entries.push(id),
                    None => report.skipped_playlist_entries += 1,
                }
            }
            import.replace_playlist_tracks(upserted.id, &entries)?;
            queue.push((Some(playlist.id.as_str()), Some(upserted.id)));
        }
    }

    for (kind, count) in &report.cues_with_unmapped_kind.clone() {
        report.warn(format!(
            "{count} cues use rekordbox cue kind {kind}, which has no hot cue slot; imported as memory cues/loops"
        ));
    }

    if dry_run {
        drop(import);
    } else {
        progress("saving", 0, 1);
        import.commit()?;
    }
    report.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(report)
}

fn default_rekordbox_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        dirs::home_dir().map(|home| home.join("Library/Pioneer/rekordbox"))
    } else if cfg!(windows) {
        dirs::config_dir().map(|appdata| appdata.join("Pioneer").join("rekordbox"))
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
