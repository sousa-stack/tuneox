use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A memory cue, hot cue, or loop. Positions are microseconds from the start
/// of the track.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Cue {
    /// Hot cue slot, 0 = A … 7 = H. `None` for memory cues.
    pub hot_cue: Option<u8>,
    pub start_us: i64,
    /// End of the loop in microseconds. `None` for a plain cue point.
    pub end_us: Option<i64>,
    /// Index into the hot cue color palette. `None` for the default color.
    pub color: Option<u8>,
    pub comment: Option<String>,
}

impl Cue {
    pub fn is_loop(&self) -> bool {
        self.end_us.is_some()
    }

    /// `A`–`H` for hot cues.
    pub fn hot_cue_letter(&self) -> Option<char> {
        self.hot_cue.map(|slot| (b'A' + slot) as char)
    }
}
