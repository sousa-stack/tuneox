//! Turning state snapshots into events: one on every change, plus a clock
//! update about 20 times a second while a deck plays.

use crate::core::Snapshot;
use crate::state::{DeckEvent, DeckState, EngineEvent, MixerState};

/// Clock updates for a playing deck, in milliseconds.
pub(crate) const PLAYING_INTERVAL_MS: f64 = 50.0;

#[derive(Default)]
pub(crate) struct EventPump {
    decks: [Option<Sent>; 2],
    mixer: Option<MixerState>,
}

struct Sent {
    /// The state with the parts that move on their own blanked out.
    key: DeckState,
    jumps: u32,
    at: f64,
}

/// What must change for an event to be needed: everything except where a
/// moving playhead has got to, which the UI extrapolates.
fn key(state: &DeckState) -> DeckState {
    let mut key = state.clone();
    key.position_us = 0;
    key.at_ms = 0.0;
    if key.loading {
        // Grows as decoding goes for files that don't state their length;
        // the event when loading finishes carries the exact value.
        key.duration_us = 0;
    }
    if key.playing {
        // A variable grid's BPM drifts beat by beat; that alone doesn't
        // need more than the periodic update.
        key.bpm = key.bpm.map(|b| (b * 100.0).round() / 100.0);
        key.original_bpm = key.original_bpm.map(|b| (b * 100.0).round() / 100.0);
    } else {
        key.position_us = state.position_us;
    }
    key
}

impl EventPump {
    pub(crate) fn poll(
        &mut self,
        snapshot: &Snapshot,
        decks: [DeckState; 2],
        mixer: MixerState,
        now_ms: f64,
    ) -> Vec<EngineEvent> {
        let mut events = Vec::new();
        for (d, state) in decks.into_iter().enumerate() {
            let key = key(&state);
            let jumps = snapshot.decks[d].jumps;
            let due = match &self.decks[d] {
                None => true,
                Some(sent) => {
                    sent.key != key
                        || sent.jumps != jumps
                        || (state.rate > 0.0 && now_ms - sent.at >= PLAYING_INTERVAL_MS)
                }
            };
            if due {
                self.decks[d] = Some(Sent {
                    key,
                    jumps,
                    at: now_ms,
                });
                events.push(EngineEvent::Deck(DeckEvent {
                    state,
                    sent_at_ms: now_ms,
                }));
            }
        }
        if self.mixer != Some(mixer) {
            self.mixer = Some(mixer);
            events.push(EngineEvent::Mixer(mixer));
        }
        events
    }
}
