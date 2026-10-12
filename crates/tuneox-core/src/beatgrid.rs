use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One beat of a beat grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Beat {
    /// Microseconds from the start of the track.
    pub time_us: i64,
    /// Position in the bar, 1–4.
    pub bar_position: u8,
    /// Tempo at this beat, BPM × 100.
    pub bpm_centi: u16,
}

/// A beat grid: every beat in the track, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BeatGrid {
    pub beats: Vec<Beat>,
}

const PACKED_BEAT: usize = 12;

impl BeatGrid {
    /// Compact storage form: 12 bytes per beat (i64 time, u16 tempo, u8 bar
    /// position, one spare byte), little-endian.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.beats.len() * PACKED_BEAT);
        for beat in &self.beats {
            out.extend_from_slice(&beat.time_us.to_le_bytes());
            out.extend_from_slice(&beat.bpm_centi.to_le_bytes());
            out.push(beat.bar_position);
            out.push(0);
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let (chunks, rest) = bytes.as_chunks::<PACKED_BEAT>();
        if !rest.is_empty() {
            return None;
        }
        let beats = chunks
            .iter()
            .map(|b| Beat {
                time_us: i64::from_le_bytes(b[0..8].try_into().unwrap()),
                bpm_centi: u16::from_le_bytes(b[8..10].try_into().unwrap()),
                bar_position: b[10],
            })
            .collect();
        Some(Self { beats })
    }

    /// The tempo if every beat has the same one.
    pub fn constant_bpm(&self) -> Option<f64> {
        let first = self.beats.first()?.bpm_centi;
        self.beats
            .iter()
            .all(|b| b.bpm_centi == first)
            .then_some(first as f64 / 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_and_unpacks() {
        let grid = BeatGrid {
            beats: vec![
                Beat {
                    time_us: 16_000,
                    bar_position: 1,
                    bpm_centi: 12_500,
                },
                Beat {
                    time_us: 496_000,
                    bar_position: 2,
                    bpm_centi: 12_500,
                },
            ],
        };
        let bytes = grid.to_bytes();
        assert_eq!(bytes.len(), 24);
        assert_eq!(BeatGrid::from_bytes(&bytes), Some(grid.clone()));
        assert_eq!(grid.constant_bpm(), Some(125.0));
        assert_eq!(BeatGrid::from_bytes(&bytes[..5]), None);
    }
}
