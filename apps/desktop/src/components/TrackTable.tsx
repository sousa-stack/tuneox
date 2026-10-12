import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  TRACK_COLORS,
  fitColumns,
  formatDuration,
  type Column,
  type ColumnId,
  type Item,
  type Sort,
} from "../model";
import { PreviewCell } from "./PreviewCell";

const ROW_HEIGHT = 26;

interface Props {
  items: Item[];
  selectedId: number | null;
  onSelect: (id: number) => void;
  sort: Sort | null;
  onSort: (column: ColumnId) => void;
  /**
   * The view and search text the items are filtered by. When it changes
   * the list starts from the top, or keeps the selected row in sight if a
   * search the user typed still matches it.
   */
  filter: { view: string; query: string };
  /** Mouse down on a row: may become a drag onto a deck. */
  onRowMouseDown?: (row: Item["row"], event: MouseEvent) => void;
  onRowDoubleClick?: (id: number) => void;
  onRowContextMenu?: (id: number, x: number, y: number) => void;
}

export function TrackTable({ items, selectedId, onSelect, sort, onSort, filter, onRowMouseDown, onRowDoubleClick, onRowContextMenu }: Props) {
  const tableRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  /** The selection just moved by keyboard: bring it into view. */
  const keyboardMove = useRef(false);
  const lastFilter = useRef(filter);

  useLayoutEffect(() => {
    const element = tableRef.current;
    if (!element) return;
    const observer = new ResizeObserver(([entry]) => setWidth(entry.contentRect.width));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const columns = useMemo(() => fitColumns(width), [width]);
  const template = `10px ${columns.map((c) => c.width).join(" ")}`;

  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 20,
  });

  const selectedIndex = items.findIndex((item) => item.row.id === selectedId);

  // Scroll only when the user asked for it: a keyboard move brings the
  // selection into view; a new search or view starts at the top, unless a
  // search the user typed still matches the selected row, which then stays
  // in sight. Library reloads, sorting and clicks never scroll, and clearing
  // the search doesn't jump to the selection deep in the collection.
  useEffect(() => {
    const filterChanged = filter.view !== lastFilter.current.view || filter.query !== lastFilter.current.query;
    const sameView = filter.view === lastFilter.current.view;
    lastFilter.current = filter;
    const element = scrollRef.current;
    if (keyboardMove.current) {
      keyboardMove.current = false;
      if (selectedIndex >= 0) virtualizer.scrollToIndex(selectedIndex, { align: "auto" });
      return;
    }
    if (!filterChanged || !element) return;
    const searching = sameView && filter.query.trim() !== "";
    if (searching && selectedIndex >= 0) {
      const top = selectedIndex * ROW_HEIGHT;
      const visible = top >= element.scrollTop && top + ROW_HEIGHT <= element.scrollTop + element.clientHeight;
      if (!visible) virtualizer.scrollToIndex(selectedIndex, { align: "center" });
      return;
    }
    element.scrollTop = 0;
  }, [filter, selectedIndex, virtualizer]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (items.length === 0) return;
    const steps: Record<string, number> = { ArrowDown: 1, ArrowUp: -1, PageDown: 20, PageUp: -20 };
    const step = steps[e.key];
    if (!step) return;
    e.preventDefault();
    const from = selectedIndex < 0 ? (step > 0 ? -1 : items.length) : selectedIndex;
    const next = items[Math.min(items.length - 1, Math.max(0, from + step))].row.id;
    if (next === selectedId) return;
    keyboardMove.current = true;
    onSelect(next);
  };

  return (
    <div className="table" ref={tableRef}>
      <div className="table-head" style={{ gridTemplateColumns: template }}>
        <span />
        {columns.map((column) =>
          column.sortable === false ? (
            <span key={column.id} className="th static">
              {column.label}
            </span>
          ) : (
            <button
              key={column.id}
              className={`th${column.align ? ` align-${column.align}` : ""}${sort?.column === column.id ? " sorted" : ""}`}
              onClick={() => onSort(column.id)}
            >
              {column.label}
              {sort?.column === column.id && <span className="sort-arrow">{sort.descending ? "↓" : "↑"}</span>}
            </button>
          ),
        )}
      </div>
      <div className="table-body" ref={scrollRef} tabIndex={0} onKeyDown={onKeyDown}>
        {items.length === 0 ? (
          <div className="table-empty">No tracks match.</div>
        ) : (
          <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
            {virtualizer.getVirtualItems().map((virtual) => {
              const item = items[virtual.index];
              const { row } = item;
              return (
                <div
                  key={`${row.id}:${item.position}`}
                  className={`tr${row.id === selectedId ? " selected" : ""}${virtual.index % 2 ? " odd" : ""}`}
                  style={{ gridTemplateColumns: template, height: ROW_HEIGHT, transform: `translateY(${virtual.start}px)` }}
                  onMouseDown={(e) => {
                    onSelect(row.id);
                    onRowMouseDown?.(row, e.nativeEvent);
                  }}
                  onDoubleClick={() => onRowDoubleClick?.(row.id)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    onSelect(row.id);
                    onRowContextMenu?.(row.id, e.clientX, e.clientY);
                  }}
                >
                  <span className="color-dot" style={{ background: row.color ? TRACK_COLORS[row.color] : "transparent" }} />
                  {columns.map((column) => cell(column, item))}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

function cell(column: Column, { row, position }: Item) {
  const align = column.align ? ` align-${column.align}` : "";
  const span = (className: string, content: React.ReactNode, title?: string) => (
    <span key={column.id} className={`td${align} ${className}`} title={title}>
      {content}
    </span>
  );
  switch (column.id) {
    case "position":
      return span("muted mono", position);
    case "preview":
      return (
        <span key={column.id} className="td preview">
          <PreviewCell trackId={row.id} durationSecs={row.duration_secs} />
        </span>
      );
    case "title":
      return span("strong", row.title, row.title);
    case "artist":
      return span("", row.artist, row.artist ?? undefined);
    case "album":
      return span("muted", row.album, row.album ?? undefined);
    case "genre":
      return span("muted", row.genre, row.genre ?? undefined);
    case "bpm":
      return span("mono", row.bpm?.toFixed(2));
    case "key":
      return span("mono", row.camelot, row.key ?? undefined);
    case "duration":
      return span("mono muted", formatDuration(row.duration_secs));
    case "rating":
      return span("rating", row.rating > 0 ? "★".repeat(row.rating) : "");
    case "added":
      return span("mono muted", row.date_added);
  }
}
