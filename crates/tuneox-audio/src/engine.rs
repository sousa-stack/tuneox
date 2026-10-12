//! The engine handle: what the command layer and the desktop app hold.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tuneox_core::{BeatGrid, Cue, TrackId};

use crate::core::{Control, Core, Garbage, MixerUpdate, Snapshot};
use crate::deck::{
    HOT_CUES, HotCue, LoadedTrack, Loop, MAX_BPM, MAX_LOOP_BEATS, MEMORY_CUES, MIN_BPM,
    MIN_LOOP_BEATS, MemoryTarget,
};
use crate::events::EventPump;
use crate::grid::Grid;
use crate::mixer::MixerSettings;
use crate::output::{self, OutputMessage};
use crate::pcm::TrackPcm;
use crate::state::{
    BendDirection, ChannelState, DeckState, EngineEvent, HotCueState, LoopState, MemoryCueState,
    MixerState, OutputInfo, PlayerStatus, SyncMode,
};
use crate::{Error, Result};

/// Commands that can wait in the queue before the audio thread takes them.
const QUEUE: usize = 256;
/// How long a command waits for the audio thread to apply it.
const COMMAND_TIMEOUT: Duration = Duration::from_millis(500);
/// How long a command waits while the output device is still opening.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
/// How often the event thread looks for changes.
pub(crate) const EVENT_POLL: Duration = Duration::from_millis(5);
/// Output rate of the offline engine unless told otherwise.
pub const OFFLINE_SAMPLE_RATE: u32 = 48_000;

/// Valid tempo fader ranges in percent (100 = WIDE).
pub const TEMPO_RANGES: [f64; 4] = [6.0, 10.0, 16.0, 100.0];

/// Where a deck's audio comes from.
#[derive(Debug, Clone)]
pub enum Source {
    /// An audio file, decoded on a background thread.
    File(PathBuf),
    /// Interleaved samples in −1…1, e.g. generated audio in tests.
    Samples {
        sample_rate: u32,
        channels: usize,
        samples: Vec<f32>,
    },
}

/// A track to load onto a deck.
#[derive(Debug, Clone)]
pub struct LoadRequest {
    pub track_id: Option<TrackId>,
    pub source: Source,
    pub beat_grid: Option<BeatGrid>,
    /// Hot cues load onto pads A–H; memory cues and loops can be called
    /// with `memory_cue`.
    pub cues: Vec<Cue>,
    /// Used for a steady beat grid if the track has none.
    pub bpm: Option<f64>,
    /// Helps size decode storage for files that don't state their length.
    pub duration_secs: Option<f64>,
    /// The track's auto gain (rekordbox analyses one per track); 0 dB if
    /// `None`.
    pub auto_gain_db: Option<f32>,
}

/// The audio engine: two decks and a mixer. Cheap to clone; every clone
/// controls the same engine, which stops when the last one is dropped.
#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

struct Inner {
    shared: Arc<Shared>,
    threads: Mutex<Vec<JoinHandle<()>>>,
    output: Option<Mutex<mpsc::Sender<OutputMessage>>>,
}

pub(crate) enum Clock {
    Real(Instant),
    /// Offline: time is what's been rendered.
    Virtual,
}

#[derive(Default)]
struct DeckMeta {
    load_id: u64,
    track_id: Option<TrackId>,
    path: Option<PathBuf>,
    pcm: Option<Arc<TrackPcm>>,
    has_grid: bool,
}

type EventHandler = Box<dyn Fn(&EngineEvent) + Send>;

pub(crate) struct Shared {
    pub core: Mutex<Core>,
    pub commands: Mutex<(u64, rtrb::Producer<(u64, Control)>)>,
    state: Mutex<triple_buffer::Output<Snapshot>>,
    garbage: Mutex<rtrb::Consumer<Garbage>>,
    decks: Mutex<[DeckMeta; 2]>,
    pub clock: Clock,
    pub output_info: Mutex<Option<OutputInfo>>,
    /// Output latency the device last reported, in microseconds.
    pub latency_us: AtomicU64,
    handler: Mutex<Option<EventHandler>>,
    pump: Mutex<EventPump>,
    pub shutdown: AtomicBool,
    /// The audio callback (or the silent fallback) is running.
    pub output_ready: AtomicBool,
    /// The core's output rate, readable without touching the core (which
    /// only the audio thread may lock while a stream runs).
    pub device_rate: AtomicU32,
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Nothing holding these locks can leave the data inconsistent.
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    fn new(device_rate: u32, clock: Clock) -> Arc<Self> {
        let (command_tx, command_rx) = rtrb::RingBuffer::new(QUEUE);
        // Room for everything the queued commands could replace.
        let (garbage_tx, garbage_rx) = rtrb::RingBuffer::new(QUEUE * 2 + 8);
        let (state_tx, state_rx) = triple_buffer::triple_buffer(&Snapshot::default());
        Arc::new(Self {
            core: Mutex::new(Core::new(device_rate, command_rx, garbage_tx, state_tx)),
            commands: Mutex::new((0, command_tx)),
            state: Mutex::new(state_rx),
            garbage: Mutex::new(garbage_rx),
            decks: Mutex::new(Default::default()),
            clock,
            output_info: Mutex::new(None),
            latency_us: AtomicU64::new(0),
            handler: Mutex::new(None),
            pump: Mutex::new(EventPump::default()),
            shutdown: AtomicBool::new(false),
            output_ready: AtomicBool::new(false),
            device_rate: AtomicU32::new(device_rate),
        })
    }

    pub(crate) fn now_ms(&self) -> f64 {
        match self.clock {
            Clock::Real(origin) => origin.elapsed().as_secs_f64() * 1000.0,
            Clock::Virtual => lock(&self.core).rendered_ms(),
        }
    }

    fn snapshot(&self) -> Snapshot {
        *lock(&self.state).read()
    }

    pub(crate) fn collect_garbage(&self) {
        let mut garbage = lock(&self.garbage);
        while garbage.pop().is_ok() {}
    }

    /// Looks for state changes and sends events to the handler.
    pub(crate) fn pump_events(&self) -> Vec<EngineEvent> {
        let snapshot = self.snapshot();
        let decks = [self.deck_state(&snapshot, 0), self.deck_state(&snapshot, 1)];
        let mixer = mixer_state(&snapshot.mixer);
        let now = self.now_ms();
        let events = lock(&self.pump).poll(&snapshot, decks, mixer, now);
        if !events.is_empty()
            && let Some(handler) = lock(&self.handler).as_ref()
        {
            for event in &events {
                handler(event);
            }
        }
        events
    }

    fn deck_state(&self, snapshot: &Snapshot, deck: usize) -> DeckState {
        let meta = lock(&self.decks);
        let meta = &meta[deck];
        let s = &snapshot.decks[deck];
        let mut state = DeckState::empty(deck as u8 + 1);
        let us = |seconds: f64| (seconds * 1e6).round() as i64;
        state.at_ms = snapshot.at_ms;
        state.tempo_percent = s.tempo_percent;
        state.tempo_range = s.tempo_range;
        state.sync_mode = s.sync;
        state.master = s.master;
        state.quantize = s.quantize;
        let Some(pcm) = meta.pcm.as_ref() else {
            return state;
        };
        state.track_id = meta.track_id;
        state.loading = !pcm.is_done();
        state.error = pcm.error().map(str::to_string);
        state.duration_us = us(pcm.duration_secs());
        if s.load_id != meta.load_id {
            // The audio thread hasn't taken the new track yet.
            state.loading = true;
            return state;
        }
        state.playing = s.playing;
        state.position_us = us(s.position);
        state.rate = s.rate;
        state.bpm = s.bpm;
        state.original_bpm = s.original_bpm;
        state.pitch_bend = match s.bend {
            b if b < 0 => Some(BendDirection::Back),
            b if b > 0 => Some(BendDirection::Forward),
            _ => None,
        };
        state.cue_us = s.cue.map(us);
        let loop_state = |lp: Loop, beats: Option<f64>| LoopState {
            start_us: us(lp.start),
            end_us: us(lp.end),
            beats,
        };
        state.active_loop = s.active_loop.map(|lp| loop_state(lp, s.active_loop_beats));
        state.last_loop = s.last_loop.map(|lp| loop_state(lp, s.last_loop_beats));
        state.loop_in_us = s.loop_in.map(us);
        state.memory_cues = s
            .memory_cues
            .iter()
            .flatten()
            .enumerate()
            .map(|(index, cue)| MemoryCueState {
                index: index as u8,
                start_us: us(cue.start),
                end_us: cue.end.map(us),
            })
            .collect();
        state.hot_cues = s
            .hot_cues
            .iter()
            .enumerate()
            .filter_map(|(slot, cue)| {
                cue.map(|cue| HotCueState {
                    slot: slot as u8,
                    start_us: us(cue.start),
                    end_us: cue.end.map(us),
                })
            })
            .collect();
        state
    }
}

pub(crate) fn mixer_state(settings: &MixerSettings) -> MixerState {
    let channel = |deck: usize| {
        let c = settings.channels[deck];
        ChannelState {
            deck: deck as u8 + 1,
            volume: c.volume,
            eq_low: c.eq_db[0],
            eq_mid: c.eq_db[1],
            eq_high: c.eq_db[2],
            kill_low: c.kill[0],
            kill_mid: c.kill[1],
            kill_high: c.kill[2],
            auto_gain_db: c.auto_gain_db,
        }
    };
    MixerState {
        channels: [channel(0), channel(1)],
        crossfader: settings.crossfader,
        master_volume: settings.master_volume,
    }
}

impl Engine {
    /// Starts the engine on the default output device, on its own thread.
    /// Returns without waiting for the device to open; the first commands
    /// wait for it instead. If no device can be opened the engine runs
    /// silently on a timer (see [`OutputInfo::error`]) so decks still work.
    pub fn start() -> Result<Engine> {
        let shared = Shared::new(OFFLINE_SAMPLE_RATE, Clock::Real(Instant::now()));
        let (output, output_thread) = output::start(shared.clone())?;
        let events = std::thread::Builder::new()
            .name("tuneox-audio-events".into())
            .spawn({
                let shared = shared.clone();
                move || {
                    while !shared.shutdown.load(Ordering::Relaxed) {
                        std::thread::sleep(EVENT_POLL);
                        shared.pump_events();
                        shared.collect_garbage();
                    }
                }
            })
            .map_err(|e| Error::Output(format!("couldn't start the event thread: {e}")))?;
        Ok(Engine {
            inner: Arc::new(Inner {
                shared,
                threads: Mutex::new(vec![output_thread, events]),
                output: Some(Mutex::new(output)),
            }),
        })
    }

    /// An engine without a device, for tests and offline rendering. Time
    /// only moves when [`Engine::render`] is called, and commands apply
    /// immediately.
    pub fn offline(sample_rate: u32) -> Engine {
        Engine {
            inner: Arc::new(Inner {
                shared: Shared::new(sample_rate, Clock::Virtual),
                threads: Mutex::new(Vec::new()),
                output: None,
            }),
        }
    }

    fn is_offline(&self) -> bool {
        matches!(self.inner.shared.clock, Clock::Virtual)
    }

    /// Offline engine only: renders interleaved stereo into `out` and moves
    /// time forward by its length. Deck states afterwards describe the end
    /// of what was rendered.
    pub fn render(&self, out: &mut [f32]) {
        assert!(self.is_offline(), "render() is for offline engines");
        {
            let mut core = lock(&self.inner.shared.core);
            let at_ms = core.rendered_ms();
            core.render_interleaved(out, at_ms);
            let now = core.rendered_ms();
            core.update(now);
        }
        self.inner.shared.collect_garbage();
    }

    #[cfg(test)]
    pub(crate) fn shared(&self) -> &Arc<Shared> {
        &self.inner.shared
    }

    /// Offline engine: checks for changes and returns the events the
    /// realtime engine would have sent (also given to the event handler).
    pub fn poll_events(&self) -> Vec<EngineEvent> {
        self.inner.shared.pump_events()
    }

    /// Calls `handler` from the engine's event thread for every event.
    /// Replaces any previous handler.
    pub fn on_event(&self, handler: impl Fn(&EngineEvent) + Send + 'static) {
        *lock(&self.inner.shared.handler) = Some(Box::new(handler));
    }

    /// The engine clock in milliseconds, the time base of `at_ms`.
    pub fn now_ms(&self) -> f64 {
        self.inner.shared.now_ms()
    }

    /// The output sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.inner.shared.device_rate.load(Ordering::Relaxed)
    }

    pub fn output(&self) -> Option<OutputInfo> {
        let shared = &self.inner.shared;
        let mut info = lock(&shared.output_info).clone()?;
        if info.device.is_some() {
            info.latency_ms = shared.latency_us.load(Ordering::Relaxed) as f64 / 1000.0;
        }
        Some(info)
    }

    /// Queues a command and waits until the audio thread has applied it.
    fn send(&self, control: Control) -> Result<()> {
        let shared = &self.inner.shared;
        let seq = {
            let mut commands = lock(&shared.commands);
            let seq = commands.0 + 1;
            commands.1.push((seq, control)).map_err(|_| Error::Busy)?;
            commands.0 = seq;
            seq
        };
        if self.is_offline() {
            let mut core = lock(&shared.core);
            let now = core.rendered_ms();
            core.update(now);
        } else {
            let started = Instant::now();
            while shared.snapshot().applied_seq < seq {
                let limit = if shared.output_ready.load(Ordering::Acquire) {
                    COMMAND_TIMEOUT
                } else {
                    STARTUP_TIMEOUT
                };
                if started.elapsed() > limit {
                    return Err(Error::NotResponding);
                }
                std::thread::sleep(Duration::from_micros(250));
            }
        }
        shared.collect_garbage();
        Ok(())
    }

    fn deck_index(deck: u8) -> Result<usize> {
        match deck {
            1 | 2 => Ok(deck as usize - 1),
            _ => Err(Error::InvalidDeck(deck)),
        }
    }

    fn require_track(&self, deck: u8) -> Result<usize> {
        let index = Self::deck_index(deck)?;
        if lock(&self.inner.shared.decks)[index].pcm.is_none() {
            return Err(Error::NoTrack(deck));
        }
        Ok(index)
    }

    fn require_grid(&self, deck: u8) -> Result<usize> {
        let index = self.require_track(deck)?;
        if !lock(&self.inner.shared.decks)[index].has_grid {
            return Err(Error::NoBeatGrid(deck));
        }
        Ok(index)
    }

    /// The current state of deck 1 or 2.
    pub fn deck(&self, deck: u8) -> Result<DeckState> {
        let index = Self::deck_index(deck)?;
        let shared = &self.inner.shared;
        Ok(shared.deck_state(&shared.snapshot(), index))
    }

    pub fn mixer(&self) -> MixerState {
        mixer_state(&self.inner.shared.snapshot().mixer)
    }

    pub fn status(&self) -> PlayerStatus {
        let shared = &self.inner.shared;
        let snapshot = shared.snapshot();
        PlayerStatus {
            running: true,
            decks: [
                shared.deck_state(&snapshot, 0),
                shared.deck_state(&snapshot, 1),
            ],
            mixer: mixer_state(&snapshot.mixer),
            hot_cue_gate: snapshot.hot_cue_gate,
            output: self.output(),
        }
    }

    pub(crate) fn default_mixer_state() -> MixerState {
        mixer_state(&MixerSettings::default())
    }

    /// Loads a track onto a deck, parked at 0:00 with no cue and no loop.
    /// Returns at once; the track decodes in the background (`loading` in
    /// the state) and can play as soon as audio is available. Loading the
    /// track that's on the other deck reuses its decoded audio. If the
    /// deck was sync master, master passes to the other deck.
    pub fn load(&self, deck: u8, request: LoadRequest) -> Result<DeckState> {
        let index = Self::deck_index(deck)?;
        let shared = &self.inner.shared;
        let grid = request
            .beat_grid
            .as_ref()
            .and_then(|g| Grid::from_beats(&g.beats))
            .or_else(|| request.bpm.and_then(|bpm| Grid::constant(0.0, bpm)));
        let (hot_cues, memory_cues) = cue_tables(&request.cues);

        let (pcm, load_id) = {
            let mut decks = lock(&shared.decks);
            let path = match &request.source {
                Source::File(path) => Some(path.clone()),
                Source::Samples { .. } => None,
            };
            let other = &decks[1 - index];
            let reuse = other
                .pcm
                .clone()
                .filter(|_| path.is_some() && other.path == path && other.pcm.is_some());
            let pcm = match reuse {
                Some(pcm) => pcm,
                None => match &request.source {
                    Source::File(path) => {
                        let pcm = Arc::new(TrackPcm::default());
                        crate::decode::spawn(path.clone(), pcm.clone(), request.duration_secs);
                        pcm
                    }
                    Source::Samples {
                        sample_rate,
                        channels,
                        samples,
                    } => Arc::new(TrackPcm::from_samples(*sample_rate, *channels, samples)),
                },
            };
            let other_pcm = decks[1 - index].pcm.clone();
            let meta = &mut decks[index];
            if let Some(old) = meta.pcm.take()
                && !other_pcm.as_ref().is_some_and(|o| Arc::ptr_eq(o, &old))
                && !Arc::ptr_eq(&old, &pcm)
            {
                old.cancel();
            }
            meta.load_id += 1;
            meta.track_id = request.track_id;
            meta.path = path;
            meta.pcm = Some(pcm.clone());
            meta.has_grid = grid.is_some();
            (pcm, meta.load_id)
        };

        self.send(Control::Load {
            deck: index,
            track: LoadedTrack {
                load_id,
                pcm,
                grid: grid.map(Box::new),
                hot_cues,
                memory_cues,
                gain_db: request.auto_gain_db.unwrap_or(0.0),
            },
        })?;
        self.deck(deck)
    }

    /// Waits until a deck's track has finished decoding (or failed).
    pub fn wait_until_loaded(&self, deck: u8, timeout: Duration) -> Result<DeckState> {
        let index = Self::deck_index(deck)?;
        let started = Instant::now();
        loop {
            let pcm = lock(&self.inner.shared.decks)[index].pcm.clone();
            if pcm.is_none_or(|p| p.is_done()) {
                return self.deck(deck);
            }
            if started.elapsed() > timeout {
                return Err(Error::NotResponding);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Sends a command for a deck that has a track and returns its state.
    fn on_track(&self, deck: u8, control: impl FnOnce(usize) -> Control) -> Result<DeckState> {
        let index = self.require_track(deck)?;
        self.send(control(index))?;
        self.deck(deck)
    }

    /// Sends a command for a deck whose track has a beat grid (or BPM).
    fn on_grid(&self, deck: u8, control: impl FnOnce(usize) -> Control) -> Result<DeckState> {
        let index = self.require_grid(deck)?;
        self.send(control(index))?;
        self.deck(deck)
    }

    pub fn play(&self, deck: u8) -> Result<DeckState> {
        self.on_track(deck, Control::Play)
    }

    pub fn pause(&self, deck: u8) -> Result<DeckState> {
        self.on_track(deck, Control::Pause)
    }

    /// The CUE button: see docs/specs/decks.md.
    pub fn cue(&self, deck: u8, press: bool) -> Result<DeckState> {
        self.on_track(deck, |deck| Control::Cue { deck, press })
    }

    /// Hot cue pad `slot` (0 = A … 7 = H).
    pub fn hot_cue(&self, deck: u8, slot: u8, press: bool) -> Result<DeckState> {
        if slot as usize >= HOT_CUES {
            return Err(Error::InvalidInput(format!(
                "hot cue slot must be 0–7 (A–H), not {slot}"
            )));
        }
        let slot = slot as usize;
        self.on_track(deck, |deck| Control::HotCue { deck, slot, press })
    }

    /// Calls a memory cue or loop: jump there and pause.
    pub fn memory_cue(&self, deck: u8, target: MemoryCueTarget) -> Result<DeckState> {
        let index = self.require_track(deck)?;
        if let MemoryCueTarget::Index(i) = target {
            let count = self.inner.shared.snapshot().decks[index]
                .memory_cues
                .iter()
                .flatten()
                .count();
            if i as usize >= count {
                return Err(Error::InvalidInput(format!(
                    "deck {deck}'s track has {count} memory cues; there's no index {i}"
                )));
            }
        }
        let target = match target {
            MemoryCueTarget::Index(i) => MemoryTarget::Index(i as usize),
            MemoryCueTarget::Next => MemoryTarget::Next,
            MemoryCueTarget::Previous => MemoryTarget::Previous,
        };
        self.on_track(deck, |deck| Control::MemoryCue { deck, target })
    }

    pub fn seek(&self, deck: u8, position_us: i64) -> Result<DeckState> {
        let seconds = position_us.max(0) as f64 / 1e6;
        self.on_track(deck, |deck| Control::Seek { deck, seconds })
    }

    pub fn beat_jump(&self, deck: u8, beats: f64) -> Result<DeckState> {
        if !beats.is_finite() {
            return Err(Error::InvalidInput("beats must be a number".into()));
        }
        self.on_grid(deck, |deck| Control::BeatJump { deck, beats })
    }

    /// Auto beat loop of `beats` beats (1/64–512); the same size again
    /// exits the loop.
    pub fn beat_loop(&self, deck: u8, beats: f64) -> Result<DeckState> {
        if !(MIN_LOOP_BEATS..=MAX_LOOP_BEATS).contains(&beats) {
            return Err(Error::InvalidInput(
                "loop length must be 1/64 to 512 beats".into(),
            ));
        }
        self.on_grid(deck, |deck| Control::BeatLoop { deck, beats })
    }

    pub fn loop_in(&self, deck: u8) -> Result<DeckState> {
        self.on_track(deck, Control::LoopIn)
    }

    pub fn loop_out(&self, deck: u8) -> Result<DeckState> {
        self.on_track(deck, Control::LoopOut)
    }

    pub fn reloop(&self, deck: u8) -> Result<DeckState> {
        self.on_track(deck, Control::Reloop)
    }

    /// Doubles or halves the active loop, anchored at loop-in.
    pub fn loop_resize(&self, deck: u8, double: bool) -> Result<DeckState> {
        self.on_track(deck, |deck| Control::LoopResize { deck, double })
    }

    pub fn loop_exit(&self, deck: u8) -> Result<DeckState> {
        self.on_track(deck, Control::LoopExit)
    }

    /// Refuses tempo changes on a deck that follows the sync master.
    fn check_tempo_is_free(&self, deck: u8, index: usize) -> Result<()> {
        let snapshot = self.inner.shared.snapshot();
        if snapshot.decks[index].following {
            return Err(Error::FollowsMaster {
                deck,
                master: if index == 0 { 2 } else { 1 },
            });
        }
        Ok(())
    }

    /// Tempo in percent, clamped to the deck's range and to 40–499 BPM.
    /// Refused while the deck follows a sync master.
    pub fn set_tempo(&self, deck: u8, percent: f64) -> Result<DeckState> {
        let index = Self::deck_index(deck)?;
        if !percent.is_finite() {
            return Err(Error::InvalidInput("tempo must be a number".into()));
        }
        self.check_tempo_is_free(deck, index)?;
        self.send(Control::SetTempo {
            deck: index,
            percent,
        })?;
        self.deck(deck)
    }

    /// Sets the tempo by BPM, like typing it into rekordbox's BPM field. It
    /// must be within 40–499 BPM and reachable within the tempo range.
    pub fn set_bpm(&self, deck: u8, bpm: f64) -> Result<DeckState> {
        let index = self.require_grid(deck)?;
        self.check_tempo_is_free(deck, index)?;
        if !(MIN_BPM..=MAX_BPM).contains(&bpm) {
            return Err(Error::InvalidInput(format!(
                "BPM must be between {MIN_BPM} and {MAX_BPM}"
            )));
        }
        let state = self.deck(deck)?;
        let original = state.original_bpm.ok_or(Error::NoBeatGrid(deck))?;
        let percent = (bpm / original - 1.0) * 100.0;
        if percent.abs() > state.tempo_range + 1e-9 {
            return Err(Error::InvalidInput(format!(
                "{bpm:.2} BPM is {percent:+.2} % from the track's {original:.2} BPM, outside the ±{} % tempo range",
                state.tempo_range
            )));
        }
        self.send(Control::SetTempo {
            deck: index,
            percent,
        })?;
        self.deck(deck)
    }

    /// Back to the track's own tempo (0 %). Refused while following a
    /// sync master.
    pub fn tempo_reset(&self, deck: u8) -> Result<DeckState> {
        self.set_tempo(deck, 0.0)
    }

    /// Tempo fader range: 6, 10, 16 or 100 (WIDE) percent.
    pub fn set_tempo_range(&self, deck: u8, range: f64) -> Result<DeckState> {
        let index = Self::deck_index(deck)?;
        if !TEMPO_RANGES.contains(&range) {
            return Err(Error::InvalidInput(format!(
                "tempo range must be 6, 10, 16 or 100, not {range}"
            )));
        }
        self.send(Control::SetTempoRange { deck: index, range })?;
        self.deck(deck)
    }

    /// Pitch bend: speeds the deck up (`forward`) or slows it down by
    /// [`PITCH_BEND`] while pressed, playing only. On a beat-synced deck it
    /// drops sync to BPM only until sync is turned off and on again.
    pub fn pitch_bend(&self, deck: u8, direction: BendDirection, press: bool) -> Result<DeckState> {
        let direction = match direction {
            BendDirection::Back => -1,
            BendDirection::Forward => 1,
        };
        self.on_track(deck, |deck| Control::PitchBend {
            deck,
            direction,
            press,
        })
    }

    /// Sync on (`Beat`: tempo and phase, or `Bpm`: tempo only) or `Off`.
    pub fn sync(&self, deck: u8, mode: SyncMode) -> Result<DeckState> {
        let index = if mode == SyncMode::Off {
            Self::deck_index(deck)?
        } else {
            self.require_grid(deck)?
        };
        self.send(Control::Sync { deck: index, mode })?;
        self.deck(deck)
    }

    pub fn set_master(&self, deck: u8) -> Result<DeckState> {
        self.on_track(deck, Control::SetMaster)
    }

    /// Quantize for one deck.
    pub fn set_quantize(&self, deck: u8, on: bool) -> Result<DeckState> {
        let index = Self::deck_index(deck)?;
        self.send(Control::Quantize { deck: index, on })?;
        self.deck(deck)
    }

    /// "Gate playback during Pause" for hot cues, both decks.
    pub fn set_hot_cue_gate(&self, on: bool) -> Result<PlayerStatus> {
        self.send(Control::HotCueGate(on))?;
        Ok(self.status())
    }

    /// Changes mixer settings. `deck` (1 or 2) is required with channel
    /// settings (volume, EQ, kills, auto gain).
    pub fn set_mixer(&self, update: MixerSettingsUpdate) -> Result<MixerState> {
        let deck = match update.deck {
            Some(deck) => Some(Self::deck_index(deck)?),
            None => None,
        };
        let per_deck = [
            update.volume,
            update.eq_low,
            update.eq_mid,
            update.eq_high,
            update.auto_gain_db,
        ]
        .iter()
        .any(Option::is_some)
            || [update.kill_low, update.kill_mid, update.kill_high]
                .iter()
                .any(Option::is_some);
        if per_deck && deck.is_none() {
            return Err(Error::InvalidInput(
                "say which deck (1 or 2) the channel settings are for".into(),
            ));
        }
        let finite = [
            update.volume,
            update.eq_low,
            update.eq_mid,
            update.eq_high,
            update.auto_gain_db,
            update.crossfader,
            update.master_volume,
        ];
        if finite.iter().flatten().any(|v| !v.is_finite()) {
            return Err(Error::InvalidInput("mixer values must be numbers".into()));
        }
        self.send(Control::Mixer(MixerUpdate {
            deck,
            volume: update.volume,
            eq: [update.eq_low, update.eq_mid, update.eq_high],
            kill: [update.kill_low, update.kill_mid, update.kill_high],
            auto_gain_db: update.auto_gain_db,
            crossfader: update.crossfader,
            master_volume: update.master_volume,
        }))?;
        Ok(self.mixer())
    }
}

/// Hot cues by slot and memory cues in position order, from library cues.
fn cue_tables(cues: &[Cue]) -> ([Option<HotCue>; HOT_CUES], [Option<HotCue>; MEMORY_CUES]) {
    let to_cue = |c: &Cue| {
        let start = c.start_us.max(0) as f64 / 1e6;
        HotCue {
            start,
            end: c.end_us.map(|e| e as f64 / 1e6).filter(|&e| e > start),
        }
    };
    let mut hot_cues = [None; HOT_CUES];
    let mut memory: Vec<HotCue> = Vec::new();
    for c in cues {
        match c.hot_cue {
            Some(slot) if (slot as usize) < HOT_CUES => hot_cues[slot as usize] = Some(to_cue(c)),
            Some(_) => {}
            None => memory.push(to_cue(c)),
        }
    }
    memory.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut memory_cues = [None; MEMORY_CUES];
    for (slot, cue) in memory_cues.iter_mut().zip(memory) {
        *slot = Some(cue);
    }
    (hot_cues, memory_cues)
}

/// Which memory cue to call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryCueTarget {
    /// 0 = the earliest.
    Index(u8),
    /// The first one after the playhead.
    Next,
    /// The last one before the playhead.
    Previous,
}

/// Mixer changes; `None` leaves a setting alone.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MixerSettingsUpdate {
    /// 1 or 2, for the channel settings below.
    pub deck: Option<u8>,
    pub volume: Option<f32>,
    pub eq_low: Option<f32>,
    pub eq_mid: Option<f32>,
    pub eq_high: Option<f32>,
    pub kill_low: Option<bool>,
    pub kill_mid: Option<bool>,
    pub kill_high: Option<bool>,
    pub auto_gain_db: Option<f32>,
    pub crossfader: Option<f32>,
    pub master_volume: Option<f32>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Relaxed);
        if let Some(output) = &self.output {
            let _ = lock(output).send(OutputMessage::Stop);
        }
        for thread in lock(&self.threads).drain(..) {
            let _ = thread.join();
        }
        for meta in lock(&self.shared.decks).iter() {
            if let Some(pcm) = &meta.pcm {
                pcm.cancel();
            }
        }
    }
}
