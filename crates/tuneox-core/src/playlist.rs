use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub type PlaylistId = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PlaylistKind {
    Folder,
    Playlist,
    /// Contents come from rules rather than a fixed track list.
    Smart,
}

impl PlaylistKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Folder => "folder",
            Self::Playlist => "playlist",
            Self::Smart => "smart",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Self::Folder, Self::Playlist, Self::Smart]
            .into_iter()
            .find(|k| k.as_str() == text)
    }
}

/// A node in the playlist tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Playlist {
    pub id: PlaylistId,
    /// `None` at the top level.
    pub parent_id: Option<PlaylistId>,
    pub name: String,
    pub kind: PlaylistKind,
    /// Order among its siblings, from 0.
    pub position: u32,
    pub track_count: u32,
}
