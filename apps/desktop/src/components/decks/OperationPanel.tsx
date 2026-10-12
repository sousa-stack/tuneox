// The operation column (research §1.3): beat jump with its size, CUE and
// PLAY/PAUSE per deck, and DUAL CONTROL between the decks.

import * as act from "../../decks/actions";
import { JUMP_SIZES, type JumpSizeId } from "../../decks/grid";
import { useDeck, useSettings } from "../../decks/hooks";
import { settings as settingsStore } from "../../decks/settings";
import type { DeckId } from "../../decks/types";
import { DeckButton, HoldButton } from "./controls";

export function OperationPanel({ deck }: { deck: DeckId }) {
  const { state, atCue } = useDeck(deck);
  const settings = useSettings();
  const loaded = state.track_id != null;
  const shift = deck === 2 ? "⇧" : "";
  return (
    <div className={`op-panel deck-${deck}`} data-deck={deck}>
      <div className="jump">
        <DeckButton className="arrow" disabled={!loaded} title={`Beat jump back (${shift}←)`} onClick={() => act.beatJump(deck, -1)}>
          ‹
        </DeckButton>
        <DeckButton className="arrow" disabled={!loaded} title={`Beat jump forward (${shift}→)`} onClick={() => act.beatJump(deck, 1)}>
          ›
        </DeckButton>
      </div>
      <select
        className="jump-size"
        tabIndex={-1}
        value={settings.jump[deck - 1]}
        title="Beat jump size"
        onChange={(e) => {
          settingsStore.updateDeck("jump", deck, e.target.value as JumpSizeId);
          e.target.blur();
        }}
      >
        {JUMP_SIZES.map((size) => (
          <option key={size.id} value={size.id}>
            {size.label}
          </option>
        ))}
      </select>
      <HoldButton
        className={`round cue${atCue ? " lit" : ""}`}
        disabled={!loaded}
        title={`CUE (${shift}C)`}
        onPress={() => act.cuePress(deck)}
        onRelease={() => act.cueRelease(deck)}
      >
        CUE
      </HoldButton>
      <DeckButton className="round play" disabled={!loaded} active={state.playing} title={`Play/pause (${shift}Space)`} onClick={() => act.playPause(deck)}>
        <svg viewBox="0 0 26 14" width="26" height="14" aria-hidden>
          <path d="M1 1 L10 7 L1 13 Z" />
          <rect x="15" y="1" width="3.2" height="12" />
          <rect x="21" y="1" width="3.2" height="12" />
        </svg>
      </DeckButton>
    </div>
  );
}

export function DualControl() {
  const settings = useSettings();
  return (
    <DeckButton
      className="dual-control"
      active={settings.dualControl}
      title="DUAL CONTROL: PLAY, CUE and beat jump act on both decks"
      onClick={() => settingsStore.update((s) => ({ dualControl: !s.dualControl }))}
    >
      <svg viewBox="0 0 20 20" width="18" height="18" aria-hidden>
        <rect x="2.5" y="2.5" width="10" height="10" rx="1.5" />
        <rect x="7.5" y="7.5" width="10" height="10" rx="1.5" />
      </svg>
    </DeckButton>
  );
}
