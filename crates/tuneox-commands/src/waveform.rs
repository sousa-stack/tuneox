//! Waveform data for drawing a track, decoded on demand from its rekordbox
//! analysis files. Nothing is stored in the library.

use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tuneox_core::{BandGains, TrackId, WaveformResolution, WaveformStyle};
use tuneox_pioneer::anlz::{self, AnlzFile};

use crate::{Command, Context, Effect, Error, Result};

pub struct GetWaveform;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetWaveformInput {
    pub track_id: TrackId,
    pub style: WaveformStyle,
    /// `preview` spans the whole track; `detail` is the scrolling waveform.
    pub resolution: WaveformResolution,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetWaveformOutput {
    pub track_id: TrackId,
    pub style: WaveformStyle,
    pub resolution: WaveformResolution,
    pub columns: u32,
    /// Columns per second of audio for detail waveforms (150); null for
    /// previews, whose columns span the whole track.
    pub columns_per_second: Option<f64>,
    pub bytes_per_column: u32,
    /// `columns` × `bytes_per_column` bytes, base64 with padding.
    pub data_base64: String,
    /// `three_band` only: the gain rekordbox's analysis applied to each band.
    /// The levels already include it.
    pub band_gains: Option<BandGains>,
}

impl Command for GetWaveform {
    const NAME: &'static str = "get_waveform";
    const DESCRIPTION: &'static str = "Waveform data for drawing a track, from its rekordbox analysis. \
        A preview spans the whole track (400 columns for blue, 1200 for rgb and three_band); \
        a detail waveform has 150 columns per second. data_base64 holds one fixed-size record \
        per column, each value 0-255 where 255 is full scale: blue = height, whiteness; \
        rgb detail = red, green, blue, height; rgb preview = rekordbox's six overview values; \
        three_band = low, mid, high.";
    const EFFECT: Effect = Effect::Read;
    type Input = GetWaveformInput;
    type Output = GetWaveformOutput;

    fn run(ctx: &mut Context, input: GetWaveformInput) -> Result<GetWaveformOutput> {
        let GetWaveformInput {
            track_id,
            style,
            resolution,
        } = input;
        let what = format!("{} {} waveform", style.as_str(), resolution.as_str());
        let Some(dir) = ctx.library.analysis_dir(track_id)? else {
            return Err(match ctx.library.track(track_id)? {
                None => Error::NotFound(format!("track {track_id}")),
                Some(_) => Error::NotFound(format!(
                    "track {track_id} has no rekordbox analysis, so no {what}"
                )),
            });
        };
        let path = Path::new(&dir).join(format!(
            "ANLZ0000.{}",
            anlz::waveform_file_extension(style, resolution)
        ));
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::NotFound(format!(
                    "track {track_id} has no {what}: {} is missing",
                    path.display()
                )));
            }
            Err(e) => return Err(e.into()),
        };
        let waveform = AnlzFile::parse(bytes)
            .and_then(|anlz| anlz.waveform(style, resolution))
            .map_err(|e| {
                Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{}: {e}", path.display()),
                ))
            })?
            .ok_or_else(|| {
                Error::NotFound(format!(
                    "track {track_id} has no {what} in {}",
                    path.display()
                ))
            })?;
        Ok(GetWaveformOutput {
            track_id,
            style,
            resolution,
            columns: waveform.columns,
            columns_per_second: waveform.columns_per_second,
            bytes_per_column: waveform.bytes_per_column() as u32,
            data_base64: STANDARD.encode(&waveform.data),
            band_gains: waveform.band_gains,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde_json::{Value, json};
    use tuneox_core::TrackInfo;
    use tuneox_library::Library;

    use crate::{Context, Error, run_json};

    /// A waveform table section: entry size, entry count, rate field, entries.
    fn table(tag: &[u8; 4], entry_size: u32, entries: &[u8]) -> Vec<u8> {
        let mut out = tag.to_vec();
        out.extend(24u32.to_be_bytes());
        out.extend((24 + entries.len() as u32).to_be_bytes());
        out.extend(entry_size.to_be_bytes());
        out.extend((entries.len() as u32 / entry_size).to_be_bytes());
        out.extend(0x0096_0000u32.to_be_bytes());
        out.extend(entries);
        out
    }

    fn anlz(sections: &[Vec<u8>]) -> Vec<u8> {
        let body = sections.concat();
        let mut out = b"PMAI".to_vec();
        out.extend(28u32.to_be_bytes());
        out.extend((28 + body.len() as u32).to_be_bytes());
        out.extend([0; 16]);
        out.extend(body);
        out
    }

    /// A library with two tracks: 1 has analysis files (an `.EXT` with an
    /// rgb detail waveform, a `.2EX` with a 3-band preview, gains and a
    /// corrupt 3-band detail, no `.DAT`), 2 has none.
    fn context(dir: &Path) -> Context {
        // Two columns: red 7 / height 31, then green 4 / blue 7 / height 2.
        let pwv5 = [0xe0, 0x7c, 0x13, 0x88];
        std::fs::write(dir.join("ANLZ0000.EXT"), anlz(&[table(b"PWV5", 2, &pwv5)])).unwrap();
        let mut pwv7 = table(b"PWV7", 3, &[1, 2, 3, 4, 5, 6]);
        pwv7[16..20].copy_from_slice(&9u32.to_be_bytes()); // claims 9 entries
        let mut pwv6 = table(b"PWV6", 3, &[127, 0, 64]);
        pwv6.drain(20..24); // the preview header has no rate field
        pwv6[4..8].copy_from_slice(&20u32.to_be_bytes());
        pwv6[8..12].copy_from_slice(&23u32.to_be_bytes());
        let mut pwvc = b"PWVC".to_vec();
        pwvc.extend(14u32.to_be_bytes());
        pwvc.extend(20u32.to_be_bytes());
        pwvc.extend([0, 0, 0, 80, 0, 100, 1, 244]);
        std::fs::write(dir.join("ANLZ0000.2EX"), anlz(&[pwv7, pwv6, pwvc])).unwrap();

        let mut library = Library::open_in_memory().unwrap();
        let import = library.import("test", "lib").unwrap();
        let dir = dir.display().to_string();
        for (id, path, analysis) in [("1", "/a.aiff", Some(dir.as_str())), ("2", "/b.mp3", None)] {
            let info = TrackInfo {
                path: path.into(),
                title: id.into(),
                ..TrackInfo::default()
            };
            import.upsert_track(id, &info, analysis).unwrap();
        }
        import.commit().unwrap();
        Context::new(library, None)
    }

    fn get(ctx: &mut Context, track: i64, style: &str, resolution: &str) -> crate::Result<Value> {
        run_json(
            ctx,
            "get_waveform",
            json!({ "track_id": track, "style": style, "resolution": resolution }),
        )
    }

    #[test]
    fn returns_records_as_base64() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = context(dir.path());
        let out = get(&mut ctx, 1, "rgb", "detail").unwrap();
        assert_eq!(out["track_id"], 1);
        assert_eq!(out["style"], "rgb");
        assert_eq!(out["resolution"], "detail");
        assert_eq!(out["columns"], 2);
        assert_eq!(out["columns_per_second"], 150.0);
        assert_eq!(out["bytes_per_column"], 4);
        let data = STANDARD
            .decode(out["data_base64"].as_str().unwrap())
            .unwrap();
        assert_eq!(data, [255, 0, 0, 255, 0, 146, 255, 16]);
        assert_eq!(out["band_gains"], Value::Null);

        let out = get(&mut ctx, 1, "three_band", "preview").unwrap();
        assert_eq!(out["columns"], 1);
        assert_eq!(out["columns_per_second"], Value::Null);
        assert_eq!(out["bytes_per_column"], 3);
        let data = STANDARD
            .decode(out["data_base64"].as_str().unwrap())
            .unwrap();
        assert_eq!(data, [255, 0, 129], "low, mid, high");
        assert_eq!(
            out["band_gains"],
            json!({ "low": 0.8, "mid": 1.0, "high": 5.0 })
        );
    }

    #[test]
    fn explains_what_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = context(dir.path());
        let not_found = |r: crate::Result<Value>| match r {
            Err(Error::NotFound(msg)) => msg,
            other => panic!("expected NotFound, got {other:?}"),
        };
        assert_eq!(not_found(get(&mut ctx, 99, "rgb", "detail")), "track 99");
        assert!(not_found(get(&mut ctx, 2, "blue", "preview")).contains("no rekordbox analysis"));
        assert!(not_found(get(&mut ctx, 1, "blue", "preview")).contains("ANLZ0000.DAT is missing"));
        assert!(not_found(get(&mut ctx, 1, "rgb", "preview")).contains("no rgb preview waveform"));
        assert!(matches!(
            get(&mut ctx, 1, "three_band", "detail"),
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::InvalidData
        ));
        assert!(matches!(
            get(&mut ctx, 1, "sparkly", "detail"),
            Err(Error::InvalidInput(_))
        ));
    }
}
