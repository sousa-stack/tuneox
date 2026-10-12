//! Research tool: does our decode line up in time with rekordbox's, with and
//! without MP3 encoder-delay trimming ("gapless")?
//!
//! rekordbox's 3-band detail waveform (PWV7) was computed from rekordbox's own
//! decode at 150 columns/s. This decodes the file both ways, computes a
//! similar low-band envelope, and cross-correlates the attacks against
//! rekordbox's low band. A lag near 0 means our samples sit where rekordbox's
//! did, so its cues and beat grids line up with what we play.
//!
//!     cargo run --release -p tuneox-audio --example grid_offset -- <file> <low band bytes>

use std::fs::File;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

fn decode(path: &str, gapless: bool) -> Result<(f64, Vec<f32>), Box<dyn std::error::Error>> {
    let stream = MediaSourceStream::new(Box::new(File::open(path)?), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.rsplit('.').next() {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe().probe(
        &hint,
        stream,
        FormatOptions::default(),
        MetadataOptions::default(),
    )?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or("no audio track")?;
    let id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or("not audio")?
        .clone();
    let rate = params.sample_rate.ok_or("no sample rate")? as f64;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default().gapless(gapless))?;
    let mut mono = Vec::new();
    let mut buf: Vec<f32> = Vec::new();
    while let Ok(Some(packet)) = format.next_packet() {
        if packet.track_id != id {
            continue;
        }
        let Ok(decoded) = decoder.decode(&packet) else {
            continue;
        };
        let channels = decoded.spec().channels().count().max(1);
        decoded.copy_to_vec_interleaved::<f32>(&mut buf);
        mono.extend(
            buf.chunks_exact(channels)
                .map(|f| f.iter().sum::<f32>() / channels as f32),
        );
    }
    Ok((rate, mono))
}

/// Low-band level per 1/150 s column, smoothed like rekordbox's 3-band
/// analysis (instant attack, slow release).
fn low_band_columns(rate: f64, mono: &[f32]) -> Vec<f64> {
    let lowpass = (-2.0 * std::f64::consts::PI * 150.0 / rate).exp();
    let (mut a, mut b) = (0.0f64, 0.0f64);
    let per_column = rate / 150.0;
    let mut columns = Vec::new();
    let (mut peak, mut level, mut next) = (0.0f64, 0.0f64, per_column);
    for (i, &x) in mono.iter().enumerate() {
        a = lowpass * a + (1.0 - lowpass) * x as f64;
        b = lowpass * b + (1.0 - lowpass) * a;
        peak = peak.max(b.abs());
        if i as f64 + 1.0 >= next {
            level = if peak > level {
                peak
            } else {
                level + (peak - level) * 0.01
            };
            columns.push(level);
            peak = 0.0;
            next += per_column;
        }
    }
    columns
}

/// Rising edges only, so the correlation keys on attacks.
fn attacks(levels: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0];
    out.extend(levels.windows(2).map(|w| (w[1] - w[0]).max(0.0)));
    out
}

fn correlation(a: &[f64], b: &[f64], lag: i64) -> f64 {
    // Pearson correlation of a[i] with b[i + lag].
    let pairs: Vec<(f64, f64)> = (0..a.len() as i64)
        .filter_map(|i| {
            let j = i + lag;
            (j >= 0 && (j as usize) < b.len()).then(|| (a[i as usize], b[j as usize]))
        })
        .collect();
    let n = pairs.len() as f64;
    let (ma, mb) = pairs
        .iter()
        .fold((0.0, 0.0), |s, p| (s.0 + p.0 / n, s.1 + p.1 / n));
    let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
    for (x, y) in pairs {
        cov += (x - ma) * (y - mb);
        va += (x - ma).powi(2);
        vb += (y - mb).powi(2);
    }
    cov / (va.sqrt() * vb.sqrt()).max(1e-12)
}

/// Lag in ms (positive = our audio is late relative to rekordbox's) with
/// parabolic interpolation around the best whole-column lag.
fn best_lag_ms(ours: &[f64], theirs: &[f64]) -> (f64, f64) {
    let scores: Vec<(i64, f64)> = (-30..=30)
        .map(|lag| (lag, correlation(theirs, ours, lag)))
        .collect();
    let (i, &(lag, peak)) = scores
        .iter()
        .enumerate()
        .max_by(|x, y| x.1.1.total_cmp(&y.1.1))
        .unwrap();
    let frac = if i > 0 && i + 1 < scores.len() {
        let (l, r) = (scores[i - 1].1, scores[i + 1].1);
        let denom = l - 2.0 * peak + r;
        if denom.abs() > 1e-12 {
            0.5 * (l - r) / denom
        } else {
            0.0
        }
    } else {
        0.0
    };
    ((lag as f64 + frac) * 1000.0 / 150.0, peak)
}

/// Full-band peak per 1/150 s column (like PWV5's height).
fn peak_columns(rate: f64, mono: &[f32]) -> Vec<f64> {
    let per_column = rate / 150.0;
    let mut columns = Vec::new();
    let (mut peak, mut next) = (0.0f64, per_column);
    for (i, &x) in mono.iter().enumerate() {
        peak = peak.max(x.abs() as f64);
        if i as f64 + 1.0 >= next {
            columns.push(peak);
            peak = 0.0;
            next += per_column;
        }
    }
    columns
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [path, low_band_file, height_file] = args.as_slice() else {
        return Err("usage: grid_offset <audio file> <rekordbox low band bytes> <rekordbox rgb height bytes>".into());
    };
    let low: Vec<f64> = std::fs::read(low_band_file)?
        .iter()
        .map(|&b| b as f64)
        .collect();
    let height: Vec<f64> = std::fs::read(height_file)?
        .iter()
        .map(|&b| b as f64)
        .collect();
    let (low, height) = (attacks(&low), attacks(&height));
    let lead_in = tuneox_audio::mp3_lead_in_frames(std::path::Path::new(path));
    for (gapless, lead) in [(false, 0), (true, 0), (false, lead_in)] {
        let (rate, mut mono) = decode(path, gapless)?;
        if lead > 0 {
            mono.splice(0..0, std::iter::repeat_n(0.0, lead));
        }
        let (lag_low, r_low) = best_lag_ms(&attacks(&low_band_columns(rate, &mono)), &low);
        let (lag_full, r_full) = best_lag_ms(&attacks(&peak_columns(rate, &mono)), &height);
        let label = if lead > 0 {
            "lead-in"
        } else if gapless {
            "trimmed"
        } else {
            "plain"
        };
        println!(
            "{label:8}: low band {lag_low:+.2} ms (r {r_low:.2}), full band {lag_full:+.2} ms (r {r_full:.2})"
        );
    }
    Ok(())
}
