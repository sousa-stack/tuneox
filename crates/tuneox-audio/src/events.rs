//! Turning state snapshots into events: one on every change, plus a clock
//! update about 20 times a second while a deck or the preview plays.

use crate::core::Snapshot;
use crate::state::{DeckEvent, DeckState, EngineEvent, MixerState, PreviewEvent, PreviewState};

/// Clock updates for a playing deck, in milliseconds.
pub(crate) const PLAYING_INTERVAL_MS: f64 = 50.0;

#[derive(Default)]
pub(crate) struct EventPump {
    decks: [Option<Sent<DeckState>>; 2],
    mixer: Option<MixerState>,
    preview: Option<Sent<PreviewState>>,
}

struct Sent<T> {
    /// The state with the parts that move on their own blanked out.
    key: T,
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
    if key.scrubbing {
        // A scrubbed deck's speed changes every block; the periodic update
        // is enough.
        key.rate = 0.0;
    } else if key.playing {
        // A variable grid's BPM drifts beat by beat; that alone doesn't
        // need more than the periodic update.
        key.bpm = key.bpm.map(|b| (b * 100.0).round() / 100.0);
        key.original_bpm = key.original_bpm.map(|b| (b * 100.0).round() / 100.0);
    } else {
        key.position_us = state.position_us;
    }
    key
}

fn preview_key(state: &PreviewState) -> PreviewState {
    let mut key = state.clone();
    key.at_ms = 0.0;
    if key.loading {
        key.duration_us = 0;
    }
    if key.rate > 0.0 {
        key.position_us = 0;
    }
    key
}

impl EventPump {
    pub(crate) fn poll(
        &mut self,
        snapshot: &Snapshot,
        decks: [DeckState; 2],
        mixer: MixerState,
        preview: PreviewState,
        now_ms: f64,
    ) -> Vec<EngineEvent> {
        let mut events = Vec::new();
        for (d, state) in decks.into_iter().enumerate() {
            let key = key(&state);
            let jumps = snapshot.decks[d].jumps;
            let moving = state.rate != 0.0 || state.scrubbing;
            let due = match &self.decks[d] {
                None => true,
                Some(sent) => {
                    sent.key != key
                        || sent.jumps != jumps
                        || (moving && now_ms - sent.at >= PLAYING_INTERVAL_MS)
                }
            };
            if due {
                self.decks[d] = Some(Sent {
                    key,
                    jumps,
                    at: now_ms,
                });
                events.push(EngineEvent::Deck(Box::new(DeckEvent {
                    state,
                    sent_at_ms: now_ms,
                })));
            }
        }
        if self.mixer != Some(mixer) {
            self.mixer = Some(mixer);
            events.push(EngineEvent::Mixer(mixer));
        }
        let key = preview_key(&preview);
        let due = match &self.preview {
            None => true,
            Some(sent) => {
                sent.key != key || (preview.rate > 0.0 && now_ms - sent.at >= PLAYING_INTERVAL_MS)
            }
        };
        if due {
            self.preview = Some(Sent {
                key,
                jumps: 0,
                at: now_ms,
            });
            events.push(EngineEvent::Preview(Box::new(PreviewEvent {
                state: preview,
                sent_at_ms: now_ms,
            })));
        }
        events
    }
}
