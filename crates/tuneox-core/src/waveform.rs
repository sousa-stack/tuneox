use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How a waveform is colored. Each style has its own per-column record; see
/// [`Waveform`] for the layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WaveformStyle {
    /// Monochrome blue: a height plus a "whiteness" that tints the column
    /// from deep blue towards white (whiter = more treble).
    Blue,
    /// Color: low, mid and high energy shown as red, green and blue.
    Rgb,
    /// CDJ-3000 style: separate low, mid and high band levels.
    ThreeBand,
}

impl WaveformStyle {
    pub const ALL: [WaveformStyle; 3] = [Self::Blue, Self::Rgb, Self::ThreeBand];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blue => "blue",
            Self::Rgb => "rgb",
            Self::ThreeBand => "three_band",
        }
    }

    /// Size of one column's record at a resolution.
    pub fn bytes_per_column(self, resolution: WaveformResolution) -> usize {
        match (self, resolution) {
            (Self::Blue, _) => 2,
            (Self::Rgb, WaveformResolution::Preview) => 6,
            (Self::Rgb, WaveformResolution::Detail) => 4,
            (Self::ThreeBand, _) => 3,
        }
    }
}

/// Whole-track overview or scrolling detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WaveformResolution {
    /// A fixed number of columns spanning the whole track (400 for `blue`,
    /// 1200 for `rgb` and `three_band` in rekordbox's analysis).
    Preview,
    /// A fixed number of columns per second of audio (150 in rekordbox's
    /// analysis), for the scrolling waveform.
    Detail,
}

impl WaveformResolution {
    pub const ALL: [WaveformResolution; 2] = [Self::Preview, Self::Detail];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preview => "preview",
            Self::Detail => "detail",
        }
    }
}

/// A waveform as fixed-size records, one per column, ready to draw.
///
/// Every value is a byte where 0 is nothing and 255 is the full scale of the
/// source format, so a height of 255 fills the lane. Records by style:
///
/// | Style | Resolution | Bytes | Record |
/// | --- | --- | --- | --- |
/// | `blue` | both | 2 | height, whiteness |
/// | `rgb` | detail | 4 | red, green, blue, height |
/// | `rgb` | preview | 6 | rekordbox's six `PWV4` values, unchanged |
/// | `three_band` | both | 3 | low, mid, high |
///
/// - `blue`: whiteness picks the column's tint, 0 = deepest blue, 255 = near
///   white. rekordbox draws eight tints; `whiteness * 7 / 255` (rounded) is
///   the tint index.
/// - `rgb` detail: red, green and blue are the column's color (rekordbox stores
///   3 bits each, so they come in steps of 255/7), height its amplitude.
/// - `rgb` preview: rekordbox's color overview keeps six separate measures per
///   column, and how it combines them into a drawing isn't settled yet, so
///   they pass through as stored (each 0–255; in practice mostly 0–127). See
///   `research/waveforms.md`.
/// - `three_band`: each band's level, already smoothed and gain-adjusted by
///   rekordbox's analysis (see [`BandGains`]). The preview's bands are drawn
///   stacked, the detail's overlaid on one axis.
///
/// Values decoded from Pioneer's narrower fields are rescaled with
/// `round(v * 255 / max)`, which `round(x * max / 255)` reverses exactly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Waveform {
    pub style: WaveformStyle,
    /// Columns per second of audio for detail waveforms; `None` for
    /// previews, whose columns span the whole track.
    pub columns_per_second: Option<f64>,
    pub columns: u32,
    /// `columns` records of [`Waveform::bytes_per_column`] bytes each.
    pub data: Vec<u8>,
    /// `three_band` only: the gain the analysis applied to each band, when
    /// known.
    pub band_gains: Option<BandGains>,
}

/// The gain a 3-band analysis multiplied each band's levels by, to balance
/// the bands and fill the range (rekordbox's `PWVC`: 0.8–3.0 for low and
/// mid, 0.8–5.0 for high). The stored levels already include it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BandGains {
    pub low: f64,
    pub mid: f64,
    pub high: f64,
}

impl Waveform {
    /// A waveform from its records, or `None` if `data` isn't a whole number
    /// of records for the style and resolution.
    pub fn new(
        style: WaveformStyle,
        columns_per_second: Option<f64>,
        data: Vec<u8>,
    ) -> Option<Self> {
        let resolution = match columns_per_second {
            Some(_) => WaveformResolution::Detail,
            None => WaveformResolution::Preview,
        };
        let size = style.bytes_per_column(resolution);
        if !data.len().is_multiple_of(size) {
            return None;
        }
        Some(Self {
            style,
            columns_per_second,
            columns: u32::try_from(data.len() / size).ok()?,
            data,
            band_gains: None,
        })
    }

    pub fn resolution(&self) -> WaveformResolution {
        match self.columns_per_second {
            Some(_) => WaveformResolution::Detail,
            None => WaveformResolution::Preview,
        }
    }

    pub fn bytes_per_column(&self) -> usize {
        self.style.bytes_per_column(self.resolution())
    }

    /// The record of one column.
    pub fn column(&self, index: u32) -> Option<&[u8]> {
        let size = self.bytes_per_column();
        let start = index as usize * size;
        self.data.get(start..start + size)
    }

    /// Every column's record, in order.
    pub fn iter_columns(&self) -> impl Iterator<Item = &[u8]> {
        self.data.chunks_exact(self.bytes_per_column())
    }

    /// How much audio the columns cover, for detail waveforms.
    pub fn duration_secs(&self) -> Option<f64> {
        let rate = self.columns_per_second.filter(|r| *r > 0.0)?;
        Some(self.columns as f64 / rate)
    }

    /// The column under a position in the track, for detail waveforms.
    pub fn column_at_us(&self, position_us: i64) -> Option<u32> {
        let rate = self.columns_per_second.filter(|r| *r > 0.0)?;
        let index = (position_us as f64 * rate / 1e6).floor();
        (index >= 0.0 && index < self.columns as f64).then_some(index as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_by_style() {
        let rgb = Waveform::new(WaveformStyle::Rgb, Some(150.0), vec![0; 4 * 300]).unwrap();
        assert_eq!(rgb.columns, 300);
        assert_eq!(rgb.bytes_per_column(), 4);
        assert_eq!(rgb.duration_secs(), Some(2.0));
        assert_eq!(rgb.column_at_us(1_000_000), Some(150));
        assert_eq!(rgb.column_at_us(2_000_000), None);
        assert_eq!(rgb.iter_columns().count(), 300);

        let preview = Waveform::new(WaveformStyle::Rgb, None, vec![1; 6 * 1200]).unwrap();
        assert_eq!(preview.resolution(), WaveformResolution::Preview);
        assert_eq!(preview.column(1199), Some(&[1u8; 6][..]));
        assert_eq!(preview.column(1200), None);
        assert_eq!(preview.duration_secs(), None);

        assert!(Waveform::new(WaveformStyle::ThreeBand, None, vec![0; 4]).is_none());
    }
}
