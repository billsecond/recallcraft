//! The others heard twice: with speakers rather than headphones, the microphone hears what the
//! computer plays, so the others' words would come back as yours.
//!
//! - Before hearing: an utterance of yours during which the computer's sound was the louder
//!   most of the time is what the speakers played ([`louder_share`]); it isn't heard at all.
//! - After: a line of yours whose words are mostly the others' at the same time is their echo
//!   ([`is_echo`]), and goes.
//!
//! The loudness of both streams is kept for the last two minutes ([`Levels`]), on the same clock.

use std::collections::VecDeque;

/// A step: 20 ms.
pub const STEP_US: u64 = 20_000;
/// Steps either side the computer's sound is looked at (the speakers reach the microphone a
/// little late, and the two devices' clocks differ a little).
const LAG: u64 = 6;
/// Below this the microphone heard nothing.
const VOICED_DB: f32 = -55.0;
/// Kept: two minutes.
const KEEP: usize = 6000;
pub const SILENT_DB: f32 = -100.0;

/// A stream's loudness every 20 ms, for the last two minutes.
#[derive(Clone, Debug, Default)]
pub struct Levels {
    first: u64,
    values: VecDeque<f32>,
}

impl Levels {
    pub fn new() -> Levels {
        Levels::default()
    }

    /// The loudness at `us`.
    pub fn push(&mut self, us: u64, db: f32) {
        let step = us / STEP_US;
        let db = if db.is_finite() { db } else { SILENT_DB };
        if self.values.is_empty() {
            self.first = step;
        }
        if step < self.first {
            if self.first - step <= KEEP as u64 {
                // Older than what's kept: nothing to do.
                return;
            }
            // Far in the past: the clock went back. Start again.
            self.values.clear();
            self.first = step;
        }
        let mut i = step - self.first;
        if i > 2 * KEEP as u64 {
            // Far ahead (nothing came for a while): start again from here.
            self.values.clear();
            self.first = step;
            i = 0;
        }
        let i = i as usize;
        while self.values.len() <= i {
            self.values.push_back(SILENT_DB);
        }
        if let Some(v) = self.values.get_mut(i) {
            *v = v.max(db);
        }
        while self.values.len() > KEEP {
            self.values.pop_front();
            self.first += 1;
        }
    }

    fn at_step(&self, step: u64) -> f32 {
        if step < self.first {
            return SILENT_DB;
        }
        usize::try_from(step - self.first).ok().and_then(|i| self.values.get(i)).copied().unwrap_or(SILENT_DB)
    }

    /// The loudness at `us` ([`SILENT_DB`] for a moment not kept).
    pub fn at(&self, us: u64) -> f32 {
        self.at_step(us / STEP_US)
    }

    /// The loudest within `LAG` steps of `us`.
    fn around(&self, us: u64) -> f32 {
        let s = us / STEP_US;
        (s.saturating_sub(LAG)..=s.saturating_add(LAG)).map(|k| self.at_step(k)).fold(SILENT_DB, f32::max)
    }
}

/// Of the moments in `[start, end)` the microphone heard something, the share when the
/// computer's sound was louder: near 1, what it heard was the speakers.
pub fn louder_share(start_us: u64, end_us: u64, mic: &Levels, system: &Levels) -> f32 {
    let (mut voiced, mut louder) = (0u32, 0u32);
    let mut us = start_us - start_us % STEP_US;
    let mut n = 0;
    while us < end_us && n < 3000 {
        let m = mic.at(us);
        if m >= VOICED_DB {
            voiced += 1;
            if system.around(us) > m {
                louder += 1;
            }
        }
        us += STEP_US;
        n += 1;
    }
    if voiced == 0 { 0.0 } else { louder as f32 / voiced as f32 }
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase().split(|c: char| !(c.is_alphanumeric() || c == '\'')).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// How many of `mine`'s words are among `theirs`, 0 to 1.
pub fn shared(mine: &str, theirs: &str) -> f32 {
    let (a, b) = (words(mine), words(theirs));
    if a.is_empty() {
        return 0.0;
    }
    a.iter().filter(|w| b.contains(w)).count() as f32 / a.len() as f32
}

/// Whether a line of yours (`text`, said `start..end`) is the others' heard again: their lines
/// at the same time (`theirs`: start, end, text) have most of its words.
pub fn is_echo(start_us: u64, end_us: u64, text: &str, theirs: &[(u64, u64, &str)]) -> bool {
    let len = end_us.saturating_sub(start_us).max(1);
    let overlapping: Vec<&(u64, u64, &str)> = theirs.iter().filter(|(s, e, _)| *s < end_us + 500_000 && start_us < e + 500_000).collect();
    if overlapping.is_empty() {
        return false;
    }
    let overlap: u64 = overlapping.iter().map(|(s, e, _)| end_us.min(*e).saturating_sub(start_us.max(*s))).sum();
    let all: String = overlapping.iter().map(|(_, _, t)| *t).collect::<Vec<_>>().join(" ");
    let share = shared(text, &all);
    share >= 0.7 || (share >= 0.5 && overlap * 10 >= len * 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(l: &mut Levels, from_s: f32, to_s: f32, db: f32) {
        let mut us = (from_s * 1e6) as u64;
        while us < (to_s * 1e6) as u64 {
            l.push(us, db);
            us += STEP_US;
        }
    }

    #[test]
    fn the_speakers_heard_by_the_microphone_are_told_from_you() {
        let (mut mic, mut sys) = (Levels::new(), Levels::new());
        // They speak 0–4 s (loud on the computer, faint in the microphone); you 5–8 s.
        fill(&mut sys, 0.0, 10.0, SILENT_DB);
        fill(&mut sys, 0.0, 4.0, -12.0);
        fill(&mut mic, 0.0, 10.0, -70.0);
        fill(&mut mic, 0.05, 4.05, -38.0);
        fill(&mut mic, 5.0, 8.0, -18.0);
        assert!(louder_share(0, 4_000_000, &mic, &sys) > 0.9);
        assert!(louder_share(5_000_000, 8_000_000, &mic, &sys) < 0.1);
        // You talking over them, louder than they are: yours.
        fill(&mut sys, 8.0, 10.0, -15.0);
        fill(&mut mic, 8.0, 10.0, -8.0);
        assert!(louder_share(8_000_000, 10_000_000, &mic, &sys) < 0.1);
        assert_eq!(louder_share(20_000_000, 21_000_000, &mic, &sys), 0.0, "nothing heard");
    }

    #[test]
    fn levels_keep_two_minutes_and_survive_the_clock() {
        let mut l = Levels::new();
        l.push(1_000_000_000, -20.0);
        assert_eq!(l.at(1_000_000_000), -20.0);
        assert_eq!(l.at(999_000_000), SILENT_DB);
        l.push(1_000_000_000 + 200_000_000, -30.0);
        assert_eq!(l.at(1_000_000_000), SILENT_DB, "more than two minutes ago");
        assert_eq!(l.at(1_200_000_000), -30.0);
        l.push(5, -10.0);
        assert_eq!(l.at(5), -10.0, "the clock went back: started again");
        l.push(u64::MAX - 1, f32::NAN);
        assert_eq!(l.at(u64::MAX - 1), SILENT_DB);
    }

    #[test]
    fn a_line_of_theirs_heard_again_is_an_echo() {
        let theirs = [(0u64, 4_000_000u64, "The budget is due on Tuesday.")];
        assert!(is_echo(300_000, 3_900_000, "The budget is due on Tuesday.", &theirs));
        assert!(is_echo(300_000, 3_900_000, "budget due Tuesday", &theirs));
        assert!(!is_echo(5_000_000, 7_000_000, "Thanks, I will send it.", &theirs));
        assert!(!is_echo(300_000, 3_900_000, "No, Monday works better for me.", &theirs));
        assert!(!is_echo(0, 1, "anything", &[]));
    }
}
