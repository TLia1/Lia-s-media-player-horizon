//! Bounded stereo PCM transport. The callback never locks, allocates, or logs.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
};

pub const SAMPLE_RATE: u32 = 48_000;
/// One atomic packed stereo frame per slot. Unique endpoints enforce SPSC ownership
/// without unsafe code; release/acquire publishes data and reclaimed slots.
pub struct PcmRing {
    slots: Box<[AtomicU32]>,
    read: AtomicUsize,
    write: AtomicUsize,
}
pub struct PcmProducer(Arc<PcmRing>);
pub struct PcmConsumer(Arc<PcmRing>);
impl PcmRing {
    pub fn bounded(frames: usize) -> (PcmProducer, PcmConsumer) {
        assert!(frames > 0 && frames <= SAMPLE_RATE as usize * 10);
        let ring = Arc::new(Self {
            slots: (0..frames + 1).map(|_| AtomicU32::new(0)).collect(),
            read: AtomicUsize::new(0),
            write: AtomicUsize::new(0),
        });
        (PcmProducer(ring.clone()), PcmConsumer(ring))
    }
}
impl PcmProducer {
    pub fn push(&mut self, frame: [i16; 2]) -> bool {
        let w = self.0.write.load(Ordering::Relaxed);
        let next = (w + 1) % self.0.slots.len();
        if next == self.0.read.load(Ordering::Acquire) {
            return false;
        }
        self.0.slots[w].store(
            frame[0] as u16 as u32 | ((frame[1] as u16 as u32) << 16),
            Ordering::Relaxed,
        );
        self.0.write.store(next, Ordering::Release);
        true
    }
}
impl PcmConsumer {
    pub fn pop(&mut self) -> Option<[f32; 2]> {
        let r = self.0.read.load(Ordering::Relaxed);
        if r == self.0.write.load(Ordering::Acquire) {
            return None;
        }
        let value = self.0.slots[r].load(Ordering::Relaxed);
        self.0
            .read
            .store((r + 1) % self.0.slots.len(), Ordering::Release);
        Some([
            value as u16 as i16 as f32 / 32768.,
            (value >> 16) as u16 as i16 as f32 / 32768.,
        ])
    }
}
/// Shared controls and the count of source PCM frames actually consumed, excluding silence.
pub struct AudioClock {
    pub played_frames: AtomicU64,
    paused: AtomicBool,
    gain: AtomicU32,
    pub failed: AtomicBool,
}
impl Default for AudioClock {
    fn default() -> Self {
        Self {
            played_frames: AtomicU64::new(0),
            paused: AtomicBool::new(true),
            gain: AtomicU32::new(1f32.to_bits()),
            failed: AtomicBool::new(false),
        }
    }
}
impl AudioClock {
    pub fn seconds(&self) -> f64 {
        self.played_frames.load(Ordering::Acquire) as f64 / SAMPLE_RATE as f64
    }
    pub fn pause(&self, paused: bool) {
        self.paused.store(paused, Ordering::Release);
    }
    pub fn set_gain(&self, value: f32) {
        if value.is_finite() {
            self.gain
                .store(value.clamp(0., 1.).to_bits(), Ordering::Relaxed);
        }
    }
}
pub struct Gain {
    current: f32,
}
impl Default for Gain {
    fn default() -> Self {
        Self { current: 0. }
    }
}
impl Gain {
    pub fn next(&mut self, target: f32, rate: u32) -> f32 {
        // A 5 ms ramp avoids discontinuities when starting, muting or changing volume.
        let step = 1. / (rate.max(1) as f32 * 0.005);
        self.current += (target - self.current).clamp(-step, step);
        self.current
    }
}
/// Stateful linear resampling from 48 kHz to the device rate. Underflow never advances
/// the media clock. Held interpolation samples survive callback boundaries and pause.
pub struct AudioRenderer {
    pcm: PcmConsumer,
    pub clock: Arc<AudioClock>,
    current: Option<[f32; 2]>,
    next: Option<[f32; 2]>,
    phase: f64,
    gain: Gain,
}
impl AudioRenderer {
    pub fn new(pcm: PcmConsumer, clock: Arc<AudioClock>) -> Self {
        Self {
            pcm,
            clock,
            current: None,
            next: None,
            phase: 0.,
            gain: Gain::default(),
        }
    }
    pub fn frame(&mut self, rate: u32) -> [f32; 2] {
        if self.clock.paused.load(Ordering::Acquire) {
            return [0.; 2];
        }
        while self.phase >= 1. {
            if self.current.take().is_some() {
                self.clock.played_frames.fetch_add(1, Ordering::Release);
            }
            self.current = self.next.take().or_else(|| self.pcm.pop());
            self.phase -= 1.;
        }
        if self.current.is_none() {
            self.current = self.pcm.pop();
        }
        let Some(a) = self.current else {
            self.phase = 0.;
            return [0.; 2];
        };
        if self.next.is_none() {
            self.next = self.pcm.pop();
        }
        let b = self.next.unwrap_or(a);
        let gain = self.gain.next(
            f32::from_bits(self.clock.gain.load(Ordering::Relaxed)),
            rate,
        );
        let t = self.phase as f32;
        self.phase += SAMPLE_RATE as f64 / rate.max(1) as f64;
        [
            (a[0] + (b[0] - a[0]) * t) * gain,
            (a[1] + (b[1] - a[1]) * t) * gain,
        ]
    }
    pub fn render<T: cpal::SizedSample + cpal::FromSample<f32>>(
        &mut self,
        data: &mut [T],
        channels: usize,
        rate: u32,
    ) {
        for output in data.chunks_mut(channels.max(1)) {
            let [left, right] = self.frame(rate);
            let mono = (left + right) * 0.5;
            for (ch, sample) in output.iter_mut().enumerate() {
                *sample = T::from_sample(if channels == 1 {
                    mono
                } else {
                    match ch {
                        0 => left,
                        1 => right,
                        _ => 0.,
                    }
                });
            }
        }
    }
}
/// Kept on the player thread: some CPAL backends do not permit sending a Stream.
pub trait AudioSink {
    fn poll(&mut self) {}
}
pub type AudioFactory =
    Arc<dyn Fn(PcmConsumer, Arc<AudioClock>) -> Result<Box<dyn AudioSink>, String> + Send + Sync>;
pub struct CpalAudioSink {
    _stream: cpal::Stream,
}
impl AudioSink for CpalAudioSink {}
impl CpalAudioSink {
    pub fn factory() -> AudioFactory {
        Arc::new(|pcm, clock| Self::open(pcm, clock).map(|s| Box::new(s) as Box<dyn AudioSink>))
    }
    pub fn open(pcm: PcmConsumer, clock: Arc<AudioClock>) -> Result<Self, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("No audio output device")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let config: cpal::StreamConfig = supported.clone().into();
        let renderer = AudioRenderer::new(pcm, clock.clone());
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => Self::build::<f32>(&device, &config, renderer, clock),
            cpal::SampleFormat::I16 => Self::build::<i16>(&device, &config, renderer, clock),
            cpal::SampleFormat::U16 => Self::build::<u16>(&device, &config, renderer, clock),
            cpal::SampleFormat::F64 => Self::build::<f64>(&device, &config, renderer, clock),
            cpal::SampleFormat::I32 => Self::build::<i32>(&device, &config, renderer, clock),
            cpal::SampleFormat::U32 => Self::build::<u32>(&device, &config, renderer, clock),
            f => return Err(format!("Unsupported audio format: {f:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self { _stream: stream })
    }
    fn build<T: cpal::SizedSample + cpal::FromSample<f32>>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        mut renderer: AudioRenderer,
        clock: Arc<AudioClock>,
    ) -> Result<cpal::Stream, cpal::BuildStreamError> {
        let channels = config.channels as usize;
        let rate = config.sample_rate.0;
        device.build_output_stream(
            config,
            move |data: &mut [T], _| renderer.render(data, channels, rate),
            move |_| {
                clock.failed.store(true, Ordering::Release);
            },
            None,
        )
    }
}
