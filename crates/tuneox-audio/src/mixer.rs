//! The mixer: per-channel 3-band EQ and fader, crossfader, master volume.
//!
//! The EQ is an isolator: Linkwitz-Riley crossovers split each channel into
//! low, mid and high bands that sum back to a flat response, so a band at
//! −∞ dB is a true kill. With every band at 0 dB the channel bypasses the
//! filters (they keep running, so engaging them doesn't click).

use std::f64::consts::PI;

/// Low/mid crossover. A DJM-style isolator split; to be confirmed against
/// rekordbox (research part A).
const LOW_MID_HZ: f64 = 250.0;
/// Mid/high crossover.
const MID_HIGH_HZ: f64 = 2_500.0;

/// EQ gains at or below this many dB kill the band (−∞ dB).
pub const EQ_KILL_DB: f32 = -80.0;
/// The most an EQ band can boost.
pub const EQ_MAX_DB: f32 = 6.0;
/// Auto gain range in dB. rekordbox's knob looks like ±10 dB, but stored
/// track gains reach +14.4 dB.
pub const AUTO_GAIN_MIN_DB: f32 = -24.0;
pub const AUTO_GAIN_MAX_DB: f32 = 24.0;

/// Samples over which gain changes are ramped, to avoid zipper noise.
pub(crate) const RAMP: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ChannelSettings {
    pub volume: f32,
    /// low, mid, high in dB.
    pub eq_db: [f32; 3],
    /// low, mid, high kill switches (rekordbox DUAL PLAYER's HIGH/MID/LOW),
    /// independent of the EQ gains.
    pub kill: [bool; 3],
    /// Per-track loudness trim.
    pub auto_gain_db: f32,
}

impl Default for ChannelSettings {
    fn default() -> Self {
        Self {
            volume: 1.0,
            eq_db: [0.0; 3],
            kill: [false; 3],
            auto_gain_db: 0.0,
        }
    }
}

impl ChannelSettings {
    /// Linear gain of each band: the EQ, or 0 when killed.
    fn band_gains(&self) -> [f32; 3] {
        let mut gains = self.eq_db.map(eq_gain);
        for (gain, kill) in gains.iter_mut().zip(self.kill) {
            if kill {
                *gain = 0.0;
            }
        }
        gains
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MixerSettings {
    pub channels: [ChannelSettings; 2],
    /// −1 = only deck 1, 0 = both, 1 = only deck 2.
    pub crossfader: f32,
    pub master_volume: f32,
}

impl Default for MixerSettings {
    fn default() -> Self {
        Self {
            channels: [ChannelSettings::default(); 2],
            crossfader: 0.0,
            master_volume: 1.0,
        }
    }
}

/// Linear gain of an EQ band.
pub(crate) fn eq_gain(db: f32) -> f32 {
    if db <= EQ_KILL_DB {
        0.0
    } else {
        10f32.powf(db.min(EQ_MAX_DB) / 20.0)
    }
}

/// Gains for decks 1 and 2 at crossfader position `x`: a constant-power
/// curve, both at −3 dB in the middle.
pub(crate) fn crossfader_gains(x: f32) -> [f32; 2] {
    let angle = (x.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    let (sin, cos) = angle.sin_cos();
    // Exact at the ends, so a hard cut is silent.
    let fix = |g: f32| if g.abs() < 1e-6 { 0.0 } else { g };
    [fix(cos), fix(sin)]
}

/// A second-order filter section (RBJ cookbook), transposed direct form II.
#[derive(Debug, Clone, Copy, Default)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

enum Kind {
    Low,
    High,
    All,
}

impl Biquad {
    fn new(kind: Kind, freq: f64, sample_rate: f64) -> Self {
        let q = std::f64::consts::FRAC_1_SQRT_2;
        let w0 = 2.0 * PI * (freq / sample_rate).min(0.49);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let (b0, b1, b2) = match kind {
            Kind::Low => ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0),
            Kind::High => ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0),
            Kind::All => (1.0 - alpha, -2.0 * cos, 1.0 + alpha),
        };
        let a0 = 1.0 + alpha;
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Three-band split of one audio channel.
#[derive(Debug, Clone, Copy, Default)]
struct Isolator {
    low: [Biquad; 2],
    rest: [Biquad; 2],
    /// Puts the low band through the same phase shift as mid + high.
    low_align: Biquad,
    mid: [Biquad; 2],
    high: [Biquad; 2],
}

impl Isolator {
    fn new(sample_rate: f64) -> Self {
        let lp1 = Biquad::new(Kind::Low, LOW_MID_HZ, sample_rate);
        let hp1 = Biquad::new(Kind::High, LOW_MID_HZ, sample_rate);
        let lp2 = Biquad::new(Kind::Low, MID_HIGH_HZ, sample_rate);
        let hp2 = Biquad::new(Kind::High, MID_HIGH_HZ, sample_rate);
        Self {
            low: [lp1; 2],
            rest: [hp1; 2],
            low_align: Biquad::new(Kind::All, MID_HIGH_HZ, sample_rate),
            mid: [lp2; 2],
            high: [hp2; 2],
        }
    }

    #[inline]
    fn split(&mut self, x: f64) -> [f64; 3] {
        let low = self.low_align.process(cascade(&mut self.low, x));
        let rest = cascade(&mut self.rest, x);
        let mid = cascade(&mut self.mid, rest);
        let high = cascade(&mut self.high, rest);
        [low, mid, high]
    }
}

/// Two identical Butterworth sections in a row: a Linkwitz-Riley filter.
#[inline]
fn cascade([a, b]: &mut [Biquad; 2], x: f64) -> f64 {
    let y = a.process(x);
    b.process(y)
}

impl Isolator {}

struct Channel {
    filters: [Isolator; 2],
    /// Band gains as applied at the end of the last block.
    bands: [f32; 3],
    /// 1 when the filters are in use, 0 when bypassed; ramps between.
    wet: f32,
    gain: f32,
}

impl Channel {
    fn new(sample_rate: f64) -> Self {
        Self {
            filters: [Isolator::new(sample_rate); 2],
            bands: [1.0; 3],
            wet: 0.0,
            gain: 0.0,
        }
    }
}

pub(crate) struct Mixer {
    pub settings: MixerSettings,
    channels: [Channel; 2],
    master: f32,
}

impl Mixer {
    pub(crate) fn new(sample_rate: f64) -> Self {
        let settings = MixerSettings::default();
        let mut mixer = Self {
            settings,
            channels: [Channel::new(sample_rate), Channel::new(sample_rate)],
            master: settings.master_volume,
        };
        let xf = crossfader_gains(settings.crossfader);
        for (channel, xf) in mixer.channels.iter_mut().zip(xf) {
            channel.gain = xf;
        }
        mixer
    }

    pub(crate) fn set_sample_rate(&mut self, sample_rate: f64) {
        for channel in &mut self.channels {
            channel.filters = [Isolator::new(sample_rate); 2];
        }
    }

    /// Mixes the two decks' stereo signals into `out` (interleaved stereo).
    /// `decks[d]` is `(left, right)` for deck `d`, each `out.len() / 2` long.
    pub(crate) fn process(&mut self, decks: [(&[f32], &[f32]); 2], out: &mut [f32]) {
        let frames = out.len() / 2;
        out.fill(0.0);
        let xf = crossfader_gains(self.settings.crossfader);
        for (d, (left, right)) in decks.into_iter().enumerate() {
            let settings = self.settings.channels[d];
            let channel = &mut self.channels[d];
            let target_bands = settings.band_gains();
            let auto_gain = 10f32.powf(settings.auto_gain_db / 20.0);
            let target_gain = settings.volume.clamp(0.0, 1.0) * xf[d] * auto_gain;
            let target_wet = if target_bands == [1.0; 3] { 0.0 } else { 1.0 };
            let start_bands = channel.bands;
            let start_gain = channel.gain;
            let start_wet = channel.wet;
            let silent = start_gain == 0.0 && target_gain == 0.0;
            for i in 0..frames {
                let t = ((i + 1) as f32 / RAMP as f32).min(1.0);
                let lerp = |a: f32, b: f32| a + (b - a) * t;
                let (l, r) = (left[i], right[i]);
                // The filters always run so their state stays warm.
                let bl = channel.filters[0].split(l as f64);
                let br = channel.filters[1].split(r as f64);
                if silent {
                    continue;
                }
                let wet = lerp(start_wet, target_wet);
                let gain = lerp(start_gain, target_gain);
                let (mut yl, mut yr) = (l, r);
                if wet > 0.0 {
                    let mut el = 0.0;
                    let mut er = 0.0;
                    for band in 0..3 {
                        let g = lerp(start_bands[band], target_bands[band]) as f64;
                        el += g * bl[band];
                        er += g * br[band];
                    }
                    yl = l + (el as f32 - l) * wet;
                    yr = r + (er as f32 - r) * wet;
                }
                out[2 * i] += yl * gain;
                out[2 * i + 1] += yr * gain;
            }
            let done = frames >= RAMP;
            let reach = |a: f32, b: f32| {
                if done {
                    b
                } else {
                    a + (b - a) * (frames as f32 / RAMP as f32)
                }
            };
            for band in 0..3 {
                channel.bands[band] = reach(start_bands[band], target_bands[band]);
            }
            channel.gain = reach(start_gain, target_gain);
            channel.wet = reach(start_wet, target_wet);
        }
        let target_master = self.settings.master_volume.clamp(0.0, 1.0);
        let start_master = self.master;
        for (i, frame) in out.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let t = ((i + 1) as f32 / RAMP as f32).min(1.0);
            let g = start_master + (target_master - start_master) * t;
            frame[0] = (frame[0] * g).clamp(-1.0, 1.0);
            frame[1] = (frame[1] * g).clamp(-1.0, 1.0);
        }
        self.master = if frames >= RAMP {
            target_master
        } else {
            start_master + (target_master - start_master) * (frames as f32 / RAMP as f32)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossfader_curve() {
        assert_eq!(crossfader_gains(-1.0), [1.0, 0.0]);
        assert_eq!(crossfader_gains(1.0), [0.0, 1.0]);
        let [a, b] = crossfader_gains(0.0);
        assert!((a - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((b - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
    }

    #[test]
    fn eq_gains() {
        assert_eq!(eq_gain(0.0), 1.0);
        assert_eq!(eq_gain(EQ_KILL_DB), 0.0);
        assert_eq!(eq_gain(-200.0), 0.0);
        assert!((eq_gain(6.0) - 1.9953).abs() < 1e-3);
        assert!((eq_gain(12.0) - 1.9953).abs() < 1e-3);
    }

    #[test]
    fn isolator_bands_sum_flat() {
        // An impulse through the split, summed, has a flat magnitude response.
        let mut iso = Isolator::new(48_000.0);
        let n = 4096;
        let response: Vec<f64> = (0..n)
            .map(|i| iso.split(if i == 0 { 1.0 } else { 0.0 }).iter().sum())
            .collect();
        for freq in [40.0, 250.0, 1000.0, 2500.0, 12_000.0] {
            let w = 2.0 * PI * freq / 48_000.0;
            let (mut re, mut im) = (0.0, 0.0);
            for (k, h) in response.iter().enumerate() {
                re += h * (w * k as f64).cos();
                im -= h * (w * k as f64).sin();
            }
            let mag = (re * re + im * im).sqrt();
            assert!((mag - 1.0).abs() < 1e-3, "{freq} Hz: {mag}");
        }
    }
}
