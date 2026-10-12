// App-level overlays for the decks: toasts and the track context menu.

import { useEffect } from "react";
import { loadTrack } from "../../decks/actions";
import { useToasts } from "../../decks/hooks";
import { dismissToast } from "../../decks/toast";

export function Toasts() {
  const toasts = useToasts();
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className="toast" onClick={() => dismissToast(t.id)}>
          {t.message}
        </div>
      ))}
    </div>
  );
}

export interface MenuRequest {
  trackId: number;
  x: number;
  y: number;
}

export function TrackMenu({ request, onClose }: { request: MenuRequest; onClose: () => void }) {
  useEffect(() => {
    const close = () => onClose();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("mousedown", close);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    window.addEventListener("keydown", onKey);
    window.addEventListener("wheel", close, { passive: true });
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("wheel", close);
    };
  }, [onClose]);
  const left = Math.min(request.x, window.innerWidth - 180);
  const top = Math.min(request.y, window.innerHeight - 80);
  const item = (label: string, deck: 1 | 2) => (
    <button
      type="button"
      role="menuitem"
      onMouseDown={(e) => e.stopPropagation()}
      onClick={() => {
        loadTrack(deck, request.trackId);
        onClose();
      }}
    >
      {label}
    </button>
  );
  return (
    <div className="context-menu" role="menu" style={{ left, top }} onContextMenu={(e) => e.preventDefault()}>
      {item("Load to Deck A", 1)}
      {item("Load to Deck B", 2)}
    </div>
  );
}
