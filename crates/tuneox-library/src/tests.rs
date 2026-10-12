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

/// A library with one track and cues: hot cues A (coloured) and C, memory
/// cues at 1 s and 4 s, and a memory loop 4–6 s.
fn with_cues() -> (crate::Library, i64) {
    let mut lib = crate::Library::open_in_memory().unwrap();
    let import = lib.import("rekordbox", "db1").unwrap();
    let id = import
        .upsert_track("1", &track("/m/a.aiff", "A", "X", 128.0, "Am"), None)
        .unwrap()
        .id;
    let looped = |start_ms: i64, end_ms: i64| Cue {
        end_us: Some(end_ms * 1000),
        ..cue(None, start_ms)
    };
    let coloured = Cue {
        color: Some(22),
        comment: Some("drop".into()),
        ..cue(Some(0), 500)
    };
    import
        .replace_cues(
            id,
            &[
                (coloured, Some(1)),
                (cue(Some(2), 9_000), Some(3)),
                (cue(None, 1_000), Some(0)),
                (cue(None, 4_000), Some(0)),
                (looped(4_000, 6_000), Some(0)),
            ],
        )
        .unwrap();
    import.commit().unwrap();
    (lib, id)
}

type CueSummary = (Option<u8>, i64, Option<i64>);

/// (slot, start ms, end ms) of a track's cues, memory cues first.
fn cue_list(lib: &crate::Library, id: i64) -> Vec<CueSummary> {
    lib.cues(id)
        .unwrap()
        .into_iter()
        .map(|c| (c.hot_cue, c.start_us / 1000, c.end_us.map(|e| e / 1000)))
        .collect()
}

#[test]
fn single_cues_are_added_and_deleted_without_touching_the_rest() {
    let (lib, id) = with_cues();
    let before = lib.track(id).unwrap().unwrap();
    // A hot cue in an empty slot.
    lib.add_cue(id, &cue(Some(1), 3_000)).unwrap();
    // A hot cue replaces its slot's old one.
    lib.add_cue(id, &cue(Some(2), 12_000)).unwrap();
    // A memory loop.
    let memory_loop = Cue {
        end_us: Some(10_000_000),
        ..cue(None, 8_000)
    };
    lib.add_cue(id, &memory_loop).unwrap();
    assert_eq!(
        cue_list(&lib, id),
        [
            (None, 1_000, None),
            (None, 4_000, None),
            (None, 4_000, Some(6_000)),
            (None, 8_000, Some(10_000)),
            (Some(0), 500, None),
            (Some(1), 3_000, None),
            (Some(2), 12_000, None),
        ]
    );
    // The untouched hot cue keeps its colour and comment.
    let a = lib
        .cues(id)
        .unwrap()
        .into_iter()
        .find(|c| c.hot_cue == Some(0))
        .unwrap();
    assert_eq!((a.color, a.comment.as_deref()), (Some(22), Some("drop")));

    assert!(lib.delete_hot_cue(id, 1).unwrap());
    assert!(!lib.delete_hot_cue(id, 1).unwrap(), "already gone");
    // The memory loop at 4 s, not the cue at the same start.
    assert!(
        lib.delete_memory_cue(id, 4_000_000, Some(6_000_000))
            .unwrap()
    );
    // Then the cue at 4 s.
    assert!(lib.delete_memory_cue(id, 4_000_000, None).unwrap());
    assert!(!lib.delete_memory_cue(id, 4_000_000, None).unwrap());
    // Hot cues are never deleted as memory cues.
    assert!(!lib.delete_memory_cue(id, 500_000, None).unwrap());
    assert_eq!(
        cue_list(&lib, id),
        [
            (None, 1_000, None),
            (None, 8_000, Some(10_000)),
            (Some(0), 500, None),
            (Some(2), 12_000, None),
        ]
    );
    // A memory cue whose end doesn't match still goes by its start.
    assert!(lib.delete_memory_cue(id, 8_000_000, None).unwrap());
    // Edits leave the track's metadata alone.
    assert_eq!(lib.track(id).unwrap().unwrap().info, before.info);

    assert!(matches!(
        lib.add_cue(999, &cue(None, 1)),
        Err(crate::Error::NoTrack(999))
    ));
    assert!(matches!(
        lib.delete_hot_cue(999, 0),
        Err(crate::Error::NoTrack(999))
    ));
}

#[test]
fn auto_gain_trims_are_saved_with_the_track() {
    let (lib, id) = with_cues();
    let gain = |lib: &crate::Library| lib.track(id).unwrap().unwrap().info.auto_gain_db;
    assert_eq!(gain(&lib), None);
    assert!(lib.set_auto_gain(id, Some(-2.5)).unwrap());
    assert_eq!(gain(&lib), Some(-2.5));
    assert!(!lib.set_auto_gain(id, Some(-2.5)).unwrap(), "unchanged");
    assert!(lib.set_auto_gain(id, None).unwrap());
    assert_eq!(gain(&lib), None);
    assert!(matches!(
        lib.set_auto_gain(999, Some(1.0)),
        Err(crate::Error::NoTrack(999))
    ));
}

#[test]
fn a_reimport_replaces_cues_edited_in_tuneox() {
    // The known gap until edits are tracked: rekordbox's cues win.
    let (mut lib, id) = with_cues();
    lib.add_cue(id, &cue(Some(5), 20_000)).unwrap();
    let import = lib.import("rekordbox", "db1").unwrap();
    import
        .replace_cues(id, &[(cue(None, 1_000), Some(0))])
        .unwrap();
    import.commit().unwrap();
    assert_eq!(cue_list(&lib, id), [(None, 1_000, None)]);
}
