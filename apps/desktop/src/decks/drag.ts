// Dragging a track from the browser onto a deck. Done with mouse events
// rather than HTML5 drag and drop, which the Tauri window uses for file
// drops. Drop zones are elements with `data-deck="1" | "2"`, or
// `data-deck="split"` (the stacked waveforms: top half = A, bottom = B).

import { loadTrack } from "./actions";
import type { DeckId } from "./types";

type Listener = () => void;

let target: DeckId | null = null;
let active = false;
const listeners = new Set<Listener>();

function set(nextActive: boolean, nextTarget: DeckId | null) {
  if (nextActive === active && nextTarget === target) return;
  active = nextActive;
  target = nextTarget;
  for (const listener of listeners) listener();
}

export const dragStore = {
  /** The deck under a track being dragged, 0 while dragging elsewhere, null when not dragging. */
  get: (): DeckId | 0 | null => (active ? (target ?? 0) : null),
  subscribe(listener: Listener) {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  },
};

export function deckAt(x: number, y: number): DeckId | null {
  const element = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-deck]");
  if (!element) return null;
  const value = element.dataset.deck;
  if (value === "split") {
    const rect = element.getBoundingClientRect();
    return y < rect.top + rect.height / 2 ? 1 : 2;
  }
  return value === "1" ? 1 : value === "2" ? 2 : null;
}

const THRESHOLD_PX = 5;

/** Call on mousedown over a track row; becomes a drag once the mouse moves. */
export function beginTrackDrag(trackId: number, label: string, event: MouseEvent): void {
  if (event.button !== 0) return;
  const startX = event.clientX;
  const startY = event.clientY;
  let ghost: HTMLElement | null = null;

  const move = (e: MouseEvent) => {
    if (!ghost) {
      if (Math.hypot(e.clientX - startX, e.clientY - startY) < THRESHOLD_PX) return;
      ghost = document.createElement("div");
      ghost.className = "drag-ghost";
      ghost.textContent = label;
      document.body.appendChild(ghost);
      document.body.classList.add("dragging-track");
    }
    ghost.style.transform = `translate(${e.clientX + 12}px, ${e.clientY + 10}px)`;
    set(true, deckAt(e.clientX, e.clientY));
  };
  const up = (e: MouseEvent) => {
    window.removeEventListener("mousemove", move);
    window.removeEventListener("mouseup", up);
    window.removeEventListener("keydown", cancel);
    if (!ghost) return;
    ghost.remove();
    document.body.classList.remove("dragging-track");
    const deck = deckAt(e.clientX, e.clientY);
    set(false, null);
    if (deck) loadTrack(deck, trackId);
  };
  const cancel = (e: KeyboardEvent) => {
    if (e.key !== "Escape") return;
    window.removeEventListener("mousemove", move);
    window.removeEventListener("mouseup", up);
    window.removeEventListener("keydown", cancel);
    ghost?.remove();
    document.body.classList.remove("dragging-track");
    set(false, null);
  };
  window.addEventListener("mousemove", move);
  window.addEventListener("mouseup", up);
  window.addEventListener("keydown", cancel);
}
