//! Sound played faster or slower without changing its pitch (WSOLA): short overlapping slices
//! are taken from the input a little further apart (or closer together) than they're laid down,
//! each slice nudged to where it best matches the one before, so a voice stays a voice at 2×.

/// A slice: 32 ms at 48 kHz.
const N: usize = 1536;
/// Slices are laid down this far apart (half overlapping).
const HS: usize = N / 2;
/// How far a slice may be nudged to match: 8 ms either way.
const DELTA: usize = 384;

pub struct Stretch {
    speed: f32,
    input: Vec<f32>,
    /// The absolute position of `input[0]`.
    base: u64,
    /// Where the next slice would be taken from, unnudged.
    next: f64,
    /// Where the last slice would have gone on.
    natural: Option<u64>,
    ola: Vec<f32>,
    window: Vec<f32>,
}

impl Stretch {
    pub fn new(speed: f32) -> Stretch {
        let window = (0..N).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N as f32).cos()).collect();
        Stretch { speed: clamp_speed(speed), input: Vec::new(), base: 0, next: 0.0, natural: None, ola: vec![0.0; N], window }
    }

    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// Changes the speed (from the next slice).
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = clamp_speed(speed);
    }

    /// Starts afresh (a jump to elsewhere).
    pub fn reset(&mut self) {
        self.input.clear();
        self.base = 0;
        self.next = 0.0;
        self.natural = None;
        self.ola.iter_mut().for_each(|v| *v = 0.0);
    }

    fn passthrough(&self) -> bool {
        (self.speed - 1.0).abs() < 0.01 && self.input.is_empty() && self.natural.is_none()
    }

    /// Takes input; returns what's ready to play.
    pub fn push(&mut self, input: &[f32]) -> Vec<f32> {
        if self.passthrough() {
            return input.iter().map(|s| if s.is_finite() { *s } else { 0.0 }).collect();
        }
        self.input.extend(input.iter().map(|s| if s.is_finite() { *s } else { 0.0 }));
        let mut out = Vec::new();
        let end = self.base + self.input.len() as u64;
        loop {
            let nominal = self.next.round().max(0.0) as u64;
            if nominal + (DELTA + N) as u64 > end {
                break;
            }
            let k = match self.natural {
                Some(nat) if nat + N as u64 <= end => self.best_offset(nominal, nat),
                _ => 0,
            };
            let pos = (nominal as i64 + k).max(self.base as i64) as u64;
            let at = (pos - self.base) as usize;
            for (i, o) in self.ola.iter_mut().enumerate() {
                *o += self.input.get(at + i).copied().unwrap_or(0.0) * self.window.get(i).copied().unwrap_or(0.0);
            }
            out.extend_from_slice(self.ola.get(..HS).unwrap_or_default());
            self.ola.drain(..HS);
            self.ola.resize(N, 0.0);
            self.natural = Some(pos + HS as u64);
            self.next += HS as f64 * f64::from(self.speed);
            // What's behind both the next slice's earliest place and the natural continuation goes.
            let keep = (self.next.floor().max(0.0) as u64).saturating_sub(DELTA as u64).min(self.natural.unwrap_or(u64::MAX));
            if keep > self.base {
                let drop = ((keep - self.base) as usize).min(self.input.len());
                self.input.drain(..drop);
                self.base += drop as u64;
            }
        }
        out
    }

    /// The nudge (-DELTA..=DELTA) that best matches the slice at `nominal` to the natural
    /// continuation at `nat`: coarse first, then exact around the best.
    fn best_offset(&self, nominal: u64, nat: u64) -> i64 {
        let at = |abs: i64| -> f32 {
            if abs < self.base as i64 {
                return 0.0;
            }
            self.input.get((abs - self.base as i64) as usize).copied().unwrap_or(0.0)
        };
        let score = |k: i64, step: usize| -> f32 {
            let (mut c, mut e) = (0f32, 1e-9f32);
            let mut i = 0;
            while i < N / 2 {
                let a = at(nominal as i64 + k + i as i64);
                c += a * at(nat as i64 + i as i64);
                e += a * a;
                i += step;
            }
            c / e.sqrt()
        };
        let lo = -(DELTA as i64).max(self.base as i64 - nominal as i64);
        let lo = lo.max(-(DELTA as i64));
        let mut best = (0i64, f32::NEG_INFINITY);
        let mut k = lo;
        while k <= DELTA as i64 {
            let s = score(k, 4);
            if s > best.1 {
                best = (k, s);
            }
            k += 2;
        }
        let (centre, mut top) = (best.0, f32::NEG_INFINITY);
        let mut fine = centre;
        for k in (centre - 2).max(lo)..=(centre + 2).min(DELTA as i64) {
            let s = score(k, 2);
            if s > top {
                top = s;
                fine = k;
            }
        }
        fine
    }
}

fn clamp_speed(s: f32) -> f32 {
    if s.is_finite() { s.clamp(0.5, 3.0) } else { 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize, hz: f32) -> Vec<f32> {
        (0..n).map(|i| (i as f32 * 2.0 * std::f32::consts::PI * hz / 48_000.0).sin() * 0.5).collect()
    }

    /// The main frequency of some sound, by counting rising zero crossings.
    fn pitch(x: &[f32]) -> f32 {
        let ups = x.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        ups as f32 * 48_000.0 / x.len() as f32
    }

    #[test]
    fn faster_is_shorter_at_the_same_pitch() {
        for speed in [1.5f32, 2.0, 0.75] {
            let mut s = Stretch::new(speed);
            let input = tone(48_000 * 4, 220.0);
            let mut out = Vec::new();
            for block in input.chunks(4800) {
                out.extend(s.push(block));
            }
            let expect = 48_000.0 * 4.0 / speed;
            assert!((out.len() as f32 - expect).abs() < expect * 0.05, "{speed}: {} vs {expect}", out.len());
            let p = pitch(&out[4800..out.len() - 4800]);
            assert!((p - 220.0).abs() < 6.0, "{speed}: pitch {p}");
            // No clicks: neighbouring samples stay close, as a smooth tone's do.
            let jump = out.windows(2).skip(N).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
            assert!(jump < 0.06, "{speed}: {jump}");
        }
    }

    #[test]
    fn normal_speed_passes_through_and_junk_is_safe() {
        let mut s = Stretch::new(1.0);
        let input = tone(1000, 440.0);
        assert_eq!(s.push(&input), input);
        let mut s = Stretch::new(f32::NAN);
        assert_eq!(s.speed(), 1.0);
        s.set_speed(99.0);
        assert_eq!(s.speed(), 3.0);
        let out = s.push(&[f32::NAN; 10_000]);
        assert!(out.iter().all(|v| v.is_finite()));
        s.reset();
        assert!(s.push(&[]).is_empty());
    }
}
