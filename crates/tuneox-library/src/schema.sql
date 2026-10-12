-- TuneOx library schema, version 2 (migrations from older versions are in lib.rs).
-- Positions are microseconds; per-beat tempos are BPM x 100; timestamps are
-- unix milliseconds.

CREATE TABLE track (
    id            INTEGER PRIMARY KEY,
    uuid          TEXT NOT NULL UNIQUE,
    path          TEXT NOT NULL UNIQUE,
    title         TEXT NOT NULL DEFAULT '',
    artist        TEXT,
    album         TEXT,
    genre         TEXT,
    label         TEXT,
    remixer       TEXT,
    comment       TEXT,
    musical_key   TEXT,
    bpm           REAL,
    duration_secs REAL,
    rating        INTEGER NOT NULL DEFAULT 0,
    color         TEXT,
    file_type     TEXT,
    sample_rate   INTEGER,
    bit_depth     INTEGER,
    bitrate       INTEGER,
    file_size     INTEGER,
    play_count    INTEGER NOT NULL DEFAULT 0,
    year          INTEGER,
    track_number  INTEGER,
    date_added    TEXT,
    auto_gain_db  REAL,
    -- Folder holding this track's analysis files (ANLZ0000.DAT/.EXT/.2EX).
    analysis_dir  TEXT,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE INDEX track_bpm ON track (bpm);
CREATE INDEX track_key ON track (musical_key);

-- Where a track came from, so re-imports update instead of duplicating.
CREATE TABLE track_source (
    source         TEXT NOT NULL,   -- e.g. 'rekordbox'
    source_library TEXT NOT NULL,   -- e.g. rekordbox djmdProperty.DBID
    source_id      TEXT NOT NULL,   -- e.g. djmdContent.ID
    track_id       INTEGER NOT NULL REFERENCES track (id) ON DELETE CASCADE,
    PRIMARY KEY (source, source_library, source_id)
);

CREATE TABLE cue (
    id          INTEGER PRIMARY KEY,
    track_id    INTEGER NOT NULL REFERENCES track (id) ON DELETE CASCADE,
    hot_cue     INTEGER,            -- 0 = A ... 7 = H; NULL for memory cues
    start_us    INTEGER NOT NULL,
    end_us      INTEGER,            -- loop end; NULL for cue points
    color       INTEGER,
    comment     TEXT,
    source_kind INTEGER             -- raw kind from the import source, if any
);

CREATE INDEX cue_track ON cue (track_id);

CREATE TABLE beat_grid (
    track_id INTEGER PRIMARY KEY REFERENCES track (id) ON DELETE CASCADE,
    beats    BLOB NOT NULL          -- tuneox_core::BeatGrid::to_bytes
);

CREATE TABLE playlist (
    id             INTEGER PRIMARY KEY,
    uuid           TEXT NOT NULL UNIQUE,
    parent_id      INTEGER REFERENCES playlist (id) ON DELETE CASCADE,
    position       INTEGER NOT NULL,
    name           TEXT NOT NULL,
    kind           TEXT NOT NULL,   -- 'folder' | 'playlist' | 'smart'
    smart_rules    TEXT,
    source         TEXT,
    source_library TEXT,
    source_id      TEXT,
    UNIQUE (source, source_library, source_id)
);

CREATE TABLE playlist_track (
    playlist_id INTEGER NOT NULL REFERENCES playlist (id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    track_id    INTEGER NOT NULL REFERENCES track (id) ON DELETE CASCADE,
    PRIMARY KEY (playlist_id, position)
);

CREATE INDEX playlist_track_track ON playlist_track (track_id);

CREATE VIRTUAL TABLE track_search USING fts5 (
    title, artist, album, genre, label, remixer, comment,
    content = 'track', content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER track_search_insert AFTER INSERT ON track BEGIN
    INSERT INTO track_search (rowid, title, artist, album, genre, label, remixer, comment)
    VALUES (new.id, new.title, new.artist, new.album, new.genre, new.label, new.remixer, new.comment);
END;

CREATE TRIGGER track_search_delete AFTER DELETE ON track BEGIN
    INSERT INTO track_search (track_search, rowid, title, artist, album, genre, label, remixer, comment)
    VALUES ('delete', old.id, old.title, old.artist, old.album, old.genre, old.label, old.remixer, old.comment);
END;

CREATE TRIGGER track_search_update AFTER UPDATE ON track BEGIN
    INSERT INTO track_search (track_search, rowid, title, artist, album, genre, label, remixer, comment)
    VALUES ('delete', old.id, old.title, old.artist, old.album, old.genre, old.label, old.remixer, old.comment);
    INSERT INTO track_search (rowid, title, artist, album, genre, label, remixer, comment)
    VALUES (new.id, new.title, new.artist, new.album, new.genre, new.label, new.remixer, new.comment);
END;
