// Small caching helpers for data fetched per row on screen (Preview column
// thumbnails): a queue that fetches only what's still wanted, and an LRU map.

/**
 * Fetches keys that are wanted, at most `concurrency` at once, newest
 * request first. A key no longer wanted when its turn comes is skipped, so
 * rows scrolled past quickly never fetch.
 */
export class FetchQueue<K> {
  private wanted = new Map<K, number>();
  private waiting: K[] = [];
  private inFlight = new Set<K>();

  constructor(
    private readonly concurrency: number,
    private readonly fetch: (key: K) => Promise<void>,
    private readonly done: (key: K) => boolean,
  ) {}

  /** Registers interest in `key`; returns the release. */
  want(key: K): () => void {
    this.wanted.set(key, (this.wanted.get(key) ?? 0) + 1);
    if (!this.done(key) && !this.inFlight.has(key)) {
      this.waiting = this.waiting.filter((k) => k !== key);
      this.waiting.push(key);
      this.pump();
    }
    let released = false;
    return () => {
      if (released) return;
      released = true;
      const count = (this.wanted.get(key) ?? 1) - 1;
      if (count > 0) this.wanted.set(key, count);
      else this.wanted.delete(key);
    };
  }

  /** Keys waiting their turn (for tests). */
  get pending(): K[] {
    return this.waiting.filter((k) => this.wanted.has(k));
  }

  private pump(): void {
    while (this.inFlight.size < this.concurrency && this.waiting.length > 0) {
      const key = this.waiting.pop()!;
      if (!this.wanted.has(key) || this.done(key)) continue;
      this.inFlight.add(key);
      this.fetch(key)
        .catch(() => {})
        .finally(() => {
          this.inFlight.delete(key);
          this.pump();
        });
    }
  }
}

/** A small least-recently-used map. */
export class Lru<K, V> {
  private map = new Map<K, V>();
  constructor(private readonly max: number) {}

  get(key: K): V | undefined {
    const value = this.map.get(key);
    if (value !== undefined) {
      this.map.delete(key);
      this.map.set(key, value);
    }
    return value;
  }

  has(key: K): boolean {
    return this.map.has(key);
  }

  delete(key: K): void {
    this.map.delete(key);
  }

  set(key: K, value: V): void {
    this.map.delete(key);
    this.map.set(key, value);
    while (this.map.size > this.max) this.map.delete(this.map.keys().next().value!);
  }

  get size(): number {
    return this.map.size;
  }
}
