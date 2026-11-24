//! A stream's sound cut into utterances as it comes in: someone starts talking, pauses for
//! longer than [`END_PAUSE_MS`], and what they said is handed over to be heard. Speech is told
//! from quiet by loudness against the stream's own background, which is followed as it changes
//! (a fan, a busy room).
//!
//! An utterance is at most [`MAX_US`] long: a long speech is cut at its quietest moment near
//! there, so the transcript keeps up.

use std::collections::VecDeque;

/// Samples a second.
pub const RATE: u32 = 16_000;
/// A step: 20 ms.
pub const FRAME: usize = 320;
pub const FRAME_US: u64 = 20_000;
/// A pause this long ends an utterance.
pub const END_PAUSE_MS: u64 = 700;
/// The longest utterance.
pub const MAX_US: u64 = 25_000_000;
/// Shorter speech than this isn't worth hearing (a cough, a click).
const MIN_VOICED: u32 = 12;
/// Speech this long in a row starts an utterance.
const ONSET: u32 = 3;
/// Kept from before speech starts, and after it ends.
const PRE_ROLL: usize = 10;
const POST_ROLL: usize = 10;
/// Louder than the background by this much is speech, and never quieter than [`MIN_DB`].
const ABOVE_FLOOR_DB: f32 = 10.0;
const MIN_DB: f32 = -52.0;
/// A sound starting this far from where the last one ended is a new stretch.
const JOIN_US: u64 = 100_000;
/// The background is taken to be between these.
const FLOOR_MIN: f32 = -90.0;
const FLOOR_MAX: f32 = -35.0;

fn level(frame: &[f32]) -> f32 {
    let sum: f32 = frame.iter().map(|s| s * s).sum();
    (10.0 * (sum / frame.len().max(1) as f32 + 1e-10).log10()).max(-100.0)
}

/// What someone said without a long pause, 16 kHz.
#[derive(Clone, Debug, PartialEq)]
pub struct Utterance {
    pub start_us: u64,
    pub samples: Vec<f32>,
    /// Its loudness every 20 ms.
    pub levels: Vec<f32>,
    /// 20 ms steps of it that were speech.
    pub voiced: u32,
}

impl Utterance {
    pub fn end_us(&self) -> u64 {
        self.start_us + self.samples.len() as u64 * 1_000_000 / u64::from(RATE)
    }

    pub fn seconds(&self) -> f32 {
        self.samples.len() as f32 / RATE as f32
    }
}

struct Building {
    start_us: u64,
    samples: Vec<f32>,
    levels: Vec<f32>,
    voiced: u32,
    quiet: u32,
}

/// Cuts one stream into utterances.
pub struct Segmenter {
    partial: Vec<f32>,
    partial_us: u64,
    /// When the next sound should start, if it carries on from the last.
    expect_us: Option<u64>,
    floor: f32,
    started: bool,
    onset: u32,
    before: VecDeque<(Vec<f32>, f32)>,
    building: Option<Building>,
}

impl Default for Segmenter {
    fn default() -> Segmenter {
        Segmenter::new()
    }
}

impl Segmenter {
    pub fn new() -> Segmenter {
        Segmenter {
            partial: Vec::new(),
            partial_us: 0,
            expect_us: None,
            floor: -60.0,
            started: false,
            onset: 0,
            before: VecDeque::new(),
            building: None,
        }
    }

    /// The background's loudness just now.
    pub fn floor(&self) -> f32 {
        self.floor
    }

    /// Whether someone is talking just now.
    pub fn talking(&self) -> bool {
        self.building.is_some()
    }

    fn threshold(&self) -> f32 {
        (self.floor + ABOVE_FLOOR_DB).max(MIN_DB)
    }

    /// Sound from `start_us`; returns the utterances it ended.
    pub fn push(&mut self, start_us: u64, samples: &[f32]) -> Vec<Utterance> {
        let mut out = Vec::new();
        let continues = self.expect_us.is_some_and(|e| start_us.abs_diff(e) <= JOIN_US);
        if !continues {
            if self.expect_us.is_some() {
                out.extend(self.flush());
            }
            self.partial.clear();
            self.partial_us = start_us;
        }
        self.partial.extend(samples.iter().map(|s| if s.is_finite() { *s } else { 0.0 }));
        self.expect_us = Some(self.partial_us + self.partial.len() as u64 * 1_000_000 / u64::from(RATE));
        let mut at = 0;
        while self.partial.len() - at >= FRAME {
            let frame: Vec<f32> = self.partial.get(at..at + FRAME).unwrap_or_default().to_vec();
            let us = self.partial_us;
            if let Some(u) = self.frame(frame, us) {
                out.push(u);
            }
            at += FRAME;
            self.partial_us += FRAME_US;
        }
        self.partial.drain(..at);
        out
    }

    fn frame(&mut self, frame: Vec<f32>, us: u64) -> Option<Utterance> {
        let db = level(&frame);
        if !self.started {
            // The background starts as loud as the first sound: steady noise is never speech.
            self.floor = db.clamp(FLOOR_MIN, FLOOR_MAX);
            self.started = true;
        }
        let voiced = db > self.threshold();
        // The background: down quickly to anything quieter (the pauses between words), up
        // slowly (1 dB a second) through speech.
        if db < self.floor {
            self.floor = self.floor * 0.7 + db * 0.3;
        } else {
            self.floor += 0.02;
        }
        self.floor = self.floor.clamp(FLOOR_MIN, FLOOR_MAX);
        match self.building.as_mut() {
            None => {
                self.onset = if voiced { self.onset + 1 } else { 0 };
                self.before.push_back((frame, db));
                if self.before.len() > PRE_ROLL + ONSET as usize {
                    self.before.pop_front();
                }
                if self.onset >= ONSET {
                    let first = us.saturating_sub((self.before.len() as u64 - 1) * FRAME_US);
                    let mut b = Building { start_us: first, samples: Vec::new(), levels: Vec::new(), voiced: self.onset, quiet: 0 };
                    for (f, l) in self.before.drain(..) {
                        b.samples.extend(f);
                        b.levels.push(l);
                    }
                    self.building = Some(b);
                    self.onset = 0;
                }
                None
            }
            Some(b) => {
                b.samples.extend_from_slice(&frame);
                b.levels.push(db);
                if voiced {
                    b.voiced += 1;
                    b.quiet = 0;
                } else {
                    b.quiet += 1;
                }
                if u64::from(b.quiet) * FRAME_US >= END_PAUSE_MS * 1000 {
                    return self.finish();
                }
                if b.levels.len() as u64 * FRAME_US >= MAX_US {
                    return self.cut();
                }
                None
            }
        }
    }

    /// Ends the utterance being built: its quiet end trimmed to [`POST_ROLL`].
    fn finish(&mut self) -> Option<Utterance> {
        let mut b = self.building.take()?;
        let keep = b.levels.len().saturating_sub(b.quiet as usize) + POST_ROLL;
        if keep < b.levels.len() {
            b.levels.truncate(keep);
            b.samples.truncate(keep * FRAME);
        }
        (b.voiced >= MIN_VOICED).then_some(Utterance { start_us: b.start_us, samples: b.samples, levels: b.levels, voiced: b.voiced })
    }

    /// A long speech cut at its quietest moment in the last three seconds; the rest goes on.
    fn cut(&mut self) -> Option<Utterance> {
        let b = self.building.take()?;
        let n = b.levels.len();
        let from = n.saturating_sub(150);
        let at = (from..n).min_by(|x, y| b.levels.get(*x).unwrap_or(&0.0).total_cmp(b.levels.get(*y).unwrap_or(&0.0))).unwrap_or(n).max(1);
        let (head_l, tail_l) = b.levels.split_at(at.min(n));
        let (head_s, tail_s) = b.samples.split_at((at * FRAME).min(b.samples.len()));
        let rest_voiced = tail_l.iter().filter(|l| **l > self.threshold()).count() as u32;
        let head =
            Utterance { start_us: b.start_us, samples: head_s.to_vec(), levels: head_l.to_vec(), voiced: b.voiced.saturating_sub(rest_voiced) };
        self.building = Some(Building {
            start_us: b.start_us + at as u64 * FRAME_US,
            samples: tail_s.to_vec(),
            levels: tail_l.to_vec(),
            voiced: rest_voiced,
            quiet: 0,
        });
        Some(head)
    }

    /// Ends whatever is being said (the stream stopped, the call ended).
    pub fn flush(&mut self) -> Option<Utterance> {
        self.partial.clear();
        self.expect_us = None;
        self.before.clear();
        self.onset = 0;
        if let Some(b) = self.building.as_mut() {
            b.quiet = 0;
        }
        self.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A voice-like sound: a buzz of 150 Hz and its harmonics, loudness `db`.
    fn voice(n: usize, from: usize, db: f32) -> Vec<f32> {
        let a = 10f32.powf(db / 20.0) * 1.2;
        (from..from + n)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                // Syllables: louder and quieter three times a second.
                let syllables = 0.55 + 0.45 * (t * 2.0 * std::f32::consts::PI * 3.0).sin();
                (1..6).map(|h| (t * 2.0 * std::f32::consts::PI * 150.0 * h as f32).sin() / h as f32).sum::<f32>() * a * 0.6 * syllables
            })
            .collect()
    }

    fn noise(n: usize, seed: u32, db: f32) -> Vec<f32> {
        let a = 10f32.powf(db / 20.0) * 1.7;
        let mut x = seed.max(1);
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x as f32 / u32::MAX as f32 - 0.5) * a
            })
            .collect()
    }

    fn run(s: &mut Segmenter, t0: u64, sound: &[f32]) -> Vec<Utterance> {
        let mut out = Vec::new();
        for (k, block) in sound.chunks(160).enumerate() {
            out.extend(s.push(t0 + k as u64 * 10_000, block));
        }
        out
    }

    #[test]
    fn two_sentences_with_a_pause_are_two_utterances() {
        let t0 = 1_000_000_000;
        let sec = RATE as usize;
        let mut sound = noise(sec, 1, -60.0);
        sound.extend(voice(2 * sec, 0, -20.0).iter().zip(noise(2 * sec, 2, -60.0)).map(|(a, b)| a + b));
        sound.extend(noise(sec, 3, -60.0));
        sound.extend(voice(sec * 3 / 2, 0, -24.0));
        sound.extend(noise(2 * sec, 4, -60.0));
        let mut s = Segmenter::new();
        let u = run(&mut s, t0, &sound);
        assert_eq!(u.len(), 2, "{:?}", u.iter().map(|x| (x.start_us, x.seconds())).collect::<Vec<_>>());
        // The first starts a little before the voice (1 s in) and ends a little after it (3 s).
        assert!(u[0].start_us >= t0 + 700_000 && u[0].start_us <= t0 + 1_000_000, "{}", u[0].start_us - t0);
        assert!(u[0].end_us() >= t0 + 3_000_000 && u[0].end_us() <= t0 + 3_400_000, "{}", u[0].end_us() - t0);
        assert!(u[1].start_us > t0 + 3_700_000 && u[1].start_us <= t0 + 4_000_000);
        assert!(u.iter().all(|x| x.samples.len() == x.levels.len() * FRAME));
        assert!(!s.talking());
    }

    #[test]
    fn noise_and_clicks_are_not_speech_and_long_speech_is_cut() {
        let sec = RATE as usize;
        let mut s = Segmenter::new();
        // A noisy room, louder than the minimum, then a click.
        let mut sound = noise(5 * sec, 9, -40.0);
        sound.extend(voice(sec / 25, 0, -10.0));
        sound.extend(noise(2 * sec, 10, -40.0));
        assert!(run(&mut s, 0, &sound).is_empty(), "the floor follows the room");
        // A minute of talking without a pause: cut into pieces of at most 25 s.
        let mut s = Segmenter::new();
        let mut talk = noise(sec, 11, -60.0);
        talk.extend(voice(60 * sec, 0, -20.0));
        let mut u = run(&mut s, 0, &talk);
        u.extend(s.flush());
        assert!(u.len() >= 3, "{}", u.len());
        assert!(u.iter().all(|x| x.end_us() - x.start_us <= MAX_US));
        let total: u64 = u.iter().map(|x| x.end_us() - x.start_us).sum();
        assert!(total >= 59_000_000, "nothing lost between the pieces: {total}");
        for w in u.windows(2) {
            assert_eq!(w[0].end_us(), w[1].start_us);
        }
    }

    #[test]
    fn a_gap_in_the_sound_ends_what_was_being_said() {
        let sec = RATE as usize;
        let mut s = Segmenter::new();
        let v = voice(sec, 0, -20.0);
        let mut u = run(&mut s, 0, &v);
        assert!(u.is_empty() && s.talking());
        // The device stopped; sound again a minute later.
        u.extend(s.push(60_000_000, &[0.0; 160]));
        assert_eq!(u.len(), 1);
        assert!(s.push(0, &[]).is_empty());
        assert!(s.push(5, &[f32::NAN; 1000]).is_empty());
    }
}
