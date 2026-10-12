// The DUAL PLAYER's mixer column (research §1.3): per deck A-GAIN with its dB
// readout and HIGH/MID/LOW kills, and the vertical crossfader (A on top).
// TuneOx's extras (EQ knobs, channel faders, master volume) live behind the
// "Mixer" toggle.

import { mixerSet } from "../../decks/actions";
import { useMixer, useTrackData, useDeck } from "../../decks/hooks";
import type { DeckId } from "../../decks/types";
import { DeckButton, Fader, Knob } from "./controls";

/** A-GAIN range: what the spec lets a track's gain apply. */
const GAIN_DB = 12;

/** rekordbox's readout: `+%2.1fdB` / `%2.1fdB`. */
export function formatGain(db: number): string {
  const rounded = Math.round(db * 10) / 10;
  return `${rounded >= 0 ? "+" : ""}${(rounded === 0 ? 0 : rounded).toFixed(1)}dB`;
}

export function ChannelStrip({ deck }: { deck: DeckId }) {
  const mixer = useMixer();
  const { state } = useDeck(deck);
  const data = useTrackData(state.track_id);
  const channel = mixer.channels[deck - 1];
  const trackGain = Math.max(-GAIN_DB, Math.min(GAIN_DB, data?.details?.track.auto_gain_db ?? 0));
  const kill = (band: "kill_high" | "kill_mid" | "kill_low", label: string) => (
    <DeckButton className="kill" active={channel[band]} title={`Kill the ${label.toLowerCase()} band`} onClick={() => mixerSet({ deck, [band]: !channel[band] })}>
      {label}
    </DeckButton>
  );
  return (
    <div className={`strip deck-${deck}`} data-deck={deck}>
      <Knob
        className="gain"
        value={channel.auto_gain_db}
        min={-GAIN_DB}
        max={GAIN_DB}
        bipolar
        size={34}
        title="A-GAIN: drag to trim, double-click for the track's own gain"
        onChange={(db) => mixerSet({ deck, auto_gain_db: Math.round(db * 10) / 10 })}
        onReset={() => mixerSet({ deck, auto_gain_db: Math.round(trackGain * 10) / 10 })}
      />
      <span className="gain-readout">{formatGain(channel.auto_gain_db)}</span>
      <span className="strip-label">A-GAIN</span>
      <div className="kills">
        {kill("kill_high", "HIGH")}
        {kill("kill_mid", "MID")}
        {kill("kill_low", "LOW")}
      </div>
    </div>
  );
}

export function Crossfader() {
  const mixer = useMixer();
  return (
    <div className="xfader">
      <span className="xfader-label">A</span>
      <Fader
        className="xfader-fader"
        value={mixer.crossfader}
        min={-1}
        max={1}
        inverted
        title="Crossfader: double-click to centre"
        onChange={(v) => mixerSet({ crossfader: Math.abs(v) < 0.03 ? 0 : Math.round(v * 1000) / 1000 })}
        onReset={() => mixerSet({ crossfader: 0 })}
      >
        <div className="xfader-center" />
      </Fader>
      <span className="xfader-label">B</span>
    </div>
  );
}

// EQ knob position −1…1 ↔ dB: the left half is linear gain down to a kill
// (−80 dB = −∞), the right half up to +6 dB.
export function eqToKnob(db: number): number {
  return db >= 0 ? db / 6 : Math.pow(10, db / 20) - 1;
}

export function knobToEq(v: number): number {
  if (v >= 0) return Math.round(v * 6 * 10) / 10;
  const gain = 1 + v;
  return gain <= 0.0001 ? -80 : Math.max(-80, Math.round(20 * Math.log10(gain) * 10) / 10);
}

function formatEq(db: number): string {
  return db <= -80 ? "−∞ dB" : `${db > 0 ? "+" : ""}${db.toFixed(1)} dB`;
}

function ExtrasChannel({ deck }: { deck: DeckId }) {
  const mixer = useMixer();
  const channel = mixer.channels[deck - 1];
  const eq = (band: "eq_high" | "eq_mid" | "eq_low", label: string) => (
    <Knob
      className="eq"
      size={26}
      bipolar
      label={label}
      value={eqToKnob(channel[band])}
      min={-1}
      max={1}
      title={`${label} ${formatEq(channel[band])} · double-click for 0 dB`}
      onChange={(v) => mixerSet({ deck, [band]: knobToEq(v) })}
      onReset={() => mixerSet({ deck, [band]: 0 })}
    />
  );
  return (
    <div className={`extras-channel deck-${deck}`}>
      <div className="eqs">
        {eq("eq_high", "HI")}
        {eq("eq_mid", "MID")}
        {eq("eq_low", "LOW")}
      </div>
      <Fader
        className="channel-fader"
        value={channel.volume}
        min={0}
        max={1}
        title={`Channel fader ${Math.round(channel.volume * 100)} % · double-click for full`}
        onChange={(v) => mixerSet({ deck, volume: Math.round(v * 1000) / 1000 })}
        onReset={() => mixerSet({ deck, volume: 1 })}
      />
    </div>
  );
}

/** TuneOx extras beyond the DUAL PLAYER. */
export function MixerExtras() {
  const mixer = useMixer();
  return (
    <div className="mix-extras">
      <ExtrasChannel deck={1} />
      <div className="master">
        <span className="strip-label">MASTER {Math.round(mixer.master_volume * 100)}%</span>
        <Fader
          className="master-fader"
          orientation="horizontal"
          value={mixer.master_volume}
          min={0}
          max={1}
          title="Master volume"
          onChange={(v) => mixerSet({ master_volume: Math.round(v * 1000) / 1000 })}
        />
      </div>
      <ExtrasChannel deck={2} />
    </div>
  );
}
