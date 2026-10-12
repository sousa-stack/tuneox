import { useMemo, useState } from "react";
import type { Playlist } from "../api";

export type View = { kind: "collection" } | { kind: "playlist"; id: number };

interface Props {
  playlists: Playlist[];
  trackCount: number;
  view: View;
  onSelect: (view: View) => void;
  footer: React.ReactNode;
}

export function Sidebar({ playlists, trackCount, view, onSelect, footer }: Props) {
  const children = useMemo(() => {
    const map = new Map<number | null, Playlist[]>();
    for (const p of playlists) {
      const siblings = map.get(p.parent_id) ?? [];
      siblings.push(p);
      map.set(p.parent_id, siblings);
    }
    return map;
  }, [playlists]);
  const [collapsed, setCollapsed] = useState<Set<number>>(new Set());

  const toggle = (id: number) =>
    setCollapsed((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const renderLevel = (parent: number | null, depth: number): React.ReactNode =>
    (children.get(parent) ?? []).map((p) => {
      const isFolder = p.kind === "folder";
      const open = isFolder && !collapsed.has(p.id);
      const selected = view.kind === "playlist" && view.id === p.id;
      return (
        <li key={p.id}>
          <button
            className={`tree-item${selected ? " selected" : ""}`}
            style={{ paddingLeft: 12 + depth * 14 }}
            onClick={() => (isFolder ? toggle(p.id) : onSelect({ kind: "playlist", id: p.id }))}
            title={p.name}
          >
            <span className="tree-icon" aria-hidden>
              {isFolder ? (open ? "▾" : "▸") : p.kind === "smart" ? "✦" : "♪"}
            </span>
            <span className="tree-name">{p.name}</span>
            {!isFolder && <span className="tree-count">{p.track_count}</span>}
          </button>
          {open && <ul>{renderLevel(p.id, depth + 1)}</ul>}
        </li>
      );
    });

  return (
    <nav className="sidebar">
      <div className="sidebar-scroll">
        <button
          className={`tree-item collection${view.kind === "collection" ? " selected" : ""}`}
          onClick={() => onSelect({ kind: "collection" })}
        >
          <span className="tree-icon" aria-hidden>
            ◉
          </span>
          <span className="tree-name">Collection</span>
          <span className="tree-count">{trackCount.toLocaleString()}</span>
        </button>
        <div className="sidebar-heading">Playlists</div>
        {playlists.length === 0 ? (
          <p className="sidebar-empty">No playlists yet.</p>
        ) : (
          <ul className="tree">{renderLevel(null, 0)}</ul>
        )}
      </div>
      <div className="sidebar-footer">{footer}</div>
    </nav>
  );
}
