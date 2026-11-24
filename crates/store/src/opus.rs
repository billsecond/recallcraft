//! Opus by rusty-opus: an encoder for one mono stream at 48 kHz, 20 ms a packet, and a decoder
//! for what was kept. A packet read from a file that the decoder can't take (or that makes it
//! panic) is silence, never a crash.

use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::StoreError;

/// Samples a second everything is kept at.
pub const RATE: u32 = 48_000;
/// One packet: 20 ms at 48 kHz.
pub const FRAME: usize = 960;
/// The largest packet kept.
pub const MAX_PACKET: usize = 1275;

/// The encoder of one stream.
pub struct Encoder {
    enc: rusty_opus::OpusEncoder,
    buf: Vec<u8>,
}

impl Encoder {
    /// `speech` tunes it for voices (the microphone); otherwise for any sound.
    pub fn new(bitrate: u32, speech: bool) -> Result<Encoder, StoreError> {
        let app = if speech { rusty_opus::Application::Voip } else { rusty_opus::Application::Audio };
        let mut enc = rusty_opus::OpusEncoder::new(RATE as i32, 1, app).map_err(|e| StoreError::Codec(e.to_string()))?;
        enc.bitrate_bps = bitrate.clamp(12_000, 128_000) as i32;
        // Around the clock: a middle complexity keeps it cheap, and speech sounds as clear.
        enc.complexity = 5;
        Ok(Encoder { enc, buf: vec![0; 4000] })
    }

    /// One packet from one frame of [`FRAME`] samples (-1 to 1).
    pub fn encode(&mut self, frame: &[f32]) -> Result<Vec<u8>, StoreError> {
        let mut input = [0f32; FRAME];
        for (d, s) in input.iter_mut().zip(frame) {
            *d = if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 };
        }
        let enc = &mut self.enc;
        let buf = &mut self.buf;
        let n = catch_unwind(AssertUnwindSafe(|| enc.encode(&input, FRAME, buf)))
            .map_err(|_| StoreError::Codec("the encoder failed unexpectedly".into()))?
            .map_err(|e| StoreError::Codec(e.to_string()))?;
        Ok(self.buf.get(..n.min(MAX_PACKET)).unwrap_or_default().to_vec())
    }
}

/// A decoder, at 48 kHz or 16 kHz (what speech recognition hears).
pub struct Decoder {
    dec: Option<rusty_opus::OpusDecoder>,
    rate: u32,
    out: Vec<f32>,
}

impl Decoder {
    /// `rate` is 48000 or 16000 (anything else is 48000).
    pub fn new(rate: u32) -> Decoder {
        let rate = if rate == 16_000 { 16_000 } else { RATE };
        let dec = rusty_opus::OpusDecoder::new(rate as i32, 1).inspect_err(|e| log::warn!("opus decoder: {e}")).ok();
        Decoder { dec, rate, out: vec![0.0; 5760] }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Samples in one packet at this rate.
    pub fn frame(&self) -> usize {
        self.rate as usize / 50
    }

    /// One packet's sound; silence for one that can't be decoded.
    pub fn decode(&mut self, packet: &[u8]) -> Vec<f32> {
        let frame = self.frame();
        let Some(dec) = self.dec.as_mut() else { return vec![0.0; frame] };
        let out = &mut self.out;
        let got = catch_unwind(AssertUnwindSafe(|| dec.decode(packet, frame, out)));
        match got {
            Ok(Ok(n)) => {
                let mut v = self.out.get(..n.min(frame)).unwrap_or_default().to_vec();
                v.resize(frame, 0.0);
                v
            }
            Ok(Err(_)) => vec![0.0; frame],
            Err(_) => {
                log::warn!("the opus decoder failed on a packet; starting afresh");
                self.dec = rusty_opus::OpusDecoder::new(self.rate as i32, 1).ok();
                vec![0.0; frame]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tone_goes_through_and_junk_is_silence() {
        let mut e = Encoder::new(24_000, true).unwrap();
        let mut d = Decoder::new(48_000);
        let mut energy = 0.0f32;
        for k in 0..25 {
            let frame: Vec<f32> = (0..FRAME).map(|i| ((k * FRAME + i) as f32 * 0.05).sin() * 0.4).collect();
            let p = e.encode(&frame).unwrap();
            assert!(!p.is_empty() && p.len() < 400);
            let out = d.decode(&p);
            assert_eq!(out.len(), FRAME);
            if k > 5 {
                energy += out.iter().map(|s| s * s).sum::<f32>();
            }
        }
        assert!(energy > 100.0, "{energy}");
        let mut d16 = Decoder::new(16_000);
        assert_eq!(d16.decode(&[0xff, 0x00, 0x13]).len(), 320);
        assert_eq!(d16.decode(&[]).len(), 320);
        for junk in [vec![0u8; 1], vec![0xfc; 900], (0..=255u8).collect::<Vec<u8>>()] {
            assert_eq!(d.decode(&junk).len(), FRAME);
        }
    }
}
