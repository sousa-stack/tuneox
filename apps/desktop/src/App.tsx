import { useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { api, loadAllTracks, onProgress, type ImportReport, type Playlist, type Progress } from "./api";
import { matcher, sortItems, toRow, type ColumnId, type Item, type Sort, type TrackRow } from "./model";
import { Sidebar, type View } from "./components/Sidebar";
import { TrackTable } from "./components/TrackTable";
import { TrackDetail } from "./components/TrackDetail";
import "./App.css";

type Loading = { loaded: number; total: number } | null;

export default function App() {
  const [tracks, setTracks] = useState<Map<number, TrackRow>>(new Map());
  const [loading, setLoading] = useState<Loading>({ loaded: 0, total: 0 });
  const [playlists, setPlaylists] = useState<Playlist[]>([]);
  const [view, setView] = useState<View>({ kind: "collection" });
  const [playlistIds, setPlaylistIds] = useState<number[] | null>(null);
  const [query, setQuery] = useState("");
  const deferredQuery = useDeferredValue(query);
  const [sort, setSort] = useState<Sort | null>(null);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [importing, setImporting] = useState<Progress | "starting" | null>(null);
  const [lastImport, setLastImport] = useState<ImportReport | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  const load = useCallback(async () => {
    setLoading({ loaded: 0, total: 0 });
    try {
      setPlaylists(await api.playlists());
      const next = new Map<number, TrackRow>();
      await loadAllTracks((page, loaded, total) => {
        for (const track of page) next.set(track.id, toRow(track));
        setTracks(new Map(next));
        setLoading(loaded < total ? { loaded, total } : null);
      });
      setLoading(null);
    } catch (e) {
      setError(String(e));
      setLoading(null);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  useEffect(() => {
    const unlisten = onProgress((p) => p.command === "import_rekordbox" && setImporting(p));
    return () => {
      unlisten.then((stop) => stop());
    };
  }, []);

  // Playlist contents come from the core; everything else is local.
  useEffect(() => {
    if (view.kind !== "playlist") {
      setPlaylistIds(null);
      return;
    }
    let current = true;
    setPlaylistIds(null);
    api
      .playlistTrackIds(view.id)
      .then((ids) => current && setPlaylistIds(ids))
      .catch((e) => current && setError(String(e)));
    return () => {
      current = false;
    };
  }, [view]);

  // Cmd/Ctrl+F focuses search.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // The view's own order: playlist order, or newest first for the collection.
  const base = useMemo<Item[]>(() => {
    if (view.kind === "playlist") {
      return (playlistIds ?? []).flatMap((id, i) => {
        const row = tracks.get(id);
        return row ? [{ row, position: i + 1 }] : [];
      });
    }
    return [...tracks.values()]
      .sort((a, b) => (b.date_added ?? "").localeCompare(a.date_added ?? "") || b.id - a.id)
      .map((row, i) => ({ row, position: i + 1 }));
  }, [view, playlistIds, tracks]);

  const items = useMemo<Item[]>(() => {
    const matches = matcher(deferredQuery);
    const filtered = base.filter((item) => matches(item.row));
    return sort ? sortItems(filtered, sort) : filtered;
  }, [base, deferredQuery, sort]);

  const onSort = (column: ColumnId) =>
    setSort((current) => {
      if (current?.column !== column) return { column, descending: column === "rating" || column === "added" };
      if (current.descending === (column === "rating" || column === "added")) {
        return { column, descending: !current.descending };
      }
      return null; // third click: back to the view's own order
    });

  const runImport = async () => {
    setImporting("starting");
    setError(null);
    try {
      setLastImport(await api.importRekordbox());
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setImporting(null);
    }
  };

  const title = view.kind === "playlist" ? (playlists.find((p) => p.id === view.id)?.name ?? "Playlist") : "Collection";
  const empty = loading === null && tracks.size === 0;

  return (
    <div className="app">
      <header className="toolbar">
        <div className="brand">
          Tune<span>Ox</span>
        </div>
        <input
          ref={searchRef}
          className="search"
          type="search"
          placeholder="Search title, artist, album, genre, label…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === "Escape" && setQuery("")}
          spellCheck={false}
        />
        <div className="toolbar-status">
          {loading
            ? loading.total
              ? `Loading ${loading.loaded.toLocaleString()} of ${loading.total.toLocaleString()}…`
              : "Loading…"
            : `${items.length.toLocaleString()} ${items.length === 1 ? "track" : "tracks"}`}
        </div>
      </header>

      {error && (
        <div className="banner error" role="alert">
          {error}
          <button onClick={() => setError(null)}>Dismiss</button>
        </div>
      )}

      <div className="body">
        <Sidebar
          playlists={playlists}
          trackCount={tracks.size}
          view={view}
          onSelect={(v) => {
            setView(v);
            setSort(null);
          }}
          footer={
            <button className="link-button" onClick={runImport} disabled={importing !== null}>
              {importing ? progressLabel(importing) : "Import from rekordbox"}
            </button>
          }
        />
        <main className="main">
          {empty ? (
            <Welcome importing={importing} onImport={runImport} />
          ) : (
            <>
              <div className="view-title">
                <h1>{title}</h1>
                {lastImport && (
                  <span className="muted">
                    Imported {lastImport.tracks.found.toLocaleString()} tracks, {lastImport.playlists.found} playlists and{" "}
                    {lastImport.cues.toLocaleString()} cues in {(lastImport.elapsed_ms / 1000).toFixed(1)} s
                  </span>
                )}
              </div>
              <TrackTable items={items} selectedId={selectedId} onSelect={setSelectedId} sort={sort} onSort={onSort} />
            </>
          )}
        </main>
        {!empty && <TrackDetail trackId={selectedId} />}
      </div>
    </div>
  );
}

function progressLabel(p: Progress | "starting"): string {
  if (p === "starting") return "Starting import…";
  return p.total > 1 ? `${capitalize(p.stage)}… ${Math.round((100 * p.done) / p.total)}%` : `${capitalize(p.stage)}…`;
}

function capitalize(text: string) {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function Welcome({ importing, onImport }: { importing: Progress | "starting" | null; onImport: () => void }) {
  const percent = importing && importing !== "starting" && importing.total > 1 ? (100 * importing.done) / importing.total : null;
  return (
    <div className="welcome">
      <h1>Bring your rekordbox library</h1>
      <p>
        TuneOx reads your tracks, playlists, cues and beat grids straight from rekordbox. rekordbox's files are only
        read, never changed, and you can re-import any time.
      </p>
      <button className="primary" onClick={onImport} disabled={importing !== null}>
        {importing ? progressLabel(importing) : "Import from rekordbox"}
      </button>
      {importing && (
        <div className="progress">
          <div className="progress-bar" style={{ width: percent != null ? `${percent}%` : "8%" }} />
        </div>
      )}
    </div>
  );
}
