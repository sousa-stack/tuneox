// React bindings for the deck stores. Components re-render only when a
// deck's non-clock state, the mixer, settings or track data change.

import { useCallback, useEffect, useSyncExternalStore } from "react";
import { applyEngineMixer, applyPlayerSettings } from "./actions";
import { commands, onDeckEvent, onMixerEvent, onPreviewEvent } from "./commands";
import { preview } from "./preview";
import { settings, type Settings } from "./settings";
import { store, type DeckView } from "./store";
import { toast, toastStore, type Toast } from "./toast";
import { trackData, type TrackData } from "./trackData";
import { DECKS, type DeckId, type MixerState } from "./types";

export function useDeck(deck: DeckId): DeckView {
  const subscribe = useCallback((listener: () => void) => store.subscribeDeck(deck, listener), [deck]);
  return useSyncExternalStore(subscribe, () => store.view(deck));
}

const subscribeMixer = (listener: () => void) => store.subscribeMixer(listener);
export function useMixer(): MixerState {
  return useSyncExternalStore(subscribeMixer, () => store.mixer);
}

const subscribeSettings = (listener: () => void) => settings.subscribe(listener);
export function useSettings(): Settings {
  return useSyncExternalStore(subscribeSettings, () => settings.get());
}

export function useTrackData(trackId: number | null): TrackData | null {
  return useSyncExternalStore(trackData.subscribe, () => trackData.peek(trackId));
}

export function useToasts(): Toast[] {
  return useSyncExternalStore(toastStore.subscribe, toastStore.get);
}

/**
 * Follows the engine: subscribes to deck and mixer events, reads the
 * initial state, and fetches each loaded track's data as soon as it lands
 * on a deck. Mount once, at the top of the app.
 */
export function useDeckEngine(): void {
  useEffect(() => {
    let alive = true;
    const unlisten = [
      onDeckEvent((event) => {
        const { sent_at_ms, ...state } = event;
        store.applyDeck(state, sent_at_ms);
      }),
      onMixerEvent(applyEngineMixer),
      onPreviewEvent((event) => {
        const { sent_at_ms, ...state } = event;
        preview.apply(state, sent_at_ms);
      }),
    ];
    commands
      .status()
      .then((status) => {
        if (!alive) return;
        store.applyStatus(status);
        preview.apply(status.preview);
        applyPlayerSettings(status);
        // Without a device the engine still runs, silently.
        if (status.output?.error) toast(`No audio output: ${status.output.error}`);
      })
      .catch(toast);
    const loaded: (number | null)[] = [null, null];
    const follow = (deck: DeckId) => {
      const id = store.view(deck).state.track_id;
      if (id !== loaded[deck - 1]) {
        loaded[deck - 1] = id;
        if (id != null) trackData.ensure(id);
      }
    };
    const stops = DECKS.map((deck) => {
      follow(deck);
      return store.subscribeDeck(deck, () => follow(deck));
    });
    return () => {
      alive = false;
      stops.forEach((stop) => stop());
      unlisten.forEach((p) => p.then((stop) => stop()));
    };
  }, []);
}
