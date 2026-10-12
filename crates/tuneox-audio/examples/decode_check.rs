//! Decodes audio files the way a deck load does and reports length and
//! speed. Never plays anything.
//!
//! ```bash
//! cargo run --release -p tuneox-audio --example decode_check -- file.mp3 file.aiff …
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tuneox_audio::{Engine, LoadRequest, Source};

fn main() {
    let engine = Engine::offline(48_000);
    let mut failed = false;
    for path in std::env::args().skip(1) {
        let started = Instant::now();
        let request = LoadRequest {
            track_id: None,
            source: Source::File(PathBuf::from(&path)),
            beat_grid: None,
            cues: Vec::new(),
            bpm: None,
            duration_secs: None,
            auto_gain_db: None,
        };
        let first = engine.load(1, request).expect("load");
        let state = engine
            .wait_until_loaded(1, Duration::from_secs(120))
            .expect("decode finished");
        let elapsed = started.elapsed();
        match &state.error {
            Some(error) => {
                failed = true;
                println!("FAIL  {path}: {error}");
            }
            None => {
                let secs = state.duration_us as f64 / 1e6;
                println!(
                    "ok    {secs:8.2} s  decoded in {:6.0} ms ({:4.0}× real time)  estimated {:8.2} s  {path}",
                    elapsed.as_secs_f64() * 1000.0,
                    secs / elapsed.as_secs_f64(),
                    first.duration_us as f64 / 1e6,
                );
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
}
