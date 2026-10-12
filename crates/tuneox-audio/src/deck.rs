//! One deck: the loaded track, transport (CDJ-style CUE, hot cues, loops,
//! beat jump), tempo, and the resampling read head.
//!
//! Positions are seconds from the start of the track (`f64`, ~1e-12 s
//! resolution), independent of the file's sample rate. The read head
//! advances `rate / device_rate` seconds per output sample and reads the
//! track with 4-point cubic (Catmull-Rom) interpolation, which resamples
//! track rate → device rate and applies the tempo in one step. Pitch follows
//! tempo (vinyl mode).
//!
//! Behavior that rekordbox research may change lives in small functions
//! named after the situation (`cue_press_while_playing`, …) and is listed
//! under "Behavior to confirm" in docs/specs/decks.md.

use std::sync::Arc;

use crate::grid::Grid;
use crate::pcm::TrackPcm;

pub const HOT_CUES: usize = 8;

/// Positions closer than this are the same position (e.g. "at the cue").
const SAME_POSITION: f64 = 0.001;
/// Output samples over which jumps, starts and stops are faded.
pub(crate) const FADE: u32 = 128;
/// A nanosecond: far below a sample, far above `f64` rounding of positions.
const WRAP_EPSILON: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HotCue {
    pub start: f64,
    /// End of a hot loop.
    pub end: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Loop {
    pub start: f64,
    pub end: f64,
}

impl Loop {
    fn len(&self) -> f64 {
        self.end - self.start
    }

    fn contains(&self, t: f64) -> bool {
        t >= self.start - SAME_POSITION && t < self.end + SAME_POSITION
    }
}

/// CUE or a hot cue held while the deck was paused: plays until released,
/// then returns.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Preview {
    return_to: f64,
    button: Button,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Button {
    Cue,
    HotCue(usize),
}

/// The previous read head, fading out after a jump or stop.
#[derive(Debug, Clone, Copy)]
struct Tail {
    pos: f64,
    step: f64,
    remaining: u32,
}

/// Everything a deck needs from a load, built off the audio thread.
pub(crate) struct LoadedTrack {
    pub load_id: u64,
    pub pcm: Arc<TrackPcm>,
    pub grid: Option<Box<Grid>>,
    pub hot_cues: [Option<HotCue>; HOT_CUES],
    pub cue: f64,
}

pub(crate) struct Deck {
    pub load_id: u64,
    pub pcm: Option<Arc<TrackPcm>>,
    pub grid: Option<Box<Grid>>,

    pub pos: f64,
    pub play_latched: bool,
    preview: Option<Preview>,
    pub cue: f64,
    pub hot_cues: [Option<HotCue>; HOT_CUES],
    pub active_loop: Option<Loop>,
    pub at_end: bool,

    pub tempo_percent: f64,
    pub tempo_range: f64,
    pub sync: bool,
    /// Tempo factor without sync's phase correction.
    pub nominal_rate: f64,
    /// Tempo factor the read head moves at.
    pub rate: f64,

    /// Counts position discontinuities (jumps, loop wraps, loads).
    pub jumps: u32,
    /// Waiting for the decoder to reach the read head.
    pub stalled: bool,
    /// The read head produced sound on the last sample.
    audible: bool,
    fade_in: u32,
    tail: Option<Tail>,
    last_step: f64,
}

impl Default for Deck {
    fn default() -> Self {
        Self {
            load_id: 0,
            pcm: None,
            grid: None,
            pos: 0.0,
            play_latched: false,
            preview: None,
            cue: 0.0,
            hot_cues: [None; HOT_CUES],
            active_loop: None,
            at_end: false,
            tempo_percent: 0.0,
            tempo_range: 10.0,
            sync: false,
            nominal_rate: 1.0,
            rate: 1.0,
            jumps: 0,
            stalled: false,
            audible: false,
            fade_in: 0,
            tail: None,
            last_step: 0.0,
        }
    }
}

impl Deck {
    pub fn loaded(&self) -> bool {
        self.pcm.is_some()
    }

    /// Audio is moving: playing, or previewing a held CUE/hot cue.
    pub fn moving(&self) -> bool {
        self.play_latched || self.preview.is_some()
    }

    pub fn duration(&self) -> f64 {
        self.pcm.as_ref().map_or(0.0, |p| p.duration_secs())
    }

    /// The grid's tempo here, if the track has a grid.
    pub fn original_bpm(&self) -> Option<f64> {
        self.grid.as_ref().map(|g| g.bpm_at(self.pos))
    }

    /// Replaces the track. Returns the old one so the caller can free it off
    /// the audio thread.
    pub fn load(&mut self, track: LoadedTrack) -> (Option<Arc<TrackPcm>>, Option<Box<Grid>>) {
        let old = (self.pcm.take(), self.grid.take());
        self.load_id = track.load_id;
        self.pcm = Some(track.pcm);
        self.grid = track.grid;
        self.hot_cues = track.hot_cues;
        self.cue = track.cue;
        self.pos = track.cue;
        self.play_latched = false;
        self.preview = None;
        self.active_loop = None;
        self.at_end = false;
        self.stalled = false;
        // The old track's audio is about to be freed: cut it rather than fade.
        self.tail = None;
        self.audible = false;
        self.fade_in = 0;
        self.jumps = self.jumps.wrapping_add(1);
        old
    }

    // Transport -----------------------------------------------------------

    pub fn play(&mut self) {
        if self.preview.take().is_some() {
            // PLAY while holding CUE/hot cue: keep playing after release.
            self.play_latched = true;
            return;
        }
        if self.play_latched || self.at_track_end() {
            return;
        }
        self.start_moving();
        self.play_latched = true;
    }

    pub fn pause(&mut self) {
        if self.moving() {
            self.stop_moving();
        }
        self.play_latched = false;
        self.preview = None;
    }

    pub fn cue_press(&mut self, quantize: bool) {
        if self.play_latched {
            self.cue_press_while_playing();
        } else if self.preview.is_none() {
            self.cue_press_while_paused(quantize);
        }
    }

    /// CDJ "back cue": return to the cue point and pause.
    fn cue_press_while_playing(&mut self) {
        self.stop_moving();
        self.play_latched = false;
        let cue = self.cue;
        self.jump_to(cue);
    }

    /// Paused at the cue point: play while held (cue point sampler).
    /// Paused elsewhere: set the cue point here (snapped to the nearest beat
    /// with quantize) and move to it.
    fn cue_press_while_paused(&mut self, quantize: bool) {
        if (self.pos - self.cue).abs() < SAME_POSITION {
            self.start_preview(self.cue, Button::Cue);
        } else {
            let cue = self.snap(self.pos, quantize);
            self.cue = cue;
            self.jump_to(cue);
        }
    }

    pub fn cue_release(&mut self) {
        self.end_preview(Button::Cue);
    }

    pub fn hot_cue_press(&mut self, slot: usize, quantize: bool) {
        let Some(hot_cue) = self.hot_cues.get(slot).copied() else {
            return;
        };
        match hot_cue {
            None => self.hot_cue_set(slot, quantize),
            Some(hot_cue) if self.play_latched => {
                self.hot_cue_press_while_playing(hot_cue, quantize)
            }
            Some(hot_cue) => self.hot_cue_press_while_paused(slot, hot_cue),
        }
    }

    /// An empty pad stores the current position (snapped with quantize).
    fn hot_cue_set(&mut self, slot: usize, quantize: bool) {
        let start = self.snap(self.pos, quantize);
        self.hot_cues[slot] = Some(HotCue { start, end: None });
    }

    /// Jump and keep playing. With quantize the jump keeps the beat phase:
    /// landing as far before/after the hot cue as the read head was from its
    /// nearest beat, so a synced or beatmatched deck stays in time.
    fn hot_cue_press_while_playing(&mut self, hot_cue: HotCue, quantize: bool) {
        let target = match (&self.grid, quantize) {
            (Some(grid), true) => {
                let offset = crate::grid::wrap_phase(grid.beat_at(self.pos));
                grid.shift(hot_cue.start, offset)
            }
            _ => hot_cue.start,
        };
        self.activate_hot_loop(hot_cue);
        self.jump_to(target);
    }

    /// Paused: go to the hot cue and play while the pad is held; release
    /// returns to it and pauses.
    fn hot_cue_press_while_paused(&mut self, slot: usize, hot_cue: HotCue) {
        self.activate_hot_loop(hot_cue);
        self.jump_to(hot_cue.start);
        self.start_preview(hot_cue.start, Button::HotCue(slot));
    }

    fn activate_hot_loop(&mut self, hot_cue: HotCue) {
        if let Some(end) = hot_cue.end.filter(|&end| end > hot_cue.start) {
            self.active_loop = Some(Loop {
                start: hot_cue.start,
                end,
            });
        }
    }

    pub fn hot_cue_release(&mut self, slot: usize) {
        self.end_preview(Button::HotCue(slot));
    }

    pub fn seek(&mut self, t: f64) {
        self.jump_to(t);
    }

    /// Moves by `beats` on the grid, keeping the phase. Inside an active
    /// loop the loop moves with the read head (CDJ-3000 loop move).
    pub fn beat_jump(&mut self, beats: f64) {
        let Some(grid) = &self.grid else { return };
        let target = grid.shift(self.pos, beats);
        if let Some(lp) = self.active_loop.filter(|lp| lp.contains(self.pos)) {
            let start = grid.shift(lp.start, beats);
            let end = grid.shift(lp.end, beats);
            self.active_loop = (start >= 0.0).then_some(Loop { start, end });
        }
        self.jump_to(target);
    }

    /// Auto beat loop of `beats` from the nearest beat (with quantize) or
    /// from here. With a loop already active it's resized from its start.
    pub fn beat_loop(&mut self, beats: f64, quantize: bool) {
        let Some(grid) = &self.grid else { return };
        let start = match self.active_loop {
            Some(lp) => lp.start,
            None => {
                if quantize {
                    grid.nearest_beat(self.pos)
                } else {
                    self.pos
                }
            }
        };
        let end = grid.shift(start, beats);
        if end <= start {
            return;
        }
        let lp = Loop { start, end };
        self.active_loop = Some(lp);
        if self.pos >= end {
            // Past the new end (e.g. a shorter loop): wrap into it in phase.
            let wrapped = start + (self.pos - start).rem_euclid(lp.len());
            self.jump_to(wrapped);
        }
    }

    pub fn loop_exit(&mut self) {
        self.active_loop = None;
    }

    // Tempo ---------------------------------------------------------------

    pub fn set_tempo(&mut self, percent: f64) {
        self.tempo_percent = percent.clamp(-self.tempo_range, self.tempo_range);
    }

    /// Changes the fader range, keeping the tempo where it fits.
    pub fn set_tempo_range(&mut self, range: f64) {
        self.tempo_range = range;
        self.tempo_percent = self.tempo_percent.clamp(-range, range);
    }

    pub fn fader_rate(&self) -> f64 {
        1.0 + self.tempo_percent / 100.0
    }

    // Helpers -------------------------------------------------------------

    fn snap(&self, t: f64, quantize: bool) -> f64 {
        match (&self.grid, quantize) {
            (Some(grid), true) => grid.nearest_beat(t).max(0.0),
            _ => t,
        }
    }

    fn at_track_end(&self) -> bool {
        self.pcm.as_ref().is_some_and(|p| p.is_done()) && self.pos >= self.duration()
    }

    /// Moves the read head. Leaving the active loop ends it.
    pub fn jump_to(&mut self, t: f64) {
        let duration = self.duration();
        let mut t = t.max(0.0);
        if self.pcm.as_ref().is_some_and(|p| p.is_done()) {
            t = t.min(duration);
        }
        if (t - self.pos).abs() < 1e-9 {
            return;
        }
        if self.active_loop.is_some_and(|lp| !lp.contains(t)) {
            self.active_loop = None;
        }
        if self.audible {
            self.tail = Some(Tail {
                pos: self.pos,
                step: self.last_step,
                remaining: FADE,
            });
            self.fade_in = FADE;
        }
        self.pos = t;
        self.at_end = false;
        self.jumps = self.jumps.wrapping_add(1);
    }

    fn start_moving(&mut self) {
        if !self.audible {
            self.fade_in = FADE;
        }
        self.at_end = false;
    }

    fn stop_moving(&mut self) {
        if self.audible {
            self.tail = Some(Tail {
                pos: self.pos,
                step: self.last_step,
                remaining: FADE,
            });
        }
        self.audible = false;
    }

    fn start_preview(&mut self, return_to: f64, button: Button) {
        if !self.moving() {
            self.start_moving();
        }
        self.preview = Some(Preview { return_to, button });
    }

    fn end_preview(&mut self, button: Button) {
        let Some(preview) = self.preview.filter(|p| p.button == button) else {
            return;
        };
        self.stop_moving();
        self.preview = None;
        self.jump_to(preview.return_to);
    }

    // Rendering -----------------------------------------------------------

    /// Renders `left.len()` samples at `device_rate` and advances the read
    /// head. Returns true if the track ended during this block.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32], device_rate: f64) -> bool {
        left.fill(0.0);
        right.fill(0.0);
        // Moved out and back rather than cloned: cloning an `Arc` here could
        // make the audio thread the one that frees it.
        let Some(pcm) = self.pcm.take() else {
            self.audible = false;
            return false;
        };
        let ended = self.render_track(&pcm, left, right, device_rate);
        self.pcm = Some(pcm);
        ended
    }

    fn render_track(
        &mut self,
        pcm: &TrackPcm,
        left: &mut [f32],
        right: &mut [f32],
        device_rate: f64,
    ) -> bool {
        let sample_rate = pcm.sample_rate() as f64;
        if sample_rate == 0.0 {
            self.stalled = self.moving();
            return false;
        }
        let readable = pcm.decoded_frames();
        let done = pcm.is_done();
        let end = if done {
            readable as f64 / sample_rate
        } else {
            f64::INFINITY
        };
        let step = self.rate / device_rate;
        let moving = self.moving();
        let mut ended = false;
        self.stalled = false;

        for i in 0..left.len() {
            let (mut l, mut r) = (0.0, 0.0);
            if let Some(tail) = &mut self.tail {
                let gain = tail.remaining as f32 / FADE as f32;
                let (tl, tr) = read(pcm, sample_rate, readable, tail.pos);
                l += tl * gain;
                r += tr * gain;
                tail.pos += tail.step;
                tail.remaining -= 1;
                if tail.remaining == 0 {
                    self.tail = None;
                }
            }
            if moving && !ended && !self.stalled {
                let frame = (self.pos * sample_rate).floor();
                if !done && frame + 3.0 > readable as f64 {
                    // The decoder hasn't got here yet: wait for it.
                    self.stalled = true;
                    self.audible = false;
                    self.fade_in = FADE;
                } else {
                    let (ml, mr) = read(pcm, sample_rate, readable, self.pos);
                    // Fades in from silence: 0 on the first sample, 1 after FADE.
                    let gain = 1.0 - self.fade_in as f32 / FADE as f32;
                    self.fade_in = self.fade_in.saturating_sub(1);
                    l += ml * gain;
                    r += mr * gain;
                    self.audible = true;
                    self.last_step = step;
                    let before = self.pos;
                    self.pos += step;
                    if let Some(lp) = self.active_loop
                        && before < lp.end - WRAP_EPSILON
                        && self.pos >= lp.end - WRAP_EPSILON
                    {
                        // Sample-accurate wrap that keeps the fractional
                        // overshoot, so the loop length is exact. The
                        // epsilon absorbs rounding in the accumulated
                        // position, so a loop of whole samples wraps on the
                        // sample it should.
                        let over = (self.pos - lp.end).max(0.0);
                        self.pos = lp.start + over % lp.len();
                        self.jumps = self.jumps.wrapping_add(1);
                    }
                    if self.pos >= end {
                        self.pos = end;
                        self.at_end = true;
                        self.play_latched = false;
                        self.preview = None;
                        self.audible = false;
                        ended = true;
                    }
                }
            }
            left[i] = l;
            right[i] = r;
        }
        if !moving {
            self.audible = false;
        }
        ended
    }
}

/// The track at `t` seconds, cubic-interpolated.
#[inline]
fn read(pcm: &TrackPcm, sample_rate: f64, readable: usize, t: f64) -> (f32, f32) {
    let x = t * sample_rate;
    let base = x.floor();
    let frac = (x - base) as f32;
    let i = base as i64;
    let a = pcm.frame(i - 1, readable);
    let b = pcm.frame(i, readable);
    let c = pcm.frame(i + 1, readable);
    let d = pcm.frame(i + 2, readable);
    (
        hermite(a.0, b.0, c.0, d.0, frac),
        hermite(a.1, b.1, c.1, d.1, frac),
    )
}

/// 4-point, 3rd-order Hermite (Catmull-Rom) interpolation between `y0` and
/// `y1` at `t` in 0…1.
#[inline]
fn hermite(ym1: f32, y0: f32, y1: f32, y2: f32, t: f32) -> f32 {
    let c1 = 0.5 * (y1 - ym1);
    let c2 = ym1 - 2.5 * y0 + 2.0 * y1 - 0.5 * y2;
    let c3 = 0.5 * (y2 - ym1) + 1.5 * (y0 - y1);
    ((c3 * t + c2) * t + c1) * t + y0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hermite_hits_the_samples_and_lines() {
        assert_eq!(hermite(1.0, 2.0, 3.0, 4.0, 0.0), 2.0);
        assert!((hermite(1.0, 2.0, 3.0, 4.0, 1.0) - 3.0).abs() < 1e-6);
        assert!((hermite(1.0, 2.0, 3.0, 4.0, 0.25) - 2.25).abs() < 1e-6);
    }
}
