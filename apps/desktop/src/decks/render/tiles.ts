// Detail waveforms are pre-rendered into tiles: canvases TILE_WIDTH device
// pixels wide, aligned to the track, so a frame only blits two or three of
// them with an offset. Tiles depend on the track, style, scale, lane height
// and direction; a change to any of them starts a fresh set.

import { paintDetailTile, type Waveform } from "../waveform";

export const TILE_WIDTH = 1024;
const MAX_TILES = 14;

export class TileCache {
  private key = "";
  private tiles = new Map<number, HTMLCanvasElement>();
  private scratch: ImageData | null = null;
  /** Tiles rendered since the counter was last reset (for the perf HUD). */
  rendered = 0;

  has(key: string, index: number): boolean {
    return key === this.key && this.tiles.has(index);
  }

  get size(): number {
    return this.tiles.size;
  }

  tile(key: string, index: number, wf: Waveform, columnsPerPixel: number, height: number, baselineAtBottom: boolean): HTMLCanvasElement {
    if (key !== this.key) {
      this.key = key;
      this.tiles.clear();
    }
    const cached = this.tiles.get(index);
    if (cached) {
      // Most recently used last.
      this.tiles.delete(index);
      this.tiles.set(index, cached);
      return cached;
    }
    let canvas: HTMLCanvasElement;
    if (this.tiles.size >= MAX_TILES) {
      const oldest = this.tiles.keys().next().value!;
      canvas = this.tiles.get(oldest)!;
      this.tiles.delete(oldest);
    } else {
      canvas = document.createElement("canvas");
    }
    if (canvas.width !== TILE_WIDTH) canvas.width = TILE_WIDTH;
    if (canvas.height !== height) canvas.height = height;
    if (!this.scratch || this.scratch.height !== height) this.scratch = new ImageData(TILE_WIDTH, height);
    paintDetailTile(
      wf,
      { x0: index * TILE_WIDTH, width: TILE_WIDTH, height, columnsPerPixel, baselineAtBottom },
      new Uint32Array(this.scratch.data.buffer),
    );
    canvas.getContext("2d")!.putImageData(this.scratch, 0, 0);
    this.tiles.set(index, canvas);
    this.rendered++;
    return canvas;
  }

  clear(): void {
    this.key = "";
    this.tiles.clear();
  }
}
