// Window keyboard handling for the EXPORT key map (keymap.ts). Keys with a
// press/release meaning (CUE, hot cues, pitch bend, tempo −/+) release on
// keyup, matched by physical key, so letting go of ⇧ first still releases
// the deck that was pressed.

import { useEffect } from "react";
import * as act from "./actions";
import { bindKey, isTextEntry, type KeyBinding } from "./keymap";
import { settings } from "./settings";

function press({ deck, action }: KeyBinding) {
  switch (action.kind) {
    case "playPause":
      return act.playPause(deck);
    case "cue":
      return act.cuePress(deck);
    case "beatJump":
      return act.beatJump(deck, action.direction);
    case "hotCue":
      return act.hotCuePress(deck, action.slot);
    case "loopIn":
      return act.loopIn(deck);
    case "loopOut":
      return act.loopOut(deck);
    case "reloopExit":
      return act.reloopExit(deck);
    case "beatLoop":
      return act.beatLoop(deck, action.beats);
    case "loopHalve":
      return act.loopSize(deck, "half");
    case "sync":
      return act.toggleSync(deck);
    case "tempoReset":
      return act.tempoReset(deck);
    case "pitchBend":
      return act.pitchBend(deck, action.direction, "press");
    case "tempoStep":
      return act.tempoStepPress(deck, action.direction);
    case "timeMode":
      return act.toggleTimeMode(deck);
    case "quantize":
      return act.toggleQuantize(deck);
    case "memoryCue":
      return act.memoryCue(deck, action.target);
    case "zoom":
      return act.zoom(action.direction);
  }
}

function release({ deck, action }: KeyBinding) {
  switch (action.kind) {
    case "cue":
      return act.cueRelease(deck);
    case "hotCue":
      return act.hotCueRelease(deck, action.slot);
    case "pitchBend":
      return act.pitchBend(deck, action.direction, "release");
    case "tempoStep":
      return act.tempoStepRelease(deck);
    default:
      return;
  }
}

export function useDeckKeys(): void {
  useEffect(() => {
    const held = new Map<string, KeyBinding>();
    const down = (e: KeyboardEvent) => {
      // ⌥⇧P: frame timing overlay.
      if (e.altKey && e.shiftKey && e.code === "KeyP") {
        e.preventDefault();
        settings.update((s) => ({ hud: !s.hud }));
        return;
      }
      if (isTextEntry(e.target)) return;
      const binding = bindKey(e.code, e);
      if (!binding) return;
      e.preventDefault();
      // OS auto-repeat is ignored; tempo −/+ repeats on its own timer.
      if (e.repeat || held.has(e.code)) return;
      if (binding.held) held.set(e.code, binding);
      press(binding);
    };
    const up = (e: KeyboardEvent) => {
      const binding = held.get(e.code);
      if (!binding) return;
      held.delete(e.code);
      e.preventDefault();
      release(binding);
    };
    const releaseAll = () => {
      for (const binding of held.values()) release(binding);
      held.clear();
    };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    window.addEventListener("blur", releaseAll);
    return () => {
      releaseAll();
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
      window.removeEventListener("blur", releaseAll);
    };
  }, []);
}
