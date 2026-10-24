//! Audio output through cpal.
//!
//! The emulator pushes mono samples into a shared queue; the device callback
//! pulls them. The app keeps the queue near a target fill level by nudging the
//! emulator's resampling rate (see [`Audio::rate_nudge`]), which avoids both
//! underruns and drift without resampling here.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

struct Shared {
    queue: VecDeque<f32>,
    volume: f32,
    last: f32,
    underruns: u64,
}

pub struct Audio {
    stream: Option<cpal::Stream>,
    shared: Arc<Mutex<Shared>>,
    pub sample_rate: u32,
    pub error: Option<String>,
    started: bool,
}

const DEFAULT_RATE: u32 = 48_000;

impl Audio {
    /// Opens the default output device. Failure leaves a silent sink so the
    /// emulator still runs (and still paces itself by wall-clock time).
    pub fn new() -> Self {
        let shared = Arc::new(Mutex::new(Shared {
            queue: VecDeque::with_capacity(16_384),
            volume: 1.0,
            last: 0.0,
            underruns: 0,
        }));
        match Self::open(shared.clone()) {
            Ok((stream, rate)) => Self {
                stream: Some(stream),
                shared,
                sample_rate: rate,
                error: None,
                started: false,
            },
            Err(e) => {
                log::warn!("audio disabled: {e}");
                Self {
                    stream: None,
                    shared,
                    sample_rate: DEFAULT_RATE,
                    error: Some(e),
                    started: false,
                }
            }
        }
    }

    fn open(shared: Arc<Mutex<Shared>>) -> Result<(cpal::Stream, u32), String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no audio output device")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let config = supported.config();
        let rate = config.sample_rate;
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, config, shared),
            SampleFormat::I16 => build::<i16>(&device, config, shared),
            SampleFormat::U16 => build::<u16>(&device, config, shared),
            SampleFormat::I32 => build::<i32>(&device, config, shared),
            other => return Err(format!("unsupported sample format {other:?}")),
        }?;
        Ok((stream, rate))
    }

    /// Starts playback. Browsers only allow this after a user gesture, so the
    /// app calls it once input has been seen.
    pub fn start(&mut self) {
        if self.started {
            return;
        }
        if let Some(s) = &self.stream {
            match s.play() {
                Ok(()) => self.started = true,
                Err(e) => self.error = Some(e.to_string()),
            }
        }
    }

    pub fn is_active(&self) -> bool {
        self.stream.is_some() && self.started
    }

    pub fn push(&self, samples: &[f32]) {
        let mut s = self.shared.lock().unwrap();
        s.queue.extend(samples.iter().copied());
        // Never let latency grow unbounded (e.g. after a stall).
        let max = self.sample_rate as usize / 2;
        if s.queue.len() > max {
            let excess = s.queue.len() - max;
            s.queue.drain(..excess);
        }
    }

    pub fn queued(&self) -> usize {
        self.shared.lock().unwrap().queue.len()
    }

    pub fn clear(&self) {
        self.shared.lock().unwrap().queue.clear();
    }

    pub fn set_volume(&self, volume: f32) {
        self.shared.lock().unwrap().volume = volume;
    }

    /// Resample-rate trim that steers the queue toward `target` samples:
    /// above target the emulator should produce slightly fewer samples.
    pub fn rate_nudge(&self, target: usize) -> f64 {
        if !self.is_active() {
            return 1.0;
        }
        let fill = self.queued() as f64;
        let error = (fill - target as f64) / target.max(1) as f64;
        1.0 - (error * 0.005).clamp(-0.005, 0.005)
    }
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    shared: Arc<Mutex<Shared>>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    device
        .build_output_stream(
            config,
            move |out: &mut [T], _| {
                let mut s = shared.lock().unwrap();
                let volume = s.volume;
                for frame in out.chunks_mut(channels) {
                    let v = match s.queue.pop_front() {
                        Some(v) => {
                            s.last = v;
                            v
                        }
                        None => {
                            // Underrun: fade the last sample out instead of clicking.
                            s.underruns += 1;
                            s.last *= 0.995;
                            s.last
                        }
                    };
                    let sample = T::from_sample((v * volume).clamp(-1.0, 1.0));
                    for ch in frame {
                        *ch = sample;
                    }
                }
            },
            |e| log::error!("audio stream error: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}
