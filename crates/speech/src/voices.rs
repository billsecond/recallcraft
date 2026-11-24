//! The others told apart by their voices: each utterance of the computer's sound gets a
//! fingerprint (the average shape of its spectrum and how much it moves, as cepstral
//! coefficients), and fingerprints close together are one voice. On a call each person has
//! their own microphone and room too, which makes their voices further apart still.
//!
//! It's a guess, shown as `Speaker 1`, `Speaker 2`: the owner names them, and a voice once named
//! is known again on later calls ([`KnownVoice`]).

use std::sync::Arc;

use rustfft::num_complex::Complex32;
use serde::{Deserialize, Serialize};

use crate::mel::{HOP, N_FFT};

/// Cepstral coefficients kept (the first, loudness, is left out).
const CEPS: usize = 16;
/// The fingerprint's length: the averages of the coefficients and the pitch, then their spreads.
pub const DIMS: usize = (CEPS + 1) * 2;
const BANDS: usize = 40;
/// Steps of 10 ms that must be speech for a fingerprint worth comparing.
pub const MIN_FRAMES: u32 = 60;
/// Closer than this, two fingerprints are one voice.
pub const SAME: f32 = 0.05;
/// Closer than this, a voice is one named before.
pub const KNOWN: f32 = 0.04;
/// The most fingerprints a voice's average is made of (so it follows a voice as it warms up).
const MAX_WEIGHT: f32 = 4000.0;
pub const MAX_VOICES: u8 = recallcraft_voices_max();

const fn recallcraft_voices_max() -> u8 {
    8
}

/// A voice's fingerprint.
#[derive(Clone, Debug, PartialEq)]
pub struct Fingerprint {
    pub v: Vec<f32>,
    /// Steps of 10 ms of speech it was made from.
    pub frames: u32,
}

/// Makes fingerprints.
pub struct Analyzer {
    filters: Vec<f32>,
    hann: Vec<f32>,
    fft: Arc<dyn rustfft::Fft<f32>>,
    dct: Vec<f32>,
}

impl Default for Analyzer {
    fn default() -> Analyzer {
        Analyzer::new()
    }
}

impl Analyzer {
    pub fn new() -> Analyzer {
        let hann = (0..N_FFT).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N_FFT as f32).cos()).collect();
        let fft = rustfft::FftPlanner::<f32>::new().plan_fft_forward(N_FFT);
        // An orthonormal DCT-II, coefficients 1..=CEPS, each weighted up a little with its
        // number (lifting the finer shape of the spectrum, which tells voices apart).
        let mut dct = vec![0f32; CEPS * BANDS];
        for k in 1..=CEPS {
            let lift = (k as f32).sqrt();
            for n in 0..BANDS {
                if let Some(d) = dct.get_mut((k - 1) * BANDS + n) {
                    *d = (2.0 / BANDS as f32).sqrt() * (std::f32::consts::PI * k as f32 * (n as f32 + 0.5) / BANDS as f32).cos() * lift;
                }
            }
        }
        Analyzer { filters: crate::mel::filters(BANDS), hann, fft, dct }
    }

    /// The fingerprint of some 16 kHz speech; None if there's too little speech in it.
    pub fn fingerprint(&self, sound: &[f32]) -> Option<Fingerprint> {
        let bins = N_FFT / 2 + 1;
        let mut frames: Vec<(f32, Vec<f32>)> = Vec::new();
        let mut buf = vec![Complex32::new(0.0, 0.0); N_FFT];
        let mut at = 0usize;
        while at + N_FFT <= sound.len() && frames.len() < 30_000 {
            let mut energy = 0f32;
            for (i, b) in buf.iter_mut().enumerate() {
                let s = sound.get(at + i).copied().unwrap_or(0.0);
                let s = if s.is_finite() { s } else { 0.0 };
                energy += s * s;
                *b = Complex32::new(s * self.hann.get(i).copied().unwrap_or(0.0), 0.0);
            }
            self.fft.process(&mut buf);
            let power: Vec<f32> = buf.iter().take(bins).map(|c| c.norm_sqr()).collect();
            let mel: Vec<f32> =
                self.filters.chunks_exact(bins).map(|row| (row.iter().zip(&power).map(|(w, p)| w * p).sum::<f32>() + 1e-9).ln()).collect();
            let db = 10.0 * (energy / N_FFT as f32 + 1e-10).log10();
            frames.push((db, mel));
            at += HOP;
        }
        let loudest = frames.iter().map(|(d, _)| *d).fold(f32::NEG_INFINITY, f32::max);
        let voiced: Vec<(usize, &Vec<f32>)> =
            frames.iter().enumerate().filter(|(_, (d, _))| *d > loudest - 25.0 && *d > -55.0).map(|(i, (_, m))| (i, m)).collect();
        if (voiced.len() as u32) < MIN_FRAMES / 2 {
            return None;
        }
        // Each coefficient's average and spread, and the pitch's.
        let mut stats = vec![Stat::default(); FEATURES];
        for (i, mel) in &voiced {
            for (k, row) in self.dct.as_chunks::<BANDS>().0.iter().enumerate() {
                let c: f32 = row.iter().zip(mel.iter()).map(|(d, m)| d * m).sum();
                if let Some(s) = stats.get_mut(k) {
                    s.add(c);
                }
            }
            if let (Some(p), Some(s)) = (pitch(sound, i * HOP), stats.get_mut(CEPS)) {
                s.add(p.ln());
            }
        }
        let mut v: Vec<f32> = stats.iter().map(Stat::mean).collect();
        v.extend(stats.iter().map(Stat::spread));
        Some(Fingerprint { v, frames: voiced.len() as u32 })
    }
}

/// What's measured: the cepstral coefficients and the pitch.
const FEATURES: usize = CEPS + 1;
/// Fewer pitched steps than this, and the pitch isn't known.
const MIN_PITCHED: f32 = 20.0;

#[derive(Clone, Copy, Debug, Default)]
struct Stat {
    n: f32,
    sum: f32,
    sq: f32,
}

impl Stat {
    fn add(&mut self, x: f32) {
        if x.is_finite() {
            self.n += 1.0;
            self.sum += x;
            self.sq += x * x;
        }
    }

    fn mean(&self) -> f32 {
        if self.n < MIN_PITCHED.min(10.0) { 0.0 } else { self.sum / self.n }
    }

    /// The spread; 0 when there's too little to say (the pitch of a whisper).
    fn spread(&self) -> f32 {
        if self.n < 10.0 {
            return 0.0;
        }
        (self.sq / self.n - (self.sum / self.n).powi(2)).max(0.0).sqrt().max(1e-3)
    }
}

/// The pitch of the 40 ms from `at` (Hz), if it's voiced: the strongest repeat between 70 and
/// 400 Hz.
fn pitch(sound: &[f32], at: usize) -> Option<f32> {
    const LEN: usize = 640;
    let x = sound.get(at..at + LEN)?;
    let (lo, hi) = (16_000 / 400, 16_000 / 70);
    let mut best = (0usize, 0f32);
    for lag in lo..=hi {
        let (a, b) = (x.get(..LEN - lag)?, x.get(lag..)?);
        let (mut c, mut ea, mut eb) = (0f32, 1e-9f32, 1e-9f32);
        for (p, q) in a.iter().zip(b).step_by(2) {
            c += p * q;
            ea += p * p;
            eb += q * q;
        }
        let r = c / (ea * eb).sqrt();
        if r > best.1 {
            best = (lag, r);
        }
    }
    (best.1 >= 0.55 && best.0 > 0).then(|| 16_000.0 / best.0 as f32)
}

/// How far apart two fingerprints are: for each measure, how far apart the averages are for how
/// much it moves within each (the Bhattacharyya distance of two bell curves), averaged. The
/// pitch counts three times. A measure either doesn't know (no pitch) is left out.
pub fn distance(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.len() != DIMS {
        return f32::INFINITY;
    }
    let (mut sum, mut weight) = (0f32, 0f32);
    for i in 0..FEATURES {
        let (Some(&m1), Some(&m2), Some(&s1), Some(&s2)) = (a.get(i), b.get(i), a.get(FEATURES + i), b.get(FEATURES + i)) else { continue };
        if s1 <= 0.0 || s2 <= 0.0 {
            continue;
        }
        let (v1, v2) = ((s1 * s1).max(1e-4), (s2 * s2).max(1e-4));
        let v = (v1 + v2) / 2.0;
        let d = 0.125 * (m1 - m2).powi(2) / v + 0.5 * (v / (v1 * v2).sqrt()).ln();
        let w = if i == CEPS { 3.0 } else { 1.0 };
        sum += d * w;
        weight += w;
    }
    let d = if weight > 0.0 { sum / weight } else { f32::INFINITY };
    if d.is_finite() { d } else { f32::INFINITY }
}

#[derive(Clone, Debug)]
struct Voice {
    n: u8,
    mean: Vec<f32>,
    weight: f32,
}

/// The voices heard on one call.
#[derive(Clone, Debug, Default)]
pub struct VoiceBook {
    voices: Vec<Voice>,
    last: Option<(u8, u64)>,
}

impl VoiceBook {
    pub fn new() -> VoiceBook {
        VoiceBook::default()
    }

    /// How many voices so far.
    pub fn len(&self) -> usize {
        self.voices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.voices.is_empty()
    }

    /// A voice's average fingerprint.
    pub fn mean(&self, n: u8) -> Option<(&[f32], f32)> {
        self.voices.iter().find(|v| v.n == n).map(|v| (v.mean.as_slice(), v.weight))
    }

    /// Whose voice an utterance said at `at_us` is: 1, 2, …; None when it's too short to tell
    /// and nobody spoke just before.
    pub fn assign(&mut self, fp: &Fingerprint, at_us: u64) -> Option<u8> {
        let best = self.voices.iter().map(|v| (v.n, distance(&v.mean, &fp.v))).min_by(|a, b| a.1.total_cmp(&b.1));
        if fp.frames < MIN_FRAMES {
            // Too little to go on: whoever spoke just before, or a voice it's clearly like.
            let recent = self.last.filter(|(_, t)| at_us.saturating_sub(*t) < 4_000_000).map(|(n, _)| n);
            let close = best.filter(|(_, d)| *d < SAME * 0.8).map(|(n, _)| n);
            let n = close.or(recent)?;
            self.last = Some((n, at_us));
            return Some(n);
        }
        let n = match best {
            Some((n, d)) if d < SAME || self.voices.len() >= usize::from(MAX_VOICES) => n,
            _ => {
                let n = self.voices.len() as u8 + 1;
                self.voices.push(Voice { n, mean: fp.v.clone(), weight: 0.0 });
                n
            }
        };
        if let Some(v) = self.voices.iter_mut().find(|v| v.n == n) {
            let w = fp.frames as f32;
            let total = (v.weight + w).max(1.0);
            for (m, x) in v.mean.iter_mut().zip(&fp.v) {
                *m = (*m * v.weight + x * w) / total;
            }
            v.weight = total.min(MAX_WEIGHT);
        }
        self.last = Some((n, at_us));
        Some(n)
    }
}

/// A voice the owner named, known again on later calls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnownVoice {
    pub name: String,
    pub v: Vec<f32>,
    /// How much speech it was learned from (10 ms steps).
    pub frames: f32,
}

/// The known voice closest to `v`, if it's close enough to be them.
pub fn recognise<'a>(known: &'a [KnownVoice], v: &[f32]) -> Option<&'a KnownVoice> {
    known
        .iter()
        .filter(|k| k.v.len() == DIMS)
        .map(|k| (k, distance(&k.v, v)))
        .filter(|(_, d)| *d < KNOWN)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(k, _)| k)
}

/// Learns a voice the owner named: added, or its fingerprint moved towards this one.
pub fn learn(known: &mut Vec<KnownVoice>, name: &str, v: &[f32], frames: f32) {
    let name = name.trim();
    if name.is_empty() || v.len() != DIMS || v.iter().any(|x| !x.is_finite()) {
        return;
    }
    if let Some(k) = known.iter_mut().find(|k| k.name.eq_ignore_ascii_case(name) && k.v.len() == DIMS) {
        let total = (k.frames + frames).max(1.0);
        for (m, x) in k.v.iter_mut().zip(v) {
            *m = (*m * k.frames + x * frames) / total;
        }
        k.frames = total.min(MAX_WEIGHT * 4.0);
        return;
    }
    if known.len() < 500 {
        known.push(KnownVoice { name: name.chars().take(120).collect(), v: v.to_vec(), frames });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up voice: a buzz at `f0` through two resonances (`f1`, `f2`), loud and soft like
    /// syllables, with a little breath noise.
    pub(crate) fn voice(secs: f32, f0: f32, f1: f32, f2: f32, seed: u32) -> Vec<f32> {
        let n = (secs * 16_000.0) as usize;
        let mut x = seed.max(1);
        let mut out = Vec::with_capacity(n);
        let (mut y1, mut y2, mut z1, mut z2) = (0f32, 0f32, 0f32, 0f32);
        let res = |f: f32| {
            let r = 0.97f32;
            let w = 2.0 * std::f32::consts::PI * f / 16_000.0;
            (2.0 * r * w.cos(), -r * r)
        };
        let (a1, b1) = res(f1);
        let (a2, b2) = res(f2);
        for i in 0..n {
            let t = i as f32 / 16_000.0;
            let wobble = 1.0 + 0.04 * (t * 5.0).sin();
            let phase = (t * f0 * wobble).fract();
            let pulse = if phase < 0.08 { 1.0 } else { 0.0 };
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let breath = (x as f32 / u32::MAX as f32 - 0.5) * 0.05;
            let e = pulse + breath;
            let o1 = e + a1 * y1 + b1 * y2;
            y2 = y1;
            y1 = o1;
            let o2 = o1 + a2 * z1 + b2 * z2;
            z2 = z1;
            z1 = o2;
            let syl = 0.6 + 0.4 * (t * 2.0 * std::f32::consts::PI * 3.3).sin();
            out.push(o2 * 0.002 * syl);
        }
        out
    }

    #[test]
    fn two_voices_are_told_apart_and_each_is_known_again() {
        let a = Analyzer::new();
        let ann = |s: u32| a.fingerprint(&voice(3.0, 210.0, 750.0, 1900.0, s)).unwrap();
        let bob = |s: u32| a.fingerprint(&voice(3.0, 110.0, 520.0, 1250.0, s)).unwrap();
        let (a1, a2, b1, b2) = (ann(1), ann(2), bob(3), bob(4));
        assert!(a1.frames >= MIN_FRAMES);
        let same = distance(&a1.v, &a2.v).max(distance(&b1.v, &b2.v));
        let apart = distance(&a1.v, &b1.v);
        assert!(same < SAME && apart > SAME, "same {same}, apart {apart}");
        let mut book = VoiceBook::new();
        let order = [&a1, &b1, &a2, &b2, &a1];
        let got: Vec<Option<u8>> = order.iter().enumerate().map(|(i, f)| book.assign(f, i as u64 * 10_000_000)).collect();
        assert_eq!(got, vec![Some(1), Some(2), Some(1), Some(2), Some(1)]);
        assert_eq!(book.len(), 2);
        // Too short to tell: whoever just spoke.
        let short = Fingerprint { v: vec![9.0; DIMS], frames: 10 };
        assert_eq!(book.assign(&short, 41_000_000), Some(1));
        assert_eq!(book.assign(&short, 99_000_000), None);
        // Named, and known again.
        let mut known = Vec::new();
        learn(&mut known, "Ann", book.mean(1).unwrap().0, 300.0);
        learn(&mut known, "Bob", book.mean(2).unwrap().0, 300.0);
        learn(&mut known, "ann", &a2.v, 300.0);
        assert_eq!(known.len(), 2);
        assert_eq!(recognise(&known, &ann(7).v).map(|k| k.name.as_str()), Some("Ann"));
        assert_eq!(recognise(&known, &bob(8).v).map(|k| k.name.as_str()), Some("Bob"));
        assert!(a.fingerprint(&[0.0; 16_000]).is_none(), "silence");
        assert!(a.fingerprint(&[]).is_none());
        assert!(a.fingerprint(&[f32::NAN; 8000]).is_none());
        assert_eq!(distance(&[1.0], &[]), f32::INFINITY);
    }
}
