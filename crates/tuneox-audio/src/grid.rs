//! Beat grid math for the audio thread: fractional beat positions, bar
//! positions, tempo at a position, snapping. All times are seconds from the
//! start of the track.

use tuneox_core::Beat;

/// Beats per bar. rekordbox's grids are 4/4 (`bar_position` 1–4).
pub(crate) const BEATS_PER_BAR: f64 = 4.0;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Grid {
    /// Beat times, strictly increasing.
    times: Vec<f64>,
    /// Tempo at each beat, as the grid states it (rekordbox stores it
    /// separately from the millisecond-rounded beat times; it's the steadier
    /// number to match tempos with). 0 when unknown.
    bpms: Vec<f64>,
    /// Each beat's place in its bar, 0 = the downbeat … 3.
    bars: Vec<u8>,
}

impl Grid {
    /// A grid from library beats. `None` if there are none usable.
    pub(crate) fn from_beats(beats: &[Beat]) -> Option<Grid> {
        let mut times = Vec::with_capacity(beats.len());
        let mut bpms = Vec::with_capacity(beats.len());
        let mut bars = Vec::with_capacity(beats.len());
        for beat in beats {
            let time = beat.time_us as f64 / 1e6;
            if times.last().is_some_and(|&last| time <= last) {
                continue;
            }
            // A missing or odd bar position continues the previous bar.
            let bar = match beat.bar_position {
                1..=4 => beat.bar_position - 1,
                _ => bars.last().map_or(0, |&b: &u8| (b + 1) % 4),
            };
            times.push(time);
            bpms.push(beat.bpm_centi as f64 / 100.0);
            bars.push(bar);
        }
        if times.is_empty() || (times.len() == 1 && bpms[0] <= 0.0) {
            return None;
        }
        Some(Grid { times, bpms, bars })
    }

    /// A steady grid with a beat at `first` (seconds), for tracks with a
    /// known BPM but no beat grid. Its phase (and bars, starting there) are
    /// a guess.
    pub(crate) fn constant(first: f64, bpm: f64) -> Option<Grid> {
        (bpm > 0.0 && bpm.is_finite()).then(|| Grid {
            times: vec![first],
            bpms: vec![bpm],
            bars: vec![0],
        })
    }

    /// Seconds from beat `i` to beat `i + 1`, extrapolated at the ends.
    fn interval(&self, i: usize) -> f64 {
        let n = self.times.len();
        if n == 1 {
            return 60.0 / self.bpms[0];
        }
        let i = i.min(n - 2);
        self.times[i + 1] - self.times[i]
    }

    /// Fractional beat index at `t`: 0.0 on the first beat, 0.5 halfway to
    /// the second. Extrapolates before the first and after the last beat.
    pub(crate) fn beat_at(&self, t: f64) -> f64 {
        let n = self.times.len();
        let first = self.times[0];
        // NaN takes this branch too: the audio thread must never panic.
        if n == 1 || t < first || t.is_nan() {
            return (t - first) / self.interval(0);
        }
        // Beats at or before t.
        let i = self.times.partition_point(|&b| b <= t) - 1;
        (i as f64) + (t - self.times[i]) / self.interval(i)
    }

    /// Time of fractional beat `beat`; the inverse of [`Self::beat_at`].
    pub(crate) fn time_at(&self, beat: f64) -> f64 {
        let n = self.times.len();
        if beat < 0.0 || n == 1 {
            return self.times[0] + beat * self.interval(0);
        }
        let i = (beat.floor() as usize).min(n - 1);
        self.times[i] + (beat - i as f64) * self.interval(i)
    }

    /// The grid's tempo at `t`.
    pub(crate) fn bpm_at(&self, t: f64) -> f64 {
        let beat = self.beat_at(t).floor().max(0.0) as usize;
        let i = beat.min(self.times.len() - 1);
        match self.bpms[i] {
            bpm if bpm > 0.0 => bpm,
            _ => 60.0 / self.interval(i),
        }
    }

    /// The time of the beat nearest to `t`.
    #[cfg(test)]
    pub(crate) fn nearest_beat(&self, t: f64) -> f64 {
        self.nearest(t, 1.0)
    }

    /// The grid position nearest to `t` at a resolution of `beats` (1 = the
    /// nearest beat, 0.25 = the nearest quarter beat).
    pub(crate) fn nearest(&self, t: f64, beats: f64) -> f64 {
        self.time_at((self.beat_at(t) / beats).round() * beats)
    }

    /// `t` moved by `beats`, keeping its position within the beat.
    pub(crate) fn shift(&self, t: f64, beats: f64) -> f64 {
        self.time_at(self.beat_at(t) + beats)
    }

    /// Where `t` is within its bar, in beats: 0 on the downbeat … just
    /// under 4. Extrapolates past the grid's ends.
    pub(crate) fn bar_beat_at(&self, t: f64) -> f64 {
        let beat = self.beat_at(t);
        if beat.is_nan() {
            return beat;
        }
        let i = (beat.floor().max(0.0) as usize).min(self.times.len() - 1);
        (self.bars[i] as f64 + beat - i as f64).rem_euclid(BEATS_PER_BAR)
    }
}

/// `x` minus the nearest integer, in −0.5…0.5.
pub(crate) fn wrap_phase(x: f64) -> f64 {
    x - x.round()
}

/// `x` minus the nearest multiple of `unit`, in −unit/2…unit/2.
pub(crate) fn wrap_to(x: f64, unit: f64) -> f64 {
    x - (x / unit).round() * unit
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(first_ms: i64, bpm: f64, beats: usize) -> Grid {
        let period_us = 60e6 / bpm;
        let beats: Vec<Beat> = (0..beats)
            .map(|i| Beat {
                time_us: first_ms * 1000 + (i as f64 * period_us).round() as i64,
                bar_position: (i % 4) as u8 + 1,
                bpm_centi: (bpm * 100.0).round() as u16,
            })
            .collect();
        Grid::from_beats(&beats).unwrap()
    }

    #[test]
    fn beats_and_times_are_inverse() {
        let g = grid(100, 120.0, 16);
        assert_eq!(g.beat_at(0.1), 0.0);
        assert!((g.beat_at(0.35) - 0.5).abs() < 1e-12);
        assert!((g.beat_at(0.0) + 0.2).abs() < 1e-12);
        assert!((g.beat_at(20.0) - 39.8).abs() < 1e-9);
        for b in [-3.25, 0.0, 0.5, 7.75, 15.0, 30.5] {
            assert!((g.beat_at(g.time_at(b)) - b).abs() < 1e-9, "{b}");
        }
        assert_eq!(g.bpm_at(1.0), 120.0);
        assert!(g.beat_at(f64::NAN).is_nan(), "no panic");
        assert!(g.time_at(f64::NAN).is_nan());
        assert!((g.nearest_beat(0.84) - 0.6).abs() < 1e-12);
        assert!((g.nearest_beat(0.86) - 1.1).abs() < 1e-12);
        assert!((g.shift(0.35, 4.0) - 2.35).abs() < 1e-12);
    }

    #[test]
    fn single_beat_grids_extrapolate_from_the_tempo() {
        let g = Grid::constant(0.25, 128.0).unwrap();
        let period = 60.0 / 128.0;
        assert!((g.time_at(10.0) - (0.25 + 10.0 * period)).abs() < 1e-12);
        assert!((g.beat_at(0.25 + 2.5 * period) - 2.5).abs() < 1e-12);
        assert_eq!(g.bpm_at(100.0), 128.0);
        assert!(Grid::constant(0.0, 0.0).is_none());
    }

    #[test]
    fn phase_wraps_to_the_nearest_beat() {
        assert!((wrap_phase(3.2) - 0.2).abs() < 1e-12);
        assert!((wrap_phase(3.8) + 0.2).abs() < 1e-12);
        assert!((wrap_phase(-0.3) + 0.3).abs() < 1e-12);
        assert!((wrap_to(3.0, 4.0) + 1.0).abs() < 1e-12);
        assert!((wrap_to(-2.5, 4.0) - 1.5).abs() < 1e-12);
    }

    #[test]
    fn snaps_at_a_resolution() {
        let g = grid(0, 120.0, 16);
        assert!((g.nearest(1.13, 1.0) - 1.0).abs() < 1e-12);
        assert!((g.nearest(1.13, 0.25) - 1.125).abs() < 1e-12);
        assert!((g.nearest(1.13, 0.125) - 1.125).abs() < 1e-12);
        assert!((g.nearest(1.2, 0.5) - 1.25).abs() < 1e-12);
    }

    #[test]
    fn bar_positions_come_from_the_grid() {
        // The first beat is the 3rd of its bar.
        let beats: Vec<Beat> = (0..8)
            .map(|i| Beat {
                time_us: i * 500_000,
                bar_position: ((i + 2) % 4) as u8 + 1,
                bpm_centi: 12_000,
            })
            .collect();
        let g = Grid::from_beats(&beats).unwrap();
        assert!((g.bar_beat_at(0.0) - 2.0).abs() < 1e-12);
        assert!((g.bar_beat_at(0.75) - 3.5).abs() < 1e-12);
        assert!(g.bar_beat_at(1.0).abs() < 1e-12, "downbeat");
        // Extrapolated past the ends.
        assert!((g.bar_beat_at(5.0) - 0.0).abs() < 1e-9);
        assert!((g.bar_beat_at(-0.5) - 1.0).abs() < 1e-9);
        assert_eq!(Grid::constant(0.0, 120.0).unwrap().bar_beat_at(0.5), 1.0);
    }
}
