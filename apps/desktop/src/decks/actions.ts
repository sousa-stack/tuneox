// What the deck controls and keys do. Every action updates the screen at once
// (an optimistic patch where the outcome is certain), sends the command, and
// replaces the guess with the engine's state when the reply lands. Errors
// show as toasts; nothing waits on the engine.

import { positionAt, tempoFactor } from "./clock";
import { commands, type MixerInput } from "./commands";
import { memoryCueAt, withoutHotCue, withoutMemoryCue } from "./cues";
import { FINE_JUMP_US, LOOP_SIZES, jumpBeats } from "./grid";
import { settings, ZOOM_SPANS, DEFAULT_ZOOM, type QuantizeValue, type SyncPhase } from "./settings";
import { store } from "./store";
import { toast } from "./toast";
import type { BendDirection, DeckId, DeckState, MixerState } from "./types";

const engineNow = () => store.clock.engineNow(performance.now());

export function currentPosition(deck: DeckId): number {
  return positionAt(store.deck(deck), engineNow());
}

/** A deck's state right now, optimistic changes included. */
export function deckState(deck: DeckId): DeckState {
  return store.deck(deck);
}

const loaded = (deck: DeckId) => store.deck(deck).track_id != null;

/** The decks a transport action goes to: both with DUAL CONTROL on. */
function targets(deck: DeckId): DeckId[] {
  if (!settings.get().dualControl) return [deck];
  return ([1, 2] as DeckId[]).filter((d) => d === deck || loaded(d));
}

function send(deck: DeckId, request: Promise<DeckState>, patch?: Partial<DeckState>): Promise<void> {
  const id = patch ? store.patch(deck, patch) : 0;
  return request
    .then((state) => store.applyDeck(state))
    .catch(toast)
    .finally(() => id && store.unpatch(deck, id));
}

// ── Loading ─────────────────────────────────────────────────────────────────

export function loadTrack(deck: DeckId, trackId: number): void {
  send(deck, commands.load(deck, trackId), {
    track_id: trackId,
    loading: true,
    error: null,
    playing: false,
    rate: 0,
    position_us: 0,
    cue_us: null,
    loop: null,
    last_loop: null,
    loop_in_us: null,
    hot_cues: [],
    memory_cues: [],
  });
}

/** Double-click in the browser: deck A unless it's playing, else deck B. */
export function loadToFreeDeck(trackId: number): void {
  loadTrack(store.deck(1).playing ? 2 : 1, trackId);
}

// ── Transport ───────────────────────────────────────────────────────────────

function play(deck: DeckId) {
  const state = store.deck(deck);
  if (state.track_id == null) return;
  send(deck, commands.play(deck), {
    playing: true,
    rate: state.loading && state.duration_us === 0 ? 0 : tempoFactor(state.tempo_percent),
    position_us: currentPosition(deck),
    at_ms: engineNow(),
  });
}

function pause(deck: DeckId) {
  if (!loaded(deck)) return;
  send(deck, commands.pause(deck), { playing: false, rate: 0, position_us: currentPosition(deck), at_ms: engineNow() });
}

export function playPause(deck: DeckId): void {
  const playing = store.deck(deck).playing;
  for (const d of targets(deck)) (playing ? pause : play)(d);
}

export function cuePress(deck: DeckId): void {
  for (const d of targets(deck)) if (loaded(d)) send(d, commands.cue(d, "press"));
}

export function cueRelease(deck: DeckId): void {
  for (const d of targets(deck)) if (loaded(d)) send(d, commands.cue(d, "release"));
}

export function beatJump(deck: DeckId, direction: -1 | 1): void {
  const beats = jumpBeats(settings.get().jump[deck - 1]);
  for (const d of targets(deck)) {
    if (!loaded(d)) continue;
    if (beats === 0) seekTo(d, currentPosition(d) + direction * FINE_JUMP_US);
    else send(d, commands.beatJump(d, direction * beats));
  }
}

export function hotCuePress(deck: DeckId, slot: number): void {
  if (loaded(deck)) send(deck, commands.hotCue(deck, slot, "press"));
}

export function hotCueRelease(deck: DeckId, slot: number): void {
  if (loaded(deck)) send(deck, commands.hotCue(deck, slot, "release"));
}

export function memoryCue(deck: DeckId, target: number | "next" | "prev"): void {
  const state = store.deck(deck);
  if (state.track_id == null) return;
  if (typeof target === "number" && target >= state.memory_cues.length) return;
  send(deck, commands.memoryCue(deck, target));
}

// ── Cue edits (saved to the track in the library by the engine side) ───────

/** ✕ on a hot cue row, ⌘1–3: clears the pad. */
export function hotCueDelete(deck: DeckId, slot: number): void {
  const state = store.deck(deck);
  if (state.track_id == null || !state.hot_cues.some((c) => c.slot === slot)) return;
  send(deck, commands.hotCueDelete(deck, slot), withoutHotCue(state, slot));
}

/** MEMORY / M: saves the cue point, or the current or last loop, as a memory cue. */
export function memoryCueSet(deck: DeckId): void {
  if (loaded(deck)) send(deck, commands.memoryCueSet(deck));
}

/** ✕ on a memory cue row. */
export function memoryCueDelete(deck: DeckId, index: number): void {
  const state = store.deck(deck);
  if (state.track_id == null || index >= state.memory_cues.length) return;
  send(deck, commands.memoryCueDelete(deck, index), withoutMemoryCue(state, index));
}

/** X: deletes the memory cue the deck is parked on (call one first, as in rekordbox). */
export function memoryCueDeleteHere(deck: DeckId): void {
  const state = store.deck(deck);
  if (state.track_id == null) return;
  const index = memoryCueAt(state, currentPosition(deck));
  if (index == null) {
    const letter = deck === 1 ? "A" : "B";
    toast(`Deck ${letter} isn't on a memory cue. Call one first (A–; or N/B), then X deletes it.`);
    return;
  }
  memoryCueDelete(deck, index);
}

// Seeks coalesce: while one is in flight only the latest target waits, so a
// fast drag never queues up stale jumps.
const seeks: { busy: boolean; next: number | null; patch: number }[] = [
  { busy: false, next: null, patch: 0 },
  { busy: false, next: null, patch: 0 },
];

export function seekTo(deck: DeckId, positionUs: number): void {
  const state = store.deck(deck);
  if (state.track_id == null) return;
  const target = Math.max(0, state.duration_us > 0 ? Math.min(positionUs, state.duration_us) : positionUs);
  const s = seeks[deck - 1];
  if (s.patch) store.unpatch(deck, s.patch);
  // Leaving the loop ends it (engine rule); show that at once too.
  const leavesLoop = state.loop && (target < state.loop.start_us || target >= state.loop.end_us);
  s.patch = store.patch(deck, { position_us: target, at_ms: engineNow(), ...(leavesLoop ? { loop: null } : {}) });
  s.next = target;
  pumpSeek(deck);
}

function pumpSeek(deck: DeckId) {
  const s = seeks[deck - 1];
  if (s.busy || s.next == null) return;
  const target = s.next;
  const patch = s.patch;
  s.next = null;
  s.patch = 0;
  s.busy = true;
  commands
    .seek(deck, target)
    .then((state) => store.applyDeck(state))
    .catch(toast)
    .finally(() => {
      store.unpatch(deck, patch);
      s.busy = false;
      pumpSeek(deck);
    });
}

// ── Scrubbing ───────────────────────────────────────────────────────────────
// Dragging the enlarged waveform moves the deck like a record (deck_scrub).
// The waveform follows the pointer exactly (a patch on the shown state); the
// audio follows in the engine. Commands go one at a time and consecutive
// moves collapse to the latest, so a fast drag never queues up. While the
// pointer is down the position is re-sent every KEEP_ALIVE_MS: the engine lets
// go of a scrub it hasn't heard from for 1.5 s, which keeps a lost pointer-up
// from leaving the deck held.

const KEEP_ALIVE_MS = 250;

interface ScrubOp {
  action: "start" | "move" | "end";
  position?: number;
}

interface Scrub {
  ops: ScrubOp[];
  busy: boolean;
  /** The patch showing the dragged position. */
  patch: number;
  /** Stops waiting for the engine to settle after a drag. */
  settle: (() => void) | null;
  /** Where the hand holds the record, and the timer re-sending it. */
  target: number;
  keepAlive: ReturnType<typeof setInterval> | undefined;
}

const scrubs: [Scrub, Scrub] = [
  { ops: [], busy: false, patch: 0, settle: null, target: 0, keepAlive: undefined },
  { ops: [], busy: false, patch: 0, settle: null, target: 0, keepAlive: undefined },
];

function showScrubPosition(deck: DeckId, positionUs: number, rate = 0) {
  const s = scrubs[deck - 1];
  const previous = s.patch;
  s.patch = store.patch(deck, { position_us: positionUs, at_ms: engineNow(), rate });
  if (previous) store.unpatch(deck, previous);
}

function queueScrub(deck: DeckId, op: ScrubOp) {
  const s = scrubs[deck - 1];
  const last = s.ops[s.ops.length - 1];
  if (op.action === "move" && last?.action === "move") last.position = op.position;
  else s.ops.push(op);
  pumpScrub(deck);
}

function pumpScrub(deck: DeckId) {
  const s = scrubs[deck - 1];
  const op = s.busy ? undefined : s.ops.shift();
  if (!op) return;
  s.busy = true;
  commands
    .scrub(deck, op.action, op.position)
    .then((state) => store.applyDeck(state))
    .catch(toast)
    .finally(() => {
      s.busy = false;
      if (op.action === "end" && s.ops.length === 0) awaitSettle(deck);
      pumpScrub(deck);
    });
}

/** Keeps the dragged position on screen until the engine has settled or handed over to playback. */
function awaitSettle(deck: DeckId) {
  const s = scrubs[deck - 1];
  const patch = s.patch;
  const done = () => {
    stop();
    clearTimeout(timer);
    store.unpatch(deck, patch);
    if (s.patch === patch) s.patch = 0;
    if (s.settle === done) s.settle = null;
  };
  const stop = store.subscribeFrames((d) => d === deck && !store.deck(deck).scrubbing && done());
  const timer = setTimeout(done, 400);
  s.settle = done;
  if (!store.deck(deck).scrubbing) done();
}

/** Pointer down and moving on a lane: the hand goes on the record. Returns where it was. */
export function scrubStart(deck: DeckId): number {
  const state = store.deck(deck);
  if (state.track_id == null) return 0;
  const s = scrubs[deck - 1];
  s.settle?.();
  const position = currentPosition(deck);
  showScrubPosition(deck, position);
  queueScrub(deck, { action: "start", position });
  s.target = position;
  clearInterval(s.keepAlive);
  s.keepAlive = setInterval(() => {
    if (!loaded(deck)) clearInterval(s.keepAlive);
    else if (s.ops.length === 0) queueScrub(deck, { action: "move", position: s.target });
  }, KEEP_ALIVE_MS);
  return position;
}

export function scrubMove(deck: DeckId, positionUs: number): void {
  const state = store.deck(deck);
  if (state.track_id == null) return;
  const target = Math.max(0, state.duration_us > 0 ? Math.min(positionUs, state.duration_us) : positionUs);
  scrubs[deck - 1].target = target;
  showScrubPosition(deck, target);
  queueScrub(deck, { action: "move", position: target });
}

/** Pointer up: let go. A playing deck plays on from here; a paused one rests here. */
export function scrubEnd(deck: DeckId, positionUs: number): void {
  const s = scrubs[deck - 1];
  clearInterval(s.keepAlive);
  s.keepAlive = undefined;
  const state = store.deck(deck);
  if (state.track_id == null) return;
  const target = Math.max(0, state.duration_us > 0 ? Math.min(positionUs, state.duration_us) : positionUs);
  showScrubPosition(deck, target, state.playing ? tempoFactor(state.tempo_percent) : 0);
  queueScrub(deck, { action: "end", position: target });
}

// ── Loops ───────────────────────────────────────────────────────────────────

export function loopIn(deck: DeckId): void {
  if (loaded(deck)) send(deck, commands.loopIn(deck));
}

export function loopOut(deck: DeckId): void {
  if (loaded(deck)) send(deck, commands.loopOut(deck));
}

/** R / RELOOP-EXIT: exits an active loop, else returns to the last one. */
export function reloopExit(deck: DeckId): void {
  const state = store.deck(deck);
  if (state.track_id == null) return;
  if (state.loop) send(deck, commands.loopExit(deck), { loop: null });
  else send(deck, commands.reloop(deck));
}

export function beatLoop(deck: DeckId, beats: number): void {
  if (loaded(deck)) send(deck, commands.loop(deck, beats));
}

/** The auto loop button: loops the selected size (the same size again exits). */
export function autoLoop(deck: DeckId): void {
  beatLoop(deck, settings.get().loopBeats[deck - 1]);
}

/** `<` / `>`: halves or doubles the loop size, and an active loop with it. */
export function loopSize(deck: DeckId, factor: "half" | "double"): void {
  const current = settings.get().loopBeats[deck - 1];
  const index = LOOP_SIZES.indexOf(current);
  const next = LOOP_SIZES[Math.max(0, Math.min(LOOP_SIZES.length - 1, index + (factor === "double" ? 1 : -1)))];
  if (next !== undefined) settings.updateDeck("loopBeats", deck, next);
  if (store.deck(deck).loop) send(deck, commands.loopResize(deck, factor));
}

// ── Tempo and sync ──────────────────────────────────────────────────────────

export function toggleSync(deck: DeckId): void {
  const state = store.deck(deck);
  if (state.track_id == null) return;
  const on = state.sync_mode === "off";
  send(deck, commands.sync(deck, on), { sync_mode: on ? "beat" : "off" });
}

export function setMaster(deck: DeckId): void {
  if (loaded(deck)) send(deck, commands.setMaster(deck), { master: true });
}

/** The quantize resolution for both decks (persisted; sent to the engine). */
export function setQuantizeValue(beats: QuantizeValue): void {
  settings.update({ quantizeBeats: beats });
  commands.quantizeValue(beats).catch(toast);
}

/** What beat sync aligns: the beat, or (a TuneOx extra) the bar. */
export function setSyncPhase(unit: SyncPhase): void {
  settings.update({ syncPhase: unit });
  commands.syncPhase(unit).catch(toast);
}

/** At launch: hands the persisted player preferences to the engine. */
export function applyPlayerSettings(status: { quantize_beats: number; sync_phase: SyncPhase }): void {
  const s = settings.get();
  if (status.quantize_beats !== s.quantizeBeats) commands.quantizeValue(s.quantizeBeats).catch(toast);
  if (status.sync_phase !== s.syncPhase) commands.syncPhase(s.syncPhase).catch(toast);
}

export function toggleQuantize(deck: DeckId): void {
  const on = !store.deck(deck).quantize;
  send(deck, commands.setQuantize(deck, on), { quantize: on });
}

export function tempoReset(deck: DeckId): void {
  if (loaded(deck)) send(deck, commands.tempoReset(deck));
}

export function setBpm(deck: DeckId, bpm: number): void {
  if (loaded(deck)) send(deck, commands.setBpm(deck, bpm));
}

function stepTempo(deck: DeckId, delta: number) {
  const state = store.deck(deck);
  if (state.track_id == null) return;
  const range = state.tempo_range || 10;
  const percent = Math.max(-range, Math.min(range, Math.round((state.tempo_percent + delta) * 10000) / 10000));
  if (percent === state.tempo_percent) return;
  const factor = tempoFactor(percent);
  send(deck, commands.setTempo(deck, percent), {
    tempo_percent: percent,
    bpm: state.original_bpm != null ? state.original_bpm * factor : state.bpm,
  });
}

/** −/+ hold: 0.02 % per step, 0.1 % once held 500 ms (rekordbox's EXPORT players). */
const FINE_STEP = 0.02;
const COARSE_STEP = 0.1;
const COARSE_AFTER_MS = 500;
const REPEAT_DELAY_MS = 300;
const REPEAT_MS = 50;
const repeaters = new Map<DeckId, ReturnType<typeof setTimeout>>();

export function tempoStepPress(deck: DeckId, direction: -1 | 1): void {
  tempoStepRelease(deck);
  const start = performance.now();
  stepTempo(deck, direction * FINE_STEP);
  const tick = () => {
    const held = performance.now() - start;
    stepTempo(deck, direction * (held >= COARSE_AFTER_MS ? COARSE_STEP : FINE_STEP));
    repeaters.set(deck, setTimeout(tick, REPEAT_MS));
  };
  repeaters.set(deck, setTimeout(tick, REPEAT_DELAY_MS));
}

export function tempoStepRelease(deck: DeckId): void {
  const timer = repeaters.get(deck);
  if (timer !== undefined) clearTimeout(timer);
  repeaters.delete(deck);
}

export function pitchBend(deck: DeckId, direction: BendDirection, action: "press" | "release"): void {
  if (loaded(deck)) send(deck, commands.pitchBend(deck, direction, action));
}

// ── View ────────────────────────────────────────────────────────────────────

export function toggleTimeMode(deck: DeckId): void {
  settings.updateDeck("timeMode", deck, settings.get().timeMode[deck - 1] === "remaining" ? "elapsed" : "remaining");
}

export function zoom(direction: -1 | 0 | 1): void {
  settings.update((s) => ({
    zoom: direction === 0 ? DEFAULT_ZOOM : Math.max(0, Math.min(ZOOM_SPANS.length - 1, s.zoom - direction)),
  }));
}

// ── Mixer ───────────────────────────────────────────────────────────────────

type MixerGroup = "1" | "2" | "global";
interface MixerQueue {
  /** Sent, not yet answered. */
  inFlight: MixerInput | null;
  /** Waiting for the one in flight. */
  next: MixerInput | null;
}
const mixerQueues = new Map<MixerGroup, MixerQueue>();

function applyLocally(input: MixerInput): void {
  const m = store.mixer;
  const next: MixerState = { ...m, channels: [{ ...m.channels[0] }, { ...m.channels[1] }] };
  if (input.deck) {
    const { deck, ...channel } = input;
    Object.assign(next.channels[deck - 1], channel);
  }
  if (input.crossfader !== undefined) next.crossfader = input.crossfader;
  if (input.master_volume !== undefined) next.master_volume = input.master_volume;
  store.applyMixer(next);
}

/**
 * The engine's mixer state (event or reply), with changes it hasn't applied
 * yet kept on top, so a slow event never drags a fader back.
 */
export function applyEngineMixer(mixer: MixerState): void {
  store.applyMixer(mixer);
  for (const queue of mixerQueues.values()) {
    if (queue.inFlight || queue.next) applyLocally({ ...queue.inFlight, ...queue.next });
  }
}

/**
 * Sets mixer values. Shown at once; while a change is in flight only the
 * latest value per channel waits, so dragging a fader stays cheap.
 */
export function mixerSet(input: MixerInput): void {
  applyLocally(input);
  const group: MixerGroup = input.deck ? (String(input.deck) as MixerGroup) : "global";
  const queue = mixerQueues.get(group) ?? { inFlight: null, next: null };
  mixerQueues.set(group, queue);
  queue.next = { ...queue.next, ...input };
  pumpMixer(queue);
}

function pumpMixer(queue: MixerQueue) {
  if (queue.inFlight || !queue.next) return;
  const input = queue.next;
  queue.next = null;
  queue.inFlight = input;
  commands
    .mixer(input)
    .then((mixer) => {
      queue.inFlight = null;
      applyEngineMixer(mixer);
    })
    .catch((e) => {
      queue.inFlight = null;
      toast(e);
    })
    .finally(() => pumpMixer(queue));
}
