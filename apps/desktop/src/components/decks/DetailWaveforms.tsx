// The two enlarged waveforms, stacked and facing each other, with zoom
// controls on the left edge. Click a lane to move that deck's playhead there;
// drag to scrub (deck_seek, play state kept).

import { useEffect, useRef } from "react";
import * as act from "../../decks/actions";
import { useSettings } from "../../decks/hooks";
import { animator } from "../../decks/render/animator";
import { ZOOM_SPANS } from "../../decks/settings";
import type { DeckId } from "../../decks/types";
import { DeckButton } from "./controls";

interface Drag {
  deck: DeckId;
  startX: number;
  lastX: number;
  moved: boolean;
}

const CLICK_SLOP_PX = 3;

export function DetailWaveforms() {
  const canvas = useRef<HTMLCanvasElement>(null);
  const drag = useRef<Drag | null>(null);
  const settings = useSettings();
  useEffect(() => animator.attachDetail(canvas.current!), []);

  const deckAt = (e: React.PointerEvent<HTMLCanvasElement>): DeckId => {
    const rect = e.currentTarget.getBoundingClientRect();
    return e.clientY < rect.top + rect.height / 2 ? 1 : 2;
  };

  /** Track µs per CSS pixel on a deck's lane. */
  const usPerPixel = (deck: DeckId) => {
    const view = animator.detailView;
    if (!view) return 0;
    return view.dpr / view.pxPerUs(ZOOM_SPANS[settings.zoom], act.deckState(deck));
  };

  return (
    <div className="detail-area">
      <div className="zoom-controls">
        <DeckButton className="zoom" title="Zoom in (=)" disabled={settings.zoom === 0} onClick={() => act.zoom(1)}>
          +
        </DeckButton>
        <DeckButton className="zoom rst" title={`Reset zoom (⇧=) · ${ZOOM_SPANS[settings.zoom]} s visible`} onClick={() => act.zoom(0)}>
          RST
        </DeckButton>
        <DeckButton className="zoom" title="Zoom out (−)" disabled={settings.zoom === ZOOM_SPANS.length - 1} onClick={() => act.zoom(-1)}>
          −
        </DeckButton>
        <span className="zoom-span">{ZOOM_SPANS[settings.zoom]}s</span>
      </div>
      <canvas
        ref={canvas}
        className="detail"
        data-deck="split"
        onPointerDown={(e) => {
          if (e.button !== 0) return;
          const deck = deckAt(e);
          if (act.deckState(deck).track_id == null) return;
          e.currentTarget.setPointerCapture(e.pointerId);
          drag.current = { deck, startX: e.clientX, lastX: e.clientX, moved: false };
        }}
        onPointerMove={(e) => {
          const d = drag.current;
          if (!d) return;
          if (!d.moved && Math.abs(e.clientX - d.startX) < CLICK_SLOP_PX) return;
          d.moved = true;
          const dx = e.clientX - d.lastX;
          d.lastX = e.clientX;
          if (dx !== 0) act.seekTo(d.deck, act.currentPosition(d.deck) - dx * usPerPixel(d.deck));
        }}
        onPointerUp={(e) => {
          const d = drag.current;
          drag.current = null;
          if (!d || d.moved) return;
          // A click moves the playhead to the clicked spot.
          const rect = e.currentTarget.getBoundingClientRect();
          const offset = e.clientX - (rect.left + rect.width / 2);
          act.seekTo(d.deck, act.currentPosition(d.deck) + offset * usPerPixel(d.deck));
        }}
        onPointerCancel={() => (drag.current = null)}
      />
    </div>
  );
}
