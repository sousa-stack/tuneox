// The sub-information panel (research §1.3): HOT CUE A–H (click a stored pad
// to jump and play, an empty one to store the position) and MEMORY (click to
// jump and pause). Hovering a row shows ✕, which deletes the cue from the
// deck and the track. Colours and comments come with the engine's cues.

import { useMemo } from "react";
import * as act from "../../decks/actions";
import { formatClock } from "../../decks/render/animator";
import { hotCues, memoryCues } from "../../decks/cues";
import { useDeck, useSettings } from "../../decks/hooks";
import { settings as settingsStore } from "../../decks/settings";
import type { DeckId } from "../../decks/types";

const noFocus = (e: React.MouseEvent) => e.preventDefault();

function DeleteButton({ title, onDelete }: { title: string; onDelete: () => void }) {
  return (
    <button
      type="button"
      tabIndex={-1}
      className="cue-delete"
      title={title}
      aria-label={title}
      onMouseDown={noFocus}
      onPointerDown={(e) => e.stopPropagation()}
      onClick={onDelete}
    >
      ✕
    </button>
  );
}

export function CuePanel({ deck }: { deck: DeckId }) {
  const { state } = useDeck(deck);
  const settings = useSettings();
  const hot = useMemo(() => hotCues(state), [state]);
  const memory = useMemo(() => memoryCues(state), [state]);
  const tab = settings.cueTab[deck - 1];
  const loaded = state.track_id != null;
  const shift = deck === 2 ? "⇧" : "";
  const tabButton = (id: "memory" | "hot", label: string, count: number) => (
    <button type="button" tabIndex={-1} className={tab === id ? "on" : ""} onMouseDown={noFocus} onClick={() => settingsStore.updateDeck("cueTab", deck, id)}>
      {label}
      {loaded && count > 0 && <span className="count">{count}</span>}
    </button>
  );
  return (
    <div className={`cue-panel deck-${deck}`} data-deck={deck}>
      <div className="cue-tabs">
        {tabButton("memory", "MEMORY", memory.length)}
        {tabButton("hot", "HOT CUE", hot.filter(Boolean).length)}
      </div>
      {tab === "hot" ? (
        <ul className="cue-list hot">
          {hot.map((cue, slot) => {
            const letter = String.fromCharCode(65 + slot);
            return (
              <li key={slot} className="cue-item">
                <button
                  type="button"
                  tabIndex={-1}
                  className={`cue-row${cue ? "" : " empty"}`}
                  disabled={!loaded}
                  title={cue ? `Hot cue ${letter}: jump and play` : `Store hot cue ${letter} here`}
                  onMouseDown={noFocus}
                  onPointerDown={(e) => {
                    if (e.button !== 0) return;
                    e.currentTarget.setPointerCapture(e.pointerId);
                    act.hotCuePress(deck, slot);
                  }}
                  onPointerUp={() => act.hotCueRelease(deck, slot)}
                >
                  <span className="pad" style={cue ? { background: cue.color, borderColor: cue.color } : undefined}>
                    {letter}
                  </span>
                  {cue ? (
                    <>
                      <span className="cue-time">{formatClock(cue.start_us)}</span>
                      <span className="cue-note">{cue.comment ?? (cue.end_us != null ? loopLabel(cue.start_us, cue.end_us, state.original_bpm) : "")}</span>
                    </>
                  ) : (
                    <span className="cue-note faint">—</span>
                  )}
                </button>
                {cue && (
                  <DeleteButton
                    title={`Delete hot cue ${letter}${slot < 3 ? ` (${shift}⌘${slot + 1})` : ""}`}
                    onDelete={() => act.hotCueDelete(deck, slot)}
                  />
                )}
              </li>
            );
          })}
        </ul>
      ) : (
        <ul className="cue-list memory">
          {memory.length === 0 && <li className="cue-empty">{loaded ? `No memory cues. MEMORY (${shift}M) saves one.` : ""}</li>}
          {memory.map((cue) => (
            <li key={cue.id} className="cue-item">
              <button
                type="button"
                tabIndex={-1}
                className="cue-row"
                title="Jump here and pause"
                onMouseDown={noFocus}
                onClick={() => act.memoryCue(deck, cue.id)}
              >
                <span className="mem-marker" style={{ borderTopColor: cue.color }} />
                <span className="cue-time">{formatClock(cue.start_us)}</span>
                <span className="cue-note">{cue.comment ?? (cue.end_us != null ? loopLabel(cue.start_us, cue.end_us, state.original_bpm) : "")}</span>
              </button>
              <DeleteButton title={`Delete this memory ${cue.end_us != null ? "loop" : "cue"}`} onDelete={() => act.memoryCueDelete(deck, cue.id)} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function loopLabel(start: number, end: number, bpm: number | null): string {
  const seconds = (end - start) / 1e6;
  if (bpm) {
    const beats = Math.round(((seconds * bpm) / 60) * 100) / 100;
    return `Loop ${beats} beats`;
  }
  return `Loop ${seconds.toFixed(2)} s`;
}
