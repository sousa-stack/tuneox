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
/// Off: rekordbox doesn't trim it, and its cues and beat grids are measured
/// on its own decoder's output (see research/mp3-timing.md).
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

    // rekordbox plays a LAME-style Xing/Info header frame as a frame of
    // silence, so its cues and beat grids for those files sit one MP3 frame
    // later than our decode. Lead in with the same silence so they line up.
    let lead_in = mp3_lead_in_frames(path);
    if lead_in > 0 {
        samples.clear();
        samples.resize(lead_in * 2, 0.0);
        writer.write_interleaved(&samples, 2);
        writer.publish();
    }
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

fn is_mp3(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("mp3"))
}

/// Frames of silence rekordbox plays before an MP3's audio: one MP3 frame when
/// the file starts with a Xing/Info header frame not written by FFmpeg, else
/// none. Measured against rekordbox's own waveforms (research/mp3-timing.md).
pub fn mp3_lead_in_frames(path: &Path) -> usize {
    if !is_mp3(path) {
        return 0;
    }
    File::open(path)
        .ok()
        .and_then(|mut file| rekordbox_lead_in_frames(&mut file))
        .unwrap_or(0)
}

fn rekordbox_lead_in_frames(file: &mut File) -> Option<usize> {
    use std::io::{Read, Seek, SeekFrom};
    let mut head = [0u8; 10];
    file.read_exact(&mut head).ok()?;
    let mut start = 0u64;
    if &head[..3] == b"ID3" {
        let size = head[6..10]
            .iter()
            .fold(0u64, |acc, &b| (acc << 7) | (b & 0x7f) as u64);
        let footer = if head[5] & 0x10 != 0 { 10 } else { 0 };
        start = 10 + size + footer;
    }
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut data = Vec::with_capacity(16 * 1024);
    file.take(16 * 1024).read_to_end(&mut data).ok()?;
    info_frame_lead_in(&data)
}

/// The lead-in for the bytes that follow any ID3v2 tag.
fn info_frame_lead_in(data: &[u8]) -> Option<usize> {
    let (pos, header) = (0..data.len().saturating_sub(4))
        .find_map(|i| Layer3Header::parse(&data[i..]).map(|h| (i, h)))?;
    let tag = pos + 4 + header.crc_bytes + header.side_info_bytes;
    match data.get(tag..tag + 4) {
        Some(b"Xing") | Some(b"Info") => {}
        _ => return Some(0),
    }
    // The LAME-style encoder string follows the Xing fields that are present.
    let flags = u32::from_be_bytes(data.get(tag + 4..tag + 8)?.try_into().ok()?);
    let mut encoder = tag + 8;
    for (flag, size) in [(1, 4), (2, 4), (4, 100), (8, 4)] {
        if flags & flag != 0 {
            encoder += size;
        }
    }
    let written_by_ffmpeg = data
        .get(encoder..encoder + 3)
        .is_some_and(|name| name == b"Lav");
    Some(if written_by_ffmpeg {
        0
    } else {
        header.samples_per_frame
    })
}

/// The parts of an MPEG audio Layer III frame header we need.
struct Layer3Header {
    samples_per_frame: usize,
    side_info_bytes: usize,
    crc_bytes: usize,
}

impl Layer3Header {
    fn parse(bytes: &[u8]) -> Option<Self> {
        let [b0, b1, b2, b3] = *bytes.get(..4)? else {
            return None;
        };
        let version = (b1 >> 3) & 3; // 3 = MPEG-1, 2 = MPEG-2, 0 = MPEG-2.5
        let layer = (b1 >> 1) & 3; // 1 = Layer III
        let bitrate = b2 >> 4;
        let sample_rate = (b2 >> 2) & 3;
        if b0 != 0xff || b1 & 0xe0 != 0xe0 || version == 1 || layer != 1 {
            return None;
        }
        if bitrate == 0 || bitrate == 15 || sample_rate == 3 {
            return None;
        }
        let mono = b3 >> 6 == 3;
        let mpeg1 = version == 3;
        Some(Self {
            samples_per_frame: if mpeg1 { 1152 } else { 576 },
            side_info_bytes: match (mpeg1, mono) {
                (true, false) => 32,
                (true, true) => 17,
                (false, false) => 17,
                (false, true) => 9,
            },
            crc_bytes: if b1 & 1 == 0 { 2 } else { 0 },
        })
    }
}

#[cfg(test)]
mod lead_in_tests {
    use super::info_frame_lead_in;

    /// A Layer III frame header, then an info tag with all four Xing fields
    /// and an encoder string.
    fn frame(header: [u8; 4], side_info: usize, tag: &[u8; 4], encoder: &[u8]) -> Vec<u8> {
        let mut data = header.to_vec();
        data.extend(vec![0; side_info]);
        data.extend(tag);
        data.extend(0x0fu32.to_be_bytes());
        data.extend(vec![0; 4 + 4 + 100 + 4]);
        data.extend(encoder);
        data.extend(vec![0; 400]);
        data
    }

    // MPEG-1 Layer III, no CRC, 128 kbps, 44.1 kHz, joint stereo / mono.
    const MPEG1_STEREO: [u8; 4] = [0xff, 0xfb, 0x90, 0x64];
    const MPEG1_MONO: [u8; 4] = [0xff, 0xfb, 0x90, 0xc4];
    // MPEG-2 Layer III, no CRC, 64 kbps, 22.05 kHz, mono.
    const MPEG2_MONO: [u8; 4] = [0xff, 0xf3, 0x80, 0xc4];

    #[test]
    fn lame_info_frames_get_one_frame() {
        let lame = frame(MPEG1_STEREO, 32, b"Info", b"LAME3.100");
        assert_eq!(info_frame_lead_in(&lame), Some(1152));
        let vbr = frame(MPEG1_MONO, 17, b"Xing", b"LAME3.99r");
        assert_eq!(info_frame_lead_in(&vbr), Some(1152));
        let unknown = frame(MPEG1_STEREO, 32, b"Xing", &[0; 9]);
        assert_eq!(info_frame_lead_in(&unknown), Some(1152));
        let mpeg2 = frame(MPEG2_MONO, 9, b"Info", b"LAME3.100");
        assert_eq!(info_frame_lead_in(&mpeg2), Some(576));
    }

    #[test]
    fn ffmpeg_info_frames_and_plain_files_get_none() {
        let ffmpeg = frame(MPEG1_STEREO, 32, b"Info", b"Lavc58.91");
        assert_eq!(info_frame_lead_in(&ffmpeg), Some(0));
        let plain = frame(MPEG1_STEREO, 32, b"\0\0\0\0", b"");
        assert_eq!(info_frame_lead_in(&plain), Some(0));
        // Junk before the first frame is skipped.
        let mut junk = vec![0x12, 0x34, 0xff, 0x00];
        junk.extend(frame(MPEG1_STEREO, 32, b"Info", b"LAME3.100"));
        assert_eq!(info_frame_lead_in(&junk), Some(1152));
        assert_eq!(info_frame_lead_in(b"no frames here"), None);
    }

    #[test]
    fn crc_protected_frames_shift_the_tag() {
        let mut header = MPEG1_STEREO;
        header[1] &= !1; // protection bit clear: 2 CRC bytes follow
        let mut data = header.to_vec();
        data.extend([0, 0]);
        data.extend(frame(MPEG1_STEREO, 32, b"Info", b"LAME3.100")[4..].iter());
        assert_eq!(info_frame_lead_in(&data), Some(1152));
    }
}
