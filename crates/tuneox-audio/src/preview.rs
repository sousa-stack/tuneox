//! The browser's preview player (rekordbox's Preview column, research
//! §1.5): a third, simple voice that plays one track from a position at its
//! own auto gain, straight into the master bus. No tempo, sync, cues or
//! loops; it reads at the track's own speed, resampled to the device rate.
//!
//! Like a deck it never allocates or frees on the audio thread: a replaced
//! track keeps fading out from its own audio and is then handed back to the
//! core, which sends it off to be freed.

use std::sync::Arc;

use crate::deck::FADE;
use crate::pcm::TrackPcm;

/// A track for the preview voice, built off the audio thread.
pub(crate) struct PreviewTrack {
    pub load_id: u64,
    pub pcm: Arc<TrackPcm>,
    /// The track's auto gain.
    pub gain_db: f32,
}

/// The previous read head fading out after a jump, a stop or a new track.
struct Tail {
    /// The audio it reads: the replaced track, or `None` for the current one.
    pcm: Option<Arc<TrackPcm>>,
    pos: f64,
    gain: f32,
    remaining: u32,
}

#[derive(Default)]
pub(crate) struct PreviewVoice {
    pub load_id: u64,
    pub pcm: Option<Arc<TrackPcm>>,
    pub pos: f64,
    pub playing: bool,
    /// Waiting for the decoder to reach the read head.
    pub stalled: bool,
    pub gain_db: f32,
    gain: f32,
    audible: bool,
    fade_in: u32,
    tail: Option<Tail>,
    /// A replaced track whose fade-out has finished, waiting to be freed off
    /// the audio thread.
    finished: Option<Arc<TrackPcm>>,
}

impl PreviewVoice {
    /// Plays from `position` seconds; with a `track`, that track replaces
    /// the current one. Returns a track to free (one replaced while silent,
    /// or a fade-out the new one cut short).
    pub fn play(&mut self, track: Option<PreviewTrack>, position: f64) -> Option<Arc<TrackPcm>> {
        let position = position.max(0.0);
        let mut free = None;
        let mut restart = !self.audible;
        match track {
            Some(track) => {
                let old = self.pcm.replace(track.pcm);
                if self.audible {
                    // The old track fades out from its own audio.
                    free = self.start_tail(old);
                    restart = true;
                } else if let Some(tail) = self.tail.as_mut().filter(|t| t.pcm.is_none()) {
                    // A stop's fade-out is still reading the old track.
                    tail.pcm = old;
                } else {
                    free = old;
                }
                self.load_id = track.load_id;
                self.gain_db = track.gain_db;
                self.gain = db_to_gain(track.gain_db);
            }
            None => {
                if self.audible && (position - self.pos).abs() > 1e-9 {
                    free = self.start_tail(None);
                    restart = true;
                }
            }
        }
        self.pos = position;
        self.playing = self.pcm.is_some();
        self.stalled = false;
        if restart {
            self.fade_in = FADE;
            self.audible = false;
        }
        free
    }

    /// Stops with a short fade, keeping the track and position.
    pub fn stop(&mut self) -> Option<Arc<TrackPcm>> {
        let free = if self.audible {
            self.start_tail(None)
        } else {
            None
        };
        self.playing = false;
        self.stalled = false;
        self.audible = false;
        free
    }

    /// A replaced track that has finished fading out.
    pub fn take_finished(&mut self) -> Option<Arc<TrackPcm>> {
        self.finished.take()
    }

    /// Starts fading out the read head as it is; returns the audio of a
    /// fade-out this one replaces, to be freed.
    fn start_tail(&mut self, pcm: Option<Arc<TrackPcm>>) -> Option<Arc<TrackPcm>> {
        let previous = self.tail.replace(Tail {
            pcm,
            pos: self.pos,
            gain: self.gain,
            remaining: FADE,
        });
        previous.and_then(|t| t.pcm)
    }

    /// Adds the voice into `left`/`right` (which the caller has zeroed).
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32], device_rate: f64) {
        let step = 1.0 / device_rate;
        if let Some(mut tail) = self.tail.take() {
            let pcm = tail.pcm.as_deref().or(self.pcm.as_deref());
            if let Some(pcm) = pcm {
                let (rate, readable) = (pcm.sample_rate() as f64, pcm.decoded_frames());
                for i in 0..left.len() {
                    if tail.remaining == 0 || rate == 0.0 {
                        break;
                    }
                    let g = tail.gain * tail.remaining as f32 / FADE as f32;
                    let (l, r) = read(pcm, rate, readable, tail.pos);
                    left[i] += l * g;
                    right[i] += r * g;
                    tail.pos += step;
                    tail.remaining -= 1;
                }
            } else {
                tail.remaining = 0;
            }
            if tail.remaining > 0 {
                self.tail = Some(tail);
            } else if let Some(old) = tail.pcm {
                self.finished = Some(old);
            }
        }
        let Some(pcm) = self.pcm.take() else {
            self.audible = false;
            return;
        };
        self.render_track(&pcm, left, right, step);
        self.pcm = Some(pcm);
    }

    fn render_track(&mut self, pcm: &TrackPcm, left: &mut [f32], right: &mut [f32], step: f64) {
        self.stalled = false;
        let rate = pcm.sample_rate() as f64;
        if !self.playing {
            self.audible = false;
            return;
        }
        if rate == 0.0 {
            self.stalled = true;
            return;
        }
        let readable = pcm.decoded_frames();
        let done = pcm.is_done();
        let end = readable as f64 / rate;
        for i in 0..left.len() {
            if done && self.pos >= end {
                // The end of the track: stop there.
                self.pos = end;
                self.playing = false;
                self.audible = false;
                return;
            }
            if !done && (self.pos * rate).floor() + 3.0 > readable as f64 {
                self.stalled = true;
                self.audible = false;
                self.fade_in = FADE;
                return;
            }
            let fade = 1.0 - self.fade_in as f32 / FADE as f32;
            self.fade_in = self.fade_in.saturating_sub(1);
            let g = self.gain * fade;
            let (l, r) = read(pcm, rate, readable, self.pos);
            left[i] += l * g;
            right[i] += r * g;
            self.audible = true;
            self.pos += step;
        }
    }
}

pub(crate) fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// The track at `t` seconds, cubic-interpolated.
#[inline]
fn read(pcm: &TrackPcm, sample_rate: f64, readable: usize, t: f64) -> (f32, f32) {
    crate::deck::read(pcm, sample_rate, readable, t)
}
