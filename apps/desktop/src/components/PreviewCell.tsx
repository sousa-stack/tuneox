// The track table's Preview column (rekordbox's browser preview player,
// research §1.5): a play/stop button and the track's overview thumbnail.
// Clicking the thumbnail previews from that spot (pausing any playing deck);
// a playhead moves across the previewing row. Only rows on screen exist
// (the table is virtualized), so only they fetch thumbnails.

import { memo, useCallback, useEffect, useLayoutEffect, useRef, useSyncExternalStore } from "react";
import { useSettings } from "../decks/hooks";
import { attachPlayhead, preview, previewPlay, previewToggle } from "../decks/preview";
import { thumbnailData, thumbnailImage, watchThumbnail } from "../decks/thumbnails";
import type { WaveformStyle } from "../decks/types";

const stop = (e: React.SyntheticEvent) => e.stopPropagation();

function useThumbnail(trackId: number, style: WaveformStyle) {
  const subscribe = useCallback((onChange: () => void) => watchThumbnail(trackId, style, onChange), [trackId, style]);
  return useSyncExternalStore(subscribe, () => thumbnailData(trackId, style));
}

export const PreviewCell = memo(function PreviewCell({ trackId, durationSecs }: { trackId: number; durationSecs: number | null }) {
  const { style } = useSettings();
  const view = useSyncExternalStore(preview.subscribe, () => preview.view);
  const thumb = useThumbnail(trackId, style);
  const canvas = useRef<HTMLCanvasElement>(null);
  const playhead = useRef<HTMLDivElement>(null);
  const active = view.trackId === trackId;
  const playing = active && view.playing;

  useLayoutEffect(() => {
    const element = canvas.current;
    if (!element) return;
    const dpr = window.devicePixelRatio || 1;
    const rect = element.getBoundingClientRect();
    const width = Math.max(1, Math.round(rect.width * dpr));
    const height = Math.max(1, Math.round(rect.height * dpr));
    if (element.width !== width) element.width = width;
    if (element.height !== height) element.height = height;
    const ctx = element.getContext("2d");
    if (!ctx) return;
    if (thumb) ctx.putImageData(thumbnailImage(trackId, thumb, width, height), 0, 0);
    else ctx.clearRect(0, 0, width, height);
  }, [thumb, trackId]);

  useEffect(() => {
    if (!active || !playhead.current) return;
    return attachPlayhead(trackId, playhead.current);
  }, [active, trackId]);

  const durationUs = () => {
    const s = preview.state;
    return s.track_id === trackId && s.duration_us > 0 ? s.duration_us : (durationSecs ?? 0) * 1e6;
  };

  return (
    <div className="preview-cell" onMouseDown={stop} onDoubleClick={stop} onContextMenu={stop}>
      <button
        type="button"
        tabIndex={-1}
        className={`preview-btn${playing ? " on" : ""}`}
        title={playing ? "Stop the preview" : "Preview this track (pauses the decks)"}
        aria-label={playing ? "Stop preview" : "Preview"}
        onClick={() => previewToggle(trackId, durationUs())}
      >
        {playing ? (
          <svg viewBox="0 0 10 10" width="8" height="8" aria-hidden>
            <rect x="1" y="1" width="8" height="8" />
          </svg>
        ) : (
          <svg viewBox="0 0 10 10" width="8" height="8" aria-hidden>
            <path d="M2 1 L9 5 L2 9 Z" />
          </svg>
        )}
      </button>
      <div
        className={`preview-thumb${thumb === null ? " none" : ""}`}
        title={thumb === null ? "No waveform: the track hasn't been analysed by rekordbox" : "Click to preview from here"}
        onClick={(e) => {
          const rect = e.currentTarget.getBoundingClientRect();
          const fraction = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
          const duration = durationUs();
          previewPlay(trackId, fraction * duration, duration);
        }}
      >
        <canvas ref={canvas} />
        {active && <div ref={playhead} className={`preview-playhead${playing ? " playing" : ""}`} />}
      </div>
    </div>
  );
});
