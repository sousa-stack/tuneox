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

const ROW_HEIGHT = 26;

interface Props {
  items: Item[];
  selectedId: number | null;
  onSelect: (id: number) => void;
  sort: Sort | null;
  onSort: (column: ColumnId) => void;
}

export function TrackTable({ items, selectedId, onSelect, sort, onSort }: Props) {
  const tableRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);

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

  // Keep the selection visible when it moves by keyboard or the list changes.
  useEffect(() => {
    if (selectedIndex >= 0) virtualizer.scrollToIndex(selectedIndex, { align: "auto" });
  }, [selectedIndex, virtualizer]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (items.length === 0) return;
    const steps: Record<string, number> = { ArrowDown: 1, ArrowUp: -1, PageDown: 20, PageUp: -20 };
    const step = steps[e.key];
    if (!step) return;
    e.preventDefault();
    const from = selectedIndex < 0 ? (step > 0 ? -1 : items.length) : selectedIndex;
    onSelect(items[Math.min(items.length - 1, Math.max(0, from + step))].row.id);
  };

  return (
    <div className="table" ref={tableRef}>
      <div className="table-head" style={{ gridTemplateColumns: template }}>
        <span />
        {columns.map((column) => (
          <button
            key={column.id}
            className={`th${column.align ? ` align-${column.align}` : ""}${sort?.column === column.id ? " sorted" : ""}`}
            onClick={() => onSort(column.id)}
          >
            {column.label}
            {sort?.column === column.id && <span className="sort-arrow">{sort.descending ? "↓" : "↑"}</span>}
          </button>
        ))}
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
                  onMouseDown={() => onSelect(row.id)}
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
