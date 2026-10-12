//! End-to-end import from a synthetic rekordbox library: a SQLCipher
//! `master.db` with the rekordbox table and column names we read, encrypted
//! with a made-up key, plus a hand-built analysis file.

use std::path::Path;

use rusqlite::Connection;
use tuneox_core::{MusicalKey, PlaylistKind};
use tuneox_library::Library;
use tuneox_pioneer::masterdb::MasterDb;

use super::import;
use crate::Context;

const KEY: &str = "tuneox-test-key";

const SCHEMA: &str = "
CREATE TABLE djmdProperty (DBID VARCHAR);
CREATE TABLE djmdContent (ID VARCHAR, FolderPath VARCHAR, Title VARCHAR, ArtistID VARCHAR,
    AlbumID VARCHAR, GenreID VARCHAR, LabelID VARCHAR, RemixerID VARCHAR, Commnt VARCHAR,
    KeyID VARCHAR, BPM INTEGER, Length INTEGER, Rating INTEGER, ColorID INTEGER,
    FileType INTEGER, SampleRate INTEGER, BitDepth INTEGER, BitRate INTEGER, FileSize INTEGER,
    DJPlayCount VARCHAR, ReleaseYear INTEGER, TrackNo INTEGER, StockDate VARCHAR,
    AnalysisDataPath VARCHAR, rb_local_deleted INTEGER);
CREATE TABLE djmdArtist (ID VARCHAR, Name VARCHAR);
CREATE TABLE djmdAlbum (ID VARCHAR, Name VARCHAR);
CREATE TABLE djmdGenre (ID VARCHAR, Name VARCHAR);
CREATE TABLE djmdLabel (ID VARCHAR, Name VARCHAR);
CREATE TABLE djmdKey (ID VARCHAR, ScaleName VARCHAR);
CREATE TABLE djmdPlaylist (ID VARCHAR, ParentID VARCHAR, Name VARCHAR, Attribute INTEGER,
    Seq INTEGER, SmartList TEXT, rb_local_deleted INTEGER);
CREATE TABLE djmdSongPlaylist (PlaylistID VARCHAR, ContentID VARCHAR, TrackNo INTEGER,
    rb_local_deleted INTEGER);
CREATE TABLE djmdCue (ContentID VARCHAR, Kind INTEGER, InMsec INTEGER, OutMsec INTEGER,
    ColorTableIndex INTEGER, Comment VARCHAR, rb_local_deleted INTEGER);
";

/// Builds the fake rekordbox folder and returns the path of its master.db.
fn rekordbox_dir(root: &Path) -> std::path::PathBuf {
    let audio = root.join("music/bulletproof.aiff");
    std::fs::create_dir_all(audio.parent().unwrap()).unwrap();
    std::fs::write(&audio, b"not really audio").unwrap();
    let audio = audio.display().to_string();

    let anlz = root.join("share/PIONEER/USBANLZ/abc/def/ANLZ0000.DAT");
    std::fs::create_dir_all(anlz.parent().unwrap()).unwrap();
    std::fs::write(
        &anlz,
        anlz_with_grid(&[(1, 12600, 24), (2, 12600, 500), (3, 12600, 976)]),
    )
    .unwrap();

    let path = root.join("master.db");
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "key", KEY).unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    conn.execute_batch(&format!(
        "INSERT INTO djmdProperty VALUES ('2411086669');
         INSERT INTO djmdArtist VALUES ('a1', 'La Roux');
         INSERT INTO djmdGenre VALUES ('g1', 'Dance / Pop');
         INSERT INTO djmdKey VALUES ('k1', '2A'), ('k2', 'Amin'), ('k3', 'All');
         INSERT INTO djmdContent VALUES
           ('1', '{audio}', 'Bulletproof', 'a1', NULL, 'g1', NULL, NULL, NULL, 'k1', 12600, 306,
            4, 5, 12, 44100, 16, 1411, 54000000, '7', 2009, 1, '2026-10-09',
            '/PIONEER/USBANLZ/abc/def/ANLZ0000.DAT', 0),
           ('2', '/nowhere/missing.mp3', 'Missing', NULL, NULL, NULL, NULL, NULL, NULL, 'k2',
            12200, 247, 0, 0, 1, 44100, 0, 320, 0, '0', 0, 0, NULL, NULL, 0),
           ('3', '/nowhere/deleted.mp3', 'Deleted', NULL, NULL, NULL, NULL, NULL, NULL, NULL,
            0, 0, 0, 0, 1, 0, 0, 0, 0, '0', 0, 0, NULL, NULL, 1),
           ('4', '{audio}', 'Bulletproof (duplicate)', NULL, NULL, NULL, NULL, NULL, NULL, 'k3',
            0, 0, 0, 0, 12, 0, 0, 0, 0, '0', 0, 0, NULL, NULL, 0);
         INSERT INTO djmdPlaylist VALUES
           ('f', 'root', 'Gigs', 1, 1, NULL, 0),
           ('p', 'f', '10-9-26', 0, 1, NULL, 0),
           ('s', 'root', 'Recent', 4, 2, '<NODE/>', 0),
           ('x', 'root', 'Gone', 0, 3, NULL, 1);
         INSERT INTO djmdSongPlaylist VALUES
           ('p', '2', 1, 0), ('p', '1', 2, 0), ('p', '3', 3, 0), ('p', '4', 4, 0);
         INSERT INTO djmdCue VALUES
           ('1', 1, 24, -1, 0, NULL, 0),
           ('1', 5, 45739, -1, 3, 'Drop', 0),
           ('1', 4, 280000, 287619, 0, NULL, 0),
           ('1', 0, 16, -1, 0, NULL, 0),
           ('1', 2, 15263, -1, 0, NULL, 1),
           ('4', 1, 999, -1, 0, NULL, 0);"
    ))
    .unwrap();
    path
}

/// A minimal analysis file: PMAI header plus one PQTZ beat grid section.
fn anlz_with_grid(beats: &[(u16, u16, u32)]) -> Vec<u8> {
    let mut pqtz = b"PQTZ".to_vec();
    pqtz.extend(24u32.to_be_bytes());
    pqtz.extend((24 + 8 * beats.len() as u32).to_be_bytes());
    pqtz.extend(0u32.to_be_bytes());
    pqtz.extend(0x80000u32.to_be_bytes());
    pqtz.extend((beats.len() as u32).to_be_bytes());
    for &(bar, bpm, ms) in beats {
        pqtz.extend(bar.to_be_bytes());
        pqtz.extend(bpm.to_be_bytes());
        pqtz.extend(ms.to_be_bytes());
    }
    let mut file = b"PMAI".to_vec();
    file.extend(28u32.to_be_bytes());
    file.extend((28 + pqtz.len() as u32).to_be_bytes());
    file.extend([0; 16]);
    file.extend(pqtz);
    file
}

fn run(ctx: &mut Context, root: &Path, master: &Path, dry_run: bool) -> super::ImportReport {
    let master = MasterDb::open_with_key(master, KEY).unwrap();
    import(ctx, &master, &root.join("share"), dry_run).unwrap()
}

#[test]
fn imports_a_rekordbox_library() {
    let root = tempfile::tempdir().unwrap();
    let master = rekordbox_dir(root.path());
    let mut ctx = Context::new(Library::open_in_memory().unwrap(), None);

    let report = run(&mut ctx, root.path(), &master, false);
    assert_eq!(report.rekordbox_library_id, "2411086669");
    assert_eq!(report.tracks.found, 3, "soft-deleted tracks are skipped");
    assert_eq!(report.tracks.created, 2);
    assert_eq!(report.merged_duplicates, 1);
    assert_eq!(report.missing_audio_files, 1);
    assert_eq!(report.beat_grids, 1);
    assert_eq!(
        report.cues,
        3 + 1,
        "deleted cues skipped; the duplicate's cue dropped"
    );
    assert!(
        report.cues_with_unmapped_kind.is_empty(),
        "kind 4 is a known memory loop"
    );
    assert_eq!(report.playlists.found, 3);
    assert_eq!(report.smart_playlists, 1);
    assert_eq!(report.skipped_playlist_entries, 1);

    let lib = &ctx.library;
    let playlists = lib.playlists().unwrap();
    let names: Vec<(&str, PlaylistKind)> = playlists
        .iter()
        .map(|p| (p.name.as_str(), p.kind))
        .collect();
    assert_eq!(
        names,
        [
            ("Gigs", PlaylistKind::Folder),
            ("10-9-26", PlaylistKind::Playlist),
            ("Recent", PlaylistKind::Smart)
        ]
    );
    let titles: Vec<String> = lib
        .playlist_tracks(playlists[1].id)
        .unwrap()
        .into_iter()
        .map(|id| lib.track(id).unwrap().unwrap().info.title)
        .collect();
    assert_eq!(
        titles,
        ["Missing", "Bulletproof", "Bulletproof"],
        "the duplicate's entry points at the track it was merged into"
    );

    let bulletproof = lib.playlist_tracks(playlists[1].id).unwrap()[1];
    let track = lib.track(bulletproof).unwrap().unwrap().info;
    assert_eq!(
        track.title, "Bulletproof",
        "the first entry for a file wins"
    );
    assert_eq!(track.key, MusicalKey::parse("Ebm"));
    assert_eq!(track.bpm, Some(126.0));
    assert_eq!(track.genre.as_deref(), Some("Dance / Pop"));
    assert_eq!(track.play_count, 7, "numbers stored as text still import");
    assert_eq!(track.color, Some(tuneox_core::TrackColor::Green));

    let cues = lib.cues(bulletproof).unwrap();
    let summary: Vec<(Option<char>, i64, Option<i64>)> = cues
        .iter()
        .map(|c| {
            (
                c.hot_cue_letter(),
                c.start_us / 1000,
                c.end_us.map(|e| e / 1000),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (None, 16, None),
            (None, 280_000, Some(287_619)),
            (Some('A'), 24, None),
            (Some('D'), 45_739, None),
        ]
    );
    assert_eq!(cues[3].comment.as_deref(), Some("Drop"));
    assert_eq!(cues[3].color, Some(3));

    let grid = lib.beat_grid(bulletproof).unwrap().unwrap();
    assert_eq!(grid.beats.len(), 3);
    assert_eq!(grid.beats[0].time_us, 24_000);
    assert_eq!(
        lib.analysis_dir(bulletproof).unwrap().unwrap(),
        root.path()
            .join("share/PIONEER/USBANLZ/abc/def")
            .display()
            .to_string()
    );
}

#[test]
fn reimport_updates_and_dry_run_changes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let master = rekordbox_dir(root.path());
    let mut ctx = Context::new(Library::open_in_memory().unwrap(), None);

    let preview = run(&mut ctx, root.path(), &master, true);
    assert_eq!(preview.tracks.created, 2);
    assert_eq!(
        ctx.library.summary().unwrap().tracks,
        0,
        "dry run writes nothing"
    );

    run(&mut ctx, root.path(), &master, false);
    let again = run(&mut ctx, root.path(), &master, false);
    assert_eq!((again.tracks.created, again.tracks.updated), (0, 2));
    assert_eq!((again.playlists.created, again.playlists.updated), (0, 3));
    let summary = ctx.library.summary().unwrap();
    assert_eq!((summary.tracks, summary.cues), (2, 4));
}
