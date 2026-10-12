use tuneox_core::{Beat, BeatGrid, Cue, MusicalKey, PlaylistKind, TrackInfo};

use crate::{Library, SearchQuery};

fn track(path: &str, title: &str, artist: &str, bpm: f64, key: &str) -> TrackInfo {
    TrackInfo {
        path: path.to_string(),
        title: title.to_string(),
        artist: Some(artist.to_string()),
        bpm: Some(bpm),
        key: MusicalKey::parse(key),
        ..TrackInfo::default()
    }
}

fn cue(hot_cue: Option<u8>, start_ms: i64) -> Cue {
    Cue {
        hot_cue,
        start_us: start_ms * 1000,
        end_us: None,
        color: None,
        comment: None,
    }
}

fn seeded() -> Library {
    let mut lib = Library::open_in_memory().unwrap();
    let import = lib.import("rekordbox", "db1").unwrap();
    let a = import
        .upsert_track(
            "10",
            &track("/m/cinema.aiff", "Cinema", "Benny Benassi", 125.0, "Am"),
            Some("/anlz/a"),
        )
        .unwrap();
    let b = import
        .upsert_track(
            "11",
            &track("/m/cheque.aiff", "Cheque", "Don Diablo", 122.0, "Em"),
            None,
        )
        .unwrap();
    let c = import
        .upsert_track(
            "12",
            &track(
                "/m/bulletproof.aiff",
                "Bulletproof",
                "La Roux",
                126.0,
                "C#m",
            ),
            None,
        )
        .unwrap();
    import
        .replace_cues(
            a.id,
            &[(cue(Some(0), 16), Some(1)), (cue(None, 16), Some(0))],
        )
        .unwrap();
    import
        .set_beat_grid(
            a.id,
            &BeatGrid {
                beats: vec![Beat {
                    time_us: 16_000,
                    bar_position: 1,
                    bpm_centi: 12_500,
                }],
            },
        )
        .unwrap();
    let folder = import
        .upsert_playlist("100", None, "Gigs", PlaylistKind::Folder, 0, None)
        .unwrap();
    let set = import
        .upsert_playlist(
            "101",
            Some(folder.id),
            "10-9-26",
            PlaylistKind::Playlist,
            0,
            None,
        )
        .unwrap();
    import
        .replace_playlist_tracks(set.id, &[c.id, a.id, b.id])
        .unwrap();
    import.commit().unwrap();
    lib
}

#[test]
fn imports_tracks_cues_grids_and_playlists() {
    let lib = seeded();
    let summary = lib.summary().unwrap();
    assert_eq!(
        (
            summary.tracks,
            summary.playlists,
            summary.folders,
            summary.cues,
            summary.beat_grids
        ),
        (3, 1, 1, 2, 1)
    );

    let playlists = lib.playlists().unwrap();
    assert_eq!(playlists[0].name, "Gigs");
    assert_eq!(playlists[1].parent_id, Some(playlists[0].id));
    assert_eq!(playlists[1].track_count, 3);

    let order: Vec<String> = lib
        .playlist_tracks(playlists[1].id)
        .unwrap()
        .into_iter()
        .map(|id| lib.track(id).unwrap().unwrap().info.title)
        .collect();
    assert_eq!(order, ["Bulletproof", "Cinema", "Cheque"]);

    let cinema = lib.search(&query("cinema")).unwrap().remove(0);
    assert_eq!(cinema.info.key, MusicalKey::parse("Am"));
    let cues = lib.cues(cinema.id).unwrap();
    assert_eq!(cues[0].hot_cue, None, "memory cues sort first");
    assert_eq!(cues[1].hot_cue_letter(), Some('A'));
    assert_eq!(
        lib.beat_grid(cinema.id).unwrap().unwrap().constant_bpm(),
        Some(125.0)
    );
    assert_eq!(
        lib.analysis_dir(cinema.id).unwrap().as_deref(),
        Some("/anlz/a")
    );
}

#[test]
fn reimport_updates_instead_of_duplicating() {
    let mut lib = seeded();
    let import = lib.import("rekordbox", "db1").unwrap();
    let updated = import
        .upsert_track(
            "10",
            &track(
                "/m/cinema.aiff",
                "Cinema (Galantis Remix)",
                "Benny Benassi",
                125.0,
                "Am",
            ),
            None,
        )
        .unwrap();
    assert!(!updated.created);
    // Same file under a different source ID merges into the existing track.
    let merged = import
        .upsert_track(
            "99",
            &track("/m/cheque.aiff", "Cheque", "Don Diablo", 122.0, "Em"),
            None,
        )
        .unwrap();
    assert!(!merged.created);
    let reparented = import
        .upsert_playlist("101", None, "10-9-26", PlaylistKind::Playlist, 1, None)
        .unwrap();
    assert!(!reparented.created);
    import.commit().unwrap();

    assert_eq!(lib.summary().unwrap().tracks, 3);
    let track = lib.track(updated.id).unwrap().unwrap();
    assert_eq!(track.info.title, "Cinema (Galantis Remix)");
    assert_eq!(
        lib.analysis_dir(updated.id).unwrap().as_deref(),
        Some("/anlz/a"),
        "a missing analysis dir keeps the old one"
    );
    assert!(
        lib.playlists()
            .unwrap()
            .iter()
            .all(|p| p.parent_id.is_none())
    );
}

#[test]
fn dropped_import_rolls_back() {
    let mut lib = seeded();
    {
        let import = lib.import("rekordbox", "db1").unwrap();
        import
            .upsert_track(
                "50",
                &track("/m/new.mp3", "New", "Someone", 128.0, "8A"),
                None,
            )
            .unwrap();
    }
    assert_eq!(lib.summary().unwrap().tracks, 3);
}

#[test]
fn searches_by_text_bpm_and_key() {
    let lib = seeded();
    let titles = |q: SearchQuery| -> Vec<String> {
        lib.search(&q)
            .unwrap()
            .into_iter()
            .map(|t| t.info.title)
            .collect()
    };

    assert_eq!(titles(query("ben ben")), ["Cinema"]);
    assert_eq!(titles(query("diab")), ["Cheque"]);
    assert_eq!(
        titles(SearchQuery {
            bpm_min: Some(124.0),
            bpm_max: Some(128.0),
            limit: 10,
            ..SearchQuery::default()
        }),
        ["Cinema", "Bulletproof"]
    );
    assert_eq!(
        titles(SearchQuery {
            // 8A (Am) mixes with 9A (Em), 7A (Dm) and 8B (C).
            compatible_with: MusicalKey::parse("8A"),
            limit: 10,
            ..SearchQuery::default()
        }),
        ["Cinema", "Cheque"]
    );
    assert!(
        titles(query("\"")).len() == 3,
        "empty text means no text filter"
    );
}

fn query(text: &str) -> SearchQuery {
    SearchQuery {
        text: Some(text.to_string()),
        limit: 10,
        ..SearchQuery::default()
    }
}

#[test]
fn migrates_a_version_1_library() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    {
        // A version 1 library: no auto_gain_db column.
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(&include_str!("schema.sql").replace("    auto_gain_db  REAL,\n", ""))
            .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute(
            "INSERT INTO track (uuid, path, title, created_at, updated_at)
             VALUES ('u', '/m/old.mp3', 'Old', 0, 0)",
            [],
        )
        .unwrap();
    }
    let mut lib = Library::open(&path).unwrap();
    let old = lib.search(&query("old")).unwrap().remove(0);
    assert_eq!(old.info.auto_gain_db, None);
    let import = lib.import("rekordbox", "db1").unwrap();
    let gained = TrackInfo {
        auto_gain_db: Some(-1.5),
        ..track("/m/new.mp3", "New", "X", 120.0, "C")
    };
    let id = import.upsert_track("1", &gained, None).unwrap().id;
    import.commit().unwrap();
    assert_eq!(
        lib.track(id).unwrap().unwrap().info.auto_gain_db,
        Some(-1.5)
    );
}

#[test]
fn reopens_an_existing_library() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/library.db");
    {
        let mut lib = crate::Library::open(&path).unwrap();
        let import = lib.import("rekordbox", "db1").unwrap();
        import
            .upsert_track("1", &track("/m/a.mp3", "A", "X", 120.0, "C"), None)
            .unwrap();
        import.commit().unwrap();
    }
    let lib = crate::Library::open(&path).unwrap();
    assert_eq!(lib.summary().unwrap().tracks, 1);
}

#[test]
fn pages_through_tracks_and_finds_playlists() {
    let lib = seeded();
    let first = lib.tracks(0, 2).unwrap();
    let rest = lib.tracks(2, 2).unwrap();
    assert_eq!(first.len(), 2);
    assert_eq!(rest.len(), 1);
    assert!(
        first[0].id < first[1].id && first[1].id < rest[0].id,
        "ID order"
    );

    let playlists = lib.playlists().unwrap();
    let set = lib.playlist(playlists[1].id).unwrap().unwrap();
    assert_eq!((set.name.as_str(), set.track_count), ("10-9-26", 3));
    assert!(lib.playlist(9999).unwrap().is_none());
}
