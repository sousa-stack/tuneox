// A deck's overview ("full waveform", research §4.6): the 1200-point preview
// in half form, hot cue letter squares above, memory cue triangles on top,
// the current cue as an orange triangle below, minute ticks, the played part
// dimmed, and the playhead (light while playing, red when paused).

import { UI } from "../colors";
import type { ShownCue } from "../cues";
import type { DeckState, WaveformStyle } from "../types";
import { paintOverview, type Waveform } from "../waveform";
import { triangle } from "./detail";

export interface OverviewInput {
  state: DeckState;
  position: number;
  wf: Waveform | null;
  style: WaveformStyle;
  hotCues: (ShownCue | null)[];
  memoryCues: ShownCue[];
}

/** How bright the played part is drawn. */
const PLAYED_ALPHA = 0.42;
const FONT = "-apple-system, BlinkMacSystemFont, 'Inter', 'Segoe UI', sans-serif";

export class OverviewView {
  private ctx: CanvasRenderingContext2D;
  private image: HTMLCanvasElement | null = null;
  private imageFor: [Waveform | null, WaveformStyle | null, number, number] = [null, null, 0, 0];
  private drawn: unknown[] = [];
  dpr = 1;
  width = 0;
  height = 0;
  top = 0;
  bottom = 0;

  constructor(readonly canvas: HTMLCanvasElement) {
    this.ctx = canvas.getContext("2d")!;
  }

  resize(cssWidth: number, cssHeight: number, dpr: number): void {
    const width = Math.max(1, Math.round(cssWidth * dpr));
    const height = Math.max(1, Math.round(cssHeight * dpr));
    if (width === this.width && height === this.height && dpr === this.dpr) return;
    this.width = width;
    this.height = height;
    if (this.canvas.width !== width) this.canvas.width = width;
    if (this.canvas.height !== height) this.canvas.height = height;
    this.dpr = dpr;
    this.top = Math.round(11 * dpr);
    this.bottom = Math.round(7 * dpr);
    this.drawn = [];
  }

  /** Draws if anything visible changed; returns whether it did. */
  draw(input: OverviewInput): boolean {
    const { state } = input;
    const duration = state.duration_us;
    const playheadX = duration > 0 ? Math.round((input.position / duration) * this.width) : 0;
    const stamp = [state, playheadX, input.wf, input.style, input.hotCues, input.memoryCues, this.width, this.height];
    if (stamp.length === this.drawn.length && stamp.every((v, i) => v === this.drawn[i])) return false;
    this.drawn = stamp;

    const { ctx, width: w, height: h, dpr: u } = this;
    const waveTop = this.top;
    const waveHeight = h - this.top - this.bottom;
    ctx.clearRect(0, 0, w, h);
    ctx.fillStyle = "#000";
    ctx.fillRect(0, waveTop, w, waveHeight);
    if (state.track_id == null || duration <= 0) return true;

    const image = this.imageOf(input.wf, input.style, w, waveHeight);
    if (image) {
      if (playheadX > 0) {
        ctx.globalAlpha = PLAYED_ALPHA;
        ctx.drawImage(image, 0, 0, Math.min(w, playheadX), waveHeight, 0, waveTop, Math.min(w, playheadX), waveHeight);
        ctx.globalAlpha = 1;
      }
      if (playheadX < w) {
        const x0 = Math.max(0, playheadX);
        ctx.drawImage(image, x0, 0, w - x0, waveHeight, x0, waveTop, w - x0, waveHeight);
      }
    }

    const xAt = (us: number) => Math.round((us / duration) * w);
    const lw = Math.max(1, Math.round(u));

    if (state.loop) {
      ctx.fillStyle = UI.loop;
      ctx.fillRect(xAt(state.loop.start_us), waveTop, Math.max(lw, xAt(state.loop.end_us) - xAt(state.loop.start_us)), waveHeight);
    }

    // Minute ticks along the bottom.
    ctx.fillStyle = UI.minuteTick;
    const tick = Math.round(2 * u);
    for (let m = 60e6; m < duration; m += 60e6) ctx.fillRect(xAt(m), h - tick, lw, tick);

    // Memory cues: triangles on top of the waveform.
    for (const cue of input.memoryCues) {
      ctx.fillStyle = cue.color;
      triangle(ctx, xAt(cue.start_us), waveTop + Math.round(5 * u), Math.round(7 * u), Math.round(5 * u));
    }

    // Hot cue letter squares above.
    const square = Math.round(10 * u);
    ctx.font = `700 ${Math.round(8 * u)}px ${FONT}`;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    for (const cue of input.hotCues) {
      if (!cue) continue;
      const x = Math.min(w - square, xAt(cue.start_us));
      ctx.fillStyle = cue.color;
      ctx.fillRect(x, 0, square, square);
      ctx.fillStyle = "#111";
      ctx.fillText(String.fromCharCode(65 + cue.id), x + square / 2, square / 2 + 0.5 * u);
    }

    // Current cue below.
    if (state.cue_us != null) {
      ctx.fillStyle = UI.currentCue;
      triangle(ctx, xAt(state.cue_us), h - this.bottom, Math.round(9 * u), -this.bottom);
    }

    // Playhead.
    ctx.fillStyle = state.playing ? UI.playheadPlaying : UI.playheadPaused;
    ctx.fillRect(Math.min(w - lw, playheadX), waveTop - Math.round(2 * u), lw, waveHeight + Math.round(4 * u));
    return true;
  }

  private imageOf(wf: Waveform | null, style: WaveformStyle, w: number, h: number): HTMLCanvasElement | null {
    if (!wf) return null;
    const [cwf, cstyle, cw, ch] = this.imageFor;
    if (this.image && cwf === wf && cstyle === style && cw === w && ch === h) return this.image;
    const image = this.image ?? document.createElement("canvas");
    image.width = w;
    image.height = h;
    const data = new ImageData(w, h);
    paintOverview(wf, style, w, h, new Uint32Array(data.data.buffer));
    image.getContext("2d")!.putImageData(data, 0, 0);
    this.image = image;
    this.imageFor = [wf, style, w, h];
    return image;
  }
}
