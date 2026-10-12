//! Decoding audio files into a [`TrackPcm`] on a background thread.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::pcm::TrackPcm;

/// Whether to drop MP3 encoder delay and padding (LAME/Xing "gapless" info).
/// Off: rekordbox's cues and beat grids are measured on its own decoder's
/// output, and until research confirms that it trims, we keep every decoded
/// frame so positions line up with what rekordbox analysed. See "Behavior to
/// confirm" in docs/specs/decks.md.
const TRIM_ENCODER_DELAY: bool = false;

/// Headroom over the expected length, for estimates that come up short.
const EXTRA_SECONDS: f64 = 30.0;
/// Storage to reserve when nothing says how long the track is.
const UNKNOWN_LENGTH_SECONDS: f64 = 3.0 * 3600.0;

/// Starts decoding `path` into `pcm` on its own thread. Failures end up in
/// `pcm.error()`; nothing here panics on bad input.
pub(crate) fn spawn(path: PathBuf, pcm: Arc<TrackPcm>, duration_hint: Option<f64>) {
    let thread = std::thread::Builder::new()
        .name("tuneox-decode".into())
        .spawn({
            let pcm = pcm.clone();
            move || {
                if let Err(message) = decode(&path, &pcm, duration_hint) {
                    pcm.fail(message);
                }
            }
        });
    if let Err(e) = thread {
        pcm.fail(format!("couldn't start the decoder: {e}"));
    }
}

fn decode(path: &Path, pcm: &TrackPcm, duration_hint: Option<f64>) -> Result<(), String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| format!("unsupported or damaged audio file: {e}"))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or("the file has no audio track")?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or("the file's audio format isn't supported")?
        .clone();
    let sample_rate = params.sample_rate.ok_or("the file has no sample rate")?;
    if sample_rate == 0 {
        return Err("the file has no sample rate".into());
    }
    let expected = track.num_frames.map(|n| n as usize);
    let options = AudioDecoderOptions::default().gapless(TRIM_ENCODER_DELAY);
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &options)
        .map_err(|e| format!("unsupported audio codec: {e}"))?;

    let rate = sample_rate as f64;
    let expected_secs = expected
        .map(|n| n as f64 / rate)
        .or(duration_hint)
        .filter(|s| *s > 0.0);
    let capacity_secs = match expected_secs {
        Some(secs) => secs * 1.1 + EXTRA_SECONDS,
        None => UNKNOWN_LENGTH_SECONDS,
    };
    pcm.begin(sample_rate, (capacity_secs * rate) as usize, expected);

    let mut writer = pcm.writer();
    let mut samples: Vec<f32> = Vec::new();
    loop {
        if pcm.cancelled() {
            return Ok(());
        }
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            // Some files end without a clean end-of-stream marker.
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => return Err(format!("couldn't read the file: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buffer) => {
                let channels = buffer.spec().channels().count();
                if channels == 0 || buffer.frames() == 0 {
                    continue;
                }
                buffer.copy_to_vec_interleaved::<f32>(&mut samples);
                if !writer.write_interleaved(&samples, channels) {
                    // Longer than the storage reserved for it: keep what fits.
                    writer.publish();
                    break;
                }
                writer.publish();
            }
            // A damaged packet: skip it like players do and carry on.
            Err(SymphoniaError::DecodeError(_)) | Err(SymphoniaError::IoError(_)) => continue,
            Err(e) => return Err(format!("couldn't decode the file: {e}")),
        }
    }
    writer.publish();
    if pcm.decoded_frames() == 0 {
        return Err("the file contains no audio".into());
    }
    pcm.finish();
    Ok(())
}
