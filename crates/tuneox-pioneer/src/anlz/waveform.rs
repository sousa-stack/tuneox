//! Waveform sections of rekordbox analysis files, decoded into
//! [`tuneox_core::Waveform`].
//!
//! | Style | Preview (whole track) | Detail (150 columns/s) |
//! | --- | --- | --- |
//! | blue | `PWAV` in `.DAT`: 400 × 1 byte | `PWV3` in `.EXT`: 1 byte |
//! | rgb | `PWV4` in `.EXT`: 1200 × 6 bytes | `PWV5` in `.EXT`: 2 bytes |
//! | three_band | `PWV6` in `.2EX`: 1200 × 3 bytes | `PWV7` in `.2EX`: 3 bytes |
//!
//! `PWVC` in `.2EX` holds the per-band gains rekordbox applied to the 3-band
//! levels; they come along as [`Waveform::band_gains`]. Byte layouts and the
//! evidence for them are in `research/waveforms.md`.

use tuneox_core::{BandGains, Waveform, WaveformResolution, WaveformStyle};

use super::{AnlzFile, Error, Result, Section, be_u16, be_u32};

/// Columns per second of rekordbox's detail waveforms: one per half CDJ frame
/// (75 frames per second). The detail headers also carry this number.
pub const DETAIL_COLUMNS_PER_SECOND: f64 = 150.0;

/// The tag of the section holding a waveform.
pub fn waveform_tag(style: WaveformStyle, resolution: WaveformResolution) -> [u8; 4] {
    use WaveformResolution::*;
    use WaveformStyle::*;
    match (style, resolution) {
        (Blue, Preview) => *b"PWAV",
        (Blue, Detail) => *b"PWV3",
        (Rgb, Preview) => *b"PWV4",
        (Rgb, Detail) => *b"PWV5",
        (ThreeBand, Preview) => *b"PWV6",
        (ThreeBand, Detail) => *b"PWV7",
    }
}

/// The extension of the analysis file holding a waveform: `DAT`, `EXT` or
/// `2EX` (as in `ANLZ0000.EXT`).
pub fn waveform_file_extension(
    style: WaveformStyle,
    resolution: WaveformResolution,
) -> &'static str {
    match (style, resolution) {
        (WaveformStyle::Blue, WaveformResolution::Preview) => "DAT",
        (WaveformStyle::Blue | WaveformStyle::Rgb, _) => "EXT",
        (WaveformStyle::ThreeBand, _) => "2EX",
    }
}

/// Rescales a value from `0..=max` to `0..=255`, rounding to the nearest
/// step. Values above `max` saturate at 255.
pub fn widen(value: u8, max: u8) -> u8 {
    let (value, max) = (value.min(max) as u32, max.max(1) as u32);
    ((value * 510 + max) / (max * 2)) as u8
}

/// The inverse of [`widen`]: back from `0..=255` to `0..=max`, exactly for
/// every value `widen` produces.
pub fn narrow(value: u8, max: u8) -> u8 {
    ((value as u32 * max as u32 + 127) / 255) as u8
}

impl AnlzFile {
    /// One of the six waveforms, or `None` if this file doesn't have it (see
    /// [`waveform_file_extension`] for which file to read).
    pub fn waveform(
        &self,
        style: WaveformStyle,
        resolution: WaveformResolution,
    ) -> Result<Option<Waveform>> {
        let Some(section) = self.section(&waveform_tag(style, resolution)) else {
            return Ok(None);
        };
        let (columns_per_second, data) = match (style, resolution) {
            (WaveformStyle::Blue, WaveformResolution::Preview) => {
                (None, blue(blue_preview(&section)?))
            }
            (WaveformStyle::Blue, WaveformResolution::Detail) => {
                (Some(detail_rate(&section)), blue(entries(&section, 1)?))
            }
            (WaveformStyle::Rgb, WaveformResolution::Preview) => {
                (None, entries(&section, 6)?.to_vec())
            }
            (WaveformStyle::Rgb, WaveformResolution::Detail) => (
                Some(detail_rate(&section)),
                rgb_detail(entries(&section, 2)?),
            ),
            (WaveformStyle::ThreeBand, WaveformResolution::Preview) => {
                (None, three_band(entries(&section, 3)?))
            }
            (WaveformStyle::ThreeBand, WaveformResolution::Detail) => (
                Some(detail_rate(&section)),
                three_band(entries(&section, 3)?),
            ),
        };
        let band_gains = match style {
            WaveformStyle::ThreeBand => self.three_band_gains()?,
            _ => None,
        };
        let columns = (data.len() / style.bytes_per_column(resolution)) as u32;
        Ok(Some(Waveform {
            style,
            columns_per_second,
            columns,
            data,
            band_gains,
        }))
    }

    /// The 3-band gains from the `PWVC` section (`.2EX`).
    pub fn three_band_gains(&self) -> Result<Option<BandGains>> {
        let Some(section) = self.section(b"PWVC") else {
            return Ok(None);
        };
        // Low, mid, high as u16s, each gain × 100.
        let gain = |i: usize| {
            be_u16(section.body, i * 2)
                .map(|v| v as f64 / 100.0)
                .ok_or(Error::Corrupt(body_offset(&section)))
        };
        Ok(Some(BandGains {
            low: gain(0)?,
            mid: gain(1)?,
            high: gain(2)?,
        }))
    }
}

fn body_offset(section: &Section) -> usize {
    section.offset + section.header.len()
}

/// The entries of a waveform table: the header holds the entry size (u32 at
/// byte 12) and the entry count (u32 at byte 16).
fn entries<'a>(section: &Section<'a>, entry_size: usize) -> Result<&'a [u8]> {
    let size = be_u32(section.header, 12).ok_or(Error::Corrupt(section.offset))? as usize;
    let count = be_u32(section.header, 16).ok_or(Error::Corrupt(section.offset))? as usize;
    if size != entry_size {
        return Err(Error::Corrupt(section.offset));
    }
    count
        .checked_mul(size)
        .and_then(|len| section.body.get(..len))
        .ok_or(Error::Corrupt(body_offset(section)))
}

/// `PWAV` has no entry size: its header holds the column count (u32 at byte
/// 12), one byte per column.
fn blue_preview<'a>(section: &Section<'a>) -> Result<&'a [u8]> {
    let count = be_u32(section.header, 12).ok_or(Error::Corrupt(section.offset))? as usize;
    section
        .body
        .get(..count)
        .ok_or(Error::Corrupt(body_offset(section)))
}

/// Detail headers end with a u32 whose high half is the column rate (always
/// 150 so far).
fn detail_rate(section: &Section) -> f64 {
    match be_u16(section.header, 20) {
        Some(rate) if rate > 0 => rate as f64,
        _ => DETAIL_COLUMNS_PER_SECOND,
    }
}

/// `PWAV`/`PWV3` byte: height in the low 5 bits, whiteness in the high 3.
fn blue(entries: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(entries.len() * 2);
    for &b in entries {
        out.push(widen(b & 0x1f, 31));
        out.push(widen(b >> 5, 7));
    }
    out
}

/// `PWV5` big-endian u16: red bits 15–13, green 12–10, blue 9–7, height 6–2.
fn rgb_detail(entries: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(entries.len() * 2);
    for pair in entries.as_chunks::<2>().0 {
        let v = u16::from_be_bytes(*pair);
        out.push(widen((v >> 13) as u8 & 7, 7));
        out.push(widen((v >> 10) as u8 & 7, 7));
        out.push(widen((v >> 7) as u8 & 7, 7));
        out.push(widen((v >> 2) as u8 & 31, 31));
    }
    out
}

/// `PWV6`/`PWV7` entry: low, mid and high levels, 0–127 each. (Deep
/// Symmetry's notes say mid, high, low; rekordbox's own accessors and the
/// data say otherwise, see `research/waveforms.md`.)
fn three_band(entries: &[u8]) -> Vec<u8> {
    entries.iter().map(|&level| widen(level, 127)).collect()
}

#[cfg(test)]
mod tests {
    use super::super::tests::{file, section};
    use super::*;

    /// A waveform table section: entry size, entry count and (for detail
    /// sections) the rate field, then the entries.
    fn table(tag: &[u8; 4], entry_size: u32, entries: &[u8], rate: Option<u32>) -> Vec<u8> {
        let count = entries.len() as u32 / entry_size;
        let mut header = entry_size.to_be_bytes().to_vec();
        header.extend(count.to_be_bytes());
        if let Some(rate) = rate {
            header.extend(rate.to_be_bytes());
        }
        section(tag, &header, entries)
    }

    fn pwav(bytes: &[u8]) -> Vec<u8> {
        let mut header = (bytes.len() as u32).to_be_bytes().to_vec();
        header.extend(0x0001_0000u32.to_be_bytes());
        section(b"PWAV", &header, bytes)
    }

    fn rgb_entry(r: u16, g: u16, b: u16, height: u16) -> [u8; 2] {
        (r << 13 | g << 10 | b << 7 | height << 2).to_be_bytes()
    }

    fn parse(sections: &[Vec<u8>]) -> AnlzFile {
        AnlzFile::parse(file(sections)).unwrap()
    }

    #[test]
    fn widen_and_narrow_round_trip() {
        for max in [7u8, 15, 31, 127] {
            for v in 0..=max {
                let wide = widen(v, max);
                assert_eq!(narrow(wide, max), v, "max {max} value {v}");
            }
            assert_eq!(widen(0, max), 0);
            assert_eq!(widen(max, max), 255);
        }
        assert_eq!(widen(1, 7), 36);
        assert_eq!(widen(4, 7), 146);
        assert_eq!(widen(16, 31), 132);
        assert_eq!(widen(64, 127), 129);
        assert_eq!(widen(200, 127), 255, "out-of-range values saturate");
    }

    #[test]
    fn decodes_blue() {
        // Height 31, whiteness 7; height 0, whiteness 0; height 16, whiteness 2.
        let bytes = [0xff, 0x00, 0x50];
        let anlz = parse(&[pwav(&bytes), table(b"PWV3", 1, &bytes, Some(0x0096_0000))]);

        let preview = anlz
            .waveform(WaveformStyle::Blue, WaveformResolution::Preview)
            .unwrap()
            .unwrap();
        assert_eq!(preview.columns, 3);
        assert_eq!(preview.columns_per_second, None);
        assert_eq!(preview.data, [255, 255, 0, 0, 132, 73]);

        let detail = anlz
            .waveform(WaveformStyle::Blue, WaveformResolution::Detail)
            .unwrap()
            .unwrap();
        assert_eq!(detail.columns_per_second, Some(150.0));
        assert_eq!(detail.data, preview.data);
    }

    #[test]
    fn decodes_rgb() {
        let detail: Vec<u8> = [rgb_entry(7, 0, 3, 31), rgb_entry(1, 4, 7, 2)].concat();
        let preview: Vec<u8> = (0..12).collect();
        let anlz = parse(&[
            table(b"PWV5", 2, &detail, Some(0x0096_0305)),
            table(b"PWV4", 6, &preview, Some(0)),
        ]);

        let rgb = anlz
            .waveform(WaveformStyle::Rgb, WaveformResolution::Detail)
            .unwrap()
            .unwrap();
        assert_eq!(rgb.columns, 2);
        assert_eq!(rgb.columns_per_second, Some(150.0));
        assert_eq!(rgb.column(0), Some(&[255, 0, 109, 255][..]));
        assert_eq!(rgb.column(1), Some(&[36, 146, 255, 16][..]));

        let overview = anlz
            .waveform(WaveformStyle::Rgb, WaveformResolution::Preview)
            .unwrap()
            .unwrap();
        assert_eq!(overview.columns, 2);
        assert_eq!(overview.bytes_per_column(), 6);
        assert_eq!(overview.data, preview, "PWV4 passes through unchanged");
    }

    #[test]
    fn decodes_three_band_and_gains() {
        // Low, mid, high per column.
        let entries = [10, 20, 127, 0, 64, 1];
        let anlz = parse(&[
            table(b"PWV7", 3, &entries, Some(0x0096_0000)),
            table(b"PWV6", 3, &entries, None),
            section(b"PWVC", &[0, 0], &[0, 83, 0, 113, 0, 103]),
        ]);

        let detail = anlz
            .waveform(WaveformStyle::ThreeBand, WaveformResolution::Detail)
            .unwrap()
            .unwrap();
        assert_eq!(detail.columns, 2);
        assert_eq!(detail.column(0), Some(&[20, 40, 255][..]));
        assert_eq!(detail.column(1), Some(&[0, 129, 2][..]));
        let gains = BandGains {
            low: 0.83,
            mid: 1.13,
            high: 1.03,
        };
        assert_eq!(detail.band_gains, Some(gains));

        let preview = anlz
            .waveform(WaveformStyle::ThreeBand, WaveformResolution::Preview)
            .unwrap()
            .unwrap();
        assert_eq!(preview.columns_per_second, None);
        assert_eq!(preview.data, detail.data);
        assert_eq!(preview.band_gains, Some(gains));
        assert_eq!(anlz.three_band_gains().unwrap(), Some(gains));

        // Without PWVC the levels still decode.
        let no_gains = parse(&[table(b"PWV6", 3, &entries, None)]);
        let preview = no_gains
            .waveform(WaveformStyle::ThreeBand, WaveformResolution::Preview)
            .unwrap()
            .unwrap();
        assert_eq!(preview.band_gains, None);
    }

    #[test]
    fn uses_the_header_rate() {
        let anlz = parse(&[table(b"PWV3", 1, &[1, 2, 3], Some(0x0064_0000))]);
        let detail = anlz
            .waveform(WaveformStyle::Blue, WaveformResolution::Detail)
            .unwrap()
            .unwrap();
        assert_eq!(detail.columns_per_second, Some(100.0));
        assert_eq!(detail.duration_secs(), Some(0.03));

        let zero_rate = parse(&[table(b"PWV3", 1, &[1], Some(0))]);
        let detail = zero_rate
            .waveform(WaveformStyle::Blue, WaveformResolution::Detail)
            .unwrap()
            .unwrap();
        assert_eq!(detail.columns_per_second, Some(DETAIL_COLUMNS_PER_SECOND));
    }

    #[test]
    fn missing_sections_are_none() {
        let anlz = parse(&[section(b"PPTH", &[0; 4], &[])]);
        for style in WaveformStyle::ALL {
            for resolution in WaveformResolution::ALL {
                assert!(anlz.waveform(style, resolution).unwrap().is_none());
            }
        }
        assert!(anlz.three_band_gains().unwrap().is_none());
    }

    #[test]
    fn rejects_corrupt_tables() {
        // Claims 4 entries, holds 3.
        let mut short = table(b"PWV7", 3, &[0; 9], Some(0x0096_0000));
        short[16..20].copy_from_slice(&4u32.to_be_bytes());
        // Wrong entry size for PWV5.
        let wrong_size = table(b"PWV5", 3, &[0; 9], Some(0x0096_0305));
        // Header too short to hold the entry count.
        let no_count = section(b"PWV4", &6u32.to_be_bytes(), &[0; 6]);
        let pwav_short = {
            let mut s = pwav(&[0; 4]);
            s[12..16].copy_from_slice(&400u32.to_be_bytes());
            s
        };
        let pwvc_short = section(b"PWVC", &[0, 0], &[0, 80]);

        let good_pwv6 = table(b"PWV6", 3, &[0; 3], None);

        let anlz = parse(&[
            short, wrong_size, no_count, pwav_short, pwvc_short, good_pwv6,
        ]);
        use WaveformResolution::*;
        use WaveformStyle::*;
        for (style, resolution) in [
            (ThreeBand, Detail),
            (ThreeBand, Preview), // fine levels, corrupt gains
            (Rgb, Detail),
            (Rgb, Preview),
            (Blue, Preview),
        ] {
            assert!(
                matches!(anlz.waveform(style, resolution), Err(Error::Corrupt(_))),
                "{style:?} {resolution:?}"
            );
        }
        assert!(matches!(anlz.three_band_gains(), Err(Error::Corrupt(_))));
    }

    #[test]
    fn file_and_tag_per_waveform() {
        use WaveformResolution::*;
        use WaveformStyle::*;
        assert_eq!(waveform_file_extension(Blue, Preview), "DAT");
        assert_eq!(waveform_file_extension(Blue, Detail), "EXT");
        assert_eq!(waveform_file_extension(Rgb, Preview), "EXT");
        assert_eq!(waveform_file_extension(ThreeBand, Detail), "2EX");
        assert_eq!(&waveform_tag(ThreeBand, Preview), b"PWV6");
    }
}
