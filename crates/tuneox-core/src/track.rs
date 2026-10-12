use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::MusicalKey;

pub type TrackId = i64;

/// A track in the library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Track {
    pub id: TrackId,
    #[serde(flatten)]
    pub info: TrackInfo,
}

/// Everything about a track except its library ID.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TrackInfo {
    /// Absolute path of the audio file.
    pub path: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub label: Option<String>,
    pub remixer: Option<String>,
    pub comment: Option<String>,
    pub key: Option<MusicalKey>,
    pub bpm: Option<f64>,
    pub duration_secs: Option<f64>,
    /// 0–5 stars.
    pub rating: u8,
    pub color: Option<TrackColor>,
    pub file_type: Option<FileType>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u16>,
    /// kbit/s.
    pub bitrate: Option<u32>,
    /// Bytes.
    pub file_size: Option<u64>,
    pub play_count: u32,
    pub year: Option<u16>,
    pub track_number: Option<u32>,
    /// `YYYY-MM-DD`.
    pub date_added: Option<String>,
}

/// The eight track colors Pioneer DJ hardware and rekordbox support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TrackColor {
    Pink,
    Red,
    Orange,
    Yellow,
    Green,
    Aqua,
    Blue,
    Purple,
}

impl TrackColor {
    pub const ALL: [TrackColor; 8] = [
        Self::Pink,
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Aqua,
        Self::Blue,
        Self::Purple,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pink => "pink",
            Self::Red => "red",
            Self::Orange => "orange",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Aqua => "aqua",
            Self::Blue => "blue",
            Self::Purple => "purple",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == text)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FileType {
    Mp3,
    M4a,
    Flac,
    Wav,
    Aiff,
}

impl FileType {
    pub const ALL: [FileType; 5] = [Self::Mp3, Self::M4a, Self::Flac, Self::Wav, Self::Aiff];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::M4a => "m4a",
            Self::Flac => "flac",
            Self::Wav => "wav",
            Self::Aiff => "aiff",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == text)
    }

    pub fn from_extension(path: &str) -> Option<Self> {
        let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
        match ext.as_str() {
            "mp3" => Some(Self::Mp3),
            "m4a" | "mp4" | "aac" => Some(Self::M4a),
            "flac" => Some(Self::Flac),
            "wav" | "wave" => Some(Self::Wav),
            "aif" | "aiff" => Some(Self::Aiff),
            _ => None,
        }
    }
}
