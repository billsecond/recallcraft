//! A few utterances of one stream heard in one go: end to end with a short pause between, in one
//! 30-second window (the model hears a window whatever is in it, so a short utterance alone
//! costs as much as a full window). Each line heard is taken back to the utterance it was in,
//! and to when that was said.

use crate::vad::{RATE, Utterance};

/// The pause put between utterances: long enough that the model ends a line there.
pub const GAP: usize = RATE as usize * 8 / 10;
/// How much goes in one window, at most.
pub const MAX: usize = RATE as usize * 28;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Piece {
    /// Where it starts in the packed sound.
    at: usize,
    len: usize,
    start_us: u64,
}

/// Utterances end to end.
#[derive(Clone, Debug, Default)]
pub struct Packed {
    pub sound: Vec<f32>,
    pieces: Vec<Piece>,
}

/// A line taken back: which utterance (its index in what was packed) and when, in µs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub piece: usize,
    pub start_us: u64,
    pub end_us: u64,
}

impl Packed {
    /// As many of `utterances` as fit, from the first; returns how many went in (at least one,
    /// cut to fit if it's longer than a window).
    pub fn new(utterances: &[&Utterance]) -> (Packed, usize) {
        let mut p = Packed::default();
        let mut taken = 0;
        for u in utterances {
            let gap = if p.sound.is_empty() { 0 } else { GAP };
            if !p.sound.is_empty() && p.sound.len() + gap + u.samples.len() > MAX {
                break;
            }
            p.sound.resize(p.sound.len() + gap, 0.0);
            let len = u.samples.len().min(MAX);
            p.pieces.push(Piece { at: p.sound.len(), len, start_us: u.start_us });
            p.sound.extend_from_slice(u.samples.get(..len).unwrap_or_default());
            taken += 1;
        }
        (p, taken)
    }

    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    /// A line heard at `start..end` seconds of the packed sound, taken back to its utterance.
    pub fn place(&self, start: f32, end: f32) -> Option<Placed> {
        if !start.is_finite() || !end.is_finite() {
            return None;
        }
        let rate = RATE as f32;
        let s = (start.max(0.0) * rate) as usize;
        // The utterance it starts in, or the next one if it starts in a pause.
        let (i, piece) = self.pieces.iter().enumerate().find(|(_, p)| s < p.at + p.len)?;
        let s = s.max(piece.at);
        // A line running on into the pause (or past it) ends with its utterance.
        let e = ((end.max(start) * rate) as usize).min(piece.at + piece.len).max(s + 1);
        let to_us = |x: usize| piece.start_us + (x - piece.at) as u64 * 1_000_000 / u64::from(RATE);
        Some(Placed { piece: i, start_us: to_us(s), end_us: to_us(e) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utt(start_us: u64, secs: f32) -> Utterance {
        let n = (secs * RATE as f32) as usize;
        Utterance { start_us, samples: vec![0.1; n], levels: vec![-20.0; n / 320], voiced: (n / 320) as u32 }
    }

    #[test]
    fn lines_go_back_to_their_utterance_and_its_time() {
        let a = utt(10_000_000, 2.0);
        let b = utt(50_000_000, 3.0);
        let (p, n) = Packed::new(&[&a, &b]);
        assert_eq!(n, 2);
        assert_eq!(p.sound.len(), (2.0 * RATE as f32) as usize + GAP + 3 * RATE as usize);
        // A line in the first.
        assert_eq!(p.place(0.5, 1.5), Some(Placed { piece: 0, start_us: 10_500_000, end_us: 11_500_000 }));
        // A line running over the pause is cut at its utterance's end.
        let over = p.place(1.0, 3.5).unwrap();
        assert_eq!((over.piece, over.end_us), (0, 12_000_000));
        // A line starting in the pause belongs to the next.
        let next = p.place(2.5, 4.0).unwrap();
        assert_eq!(next.piece, 1);
        assert_eq!(next.start_us, 50_000_000);
        assert_eq!(p.place(100.0, 101.0), None);
        assert_eq!(p.place(f32::NAN, 1.0), None);
    }

    #[test]
    fn what_doesnt_fit_waits_and_a_long_one_is_cut() {
        let long = utt(0, 40.0);
        let short = utt(0, 1.0);
        let (p, n) = Packed::new(&[&long, &short]);
        assert_eq!((n, p.sound.len()), (1, MAX));
        let many: Vec<Utterance> = (0..20).map(|i| utt(i * 10_000_000, 2.0)).collect();
        let refs: Vec<&Utterance> = many.iter().collect();
        let (p, n) = Packed::new(&refs);
        assert!(n > 5 && n < 20 && p.sound.len() <= MAX, "{n}");
        assert_eq!(Packed::new(&[]).1, 0);
    }
}
