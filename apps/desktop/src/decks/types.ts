// JSON shapes of the deck, mixer and waveform commands (spec part C,
// docs/specs/decks.md). Mirrors crates/tuneox-audio/src/state.rs.

export type DeckId = 1 | 2;
export const DECKS: readonly DeckId[] = [1, 2];

export type SyncMode = "off" | "beat" | "bpm";
export type BendDirection = "back" | "forward";
export type ButtonAction = "press" | "release";

export interface LoopState {
  start_us: number;
  end_us: number;
  /** Length in beats on the grid, if the track has one. */
  beats: number | null;
}

export interface HotCueState {
  /** 0 = A … 7 = H. */
  slot: number;
  start_us: number;
  /** End of a hot loop. */
  end_us: number | null;
}

export interface MemoryCueState {
  /** 0 = the earliest; what deck_memory_cue calls. */
  index: number;
  start_us: number;
  end_us: number | null;
}

export interface DeckState {
  deck: DeckId;
  track_id: number | null;
  loading: boolean;
  error: string | null;
  playing: boolean;
  /** Where the deck was (as heard) at `at_ms`. */
  position_us: number;
  /** Engine clock, monotonic ms. */
  at_ms: number;
  /** Track µs per real µs, including a held pitch bend; 0 when paused. */
  rate: number;
  duration_us: number;
  tempo_percent: number;
  tempo_range: number;
  bpm: number | null;
  original_bpm: number | null;
  sync_mode: SyncMode;
  master: boolean;
  quantize: boolean;
  pitch_bend: BendDirection | null;
  cue_us: number | null;
  loop: LoopState | null;
  last_loop: LoopState | null;
  loop_in_us: number | null;
  hot_cues: HotCueState[];
  memory_cues: MemoryCueState[];
}

/** A `tuneox://deck` event: the state plus the engine clock at send time. */
export interface DeckEvent extends DeckState {
  sent_at_ms: number;
}

export interface ChannelState {
  deck: DeckId;
  volume: number;
  eq_low: number;
  eq_mid: number;
  eq_high: number;
  kill_low: boolean;
  kill_mid: boolean;
  kill_high: boolean;
  auto_gain_db: number;
}

export interface MixerState {
  channels: [ChannelState, ChannelState];
  /** −1 = deck 1 only … 1 = deck 2 only. */
  crossfader: number;
  master_volume: number;
}

export interface OutputInfo {
  device: string | null;
  sample_rate: number;
  channels: number;
  latency_ms: number;
  error: string | null;
}

export interface PlayerStatus {
  running: boolean;
  decks: [DeckState, DeckState];
  mixer: MixerState;
  hot_cue_gate: boolean;
  output: OutputInfo | null;
}

export type WaveformStyle = "blue" | "rgb" | "three_band";
export type WaveformResolution = "preview" | "detail";

export interface BandGains {
  low: number;
  mid: number;
  high: number;
}

/** `get_waveform` output. */
export interface WaveformResponse {
  track_id: number;
  style: WaveformStyle;
  resolution: WaveformResolution;
  columns: number;
  columns_per_second: number | null;
  bytes_per_column: number;
  data_base64: string;
  band_gains: BandGains | null;
}

/** One beat of `get_beat_grid`. */
export interface Beat {
  time_us: number;
  /** 1 = downbeat … 4. */
  bar_position: number;
  /** BPM × 100. */
  bpm_centi: number;
}

export function emptyDeck(deck: DeckId): DeckState {
  return {
    deck,
    track_id: null,
    loading: false,
    error: null,
    playing: false,
    position_us: 0,
    at_ms: 0,
    rate: 0,
    duration_us: 0,
    tempo_percent: 0,
    tempo_range: 10,
    bpm: null,
    original_bpm: null,
    sync_mode: "off",
    master: false,
    quantize: true,
    pitch_bend: null,
    cue_us: null,
    loop: null,
    last_loop: null,
    loop_in_us: null,
    hot_cues: [],
    memory_cues: [],
  };
}

export function emptyMixer(): MixerState {
  const channel = (deck: DeckId): ChannelState => ({
    deck,
    volume: 1,
    eq_low: 0,
    eq_mid: 0,
    eq_high: 0,
    kill_low: false,
    kill_mid: false,
    kill_high: false,
    auto_gain_db: 0,
  });
  return { channels: [channel(1), channel(2)], crossfader: 0, master_volume: 1 };
}

export const deckLetter = (deck: DeckId) => (deck === 1 ? "A" : "B");
