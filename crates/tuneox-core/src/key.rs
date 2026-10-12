//! Musical keys, and parsing the notations DJ software writes into tags.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Major,
    Minor,
}

/// A key as tonic pitch class (0 = C … 11 = B) plus mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MusicalKey {
    pub tonic: u8,
    pub mode: Mode,
}

/// Spelling used for display, matching rekordbox's "classic" notation.
const NAMES: [&str; 12] = [
    "C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
];

impl MusicalKey {
    pub fn new(tonic: u8, mode: Mode) -> Self {
        Self {
            tonic: tonic % 12,
            mode,
        }
    }

    /// Parses classic names (`Am`, `Amin`, `F#m`, `Bbmaj`, `C`), Camelot
    /// (`8A`, `11B`) and Open Key (`1m`, `6d`). Returns `None` for anything
    /// else, including placeholders like rekordbox's `All`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        Self::parse_wheel(text).or_else(|| Self::parse_name(text))
    }

    /// Camelot (`8A` = A minor, `8B` = C major) or Open Key (`1m` = A minor,
    /// `1d` = C major). Both step by a fifth per number.
    fn parse_wheel(text: &str) -> Option<Self> {
        let split = text.find(|c: char| !c.is_ascii_digit())?;
        let (number, letter) = text.split_at(split);
        let number: i32 = number.parse().ok().filter(|n| (1..=12).contains(n))?;
        let (offset, mode) = match letter {
            "A" | "a" => (number - 8, Mode::Minor),
            "B" | "b" => (number - 8, Mode::Major),
            "m" => (number - 1, Mode::Minor),
            "d" => (number - 1, Mode::Major),
            _ => return None,
        };
        let base = match mode {
            Mode::Minor => 9, // A
            Mode::Major => 0, // C
        };
        Some(Self::new((base + 7 * offset).rem_euclid(12) as u8, mode))
    }

    fn parse_name(text: &str) -> Option<Self> {
        let mut chars = text.chars();
        let natural = match chars.next()?.to_ascii_uppercase() {
            'C' => 0,
            'D' => 2,
            'E' => 4,
            'F' => 5,
            'G' => 7,
            'A' => 9,
            'B' => 11,
            _ => return None,
        };
        let rest = chars.as_str();
        let (accidental, rest) = if let Some(rest) = rest.strip_prefix(['#', '♯']) {
            (1, rest)
        } else if let Some(rest) = rest.strip_prefix(['b', '♭']) {
            (-1, rest)
        } else {
            (0, rest)
        };
        let mode = match rest.trim() {
            "" | "maj" | "major" | "Maj" | "Major" | "M" => Mode::Major,
            "m" | "min" | "minor" | "Min" | "Minor" => Mode::Minor,
            _ => return None,
        };
        Some(Self::new((natural + accidental + 12) as u8, mode))
    }

    /// Keys that mix harmonically with this one: itself, its neighbors a
    /// fifth up and down (±1 on the Camelot wheel), and its relative
    /// major/minor.
    pub fn compatible_keys(self) -> [MusicalKey; 4] {
        let relative = match self.mode {
            Mode::Minor => MusicalKey::new(self.tonic + 3, Mode::Major),
            Mode::Major => MusicalKey::new(self.tonic + 9, Mode::Minor),
        };
        [
            self,
            MusicalKey::new(self.tonic + 7, self.mode),
            MusicalKey::new(self.tonic + 5, self.mode),
            relative,
        ]
    }

    /// Camelot notation, e.g. `8A` for A minor.
    pub fn camelot(self) -> String {
        let base = match self.mode {
            Mode::Minor => 9,
            Mode::Major => 0,
        };
        // Each step of a fifth (7 semitones) is one step on the wheel; 7 is its own inverse mod 12.
        let steps = (7 * (self.tonic as i32 - base)).rem_euclid(12);
        let number = (steps + 7) % 12 + 1;
        let letter = match self.mode {
            Mode::Minor => 'A',
            Mode::Major => 'B',
        };
        format!("{number}{letter}")
    }
}

impl fmt::Display for MusicalKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = NAMES[self.tonic as usize];
        match self.mode {
            Mode::Major => write!(f, "{name}"),
            Mode::Minor => write!(f, "{name}m"),
        }
    }
}

impl Serialize for MusicalKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for MusicalKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("not a musical key: {text}")))
    }
}

impl JsonSchema for MusicalKey {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "MusicalKey".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "Musical key, e.g. \"Am\", \"F#m\", \"Db\". Camelot (\"8A\") and Open Key (\"1m\") are accepted as input."
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> String {
        MusicalKey::parse(text)
            .map(|k| k.to_string())
            .unwrap_or_default()
    }

    #[test]
    fn parses_classic_names() {
        assert_eq!(key("Am"), "Am");
        assert_eq!(key("Amin"), "Am");
        assert_eq!(key("Dmin"), "Dm");
        assert_eq!(key("F#m"), "F#m");
        assert_eq!(key("Gbm"), "F#m");
        assert_eq!(key("Bbmaj"), "Bb");
        assert_eq!(key("C"), "C");
        assert_eq!(key("B"), "B");
        assert_eq!(key("Bm"), "Bm");
        assert_eq!(key("Db"), "Db");
        assert_eq!(key("Ab"), "Ab");
        assert_eq!(key("Ebmin"), "Ebm");
    }

    #[test]
    fn parses_camelot_and_open_key() {
        assert_eq!(key("8A"), "Am");
        assert_eq!(key("8B"), "C");
        assert_eq!(key("9A"), "Em");
        assert_eq!(key("5A"), "Cm");
        assert_eq!(key("12B"), "E");
        assert_eq!(key("1m"), "Am");
        assert_eq!(key("2m"), "Em");
        assert_eq!(key("6d"), "B");
        assert_eq!(key("12d"), "F");
    }

    #[test]
    fn rejects_placeholders() {
        assert_eq!(key("All"), "");
        assert_eq!(key(""), "");
        assert_eq!(key("13A"), "");
    }

    #[test]
    fn compatible_keys_are_camelot_neighbors() {
        let camelot = |k: &str| {
            MusicalKey::parse(k)
                .unwrap()
                .compatible_keys()
                .map(|k| k.camelot())
        };
        assert_eq!(camelot("8A"), ["8A", "9A", "7A", "8B"]);
        assert_eq!(camelot("12B"), ["12B", "1B", "11B", "12A"]);
        assert_eq!(camelot("1A"), ["1A", "2A", "12A", "1B"]);
    }

    #[test]
    fn camelot_round_trips_for_every_key() {
        for tonic in 0..12 {
            for mode in [Mode::Major, Mode::Minor] {
                let k = MusicalKey::new(tonic, mode);
                assert_eq!(MusicalKey::parse(&k.camelot()), Some(k), "{k}");
                assert_eq!(MusicalKey::parse(&k.to_string()), Some(k), "{k}");
            }
        }
    }
}
