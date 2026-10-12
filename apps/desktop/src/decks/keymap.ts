// rekordbox's EXPORT keyboard preset (research/rekordbox-decks.md §6): plain
// keys act on deck A, ⇧ on deck B. Matched on `KeyboardEvent.code`, so the
// keys stay put on any keyboard layout.

import type { BendDirection, DeckId } from "./types";

export type KeyAction =
  | { kind: "playPause" }
  | { kind: "cue" }
  | { kind: "beatJump"; direction: -1 | 1 }
  | { kind: "hotCue"; slot: number }
  | { kind: "loopIn" }
  | { kind: "loopOut" }
  | { kind: "reloopExit" }
  | { kind: "beatLoop"; beats: number }
  | { kind: "loopHalve" }
  | { kind: "sync" }
  | { kind: "tempoReset" }
  | { kind: "pitchBend"; direction: BendDirection }
  | { kind: "tempoStep"; direction: -1 | 1 }
  | { kind: "timeMode" }
  | { kind: "quantize" }
  | { kind: "memoryCue"; target: number | "next" | "prev" }
  | { kind: "zoom"; direction: -1 | 0 | 1 };

export interface KeyBinding {
  deck: DeckId;
  action: KeyAction;
  /** Acts while held: the key's release matters (CUE, hot cues, pitch bend, tempo auto-repeat). */
  held: boolean;
}

export interface Modifiers {
  shiftKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
}

const BEAT_LOOPS: Record<string, number> = { Digit4: 1, Digit5: 2, Digit6: 4, Digit7: 8, Digit8: 16, Digit9: 32 };
const HOT_CUES: Record<string, number> = { Digit1: 0, Digit2: 1, Digit3: 2 };
const MEMORY_CUES = ["KeyA", "KeyS", "KeyD", "KeyF", "KeyG", "KeyH", "KeyJ", "KeyK", "KeyL", "Semicolon"];

function action(code: string): [KeyAction, boolean] | null {
  if (code in HOT_CUES) return [{ kind: "hotCue", slot: HOT_CUES[code] }, true];
  if (code in BEAT_LOOPS) return [{ kind: "beatLoop", beats: BEAT_LOOPS[code] }, false];
  const memory = MEMORY_CUES.indexOf(code);
  if (memory >= 0) return [{ kind: "memoryCue", target: memory }, false];
  switch (code) {
    case "Space":
      return [{ kind: "playPause" }, false];
    case "KeyC":
      return [{ kind: "cue" }, true];
    case "ArrowLeft":
      return [{ kind: "beatJump", direction: -1 }, false];
    case "ArrowRight":
      return [{ kind: "beatJump", direction: 1 }, false];
    case "KeyI":
      return [{ kind: "loopIn" }, false];
    case "KeyO":
      return [{ kind: "loopOut" }, false];
    case "KeyR":
      return [{ kind: "reloopExit" }, false];
    case "Slash":
      return [{ kind: "loopHalve" }, false];
    case "KeyN":
      return [{ kind: "memoryCue", target: "next" }, false];
    case "KeyB":
      return [{ kind: "memoryCue", target: "prev" }, false];
    case "KeyQ":
      return [{ kind: "quantize" }, false];
    case "KeyT":
      return [{ kind: "timeMode" }, false];
    case "F1":
      return [{ kind: "sync" }, false];
    case "F3":
      return [{ kind: "tempoReset" }, false];
    case "F4":
      return [{ kind: "pitchBend", direction: "back" }, true];
    case "F5":
      return [{ kind: "pitchBend", direction: "forward" }, true];
    case "F6":
      return [{ kind: "tempoStep", direction: -1 }, true];
    case "F7":
      return [{ kind: "tempoStep", direction: 1 }, true];
    // Waveform zoom, as in rekordbox's Performance presets.
    case "Equal":
      return [{ kind: "zoom", direction: 1 }, false];
    case "Minus":
      return [{ kind: "zoom", direction: -1 }, false];
    default:
      return null;
  }
}

/** The deck action for a key, or null. ⌘/Ctrl/⌥ combinations are left to the app (⌘F etc.). */
export function bindKey(code: string, mods: Modifiers): KeyBinding | null {
  if (mods.metaKey || mods.ctrlKey || mods.altKey) return null;
  const found = action(code);
  if (!found) return null;
  const [act, held] = found;
  // Zoom is shared by both lanes: ⇧= resets instead of picking deck B.
  if (act.kind === "zoom") {
    return { deck: 1, action: mods.shiftKey && act.direction === 1 ? { kind: "zoom", direction: 0 } : act, held };
  }
  return { deck: mods.shiftKey ? 2 : 1, action: act, held };
}

/** Typing in these never triggers deck keys. */
export function isTextEntry(target: EventTarget | null): boolean {
  if (!target || typeof (target as Element).closest !== "function") return false;
  const element = target as HTMLElement;
  if (element.isContentEditable) return true;
  const tag = element.tagName;
  if (tag === "TEXTAREA" || tag === "SELECT") return true;
  if (tag !== "INPUT") return false;
  const type = (element as HTMLInputElement).type;
  return !["button", "checkbox", "radio", "range", "submit", "reset", "color"].includes(type);
}
