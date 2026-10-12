//! One deck: the loaded track, transport (CUE, hot cues, memory cues, loops,
//! beat jump), tempo, scrubbing, and the resampling read head.
//!
//! Positions are seconds from the start of the track (`f64`, ~1e-12 s
//! resolution), independent of the file's sample rate. The read head
//! advances `rate / device_rate` seconds per output sample and reads the
//! track with 4-point cubic (Catmull-Rom) interpolation, which resamples
//! track rate → device rate and applies the tempo in one step. Pitch follows
//! tempo (vinyl mode).
//!
//! Transport follows rekordbox 7's EXPORT-mode players as researched in
//! research/rekordbox-decks.md §2. Each rule lives in a small function named
//! after the situation (`cue_press_while_playing`, …) so a correction stays
//! local; open points are listed in docs/specs/decks.md.

use std::sync::Arc;

use crate::grid::Grid;
use crate::pcm::TrackPcm;
pub use crate::state::SyncMode;

pub const HOT_CUES: usize = 8;
/// Memory cues a deck can hold (imported tracks may have more than the 10
/// rekordbox lets you save; see [`MAX_SAVED_MEMORY_CUES`]).
pub const MEMORY_CUES: usize = 32;
/// Memory cues and loops rekordbox allows per track: MEMORY refuses more.
pub const MAX_SAVED_MEMORY_CUES: usize = 10;
/// Auto beat loop and loop resize limits, in beats.
pub const MIN_LOOP_BEATS: f64 = 1.0 / 64.0;
pub const MAX_LOOP_BEATS: f64 = 512.0;
/// Effective tempo limits.
pub const MIN_BPM: f64 = 40.0;
pub const MAX_BPM: f64 = 499.0;
/// Speed change while a pitch bend button is held. rekordbox nudges through
/// its jog-ring speed (`setJogRingSpeed(±0.1)`), whose percent is unknown;
/// 4 % is a clearly audible but controllable nudge.
pub const PITCH_BEND: f64 = 0.04;

/// Positions closer than this are the same position (e.g. "at the cue").
const SAME_POSITION: f64 = 0.001;
/// Output samples over which jumps, starts and stops are faded.
pub(crate) const FADE: u32 = 128;
/// A nanosecond: far below a sample, far above `f64` rounding of positions.
const WRAP_EPSILON: f64 = 1e-9;

// Scrubbing: the read head chases the dragged position like a record moved
// by hand. Its speed is the distance to the target over `SCRUB_FOLLOW`,
// smoothed with time constant `SCRUB_SMOOTH`; FOLLOW = 4 × SMOOTH makes the
// chase critically damped (no overshoot, settles in ~100 ms).

/// Seconds of distance per unit of speed: the chase's proportional term.
pub(crate) const SCRUB_FOLLOW: f64 = 0.040;
/// Time constant of the speed smoothing, in seconds.
pub(crate) const SCRUB_SMOOTH: f64 = 0.010;
/// Fastest a scrubbed read head moves, × normal speed.
pub(crate) const SCRUB_MAX_RATE: f64 = 32.0;
/// A target further away than this is jumped to (with the usual fade).
pub(crate) const SCRUB_JUMP_SECONDS: f64 = 2.0;
/// Below this speed a scrubbed deck is silent (a held record): no DC, no
/// rumble.
const SCRUB_SILENT_RATE: f64 = 0.01;
/// From this speed up a scrubbed deck plays at full level.
const SCRUB_FULL_LEVEL_RATE: f64 = 0.25;
/// A released scrub hands over to playback once its speed is this close
/// (relative) to the deck's tempo.
const SCRUB_HANDOVER: f64 = 0.002;
/// A released scrub on a paused deck stops once this close to its target.
const SCRUB_SETTLED_SECONDS: f64 = 2e-5;
/// A held scrub that hears no command for this long lets go by itself, so a
/// lost pointer-up or a controller that went away can't leave the deck held.
/// The UI re-sends its position while the pointer is down and still.
pub(crate) const SCRUB_IDLE_RELEASE_SECONDS: f64 = 1.5;

/// A stored cue point; with an end it's a loop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HotCue {
    pub start: f64,
    pub end: Option<f64>,
    /// Which library cue this is, as index + 1 into the deck's cue metadata
    /// (colour and comment, kept off the audio thread). 0 for cues stored on
    /// the deck.
    pub tag: u32,
}

impl HotCue {
    pub fn point(start: f64) -> Self {
        Self {
            start,
            end: None,
            tag: 0,
        }
    }

    fn from_loop(lp: &Loop) -> Self {
        Self {
            start: lp.start,
            end: Some(lp.end),
            tag: 0,
        }
    }

    /// The same cue or loop, to the millisecond.
    pub(crate) fn same_place(&self, other: &HotCue) -> bool {
        let ends = match (self.end, other.end) {
            (None, None) => true,
            (Some(a), Some(b)) => (a - b).abs() < SAME_POSITION,
            _ => false,
        };
        ends && (self.start - other.start).abs() < SAME_POSITION
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Loop {
    pub start: f64,
    pub end: f64,
    /// Length in beats for loops made from a beat count (auto beat loops,
    /// resized loops), so pressing the same size again can exit.
    pub beats: Option<f64>,
}

impl Loop {
    fn new(start: f64, end: f64) -> Self {
        Self {
            start,
            end,
            beats: None,
        }
    }

    fn len(&self) -> f64 {
        self.end - self.start
    }

    fn contains(&self, t: f64) -> bool {
        t >= self.start - SAME_POSITION && t < self.end + SAME_POSITION
    }
}

/// Which memory cue to call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryTarget {
    /// 0 = the earliest.
    Index(usize),
    Next,
    Previous,
}

/// CUE (or a hot cue in gate mode) held while the deck was paused: plays
/// until released, then returns.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Held {
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
    /// Level it fades from (below 1 for a slowly scrubbed deck).
    level: f32,
}

/// The deck being dragged like a record (`deck_scrub`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Scrub {
    /// Where the hand has moved the record to.
    target: f64,
    /// Read head speed now, track seconds per second (negative backwards).
    velocity: f64,
    /// Output level for the current speed.
    level: f32,
    /// The hand let go: hand over to playback (if PLAY is latched) or come
    /// to rest at the target.
    released: bool,
    /// Output samples since the last scrub command.
    idle: u32,
}

/// What a scrub command does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrubPhase {
    Start,
    Move,
    End,
}

/// Everything a deck needs from a load, built off the audio thread.
pub(crate) struct LoadedTrack {
    pub load_id: u64,
    pub pcm: Arc<TrackPcm>,
    pub grid: Option<Box<Grid>>,
    pub hot_cues: [Option<HotCue>; HOT_CUES],
    /// Sorted by position.
    pub memory_cues: [Option<HotCue>; MEMORY_CUES],
    /// Auto gain for the track's mixer channel.
    pub gain_db: f32,
}

pub(crate) struct Deck {
    pub load_id: u64,
    pub pcm: Option<Arc<TrackPcm>>,
    pub grid: Option<Box<Grid>>,

    pub pos: f64,
    pub play_latched: bool,
    held: Option<Held>,
    /// None until set: a freshly loaded deck has no cue.
    pub cue: Option<f64>,
    pub hot_cues: [Option<HotCue>; HOT_CUES],
    /// Sorted by position, packed at the front.
    pub memory_cues: [Option<HotCue>; MEMORY_CUES],
    pub active_loop: Option<Loop>,
    /// The most recent loop, the target of RELOOP.
    pub last_loop: Option<Loop>,
    /// The last loop was set after the cue point: MEMORY saves the loop.
    loop_is_latest: bool,
    /// Loop in set with IN, waiting for OUT.
    pub loop_in: Option<f64>,
    pub at_end: bool,
    scrub: Option<Scrub>,

    pub tempo_percent: f64,
    pub tempo_range: f64,
    pub sync: SyncMode,
    pub quantize: bool,
    /// Quantize resolution in beats (rekordbox EXPORT's "Quantize Beat
    /// Value": 1/8, 1/4, 1/2 or 1), the same on both decks.
    pub quantize_beats: f64,
    /// −1 while bending back, +1 forward.
    pub bend: i8,
    /// Tempo factor without sync's phase correction or a pitch bend.
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
            held: None,
            cue: None,
            hot_cues: [None; HOT_CUES],
            memory_cues: [None; MEMORY_CUES],
            active_loop: None,
            last_loop: None,
            loop_is_latest: false,
            loop_in: None,
            at_end: false,
            scrub: None,
            tempo_percent: 0.0,
            tempo_range: 10.0,
            sync: SyncMode::Off,
            quantize: true,
            quantize_beats: 1.0,
            bend: 0,
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

    /// Audio is moving: playing, or a held CUE/hot cue from pause.
    pub fn moving(&self) -> bool {
        self.play_latched || self.held.is_some()
    }

    /// Being dragged like a record.
    pub fn scrubbing(&self) -> bool {
        self.scrub.is_some()
    }

    /// The scrubbed read head's speed, if scrubbing.
    pub fn scrub_velocity(&self) -> Option<f64> {
        self.scrub.map(|s| s.velocity)
    }

    pub fn duration(&self) -> f64 {
        self.pcm.as_ref().map_or(0.0, |p| p.duration_secs())
    }

    /// The grid's tempo here, if the track has a grid.
    pub fn original_bpm(&self) -> Option<f64> {
        self.grid.as_ref().map(|g| g.bpm_at(self.pos))
    }

    /// Length of a loop in beats, if it can be measured.
    pub fn loop_beats(&self, lp: &Loop) -> Option<f64> {
        lp.beats.or_else(|| {
            self.grid
                .as_ref()
                .map(|g| g.beat_at(lp.end) - g.beat_at(lp.start))
        })
    }

    /// Memory cues and loops held now.
    pub fn memory_cue_count(&self) -> usize {
        self.memory_cues.iter().take_while(|c| c.is_some()).count()
    }

    /// Replaces the track and parks at 0:00 with no cue and no loop (EXPORT
    /// mode doesn't jump to the first memory cue). Returns the old track so
    /// the caller can free it off the audio thread.
    pub fn load(&mut self, track: LoadedTrack) -> (Option<Arc<TrackPcm>>, Option<Box<Grid>>) {
        let old = (self.pcm.take(), self.grid.take());
        self.load_id = track.load_id;
        self.pcm = Some(track.pcm);
        self.grid = track.grid;
        self.hot_cues = track.hot_cues;
        self.memory_cues = track.memory_cues;
        self.cue = None;
        self.pos = 0.0;
        self.play_latched = false;
        self.held = None;
        self.active_loop = None;
        self.last_loop = None;
        self.loop_is_latest = false;
        self.loop_in = None;
        self.at_end = false;
        self.scrub = None;
        self.stalled = false;
        self.bend = 0;
        // The old track's audio is about to be freed: cut it rather than fade.
        self.tail = None;
        self.audible = false;
        self.fade_in = 0;
        self.jumps = self.jumps.wrapping_add(1);
        old
    }

    // Transport -----------------------------------------------------------

    pub fn play(&mut self) {
        if self.held.take().is_some() {
            // PLAY while holding CUE: keep playing after release.
            self.play_latched = true;
            return;
        }
        if self.play_latched || (self.at_track_end() && self.scrub.is_none()) {
            return;
        }
        if self.scrub.is_none() {
            self.start_moving();
        }
        // While scrubbing, PLAY only says what happens on release.
        self.play_latched = true;
    }

    pub fn pause(&mut self) {
        let pos = self.pos;
        match &mut self.scrub {
            // A released record stops where it is.
            Some(scrub) if scrub.released => scrub.target = pos,
            Some(_) => {}
            None => {
                if self.moving() {
                    self.stop_moving();
                }
            }
        }
        self.play_latched = false;
        self.held = None;
    }

    pub fn cue_press(&mut self) {
        if self.play_latched {
            self.cue_press_while_playing();
        } else if self.held.is_none() {
            self.cue_press_while_paused();
        }
    }

    /// Back cue: return to the cue point (the start if none is set) and
    /// pause.
    fn cue_press_while_playing(&mut self) {
        self.pause();
        self.jump_to(self.cue.unwrap_or(0.0));
    }

    /// Paused at the cue point: play while held (cue point sampler).
    /// Paused elsewhere: the cue point moves here (snapped with quantize).
    fn cue_press_while_paused(&mut self) {
        match self.cue {
            Some(cue) if (self.pos - cue).abs() < SAME_POSITION => {
                self.start_held(cue, Button::Cue);
            }
            _ => {
                let cue = self.snap(self.pos);
                self.set_cue(cue);
                self.jump_to(cue);
            }
        }
    }

    pub fn cue_release(&mut self) {
        self.end_held(Button::Cue);
    }

    /// A hot cue pad. `gate`: the "Gate playback during Pause" preference.
    pub fn hot_cue_press(&mut self, slot: usize, gate: bool) {
        let Some(hot_cue) = self.hot_cues.get(slot).copied() else {
            return;
        };
        match hot_cue {
            None => self.hot_cue_store(slot),
            Some(hot_cue) if self.play_latched => self.hot_cue_press_while_playing(hot_cue),
            Some(hot_cue) if gate && hot_cue.end.is_none() => {
                self.hot_cue_press_gated(slot, hot_cue)
            }
            Some(hot_cue) => self.hot_cue_press_while_paused(hot_cue),
        }
    }

    /// An empty pad stores the current position (snapped with quantize), or
    /// the loop that's playing as a hot loop.
    fn hot_cue_store(&mut self, slot: usize) {
        self.hot_cues[slot] = Some(match self.active_loop.filter(|lp| lp.contains(self.pos)) {
            Some(lp) => HotCue::from_loop(&lp),
            None => HotCue::point(self.snap(self.pos)),
        });
    }

    /// Clears a pad (rekordbox's ✕ in the HOT CUE list, ⌘1–3).
    pub fn hot_cue_delete(&mut self, slot: usize) {
        if let Some(pad) = self.hot_cues.get_mut(slot) {
            *pad = None;
        }
    }

    /// Jump and keep playing. With quantize the jump keeps the beat phase:
    /// landing as far before/after the hot cue as the read head was from its
    /// nearest grid position at the quantize resolution, so a synced or
    /// beatmatched deck stays in time.
    fn hot_cue_press_while_playing(&mut self, hot_cue: HotCue) {
        let mut target = match (&self.grid, self.quantize) {
            (Some(grid), true) => {
                let offset = crate::grid::wrap_to(grid.beat_at(self.pos), self.quantize_beats);
                grid.shift(hot_cue.start, offset)
            }
            _ => hot_cue.start,
        };
        self.activate_hot_loop(hot_cue);
        if let Some(lp) = self.active_loop.filter(|_| hot_cue.end.is_some())
            && !(lp.start..lp.end).contains(&target)
        {
            // Landing just before a hot loop would leave it at once: take
            // the same phase inside it instead.
            target = lp.start + (target - lp.start).rem_euclid(lp.len());
        }
        self.jump_to(target);
    }

    /// Paused: jump and play (rekordbox's default).
    fn hot_cue_press_while_paused(&mut self, hot_cue: HotCue) {
        self.activate_hot_loop(hot_cue);
        self.jump_to(hot_cue.start);
        if !self.moving() && self.scrub.is_none() {
            self.start_moving();
        }
        // Pressed while CUE was held: the hot cue takes over and latches.
        self.held = None;
        self.play_latched = true;
    }

    /// Paused with gate mode on: play only while held, then return and
    /// pause.
    fn hot_cue_press_gated(&mut self, slot: usize, hot_cue: HotCue) {
        self.jump_to(hot_cue.start);
        self.start_held(hot_cue.start, Button::HotCue(slot));
    }

    fn activate_hot_loop(&mut self, hot_cue: HotCue) {
        if let Some(end) = hot_cue.end.filter(|&end| end > hot_cue.start) {
            self.set_loop(Loop::new(hot_cue.start, end));
        }
    }

    pub fn hot_cue_release(&mut self, slot: usize) {
        self.end_held(Button::HotCue(slot));
    }

    /// Jumps to a memory cue and pauses there; it becomes the cue point. A
    /// memory loop is armed, so PLAY loops it.
    pub fn memory_cue_call(&mut self, target: MemoryTarget) {
        let cues = self.memory_cues.iter().flatten();
        let found = match target {
            MemoryTarget::Index(i) => cues.copied().nth(i),
            MemoryTarget::Next => cues.copied().find(|c| c.start > self.pos + SAME_POSITION),
            MemoryTarget::Previous => cues
                .copied()
                .rev()
                .find(|c| c.start < self.pos - SAME_POSITION),
        };
        let Some(cue) = found else { return };
        self.pause();
        self.set_cue(cue.start);
        match cue.end.filter(|&end| end > cue.start) {
            Some(end) => {
                self.set_loop(Loop::new(cue.start, end));
                self.jump_to(cue.start);
            }
            None => self.jump_to(cue.start),
        }
    }

    /// What MEMORY saves. rekordbox: "the current cue, or the current loop
    /// during or just after loop play". Here: the active loop; else the last
    /// loop if it was set after the cue point (or there's no cue point);
    /// else the cue point.
    pub fn memory_cue_candidate(&self) -> Option<HotCue> {
        match (self.active_loop, self.last_loop, self.cue) {
            (Some(lp), _, _) => Some(HotCue::from_loop(&lp)),
            (None, Some(lp), None) => Some(HotCue::from_loop(&lp)),
            (None, Some(lp), Some(_)) if self.loop_is_latest => Some(HotCue::from_loop(&lp)),
            (None, _, Some(cue)) => Some(HotCue::point(cue)),
            (None, None, None) => None,
        }
    }

    /// MEMORY: saves [`Self::memory_cue_candidate`] among the memory cues
    /// (in position order). Nothing happens if there's nothing to save, the
    /// same cue is already there, or the deck is full.
    pub fn memory_cue_set(&mut self) {
        let Some(cue) = self.memory_cue_candidate() else {
            return;
        };
        let count = self.memory_cue_count();
        let existing = &self.memory_cues[..count];
        if count >= MEMORY_CUES || existing.iter().flatten().any(|c| c.same_place(&cue)) {
            return;
        }
        let at = existing
            .iter()
            .position(|c| c.is_some_and(|c| c.start > cue.start))
            .unwrap_or(count);
        self.memory_cues.copy_within(at..count, at + 1);
        self.memory_cues[at] = Some(cue);
    }

    /// Deletes the memory cue at `index` (position order).
    pub fn memory_cue_delete(&mut self, index: usize) {
        let count = self.memory_cue_count();
        if index < count {
            self.memory_cues.copy_within(index + 1..count, index);
            self.memory_cues[count - 1] = None;
        }
    }

    pub fn seek(&mut self, t: f64) {
        self.jump_to(t);
    }

    /// Moves by `beats` on the grid, keeping the phase. During a loop the
    /// loop moves with the read head.
    pub fn beat_jump(&mut self, beats: f64) {
        let Some(grid) = &self.grid else { return };
        let target = grid.shift(self.pos, beats);
        if let Some(lp) = self.active_loop.filter(|lp| lp.contains(self.pos)) {
            let moved = Loop {
                start: grid.shift(lp.start, beats),
                end: grid.shift(lp.end, beats),
                beats: lp.beats,
            };
            if moved.start >= 0.0 {
                self.set_loop(moved);
            } else {
                self.active_loop = None;
            }
        }
        self.jump_to(target);
    }

    /// Auto beat loop: `beats` from the current position (snapped to the
    /// beat with quantize). The same size again exits; another size resizes
    /// the active loop from its start.
    pub fn beat_loop(&mut self, beats: f64) {
        let Some(grid) = &self.grid else { return };
        if let Some(lp) = self.active_loop
            && lp.beats.is_some_and(|b| (b - beats).abs() < 1e-9)
        {
            self.loop_exit();
            return;
        }
        let start = match self.active_loop {
            Some(lp) => lp.start,
            None => self.snap(self.pos),
        };
        let end = grid.shift(start, beats);
        if end > start {
            self.set_loop_and_wrap(Loop {
                start,
                end,
                beats: Some(beats),
            });
        }
    }

    /// Manual loop IN: sets loop-in here (snapped with quantize) and, like
    /// rekordbox's real-time cue, the cue point.
    pub fn loop_in(&mut self) {
        let t = self.snap(self.pos);
        self.active_loop = None;
        self.loop_in = Some(t);
        self.set_cue(t);
    }

    /// Manual loop OUT: loops from IN to here (snapped with quantize). With
    /// a loop already playing it moves that loop's end here instead.
    pub fn loop_out(&mut self) {
        let start = match (self.loop_in, self.active_loop) {
            (Some(start), _) => start,
            (None, Some(lp)) => lp.start,
            (None, None) => return,
        };
        let mut end = self.snap(self.pos);
        if end <= start + SAME_POSITION {
            // Quantize snapped OUT onto IN: make it a one-beat loop.
            match &self.grid {
                Some(grid) if self.quantize => end = grid.shift(start, 1.0),
                _ => return,
            }
        }
        self.loop_in = None;
        self.set_loop_and_wrap(Loop::new(start, end));
    }

    /// RELOOP: back to the last loop's start, looping again.
    pub fn reloop(&mut self) {
        if let Some(lp) = self.last_loop {
            self.set_loop(lp);
            self.jump_to(lp.start);
        }
    }

    /// Doubles or halves the active loop, anchored at loop-in, within
    /// 1/64–512 beats (by time without a beat grid).
    pub fn loop_resize(&mut self, double: bool) {
        let Some(lp) = self.active_loop else { return };
        let factor = if double { 2.0 } else { 0.5 };
        let resized = match (&self.grid, self.loop_beats(&lp)) {
            (Some(grid), Some(beats)) => {
                let beats = (beats * factor).clamp(MIN_LOOP_BEATS, MAX_LOOP_BEATS);
                Loop {
                    start: lp.start,
                    end: grid.shift(lp.start, beats),
                    beats: Some(beats),
                }
            }
            _ => Loop::new(lp.start, lp.start + lp.len() * factor),
        };
        if resized.end > resized.start {
            self.set_loop_and_wrap(resized);
        }
    }

    /// EXIT: playback runs on past loop-out. RELOOP can bring it back.
    pub fn loop_exit(&mut self) {
        self.active_loop = None;
    }

    fn set_cue(&mut self, t: f64) {
        self.cue = Some(t);
        self.loop_is_latest = false;
    }

    fn set_loop(&mut self, lp: Loop) {
        self.active_loop = Some(lp);
        self.last_loop = Some(lp);
        self.loop_is_latest = true;
    }

    /// Sets a loop and, if the read head is already past its end, wraps it
    /// into the loop at the same phase.
    fn set_loop_and_wrap(&mut self, lp: Loop) {
        self.set_loop(lp);
        if self.pos >= lp.end {
            let wrapped = lp.start + (self.pos - lp.start).rem_euclid(lp.len());
            self.jump_to(wrapped);
        }
    }

    // Scrubbing -----------------------------------------------------------

    /// `deck_scrub`: the deck is dragged like a record. `Start` puts the
    /// hand on it (a playing deck slows to a stop unless dragged on);
    /// `Move` drags it to `target`; `End` lets go at `target`, after which
    /// it plays on if PLAY is latched or comes to rest. `Move` without a
    /// `Start` starts; `End` without one does nothing. A held scrub with no
    /// command for [`SCRUB_IDLE_RELEASE_SECONDS`] ends as if let go.
    pub fn scrub(&mut self, phase: ScrubPhase, target: Option<f64>) {
        let duration = self.duration();
        let clamp = |t: f64| {
            if self.pcm.as_ref().is_some_and(|p| p.is_done()) {
                t.clamp(0.0, duration)
            } else {
                t.max(0.0)
            }
        };
        let target = target.map(clamp);
        match (phase, &mut self.scrub) {
            (ScrubPhase::End, None) => return,
            (ScrubPhase::End, Some(scrub)) => {
                scrub.released = true;
                scrub.idle = 0;
                if let Some(t) = target {
                    scrub.target = t;
                }
            }
            (_, Some(scrub)) => {
                scrub.released = false;
                scrub.idle = 0;
                if let Some(t) = target {
                    scrub.target = t;
                }
            }
            (_, None) => {
                // The record was turning at the deck's speed (also under a
                // held CUE / gated hot cue, which the hand takes over from:
                // it won't play on when let go).
                let moving = self.moving() && !self.stalled && self.audible;
                self.held = None;
                self.scrub = Some(Scrub {
                    target: target.unwrap_or(self.pos),
                    velocity: if moving { self.rate } else { 0.0 },
                    level: if moving { 1.0 } else { 0.0 },
                    released: false,
                    idle: 0,
                });
            }
        }
        let Some(scrub) = self.scrub else { return };
        if self
            .active_loop
            .is_some_and(|lp| !lp.contains(scrub.target))
        {
            // Dragging out of a loop ends it, like a seek.
            self.active_loop = None;
        }
        if (scrub.target - self.pos).abs() > SCRUB_JUMP_SECONDS {
            // Too far to race there: jump (with a fade) and chase from there.
            self.jump_to(scrub.target);
        }
        self.at_end = false;
    }

    // Tempo ---------------------------------------------------------------

    /// Tempo fader in percent: clamped to the range, and so the effective
    /// tempo stays within 40–499 BPM.
    pub fn set_tempo(&mut self, percent: f64) {
        let mut percent = percent.clamp(-self.tempo_range, self.tempo_range);
        if let Some(bpm) = self.original_bpm() {
            let lo = (MIN_BPM / bpm - 1.0) * 100.0;
            let hi = (MAX_BPM / bpm - 1.0) * 100.0;
            percent = percent.clamp(lo, hi);
        }
        self.tempo_percent = percent;
    }

    /// Changes the fader range, keeping the tempo where it fits.
    pub fn set_tempo_range(&mut self, range: f64) {
        self.tempo_range = range;
        self.tempo_percent = self.tempo_percent.clamp(-range, range);
    }

    pub fn fader_rate(&self) -> f64 {
        1.0 + self.tempo_percent / 100.0
    }

    /// Pitch bend button. Only acts while playing; releasing either button
    /// ends the bend. Returns true if a bend started.
    pub fn pitch_bend(&mut self, direction: i8, press: bool) -> bool {
        if press && self.moving() {
            self.bend = direction.signum();
            true
        } else {
            if !press {
                self.bend = 0;
            }
            false
        }
    }

    /// The pitch bend's speed factor right now.
    pub fn bend_factor(&self) -> f64 {
        if self.moving() {
            1.0 + self.bend as f64 * PITCH_BEND
        } else {
            1.0
        }
    }

    // Helpers -------------------------------------------------------------

    /// Snaps to the grid at the quantize resolution when quantize is on and
    /// there's a grid.
    fn snap(&self, t: f64) -> f64 {
        match (&self.grid, self.quantize) {
            (Some(grid), true) => grid.nearest(t, self.quantize_beats).max(0.0),
            _ => t,
        }
    }

    fn at_track_end(&self) -> bool {
        self.pcm.as_ref().is_some_and(|p| p.is_done()) && self.pos >= self.duration()
    }

    /// Moves the read head. Leaving the active loop ends it. While
    /// scrubbing, the scrub's target moves too.
    pub fn jump_to(&mut self, t: f64) {
        let duration = self.duration();
        let mut t = t.max(0.0);
        if self.pcm.as_ref().is_some_and(|p| p.is_done()) {
            t = t.min(duration);
        }
        if let Some(scrub) = &mut self.scrub {
            scrub.target = t;
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
                level: self.scrub.map_or(1.0, |s| s.level),
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
                level: 1.0,
            });
        }
        self.audible = false;
    }

    fn start_held(&mut self, return_to: f64, button: Button) {
        if !self.moving() {
            self.start_moving();
        }
        self.held = Some(Held { return_to, button });
    }

    fn end_held(&mut self, button: Button) {
        let Some(held) = self.held.filter(|p| p.button == button) else {
            return;
        };
        self.stop_moving();
        self.held = None;
        self.jump_to(held.return_to);
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
        if let Some(scrub) = &mut self.scrub
            && !scrub.released
        {
            scrub.idle = scrub.idle.saturating_add(left.len() as u32);
            if scrub.idle as f64 > SCRUB_IDLE_RELEASE_SECONDS * device_rate {
                scrub.released = true;
            }
        }
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
        let smoothing = 1.0 - (-1.0 / (SCRUB_SMOOTH * device_rate)).exp();
        let mut ended = false;
        self.stalled = false;

        for i in 0..left.len() {
            let (mut l, mut r) = (0.0, 0.0);
            if let Some(tail) = &mut self.tail {
                let gain = tail.level * tail.remaining as f32 / FADE as f32;
                let (tl, tr) = read(pcm, sample_rate, readable, tail.pos);
                l += tl * gain;
                r += tr * gain;
                tail.pos += tail.step;
                tail.remaining -= 1;
                if tail.remaining == 0 {
                    self.tail = None;
                }
            }
            if self.scrub.is_some() {
                let (sl, sr) = self.render_scrub_sample(pcm, readable, end, device_rate, smoothing);
                l += sl;
                r += sr;
            } else if moving && !ended && !self.stalled {
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
                        self.held = None;
                        self.audible = false;
                        ended = true;
                    }
                }
            }
            left[i] = l;
            right[i] = r;
        }
        if !self.moving() && self.scrub.is_none() {
            self.audible = false;
        }
        ended
    }

    /// One output sample of a scrubbed deck: the read head's speed chases
    /// the target (critically damped), the level follows the speed, and a
    /// released scrub hands over to playback or settles on its target.
    #[inline]
    fn render_scrub_sample(
        &mut self,
        pcm: &TrackPcm,
        readable: usize,
        end: f64,
        device_rate: f64,
        smoothing: f64,
    ) -> (f32, f32) {
        let Some(mut scrub) = self.scrub else {
            return (0.0, 0.0);
        };
        let sample_rate = pcm.sample_rate() as f64;
        let resume = scrub.released && self.play_latched;
        let desired = if resume {
            self.rate
        } else {
            ((scrub.target - self.pos) / SCRUB_FOLLOW).clamp(-SCRUB_MAX_RATE, SCRUB_MAX_RATE)
        };
        scrub.velocity += (desired - scrub.velocity) * smoothing;
        scrub.level = ((scrub.velocity.abs() - SCRUB_SILENT_RATE)
            / (SCRUB_FULL_LEVEL_RATE - SCRUB_SILENT_RATE))
            .clamp(0.0, 1.0) as f32;
        let fade = 1.0 - self.fade_in as f32 / FADE as f32;
        self.fade_in = self.fade_in.saturating_sub(1);
        let gain = scrub.level * fade;
        let (sl, sr) = if gain > 0.0 {
            let (a, b) = read(pcm, sample_rate, readable, self.pos);
            (a * gain, b * gain)
        } else {
            (0.0, 0.0)
        };
        let step = scrub.velocity / device_rate;
        self.last_step = step;
        self.audible = scrub.level > 0.0;
        self.pos = (self.pos + step).clamp(0.0, end);

        let handed_over = resume
            && (scrub.velocity - self.rate).abs() <= SCRUB_HANDOVER * self.rate.abs().max(1e-3);
        let settled = scrub.released
            && !self.play_latched
            && scrub.velocity.abs() < SCRUB_SILENT_RATE
            && (scrub.target - self.pos).abs() < SCRUB_SETTLED_SECONDS;
        if handed_over {
            // Playback takes over from here at the deck's tempo, already
            // audible: no fade.
            self.scrub = None;
            self.audible = true;
            self.fade_in = 0;
        } else if settled {
            // Silent already (below the silent speed): rest on the target.
            self.scrub = None;
            self.pos = scrub.target;
            self.audible = false;
        } else {
            self.scrub = Some(scrub);
        }
        (sl, sr)
    }
}

/// The track at `t` seconds, cubic-interpolated.
#[inline]
pub(crate) fn read(pcm: &TrackPcm, sample_rate: f64, readable: usize, t: f64) -> (f32, f32) {
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
