//! RecallCraft's sound devices.
//!
//! - [`capture`]: the microphone and what the computer plays, each on a thread of its own,
//!   handed over as [`Block`]s: mono 48 kHz samples and the time (µs since 1970) of the first.
//!   On Windows through WASAPI: the microphone the settings or the call choose, and the
//!   computer's sound through a process loopback that leaves RecallCraft's own sound out (so a
//!   replay isn't recorded again). Elsewhere the microphone only, through cpal.
//! - [`place`]: when each block was heard, on the system clock, without drifting.
//! - [`playback`]: the recording played back at any speed ([`stretch`] keeps voices' pitch).
//!
//! Layer L3.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod capture;
pub mod devices;
pub mod place;
pub mod playback;
pub mod stretch;

#[cfg(target_os = "windows")]
mod wasapi_capture;

#[cfg(not(target_os = "windows"))]
mod cpal_capture;

use recallcraft_model::Stream;

/// Samples a second, for everything.
pub const RATE: u32 = 48_000;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("{0}")]
    Device(String),
    #[error("not on this system: {0}")]
    Unsupported(&'static str),
}

/// Some sound from a device.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub stream: Stream,
    /// When the first sample was heard, µs since 1970.
    pub start_us: u64,
    /// Mono, 48 kHz, -1 to 1.
    pub samples: Vec<f32>,
}

/// Interleaved samples made mono: the channels' average.
pub fn to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    let ch = channels.max(1);
    if ch == 1 {
        return samples.iter().map(|s| if s.is_finite() { *s } else { 0.0 }).collect();
    }
    samples
        .chunks_exact(ch)
        .map(|f| {
            let s = f.iter().map(|v| if v.is_finite() { *v } else { 0.0 }).sum::<f32>() / ch as f32;
            s.clamp(-1.0, 1.0)
        })
        .collect()
}

/// The loudest sample, 0 to 1.
pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0f32, |m, s| if s.is_finite() { m.max(s.abs()) } else { m }).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_are_averaged() {
        assert_eq!(to_mono(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
        assert_eq!(to_mono(&[0.2, f32::NAN], 1), vec![0.2, 0.0]);
        assert_eq!(to_mono(&[1.0, 1.0, 1.0], 2), vec![1.0], "a part frame is left out");
        assert_eq!(peak(&[-0.7, 0.2, f32::INFINITY]), 0.7);
    }
}
