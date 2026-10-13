//! Low-latency audio output. Capture boards deliver interleaved stereo i16
//! at the DS/3DS native rate, which is resampled to the output device rate.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// 67027964 Hz / 2048, the DS and 3DS audio rate.
pub const SOURCE_RATE: f64 = 67_027_964.0 / 2048.0;

/// Buffered audio above this is dropped to keep latency down.
const MAX_BUFFERED_SECONDS: f64 = 0.10;
/// After an underrun, wait for this much audio before resuming.
const RESUME_SECONDS: f64 = 0.03;

#[derive(Default)]
pub struct AudioQueue {
    samples: Mutex<VecDeque<[i16; 2]>>,
}

impl AudioQueue {
    pub fn push(&self, interleaved: &[i16]) {
        let mut q = self.samples.lock().unwrap();
        q.extend(interleaved.chunks_exact(2).map(|c| [c[0], c[1]]));
        let max = (SOURCE_RATE * MAX_BUFFERED_SECONDS) as usize;
        if q.len() > max {
            let excess = q.len() - max;
            q.drain(..excess);
        }
    }

    pub fn clear(&self) {
        self.samples.lock().unwrap().clear();
    }
}

pub struct AudioSettings {
    /// 0.0 ..= 2.0, stored as f32 bits.
    volume: AtomicU32,
    muted: AtomicBool,
}

impl AudioSettings {
    pub fn set_volume(&self, volume: f32) {
        self.volume.store(volume.clamp(0.0, 2.0).to_bits(), Ordering::Relaxed);
    }

    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    fn gain(&self) -> f32 {
        if self.muted.load(Ordering::Relaxed) { 0.0 } else { f32::from_bits(self.volume.load(Ordering::Relaxed)) }
    }
}

pub struct AudioOutput {
    pub queue: Arc<AudioQueue>,
    pub settings: Arc<AudioSettings>,
    stream: Option<cpal::Stream>,
}

pub fn output_device_names() -> Vec<String> {
    let host = cpal::default_host();
    let Ok(devices) = host.output_devices() else { return Vec::new() };
    devices.filter_map(|d| d.description().ok().map(|desc| desc.name().to_string())).collect()
}

impl AudioOutput {
    pub fn new() -> Self {
        Self {
            queue: Arc::new(AudioQueue::default()),
            settings: Arc::new(AudioSettings { volume: AtomicU32::new(1.0f32.to_bits()), muted: AtomicBool::new(false) }),
            stream: None,
        }
    }

    /// (Re)open the output stream on `device_name`, or the system default.
    pub fn open(&mut self, device_name: Option<&str>) -> Result<(), String> {
        self.stream = None;
        let host = cpal::default_host();
        let device = device_name
            .and_then(|name| {
                host.output_devices().ok()?.find(|d| d.description().is_ok_and(|desc| desc.name() == name))
            })
            .or_else(|| host.default_output_device())
            .ok_or("No audio output device")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let config = supported.config();
        let channels = config.channels as usize;
        let mut resampler = Resampler::new(self.queue.clone(), self.settings.clone(), config.sample_rate as f64);

        let err_fn = |e| eprintln!("audio stream error: {e}");
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(
                config,
                move |data: &mut [f32], _| resampler.fill(data, channels, |v| v),
                err_fn,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                config,
                move |data: &mut [i16], _| resampler.fill(data, channels, |v| (v * 32767.0) as i16),
                err_fn,
                None,
            ),
            other => return Err(format!("Unsupported audio sample format {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        self.stream = Some(stream);
        Ok(())
    }
}

/// Linear-interpolating resampler pulling from the shared queue.
struct Resampler {
    queue: Arc<AudioQueue>,
    settings: Arc<AudioSettings>,
    step: f64,
    pos: f64,
    prev: [f32; 2],
    curr: [f32; 2],
    starved: bool,
}

impl Resampler {
    fn new(queue: Arc<AudioQueue>, settings: Arc<AudioSettings>, output_rate: f64) -> Self {
        Self { queue, settings, step: SOURCE_RATE / output_rate, pos: 0.0, prev: [0.0; 2], curr: [0.0; 2], starved: true }
    }

    fn fill<T: Copy>(&mut self, data: &mut [T], channels: usize, convert: impl Fn(f32) -> T) {
        let gain = self.settings.gain();
        let mut q = self.queue.samples.lock().unwrap();
        if self.starved && q.len() as f64 >= SOURCE_RATE * RESUME_SECONDS {
            self.starved = false;
        }
        for frame in data.chunks_mut(channels) {
            let (l, r) = if self.starved {
                (0.0, 0.0)
            } else {
                self.pos += self.step;
                while self.pos >= 1.0 {
                    self.pos -= 1.0;
                    self.prev = self.curr;
                    match q.pop_front() {
                        Some([l, r]) => self.curr = [l as f32 / 32768.0, r as f32 / 32768.0],
                        None => {
                            self.starved = true;
                            self.curr = [0.0; 2];
                            break;
                        }
                    }
                }
                let t = self.pos as f32;
                (self.prev[0] + (self.curr[0] - self.prev[0]) * t, self.prev[1] + (self.curr[1] - self.prev[1]) * t)
            };
            let (l, r) = ((l * gain).clamp(-1.0, 1.0), (r * gain).clamp(-1.0, 1.0));
            match frame.len() {
                1 => frame[0] = convert((l + r) * 0.5),
                _ => {
                    frame[0] = convert(l);
                    frame[1] = convert(r);
                    for extra in &mut frame[2..] {
                        *extra = convert(0.0);
                    }
                }
            }
        }
    }
}
