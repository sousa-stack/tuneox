//! Research tool: lists the sections, beat grid summary, cues and waveforms
//! of rekordbox analysis files.
//!
//!     cargo run -p tuneox-pioneer --example anlz_dump -- path/to/ANLZ0000.EXT [...]

use std::time::Instant;

use tuneox_core::{WaveformResolution, WaveformStyle};
use tuneox_pioneer::anlz::AnlzFile;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for path in std::env::args().skip(1) {
        let anlz = AnlzFile::parse(std::fs::read(&path)?)?;
        let tags: Vec<String> = anlz
            .sections()
            .map(|s| String::from_utf8_lossy(&s.fourcc).into_owned())
            .collect();
        println!("{path}\n  sections: {}", tags.join(" "));
        if let Some(grid) = anlz.beat_grid()? {
            let first = grid.beats.first();
            println!(
                "  beat grid: {} beats, first at {} ms, bpm {:?}",
                grid.beats.len(),
                first.map_or(0, |b| b.time_us / 1000),
                grid.constant_bpm()
            );
        }
        for cue in anlz.cues()? {
            let slot = match cue.hot_cue {
                0 => "memory".to_string(),
                n => format!("hot {}", (b'A' + n as u8 - 1) as char),
            };
            let span = match cue.loop_end_ms {
                Some(end) => format!("{}–{} ms", cue.time_ms, end),
                None => format!("{} ms", cue.time_ms),
            };
            println!(
                "  cue: {slot:>8} @ {span} {}",
                cue.comment.as_deref().unwrap_or("")
            );
        }
        for style in WaveformStyle::ALL {
            for resolution in WaveformResolution::ALL {
                let started = Instant::now();
                let Some(waveform) = anlz.waveform(style, resolution)? else {
                    continue;
                };
                let elapsed = started.elapsed();
                let size = waveform.bytes_per_column();
                let max: Vec<u8> = (0..size)
                    .map(|field| waveform.iter_columns().map(|c| c[field]).max().unwrap_or(0))
                    .collect();
                let middle = waveform.column(waveform.columns / 2).unwrap_or(&[]);
                println!(
                    "  waveform {} {}: {} columns{}, {} bytes each, field max {max:?}, middle column {middle:?}, decoded in {elapsed:?}",
                    style.as_str(),
                    resolution.as_str(),
                    waveform.columns,
                    waveform
                        .duration_secs()
                        .map(|d| format!(
                            " ({d:.2} s at {}/s)",
                            waveform.columns_per_second.unwrap_or(0.0)
                        ))
                        .unwrap_or_default(),
                    size,
                );
            }
        }
        if let Some(gains) = anlz.three_band_gains()? {
            println!(
                "  3-band gains: low {:.2}, mid {:.2}, high {:.2}",
                gains.low, gains.mid, gains.high
            );
        }
    }
    Ok(())
}
