// Typed deck, mixer and waveform commands (spec part C) and the engine's
// events. Everything goes through `run_command`, like the rest of the UI.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { run } from "../api";
import type {
  Beat,
  BendDirection,
  ButtonAction,
  DeckEvent,
  DeckId,
  DeckState,
  MixerState,
  PlayerStatus,
  PreviewEvent,
  PreviewState,
  WaveformResolution,
  WaveformResponse,
  WaveformStyle,
} from "./types";

export interface MixerInput {
  deck?: DeckId;
  volume?: number;
  eq_low?: number;
  eq_mid?: number;
  eq_high?: number;
  kill_low?: boolean;
  kill_mid?: boolean;
  kill_high?: boolean;
  auto_gain_db?: number;
  crossfader?: number;
  master_volume?: number;
}

const deck = (name: string) => (d: DeckId) => run<DeckState>(name, { deck: d });

export const commands = {
  load: (d: DeckId, trackId: number) => run<DeckState>("deck_load", { deck: d, track_id: trackId }),
  play: deck("deck_play"),
  pause: deck("deck_pause"),
  cue: (d: DeckId, action: ButtonAction) => run<DeckState>("deck_cue", { deck: d, action }),
  hotCue: (d: DeckId, slot: number, action: ButtonAction) => run<DeckState>("deck_hot_cue", { deck: d, slot, action }),
  hotCueDelete: (d: DeckId, slot: number) => run<DeckState>("deck_hot_cue_delete", { deck: d, slot }),
  memoryCue: (d: DeckId, target: number | "next" | "prev") =>
    run<DeckState>("deck_memory_cue", { deck: d, action: "call", target }),
  memoryCueSet: deck("deck_memory_cue_set"),
  memoryCueDelete: (d: DeckId, index: number) => run<DeckState>("deck_memory_cue_delete", { deck: d, index }),
  seek: (d: DeckId, positionUs: number) => run<DeckState>("deck_seek", { deck: d, position_us: Math.round(positionUs) }),
  scrub: (d: DeckId, action: "start" | "move" | "end", positionUs?: number) =>
    run<DeckState>("deck_scrub", {
      deck: d,
      action,
      ...(positionUs === undefined ? {} : { position_us: Math.max(0, Math.round(positionUs)) }),
    }),
  previewPlay: (trackId: number, positionUs: number) =>
    run<PreviewState>("preview_play", { track_id: trackId, position_us: Math.max(0, Math.round(positionUs)) }),
  previewStop: () => run<PreviewState>("preview_stop"),
  beatJump: (d: DeckId, beats: number) => run<DeckState>("deck_beat_jump", { deck: d, beats }),
  loop: (d: DeckId, beats: number) => run<DeckState>("deck_loop", { deck: d, beats }),
  loopIn: deck("deck_loop_in"),
  loopOut: deck("deck_loop_out"),
  reloop: deck("deck_reloop"),
  loopExit: deck("deck_loop_exit"),
  loopResize: (d: DeckId, factor: "double" | "half") => run<DeckState>("deck_loop_resize", { deck: d, factor }),
  setTempo: (d: DeckId, percent: number) => run<DeckState>("deck_set_tempo", { deck: d, percent }),
  setBpm: (d: DeckId, bpm: number) => run<DeckState>("deck_set_bpm", { deck: d, bpm }),
  tempoReset: deck("deck_tempo_reset"),
  pitchBend: (d: DeckId, direction: BendDirection, action: ButtonAction) =>
    run<DeckState>("deck_pitch_bend", { deck: d, direction, action }),
  sync: (d: DeckId, on: boolean) => run<DeckState>("deck_sync", { deck: d, on }),
  setMaster: deck("deck_set_master"),
  setQuantize: (d: DeckId, on: boolean) => run<DeckState>("deck_set_quantize", { deck: d, on }),
  mixer: (input: MixerInput) => run<MixerState>("mixer_set", input),
  status: () => run<PlayerStatus>("player_status"),
  quantizeValue: (beats: number) => run<PlayerStatus>("player_set_quantize_value", { beats }),
  syncPhase: (unit: "beat" | "bar") => run<PlayerStatus>("player_set_sync_phase", { unit }),
  waveform: (trackId: number, style: WaveformStyle, resolution: WaveformResolution) =>
    run<WaveformResponse>("get_waveform", { track_id: trackId, style, resolution }),
  beatGrid: (trackId: number) => run<{ track_id: number; beats: Beat[] }>("get_beat_grid", { track_id: trackId }),
};

export function onDeckEvent(handler: (event: DeckEvent) => void): Promise<UnlistenFn> {
  return listen<DeckEvent>("tuneox://deck", (event) => handler(event.payload));
}

export function onMixerEvent(handler: (mixer: MixerState) => void): Promise<UnlistenFn> {
  return listen<MixerState>("tuneox://mixer", (event) => handler(event.payload));
}

export function onPreviewEvent(handler: (event: PreviewEvent) => void): Promise<UnlistenFn> {
  return listen<PreviewEvent>("tuneox://preview", (event) => handler(event.payload));
}
