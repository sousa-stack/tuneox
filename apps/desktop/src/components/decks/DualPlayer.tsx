// rekordbox's DUAL PLAYER (research/rekordbox-decks.md §1.2): deck A on top,
// deck B below, their enlarged waveforms stacked in the middle facing each
// other. Columns: operation | mixer | (TuneOx mixer extras) | decks | cue lists.

import { memo, useEffect, useRef, useSyncExternalStore } from "react";
import { dragStore } from "../../decks/drag";
import { useSettings } from "../../decks/hooks";
import { animator } from "../../decks/render/animator";
import { CuePanel } from "./CuePanel";
import { DeckHeader } from "./DeckHeader";
import { DetailWaveforms } from "./DetailWaveforms";
import { ChannelStrip, Crossfader, MixerExtras } from "./MixerPanel";
import { DualControl, OperationPanel } from "./OperationPanel";

export const DualPlayer = memo(function DualPlayer() {
  const settings = useSettings();
  const dropTarget = useSyncExternalStore(dragStore.subscribe, dragStore.get);
  const dropClass = dropTarget === null ? "" : ` dragging drop-${dropTarget}`;
  return (
    <section className={`dual${dropClass}`} aria-label="Dual player">
      <div className="col op-col">
        <OperationPanel deck={1} />
        <DualControl />
        <OperationPanel deck={2} />
      </div>
      <div className="col mix-col">
        <ChannelStrip deck={1} />
        <Crossfader />
        <ChannelStrip deck={2} />
      </div>
      {settings.mixerOpen && (
        <div className="col extras-col">
          <MixerExtras />
        </div>
      )}
      <div className="col deck-col">
        <DeckHeader deck={1} />
        <DetailWaveforms />
        <DeckHeader deck={2} />
      </div>
      <div className="col sub-col">
        <CuePanel deck={1} />
        <CuePanel deck={2} />
      </div>
      {settings.hud && <PerfHud />}
    </section>
  );
});

function PerfHud() {
  const element = useRef<HTMLDivElement>(null);
  useEffect(() => animator.attachHud(element.current!), []);
  return <div ref={element} className="perf-hud" title="⌥⇧P hides this" />;
}
