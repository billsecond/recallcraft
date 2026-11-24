//! When a block of sound was heard, on the system clock.
//!
//! A device's clock and the computer's differ a little (some parts in 100 000), and a device
//! hands sound over a block at a time, a little late. Blocks are numbered one after another from
//! when the device started, so they join up exactly; when that count has drifted from the
//! system clock by more than [`TOLERANCE_US`] (or the device stopped and started again), it
//! starts again from the system clock. Both streams are kept within that of the system clock,
//! and so of each other.

use crate::RATE;

/// How far a device's count may drift from the system clock.
pub const TOLERANCE_US: u64 = 80_000;

#[derive(Clone, Debug, Default)]
pub struct Placer {
    start_us: Option<u64>,
    frames: u64,
}

impl Placer {
    pub fn new() -> Placer {
        Placer::default()
    }

    /// When a block of `frames` samples that has just arrived (at `now_us`) started.
    pub fn place(&mut self, frames: u64, now_us: u64) -> u64 {
        let dur = frames * 1_000_000 / u64::from(RATE);
        let ideal = now_us.saturating_sub(dur);
        let start = match self.start_us {
            Some(s) => {
                let expected = s + self.frames * 1_000_000 / u64::from(RATE);
                if expected.abs_diff(ideal) > TOLERANCE_US {
                    self.start_us = Some(ideal);
                    self.frames = 0;
                    ideal
                } else {
                    expected
                }
            }
            None => {
                self.start_us = Some(ideal);
                self.frames = 0;
                ideal
            }
        };
        self.frames += frames;
        start
    }

    /// The device stopped: the next block starts afresh.
    pub fn reset(&mut self) {
        self.start_us = None;
        self.frames = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_join_up_and_drift_is_put_right() {
        let mut p = Placer::new();
        let t0 = 1_000_000_000u64;
        // 10 ms blocks, arriving a little unevenly.
        assert_eq!(p.place(480, t0 + 10_000), t0);
        assert_eq!(p.place(480, t0 + 21_000), t0 + 10_000);
        assert_eq!(p.place(480, t0 + 29_500), t0 + 20_000);
        // A device a little fast: after a long while it's put right, by less than the tolerance.
        let mut p = Placer::new();
        let mut now = t0;
        let mut last = 0;
        let mut jumps = 0;
        for _ in 0..100_000 {
            // 480 samples every 9.99 ms: the device's clock is 0.1 % fast.
            now += 9_990;
            let s = p.place(480, now);
            if last != 0 && s != last + 10_000 {
                jumps += 1;
                assert!(s.abs_diff(now - 10_000) <= 1, "put right to the system clock");
            }
            last = s;
            assert!(s.abs_diff(now - 10_000) <= TOLERANCE_US + 10_000);
        }
        assert!(jumps > 5 && jumps < 20, "{jumps}");
        // Nothing for a minute (the device stopped): starts from the clock again.
        let s = p.place(480, now + 60_000_000);
        assert_eq!(s, now + 60_000_000 - 10_000);
    }
}
