//! Decoded audio of one track, shared between its decoder thread and the
//! audio thread.
//!
//! Frames are 16-bit stereo packed into one `AtomicU32` each (left in the low
//! half), so a track costs 4 bytes per frame: about 10.6 MB per minute at
//! 44.1 kHz and 11.5 MB at 48 kHz. Storage grows in fixed-size chunks that
//! the decoder allocates and publishes through `OnceLock`, and the number of
//! readable frames is published with release/acquire ordering. The audio
//! thread therefore reads a track while it's still being decoded without
//! locks, allocation or `unsafe`, and relaxed atomic loads of a `u32` compile
//! to plain loads.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicUsize, Ordering};

const CHUNK_SHIFT: u32 = 15;
/// Frames per storage chunk (0.74 s at 44.1 kHz, 128 KB).
pub(crate) const CHUNK_FRAMES: usize = 1 << CHUNK_SHIFT;
const CHUNK_MASK: usize = CHUNK_FRAMES - 1;

const LOADING: u8 = 0;
const DONE: u8 = 1;
const FAILED: u8 = 2;

type Chunk = Box<[AtomicU32]>;

#[derive(Default)]
pub struct TrackPcm {
    /// 0 until the decoder has read the file's header.
    sample_rate: AtomicU32,
    chunks: OnceLock<Box<[OnceLock<Chunk>]>>,
    /// Frames the audio thread may read.
    decoded: AtomicUsize,
    /// Expected length while decoding (0 if unknown), exact once done.
    total: AtomicUsize,
    state: AtomicU8,
    error: OnceLock<String>,
    cancel: AtomicBool,
}

impl TrackPcm {
    /// Prepares storage for up to `capacity` frames at `sample_rate`.
    /// `expected` is the decoder's estimate of the track length, if any.
    pub(crate) fn begin(&self, sample_rate: u32, capacity: usize, expected: Option<usize>) {
        let chunks = capacity.div_ceil(CHUNK_FRAMES).max(1);
        let table: Box<[OnceLock<Chunk>]> = (0..chunks).map(|_| OnceLock::new()).collect();
        let _ = self.chunks.set(table);
        self.total.store(expected.unwrap_or(0), Ordering::Relaxed);
        self.sample_rate.store(sample_rate, Ordering::Release);
    }

    pub(crate) fn writer(&self) -> PcmWriter<'_> {
        PcmWriter {
            pcm: self,
            written: self.decoded.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn finish(&self) {
        let decoded = self.decoded.load(Ordering::Relaxed);
        self.total.store(decoded, Ordering::Relaxed);
        self.state.store(DONE, Ordering::Release);
    }

    pub(crate) fn fail(&self, message: String) {
        let _ = self.error.set(message);
        self.finish();
        self.state.store(FAILED, Ordering::Release);
    }

    /// Asks the decoder to stop; used when a deck loads another track.
    pub(crate) fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate.load(Ordering::Acquire)
    }

    pub fn decoded_frames(&self) -> usize {
        self.decoded.load(Ordering::Acquire)
    }

    /// True once decoding has finished or failed: no more frames will come.
    pub fn is_done(&self) -> bool {
        self.state.load(Ordering::Acquire) != LOADING
    }

    pub fn error(&self) -> Option<&str> {
        if self.state.load(Ordering::Acquire) == FAILED {
            self.error.get().map(String::as_str)
        } else {
            None
        }
    }

    /// The track length in frames: exact once done, otherwise the decoder's
    /// estimate or what's decoded so far, whichever is longer.
    pub fn total_frames(&self) -> usize {
        let decoded = self.decoded_frames();
        if self.is_done() {
            decoded
        } else {
            self.total.load(Ordering::Relaxed).max(decoded)
        }
    }

    pub fn duration_secs(&self) -> f64 {
        match self.sample_rate() {
            0 => 0.0,
            rate => self.total_frames() as f64 / rate as f64,
        }
    }

    /// Frame `index` as (left, right) in −1…1, or silence outside
    /// `0..readable`. `readable` must not exceed [`Self::decoded_frames`].
    #[inline]
    pub(crate) fn frame(&self, index: i64, readable: usize) -> (f32, f32) {
        if index < 0 || index as usize >= readable {
            return (0.0, 0.0);
        }
        let index = index as usize;
        let Some(chunk) = self
            .chunks
            .get()
            .and_then(|table| table.get(index >> CHUNK_SHIFT))
            .and_then(OnceLock::get)
        else {
            return (0.0, 0.0);
        };
        let packed = chunk[index & CHUNK_MASK].load(Ordering::Relaxed);
        (
            (packed as u16 as i16) as f32 / 32768.0,
            ((packed >> 16) as u16 as i16) as f32 / 32768.0,
        )
    }

    /// A fully decoded track from interleaved samples, for tests and
    /// generated audio.
    pub(crate) fn from_samples(sample_rate: u32, channels: usize, samples: &[f32]) -> TrackPcm {
        let pcm = TrackPcm::default();
        let channels = channels.max(1);
        let frames = samples.len() / channels;
        pcm.begin(sample_rate, frames, Some(frames));
        let mut writer = pcm.writer();
        writer.write_interleaved(samples, channels);
        writer.publish();
        pcm.finish();
        pcm
    }
}

/// Appends frames on the decoder thread.
pub(crate) struct PcmWriter<'a> {
    pcm: &'a TrackPcm,
    written: usize,
}

impl PcmWriter<'_> {
    /// Appends interleaved frames, converted to 16-bit stereo: mono is
    /// copied to both sides, channels past the second are dropped. Returns
    /// false once storage is full.
    pub(crate) fn write_interleaved(&mut self, samples: &[f32], channels: usize) -> bool {
        let Some(table) = self.pcm.chunks.get() else {
            return false;
        };
        for frame in samples.chunks_exact(channels) {
            let left = frame[0];
            let right = if channels > 1 { frame[1] } else { left };
            let index = self.written;
            let Some(slot) = table.get(index >> CHUNK_SHIFT) else {
                return false;
            };
            let chunk = slot.get_or_init(|| (0..CHUNK_FRAMES).map(|_| AtomicU32::new(0)).collect());
            let packed = (to_i16(left) as u16 as u32) | ((to_i16(right) as u16 as u32) << 16);
            chunk[index & CHUNK_MASK].store(packed, Ordering::Relaxed);
            self.written += 1;
        }
        true
    }

    /// Makes everything written so far visible to the audio thread.
    pub(crate) fn publish(&self) {
        self.pcm.decoded.store(self.written, Ordering::Release);
    }
}

/// Rounds to 16 bits. 16-bit sources round-trip exactly; deeper sources lose
/// their bottom bits (−96 dB), which is inaudible in a mix.
#[inline]
fn to_i16(sample: f32) -> i16 {
    (sample * 32768.0).round().clamp(-32768.0, 32767.0) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_frames_across_chunks() {
        let frames = CHUNK_FRAMES + 10;
        let samples: Vec<f32> = (0..frames * 2)
            .map(|i| ((i % 200) as f32 - 100.0) / 128.0)
            .collect();
        let pcm = TrackPcm::from_samples(48_000, 2, &samples);
        assert!(pcm.is_done());
        assert_eq!(pcm.total_frames(), frames);
        let readable = pcm.decoded_frames();
        for index in [0, 1, CHUNK_FRAMES - 1, CHUNK_FRAMES, frames - 1] {
            let (l, r) = pcm.frame(index as i64, readable);
            assert_eq!(l, samples[index * 2]);
            assert_eq!(r, samples[index * 2 + 1]);
        }
        assert_eq!(pcm.frame(-1, readable), (0.0, 0.0));
        assert_eq!(pcm.frame(frames as i64, readable), (0.0, 0.0));
    }

    #[test]
    fn mono_plays_on_both_sides() {
        let pcm = TrackPcm::from_samples(44_100, 1, &[0.5, -0.25]);
        assert_eq!(pcm.frame(0, 2), (0.5, 0.5));
        assert_eq!(pcm.frame(1, 2), (-0.25, -0.25));
    }
}
