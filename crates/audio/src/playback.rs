//! The recording played back: from any moment, at any speed, through the system's speaker.
//!
//! A feeder thread reads the sound a tenth of a second ahead (decoding is slower than playing),
//! stretches it to the speed ([`crate::stretch`]), and queues it for the speaker's callback.
//! Played while it's being recorded ("live"), it never runs past what's been recorded: at speed,
//! it slows to normal once it has caught up.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use recallcraft_model::Stream;
use recallcraft_store::export::mix;
use recallcraft_store::ring::RingReader;

use crate::stretch::Stretch;
use crate::{AudioError, RATE};

/// How loud each side is in the mix (0 mutes it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mix {
    pub mic: f32,
    pub system: f32,
}

impl Default for Mix {
    fn default() -> Mix {
        Mix { mic: 1.0, system: 1.0 }
    }
}

/// Where the sound comes from.
pub trait Source: Send + 'static {
    /// `n` samples (48 kHz mono) from `at_us`.
    fn read(&mut self, at_us: u64, n: usize, mix: Mix) -> Vec<f32>;
    /// How far there is sound to play, if it's still being recorded.
    fn live_edge_us(&mut self) -> Option<u64>;
}

/// The recording on disk, as it's being written.
pub struct RingSource {
    reader: RingReader,
    /// The recording is still being made (so the latest moment is now, less what isn't written).
    pub live: bool,
}

impl RingSource {
    pub fn new(reader: RingReader, live: bool) -> RingSource {
        RingSource { reader, live }
    }
}

/// How far behind now the recording on disk is (it's written every second).
pub const WRITE_LAG_US: u64 = 1_500_000;

impl Source for RingSource {
    fn read(&mut self, at_us: u64, n: usize, m: Mix) -> Vec<f32> {
        let mut out = vec![0f32; n];
        if m.mic > 0.0 {
            for (o, s) in out.iter_mut().zip(self.reader.read(Stream::Mic, at_us, n, RATE)) {
                *o = s * m.mic;
            }
        }
        if m.system > 0.0 {
            for (o, s) in out.iter_mut().zip(self.reader.read(Stream::System, at_us, n, RATE)) {
                *o = mix(*o, s * m.system);
            }
        }
        out
    }

    fn live_edge_us(&mut self) -> Option<u64> {
        self.live.then(|| recallcraft_model::time::now_us().saturating_sub(WRITE_LAG_US))
    }
}

struct State {
    playing: bool,
    /// The next moment to read.
    feed_us: u64,
    /// The moment being heard.
    play_us: u64,
    speed: f32,
    /// Speed asked for (it drops to 1 when live playback catches up).
    asked_speed: f32,
    queue: VecDeque<f32>,
    end_us: Option<u64>,
    live: bool,
    caught_up: bool,
    finished: bool,
    generation: u64,
    quit: bool,
    volume: f32,
    mix: Mix,
    rate: u32,
}

/// Plays the recording.
pub struct Player {
    state: Arc<(Mutex<State>, Condvar)>,
    _stream: cpal::Stream,
    feeder: Option<JoinHandle<()>>,
}

fn lock(s: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    s.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 48 kHz to the speaker's rate, linearly (most speakers run at 48 kHz, and then it's a copy).
struct ToDevice {
    step: f64,
    pos: f64,
    prev: f32,
}

impl ToDevice {
    fn run(&mut self, input: &[f32]) -> Vec<f32> {
        if (self.step - 1.0).abs() < 1e-9 {
            return input.to_vec();
        }
        let n = input.len();
        let mut out = Vec::with_capacity((n as f64 / self.step) as usize + 2);
        while self.pos <= n as f64 - 1.0 {
            let i = self.pos.floor() as isize;
            let f = (self.pos - self.pos.floor()) as f32;
            let at = |k: isize| if k < 0 { self.prev } else { input.get(k as usize).copied().unwrap_or(0.0) };
            out.push(at(i) + (at(i + 1) - at(i)) * f);
            self.pos += self.step;
        }
        self.pos -= n as f64;
        if let Some(l) = input.last() {
            self.prev = *l;
        }
        out
    }
}

impl Player {
    pub fn new(source: impl Source) -> Result<Player, AudioError> {
        let device = cpal::default_host().default_output_device().ok_or_else(|| AudioError::Device("there is no speaker".to_string()))?;
        let config = device.default_output_config().map_err(|e| AudioError::Device(e.to_string()))?;
        let rate = config.sample_rate().max(8000);
        let channels = usize::from(config.channels().max(1));
        let state = Arc::new((
            Mutex::new(State {
                playing: false,
                feed_us: 0,
                play_us: 0,
                speed: 1.0,
                asked_speed: 1.0,
                queue: VecDeque::new(),
                end_us: None,
                live: false,
                caught_up: false,
                finished: false,
                generation: 0,
                quit: false,
                volume: 1.0,
                mix: Mix::default(),
                rate,
            }),
            Condvar::new(),
        ));
        let cb = state.clone();
        let fill = move |out: &mut [f32]| {
            let mut s = lock(&cb.0);
            let step_us = f64::from(s.speed) * 1_000_000.0 / f64::from(s.rate);
            let mut heard = 0u64;
            for frame in out.chunks_mut(channels) {
                let v = if s.playing { s.queue.pop_front() } else { None };
                if v.is_some() {
                    heard += 1;
                }
                let v = v.unwrap_or(0.0) * s.volume;
                frame.iter_mut().for_each(|o| *o = v);
            }
            s.play_us += (heard as f64 * step_us) as u64;
            if s.queue.len() < (s.rate / 5) as usize {
                cb.1.notify_all();
            }
        };
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                device.build_output_stream(config.config(), move |out: &mut [f32], _| fill(out), |e| log::warn!("speaker: {e}"), None)
            }
            other => {
                // Rare: a speaker that takes no floats. Its samples are made from floats.
                let mut scratch: Vec<f32> = Vec::new();
                match other {
                    cpal::SampleFormat::I16 => device.build_output_stream(
                        config.config(),
                        move |out: &mut [i16], _| {
                            scratch.resize(out.len(), 0.0);
                            fill(&mut scratch);
                            for (o, s) in out.iter_mut().zip(&scratch) {
                                *o = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                            }
                        },
                        |e| log::warn!("speaker: {e}"),
                        None,
                    ),
                    _ => return Err(AudioError::Device(format!("the speaker takes {other:?} samples"))),
                }
            }
        }
        .map_err(|e| AudioError::Device(e.to_string()))?;
        stream.play().map_err(|e| AudioError::Device(e.to_string()))?;
        let fs = state.clone();
        let feeder =
            std::thread::Builder::new().name("playback".into()).spawn(move || feed(source, &fs)).map_err(|e| AudioError::Device(e.to_string()))?;
        Ok(Player { state, _stream: stream, feeder: Some(feeder) })
    }

    fn with<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        let r = f(&mut lock(&self.state.0));
        self.state.1.notify_all();
        r
    }

    /// Plays from `at_us`, until `end_us` if given.
    pub fn play_from(&self, at_us: u64, end_us: Option<u64>) {
        self.with(|s| {
            s.generation += 1;
            s.queue.clear();
            s.feed_us = at_us;
            s.play_us = at_us;
            s.end_us = end_us;
            s.playing = true;
            s.finished = false;
            s.caught_up = false;
            s.speed = s.asked_speed;
        });
    }

    /// Goes to `at_us`, playing or not.
    pub fn seek(&self, at_us: u64) {
        self.with(|s| {
            s.generation += 1;
            s.queue.clear();
            s.feed_us = at_us;
            s.play_us = at_us;
            s.finished = false;
            s.caught_up = false;
            s.speed = s.asked_speed;
        });
    }

    pub fn set_playing(&self, on: bool) {
        self.with(|s| {
            s.playing = on;
            if on {
                s.finished = false;
            }
        });
    }

    pub fn is_playing(&self) -> bool {
        self.with(|s| s.playing)
    }

    /// The speed, 0.5 to 3; what's queued is played again at the new speed.
    pub fn set_speed(&self, speed: f32) {
        let speed = if speed.is_finite() { speed.clamp(0.5, 3.0) } else { 1.0 };
        self.with(|s| {
            s.asked_speed = speed;
            if !s.caught_up {
                s.speed = speed;
                s.generation += 1;
                s.queue.clear();
                s.feed_us = s.play_us;
            }
        });
    }

    /// The speed it's playing at (1 once live playback has caught up).
    pub fn speed(&self) -> f32 {
        self.with(|s| s.speed)
    }

    pub fn set_live(&self, live: bool) {
        self.with(|s| s.live = live);
    }

    pub fn set_end(&self, end_us: Option<u64>) {
        self.with(|s| s.end_us = end_us);
    }

    pub fn set_volume(&self, v: f32) {
        self.with(|s| s.volume = if v.is_finite() { v.clamp(0.0, 2.0) } else { 1.0 });
    }

    pub fn set_mix(&self, m: Mix) {
        self.with(|s| s.mix = m);
    }

    pub fn position_us(&self) -> u64 {
        self.with(|s| s.play_us)
    }

    /// It played to the end it was given.
    pub fn finished(&self) -> bool {
        self.with(|s| s.finished)
    }

    /// Live playback has caught up with the recording.
    pub fn caught_up(&self) -> bool {
        self.with(|s| s.caught_up)
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.with(|s| s.quit = true);
        if let Some(t) = self.feeder.take() {
            let _ = t.join();
        }
    }
}

/// Reads ahead while playing.
fn feed(mut source: impl Source, state: &(Mutex<State>, Condvar)) {
    let mut stretch = Stretch::new(1.0);
    let mut seen = u64::MAX;
    let mut to_device = ToDevice { step: 1.0, pos: 0.0, prev: 0.0 };
    loop {
        let (generation, at, speed, end, live, m, rate) = {
            let mut s = lock(&state.0);
            loop {
                if s.quit {
                    return;
                }
                let ahead = s.queue.len() as u64 * 1000 / u64::from(s.rate.max(1));
                if s.playing && !s.finished && ahead < 300 {
                    break;
                }
                s = state.1.wait_timeout(s, Duration::from_millis(50)).map(|(g, _)| g).unwrap_or_else(|e| e.into_inner().0);
            }
            (s.generation, s.feed_us, s.speed, s.end_us, s.live, s.mix, s.rate)
        };
        if generation != seen {
            seen = generation;
            stretch.reset();
            to_device = ToDevice { step: f64::from(RATE) / f64::from(rate), pos: 0.0, prev: 0.0 };
        }
        stretch.set_speed(speed);
        // A tenth of a second of what's heard: that much more of the recording at speed.
        let mut n = (f64::from(RATE) / 10.0 * f64::from(speed)) as usize;
        if let Some(end) = end {
            if at >= end {
                let mut s = lock(&state.0);
                if s.generation == generation && s.queue.is_empty() {
                    s.finished = true;
                    s.playing = false;
                }
                drop(s);
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            n = n.min(((end - at) * u64::from(RATE) / 1_000_000).max(1) as usize);
        }
        if live && let Some(edge) = source.live_edge_us() {
            let dur = n as u64 * 1_000_000 / u64::from(RATE);
            if at + dur > edge {
                let mut s = lock(&state.0);
                if s.generation == generation && (s.speed - 1.0).abs() > 0.01 {
                    // Caught up: on at normal speed.
                    s.speed = 1.0;
                    s.caught_up = true;
                }
                drop(s);
                if at + 100_000 > edge {
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                }
            }
        }
        let input = source.read(at, n, m);
        let out = to_device.run(&stretch.push(&input));
        let mut s = lock(&state.0);
        if s.generation != generation {
            continue;
        }
        s.queue.extend(out);
        s.feed_us = at + n as u64 * 1_000_000 / u64::from(RATE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_speaker_rate_is_met() {
        let mut d = ToDevice { step: 48_000.0 / 44_100.0, pos: 0.0, prev: 0.0 };
        let input = vec![0.25f32; 48_000];
        let out: Vec<f32> = input.chunks(480).flat_map(|c| d.run(c)).collect();
        assert!((out.len() as i64 - 44_100).abs() <= 2, "{}", out.len());
        assert!(out.iter().skip(1).all(|v| (v - 0.25).abs() < 1e-6));
    }
}
