//! The audio device, on a dedicated thread.
//!
//! The thread opens the default output with cpal and owns the stream, so the
//! engine handle stays `Send + Sync` whatever the platform's stream type is.
//! The audio callback renders straight from the [`Core`]: it `try_lock`s it
//! (never contended in realtime mode, so that's one atomic swap, and on the
//! off chance it is contended the callback outputs silence instead of
//! waiting). If no device can be opened, or the device disappears, the
//! thread keeps the core running silently on a timer so decks keep moving,
//! and retries the device every few seconds.

use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, FromSample, OutputCallbackInfo, SampleFormat, SizedSample};

use crate::core::{Core, MAX_BLOCK};
use crate::engine::Shared;
use crate::state::OutputInfo;
use crate::{Error, Result};

/// Requested device buffer: 256 frames is 5.3 ms at 48 kHz.
const PREFERRED_BUFFER: u32 = 256;
/// Block size of the silent fallback clock.
const SILENT_BLOCK: usize = 512;
/// How often to try the device again while running silently.
const RETRY: Duration = Duration::from_secs(5);

pub(crate) enum OutputMessage {
    Stop,
    /// The stream died (device unplugged, …): reopen.
    Failed(String),
}

/// Starts the output thread and waits until audio (or the silent fallback)
/// is running.
pub(crate) fn start(shared: Arc<Shared>) -> Result<(mpsc::Sender<OutputMessage>, JoinHandle<()>)> {
    let (tx, rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let thread_tx = tx.clone();
    let thread = std::thread::Builder::new()
        .name("tuneox-audio-out".into())
        .spawn(move || run(shared, rx, thread_tx, ready_tx))
        .map_err(|e| Error::Output(format!("couldn't start the audio thread: {e}")))?;
    // Opening a device can take a moment; don't hang if it takes forever.
    let _ = ready_rx.recv_timeout(Duration::from_secs(10));
    Ok((tx, thread))
}

fn run(
    shared: Arc<Shared>,
    rx: mpsc::Receiver<OutputMessage>,
    tx: mpsc::Sender<OutputMessage>,
    ready: mpsc::Sender<()>,
) {
    let mut ready = Some(ready);
    loop {
        if shared.shutdown.load(Ordering::Relaxed) {
            return;
        }
        let error = match open(&shared, tx.clone()) {
            Ok(stream) => {
                if let Some(ready) = ready.take() {
                    let _ = ready.send(());
                }
                match rx.recv() {
                    Ok(OutputMessage::Failed(message)) => {
                        drop(stream);
                        message
                    }
                    Ok(OutputMessage::Stop) | Err(_) => return,
                }
            }
            Err(message) => message,
        };
        let rate = lock(&shared.core).device_rate();
        *lock(&shared.output_info) = Some(OutputInfo {
            device: None,
            sample_rate: rate,
            channels: 2,
            latency_ms: 0.0,
            error: Some(error),
        });
        if let Some(ready) = ready.take() {
            let _ = ready.send(());
        }
        if run_silent(&shared, &rx) {
            return;
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Renders into nothing at real-time pace. Returns true when told to stop,
/// false when it's time to retry the device.
fn run_silent(shared: &Shared, rx: &mpsc::Receiver<OutputMessage>) -> bool {
    let started = Instant::now();
    let mut scratch = vec![0.0f32; SILENT_BLOCK * 2];
    let rate = lock(&shared.core).device_rate();
    let block = Duration::from_secs_f64(SILENT_BLOCK as f64 / rate as f64);
    let mut next = Instant::now();
    loop {
        next += block;
        let wait = next.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(OutputMessage::Stop) | Err(RecvTimeoutError::Disconnected) => return true,
            Ok(OutputMessage::Failed(_)) => {}
            Err(RecvTimeoutError::Timeout) => {}
        }
        let at_ms = shared.now_ms();
        lock(&shared.core).render_interleaved(&mut scratch, at_ms);
        if started.elapsed() > RETRY {
            return false;
        }
    }
}

fn open(
    shared: &Arc<Shared>,
    tx: mpsc::Sender<OutputMessage>,
) -> std::result::Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("no audio output device")?;
    let name = device
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_else(|_| "default output".to_string());
    let supported = device
        .default_output_config()
        .map_err(|e| format!("{name}: {e}"))?;
    let sample_rate = supported.sample_rate();
    let channels = supported.channels();
    let mut config = supported.config();
    if let cpal::SupportedBufferSize::Range { min, max } = *supported.buffer_size() {
        config.buffer_size = BufferSize::Fixed(PREFERRED_BUFFER.clamp(min, max));
    }
    lock(&shared.core).set_device_rate(sample_rate);

    let build = |config: cpal::StreamConfig| match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, config, shared, tx.clone()),
        SampleFormat::F64 => build::<f64>(&device, config, shared, tx.clone()),
        SampleFormat::I32 => build::<i32>(&device, config, shared, tx.clone()),
        SampleFormat::I16 => build::<i16>(&device, config, shared, tx.clone()),
        SampleFormat::U16 => build::<u16>(&device, config, shared, tx.clone()),
        other => Err(format!("unsupported sample format {other}")),
    };
    let stream = match build(config) {
        Ok(stream) => stream,
        // Some devices refuse a fixed buffer size; take their default.
        Err(_) if config.buffer_size != BufferSize::Default => build(cpal::StreamConfig {
            buffer_size: BufferSize::Default,
            ..config
        })?,
        Err(e) => return Err(e),
    };
    stream.play().map_err(|e| format!("{name}: {e}"))?;
    *lock(&shared.output_info) = Some(OutputInfo {
        device: Some(name),
        sample_rate,
        channels,
        latency_ms: 0.0,
        error: None,
    });
    Ok(stream)
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    shared: &Arc<Shared>,
    tx: mpsc::Sender<OutputMessage>,
) -> std::result::Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let callback_shared = shared.clone();
    device
        .build_output_stream::<T, _, _>(
            config,
            move |data: &mut [T], info: &OutputCallbackInfo| {
                let shared = &callback_shared;
                let timestamp = info.timestamp();
                let latency = timestamp.playback - timestamp.callback;
                shared
                    .latency_us
                    .store(latency.as_micros() as u64, Ordering::Relaxed);
                // What's rendered now is heard `latency` from now.
                let at_ms = shared.now_ms() + latency.as_secs_f64() * 1000.0;
                match shared.core.try_lock() {
                    Ok(mut core) => fill(&mut core, data, channels, at_ms),
                    Err(_) => data.fill(T::EQUILIBRIUM),
                }
            },
            move |error: cpal::Error| {
                use cpal::ErrorKind::*;
                if matches!(
                    error.kind(),
                    DeviceNotAvailable | StreamInvalidated | HostUnavailable
                ) {
                    let _ = tx.send(OutputMessage::Failed(error.to_string()));
                }
            },
            None,
        )
        .map_err(|e| e.to_string())
}

/// Renders `data` (interleaved, `channels` per frame) from the core.
fn fill<T>(core: &mut Core, data: &mut [T], channels: usize, at_ms: f64)
where
    T: SizedSample + FromSample<f32>,
{
    let channels = channels.max(1);
    let ms_per_frame = 1000.0 / core.device_rate() as f64;
    let mut done = 0;
    for chunk in data.chunks_mut(MAX_BLOCK * channels) {
        let frames = chunk.len() / channels;
        let block = core.render_block(frames, at_ms + done as f64 * ms_per_frame);
        for (frame, stereo) in chunk
            .chunks_exact_mut(channels)
            .zip(block.as_chunks::<2>().0)
        {
            if channels == 1 {
                frame[0] = T::from_sample((stereo[0] + stereo[1]) * 0.5);
            } else {
                frame[0] = T::from_sample(stereo[0]);
                frame[1] = T::from_sample(stereo[1]);
                for sample in &mut frame[2..] {
                    *sample = T::EQUILIBRIUM;
                }
            }
        }
        done += frames;
    }
}
