//! 48 kHz made 16 kHz: a low-pass filter (a windowed sinc, cut off below 8 kHz so nothing folds
//! back) and every third sample, a block at a time with nothing lost at the blocks' edges.

const TAPS: usize = 63;

fn design() -> Vec<f32> {
    // Cut off at 7.2 kHz of 48 kHz, Blackman window.
    let fc = 7200.0f64 / 48_000.0;
    let m = (TAPS - 1) as f64;
    let mut taps: Vec<f64> = (0..TAPS)
        .map(|i| {
            let x = i as f64 - m / 2.0;
            let sinc = if x == 0.0 { 2.0 * fc } else { (2.0 * std::f64::consts::PI * fc * x).sin() / (std::f64::consts::PI * x) };
            let w = 0.42 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / m).cos() + 0.08 * (4.0 * std::f64::consts::PI * i as f64 / m).cos();
            sinc * w
        })
        .collect();
    let sum: f64 = taps.iter().sum();
    for t in &mut taps {
        *t /= sum;
    }
    taps.into_iter().map(|t| t as f32).collect()
}

/// 48 kHz in, 16 kHz out.
pub struct Down3 {
    taps: Vec<f32>,
    history: Vec<f32>,
    phase: usize,
}

impl Default for Down3 {
    fn default() -> Down3 {
        Down3::new()
    }
}

impl Down3 {
    pub fn new() -> Down3 {
        Down3 { taps: design(), history: vec![0.0; TAPS - 1], phase: 0 }
    }

    /// Starts afresh (the sound before is gone).
    pub fn reset(&mut self) {
        self.history.iter_mut().for_each(|h| *h = 0.0);
        self.phase = 0;
    }

    pub fn push(&mut self, input: &[f32]) -> Vec<f32> {
        let mut buf = Vec::with_capacity(self.history.len() + input.len());
        buf.extend_from_slice(&self.history);
        buf.extend(input.iter().map(|s| if s.is_finite() { *s } else { 0.0 }));
        let mut out = Vec::with_capacity(input.len() / 3 + 1);
        let mut i = self.phase;
        while i < input.len() {
            // The output at input sample `i` uses `buf[i ..= i + TAPS - 1]`.
            if let Some(w) = buf.get(i..i + TAPS) {
                out.push(w.iter().zip(&self.taps).map(|(a, b)| a * b).sum());
            }
            i += 3;
        }
        self.phase = i - input.len();
        let keep = buf.len().saturating_sub(TAPS - 1);
        self.history = buf.get(keep..).unwrap_or_default().to_vec();
        self.history.resize(TAPS - 1, 0.0);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn speech_frequencies_pass_and_high_ones_dont() {
        let tone = |hz: f32| (0..48_000).map(move |i| (i as f32 * 2.0 * std::f32::consts::PI * hz / 48_000.0).sin() * 0.5).collect::<Vec<f32>>();
        let mut d = Down3::new();
        let low: Vec<f32> = tone(1000.0).chunks(441).flat_map(|c| d.push(c)).collect();
        assert!((low.len() as i64 - 16_000).abs() <= 1, "{}", low.len());
        assert!((rms(&low[1000..]) - 0.5 / 2f32.sqrt()).abs() < 0.02, "{}", rms(&low[1000..]));
        // Smooth across blocks: no sample jumps more than a sine of 1 kHz can at 16 kHz.
        assert!(low.windows(2).skip(100).all(|w| (w[1] - w[0]).abs() < 0.21));
        let mut d = Down3::new();
        let high: Vec<f32> = d.push(&tone(12_000.0));
        assert!(rms(&high[500..]) < 0.01, "a 12 kHz tone would fold back to 4 kHz: {}", rms(&high[500..]));
        assert!(Down3::new().push(&[]).is_empty());
        assert!(Down3::new().push(&[f32::NAN; 9]).iter().all(|v| v.is_finite()));
    }
}
