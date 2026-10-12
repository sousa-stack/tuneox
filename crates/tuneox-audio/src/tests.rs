//! Offline engine tests on synthetic audio generated here: click tracks on
//! known beat grids and tones. Nothing is played.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tuneox_core::{Beat, BeatGrid, Cue};

use crate::core::Control;
use crate::deck::FADE;
use crate::engine::lock;
use crate::{Engine, EngineEvent, Error, LoadRequest, MixerSettingsUpdate, Source};

const RATE: u32 = 48_000;
/// One output sample at 48 kHz, in microseconds.
const SAMPLE_US: f64 = 1e6 / RATE as f64;

// Allocation counting ------------------------------------------------------

/// Counts allocations made on threads that ask for it, to prove the render
/// path doesn't allocate.
struct CountingAllocator;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
}
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

fn count_here() {
    if COUNTING.with(Cell::get) {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_here();
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count_here();
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_here();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count_here();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

// Synthetic audio ------------------------------------------------------------

/// Stereo clicks: a few samples of a decaying pulse at every beat.
fn click_track(sample_rate: u32, bpm: f64, first_beat: f64, seconds: f64) -> Vec<f32> {
    let frames = (seconds * sample_rate as f64) as usize;
    let mut samples = vec![0.0f32; frames * 2];
    let period = 60.0 / bpm;
    let mut beat = first_beat;
    while beat < seconds {
        let start = (beat * sample_rate as f64).round() as usize;
        for k in 0..32 {
            if let Some(frame) = samples.get_mut((start + k) * 2..(start + k) * 2 + 2) {
                let v = 0.8 * (1.0 - k as f32 / 32.0) * if k % 2 == 0 { 1.0 } else { -1.0 };
                frame[0] = v;
                frame[1] = v;
            }
        }
        beat += period;
    }
    samples
}

/// Stereo sum of sines `(frequency, amplitude)`.
fn tones(sample_rate: u32, seconds: f64, parts: &[(f64, f32)]) -> Vec<f32> {
    let frames = (seconds * sample_rate as f64) as usize;
    let mut samples = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let t = i as f64 / sample_rate as f64;
        let v: f32 = parts
            .iter()
            .map(|&(f, a)| a * (2.0 * std::f64::consts::PI * f * t).sin() as f32)
            .sum();
        // On the 16-bit grid, so storage is lossless and output comparable.
        let v = (v * 32768.0).round() / 32768.0;
        samples.push(v);
        samples.push(v);
    }
    samples
}

/// Deterministic noise, so every sample differs from its neighbours.
fn noise(sample_rate: u32, seconds: f64) -> Vec<f32> {
    let frames = (seconds * sample_rate as f64) as usize;
    let mut state = 0x2545_f491_u32;
    (0..frames * 2)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            // Exactly representable in 16 bits, so storage is lossless.
            ((state >> 16) as i16 / 2) as f32 / 32768.0
        })
        .collect()
}

fn grid(first_beat: f64, bpm: f64, beats: usize) -> BeatGrid {
    let period = 60.0 / bpm;
    BeatGrid {
        beats: (0..beats)
            .map(|i| Beat {
                time_us: ((first_beat + i as f64 * period) * 1e6).round() as i64,
                bar_position: (i % 4) as u8 + 1,
                bpm_centi: (bpm * 100.0).round() as u16,
            })
            .collect(),
    }
}

fn hot_cue(slot: u8, start: f64, end: Option<f64>) -> Cue {
    Cue {
        hot_cue: Some(slot),
        start_us: (start * 1e6) as i64,
        end_us: end.map(|e| (e * 1e6) as i64),
        color: None,
        comment: None,
    }
}

fn memory_cue(start: f64) -> Cue {
    Cue {
        hot_cue: None,
        ..hot_cue(0, start, None)
    }
}

fn samples(
    sample_rate: u32,
    samples: Vec<f32>,
    grid: Option<BeatGrid>,
    cues: Vec<Cue>,
) -> LoadRequest {
    LoadRequest {
        track_id: Some(7),
        source: Source::Samples {
            sample_rate,
            channels: 2,
            samples,
        },
        beat_grid: grid,
        cues,
        bpm: None,
        duration_secs: None,
    }
}

fn render(engine: &Engine, seconds: f64) -> Vec<f32> {
    let frames = (seconds * engine.sample_rate() as f64).round() as usize;
    let mut out = vec![0.0; frames * 2];
    engine.render(&mut out);
    out
}

fn left(out: &[f32]) -> Vec<f32> {
    out.iter().step_by(2).copied().collect()
}

/// Amplitude of `freq` in `signal` (Goertzel).
fn amplitude(signal: &[f32], freq: f64, sample_rate: u32) -> f64 {
    let w = 2.0 * std::f64::consts::PI * freq / sample_rate as f64;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &x in signal {
        let s0 = x as f64 + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
    2.0 * power.sqrt() / signal.len() as f64
}

fn db(ratio: f64) -> f64 {
    20.0 * ratio.log10()
}

fn us(seconds: f64) -> i64 {
    (seconds * 1e6).round() as i64
}

fn assert_near(actual: i64, expected: i64, tolerance_us: f64, what: &str) {
    assert!(
        ((actual - expected) as f64).abs() <= tolerance_us,
        "{what}: {actual} µs, expected {expected} µs (±{tolerance_us})"
    );
}

/// Deck 1 only, crossfader hard left so its gain is exactly 1.
fn deck_one_only(engine: &Engine) {
    engine
        .set_mixer(MixerSettingsUpdate {
            crossfader: Some(-1.0),
            ..Default::default()
        })
        .unwrap();
}

// Loading ----------------------------------------------------------------------

#[test]
fn loads_a_track_with_grid_and_cues() {
    let engine = Engine::offline(RATE);
    let state = engine
        .load(
            1,
            samples(
                RATE,
                click_track(RATE, 120.0, 0.0, 10.0),
                Some(grid(0.0, 120.0, 20)),
                vec![memory_cue(2.0), memory_cue(1.0), hot_cue(2, 4.0, None)],
            ),
        )
        .unwrap();
    assert_eq!(state.deck, 1);
    assert_eq!(state.track_id, Some(7));
    assert!(!state.loading);
    assert_eq!(state.error, None);
    assert_eq!(state.duration_us, 10_000_000);
    assert!(!state.playing);
    assert_eq!(state.rate, 0.0);
    // The first memory cue is the cue point, and the deck waits there.
    assert_eq!(state.cue_us, 1_000_000);
    assert_eq!(state.position_us, 1_000_000);
    assert_eq!(state.original_bpm, Some(120.0));
    assert_eq!(state.bpm, Some(120.0));
    assert_eq!(state.hot_cues.len(), 1);
    assert_eq!(state.hot_cues[0].slot, 2);
    assert_eq!(state.hot_cues[0].start_us, 4_000_000);
    assert_eq!(engine.deck(2).unwrap().track_id, None);
}

fn write_wav(path: &Path, sample_rate: u32, samples: &[f32]) {
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|&s| ((s * 32768.0).round() as i16).to_le_bytes())
        .collect();
    let mut file = Vec::new();
    file.extend_from_slice(b"RIFF");
    file.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    file.extend_from_slice(b"WAVEfmt ");
    file.extend_from_slice(&16u32.to_le_bytes());
    file.extend_from_slice(&1u16.to_le_bytes());
    file.extend_from_slice(&2u16.to_le_bytes());
    file.extend_from_slice(&sample_rate.to_le_bytes());
    file.extend_from_slice(&(sample_rate * 4).to_le_bytes());
    file.extend_from_slice(&4u16.to_le_bytes());
    file.extend_from_slice(&16u16.to_le_bytes());
    file.extend_from_slice(b"data");
    file.extend_from_slice(&(data.len() as u32).to_le_bytes());
    file.extend_from_slice(&data);
    std::fs::write(path, file).unwrap();
}

/// 80-bit IEEE extended, as AIFF stores its sample rate.
fn extended(value: u32) -> [u8; 10] {
    let mut mantissa = value as u64;
    let mut exponent: u16 = 16383 + 63;
    while mantissa & (1 << 63) == 0 {
        mantissa <<= 1;
        exponent -= 1;
    }
    let mut out = [0u8; 10];
    out[..2].copy_from_slice(&exponent.to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

fn write_aiff(path: &Path, sample_rate: u32, samples: &[f32]) {
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|&s| ((s * 32768.0).round() as i16).to_be_bytes())
        .collect();
    let frames = samples.len() as u32 / 2;
    let mut comm = Vec::new();
    comm.extend_from_slice(&2i16.to_be_bytes());
    comm.extend_from_slice(&frames.to_be_bytes());
    comm.extend_from_slice(&16i16.to_be_bytes());
    comm.extend_from_slice(&extended(sample_rate));
    let mut body = Vec::new();
    body.extend_from_slice(b"AIFF");
    body.extend_from_slice(b"COMM");
    body.extend_from_slice(&(comm.len() as u32).to_be_bytes());
    body.extend_from_slice(&comm);
    body.extend_from_slice(b"SSND");
    body.extend_from_slice(&(data.len() as u32 + 8).to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&data);
    let mut file = Vec::new();
    file.extend_from_slice(b"FORM");
    file.extend_from_slice(&(body.len() as u32).to_be_bytes());
    file.extend_from_slice(&body);
    std::fs::write(path, file).unwrap();
}

fn file(path: &Path) -> LoadRequest {
    LoadRequest {
        track_id: Some(1),
        source: Source::File(path.to_path_buf()),
        beat_grid: None,
        cues: Vec::new(),
        bpm: None,
        duration_secs: None,
    }
}

#[test]
fn decodes_wav_and_aiff_files_and_plays_them_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let source = noise(RATE, 3.0);
    let wav = dir.path().join("noise.wav");
    let aiff = dir.path().join("noise.aiff");
    write_wav(&wav, RATE, &source);
    write_aiff(&aiff, RATE, &source);
    for path in [&wav, &aiff] {
        let engine = Engine::offline(RATE);
        deck_one_only(&engine);
        engine.load(1, file(path)).unwrap();
        let state = engine
            .wait_until_loaded(1, Duration::from_secs(10))
            .unwrap();
        assert_eq!(state.error, None, "{}", path.display());
        assert!(!state.loading);
        assert_eq!(state.duration_us, 3_000_000, "{}", path.display());
        engine.play(1).unwrap();
        let out = render(&engine, 1.0);
        // Past the start fade, output is the file's samples exactly.
        for i in FADE as usize..RATE as usize {
            for ch in 0..2 {
                let (got, want) = (out[2 * i + ch], source[2 * i + ch]);
                assert!(
                    (got - want).abs() < 1e-6,
                    "{} sample {i}: {got} vs {want}",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn reports_decode_errors_as_deck_state() {
    let dir = tempfile::tempdir().unwrap();
    let junk = dir.path().join("junk.mp3");
    std::fs::write(&junk, vec![0x55u8; 10_000]).unwrap();
    let engine = Engine::offline(RATE);
    for path in [dir.path().join("missing.aiff"), junk] {
        engine.load(1, file(&path)).unwrap();
        let state = engine
            .wait_until_loaded(1, Duration::from_secs(10))
            .unwrap();
        assert!(!state.loading);
        assert!(state.error.is_some(), "{}", path.display());
        // Playing a broken track is harmless silence.
        engine.play(1).unwrap();
        assert!(render(&engine, 0.1).iter().all(|&s| s == 0.0));
    }
}

#[test]
fn loading_the_other_decks_file_reuses_its_audio() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("tone.wav");
    write_wav(&wav, RATE, &tones(RATE, 2.0, &[(440.0, 0.5)]));
    let engine = Engine::offline(RATE);
    engine.load(1, file(&wav)).unwrap();
    engine
        .wait_until_loaded(1, Duration::from_secs(10))
        .unwrap();
    let state = engine.load(2, file(&wav)).unwrap();
    assert!(!state.loading, "instant double");
    assert_eq!(state.duration_us, 2_000_000);
}

// Transport --------------------------------------------------------------------

#[test]
fn play_and_pause_move_the_clock() {
    let engine = Engine::offline(RATE);
    engine
        .load(1, samples(RATE, noise(RATE, 10.0), None, vec![]))
        .unwrap();
    let state = engine.play(1).unwrap();
    assert!(state.playing);
    assert_eq!(state.rate, 1.0);
    assert_eq!(state.at_ms, 0.0);
    render(&engine, 0.5);
    let state = engine.deck(1).unwrap();
    assert_eq!(state.at_ms, 500.0);
    assert_near(state.position_us, 500_000, 0.5, "after 0.5 s");
    // The UI's extrapolation from an earlier sample lands on the same spot.
    render(&engine, 0.25);
    let later = engine.deck(1).unwrap();
    let predicted = state.position_us as f64 + (later.at_ms - state.at_ms) * 1000.0 * state.rate;
    assert_near(later.position_us, predicted as i64, 1.0, "extrapolated");

    let paused = engine.pause(1).unwrap();
    assert!(!paused.playing);
    assert_eq!(paused.rate, 0.0);
    render(&engine, 0.5);
    assert_eq!(engine.deck(1).unwrap().position_us, paused.position_us);
}

#[test]
fn output_is_the_track_at_unity_rate() {
    let engine = Engine::offline(RATE);
    deck_one_only(&engine);
    let source = noise(RATE, 2.0);
    engine
        .load(1, samples(RATE, source.clone(), None, vec![]))
        .unwrap();
    engine.play(1).unwrap();
    let out = render(&engine, 1.0);
    // Starts fade in from silence over FADE samples instead of clicking.
    assert_eq!(out[0], 0.0);
    assert!(
        (out[128] - source[128] * 0.5).abs() < 1e-6,
        "halfway through the fade"
    );
    for i in FADE as usize..RATE as usize {
        assert!((out[2 * i] - source[2 * i]).abs() < 1e-6, "sample {i}");
    }
}

#[test]
fn resamples_to_the_device_rate() {
    let engine = Engine::offline(RATE);
    deck_one_only(&engine);
    engine
        .load(
            1,
            samples(44_100, tones(44_100, 3.0, &[(1000.0, 0.5)]), None, vec![]),
        )
        .unwrap();
    engine.play(1).unwrap();
    let out = left(&render(&engine, 1.0));
    assert_near(engine.deck(1).unwrap().position_us, 1_000_000, 0.5, "1 s");
    let crossings = out
        .windows(2)
        .skip(FADE as usize)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    assert!(
        (1990..=2001).contains(&crossings),
        "{crossings} zero crossings"
    );
    assert!((amplitude(&out[4800..], 1000.0, RATE) - 0.5).abs() < 0.005);
}

#[test]
fn tempo_changes_rate_and_bpm() {
    let engine = Engine::offline(RATE);
    engine
        .load(
            1,
            samples(
                RATE,
                click_track(RATE, 120.0, 0.0, 30.0),
                Some(grid(0.0, 120.0, 60)),
                vec![],
            ),
        )
        .unwrap();
    let state = engine.set_tempo(1, 8.0).unwrap();
    assert_eq!(state.tempo_percent, 8.0);
    assert_eq!(state.tempo_range, 10.0);
    assert!((state.bpm.unwrap() - 129.6).abs() < 1e-9);
    assert_eq!(state.rate, 0.0, "paused");
    assert!((engine.play(1).unwrap().rate - 1.08).abs() < 1e-12);
    render(&engine, 1.0);
    assert_near(
        engine.deck(1).unwrap().position_us,
        1_080_000,
        0.5,
        "+8% for 1 s",
    );

    // Clamped to the range; narrowing the range clamps the tempo.
    assert_eq!(engine.set_tempo(1, 12.0).unwrap().tempo_percent, 10.0);
    assert_eq!(engine.set_tempo_range(1, 6.0).unwrap().tempo_percent, 6.0);
    assert_eq!(engine.set_tempo(1, -50.0).unwrap().tempo_percent, -6.0);
    engine.set_tempo_range(1, 100.0).unwrap();
    let wide = engine.set_tempo(1, -50.0).unwrap();
    assert_eq!(wide.tempo_percent, -50.0);
    assert_eq!(wide.rate, 0.5);
    assert!(matches!(
        engine.set_tempo_range(1, 7.0),
        Err(Error::InvalidInput(_))
    ));
}

fn click_deck(engine: &Engine, deck: u8, bpm: f64, first_beat: f64, cues: Vec<Cue>) {
    let seconds = 40.0;
    let beats = ((seconds - first_beat) * bpm / 60.0) as usize;
    engine
        .load(
            deck,
            samples(
                RATE,
                click_track(RATE, bpm, first_beat, seconds),
                Some(grid(first_beat, bpm, beats)),
                cues,
            ),
        )
        .unwrap();
}

#[test]
fn cue_button_follows_cdj_conventions() {
    let engine = Engine::offline(RATE);
    click_deck(&engine, 1, 120.0, 0.0, vec![memory_cue(1.0)]);

    // Paused away from the cue: CUE sets it, snapped to the nearest beat.
    engine.seek(1, 2_300_000).unwrap();
    let state = engine.cue(1, true).unwrap();
    assert_eq!(state.cue_us, 2_500_000);
    assert_eq!(state.position_us, 2_500_000);
    assert!(!state.playing);
    engine.cue(1, false).unwrap();

    // Paused at the cue: CUE plays while held, release returns and pauses.
    assert!(engine.cue(1, true).unwrap().playing);
    render(&engine, 0.25);
    assert_near(
        engine.deck(1).unwrap().position_us,
        2_750_000,
        0.5,
        "previewing",
    );
    let state = engine.cue(1, false).unwrap();
    assert!(!state.playing);
    assert_eq!(state.position_us, 2_500_000);

    // PLAY while holding CUE keeps playing after release.
    engine.cue(1, true).unwrap();
    engine.play(1).unwrap();
    render(&engine, 0.1);
    assert!(engine.cue(1, false).unwrap().playing);

    // CUE while playing: back to the cue point, paused.
    render(&engine, 0.4);
    let state = engine.cue(1, true).unwrap();
    assert!(!state.playing);
    assert_eq!(state.position_us, 2_500_000);
    engine.cue(1, false).unwrap();

    // Without quantize the cue lands exactly where the deck is.
    engine.set_quantize(false).unwrap();
    engine.seek(1, 3_300_000).unwrap();
    assert_eq!(engine.cue(1, true).unwrap().cue_us, 3_300_000);
}

#[test]
fn hot_cues_jump_set_and_preview() {
    let engine = Engine::offline(RATE);
    click_deck(
        &engine,
        1,
        120.0,
        0.0,
        vec![
            memory_cue(1.0),
            hot_cue(0, 4.0, None),
            hot_cue(1, 8.0, Some(10.0)),
        ],
    );
    // Playing, 0.2 s after a beat: quantized jump keeps that phase.
    engine.play(1).unwrap();
    render(&engine, 0.2);
    let state = engine.hot_cue(1, 0, true).unwrap();
    assert!(state.playing);
    assert_near(state.position_us, 4_200_000, 0.5, "phase-kept jump");
    assert!(
        engine.hot_cue(1, 0, false).unwrap().playing,
        "release while playing"
    );

    // An empty pad stores the (snapped) position.
    let state = engine.hot_cue(1, 2, true).unwrap();
    let c = state.hot_cues.iter().find(|c| c.slot == 2).unwrap();
    assert_eq!(c.start_us, 4_000_000);
    engine.hot_cue(1, 2, false).unwrap();

    // A hot loop starts looping.
    let state = engine.hot_cue(1, 1, true).unwrap();
    let lp = state.active_loop.unwrap();
    assert_eq!((lp.start_us, lp.end_us), (8_000_000, 10_000_000));
    engine.hot_cue(1, 1, false).unwrap();
    engine.loop_exit(1).unwrap();

    // Paused: plays while held, returns on release.
    engine.pause(1).unwrap();
    let state = engine.hot_cue(1, 0, true).unwrap();
    assert!(state.playing);
    assert_eq!(state.position_us, 4_000_000);
    render(&engine, 0.1);
    let state = engine.hot_cue(1, 0, false).unwrap();
    assert!(!state.playing);
    assert_eq!(state.position_us, 4_000_000);

    // Without quantize the jump is exact.
    engine.set_quantize(false).unwrap();
    engine.seek(1, 6_123_000).unwrap();
    engine.play(1).unwrap();
    assert_eq!(engine.hot_cue(1, 0, true).unwrap().position_us, 4_000_000);
    assert!(matches!(
        engine.hot_cue(1, 8, true),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn seek_and_beat_jump() {
    let engine = Engine::offline(RATE);
    click_deck(&engine, 1, 120.0, 0.0, vec![]);
    engine.play(1).unwrap();
    let state = engine.seek(1, 10_000_000).unwrap();
    assert!(state.playing);
    assert_eq!(state.position_us, 10_000_000);
    // Past the end clamps to the end.
    assert_eq!(engine.seek(1, 99_000_000).unwrap().position_us, 40_000_000);

    engine.pause(1).unwrap();
    engine.seek(1, 2_100_000).unwrap();
    assert_near(
        engine.beat_jump(1, 4.0).unwrap().position_us,
        4_100_000,
        0.5,
        "+4",
    );
    assert_near(
        engine.beat_jump(1, -8.0).unwrap().position_us,
        100_000,
        0.5,
        "-8",
    );
    assert_near(
        engine.beat_jump(1, 0.5).unwrap().position_us,
        350_000,
        0.5,
        "+1/2",
    );
    assert_eq!(engine.beat_jump(1, -8.0).unwrap().position_us, 0, "clamped");
}

#[test]
fn beat_jump_inside_a_loop_moves_the_loop() {
    let engine = Engine::offline(RATE);
    click_deck(&engine, 1, 120.0, 0.0, vec![]);
    engine.seek(1, 2_000_000).unwrap();
    engine.beat_loop(1, 4.0).unwrap();
    let state = engine.beat_jump(1, 8.0).unwrap();
    let lp = state.active_loop.unwrap();
    assert_eq!((lp.start_us, lp.end_us), (6_000_000, 8_000_000));
    assert_eq!(state.position_us, 6_000_000);
}

#[test]
fn loops_wrap_sample_accurately() {
    let engine = Engine::offline(RATE);
    deck_one_only(&engine);
    let source = noise(RATE, 20.0);
    engine
        .load(
            1,
            samples(RATE, source.clone(), Some(grid(0.0, 120.0, 40)), vec![]),
        )
        .unwrap();
    engine.seek(1, 2_200_000).unwrap();
    // Quantized: starts on the nearest beat (2.0 s); 4 beats = 2 s.
    let state = engine.beat_loop(1, 4.0).unwrap();
    let lp = state.active_loop.unwrap();
    assert_eq!((lp.start_us, lp.end_us), (2_000_000, 4_000_000));
    engine.play(1).unwrap();
    let out = left(&render(&engine, 6.0));
    // From 2.2 s, the deck hits the loop end after 1.8 s, then repeats every
    // 2 s (96 000 samples) exactly.
    let first_wrap = (1.8 * RATE as f64) as usize;
    let period = 2 * RATE as usize;
    for k in 0..period / 2 {
        let a = out[first_wrap + k];
        let b = out[first_wrap + period + k];
        assert!(
            (a - b).abs() < 1e-6,
            "sample {k} after the wrap: {a} vs {b}"
        );
    }
    // The wrap is on the exact sample: the loop end's last sample, then the
    // loop start's first.
    let at = |seconds: f64| source[2 * (seconds * RATE as f64).round() as usize];
    assert!((out[first_wrap - 1] - at(4.0 - 1.0 / RATE as f64)).abs() < 1e-6);
    assert!((out[first_wrap] - at(2.0)).abs() < 1e-6);
    assert!((out[first_wrap + 1] - at(2.0 + 1.0 / RATE as f64)).abs() < 1e-6);
    // 2.2 + 6.0 = 8.2 s of travel → 2.2 s into a 2 s loop from 2.0 → 2.2 s.
    assert_near(
        engine.deck(1).unwrap().position_us,
        2_200_000,
        0.5,
        "in the loop",
    );

    // A shorter loop wraps the read head into it, in phase.
    let state = engine.beat_loop(1, 0.25).unwrap();
    let lp = state.active_loop.unwrap();
    assert_eq!((lp.start_us, lp.end_us), (2_000_000, 2_125_000));
    assert_near(state.position_us, 2_075_000, 0.5, "wrapped into 1/4 beat");

    // Exit: playback continues through the old end.
    engine.loop_exit(1).unwrap();
    render(&engine, 1.0);
    let state = engine.deck(1).unwrap();
    assert!(state.active_loop.is_none());
    assert_near(state.position_us, 3_075_000, 0.5, "after exit");
}

#[test]
fn loops_keep_fractional_overshoot_at_any_rate() {
    // 44.1 kHz track on a 48 kHz engine at +3%: every wrap lands between
    // samples. Dropping the overshoot would drift ~20 µs per wrap.
    let engine = Engine::offline(RATE);
    engine
        .load(
            1,
            samples(
                44_100,
                click_track(44_100, 120.0, 0.0, 20.0),
                Some(grid(0.0, 120.0, 40)),
                vec![],
            ),
        )
        .unwrap();
    engine.set_tempo(1, 3.0).unwrap();
    engine.seek(1, 2_000_000).unwrap();
    engine.beat_loop(1, 1.0).unwrap();
    engine.play(1).unwrap();
    let seconds = 7.0;
    render(&engine, seconds);
    let travelled = seconds * 1.03;
    let expected = 2.0 + travelled % 0.5;
    assert_near(
        engine.deck(1).unwrap().position_us,
        us(expected),
        1.0,
        "after 14 wraps",
    );
}

#[test]
fn track_end_stops_the_deck() {
    let engine = Engine::offline(RATE);
    engine
        .load(1, samples(RATE, noise(RATE, 2.0), None, vec![]))
        .unwrap();
    engine.seek(1, 1_900_000).unwrap();
    engine.play(1).unwrap();
    render(&engine, 0.5);
    let state = engine.deck(1).unwrap();
    assert!(!state.playing);
    assert_eq!(state.rate, 0.0);
    assert_eq!(state.position_us, 2_000_000);
    // PLAY at the end does nothing until the deck moves.
    assert!(!engine.play(1).unwrap().playing);
}

#[test]
fn commands_check_their_deck() {
    let engine = Engine::offline(RATE);
    assert!(matches!(engine.play(3), Err(Error::InvalidDeck(3))));
    assert!(matches!(engine.play(1), Err(Error::NoTrack(1))));
    engine
        .load(1, samples(RATE, noise(RATE, 1.0), None, vec![]))
        .unwrap();
    assert!(matches!(
        engine.beat_jump(1, 4.0),
        Err(Error::NoBeatGrid(1))
    ));
    assert!(matches!(
        engine.beat_loop(1, 4.0),
        Err(Error::NoBeatGrid(1))
    ));
    assert!(matches!(engine.sync(1, true), Err(Error::NoBeatGrid(1))));
    assert!(matches!(
        engine.beat_loop(1, 0.0),
        Err(Error::NoBeatGrid(1))
    ));
    // A library BPM without a grid is enough for beat-based controls.
    let mut request = samples(RATE, noise(RATE, 4.0), None, vec![]);
    request.bpm = Some(120.0);
    engine.load(2, request).unwrap();
    assert_near(
        engine.beat_jump(2, 2.0).unwrap().position_us,
        1_000_000,
        0.5,
        "1 s",
    );
}

// Sync -------------------------------------------------------------------------

/// Seconds from each deck's playhead to its next beat, in real time.
fn time_to_next_beat(position_us: i64, first_beat: f64, bpm: f64, rate: f64) -> f64 {
    let period = 60.0 / bpm;
    let beat = (position_us as f64 / 1e6 - first_beat) / period;
    (beat.ceil() - beat) * period / rate
}

fn phase_error_ms(engine: &Engine, a: (f64, f64), b: (f64, f64)) -> f64 {
    let s1 = engine.deck(1).unwrap();
    let s2 = engine.deck(2).unwrap();
    assert_eq!(s1.at_ms, s2.at_ms);
    let t1 = time_to_next_beat(s1.position_us, a.0, a.1, s1.rate);
    let t2 = time_to_next_beat(s2.position_us, b.0, b.1, s2.rate);
    let period = 60.0 / s1.bpm.unwrap();
    // Distance on the beat circle.
    let d = (t1 - t2).rem_euclid(period);
    d.min(period - d) * 1000.0
}

#[test]
fn sync_matches_tempo_and_phase() {
    let engine = Engine::offline(RATE);
    click_deck(&engine, 1, 120.0, 0.0, vec![]);
    click_deck(&engine, 2, 126.0, 0.1, vec![memory_cue(0.1)]);
    engine.play(1).unwrap();
    render(&engine, 1.3);
    assert!(
        engine.deck(1).unwrap().master,
        "first deck playing is master"
    );

    let state = engine.sync(2, true).unwrap();
    assert!(state.sync);
    assert!(!state.master);
    assert!((state.bpm.unwrap() - 120.0).abs() < 1e-9);
    assert!((state.tempo_percent - (120.0 / 126.0 - 1.0) * 100.0).abs() < 1e-9);
    engine.set_tempo(2, 9.0).unwrap();
    assert!(
        (engine.deck(2).unwrap().bpm.unwrap() - 120.0).abs() < 1e-9,
        "locked"
    );

    engine.play(2).unwrap();
    render(&engine, 2.0);
    let err = phase_error_ms(&engine, (0.0, 120.0), (0.1, 126.0));
    assert!(err < 1.0, "phase error {err} ms");

    // The slave follows the master's tempo changes and stays in phase.
    engine.set_tempo(1, 4.0).unwrap();
    render(&engine, 5.0);
    let s2 = engine.deck(2).unwrap();
    assert!((s2.bpm.unwrap() - 124.8).abs() < 1e-9);
    let err = phase_error_ms(&engine, (0.0, 120.0), (0.1, 126.0));
    assert!(err < 1.0, "phase error after tempo change {err} ms");

    // An unquantized jump on the master: the slave jumps back into phase.
    engine.set_quantize(false).unwrap();
    engine.seek(1, 20_123_000).unwrap();
    render(&engine, 0.5);
    let err = phase_error_ms(&engine, (0.0, 120.0), (0.1, 126.0));
    assert!(err < 1.0, "phase error after master jump {err} ms");

    // The master stops: the playing deck takes over and keeps its tempo.
    engine.pause(1).unwrap();
    let s2 = engine.deck(2).unwrap();
    assert!(s2.master);
    assert!((s2.bpm.unwrap() - 124.8).abs() < 1e-9);
    // Sync off keeps the tempo it had.
    let s2 = engine.sync(2, false).unwrap();
    assert!(!s2.sync);
    assert!((s2.bpm.unwrap() - 124.8).abs() < 1e-9);

    assert!(engine.set_master(1).unwrap().master);
    assert!(!engine.deck(2).unwrap().master);
}

#[test]
fn sync_matches_half_and_double_time() {
    let engine = Engine::offline(RATE);
    click_deck(&engine, 1, 120.0, 0.0, vec![]);
    click_deck(&engine, 2, 63.0, 0.0, vec![]);
    engine.play(1).unwrap();
    let state = engine.sync(2, true).unwrap();
    assert!((state.bpm.unwrap() - 60.0).abs() < 1e-9);
    engine.play(2).unwrap();
    render(&engine, 3.0);
    let s1 = engine.deck(1).unwrap();
    let s2 = engine.deck(2).unwrap();
    // One beat of deck 2 is two of deck 1: deck 2's beats land on deck 1's.
    let t1 = time_to_next_beat(s1.position_us, 0.0, 120.0, s1.rate);
    let t2 = time_to_next_beat(s2.position_us, 0.0, 63.0, s2.rate);
    let d = (t2 - t1).rem_euclid(0.5);
    assert!(d.min(0.5 - d) < 0.001, "{t1} vs {t2}");
}

#[test]
fn sync_with_no_master_makes_the_deck_master() {
    let engine = Engine::offline(RATE);
    click_deck(&engine, 1, 120.0, 0.0, vec![]);
    click_deck(&engine, 2, 125.0, 0.0, vec![]);
    let s1 = engine.sync(1, true).unwrap();
    assert!(s1.master);
    let s2 = engine.sync(2, true).unwrap();
    assert!(!s2.master);
    assert!(
        (s2.bpm.unwrap() - 120.0).abs() < 1e-9,
        "follows the paused master"
    );
}

// Mixer ------------------------------------------------------------------------

fn two_tones(engine: &Engine) {
    engine
        .load(
            1,
            samples(RATE, tones(RATE, 10.0, &[(440.0, 0.5)]), None, vec![]),
        )
        .unwrap();
    engine
        .load(
            2,
            samples(RATE, tones(RATE, 10.0, &[(1000.0, 0.5)]), None, vec![]),
        )
        .unwrap();
    engine.play(1).unwrap();
    engine.play(2).unwrap();
    render(engine, 0.2);
}

fn levels(engine: &Engine) -> (f64, f64) {
    let out = left(&render(engine, 0.5));
    let settled = &out[RATE as usize / 10..];
    (
        amplitude(settled, 440.0, RATE),
        amplitude(settled, 1000.0, RATE),
    )
}

#[test]
fn crossfader_and_faders_set_gains() {
    let engine = Engine::offline(RATE);
    two_tones(&engine);
    let mixer = |update: MixerSettingsUpdate| engine.set_mixer(update).unwrap();
    let h = std::f64::consts::FRAC_1_SQRT_2;

    let (a, b) = levels(&engine);
    assert!(
        (a - 0.5 * h).abs() < 1e-3 && (b - 0.5 * h).abs() < 1e-3,
        "centre: {a} {b}"
    );

    mixer(MixerSettingsUpdate {
        crossfader: Some(-1.0),
        ..Default::default()
    });
    let (a, b) = levels(&engine);
    assert!((a - 0.5).abs() < 1e-3 && b < 1e-4, "left: {a} {b}");

    let state = mixer(MixerSettingsUpdate {
        crossfader: Some(1.0),
        ..Default::default()
    });
    assert_eq!(state.crossfader, 1.0);
    let (a, b) = levels(&engine);
    assert!(a < 1e-4 && (b - 0.5).abs() < 1e-3, "right: {a} {b}");

    let state = mixer(MixerSettingsUpdate {
        deck: Some(2),
        volume: Some(0.5),
        ..Default::default()
    });
    assert_eq!(state.channels[1].volume, 0.5);
    let (_, b) = levels(&engine);
    assert!((b - 0.25).abs() < 1e-3, "channel fader: {b}");

    mixer(MixerSettingsUpdate {
        master_volume: Some(0.5),
        ..Default::default()
    });
    let (_, b) = levels(&engine);
    assert!((b - 0.125).abs() < 1e-3, "master: {b}");

    // Out-of-range values clamp; per-deck settings need a deck.
    assert_eq!(
        mixer(MixerSettingsUpdate {
            crossfader: Some(-7.0),
            ..Default::default()
        })
        .crossfader,
        -1.0
    );
    assert!(matches!(
        engine.set_mixer(MixerSettingsUpdate {
            volume: Some(0.5),
            ..Default::default()
        }),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn eq_bands_boost_cut_and_kill() {
    let engine = Engine::offline(RATE);
    deck_one_only(&engine);
    let source = tones(RATE, 20.0, &[(60.0, 0.2), (1000.0, 0.2), (10_000.0, 0.2)]);
    engine
        .load(1, samples(RATE, source.clone(), None, vec![]))
        .unwrap();
    engine.play(1).unwrap();
    let band_levels = |engine: &Engine| {
        render(engine, 0.3);
        let out = left(&render(engine, 0.5));
        [60.0, 1000.0, 10_000.0].map(|f| amplitude(&out, f, RATE))
    };
    let eq = |low: f32, mid: f32, high: f32| {
        engine
            .set_mixer(MixerSettingsUpdate {
                deck: Some(1),
                eq_low: Some(low),
                eq_mid: Some(mid),
                eq_high: Some(high),
                ..Default::default()
            })
            .unwrap()
    };

    // Flat: bit-exact bypass.
    render(&engine, 0.1);
    let start = (engine.deck(1).unwrap().position_us as f64 / SAMPLE_US).round() as usize;
    let out = render(&engine, 0.2);
    for i in 0..1000 {
        assert!(
            (out[2 * i] - source[2 * (start + i)]).abs() < 1e-6,
            "sample {i}"
        );
    }

    let kill = crate::EQ_KILL_DB;
    let state = eq(kill, 0.0, 0.0);
    assert_eq!(state.channels[0].eq_low, kill);
    let [low, mid, high] = band_levels(&engine);
    assert!(db(low / 0.2) < -40.0, "low kill: {} dB", db(low / 0.2));
    assert!(
        db(mid / 0.2).abs() < 1.0,
        "mid with low kill: {} dB",
        db(mid / 0.2)
    );
    assert!(
        db(high / 0.2).abs() < 1.0,
        "high with low kill: {} dB",
        db(high / 0.2)
    );

    eq(0.0, kill, 0.0);
    let [low, mid, high] = band_levels(&engine);
    assert!(db(mid / 0.2) < -24.0, "mid kill: {} dB", db(mid / 0.2));
    assert!(db(low / 0.2).abs() < 1.0 && db(high / 0.2).abs() < 1.0);

    eq(0.0, 0.0, kill);
    let [low, mid, high] = band_levels(&engine);
    assert!(db(high / 0.2) < -40.0, "high kill: {} dB", db(high / 0.2));
    assert!(db(low / 0.2).abs() < 1.0 && db(mid / 0.2).abs() < 1.0);

    eq(0.0, 6.0, -12.0);
    let [low, mid, high] = band_levels(&engine);
    assert!(
        (db(mid / 0.2) - 6.0).abs() < 0.5,
        "mid +6: {} dB",
        db(mid / 0.2)
    );
    assert!(
        (db(high / 0.2) + 12.0).abs() < 1.0,
        "high -12: {} dB",
        db(high / 0.2)
    );
    assert!(db(low / 0.2).abs() < 1.0);

    // Clamped to −80 (kill) … +6.
    let state = eq(-200.0, 20.0, 0.0);
    assert_eq!(state.channels[0].eq_low, kill);
    assert_eq!(state.channels[0].eq_mid, 6.0);
}

// Events -----------------------------------------------------------------------

fn deck_events(events: &[EngineEvent], deck: u8) -> Vec<&crate::DeckEvent> {
    events
        .iter()
        .filter_map(|e| match e {
            EngineEvent::Deck(d) if d.state.deck == deck => Some(d),
            _ => None,
        })
        .collect()
}

#[test]
fn events_on_change_and_while_playing() {
    let engine = Engine::offline(RATE);
    let first = engine.poll_events();
    assert_eq!(deck_events(&first, 1).len(), 1);
    assert_eq!(deck_events(&first, 2).len(), 1);
    assert!(first.iter().any(|e| matches!(e, EngineEvent::Mixer(_))));
    assert!(engine.poll_events().is_empty(), "nothing changed");

    click_deck(&engine, 1, 120.0, 0.0, vec![]);
    engine.play(1).unwrap();
    let events = engine.poll_events();
    let ev = deck_events(&events, 1);
    assert_eq!(ev.len(), 1);
    assert!(ev[0].state.playing);
    assert_eq!(ev[0].sent_at_ms, engine.now_ms());
    assert!(deck_events(&events, 2).is_empty());

    // About 20 clock updates a second while playing.
    let mut updates = 0;
    for _ in 0..100 {
        render(&engine, 0.01);
        updates += deck_events(&engine.poll_events(), 1).len();
    }
    assert!((19..=21).contains(&updates), "{updates} updates in 1 s");

    // Loop wraps are discontinuities: each one is reported.
    engine.beat_loop(1, 1.0).unwrap();
    engine.poll_events();
    let mut wraps = 0;
    for _ in 0..20 {
        render(&engine, 0.1);
        wraps += deck_events(&engine.poll_events(), 1).len();
    }
    assert!(wraps >= 4, "{wraps} events over 4 loop wraps");

    engine.pause(1).unwrap();
    assert_eq!(deck_events(&engine.poll_events(), 1).len(), 1);
    render(&engine, 0.5);
    assert!(engine.poll_events().is_empty(), "paused decks are quiet");

    engine
        .set_mixer(MixerSettingsUpdate {
            crossfader: Some(0.5),
            ..Default::default()
        })
        .unwrap();
    let events = engine.poll_events();
    assert!(matches!(events.as_slice(), [EngineEvent::Mixer(m)] if m.crossfader == 0.5));
}

#[test]
fn event_handler_receives_events() {
    let engine = Engine::offline(RATE);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = seen.clone();
    engine.on_event(move |e| sink.lock().unwrap().push(e.clone()));
    engine.poll_events();
    assert_eq!(seen.lock().unwrap().len(), 3);
    let json = serde_json::to_value(&seen.lock().unwrap()[0]).unwrap();
    assert_eq!(json["kind"], "deck");
    assert!(json["sent_at_ms"].is_number());
    assert!(json["position_us"].is_number());
}

// Real-time safety -------------------------------------------------------------

#[test]
fn render_path_does_not_allocate() {
    let engine = Engine::offline(RATE);
    click_deck(&engine, 1, 120.0, 0.0, vec![hot_cue(0, 4.0, Some(6.0))]);
    click_deck(&engine, 2, 126.0, 0.1, vec![]);
    // The replacement track for the load below, built off the audio thread.
    let replacement = {
        let request = samples(
            RATE,
            click_track(RATE, 124.0, 0.0, 20.0),
            Some(grid(0.0, 124.0, 40)),
            vec![],
        );
        let Source::Samples {
            sample_rate,
            channels,
            samples,
        } = request.source
        else {
            unreachable!()
        };
        crate::deck::LoadedTrack {
            load_id: 99,
            pcm: std::sync::Arc::new(crate::pcm::TrackPcm::from_samples(
                sample_rate,
                channels,
                &samples,
            )),
            grid: crate::grid::Grid::from_beats(&grid(0.0, 124.0, 40).beats).map(Box::new),
            hot_cues: [None; crate::deck::HOT_CUES],
            cue: 0.0,
        }
    };
    let controls = vec![
        Control::Play(0),
        Control::Sync { deck: 1, on: true },
        Control::Play(1),
        Control::Mixer(crate::core::MixerUpdate {
            deck: Some(0),
            eq: [Some(crate::EQ_KILL_DB), Some(3.0), None],
            crossfader: Some(0.3),
            ..Default::default()
        }),
        Control::BeatLoop {
            deck: 0,
            beats: 2.0,
        },
        Control::HotCue {
            deck: 0,
            slot: 0,
            press: true,
        },
        Control::SetTempo {
            deck: 0,
            percent: 5.0,
        },
        Control::BeatJump {
            deck: 1,
            beats: 3.5,
        },
        Control::Seek {
            deck: 1,
            seconds: 12.34,
        },
        Control::Load {
            deck: 1,
            track: replacement,
        },
        Control::Play(1),
    ];
    let shared = engine.shared();
    let mut out = vec![0.0f32; 512 * 2];
    let mut core = lock(&shared.core);
    let mut controls = controls.into_iter();
    // Interleave queued commands with rendering, as the audio thread sees them.
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    let mut sound = false;
    for block in 0..400 {
        if block % 20 == 0
            && let Some(control) = controls.next()
        {
            let mut queue = lock(&shared.commands);
            queue.0 += 1;
            let seq = queue.0;
            assert!(queue.1.push((seq, control)).is_ok());
        }
        COUNTING.with(|c| c.set(true));
        let at = core.rendered_ms();
        core.render_interleaved(&mut out, at);
        COUNTING.with(|c| c.set(false));
        sound |= out.iter().any(|&s| s != 0.0);
    }
    let allocations = ALLOCATIONS.load(Ordering::Relaxed) - before;
    assert_eq!(allocations, 0, "the render path allocated or freed");
    assert!(sound, "it rendered audio");
    drop(core);
    let state = engine.deck(1).unwrap();
    assert!(state.playing && state.active_loop.is_some());
}

// Real device -------------------------------------------------------------------

/// Opens the default output with nothing loaded (silence), checks the clock
/// and a command round trip, and closes it. Run by hand:
/// `cargo test -p tuneox-audio -- --ignored`.
#[test]
#[ignore = "opens the real audio device (silently)"]
fn opens_the_default_device_silently() {
    let engine = Engine::start().unwrap();
    let output = engine.output().unwrap();
    eprintln!("output: {output:?}");
    let before = engine.deck(1).unwrap().at_ms;
    std::thread::sleep(Duration::from_millis(200));
    let after = engine.deck(1).unwrap().at_ms;
    assert!(after > before + 100.0, "clock moves: {before} → {after}");
    let started = std::time::Instant::now();
    let status = engine.set_quantize(false).unwrap();
    eprintln!(
        "command round trip: {:.2} ms, latency {:.2} ms",
        started.elapsed().as_secs_f64() * 1000.0,
        engine.output().unwrap().latency_ms
    );
    assert!(status.running && !status.quantize);
    drop(engine);
}
