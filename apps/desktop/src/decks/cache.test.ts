import { describe, expect, test } from "bun:test";
import { FetchQueue, Lru } from "./cache";

/** A fetch whose calls can be finished one at a time. */
function controlled() {
  const started: number[] = [];
  const finish = new Map<number, () => void>();
  const fetch = (key: number) =>
    new Promise<void>((resolve) => {
      started.push(key);
      finish.set(key, resolve);
    });
  return { started, finish, fetch };
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("FetchQueue", () => {
  test("fetches at most `concurrency` at once, newest request first", async () => {
    const { started, finish, fetch } = controlled();
    const done = new Set<number>();
    const queue = new FetchQueue<number>(2, (k) => fetch(k).then(() => void done.add(k)), (k) => done.has(k));
    for (const key of [1, 2, 3, 4, 5]) queue.want(key);
    expect(started).toEqual([1, 2]);
    finish.get(1)!();
    await tick();
    // The rows shown most recently go first.
    expect(started).toEqual([1, 2, 5]);
    finish.get(2)!();
    finish.get(5)!();
    await tick();
    expect(started).toEqual([1, 2, 5, 4, 3]);
  });

  test("skips keys nobody wants any more (rows scrolled past)", async () => {
    const { started, finish, fetch } = controlled();
    const done = new Set<number>();
    const queue = new FetchQueue<number>(1, (k) => fetch(k).then(() => void done.add(k)), (k) => done.has(k));
    queue.want(1);
    const release2 = queue.want(2);
    const release3 = queue.want(3);
    release3();
    release2();
    expect(queue.pending).toEqual([]);
    // Wanted twice (two rows), released once: still wanted.
    queue.want(4);
    const again = queue.want(4);
    again();
    expect(queue.pending).toEqual([4]);
    finish.get(1)!();
    await tick();
    expect(started).toEqual([1, 4]);
  });

  test("doesn't fetch what's already there", () => {
    const { started, fetch } = controlled();
    const queue = new FetchQueue<number>(4, fetch, (k) => k === 7);
    queue.want(7);
    expect(started).toEqual([]);
  });
});

describe("Lru", () => {
  test("evicts the least recently used", () => {
    const lru = new Lru<string, number | null>(2);
    lru.set("a", 1);
    lru.set("b", null);
    expect(lru.get("a")).toBe(1);
    lru.set("c", 3);
    expect(lru.has("b")).toBe(false);
    expect(lru.has("a")).toBe(true);
    expect(lru.size).toBe(2);
    lru.set("d", 4);
    expect(lru.has("c")).toBe(true);
    expect(lru.has("a")).toBe(false);
  });
});
