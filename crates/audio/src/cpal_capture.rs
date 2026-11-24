//! Outside Windows: the microphone through cpal (Core Audio on macOS, ALSA on Linux). There's no
//! built-in way there to record what the computer plays, so that isn't offered.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::capture::{Mic, Ready, Shared, Source};
use crate::devices::Device;
use crate::place::Placer;
use crate::{Block, RATE};

fn device_name(d: &cpal::Device) -> String {
    d.description().map(|n| n.name().to_string()).unwrap_or_else(|_| "Microphone".to_string())
}

pub fn microphones() -> Vec<Device> {
    match cpal::default_host().input_devices() {
        Ok(ds) => ds.map(|d| Device { id: String::new(), name: device_name(&d) }).collect(),
        Err(e) => {
            log::warn!("listing microphones: {e}");
            Vec::new()
        }
    }
}

pub fn default_microphone() -> Option<Device> {
    cpal::default_host().default_input_device().map(|d| Device { id: String::new(), name: device_name(&d) })
}

fn to_f32(data: &cpal::Data) -> Option<Vec<f32>> {
    use cpal::SampleFormat as F;
    Some(match data.sample_format() {
        F::F32 => data.as_slice::<f32>()?.to_vec(),
        F::I16 => data.as_slice::<i16>()?.iter().map(|s| f32::from(*s) / 32768.0).collect(),
        F::I32 => data.as_slice::<i32>()?.iter().map(|s| *s as f32 / 2_147_483_648.0).collect(),
        F::U16 => data.as_slice::<u16>()?.iter().map(|s| (f32::from(*s) - 32768.0) / 32768.0).collect(),
        _ => return None,
    })
}

/// A linear resampler to 48 kHz, for a device that runs at another rate.
struct Linear {
    step: f64,
    pos: f64,
    prev: f32,
}

impl Linear {
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

pub(crate) fn run(source: Source, tx: Sender<Block>, ready: Ready, shared: &Shared) {
    let Source::Mic(mic) = source else {
        let _ = ready.send(Err("the computer's sound can't be recorded on this system".to_string()));
        return;
    };
    let host = cpal::default_host();
    let device = match &mic {
        Mic::Named(name) => host.input_devices().ok().and_then(|mut ds| ds.find(|d| device_name(d) == *name)).or_else(|| host.default_input_device()),
        _ => host.default_input_device(),
    };
    let Some(device) = device else {
        let _ = ready.send(Err("there is no microphone".to_string()));
        return;
    };
    let name = device_name(&device);
    let config = match device.default_input_config() {
        Ok(c) => c,
        Err(e) => {
            let _ = ready.send(Err(format!("{name}: {e}")));
            return;
        }
    };
    let ch = usize::from(config.channels().max(1));
    let resampler = Arc::new(Mutex::new(Linear { step: f64::from(config.sample_rate().max(1)) / f64::from(RATE), pos: 0.0, prev: 0.0 }));
    let placer = Arc::new(Mutex::new(Placer::new()));
    let (level_tx, level_rx) = std::sync::mpsc::channel::<f32>();
    let stream = device.build_input_stream_raw(
        config.config(),
        config.sample_format(),
        move |data: &cpal::Data, _: &cpal::InputCallbackInfo| {
            let Some(raw) = to_f32(data) else { return };
            let mono = crate::to_mono(&raw, ch);
            let at48 = resampler.lock().unwrap_or_else(PoisonError::into_inner).run(&mono);
            if at48.is_empty() {
                return;
            }
            let _ = level_tx.send(crate::peak(&at48));
            let start_us = placer.lock().unwrap_or_else(PoisonError::into_inner).place(at48.len() as u64, recallcraft_model::time::now_us());
            let _ = tx.send(Block { stream: recallcraft_model::Stream::Mic, start_us, samples: at48 });
        },
        move |e| log::warn!("microphone: {e}"),
        None,
    );
    let stream = match stream {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(format!("{name}: {e}")));
            return;
        }
    };
    if let Err(e) = stream.play() {
        let _ = ready.send(Err(format!("{name}: {e}")));
        return;
    }
    let _ = ready.send(Ok((name, String::new())));
    while !shared.stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(50));
        for l in level_rx.try_iter() {
            shared.set_level(l);
            shared.heard();
        }
    }
    drop(stream);
}
