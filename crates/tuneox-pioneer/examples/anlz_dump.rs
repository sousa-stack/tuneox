//! Research tool: lists the sections, beat grid summary and cues of rekordbox
//! analysis files.
//!
//!     cargo run -p tuneox-pioneer --example anlz_dump -- path/to/ANLZ0000.EXT [...]

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
    }
    Ok(())
}
