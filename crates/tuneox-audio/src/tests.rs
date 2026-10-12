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
use crate::{
    BendDirection, Engine, EngineEvent, Error, LoadRequest, MemoryCueTarget, MixerSettingsUpdate,
    Source, SyncMode,
};

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
        auto_gain_db: None,
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
    // Parked at the start with no cue (rekordbox EXPORT mode).
    assert_eq!(state.cue_us, None);
    assert_eq!(state.position_us, 0);
    assert_eq!(state.original_bpm, Some(120.0));
    assert_eq!(state.bpm, Some(120.0));
    assert_eq!(state.sync_mode, SyncMode::Off);
    assert!(state.quantize);
    assert_eq!(state.hot_cues.len(), 1);
    assert_eq!(state.hot_cues[0].slot, 2);
    assert_eq!(state.hot_cues[0].start_us, 4_000_000);
    // Memory cues in position order.
    let starts: Vec<i64> = state.memory_cues.iter().map(|c| c.start_us).collect();
    assert_eq!(starts, [1_000_000, 2_000_000]);
    assert_eq!(state.memory_cues[1].index, 1);
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
        auto_gain_db: None,
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
fn cue_button_follows_rekordbox() {
    let engine = Engine::offline(RATE);
    click_deck(&engine, 1, 120.0, 0.0, vec![]);

    // No cue yet: CUE while playing goes back to the start.
    engine.play(1).unwrap();
    render(&engine, 0.3);
    let state = engine.cue(1, true).unwrap();
    assert!(!state.playing);
    assert_eq!(state.position_us, 0);
    assert_eq!(state.cue_us, None);
    engine.cue(1, false).unwrap();

    // Paused away from the cue: CUE sets it, snapped to the nearest beat.
    engine.seek(1, 2_300_000).unwrap();
    let state = engine.cue(1, true).unwrap();
    assert_eq!(state.cue_us, Some(2_500_000));
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
    assert!(!engine.set_quantize(1, false).unwrap().quantize);
    engine.seek(1, 3_300_000).unwrap();
    assert_eq!(engine.cue(1, true).unwrap().cue_us, Some(3_300_000));
}

#[test]
fn hot_cues_jump_and_play() {
    let engine = Engine::offline(RATE);
    click_deck(
        &engine,
        1,
        120.0,
        0.0,
        vec![hot_cue(0, 4.0, None), hot_cue(1, 8.0, Some(10.0))],
    );
    // Paused (rekordbox's default): jump and play; release changes nothing.
    let state = engine.hot_cue(1, 0, true).unwrap();
    assert!(state.playing);
    assert_eq!(state.position_us, 4_000_000);
    assert!(engine.hot_cue(1, 0, false).unwrap().playing);

    // Playing, 0.2 s after a beat: quantized jump keeps that phase.
    engine.seek(1, 1_200_000).unwrap();
    let state = engine.hot_cue(1, 0, true).unwrap();
    assert!(state.playing);
    assert_near(state.position_us, 4_200_000, 0.5, "phase-kept jump");
    engine.hot_cue(1, 0, false).unwrap();

    // An empty pad stores the (snapped) position.
    let state = engine.hot_cue(1, 2, true).unwrap();
    let c = state.hot_cues.iter().find(|c| c.slot == 2).unwrap();
    assert_eq!((c.start_us, c.end_us), (4_000_000, None));
    engine.hot_cue(1, 2, false).unwrap();

    // A hot loop starts looping.
    let state = engine.hot_cue(1, 1, true).unwrap();
    let lp = state.active_loop.unwrap();
    assert_eq!((lp.start_us, lp.end_us), (8_000_000, 10_000_000));
    engine.hot_cue(1, 1, false).unwrap();
    // An empty pad during a loop stores the loop as a hot loop.
    let state = engine.hot_cue(1, 3, true).unwrap();
    let c = state.hot_cues.iter().find(|c| c.slot == 3).unwrap();
    assert_eq!((c.start_us, c.end_us), (8_000_000, Some(10_000_000)));
    engine.hot_cue(1, 3, false).unwrap();
    engine.loop_exit(1).unwrap();
    // Just before a beat, the phase-kept landing point would be before the
    // loop: it lands at the same phase inside the loop instead.
    engine.seek(1, 4_400_000).unwrap();
    let state = engine.hot_cue(1, 1, true).unwrap();
    assert!(state.active_loop.is_some());
    assert_near(
        state.position_us,
        9_900_000,
        0.5,
        "phase-kept, inside the loop",
    );
    engine.hot_cue(1, 1, false).unwrap();
    engine.loop_exit(1).unwrap();

    // Gate mode: from pause, a hot cue plays only while held.
    assert!(engine.set_hot_cue_gate(true).unwrap().hot_cue_gate);
    engine.pause(1).unwrap();
    let state = engine.hot_cue(1, 0, true).unwrap();
    assert!(state.playing);
    assert_eq!(state.position_us, 4_000_000);
    render(&engine, 0.1);
    let state = engine.hot_cue(1, 0, false).unwrap();
    assert!(!state.playing);
    assert_eq!(state.position_us, 4_000_000);
    // …except for hot loops, which latch.
    engine.hot_cue(1, 1, true).unwrap();
    assert!(engine.hot_cue(1, 1, false).unwrap().playing);
    engine.set_hot_cue_gate(false).unwrap();

    // Without quantize the jump is exact.
    engine.set_quantize(1, false).unwrap();
    engine.seek(1, 6_123_000).unwrap();
    assert_eq!(engine.hot_cue(1, 0, true).unwrap().position_us, 4_000_000);
    assert!(matches!(
        engine.hot_cue(1, 8, true),
        Err(Error::InvalidInput(_))
    ));
}

fn memory_loop(start: f64, end: f64) -> Cue {
    Cue {
        hot_cue: None,
        ..hot_cue(0, start, Some(end))
    }
}

#[test]
fn memory_cues_jump_and_pause() {
    let engine = Engine::offline(RATE);
    click_deck(
        &engine,
        1,
        120.0,
        0.0,
        vec![memory_cue(6.0), memory_cue(2.0), memory_loop(10.0, 12.0)],
    );
    engine.play(1).unwrap();
    render(&engine, 0.5);
    // By index (position order): jump, pause, and it's the cue point.
    let state = engine.memory_cue(1, MemoryCueTarget::Index(1)).unwrap();
    assert!(!state.playing);
    assert_eq!(state.position_us, 6_000_000);
    assert_eq!(state.cue_us, Some(6_000_000));
    // Next: a memory loop is armed; PLAY loops it.
    let state = engine.memory_cue(1, MemoryCueTarget::Next).unwrap();
    assert_eq!(state.position_us, 10_000_000);
    assert!(!state.playing);
    let lp = state.active_loop.unwrap();
    assert_eq!((lp.start_us, lp.end_us), (10_000_000, 12_000_000));
    engine.play(1).unwrap();
    render(&engine, 2.5);
    assert_near(
        engine.deck(1).unwrap().position_us,
        10_500_000,
        0.5,
        "looping",
    );
    // Previous: the one before the playhead.
    let state = engine.memory_cue(1, MemoryCueTarget::Previous).unwrap();
    assert_eq!(state.position_us, 10_000_000);
    let state = engine.memory_cue(1, MemoryCueTarget::Previous).unwrap();
    assert_eq!(state.position_us, 6_000_000);
    assert!(state.active_loop.is_none(), "left the loop");
    assert!(matches!(
        engine.memory_cue(1, MemoryCueTarget::Index(3)),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn manual_loops_reloop_and_resize() {
    let engine = Engine::offline(RATE);
    click_deck(&engine, 1, 120.0, 0.0, vec![]);
    engine.seek(1, 2_200_000).unwrap();
    engine.play(1).unwrap();
    // IN snaps to the beat and sets the cue (real-time cue).
    let state = engine.loop_in(1).unwrap();
    assert_eq!(state.loop_in_us, Some(2_000_000));
    assert_eq!(state.cue_us, Some(2_000_000));
    render(&engine, 1.0);
    // OUT at 3.2 s snaps to 3.0 s: the deck is past it, so it wraps in.
    let state = engine.loop_out(1).unwrap();
    let lp = state.active_loop.unwrap();
    assert_eq!(
        (lp.start_us, lp.end_us, lp.beats),
        (2_000_000, 3_000_000, Some(2.0))
    );
    assert_eq!(state.loop_in_us, None);
    assert_near(state.position_us, 2_200_000, 0.5, "wrapped into the loop");
    render(&engine, 1.0);
    assert_near(
        engine.deck(1).unwrap().position_us,
        2_200_000,
        0.5,
        "looping",
    );

    // Resize anchored at loop-in, within 1/64–512 beats.
    let lp = engine.loop_resize(1, true).unwrap().active_loop.unwrap();
    assert_eq!((lp.start_us, lp.end_us), (2_000_000, 4_000_000));
    engine.loop_resize(1, false).unwrap();
    let lp = engine.loop_resize(1, false).unwrap().active_loop.unwrap();
    assert_eq!((lp.end_us, lp.beats), (2_500_000, Some(1.0)));
    for _ in 0..10 {
        engine.loop_resize(1, false).unwrap();
    }
    let lp = engine.deck(1).unwrap().active_loop.unwrap();
    assert_eq!(lp.beats, Some(1.0 / 64.0));

    // Exit runs on past loop-out; RELOOP goes back and loops again.
    let state = engine.loop_exit(1).unwrap();
    assert!(state.active_loop.is_none());
    assert_eq!(state.last_loop.unwrap().start_us, 2_000_000);
    render(&engine, 1.0);
    assert!(engine.deck(1).unwrap().position_us > 3_000_000);
    let state = engine.reloop(1).unwrap();
    assert_eq!(state.position_us, 2_000_000);
    assert!(state.active_loop.is_some());
    engine.loop_exit(1).unwrap();

    // Auto beat loop: the same size again exits.
    engine.seek(1, 5_100_000).unwrap();
    let lp = engine.beat_loop(1, 4.0).unwrap().active_loop.unwrap();
    assert_eq!(
        (lp.start_us, lp.end_us, lp.beats),
        (5_000_000, 7_000_000, Some(4.0))
    );
    assert!(engine.beat_loop(1, 4.0).unwrap().active_loop.is_none());
    engine.beat_loop(1, 256.0).unwrap();
    engine.loop_resize(1, true).unwrap();
    let lp = engine.loop_resize(1, true).unwrap().active_loop.unwrap();
    assert_eq!(lp.beats, Some(512.0));
    assert!(matches!(
        engine.beat_loop(1, 1024.0),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn quantize_is_per_deck() {
    let engine = Engine::offline(RATE);
    click_deck(&engine, 1, 120.0, 0.0, vec![]);
    click_deck(&engine, 2, 120.0, 0.0, vec![]);
    engine.set_quantize(1, false).unwrap();
    assert!(engine.deck(2).unwrap().quantize);
    for deck in [1, 2] {
        engine.seek(deck, 2_300_000).unwrap();
    }
    assert_eq!(engine.cue(1, true).unwrap().cue_us, Some(2_300_000));
    assert_eq!(engine.cue(2, true).unwrap().cue_us, Some(2_500_000));
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
    assert!(matches!(
        engine.sync(1, SyncMode::Beat),
        Err(Error::NoBeatGrid(1))
    ));
    assert!(matches!(
        engine.beat_loop(1, 0.0),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        engine.set_bpm(1, 120.0),
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

    let state = engine.sync(2, SyncMode::Beat).unwrap();
    assert_eq!(state.sync_mode, SyncMode::Beat);
    assert!(!state.master);
    assert!((state.bpm.unwrap() - 120.0).abs() < 1e-9);
    assert!((state.tempo_percent - (120.0 / 126.0 - 1.0) * 100.0).abs() < 1e-9);
    // A synced deck's tempo belongs to the master.
    for result in [
        engine.set_tempo(2, 9.0),
        engine.set_bpm(2, 125.0),
        engine.tempo_reset(2),
    ] {
        assert!(matches!(
            result,
            Err(Error::FollowsMaster { deck: 2, master: 1 })
        ));
    }

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

    // An off-beat jump on the master: the slave jumps back into phase.
    engine.seek(1, 20_123_000).unwrap();
    render(&engine, 0.5);
    let err = phase_error_ms(&engine, (0.0, 120.0), (0.1, 126.0));
    assert!(err < 1.0, "phase error after master jump {err} ms");

    // Pausing the master doesn't hand master over.
    engine.pause(1).unwrap();
    assert!(engine.deck(1).unwrap().master);
    // Replacing the master's track does; the new master keeps its tempo.
    click_deck(&engine, 1, 120.0, 0.0, vec![]);
    let s2 = engine.deck(2).unwrap();
    assert!(s2.master);
    assert!(!engine.deck(1).unwrap().master);
    assert!((s2.bpm.unwrap() - 124.8).abs() < 1e-9);
    // Sync off keeps the tempo it had.
    let s2 = engine.sync(2, SyncMode::Off).unwrap();
    assert_eq!(s2.sync_mode, SyncMode::Off);
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
    let state = engine.sync(2, SyncMode::Beat).unwrap();
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
    let s1 = engine.sync(1, SyncMode::Beat).unwrap();
    assert!(s1.master);
    let s2 = engine.sync(2, SyncMode::Beat).unwrap();
    assert!(!s2.master);
    assert!(
        (s2.bpm.unwrap() - 120.0).abs() < 1e-9,
        "follows the paused master"
    );
}

#[test]
fn sync_holds_on_millisecond_rounded_grids() {
    // rekordbox stores beat times in whole milliseconds, so a 126 BPM grid
    // alternates 476 and 477 ms beats.
    let ms_grid = |bpm: f64, first: f64, beats: usize| BeatGrid {
        beats: (0..beats)
            .map(|i| Beat {
                time_us: ((first + i as f64 * 60.0 / bpm) * 1000.0).round() as i64 * 1000,
                bar_position: (i % 4) as u8 + 1,
                bpm_centi: (bpm * 100.0).round() as u16,
            })
            .collect(),
    };
    let engine = Engine::offline(RATE);
    for (deck, bpm, first) in [(1, 128.0, 0.0), (2, 126.0, 0.2)] {
        engine
            .load(
                deck,
                samples(
                    RATE,
                    click_track(RATE, bpm, first, 40.0),
                    Some(ms_grid(bpm, first, 80)),
                    vec![],
                ),
            )
            .unwrap();
    }
    engine.play(1).unwrap();
    engine.sync(2, SyncMode::Beat).unwrap();
    engine.play(2).unwrap();
    let mut worst: f64 = 0.0;
    for _ in 0..30 {
        render(&engine, 1.0);
        worst = worst.max(phase_error_ms(&engine, (0.0, 128.0), (0.2, 126.0)));
    }
    assert!(worst < 1.0, "worst phase error {worst} ms");
}

#[test]
fn sync_and_loops() {
    let engine = Engine::offline(RATE);
    click_deck(&engine, 1, 120.0, 0.0, vec![]);
    click_deck(&engine, 2, 126.0, 0.1, vec![]);
    engine.play(1).unwrap();
    engine.sync(2, SyncMode::Beat).unwrap();
    engine.play(2).unwrap();
    render(&engine, 1.0);

    // A whole-beat loop on the synced deck stays locked across wraps.
    engine.beat_loop(2, 4.0).unwrap();
    render(&engine, 6.0);
    assert!(
        engine.deck(2).unwrap().active_loop.is_some(),
        "still looping"
    );
    let err = phase_error_ms(&engine, (0.0, 120.0), (0.1, 126.0));
    assert!(err < 1.0, "phase error in a loop {err} ms");
    engine.loop_exit(2).unwrap();

    // A ½-beat loop on the master restarts its phase every wrap; the synced
    // deck keeps the tempo and doesn't chase it.
    engine.beat_loop(1, 0.5).unwrap();
    render(&engine, 0.1);
    let before = engine.deck(2).unwrap();
    render(&engine, 1.0);
    let after = engine.deck(2).unwrap();
    let travelled = (after.position_us - before.position_us) as f64;
    assert!(
        (travelled - 1e6 * before.rate).abs() < 1.0,
        "no jumps: {travelled} µs"
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

// Tempo controls ---------------------------------------------------------------

#[test]
fn bpm_entry_reset_and_limits() {
    let engine = Engine::offline(RATE);
    click_deck(&engine, 1, 120.0, 0.0, vec![]);
    let state = engine.set_bpm(1, 126.0).unwrap();
    assert!((state.tempo_percent - 5.0).abs() < 1e-9);
    assert!((state.bpm.unwrap() - 126.0).abs() < 1e-9);
    // Outside the ±10 % range or 40–499 BPM: refused with a reason.
    assert!(matches!(
        engine.set_bpm(1, 140.0),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        engine.set_bpm(1, 20.0),
        Err(Error::InvalidInput(_))
    ));
    let state = engine.tempo_reset(1).unwrap();
    assert_eq!(state.tempo_percent, 0.0);
    assert_eq!(state.bpm, Some(120.0));
    // WIDE allows −100 %, but the effective tempo stops at 40 BPM.
    engine.set_tempo_range(1, 100.0).unwrap();
    let state = engine.set_tempo(1, -90.0).unwrap();
    assert!((state.bpm.unwrap() - 40.0).abs() < 1e-9);
    assert!((state.tempo_percent - (40.0 / 120.0 - 1.0) * 100.0).abs() < 1e-9);
}

#[test]
fn pitch_bend_nudges_while_held() {
    let engine = Engine::offline(RATE);
    click_deck(&engine, 1, 120.0, 0.0, vec![]);
    // Paused: nothing to bend.
    let state = engine.pitch_bend(1, BendDirection::Forward, true).unwrap();
    assert_eq!(state.pitch_bend, None);
    engine.play(1).unwrap();
    let state = engine.pitch_bend(1, BendDirection::Forward, true).unwrap();
    assert_eq!(state.pitch_bend, Some(BendDirection::Forward));
    assert!((state.rate - (1.0 + crate::PITCH_BEND)).abs() < 1e-12);
    assert_eq!(state.bpm, Some(120.0), "BPM shows the tempo, not the nudge");
    render(&engine, 1.0);
    assert_near(
        engine.deck(1).unwrap().position_us,
        us(1.0 + crate::PITCH_BEND),
        0.5,
        "bent second",
    );
    let state = engine.pitch_bend(1, BendDirection::Forward, false).unwrap();
    assert_eq!(state.pitch_bend, None);
    assert_eq!(state.rate, 1.0);
    let state = engine.pitch_bend(1, BendDirection::Back, true).unwrap();
    assert!((state.rate - (1.0 - crate::PITCH_BEND)).abs() < 1e-12);
    engine.pitch_bend(1, BendDirection::Back, false).unwrap();
}

#[test]
fn pitch_bending_a_beat_synced_deck_drops_to_bpm_sync() {
    let engine = Engine::offline(RATE);
    click_deck(&engine, 1, 120.0, 0.0, vec![]);
    click_deck(&engine, 2, 126.0, 0.1, vec![]);
    engine.play(1).unwrap();
    engine.sync(2, SyncMode::Beat).unwrap();
    engine.play(2).unwrap();
    render(&engine, 1.0);
    let state = engine.pitch_bend(2, BendDirection::Forward, true).unwrap();
    assert_eq!(state.sync_mode, SyncMode::Bpm);
    render(&engine, 0.3);
    engine.pitch_bend(2, BendDirection::Forward, false).unwrap();
    // Tempo still matched, phase left where the nudge put it.
    render(&engine, 2.0);
    let s2 = engine.deck(2).unwrap();
    assert!((s2.bpm.unwrap() - 120.0).abs() < 1e-9);
    let err = phase_error_ms(&engine, (0.0, 120.0), (0.1, 126.0));
    assert!(err > 5.0, "nudged out of phase: {err} ms");
    // Toggling SYNC restores beat sync, which pulls it back into phase.
    engine.sync(2, SyncMode::Off).unwrap();
    assert_eq!(
        engine.sync(2, SyncMode::Beat).unwrap().sync_mode,
        SyncMode::Beat
    );
    render(&engine, 0.5);
    let err = phase_error_ms(&engine, (0.0, 120.0), (0.1, 126.0));
    assert!(err < 1.0, "back in phase: {err} ms");
}

#[test]
fn bpm_sync_matches_tempo_only() {
    let engine = Engine::offline(RATE);
    click_deck(&engine, 1, 120.0, 0.0, vec![]);
    click_deck(&engine, 2, 126.0, 0.1, vec![]);
    engine.play(1).unwrap();
    render(&engine, 0.13);
    engine.sync(2, SyncMode::Bpm).unwrap();
    let before = engine.play(2).unwrap();
    render(&engine, 1.0);
    let after = engine.deck(2).unwrap();
    assert!((after.bpm.unwrap() - 120.0).abs() < 1e-9);
    // No phase jump: it moved exactly at the matched rate.
    let travelled = (after.position_us - before.position_us) as f64;
    assert!((travelled - 1e6 * 120.0 / 126.0).abs() < 1.0, "{travelled}");
}

// Mixer extras -------------------------------------------------------------------

#[test]
fn band_kills_and_auto_gain() {
    let engine = Engine::offline(RATE);
    deck_one_only(&engine);
    let source = tones(RATE, 20.0, &[(60.0, 0.2), (1000.0, 0.2), (10_000.0, 0.2)]);
    let mut request = samples(RATE, source, None, vec![]);
    request.auto_gain_db = Some(-6.0);
    engine.load(1, request).unwrap();
    assert_eq!(engine.mixer().channels[0].auto_gain_db, -6.0);
    engine.play(1).unwrap();
    let levels = |engine: &Engine| {
        render(engine, 0.3);
        let out = left(&render(engine, 0.5));
        [60.0, 1000.0, 10_000.0].map(|f| amplitude(&out, f, RATE))
    };
    let half = 0.2 * 10f64.powf(-6.0 / 20.0);
    let [low, mid, high] = levels(&engine);
    for level in [low, mid, high] {
        assert!((level - half).abs() < 1e-3, "auto gain −6 dB: {level}");
    }

    // A kill is independent of the EQ gain, which comes back on release.
    let set = |update: MixerSettingsUpdate| engine.set_mixer(update).unwrap();
    set(MixerSettingsUpdate {
        deck: Some(1),
        eq_low: Some(3.0),
        auto_gain_db: Some(0.0),
        ..Default::default()
    });
    let state = set(MixerSettingsUpdate {
        deck: Some(1),
        kill_low: Some(true),
        kill_high: Some(true),
        ..Default::default()
    });
    assert!(state.channels[0].kill_low && state.channels[0].kill_high);
    assert!(!state.channels[0].kill_mid);
    assert_eq!(state.channels[0].eq_low, 3.0);
    let [low, mid, high] = levels(&engine);
    assert!(db(low / 0.2) < -40.0, "low killed: {} dB", db(low / 0.2));
    assert!(db(high / 0.2) < -40.0, "high killed: {} dB", db(high / 0.2));
    assert!(db(mid / 0.2).abs() < 1.0);
    set(MixerSettingsUpdate {
        deck: Some(1),
        kill_low: Some(false),
        kill_high: Some(false),
        ..Default::default()
    });
    let [low, _, high] = levels(&engine);
    assert!(
        (db(low / 0.2) - 3.0).abs() < 0.5,
        "EQ back: {} dB",
        db(low / 0.2)
    );
    assert!(db(high / 0.2).abs() < 1.0);
    // Channel settings need a deck; auto gain is clamped.
    assert!(matches!(
        engine.set_mixer(MixerSettingsUpdate {
            kill_mid: Some(true),
            ..Default::default()
        }),
        Err(Error::InvalidInput(_))
    ));
    let state = set(MixerSettingsUpdate {
        deck: Some(1),
        auto_gain_db: Some(99.0),
        ..Default::default()
    });
    assert_eq!(state.channels[0].auto_gain_db, crate::AUTO_GAIN_MAX_DB);
}

// Events -----------------------------------------------------------------------

fn deck_events(events: &[EngineEvent], deck: u8) -> Vec<&crate::DeckEvent> {
    events
        .iter()
        .filter_map(|e| match e {
            EngineEvent::Deck(d) if d.state.deck == deck => Some(&**d),
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
    // Two decks, the mixer and the preview player.
    assert_eq!(seen.lock().unwrap().len(), 4);
    let json = serde_json::to_value(&seen.lock().unwrap()[0]).unwrap();
    assert_eq!(json["kind"], "deck");
    assert!(json["sent_at_ms"].is_number());
    assert!(json["position_us"].is_number());
    let json = serde_json::to_value(&seen.lock().unwrap()[3]).unwrap();
    assert_eq!(json["kind"], "preview");
    assert_eq!(json["playing"], false);
}

// Cue editing --------------------------------------------------------------------

#[test]
fn cue_colours_and_comments_come_with_the_state() {
    let engine = Engine::offline(RATE);
    let cues = vec![
        Cue {
            color: Some(22),
            comment: Some("drop".into()),
            ..hot_cue(0, 4.0, None)
        },
        Cue {
            color: Some(3),
            comment: Some("intro".into()),
            ..memory_cue(1.0)
        },
        memory_cue(2.0),
    ];
    click_deck(&engine, 1, 120.0, 0.0, cues);
    let state = engine.deck(1).unwrap();
    assert_eq!(state.hot_cues[0].color, Some(22));
    assert_eq!(state.hot_cues[0].comment.as_deref(), Some("drop"));
    assert_eq!(state.memory_cues[0].color, Some(3));
    assert_eq!(state.memory_cues[0].comment.as_deref(), Some("intro"));
    assert_eq!(state.memory_cues[1].color, None);
    assert_eq!(state.memory_cues[1].comment, None);
    let json = serde_json::to_value(&state).unwrap();
    assert_eq!(json["hot_cues"][0]["color"], 22);
    assert_eq!(json["memory_cues"][1]["comment"], serde_json::Value::Null);

    // A pad stored on the deck has neither.
    engine.seek(1, 8_000_000).unwrap();
    let state = engine.hot_cue(1, 1, true).unwrap();
    assert_eq!(state.hot_cues[1].start_us, 8_000_000);
    assert_eq!(
        (state.hot_cues[1].color, &state.hot_cues[1].comment),
        (None, &None)
    );
    // Deleting a memory cue keeps the others' colours with them.
    let (state, _) = engine.memory_cue_delete(1, 0).unwrap();
    assert_eq!(state.memory_cues.len(), 1);
    assert_eq!(state.memory_cues[0].start_us, 2_000_000);
    assert_eq!(state.memory_cues[0].color, None);
    assert_eq!(state.hot_cues[0].comment.as_deref(), Some("drop"));
}

#[test]
fn hot_cues_can_be_deleted() {
    let engine = Engine::offline(RATE);
    let coloured = Cue {
        color: Some(9),
        ..hot_cue(1, 4.0, None)
    };
    click_deck(&engine, 1, 120.0, 0.0, vec![coloured]);
    let (state, removed) = engine.hot_cue_delete(1, 1).unwrap();
    assert!(state.hot_cues.is_empty());
    let removed = removed.unwrap();
    assert_eq!(
        (removed.hot_cue, removed.start_us, removed.end_us),
        (Some(1), 4_000_000, None)
    );
    // An empty pad has nothing to remove.
    assert_eq!(engine.hot_cue_delete(1, 1).unwrap().1, None);
    // The pad stores a new cue, without the old colour.
    engine.seek(1, 3_000_000).unwrap();
    let state = engine.hot_cue(1, 1, true).unwrap();
    assert_eq!(state.hot_cues[0].start_us, 3_000_000);
    assert_eq!(state.hot_cues[0].color, None);
    assert!(matches!(
        engine.hot_cue_delete(1, 8),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn memory_cues_are_set_and_deleted() {
    let engine = Engine::offline(RATE);
    click_deck(&engine, 1, 120.0, 0.0, vec![memory_cue(6.0)]);
    // A fresh deck has nothing to save.
    assert!(matches!(
        engine.memory_cue_set(1),
        Err(Error::InvalidInput(_))
    ));

    // The cue point (set by CUE while paused, snapped to the beat).
    engine.seek(1, 2_010_000).unwrap();
    engine.cue(1, true).unwrap();
    engine.cue(1, false).unwrap();
    let (state, added) = engine.memory_cue_set(1).unwrap();
    let added = added.unwrap();
    assert_eq!(
        (added.hot_cue, added.start_us, added.end_us),
        (None, 2_000_000, None)
    );
    let starts: Vec<i64> = state.memory_cues.iter().map(|c| c.start_us).collect();
    assert_eq!(starts, [2_000_000, 6_000_000]);
    // Saving the same cue again adds nothing.
    let (state, added) = engine.memory_cue_set(1).unwrap();
    assert_eq!((added, state.memory_cues.len()), (None, 2));

    // During a loop: the loop, as a memory loop.
    engine.beat_loop(1, 4.0).unwrap();
    let (state, added) = engine.memory_cue_set(1).unwrap();
    assert_eq!(added.unwrap().end_us, Some(4_000_000));
    assert_eq!(state.memory_cues[1].end_us, Some(4_000_000));

    // Just after a loop (exited): still the loop, not the cue IN set.
    engine.seek(1, 10_000_000).unwrap();
    engine.loop_in(1).unwrap();
    engine.seek(1, 11_000_000).unwrap();
    engine.loop_out(1).unwrap();
    engine.loop_exit(1).unwrap();
    let (_, added) = engine.memory_cue_set(1).unwrap();
    let added = added.unwrap();
    assert_eq!(
        (added.start_us, added.end_us),
        (10_000_000, Some(11_000_000))
    );
    // A cue point set afterwards is newer: that's what MEMORY saves.
    engine.seek(1, 14_000_000).unwrap();
    engine.cue(1, true).unwrap();
    engine.cue(1, false).unwrap();
    let (state, added) = engine.memory_cue_set(1).unwrap();
    assert_eq!(
        (added.unwrap().start_us, state.memory_cues.len()),
        (14_000_000, 5)
    );

    // Delete by index; the rest close up.
    let (state, removed) = engine.memory_cue_delete(1, 0).unwrap();
    assert_eq!((removed.start_us, removed.end_us), (2_000_000, None));
    assert_eq!(state.memory_cues[0].start_us, 2_000_000);
    assert_eq!(state.memory_cues[0].end_us, Some(4_000_000));
    assert_eq!(state.memory_cues[0].index, 0);
    assert!(matches!(
        engine.memory_cue_delete(1, 9),
        Err(Error::InvalidInput(_))
    ));

    // rekordbox's limit: 10 per track.
    for second in 20..26 {
        engine.seek(1, second * 1_000_000).unwrap();
        engine.cue(1, true).unwrap();
        engine.cue(1, false).unwrap();
        engine.memory_cue_set(1).unwrap();
    }
    assert_eq!(engine.deck(1).unwrap().memory_cues.len(), 10);
    engine.seek(1, 30_000_000).unwrap();
    engine.cue(1, true).unwrap();
    engine.cue(1, false).unwrap();
    assert!(matches!(
        engine.memory_cue_set(1),
        Err(Error::InvalidInput(m)) if m.contains("10 memory cues")
    ));
}

// Quantize resolution and bar sync --------------------------------------------------

#[test]
fn quantize_snaps_at_the_chosen_resolution() {
    let engine = Engine::offline(RATE);
    // 120 BPM: a beat is 500 ms, ¼ beat 125 ms, ⅛ beat 62.5 ms.
    click_deck(&engine, 1, 120.0, 0.0, vec![]);
    assert_eq!(engine.status().quantize_beats, 1.0);
    engine.seek(1, 2_130_000).unwrap();
    assert_eq!(engine.cue(1, true).unwrap().cue_us, Some(2_000_000));
    engine.cue(1, false).unwrap();

    let status = engine.set_quantize_beats(0.25).unwrap();
    assert_eq!(status.quantize_beats, 0.25);
    engine.seek(1, 2_130_000).unwrap();
    assert_eq!(engine.cue(1, true).unwrap().cue_us, Some(2_125_000));
    engine.cue(1, false).unwrap();
    // Loop in and out.
    engine.seek(1, 3_070_000).unwrap();
    assert_eq!(engine.loop_in(1).unwrap().loop_in_us, Some(3_125_000));
    engine.seek(1, 3_880_000).unwrap();
    let state = engine.loop_out(1).unwrap();
    let lp = state.active_loop.unwrap();
    assert_eq!((lp.start_us, lp.end_us), (3_125_000, 3_875_000));
    engine.loop_exit(1).unwrap();
    // Hot cue store.
    engine.seek(1, 5_190_000).unwrap();
    let state = engine.hot_cue(1, 0, true).unwrap();
    assert_eq!(state.hot_cues[0].start_us, 5_250_000);
    engine.hot_cue(1, 0, false).unwrap();
    // The auto loop's start, at ⅛ beat.
    engine.set_quantize_beats(0.125).unwrap();
    engine.seek(1, 7_040_000).unwrap();
    let lp = engine.beat_loop(1, 1.0).unwrap().active_loop.unwrap();
    assert_eq!((lp.start_us, lp.end_us), (7_062_500, 7_562_500));
    engine.loop_exit(1).unwrap();
    // Beat jumps stay whole beats.
    engine.seek(1, 8_010_000).unwrap();
    assert_eq!(engine.beat_jump(1, 4.0).unwrap().position_us, 10_010_000);
    assert!(matches!(
        engine.set_quantize_beats(0.3),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn hot_cue_jumps_keep_the_phase_at_the_quantize_resolution() {
    for (beats, landing) in [(1.0, 10_200_000), (0.25, 9_950_000)] {
        let engine = Engine::offline(RATE);
        click_deck(&engine, 1, 120.0, 0.0, vec![hot_cue(0, 10.0, None)]);
        engine.set_quantize_beats(beats).unwrap();
        engine.seek(1, 2_200_000).unwrap();
        engine.play(1).unwrap();
        // 2.2 s is 200 ms past the beat at 2.0 s, and 50 ms before the
        // quarter beat at 2.25 s.
        let state = engine.hot_cue(1, 0, true).unwrap();
        assert_near(state.position_us, landing, 1.0, "landing");
    }
}

/// Where a deck is within its bar, in beats, for the test grids (first
/// beat = a downbeat).
fn bar_beat(position_us: i64, first_beat: f64, bpm: f64) -> f64 {
    ((position_us as f64 / 1e6 - first_beat) * bpm / 60.0).rem_euclid(4.0)
}

#[test]
fn bar_sync_aligns_downbeats_and_beat_sync_only_beats() {
    for phase in [crate::SyncPhase::Beat, crate::SyncPhase::Bar] {
        let engine = Engine::offline(RATE);
        click_deck(&engine, 1, 120.0, 0.0, vec![]);
        click_deck(&engine, 2, 126.0, 0.1, vec![]);
        assert_eq!(
            engine.set_sync_phase(phase).unwrap().sync_phase,
            phase,
            "reported"
        );
        engine.play(1).unwrap();
        render(&engine, 1.3);
        // Deck 2 waits on the 2nd beat of a bar; deck 1 is in its 3rd.
        let start = 0.1 + 5.0 * 60.0 / 126.0;
        engine.seek(2, us(start)).unwrap();
        engine.sync(2, SyncMode::Beat).unwrap();
        engine.play(2).unwrap();
        render(&engine, 2.0);
        let err = phase_error_ms(&engine, (0.0, 120.0), (0.1, 126.0));
        assert!(err < 1.0, "{phase:?}: phase error {err} ms");
        let bars = |engine: &Engine| {
            let b1 = bar_beat(engine.deck(1).unwrap().position_us, 0.0, 120.0);
            let b2 = bar_beat(engine.deck(2).unwrap().position_us, 0.1, 126.0);
            let d = (b1 - b2).rem_euclid(4.0);
            d.min(4.0 - d)
        };
        // How far sync moved deck 2 from where it would have played to.
        let free = start + 2.0 * 120.0 / 126.0;
        let moved = (engine.deck(2).unwrap().position_us as f64 / 1e6 - free) * 126.0 / 60.0;
        match phase {
            crate::SyncPhase::Bar => {
                assert!(bars(&engine) < 0.002, "bar positions equal");
                assert!(moved.abs() <= 2.0, "at most half a bar: {moved} beats");
                // Staying in sync: the master jumps a beat, the slave follows
                // to the same place in the bar.
                engine.beat_jump(1, 1.0).unwrap();
                render(&engine, 0.5);
                assert!(bars(&engine) < 0.002, "still downbeat to downbeat");
            }
            crate::SyncPhase::Beat => {
                let d = bars(&engine);
                assert!(
                    d > 0.99 && (d - d.round()).abs() < 0.002,
                    "beats aligned, bars not: {d}"
                );
                assert!(moved.abs() < 0.5, "only the beat phase: {moved} beats");
            }
        }
    }
}

// Scrubbing -----------------------------------------------------------------------

/// The largest jump between neighbouring samples.
fn max_step(signal: &[f32]) -> f32 {
    signal
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

fn rms(signal: &[f32]) -> f64 {
    (signal.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / signal.len() as f64).sqrt()
}

/// Neighbouring samples of a 0.5 sine at 200 Hz differ by at most
/// 2π·200·0.5/48000 ≈ 0.013 at normal speed: a click is far above that.
const TONE_HZ: f64 = 200.0;
const MAX_SMOOTH_STEP: f32 = 0.03;

fn tone_deck(engine: &Engine) {
    deck_one_only(engine);
    engine
        .load(
            1,
            samples(RATE, tones(RATE, 30.0, &[(TONE_HZ, 0.5)]), None, vec![]),
        )
        .unwrap();
}

/// Drags deck 1 at `speed` × normal for `seconds` the way the UI does (a
/// move every 8 ms), rendering in between. Returns the left channel and the
/// final target.
fn drag(engine: &Engine, from: f64, speed: f64, seconds: f64, out: &mut Vec<f32>) -> f64 {
    let chunk = 0.008;
    let mut target = from;
    for _ in 0..(seconds / chunk).round() as usize {
        target += chunk * speed;
        engine
            .scrub(1, crate::ScrubAction::Move, Some(us(target)))
            .unwrap();
        out.extend(left(&render(engine, chunk)));
    }
    target
}

#[test]
fn scrubbing_follows_the_drag_smoothly() {
    let engine = Engine::offline(RATE);
    tone_deck(&engine);
    engine.seek(1, 5_000_000).unwrap();
    let state = engine
        .scrub(1, crate::ScrubAction::Start, Some(5_000_000))
        .unwrap();
    assert!(state.scrubbing && !state.playing);

    // Dragged forward at normal speed for a second.
    let mut out = Vec::new();
    let target = drag(&engine, 5.0, 1.0, 1.0, &mut out);
    let state = engine.deck(1).unwrap();
    assert!((state.rate - 1.0).abs() < 0.15, "speed {}", state.rate);
    assert_near(
        state.position_us,
        us(target - crate::deck::SCRUB_FOLLOW),
        10_000.0,
        "a little behind the hand",
    );
    // No fades per move: a steady level once up to speed (the tone's RMS
    // is 0.354).
    let window = (RATE as f64 / TONE_HZ) as usize;
    for (i, w) in out[RATE as usize / 5..].chunks(window).enumerate() {
        assert!(rms(w) > 0.3, "window {i}: level {}", rms(w));
    }

    // Let go: it comes to rest exactly where it was let go, paused, silent.
    engine
        .scrub(1, crate::ScrubAction::End, Some(us(target)))
        .unwrap();
    out.extend(left(&render(&engine, 0.5)));
    let state = engine.deck(1).unwrap();
    assert!(!state.scrubbing && !state.playing);
    assert_eq!(state.rate, 0.0);
    assert_near(state.position_us, us(target), 1.0, "at rest");
    assert!(out[out.len() - 2000..].iter().all(|&s| s == 0.0));
    // Never a click.
    assert!(max_step(&out) < MAX_SMOOTH_STEP, "step {}", max_step(&out));
    // Position kept moving forward smoothly: no jumps reported.
    render(&engine, 0.2);
    assert_eq!(engine.deck(1).unwrap().position_us, state.position_us);
}

#[test]
fn scrubbing_a_playing_deck_plays_on_when_let_go() {
    let engine = Engine::offline(RATE);
    tone_deck(&engine);
    engine.seek(1, 5_000_000).unwrap();
    engine.play(1).unwrap();
    let mut out = left(&render(&engine, 0.5));
    let grabbed = engine.deck(1).unwrap().position_us as f64 / 1e6;
    // Hand on the record at the playhead, then dragged backwards.
    let state = engine.scrub(1, crate::ScrubAction::Start, None).unwrap();
    assert!(state.scrubbing && state.playing, "PLAY stays latched");
    let target = drag(&engine, grabbed, -1.0, 0.4, &mut out);
    let state = engine.deck(1).unwrap();
    assert!(state.rate < -0.7, "running backwards: {}", state.rate);
    assert!(state.position_us < us(grabbed));

    // Let go: playback takes over at the deck's tempo from about there.
    engine
        .scrub(1, crate::ScrubAction::End, Some(us(target)))
        .unwrap();
    out.extend(left(&render(&engine, 0.3)));
    let state = engine.deck(1).unwrap();
    assert!(state.playing && !state.scrubbing);
    assert_eq!(state.rate, 1.0);
    let expected = target + 0.3;
    assert_near(state.position_us, us(expected), 60_000.0, "playing on");
    assert!(max_step(&out) < MAX_SMOOTH_STEP, "step {}", max_step(&out));
    // Full level again.
    assert!(rms(&out[out.len() - 2400..]) > 0.34);
}

#[test]
fn scrubbing_jumps_to_far_targets_and_ends_loops_it_leaves() {
    let engine = Engine::offline(RATE);
    click_deck(&engine, 1, 120.0, 0.0, vec![]);
    engine.seek(1, 4_000_000).unwrap();
    engine.beat_loop(1, 4.0).unwrap();
    engine.scrub(1, crate::ScrubAction::Start, None).unwrap();
    // Inside the loop: it stays.
    let state = engine
        .scrub(1, crate::ScrubAction::Move, Some(5_000_000))
        .unwrap();
    assert!(state.active_loop.is_some());
    // Far away: a jump rather than a race, and the loop ends.
    let state = engine
        .scrub(1, crate::ScrubAction::Move, Some(20_000_000))
        .unwrap();
    assert_eq!(state.position_us, 20_000_000);
    assert!(state.active_loop.is_none());
    // End without a scrub does nothing; end with one rests.
    engine.scrub(1, crate::ScrubAction::End, None).unwrap();
    render(&engine, 0.5);
    let state = engine.deck(1).unwrap();
    assert!(!state.scrubbing);
    assert_near(state.position_us, 20_000_000, 1.0, "at rest");
    let state = engine.scrub(1, crate::ScrubAction::End, None).unwrap();
    assert!(!state.scrubbing);
    assert!(matches!(
        engine.scrub(2, crate::ScrubAction::Start, None),
        Err(Error::NoTrack(2))
    ));
}

// Preview player --------------------------------------------------------------------

fn preview(samples: Vec<f32>, position_us: i64, gain: Option<f32>) -> crate::PreviewRequest {
    crate::PreviewRequest {
        track_id: Some(9),
        source: Source::Samples {
            sample_rate: RATE,
            channels: 2,
            samples,
        },
        position_us,
        duration_secs: None,
        auto_gain_db: gain,
    }
}

#[test]
fn the_preview_plays_at_its_gain_and_pauses_the_decks() {
    let engine = Engine::offline(RATE);
    two_tones(&engine);
    let state = engine
        .preview_play(preview(
            tones(RATE, 10.0, &[(2000.0, 0.5)]),
            3_000_000,
            Some(-6.0),
        ))
        .unwrap();
    assert!(state.playing);
    assert_eq!(state.track_id, Some(9));
    assert_eq!(state.position_us, 3_000_000);
    assert_eq!(state.rate, 1.0);
    assert_eq!(state.auto_gain_db, -6.0);
    assert_eq!(state.duration_us, 10_000_000);
    // Previewing pauses the decks (rekordbox).
    assert!(!engine.deck(1).unwrap().playing);
    assert!(!engine.deck(2).unwrap().playing);

    let out = left(&render(&engine, 0.5));
    let settled = &out[RATE as usize / 10..];
    let expected = 0.5 * 10f64.powf(-6.0 / 20.0);
    assert!((amplitude(settled, 2000.0, RATE) - expected).abs() < 0.005);
    assert!(amplitude(settled, 440.0, RATE) < 1e-3, "deck 1 paused");
    assert!(amplitude(settled, 1000.0, RATE) < 1e-3, "deck 2 paused");
    let state = engine.preview();
    assert_near(state.position_us, 3_500_000, 1.0, "after 0.5 s");
    assert_eq!(engine.status().preview, state);

    // The master volume applies (TUNEOX_MUTED silences previews too).
    engine
        .set_mixer(MixerSettingsUpdate {
            master_volume: Some(0.0),
            ..Default::default()
        })
        .unwrap();
    let out = render(&engine, 0.1);
    assert!(out[256..].iter().all(|&s| s == 0.0));
    engine
        .set_mixer(MixerSettingsUpdate {
            master_volume: Some(1.0),
            ..Default::default()
        })
        .unwrap();

    // Stop: fades out, keeps the track and position.
    let state = engine.preview_stop().unwrap();
    assert!(!state.playing);
    assert_eq!(state.rate, 0.0);
    assert_eq!(state.track_id, Some(9));
    let out = render(&engine, 0.1);
    assert!(out[2 * FADE as usize..].iter().all(|&s| s == 0.0));
    assert!(
        out[..2 * FADE as usize].iter().any(|&s| s != 0.0),
        "a fade, not a cut"
    );
}

#[test]
fn a_deck_starting_stops_the_preview() {
    let engine = Engine::offline(RATE);
    engine
        .load(1, samples(RATE, noise(RATE, 5.0), None, vec![]))
        .unwrap();
    engine
        .preview_play(preview(noise(RATE, 5.0), 0, None))
        .unwrap();
    render(&engine, 0.1);
    assert!(engine.preview().playing);
    engine.play(1).unwrap();
    assert!(!engine.preview().playing, "one output: the deck wins");
}

#[test]
fn the_preview_stops_at_the_end_and_reuses_decoded_files() {
    let engine = Engine::offline(RATE);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tone.wav");
    write_wav(&path, RATE, &tones(RATE, 2.0, &[(440.0, 0.5)]));
    let request = |position_us| crate::PreviewRequest {
        track_id: Some(3),
        source: Source::File(path.clone()),
        position_us,
        duration_secs: None,
        auto_gain_db: Some(2.0),
    };
    engine.preview_play(request(1_000_000)).unwrap();
    let state = engine
        .wait_until_preview_loaded(Duration::from_secs(10))
        .unwrap();
    assert!(!state.loading && state.error.is_none());
    render(&engine, 0.5);
    // Another spot in the same track: just a jump.
    let load_id = lock(&engine.shared().preview).load_id;
    let state = engine.preview_play(request(500_000)).unwrap();
    assert_eq!(lock(&engine.shared().preview).load_id, load_id);
    assert!(!state.loading);
    assert_eq!(state.position_us, 500_000);
    // Loading it onto a deck reuses the decoded audio.
    let deck = engine.load(1, file(&path)).unwrap();
    assert!(!deck.loading);
    // It stops at the end.
    render(&engine, 2.0);
    let state = engine.preview();
    assert!(!state.playing);
    assert_eq!(state.position_us, 2_000_000);
    let events = engine.poll_events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, EngineEvent::Preview(p) if !p.state.playing))
    );
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
            memory_cues: [None; crate::deck::MEMORY_CUES],
            gain_db: -3.0,
        }
    };
    // Preview tracks, built off the audio thread too.
    let preview_track = |load_id, seconds| crate::preview::PreviewTrack {
        load_id,
        pcm: std::sync::Arc::new(crate::pcm::TrackPcm::from_samples(
            RATE,
            2,
            &noise(RATE, seconds),
        )),
        gain_db: -3.0,
    };
    let (preview_a, preview_b) = (preview_track(1, 5.0), preview_track(2, 3.0));
    use crate::deck::ScrubPhase;
    let controls = vec![
        Control::Play(0),
        Control::QuantizeBeats(0.25),
        Control::SyncBars(true),
        Control::Sync {
            deck: 1,
            mode: SyncMode::Beat,
        },
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
        Control::PitchBend {
            deck: 1,
            direction: 1,
            press: true,
        },
        Control::PitchBend {
            deck: 1,
            direction: 1,
            press: false,
        },
        Control::LoopIn(1),
        Control::LoopOut(1),
        Control::LoopResize {
            deck: 1,
            double: true,
        },
        Control::LoopExit(1),
        Control::Reloop(1),
        Control::MemoryCue {
            deck: 0,
            target: crate::deck::MemoryTarget::Next,
        },
        // Scrubbing inside the hot loop, then playing on.
        Control::Scrub {
            deck: 0,
            phase: ScrubPhase::Start,
            seconds: None,
        },
        Control::Scrub {
            deck: 0,
            phase: ScrubPhase::Move,
            seconds: Some(4.5),
        },
        Control::Scrub {
            deck: 0,
            phase: ScrubPhase::Move,
            seconds: Some(4.6),
        },
        Control::Scrub {
            deck: 0,
            phase: ScrubPhase::End,
            seconds: Some(4.6),
        },
        Control::MemoryCueSet(0),
        Control::MemoryCueDelete { deck: 0, index: 0 },
        Control::HotCueDelete { deck: 0, slot: 3 },
        // The preview: a track, a jump, another track (cross-fading from the
        // first, which is then sent off to be freed), stop, play again.
        Control::PreviewPlay {
            track: Some(preview_a),
            seconds: 1.0,
        },
        Control::PreviewPlay {
            track: None,
            seconds: 2.0,
        },
        Control::PreviewPlay {
            track: Some(preview_b),
            seconds: 0.5,
        },
        Control::PreviewStop,
        Control::PreviewPlay {
            track: None,
            seconds: 0.0,
        },
        // Deck 1 plays again (the preview paused it), which stops the preview.
        Control::Play(0),
        Control::Quantize { deck: 0, on: false },
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
    for block in 0..900 {
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
    assert!(!state.scrubbing);
    assert_eq!(engine.mixer().channels[1].auto_gain_db, -3.0);
    assert!(
        !engine.preview().playing,
        "deck 1 playing stopped the preview"
    );
}

// Real device -------------------------------------------------------------------

/// Opens the default output with nothing loaded (silence), checks the clock
/// and a command round trip, and closes it. Run by hand:
/// `cargo test -p tuneox-audio -- --ignored`.
#[test]
#[ignore = "opens the real audio device (silently)"]
fn opens_the_default_device_silently() {
    let started = std::time::Instant::now();
    let engine = Engine::start().unwrap();
    let returned = started.elapsed();
    // The first command waits for the device to open.
    engine.set_hot_cue_gate(false).unwrap();
    let output = engine.output().unwrap();
    eprintln!(
        "start returned in {:.2} ms, device ready after {:.1} ms: {output:?}",
        returned.as_secs_f64() * 1000.0,
        started.elapsed().as_secs_f64() * 1000.0
    );
    let before = engine.deck(1).unwrap().at_ms;
    std::thread::sleep(Duration::from_millis(200));
    let after = engine.deck(1).unwrap().at_ms;
    assert!(after > before + 100.0, "clock moves: {before} → {after}");
    let started = std::time::Instant::now();
    let status = engine.set_hot_cue_gate(true).unwrap();
    eprintln!(
        "command round trip: {:.2} ms, latency {:.2} ms",
        started.elapsed().as_secs_f64() * 1000.0,
        engine.output().unwrap().latency_ms
    );
    assert!(status.running && status.hot_cue_gate);
    drop(engine);
}
