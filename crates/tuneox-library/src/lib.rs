//! The TuneOx library: tracks, cues, beat grids and playlists in SQLite.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, ToSql, Transaction, params};
use tuneox_core::{
    BeatGrid, Cue, FileType, MusicalKey, Playlist, PlaylistId, PlaylistKind, Track, TrackColor,
    TrackId, TrackInfo,
};

const SCHEMA: &str = include_str!("schema.sql");
const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("library was created by a newer TuneOx (schema {0}); update TuneOx to open it")]
    NewerSchema(i64),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct Library {
    conn: Connection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub tracks: u64,
    pub playlists: u64,
    pub folders: u64,
    pub cues: u64,
    pub beat_grids: u64,
}

/// Filters for [`Library::search`]. Everything is optional and combined with AND.
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    /// Words matched against title, artist, album, genre, label, remixer and
    /// comment, as prefixes ("ben ben" finds "Benny Benassi").
    pub text: Option<String>,
    pub bpm_min: Option<f64>,
    pub bpm_max: Option<f64>,
    pub key: Option<MusicalKey>,
    /// Only keys that mix harmonically with this one.
    pub compatible_with: Option<MusicalKey>,
    pub genre: Option<String>,
    pub playlist: Option<PlaylistId>,
    pub limit: u32,
}

/// Result of inserting or updating a row during an import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Upserted {
    pub id: i64,
    pub created: bool,
}

impl Library {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(dir) = path.as_ref().parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::init(Connection::open(path)?)
    }

    /// Where the library lives unless told otherwise: `$TUNEOX_LIBRARY`, else
    /// `TuneOx/library.db` in the OS data folder
    /// (`~/Library/Application Support` on macOS).
    pub fn default_path() -> Option<PathBuf> {
        std::env::var_os("TUNEOX_LIBRARY")
            .map(PathBuf::from)
            .or_else(|| dirs::data_dir().map(|d| d.join("TuneOx").join("library.db")))
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        match version {
            0 => {
                let tx = conn.transaction()?;
                tx.execute_batch(SCHEMA)?;
                tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
                tx.commit()?;
            }
            SCHEMA_VERSION => {}
            newer => return Err(Error::NewerSchema(newer)),
        }
        Ok(Self { conn })
    }

    /// Starts an import from `source` (e.g. `rekordbox`) library
    /// `source_library`. Nothing is written until [`Import::commit`];
    /// dropping the import rolls it back, which is how dry runs work.
    pub fn import(&mut self, source: &str, source_library: &str) -> Result<Import<'_>> {
        Ok(Import {
            tx: self.conn.transaction()?,
            source: source.to_string(),
            source_library: source_library.to_string(),
            now: now_ms(),
        })
    }

    pub fn summary(&self) -> Result<Summary> {
        Ok(self.conn.query_row(
            "SELECT (SELECT count(*) FROM track),
                    (SELECT count(*) FROM playlist WHERE kind != 'folder'),
                    (SELECT count(*) FROM playlist WHERE kind = 'folder'),
                    (SELECT count(*) FROM cue),
                    (SELECT count(*) FROM beat_grid)",
            [],
            |r| {
                Ok(Summary {
                    tracks: r.get(0)?,
                    playlists: r.get(1)?,
                    folders: r.get(2)?,
                    cues: r.get(3)?,
                    beat_grids: r.get(4)?,
                })
            },
        )?)
    }

    /// The whole playlist tree, parents before children, siblings in order.
    pub fn playlists(&self) -> Result<Vec<Playlist>> {
        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE tree (id, sort) AS (
                 SELECT id, printf('%08d', position) FROM playlist WHERE parent_id IS NULL
                 UNION ALL
                 SELECT p.id, tree.sort || '/' || printf('%08d', p.position)
                 FROM playlist p JOIN tree ON p.parent_id = tree.id
             )
             SELECT p.id, p.parent_id, p.name, p.kind, p.position,
                    (SELECT count(*) FROM playlist_track pt WHERE pt.playlist_id = p.id)
             FROM playlist p JOIN tree ON tree.id = p.id
             ORDER BY tree.sort",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Playlist {
                id: r.get(0)?,
                parent_id: r.get(1)?,
                name: r.get(2)?,
                kind: PlaylistKind::parse(&r.get::<_, String>(3)?)
                    .unwrap_or(PlaylistKind::Playlist),
                position: r.get(4)?,
                track_count: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn track(&self, id: TrackId) -> Result<Option<Track>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {TRACK_COLUMNS} FROM track WHERE id = ?1"),
                [id],
                track_from_row,
            )
            .optional()?)
    }

    pub fn cues(&self, track: TrackId) -> Result<Vec<Cue>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT hot_cue, start_us, end_us, color, comment FROM cue
             WHERE track_id = ?1 ORDER BY hot_cue IS NOT NULL, hot_cue, start_us",
        )?;
        let rows = stmt.query_map([track], |r| {
            Ok(Cue {
                hot_cue: r.get(0)?,
                start_us: r.get(1)?,
                end_us: r.get(2)?,
                color: r.get(3)?,
                comment: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn beat_grid(&self, track: TrackId) -> Result<Option<BeatGrid>> {
        let bytes: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT beats FROM beat_grid WHERE track_id = ?1",
                [track],
                |r| r.get(0),
            )
            .optional()?;
        Ok(bytes.and_then(|b| BeatGrid::from_bytes(&b)))
    }

    pub fn search(&self, query: &SearchQuery) -> Result<Vec<Track>> {
        let mut from = "track".to_string();
        let mut conditions: Vec<String> = Vec::new();
        let mut args: Vec<Box<dyn ToSql>> = Vec::new();
        let mut order = "track.artist COLLATE NOCASE, track.title COLLATE NOCASE";

        if let Some(fts) = query.text.as_deref().and_then(fts_query) {
            from = "track JOIN track_search ON track_search.rowid = track.id".to_string();
            conditions.push("track_search MATCH ?".to_string());
            args.push(Box::new(fts));
            order = "bm25(track_search)";
        }
        if let Some(min) = query.bpm_min {
            conditions.push("track.bpm >= ?".to_string());
            args.push(Box::new(min));
        }
        if let Some(max) = query.bpm_max {
            conditions.push("track.bpm <= ?".to_string());
            args.push(Box::new(max));
        }
        if let Some(key) = query.key {
            conditions.push("track.musical_key = ?".to_string());
            args.push(Box::new(key.to_string()));
        }
        if let Some(key) = query.compatible_with {
            conditions.push("track.musical_key IN (?, ?, ?, ?)".to_string());
            for k in key.compatible_keys() {
                args.push(Box::new(k.to_string()));
            }
        }
        if let Some(genre) = &query.genre {
            conditions.push("track.genre = ? COLLATE NOCASE".to_string());
            args.push(Box::new(genre.clone()));
        }
        if let Some(playlist) = query.playlist {
            conditions.push(
                "track.id IN (SELECT track_id FROM playlist_track WHERE playlist_id = ?)"
                    .to_string(),
            );
            args.push(Box::new(playlist));
        }
        let filter = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };
        let columns = TRACK_COLUMNS
            .split(", ")
            .map(|c| format!("track.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        args.push(Box::new(query.limit.max(1)));
        let sql = format!("SELECT {columns} FROM {from} {filter} ORDER BY {order} LIMIT ?");
        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<&dyn ToSql> = args.iter().map(|a| a.as_ref()).collect();
        let rows = stmt.query_map(params.as_slice(), track_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn playlist(&self, id: PlaylistId) -> Result<Option<Playlist>> {
        Ok(self
            .playlists()?
            .into_iter()
            .find(|playlist| playlist.id == id))
    }

    /// Tracks in ID order, a page at a time.
    pub fn tracks(&self, offset: u32, limit: u32) -> Result<Vec<Track>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {TRACK_COLUMNS} FROM track ORDER BY id LIMIT ?1 OFFSET ?2"
        ))?;
        let rows = stmt.query_map([limit, offset], track_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Track IDs of a playlist, in order.
    pub fn playlist_tracks(&self, playlist: PlaylistId) -> Result<Vec<TrackId>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT track_id FROM playlist_track WHERE playlist_id = ?1 ORDER BY position",
        )?;
        let rows = stmt.query_map([playlist], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Folder holding a track's analysis files, if it has any.
    pub fn analysis_dir(&self, track: TrackId) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT analysis_dir FROM track WHERE id = ?1",
                [track],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }
}

/// An import in progress. See [`Library::import`].
pub struct Import<'a> {
    tx: Transaction<'a>,
    source: String,
    source_library: String,
    now: i64,
}

impl Import<'_> {
    /// Inserts or updates a track by its source ID. A track whose file path
    /// is already in the library is merged with it rather than duplicated.
    pub fn upsert_track(
        &self,
        source_id: &str,
        info: &TrackInfo,
        analysis_dir: Option<&str>,
    ) -> Result<Upserted> {
        let by_source: Option<TrackId> = self
            .tx
            .prepare_cached(
                "SELECT track_id FROM track_source
                 WHERE source = ?1 AND source_library = ?2 AND source_id = ?3",
            )?
            .query_row(params![self.source, self.source_library, source_id], |r| {
                r.get(0)
            })
            .optional()?;
        let by_path: Option<TrackId> = self
            .tx
            .prepare_cached("SELECT id FROM track WHERE path = ?1")?
            .query_row([&info.path], |r| r.get(0))
            .optional()?;

        let upserted = match by_path.or(by_source) {
            Some(id) => {
                self.tx
                    .prepare_cached(
                        "UPDATE track SET path = ?2, title = ?3, artist = ?4, album = ?5,
                            genre = ?6, label = ?7, remixer = ?8, comment = ?9,
                            musical_key = ?10, bpm = ?11, duration_secs = ?12, rating = ?13,
                            color = ?14, file_type = ?15, sample_rate = ?16, bit_depth = ?17,
                            bitrate = ?18, file_size = ?19, play_count = ?20, year = ?21,
                            track_number = ?22, date_added = ?23,
                            analysis_dir = coalesce(?24, analysis_dir), updated_at = ?25
                         WHERE id = ?1",
                    )?
                    .execute(params![
                        id,
                        info.path,
                        info.title,
                        info.artist,
                        info.album,
                        info.genre,
                        info.label,
                        info.remixer,
                        info.comment,
                        info.key.map(|k| k.to_string()),
                        info.bpm,
                        info.duration_secs,
                        info.rating,
                        info.color.map(TrackColor::as_str),
                        info.file_type.map(FileType::as_str),
                        info.sample_rate,
                        info.bit_depth,
                        info.bitrate,
                        info.file_size,
                        info.play_count,
                        info.year,
                        info.track_number,
                        info.date_added,
                        analysis_dir,
                        self.now,
                    ])?;
                Upserted { id, created: false }
            }
            None => {
                self.tx
                    .prepare_cached(
                        "INSERT INTO track (uuid, path, title, artist, album, genre, label,
                            remixer, comment, musical_key, bpm, duration_secs, rating, color,
                            file_type, sample_rate, bit_depth, bitrate, file_size, play_count,
                            year, track_number, date_added, analysis_dir, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                            ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?25)",
                    )?
                    .execute(params![
                        uuid::Uuid::now_v7().to_string(),
                        info.path,
                        info.title,
                        info.artist,
                        info.album,
                        info.genre,
                        info.label,
                        info.remixer,
                        info.comment,
                        info.key.map(|k| k.to_string()),
                        info.bpm,
                        info.duration_secs,
                        info.rating,
                        info.color.map(TrackColor::as_str),
                        info.file_type.map(FileType::as_str),
                        info.sample_rate,
                        info.bit_depth,
                        info.bitrate,
                        info.file_size,
                        info.play_count,
                        info.year,
                        info.track_number,
                        info.date_added,
                        analysis_dir,
                        self.now,
                    ])?;
                Upserted {
                    id: self.tx.last_insert_rowid(),
                    created: true,
                }
            }
        };
        self.tx
            .prepare_cached(
                "INSERT OR REPLACE INTO track_source (source, source_library, source_id, track_id)
                 VALUES (?1, ?2, ?3, ?4)",
            )?
            .execute(params![
                self.source,
                self.source_library,
                source_id,
                upserted.id
            ])?;
        Ok(upserted)
    }

    /// Records that `source_id` refers to an existing track, without
    /// changing the track.
    pub fn link_source(&self, source_id: &str, track: TrackId) -> Result<()> {
        self.tx
            .prepare_cached(
                "INSERT OR REPLACE INTO track_source (source, source_library, source_id, track_id)
                 VALUES (?1, ?2, ?3, ?4)",
            )?
            .execute(params![self.source, self.source_library, source_id, track])?;
        Ok(())
    }

    /// Replaces all of a track's cues. `source_kind` is the raw cue type from
    /// the import source, kept so mappings can be corrected later.
    pub fn replace_cues(&self, track: TrackId, cues: &[(Cue, Option<i64>)]) -> Result<()> {
        self.tx
            .prepare_cached("DELETE FROM cue WHERE track_id = ?1")?
            .execute([track])?;
        let mut insert = self.tx.prepare_cached(
            "INSERT INTO cue (track_id, hot_cue, start_us, end_us, color, comment, source_kind)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for (cue, source_kind) in cues {
            insert.execute(params![
                track,
                cue.hot_cue,
                cue.start_us,
                cue.end_us,
                cue.color,
                cue.comment,
                source_kind
            ])?;
        }
        Ok(())
    }

    pub fn set_beat_grid(&self, track: TrackId, grid: &BeatGrid) -> Result<()> {
        self.tx
            .prepare_cached("INSERT OR REPLACE INTO beat_grid (track_id, beats) VALUES (?1, ?2)")?
            .execute(params![track, grid.to_bytes()])?;
        Ok(())
    }

    pub fn upsert_playlist(
        &self,
        source_id: &str,
        parent: Option<PlaylistId>,
        name: &str,
        kind: PlaylistKind,
        position: u32,
        smart_rules: Option<&str>,
    ) -> Result<Upserted> {
        let existing: Option<PlaylistId> = self
            .tx
            .prepare_cached(
                "SELECT id FROM playlist
                 WHERE source = ?1 AND source_library = ?2 AND source_id = ?3",
            )?
            .query_row(params![self.source, self.source_library, source_id], |r| {
                r.get(0)
            })
            .optional()?;
        match existing {
            Some(id) => {
                self.tx
                    .prepare_cached(
                        "UPDATE playlist SET parent_id = ?2, name = ?3, kind = ?4,
                            position = ?5, smart_rules = ?6
                         WHERE id = ?1",
                    )?
                    .execute(params![
                        id,
                        parent,
                        name,
                        kind.as_str(),
                        position,
                        smart_rules
                    ])?;
                Ok(Upserted { id, created: false })
            }
            None => {
                self.tx
                    .prepare_cached(
                        "INSERT INTO playlist (uuid, parent_id, position, name, kind, smart_rules,
                            source, source_library, source_id)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    )?
                    .execute(params![
                        uuid::Uuid::now_v7().to_string(),
                        parent,
                        position,
                        name,
                        kind.as_str(),
                        smart_rules,
                        self.source,
                        self.source_library,
                        source_id
                    ])?;
                Ok(Upserted {
                    id: self.tx.last_insert_rowid(),
                    created: true,
                })
            }
        }
    }

    pub fn replace_playlist_tracks(&self, playlist: PlaylistId, tracks: &[TrackId]) -> Result<()> {
        self.tx
            .prepare_cached("DELETE FROM playlist_track WHERE playlist_id = ?1")?
            .execute([playlist])?;
        let mut insert = self.tx.prepare_cached(
            "INSERT INTO playlist_track (playlist_id, position, track_id) VALUES (?1, ?2, ?3)",
        )?;
        for (position, track) in tracks.iter().enumerate() {
            insert.execute(params![playlist, position as i64, track])?;
        }
        Ok(())
    }

    pub fn commit(self) -> Result<()> {
        self.tx.commit()?;
        Ok(())
    }
}

const TRACK_COLUMNS: &str = "id, path, title, artist, album, genre, label, remixer, comment, \
    musical_key, bpm, duration_secs, rating, color, file_type, sample_rate, bit_depth, bitrate, \
    file_size, play_count, year, track_number, date_added";

fn track_from_row(r: &Row) -> rusqlite::Result<Track> {
    Ok(Track {
        id: r.get(0)?,
        info: TrackInfo {
            path: r.get(1)?,
            title: r.get(2)?,
            artist: r.get(3)?,
            album: r.get(4)?,
            genre: r.get(5)?,
            label: r.get(6)?,
            remixer: r.get(7)?,
            comment: r.get(8)?,
            key: r
                .get::<_, Option<String>>(9)?
                .and_then(|k| MusicalKey::parse(&k)),
            bpm: r.get(10)?,
            duration_secs: r.get(11)?,
            rating: r.get(12)?,
            color: r
                .get::<_, Option<String>>(13)?
                .and_then(|c| TrackColor::parse(&c)),
            file_type: r
                .get::<_, Option<String>>(14)?
                .and_then(|t| FileType::parse(&t)),
            sample_rate: r.get(15)?,
            bit_depth: r.get(16)?,
            bitrate: r.get(17)?,
            file_size: r.get(18)?,
            play_count: r.get(19)?,
            year: r.get(20)?,
            track_number: r.get(21)?,
            date_added: r.get(22)?,
        },
    })
}

/// Turns free text into an FTS5 query: every word must match as a prefix.
fn fts_query(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split_whitespace()
        .map(|word| word.replace('"', ""))
        .filter(|word| !word.is_empty())
        .map(|word| format!("\"{word}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
