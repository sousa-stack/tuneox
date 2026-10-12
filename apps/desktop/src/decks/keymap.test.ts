import { describe, expect, test } from "bun:test";
import { bindKey, type Modifiers } from "./keymap";

const plain: Modifiers = { shiftKey: false, metaKey: false, ctrlKey: false, altKey: false };
const shift: Modifiers = { ...plain, shiftKey: true };

describe("EXPORT key map", () => {
  test("plain keys act on deck A, ⇧ on deck B", () => {
    expect(bindKey("Space", plain)).toEqual({ deck: 1, action: { kind: "playPause" }, held: false });
    expect(bindKey("Space", shift)?.deck).toBe(2);
    expect(bindKey("KeyC", shift)).toEqual({ deck: 2, action: { kind: "cue" }, held: true });
  });

  test("transport, cues and loops", () => {
    expect(bindKey("ArrowLeft", plain)?.action).toEqual({ kind: "beatJump", direction: -1 });
    expect(bindKey("ArrowRight", plain)?.action).toEqual({ kind: "beatJump", direction: 1 });
    expect(bindKey("Digit1", plain)?.action).toEqual({ kind: "hotCue", slot: 0 });
    expect(bindKey("Digit3", shift)).toEqual({ deck: 2, action: { kind: "hotCue", slot: 2 }, held: true });
    expect(bindKey("KeyI", plain)?.action.kind).toBe("loopIn");
    expect(bindKey("KeyO", plain)?.action.kind).toBe("loopOut");
    expect(bindKey("KeyR", plain)?.action.kind).toBe("reloopExit");
    expect([4, 5, 6, 7, 8, 9].map((n) => bindKey(`Digit${n}`, plain)?.action)).toEqual(
      [1, 2, 4, 8, 16, 32].map((beats) => ({ kind: "beatLoop", beats })),
    );
    expect(bindKey("KeyA", plain)?.action).toEqual({ kind: "memoryCue", target: 0 });
    expect(bindKey("Semicolon", plain)?.action).toEqual({ kind: "memoryCue", target: 9 });
    expect(bindKey("KeyN", plain)?.action).toEqual({ kind: "memoryCue", target: "next" });
    expect(bindKey("KeyB", plain)?.action).toEqual({ kind: "memoryCue", target: "prev" });
  });

  test("function keys: sync, tempo reset, pitch bend, BPM −/+", () => {
    expect(bindKey("F1", plain)?.action.kind).toBe("sync");
    expect(bindKey("F3", plain)?.action.kind).toBe("tempoReset");
    expect(bindKey("F4", plain)).toEqual({ deck: 1, action: { kind: "pitchBend", direction: "back" }, held: true });
    expect(bindKey("F5", shift)).toEqual({ deck: 2, action: { kind: "pitchBend", direction: "forward" }, held: true });
    expect(bindKey("F6", plain)).toEqual({ deck: 1, action: { kind: "tempoStep", direction: -1 }, held: true });
    expect(bindKey("F7", plain)?.action).toEqual({ kind: "tempoStep", direction: 1 });
    expect(bindKey("KeyT", shift)).toEqual({ deck: 2, action: { kind: "timeMode" }, held: false });
  });

  test("leaves ⌘/Ctrl/⌥ combinations to the app (⌘F searches)", () => {
    expect(bindKey("KeyF", { ...plain, metaKey: true })).toBeNull();
    expect(bindKey("Space", { ...plain, ctrlKey: true })).toBeNull();
    expect(bindKey("KeyC", { ...plain, altKey: true })).toBeNull();
    expect(bindKey("KeyZ", plain)).toBeNull();
  });

  test("zoom is shared; ⇧= resets", () => {
    expect(bindKey("Equal", plain)?.action).toEqual({ kind: "zoom", direction: 1 });
    expect(bindKey("Minus", plain)?.action).toEqual({ kind: "zoom", direction: -1 });
    expect(bindKey("Equal", shift)?.action).toEqual({ kind: "zoom", direction: 0 });
  });
});
