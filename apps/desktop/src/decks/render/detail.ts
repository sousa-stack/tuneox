// The DUAL PLAYER's enlarged waveforms: deck A's half-waveform grows up from
// its baseline, deck B's grows down from its own, facing each other around
// one shared centre playhead (research/rekordbox-decks.md §1.2, §4.5).
//
// Layout, top to bottom: deck A's marker strip (hot cue flags, memory cue
// triangles, bar counter), deck A's lane, a 2 px gap, deck B's lane, deck B's
// marker strip. Everything is drawn in device pixels.

import { tempoFactor } from "../clock";
import { UI } from "../colors";
import type { ShownCue } from "../cues";
import { formatBars, lowerBound, type BeatGrid } from "../grid";
import type { DeckState } from "../types";
import type { Waveform } from "../waveform";
import { TILE_WIDTH, TileCache } from "./tiles";

export interface LaneInput {
  state: DeckState;
  /** Playhead, track µs. */
  position: number;
  wf: Waveform | null;
  /** Shown in the lane instead of a waveform. */
  message: string | null;
  grid: BeatGrid | null;
  hotCues: (ShownCue | null)[];
  memoryCues: ShownCue[];
}

/** rekordbox hides the beat grid when more than this much is visible. */
const GRID_MAX_SPAN_S = 120;
const FONT = "-apple-system, BlinkMacSystemFont, 'Inter', 'Segoe UI', sans-serif";

export interface DetailGeometry {
  width: number;
  height: number;
  strip: number;
  lane: number;
  gap: number;
  /** Top of each lane. */
  laneTop: [number, number];
  centerX: number;
}

export class DetailView {
  private ctx: CanvasRenderingContext2D;
  readonly tiles: [TileCache, TileCache] = [new TileCache(), new TileCache()];
  dpr = 1;
  geometry: DetailGeometry = { width: 0, height: 0, strip: 0, lane: 0, gap: 0, laneTop: [0, 0], centerX: 0 };

  constructor(readonly canvas: HTMLCanvasElement) {
    this.ctx = canvas.getContext("2d", { alpha: false })!;
  }

  resize(cssWidth: number, cssHeight: number, dpr: number): void {
    const width = Math.max(1, Math.round(cssWidth * dpr));
    const height = Math.max(1, Math.round(cssHeight * dpr));
    const g = this.geometry;
    if (width === g.width && height === g.height && dpr === this.dpr) return;
    if (this.canvas.width !== width) this.canvas.width = width;
    if (this.canvas.height !== height) this.canvas.height = height;
    this.dpr = dpr;
    const strip = Math.round(14 * dpr);
    const gap = Math.max(1, Math.round(2 * dpr));
    const lane = Math.max(1, Math.floor((height - 2 * strip - gap) / 2));
    this.geometry = { width, height, strip, lane, gap, laneTop: [strip, strip + lane + gap], centerX: Math.floor(width / 2) };
  }

  /** Device pixels per track µs for a deck at the current zoom. */
  pxPerUs(spanSeconds: number, state: DeckState): number {
    return this.geometry.width / spanSeconds / 1e6 / tempoFactor(state.tempo_percent);
  }

  /** The track position under a device x on a deck's lane. */
  positionAtX(x: number, spanSeconds: number, lane: LaneInput): number {
    return lane.position + (x - this.geometry.centerX) / this.pxPerUs(spanSeconds, lane.state);
  }

  /**
   * Draws both lanes. `spanSeconds` is the visible real time (tempo changes
   * the beat spacing, so synced decks' beats line up across the playhead).
   * Renders at most `tileBudget` uncached tiles that aren't visible yet.
   */
  draw(lanes: [LaneInput, LaneInput], spanSeconds: number, tileBudget: number): void {
    const { ctx } = this;
    const g = this.geometry;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.globalAlpha = 1;
    ctx.fillStyle = "#000";
    ctx.fillRect(0, 0, g.width, g.height);
    lanes.forEach((lane, i) => this.drawLane(lane, i as 0 | 1, spanSeconds, tileBudget));
    this.drawPlayhead(lanes);
    lanes.forEach((lane, i) => this.drawBarCounter(lane, i as 0 | 1));
  }

  private drawLane(lane: LaneInput, i: 0 | 1, span: number, tileBudget: number) {
    const { ctx } = this;
    const g = this.geometry;
    const u = this.dpr;
    const top = g.laneTop[i];
    const bottom = top + g.lane;
    const up = i === 0; // deck A grows up from its baseline at the lane's bottom
    const state = lane.state;
    if (state.track_id == null) {
      this.message(i === 0 ? "Deck A: drag a track here" : "Deck B: drag a track here", top, g.lane);
      return;
    }
    const pxPerUs = this.pxPerUs(span, state);
    const cx = g.centerX;
    // Track pixel at the canvas's left edge. Tiles and markers both snap to
    // it, so grid lines never shimmer against the waveform while scrolling.
    const origin = Math.round(lane.position * pxPerUs) - cx;
    const xAt = (us: number) => Math.floor(us * pxPerUs) - origin;
    const lw = Math.max(1, Math.round(u));

    // Waveform tiles.
    const wf = lane.wf;
    if (wf && wf.columnsPerSecond) {
      const cpp = wf.columnsPerSecond / (pxPerUs * 1e6);
      const key = `${state.track_id}|${wf.style}|${cpp.toFixed(8)}|${g.lane}|${i}`;
      const total = Math.ceil(wf.columns / cpp);
      const first = Math.max(0, Math.floor(origin / TILE_WIDTH));
      const last = Math.min(Math.floor((total - 1) / TILE_WIDTH), Math.floor((origin + g.width - 1) / TILE_WIDTH));
      const cache = this.tiles[i];
      for (let k = first; k <= last; k++) {
        ctx.drawImage(cache.tile(key, k, wf, cpp, g.lane, up), k * TILE_WIDTH - origin, top);
      }
      // Get the next tile ready in the direction of play.
      const ahead = last + 1;
      if (tileBudget > 0 && state.rate > 0 && ahead * TILE_WIDTH < total && !cache.has(key, ahead)) {
        cache.tile(key, ahead, wf, cpp, g.lane, up);
      }
    }
    if (lane.message) this.message(lane.message, top, g.lane);

    const tMin = lane.position - cx / pxPerUs;
    const tMax = lane.position + (g.width - cx) / pxPerUs;
    const inView = (a: number, b: number) => b >= tMin && a <= tMax;

    // Loops: the last (stored) loop faint, the active one #FFA300 at 50 %.
    const loop = state.loop;
    const stored = state.last_loop;
    if (stored && !loop && inView(stored.start_us, stored.end_us)) {
      ctx.fillStyle = UI.storedLoop;
      ctx.fillRect(xAt(stored.start_us), top, Math.max(lw, xAt(stored.end_us) - xAt(stored.start_us)), g.lane);
    }
    if (loop && inView(loop.start_us, loop.end_us)) {
      ctx.fillStyle = UI.loop;
      ctx.fillRect(xAt(loop.start_us), top, Math.max(lw, xAt(loop.end_us) - xAt(loop.start_us)), g.lane);
    }

    // Beat grid.
    if (lane.grid && span * tempoFactor(state.tempo_percent) < GRID_MAX_SPAN_S) {
      const { times, bars } = lane.grid;
      const mark = Math.round(4 * u);
      const tick = Math.round(3 * u);
      for (let b = lowerBound(times, tMin); b < times.length && times[b] <= tMax; b++) {
        const x = xAt(times[b]);
        if (bars[b] === 1) {
          ctx.fillStyle = "#000";
          ctx.fillRect(x - lw, top, 3 * lw, g.lane);
          ctx.fillStyle = "#FFF";
          ctx.fillRect(x, top, lw, g.lane);
          ctx.fillStyle = UI.downbeatMarker;
          ctx.fillRect(x - lw, top, 3 * lw, mark);
          ctx.fillRect(x - lw, bottom - mark, 3 * lw, mark);
        } else {
          ctx.fillStyle = "rgba(255,255,255,0.3)";
          ctx.fillRect(x, top, lw, g.lane);
          ctx.fillStyle = "#FFF";
          ctx.fillRect(x, top, lw, tick);
          ctx.fillRect(x, bottom - tick, lw, tick);
        }
      }
    }

    // Manual loop in, waiting for OUT; active loop in/out cursors.
    if (state.loop_in_us != null && !loop) {
      ctx.fillStyle = UI.loopIn;
      ctx.fillRect(xAt(state.loop_in_us), top, lw, g.lane);
    }
    if (loop) {
      ctx.fillStyle = UI.loopIn;
      ctx.fillRect(xAt(loop.start_us), top, lw, g.lane);
      ctx.fillStyle = "#FFF";
      ctx.fillRect(xAt(loop.end_us) - lw, top, lw, g.lane);
    }

    // Marker strip: memory cues (loops as bars), hot cue flags and lines.
    const stripTop = i === 0 ? 0 : bottom + 0;
    const stripEdge = i === 0 ? top : bottom; // where the strip meets the lane
    const tw = Math.round(9 * u);
    const th = Math.round(6 * u);
    for (const cue of lane.memoryCues) {
      if (!inView(cue.start_us, cue.end_us ?? cue.start_us)) continue;
      const x = xAt(cue.start_us);
      ctx.fillStyle = cue.color;
      if (cue.end_us != null) {
        ctx.globalAlpha = 0.7;
        const barH = Math.round(3 * u);
        ctx.fillRect(x, i === 0 ? stripEdge - barH : stripEdge, Math.max(lw, xAt(cue.end_us) - x), barH);
        ctx.globalAlpha = 1;
      }
      triangle(ctx, x, stripEdge, tw, i === 0 ? th : -th);
    }
    const flag = Math.round(11 * u);
    ctx.font = `700 ${Math.round(9 * u)}px ${FONT}`;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    for (const cue of lane.hotCues) {
      if (!cue || !inView(cue.start_us, cue.end_us ?? cue.start_us)) continue;
      const x = xAt(cue.start_us);
      ctx.fillStyle = cue.color;
      ctx.fillRect(x, top, lw, g.lane);
      if (cue.end_us != null) {
        ctx.globalAlpha = 0.18;
        ctx.fillRect(x, top, Math.max(lw, xAt(cue.end_us) - x), g.lane);
        ctx.globalAlpha = 1;
      }
      const fy = i === 0 ? stripTop + Math.round(1 * u) : g.height - flag - Math.round(1 * u);
      ctx.fillRect(x, fy, flag, flag);
      ctx.fillStyle = "#111";
      ctx.fillText(String.fromCharCode(65 + cue.id), x + flag / 2, fy + flag / 2 + 0.5 * u);
    }

    // Current cue: orange triangle on the baseline.
    if (state.cue_us != null && inView(state.cue_us, state.cue_us)) {
      ctx.fillStyle = UI.currentCue;
      const baseline = i === 0 ? bottom : top;
      triangle(ctx, xAt(state.cue_us), baseline - (i === 0 ? Math.round(8 * u) : -Math.round(8 * u)), Math.round(11 * u), i === 0 ? -Math.round(8 * u) : Math.round(8 * u));
    }
  }

  private drawPlayhead(lanes: [LaneInput, LaneInput]) {
    const { ctx } = this;
    const g = this.geometry;
    const lw = Math.max(1, Math.round(this.dpr));
    const split = g.laneTop[1] - Math.ceil(g.gap / 2);
    const parts: [number, number, DeckState][] = [
      [0, split, lanes[0].state],
      [split, g.height, lanes[1].state],
    ];
    for (const [y0, y1, state] of parts) {
      ctx.fillStyle = "rgba(0,0,0,0.5)";
      ctx.fillRect(g.centerX - lw, y0, 3 * lw, y1 - y0);
      ctx.fillStyle = state.track_id == null ? "#707070" : state.playing ? UI.playheadPlaying : UI.playheadPaused;
      ctx.fillRect(g.centerX, y0, lw, y1 - y0);
    }
  }

  private drawBarCounter(lane: LaneInput, i: 0 | 1) {
    if (lane.state.track_id == null || !lane.grid) return;
    const text = formatBars(lane.grid, lane.position);
    if (!text) return;
    const { ctx } = this;
    const g = this.geometry;
    const u = this.dpr;
    ctx.font = `600 ${Math.round(10.5 * u)}px ${FONT}`;
    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    // Left of the playhead, over what has played, so upcoming cue flags stay visible.
    const x = g.centerX - Math.round(5 * u);
    const y = i === 0 ? g.strip / 2 : g.height - g.strip / 2;
    const w = ctx.measureText(text).width;
    ctx.fillStyle = "#000";
    ctx.fillRect(x - w - Math.round(2 * u), y - g.strip / 2, w + Math.round(4 * u), g.strip);
    ctx.fillStyle = UI.accent;
    ctx.fillText(text, x, y + 0.5 * u);
  }

  private message(text: string, top: number, height: number) {
    const { ctx } = this;
    const u = this.dpr;
    ctx.font = `500 ${Math.round(11 * u)}px ${FONT}`;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillStyle = "#5c5f68";
    ctx.fillText(text, this.geometry.width / 2 + this.geometry.width / 6, top + height / 2);
  }
}

/** A triangle with its apex at (x, y), `h` tall (negative points up), `w` wide. */
export function triangle(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number): void {
  ctx.beginPath();
  ctx.moveTo(x, y);
  ctx.lineTo(x - w / 2, y - h);
  ctx.lineTo(x + w / 2, y - h);
  ctx.closePath();
  ctx.fill();
}
