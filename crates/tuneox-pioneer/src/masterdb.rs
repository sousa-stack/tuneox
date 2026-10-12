//! The local rekordbox 6/7 library database, `master.db`.
//!
//! `master.db` is a SQLCipher database. TuneOx only imports from it, so it is
//! always opened read-only. Field meanings are documented in
//! `research/rekordbox-masterdb.md`.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, Row};
use tuneox_core::{Cue, FileType, MusicalKey, PlaylistKind, TrackColor, TrackInfo};

use crate::keys::{self, MissingKey};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    MissingKey(#[from] MissingKey),
    #[error("could not unlock master.db: wrong key, or not a rekordbox database")]
    Locked,
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct MasterDb {
    conn: Connection,
}

/// Everything TuneOx imports from a rekordbox library.
#[derive(Debug, Default)]
pub struct RekordboxLibrary {
    /// `djmdProperty.DBID`, which identifies this rekordbox library. Exports
    /// carry it as `masterDbId`.
    pub library_id: String,
    pub tracks: Vec<RbTrack>,
    pub playlists: Vec<RbPlaylist>,
    pub cues: Vec<RbCue>,
}

#[derive(Debug, Clone)]
pub struct RbTrack {
    /// `djmdContent.ID`.
    pub id: String,
    pub info: TrackInfo,
    /// Path of the track's `ANLZ0000.DAT`, relative to rekordbox's `share`
    /// folder (e.g. `/PIONEER/USBANLZ/a6f/…/ANLZ0000.DAT`).
    pub analysis_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RbPlaylist {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub kind: PlaylistKind,
    /// The raw `Attribute`, kept so unknown values can be reported.
    pub attribute: i64,
    /// Order among siblings.
    pub seq: i64,
    /// rekordbox's smart playlist rules (XML), for smart playlists.
    pub smart_rules: Option<String>,
    /// Track IDs in playlist order.
    pub track_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RbCue {
    pub track_id: String,
    pub cue: Cue,
    /// The raw `djmdCue.Kind`.
    pub kind: i64,
}

impl MasterDb {
    /// Opens `master.db` with the built-in key.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_key(path, keys::masterdb()?)
    }

    pub fn open_with_key(path: impl AsRef<Path>, key: &str) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.pragma_update(None, "key", key)?;
        // SQLCipher doesn't check the key until the first read.
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
            .map_err(|_| Error::Locked)?;
        Ok(Self { conn })
    }

    /// Reads tracks, playlists and cues, skipping soft-deleted rows.
    pub fn read_library(&self) -> Result<RekordboxLibrary> {
        Ok(RekordboxLibrary {
            library_id: self
                .conn
                .query_row("SELECT DBID FROM djmdProperty LIMIT 1", [], |r| {
                    Ok(text(r, 0))
                })?
                .unwrap_or_default(),
            tracks: self.tracks()?,
            playlists: self.playlists()?,
            cues: self.cues()?,
        })
    }

    fn tracks(&self) -> Result<Vec<RbTrack>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.ID, c.FolderPath, c.Title, artist.Name, album.Name, genre.Name,
                    label.Name, remixer.Name, c.Commnt, key.ScaleName, c.BPM, c.Length,
                    c.Rating, c.ColorID, c.FileType, c.SampleRate, c.BitDepth, c.BitRate,
                    c.FileSize, c.DJPlayCount, c.ReleaseYear, c.TrackNo, c.StockDate,
                    c.AnalysisDataPath,
                    (SELECT GainHigh FROM djmdMixerParam
                     WHERE ContentID = c.ID AND rb_local_deleted = 0 LIMIT 1),
                    (SELECT GainLow FROM djmdMixerParam
                     WHERE ContentID = c.ID AND rb_local_deleted = 0 LIMIT 1)
             FROM djmdContent c
             LEFT JOIN djmdArtist artist ON artist.ID = c.ArtistID
             LEFT JOIN djmdAlbum album ON album.ID = c.AlbumID
             LEFT JOIN djmdGenre genre ON genre.ID = c.GenreID
             LEFT JOIN djmdLabel label ON label.ID = c.LabelID
             LEFT JOIN djmdArtist remixer ON remixer.ID = c.RemixerID
             LEFT JOIN djmdKey key ON key.ID = c.KeyID
             WHERE c.rb_local_deleted = 0",
        )?;
        let rows = stmt.query_map([], |r| {
            let path = text(r, 1).unwrap_or_default();
            let info = TrackInfo {
                title: text(r, 2).unwrap_or_default(),
                artist: text(r, 3),
                album: text(r, 4),
                genre: text(r, 5),
                label: text(r, 6),
                remixer: text(r, 7),
                comment: text(r, 8),
                key: text(r, 9).and_then(|k| MusicalKey::parse(&k)),
                bpm: int(r, 10).filter(|&b| b > 0).map(|b| b as f64 / 100.0),
                duration_secs: int(r, 11).filter(|&s| s > 0).map(|s| s as f64),
                rating: int(r, 12).unwrap_or(0).clamp(0, 5) as u8,
                color: int(r, 13).and_then(track_color),
                file_type: int(r, 14)
                    .and_then(file_type)
                    .or_else(|| FileType::from_extension(&path)),
                sample_rate: int(r, 15).filter(|&v| v > 0).map(|v| v as u32),
                bit_depth: int(r, 16).filter(|&v| v > 0).map(|v| v as u16),
                bitrate: int(r, 17).filter(|&v| v > 0).map(|v| v as u32),
                file_size: int(r, 18).filter(|&v| v > 0).map(|v| v as u64),
                play_count: int(r, 19).unwrap_or(0).max(0) as u32,
                year: int(r, 20).filter(|&v| v > 0).map(|v| v as u16),
                track_number: int(r, 21).filter(|&v| v > 0).map(|v| v as u32),
                date_added: text(r, 22),
                auto_gain_db: mixer_gain_db(int(r, 24), int(r, 25)),
                path,
            };
            Ok(RbTrack {
                id: text(r, 0).unwrap_or_default(),
                info,
                analysis_path: text(r, 23),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn playlists(&self) -> Result<Vec<RbPlaylist>> {
        let mut members: HashMap<String, Vec<String>> = HashMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT PlaylistID, ContentID FROM djmdSongPlaylist
             WHERE rb_local_deleted = 0
             ORDER BY PlaylistID, TrackNo",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            if let (Some(playlist), Some(track)) = (text(r, 0), text(r, 1)) {
                members.entry(playlist).or_default().push(track);
            }
        }

        let mut stmt = self.conn.prepare(
            "SELECT ID, ParentID, Name, Attribute, Seq, SmartList FROM djmdPlaylist
             WHERE rb_local_deleted = 0",
        )?;
        let rows = stmt.query_map([], |r| {
            let id = text(r, 0).unwrap_or_default();
            let attribute = int(r, 3).unwrap_or(0);
            Ok(RbPlaylist {
                track_ids: members.remove(&id).unwrap_or_default(),
                parent_id: text(r, 1).filter(|p| p != "root"),
                name: text(r, 2).unwrap_or_default(),
                kind: match attribute {
                    1 => PlaylistKind::Folder,
                    4 => PlaylistKind::Smart,
                    _ => PlaylistKind::Playlist,
                },
                attribute,
                seq: int(r, 4).unwrap_or(0),
                smart_rules: text(r, 5),
                id,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn cues(&self) -> Result<Vec<RbCue>> {
        let mut stmt = self.conn.prepare(
            "SELECT ContentID, Kind, InMsec, OutMsec, ColorTableIndex, Comment FROM djmdCue
             WHERE rb_local_deleted = 0
             ORDER BY ContentID, InMsec",
        )?;
        let rows = stmt.query_map([], |r| {
            let kind = int(r, 1).unwrap_or(0);
            Ok(RbCue {
                track_id: text(r, 0).unwrap_or_default(),
                cue: Cue {
                    hot_cue: hot_cue_slot(kind),
                    start_us: int(r, 2).unwrap_or(0) * 1000,
                    end_us: int(r, 3).filter(|&ms| ms > 0).map(|ms| ms * 1000),
                    color: int(r, 4).filter(|&c| c > 0).map(|c| c as u8),
                    comment: text(r, 5),
                },
                kind,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every table in the database with its row count, for schema research.
    pub fn table_row_counts(&self) -> Result<Vec<(String, i64)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")?;
        let names = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        names
            .into_iter()
            .map(|name| {
                let count = self.conn.query_row(
                    &format!("SELECT count(*) FROM \"{name}\""),
                    [],
                    |row| row.get(0),
                )?;
                Ok((name, count))
            })
            .collect()
    }
}

/// `djmdCue.Kind` to a hot cue slot (0 = A). 0 is a memory cue; hot cues
/// A–C are 1–3 and D–H are 5–9. Kind 4 is a loop without a hot cue slot,
/// which rekordbox exports as a memory loop (see
/// `research/rekordbox-masterdb.md`).
pub fn hot_cue_slot(kind: i64) -> Option<u8> {
    match kind {
        1..=3 => Some(kind as u8 - 1),
        5..=9 => Some(kind as u8 - 2),
        _ => None,
    }
}

/// Whether TuneOx knows what a `djmdCue.Kind` value means.
pub fn is_known_cue_kind(kind: i64) -> bool {
    (0..=9).contains(&kind)
}

/// `djmdMixerParam.GainHigh`/`GainLow` hold the two 16-bit halves of a
/// float32 linear gain (high half first). Returns it in dB.
fn mixer_gain_db(high: Option<i64>, low: Option<i64>) -> Option<f64> {
    let bits = ((high? as u32 & 0xffff) << 16) | (low? as u32 & 0xffff);
    let gain = f32::from_bits(bits) as f64;
    (gain.is_finite() && gain > 0.0).then(|| 20.0 * gain.log10())
}

fn track_color(id: i64) -> Option<TrackColor> {
    TrackColor::ALL
        .get(usize::try_from(id).ok()?.checked_sub(1)?)
        .copied()
}

fn file_type(code: i64) -> Option<FileType> {
    match code {
        1 => Some(FileType::Mp3),
        4 => Some(FileType::M4a),
        5 => Some(FileType::Flac),
        11 => Some(FileType::Wav),
        12 => Some(FileType::Aiff),
        _ => None,
    }
}

/// A column as non-empty text, whatever type rekordbox stored it as.
fn text(row: &Row, idx: usize) -> Option<String> {
    match row.get_ref(idx).ok()? {
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).trim().to_string()),
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        _ => None,
    }
    .filter(|s| !s.is_empty())
}

/// A column as an integer, whatever type rekordbox stored it as.
fn int(row: &Row, idx: usize) -> Option<i64> {
    match row.get_ref(idx).ok()? {
        ValueRef::Integer(i) => Some(i),
        ValueRef::Real(f) => Some(f as i64),
        ValueRef::Text(t) => std::str::from_utf8(t).ok()?.trim().parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_cue_kinds_to_slots() {
        let slots: Vec<_> = (0..=10).map(hot_cue_slot).collect();
        assert_eq!(
            slots,
            [
                None,
                Some(0),
                Some(1),
                Some(2),
                None,
                Some(3),
                Some(4),
                Some(5),
                Some(6),
                Some(7),
                None
            ]
        );
        assert!(is_known_cue_kind(4));
        assert!(is_known_cue_kind(9));
        assert!(!is_known_cue_kind(10));
    }

    #[test]
    fn decodes_mixer_gain() {
        // 0.5 as float32 is 0x3F000000.
        let db = mixer_gain_db(Some(0x3F00), Some(0)).unwrap();
        assert!((db - -6.0206).abs() < 1e-3, "{db}");
        // Bulletproof in the sample library: 0x3F58C124 ≈ 0.8467.
        let db = mixer_gain_db(Some(16216), Some(49444)).unwrap();
        assert!((db - -1.445).abs() < 1e-2, "{db}");
        assert_eq!(mixer_gain_db(Some(0), Some(0)), None);
        assert_eq!(mixer_gain_db(None, Some(1)), None);
    }

    #[test]
    fn maps_track_colors() {
        assert_eq!(track_color(0), None);
        assert_eq!(track_color(1), Some(TrackColor::Pink));
        assert_eq!(track_color(8), Some(TrackColor::Purple));
        assert_eq!(track_color(9), None);
    }
}
