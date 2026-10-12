// One deck's track information row, overview and function row
// (research/rekordbox-decks.md §1.3).

import { useEffect, useRef, useState } from "react";
import * as act from "../../decks/actions";
import { formatBeats } from "../../decks/grid";
import { useDeck, useSettings, useTrackData } from "../../decks/hooks";
import { animator } from "../../decks/render/animator";
import { deckLetter, type DeckId, type DeckState } from "../../decks/types";
import { camelot } from "../../model";
import { DeckButton, HoldButton } from "./controls";

export function DeckHeader({ deck }: { deck: DeckId }) {
  const { state } = useDeck(deck);
  return (
    <div className={`deck-head deck-${deck}`} data-deck={deck}>
      <InfoRow deck={deck} state={state} />
      <Overview deck={deck} />
      <FunctionRow deck={deck} state={state} />
    </div>
  );
}

function InfoRow({ deck, state }: { deck: DeckId; state: DeckState }) {
  const data = useTrackData(state.track_id);
  const track = data?.details?.track.id === state.track_id ? data.details.track : null;
  const loaded = state.track_id != null;
  const bpm = state.bpm ?? track?.bpm ?? null;
  const syncTitle =
    state.sync_mode === "bpm"
      ? "BPM sync (tempo only). Turn SYNC off and on to sync beats again."
      : state.sync_mode === "beat"
        ? "Beat sync: tempo and phase follow the master deck"
        : "Sync tempo and beats to the master deck (F1)";
  return (
    <div className="deck-info">
      <span className="deck-badge">{deckLetter(deck)}</span>
      <div className="deck-title">
        {loaded ? (
          <>
            <div className="title" title={track?.title}>
              {track?.title ?? "Loading…"}
            </div>
            <div className="artist" title={track?.artist ?? undefined}>
              {state.error ? <span className="deck-error">{state.error}</span> : (track?.artist ?? "")}
              {state.loading && !state.error && <span className="deck-loading">Loading audio…</span>}
            </div>
          </>
        ) : (
          <>
            <div className="title muted">No track loaded</div>
            <div className="artist faint">Drag a track here, or right-click or double-click one</div>
          </>
        )}
      </div>
      <div className="deck-bpm" title={state.original_bpm != null ? `Track ${state.original_bpm.toFixed(2)} BPM` : undefined}>
        <span className="bpm">{loaded && bpm != null ? bpm.toFixed(2) : "--.--"}</span>
        <span className="unit">BPM</span>
        <span className="key" title={track?.key ?? undefined}>
          {camelot(track?.key ?? null) ?? ""}
        </span>
      </div>
      <TimeDisplay deck={deck} loaded={loaded} />
      <div className="deck-sync">
        <DeckButton
          className={`sync-btn ${state.sync_mode}`}
          active={state.sync_mode !== "off"}
          disabled={!loaded}
          title={syncTitle}
          onClick={() => act.toggleSync(deck)}
        >
          BEAT SYNC
        </DeckButton>
        <DeckButton className="master-btn" active={state.master} disabled={!loaded} title="Make this deck the sync master" onClick={() => act.setMaster(deck)}>
          MASTER
        </DeckButton>
      </div>
    </div>
  );
}

function TimeDisplay({ deck, loaded }: { deck: DeckId; loaded: boolean }) {
  const big = useRef<HTMLSpanElement>(null);
  const small = useRef<HTMLSpanElement>(null);
  useEffect(() => animator.attachTime(deck, big.current!, small.current!), [deck]);
  return (
    <div className={`deck-time${loaded ? "" : " empty"}`} title="T (⇧T for deck B) switches remaining and elapsed time" onClick={() => act.toggleTimeMode(deck)}>
      <span ref={big} className="time-big" />
      <span ref={small} className="time-small" />
    </div>
  );
}

function Overview({ deck }: { deck: DeckId }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const seeking = useRef(false);
  useEffect(() => animator.attachOverview(deck, canvas.current!), [deck]);
  const seekTo = (e: React.PointerEvent<HTMLCanvasElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const state = act.deckState(deck);
    if (state.track_id == null || state.duration_us <= 0) return;
    const fraction = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
    act.seekTo(deck, fraction * state.duration_us);
  };
  return (
    <canvas
      ref={canvas}
      className="overview"
      title="Click to jump; playback carries on"
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        seeking.current = true;
        seekTo(e);
      }}
      onPointerMove={(e) => seeking.current && seekTo(e)}
      onPointerUp={() => (seeking.current = false)}
      onPointerCancel={() => (seeking.current = false)}
    />
  );
}

function FunctionRow({ deck, state }: { deck: DeckId; state: DeckState }) {
  const settings = useSettings();
  const size = settings.loopBeats[deck - 1];
  const loaded = state.track_id != null;
  const loopActive = state.loop != null;
  const sizeActive = loopActive && state.loop!.beats != null && Math.abs(state.loop!.beats - size) < 1e-6;
  const percent = state.tempo_percent;
  return (
    <div className="deck-func">
      <DeckButton className="txt" disabled title="Saving memory cues comes with cue editing">
        MEMORY
      </DeckButton>
      <div className="group">
        <DeckButton className="txt" disabled={!loaded} active={state.loop_in_us != null && !loopActive} title="Loop in (I)" onClick={() => act.loopIn(deck)}>
          IN
        </DeckButton>
        <DeckButton className="txt" disabled={!loaded} title="Loop out (O)" onClick={() => act.loopOut(deck)}>
          OUT
        </DeckButton>
        <DeckButton
          className="txt reloop"
          disabled={!loaded || (!loopActive && !state.last_loop)}
          active={loopActive}
          title="Exit the loop, or reloop (R)"
          onClick={() => act.reloopExit(deck)}
        >
          {loopActive ? "EXIT" : "RELOOP"}
        </DeckButton>
      </div>
      <div className="group autoloop">
        <DeckButton className="arrow" disabled={!loaded} title="Halve the loop size" onClick={() => act.loopSize(deck, "half")}>
          ‹
        </DeckButton>
        <DeckButton className="loop-size" disabled={!loaded} active={sizeActive} title={`${formatBeats(size)}-beat loop (4–9 for 1–32 beats)`} onClick={() => act.autoLoop(deck)}>
          {formatBeats(size)}
        </DeckButton>
        <DeckButton className="arrow" disabled={!loaded} title="Double the loop size" onClick={() => act.loopSize(deck, "double")}>
          ›
        </DeckButton>
      </div>
      <div className="group tempo">
        <HoldButton className="arrow" disabled={!loaded} title="Tempo −0.02 % (hold: −0.1 %) (F6)" onPress={() => act.tempoStepPress(deck, -1)} onRelease={() => act.tempoStepRelease(deck)}>
          −
        </HoldButton>
        <BpmField deck={deck} state={state} />
        <HoldButton className="arrow" disabled={!loaded} title="Tempo +0.02 % (hold: +0.1 %) (F7)" onPress={() => act.tempoStepPress(deck, 1)} onRelease={() => act.tempoStepRelease(deck)}>
          +
        </HoldButton>
        <span className={`tempo-pct${percent !== 0 ? " shifted" : ""}`}>
          {percent > 0 ? "+" : percent < 0 ? "−" : "±"}
          {Math.abs(percent).toFixed(2)}%
        </span>
      </div>
      <div className="group">
        <HoldButton
          className="arrow bend"
          disabled={!loaded}
          active={state.pitch_bend === "back"}
          title="Pitch bend: slow down while held (F4)"
          onPress={() => act.pitchBend(deck, "back", "press")}
          onRelease={() => act.pitchBend(deck, "back", "release")}
        >
          ◀◀
        </HoldButton>
        <HoldButton
          className="arrow bend"
          disabled={!loaded}
          active={state.pitch_bend === "forward"}
          title="Pitch bend: speed up while held (F5)"
          onPress={() => act.pitchBend(deck, "forward", "press")}
          onRelease={() => act.pitchBend(deck, "forward", "release")}
        >
          ▶▶
        </HoldButton>
      </div>
      <DeckButton className="txt" disabled={!loaded} title="Tempo reset (F3)" onClick={() => act.tempoReset(deck)}>
        RST
      </DeckButton>
      <DeckButton className="txt q" active={state.quantize} title="Quantize (Q)" onClick={() => act.toggleQuantize(deck)}>
        Q
      </DeckButton>
    </div>
  );
}

/** The BPM between − and +; double-click to type one. */
function BpmField({ deck, state }: { deck: DeckId; state: DeckState }) {
  const [editing, setEditing] = useState<string | null>(null);
  const loaded = state.track_id != null && state.bpm != null;
  if (editing !== null) {
    const commit = () => {
      const bpm = parseFloat(editing.replace(",", "."));
      setEditing(null);
      if (Number.isFinite(bpm)) act.setBpm(deck, bpm);
    };
    return (
      <input
        className="bpm-input"
        autoFocus
        value={editing}
        inputMode="decimal"
        onFocus={(e) => e.target.select()}
        onChange={(e) => setEditing(e.target.value)}
        onBlur={() => setEditing(null)}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit();
          if (e.key === "Escape") setEditing(null);
          e.stopPropagation();
        }}
      />
    );
  }
  return (
    <span
      className={`bpm-field${loaded ? "" : " empty"}`}
      title="Double-click to type a BPM"
      onDoubleClick={() => loaded && setEditing(state.bpm!.toFixed(2))}
    >
      {loaded ? state.bpm!.toFixed(2) : "--.--"}
    </span>
  );
}
