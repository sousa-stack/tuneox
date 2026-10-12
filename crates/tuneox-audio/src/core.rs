//! The DSP core: two decks, sync and the mixer. It runs on the audio thread
//! (or the caller's thread for the offline engine) and talks to the rest of
//! the engine only through lock-free queues and a triple buffer:
//!
//! - commands arrive on an `rtrb` queue, applied at the start of each block;
//! - state leaves as a [`Snapshot`] written to a triple buffer once per block;
//! - replaced tracks go back on a "garbage" queue so they're freed off the
//!   audio thread.
//!
//! Nothing here allocates, locks, logs or does I/O once constructed.

use std::sync::Arc;

use rtrb::{Consumer, Producer};

use crate::deck::{Deck, HOT_CUES, HotCue, LoadedTrack, Loop};
use crate::grid::{Grid, wrap_phase};
use crate::mixer::{EQ_KILL_DB, EQ_MAX_DB, Mixer, MixerSettings};
use crate::pcm::TrackPcm;

/// Frames rendered per snapshot; device buffers are split into blocks of at
/// most this size.
pub(crate) const MAX_BLOCK: usize = 1024;
/// Frames between sync/tempo updates within a block.
const SUB_BLOCK: usize = 64;

/// Phase errors larger than this jump the synced deck into phase at once.
const SYNC_SNAP_SECONDS: f64 = 0.010;
/// Phase errors smaller than this are left alone (millisecond-rounded grids
/// jitter by about this much).
const SYNC_DEADBAND_SECONDS: f64 = 0.0005;
/// Time constant for pulling a synced deck back into phase.
const SYNC_CORRECTION_SECONDS: f64 = 0.5;
/// Most the phase correction may bend the tempo.
const SYNC_MAX_CORRECTION: f64 = 0.01;

pub(crate) enum Control {
    Load {
        deck: usize,
        track: LoadedTrack,
    },
    Play(usize),
    Pause(usize),
    Cue {
        deck: usize,
        press: bool,
    },
    HotCue {
        deck: usize,
        slot: usize,
        press: bool,
    },
    Seek {
        deck: usize,
        seconds: f64,
    },
    BeatJump {
        deck: usize,
        beats: f64,
    },
    BeatLoop {
        deck: usize,
        beats: f64,
    },
    LoopExit(usize),
    SetTempo {
        deck: usize,
        percent: f64,
    },
    SetTempoRange {
        deck: usize,
        range: f64,
    },
    Sync {
        deck: usize,
        on: bool,
    },
    SetMaster(usize),
    Quantize(bool),
    Mixer(MixerUpdate),
}

/// Changes to the mixer; `None` leaves a setting alone.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MixerUpdate {
    /// Deck (0 or 1) that `volume` and `eq` apply to.
    pub deck: Option<usize>,
    pub volume: Option<f32>,
    /// low, mid, high in dB.
    pub eq: [Option<f32>; 3],
    pub crossfader: Option<f32>,
    pub master_volume: Option<f32>,
}

/// Things to free off the audio thread.
#[expect(dead_code, reason = "held only so they're dropped by the event thread")]
pub(crate) enum Garbage {
    Track(Arc<TrackPcm>),
    Grid(Box<Grid>),
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct DeckSnapshot {
    pub load_id: u64,
    pub playing: bool,
    pub position: f64,
    pub rate: f64,
    pub tempo_percent: f64,
    pub tempo_range: f64,
    pub bpm: Option<f64>,
    pub original_bpm: Option<f64>,
    pub sync: bool,
    pub master: bool,
    pub cue: f64,
    pub active_loop: Option<Loop>,
    pub hot_cues: [Option<HotCue>; HOT_CUES],
    pub jumps: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Snapshot {
    pub decks: [DeckSnapshot; 2],
    pub mixer: MixerSettings,
    pub quantize: bool,
    /// Sequence number of the last command applied.
    pub applied_seq: u64,
    /// Engine clock when `decks[_].position` is heard.
    pub at_ms: f64,
}

pub(crate) struct Core {
    decks: [Deck; 2],
    mixer: Mixer,
    master: Option<usize>,
    quantize: bool,
    device_rate: f64,
    commands: Consumer<(u64, Control)>,
    garbage: Producer<Garbage>,
    state: triple_buffer::Input<Snapshot>,
    applied_seq: u64,
    deck_buffers: [[f32; SUB_BLOCK]; 4],
    block: Box<[f32]>,
    frames_rendered: u64,
}

impl Core {
    pub(crate) fn new(
        device_rate: u32,
        commands: Consumer<(u64, Control)>,
        garbage: Producer<Garbage>,
        state: triple_buffer::Input<Snapshot>,
    ) -> Self {
        let mut core = Self {
            decks: [Deck::default(), Deck::default()],
            mixer: Mixer::new(device_rate as f64),
            master: None,
            quantize: true,
            device_rate: device_rate as f64,
            commands,
            garbage,
            state,
            applied_seq: 0,
            deck_buffers: [[0.0; SUB_BLOCK]; 4],
            block: vec![0.0; MAX_BLOCK * 2].into_boxed_slice(),
            frames_rendered: 0,
        };
        core.publish(0.0);
        core
    }

    pub(crate) fn device_rate(&self) -> u32 {
        self.device_rate as u32
    }

    /// Changes the output rate, e.g. for a new device. Not for the audio
    /// thread: call it while no stream is running.
    pub(crate) fn set_device_rate(&mut self, rate: u32) {
        self.device_rate = rate as f64;
        self.mixer.set_sample_rate(rate as f64);
    }

    /// Engine clock for the offline engine: time rendered so far.
    pub(crate) fn rendered_ms(&self) -> f64 {
        self.frames_rendered as f64 / self.device_rate * 1000.0
    }

    /// Applies queued commands and publishes the resulting state.
    pub(crate) fn update(&mut self, at_ms: f64) {
        self.process_commands();
        self.update_rates();
        self.publish(at_ms);
    }

    /// Renders up to [`MAX_BLOCK`] frames heard from `at_ms` on and returns
    /// them as interleaved stereo.
    pub(crate) fn render_block(&mut self, frames: usize, at_ms: f64) -> &[f32] {
        let frames = frames.min(MAX_BLOCK);
        self.update(at_ms);
        let mut start = 0;
        while start < frames {
            let n = (frames - start).min(SUB_BLOCK);
            if start > 0 {
                self.update_rates();
            }
            let [l0, r0, l1, r1] = &mut self.deck_buffers;
            let rate = self.device_rate;
            let ended = [
                self.decks[0].render(&mut l0[..n], &mut r0[..n], rate),
                self.decks[1].render(&mut l1[..n], &mut r1[..n], rate),
            ];
            self.mixer.process(
                [(&l0[..n], &r0[..n]), (&l1[..n], &r1[..n])],
                &mut self.block[2 * start..2 * (start + n)],
            );
            for (d, ended) in ended.into_iter().enumerate() {
                if ended {
                    self.on_stopped(d);
                }
            }
            start += n;
        }
        self.frames_rendered += frames as u64;
        &self.block[..2 * frames]
    }

    /// Fills interleaved stereo `out`, the first frame heard at `at_ms`.
    pub(crate) fn render_interleaved(&mut self, out: &mut [f32], at_ms: f64) {
        let ms_per_frame = 1000.0 / self.device_rate;
        let mut done = 0;
        for chunk in out.chunks_mut(MAX_BLOCK * 2) {
            let frames = chunk.len() / 2;
            let block = self.render_block(frames, at_ms + done as f64 * ms_per_frame);
            chunk[..frames * 2].copy_from_slice(block);
            done += frames;
        }
    }

    fn process_commands(&mut self) {
        while let Ok((seq, control)) = self.commands.pop() {
            self.apply(control);
            self.applied_seq = seq;
        }
    }

    fn apply(&mut self, control: Control) {
        let quantize = self.quantize;
        match control {
            Control::Load { deck, track } => {
                let (pcm, grid) = self.decks[deck].load(track);
                if let Some(pcm) = pcm {
                    self.discard(Garbage::Track(pcm));
                }
                if let Some(grid) = grid {
                    self.discard(Garbage::Grid(grid));
                }
                self.hand_over_master(deck);
            }
            Control::Play(deck) => {
                if !self.decks[deck].loaded() {
                    return;
                }
                self.decks[deck].play();
                let master_gone = self.master.is_none_or(|m| !self.decks[m].loaded());
                if self.decks[deck].play_latched && master_gone {
                    self.make_master(deck);
                }
            }
            Control::Pause(deck) => {
                self.decks[deck].pause();
                self.on_stopped(deck);
            }
            Control::Cue { deck, press } => {
                if press {
                    self.decks[deck].cue_press(quantize);
                    self.on_stopped(deck);
                } else {
                    self.decks[deck].cue_release();
                }
            }
            Control::HotCue { deck, slot, press } => {
                if press {
                    self.decks[deck].hot_cue_press(slot, quantize);
                } else {
                    self.decks[deck].hot_cue_release(slot);
                }
            }
            Control::Seek { deck, seconds } => self.decks[deck].seek(seconds),
            Control::BeatJump { deck, beats } => self.decks[deck].beat_jump(beats),
            Control::BeatLoop { deck, beats } => self.decks[deck].beat_loop(beats, quantize),
            Control::LoopExit(deck) => self.decks[deck].loop_exit(),
            Control::SetTempo { deck, percent } => {
                if !self.following(deck) {
                    self.decks[deck].set_tempo(percent);
                }
            }
            Control::SetTempoRange { deck, range } => self.decks[deck].set_tempo_range(range),
            Control::Sync { deck, on } => self.set_sync(deck, on),
            Control::SetMaster(deck) => {
                if self.decks[deck].loaded() {
                    self.make_master(deck);
                }
            }
            Control::Quantize(on) => self.quantize = on,
            Control::Mixer(update) => self.update_mixer(update),
        }
    }

    fn discard(&mut self, garbage: Garbage) {
        if let Err(rtrb::PushError::Full(garbage)) = self.garbage.push(garbage) {
            // Never free on the audio thread; a full queue means the engine
            // handle is gone, so leaking is harmless.
            std::mem::forget(garbage);
        }
    }

    /// Sync on: follow the master's tempo and beat phase. If there's no
    /// master yet, the other deck becomes master if it's playing, otherwise
    /// this one. Sync off keeps the tempo sync chose.
    fn set_sync(&mut self, deck: usize, on: bool) {
        if on {
            self.decks[deck].sync = true;
            if self.master.is_none_or(|m| !self.decks[m].loaded()) {
                let other = 1 - deck;
                let other_playing = self.decks[other].loaded() && self.decks[other].play_latched;
                self.make_master(if other_playing { other } else { deck });
            }
        } else {
            self.keep_synced_tempo(deck);
            self.decks[deck].sync = false;
        }
    }

    fn make_master(&mut self, deck: usize) {
        if self.master != Some(deck) {
            self.keep_synced_tempo(deck);
            self.master = Some(deck);
        }
    }

    /// A deck that stops following the master keeps the tempo sync gave it
    /// rather than jumping back to its fader.
    fn keep_synced_tempo(&mut self, deck: usize) {
        if self.following(deck) {
            let d = &mut self.decks[deck];
            d.tempo_percent = ((d.nominal_rate - 1.0) * 100.0).clamp(-100.0, 100.0);
        }
    }

    /// Whether `deck` takes its tempo from the master.
    fn following(&self, deck: usize) -> bool {
        self.decks[deck].sync
            && self.decks[deck].grid.is_some()
            && self.master.is_some_and(|m| {
                m != deck && self.decks[m].loaded() && self.decks[m].grid.is_some()
            })
    }

    /// A deck stopped: if it was master and the other deck is playing, the
    /// other deck takes over.
    fn on_stopped(&mut self, deck: usize) {
        if !self.decks[deck].play_latched {
            self.hand_over_master(deck);
        }
    }

    fn hand_over_master(&mut self, deck: usize) {
        let other = 1 - deck;
        if self.master == Some(deck) && self.decks[other].loaded() && self.decks[other].play_latched
        {
            self.make_master(other);
        }
    }

    fn update_mixer(&mut self, update: MixerUpdate) {
        let settings = &mut self.mixer.settings;
        if let Some(deck) = update.deck.filter(|&d| d < 2) {
            let channel = &mut settings.channels[deck];
            if let Some(volume) = update.volume {
                channel.volume = volume.clamp(0.0, 1.0);
            }
            for (band, db) in update.eq.into_iter().enumerate() {
                if let Some(db) = db {
                    channel.eq_db[band] = db.clamp(EQ_KILL_DB, EQ_MAX_DB);
                }
            }
        }
        if let Some(x) = update.crossfader {
            settings.crossfader = x.clamp(-1.0, 1.0);
        }
        if let Some(volume) = update.master_volume {
            settings.master_volume = volume.clamp(0.0, 1.0);
        }
    }

    /// Sets each deck's tempo factor: the fader's, or for a synced deck the
    /// master's effective BPM over this track's BPM, plus a small phase
    /// correction (or a jump, for large errors) while both decks play.
    fn update_rates(&mut self) {
        for deck in 0..2 {
            let fader = self.decks[deck].fader_rate();
            let follow = self
                .master
                .filter(|_| self.following(deck))
                .and_then(|m| self.sync_target(deck, m));
            let Some(target) = follow else {
                self.decks[deck].nominal_rate = fader;
                self.decks[deck].rate = fader;
                continue;
            };
            let mut correction = 0.0;
            if let Some(error_beats) = target.phase_error {
                let d = &mut self.decks[deck];
                let beat_seconds = 60.0 / (target.track_bpm * target.rate);
                let error = error_beats * beat_seconds;
                if error.abs() > SYNC_SNAP_SECONDS {
                    if let Some(mut to) = d.grid.as_ref().map(|g| g.shift(d.pos, error_beats)) {
                        // Stay inside a loop: it's whole beats, so wrapping
                        // keeps the phase.
                        if let Some(lp) = d.active_loop
                            && (to < lp.start || to >= lp.end)
                        {
                            to = lp.start + (to - lp.start).rem_euclid(lp.end - lp.start);
                        }
                        d.jump_to(to);
                    }
                } else if error.abs() > SYNC_DEADBAND_SECONDS {
                    let excess = error.abs() - SYNC_DEADBAND_SECONDS;
                    correction = (excess.copysign(error) / SYNC_CORRECTION_SECONDS)
                        .clamp(-SYNC_MAX_CORRECTION, SYNC_MAX_CORRECTION);
                }
            }
            let d = &mut self.decks[deck];
            d.nominal_rate = target.rate;
            d.rate = target.rate * (1.0 + correction);
        }
    }

    fn sync_target(&self, deck: usize, master: usize) -> Option<SyncTarget> {
        let slave = &self.decks[deck];
        let lead = &self.decks[master];
        let slave_grid = slave.grid.as_ref()?;
        let lead_grid = lead.grid.as_ref()?;
        let track_bpm = slave_grid.bpm_at(slave.pos);
        let lead_bpm = lead_grid.bpm_at(lead.pos) * lead.fader_rate();
        if track_bpm <= 0.0 || lead_bpm <= 0.0 {
            return None;
        }
        // Match half or double time when that's closer to the track's own
        // tempo (e.g. 70 BPM hip hop against 140 BPM).
        let factor = [0.5, 1.0, 2.0]
            .into_iter()
            .min_by(|a, b| {
                let da = (lead_bpm * a / track_bpm).ln().abs();
                let db = (lead_bpm * b / track_bpm).ln().abs();
                da.total_cmp(&db)
            })
            .unwrap_or(1.0);
        let rate = lead_bpm * factor / track_bpm;
        let lock_phase = slave.play_latched
            && !slave.stalled
            && loop_keeps_phase(slave)
            && lead.moving()
            && !lead.stalled
            && loop_keeps_phase(lead);
        let phase_error = lock_phase.then(|| {
            let target = lead_grid.beat_at(lead.pos) * factor;
            wrap_phase(target - slave_grid.beat_at(slave.pos))
        });
        Some(SyncTarget {
            rate,
            track_bpm,
            phase_error,
        })
    }

    fn publish(&mut self, at_ms: f64) {
        let snapshot = Snapshot {
            decks: [self.deck_snapshot(0), self.deck_snapshot(1)],
            mixer: self.mixer.settings,
            quantize: self.quantize,
            applied_seq: self.applied_seq,
            at_ms,
        };
        self.state.write(snapshot);
    }

    fn deck_snapshot(&self, deck: usize) -> DeckSnapshot {
        let d = &self.decks[deck];
        let following = self.following(deck);
        let original_bpm = d.original_bpm();
        DeckSnapshot {
            load_id: d.load_id,
            playing: d.moving(),
            position: d.pos,
            rate: if d.moving() && !d.stalled {
                d.nominal_rate
            } else {
                0.0
            },
            tempo_percent: if following {
                (d.nominal_rate - 1.0) * 100.0
            } else {
                d.tempo_percent
            },
            tempo_range: d.tempo_range,
            bpm: original_bpm.map(|bpm| bpm * d.nominal_rate),
            original_bpm,
            sync: d.sync,
            master: self.master == Some(deck),
            cue: d.cue,
            active_loop: d.active_loop,
            hot_cues: d.hot_cues,
            jumps: d.jumps,
        }
    }
}

/// Whether the deck's beat phase runs on steadily: no loop, or a loop of
/// whole beats. A ½-beat loop restarts the phase every wrap, so sync leaves
/// phase alone (tempo stays matched) until the loop ends.
fn loop_keeps_phase(deck: &Deck) -> bool {
    match (deck.active_loop, &deck.grid) {
        (None, _) => true,
        (Some(lp), Some(grid)) => {
            let beats = grid.beat_at(lp.end) - grid.beat_at(lp.start);
            beats > 0.99 && (beats - beats.round()).abs() < 0.01
        }
        (Some(_), None) => false,
    }
}

struct SyncTarget {
    /// Tempo factor that matches the master.
    rate: f64,
    /// This track's grid BPM at the read head.
    track_bpm: f64,
    /// Beats to move forward to be in phase, when phase locking applies.
    phase_error: Option<f64>,
}
