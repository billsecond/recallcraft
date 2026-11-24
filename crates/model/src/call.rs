//! A call and its transcript.
//!
//! A call is a stretch of the recording when a calling app (a meeting app, a chat app's call, a
//! browser in a meeting) was using the microphone. Its sound stays in the recording itself
//! (kept as long as the call is); the call adds when, which app, a title, and the names given to
//! who spoke. Times are milliseconds since 1970 (UTC).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::speaker::Speaker;

/// The longest text kept for a line, a title or a name (a damaged file can say anything).
pub const MAX_TEXT: usize = 4000;
pub const MAX_NAME: usize = 120;

/// One line of a transcript: who said what, and when.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker: Speaker,
    pub text: String,
}

impl Line {
    /// The line made safe to keep and show: its end after its start, its text trimmed and not
    /// too long. None for a line with nothing said.
    pub fn checked(mut self) -> Option<Line> {
        self.text = clip(self.text.trim(), MAX_TEXT);
        if self.text.is_empty() {
            return None;
        }
        self.end_ms = self.end_ms.max(self.start_ms.saturating_add(1));
        Some(self)
    }

    pub fn contains(&self, at_ms: u64) -> bool {
        self.start_ms <= at_ms && at_ms < self.end_ms
    }
}

/// `text` cut to at most `max` characters, at a character boundary.
pub fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// A call.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Call {
    /// Its folder's name: when it started, locally, and the app (`2026-11-24_14-30-05_teams`).
    pub id: String,
    /// The app, as people call it ("Microsoft Teams").
    pub app: String,
    /// The app's program file ("ms-teams.exe").
    pub app_exe: String,
    /// The meeting's title, from the app's window, or one the owner gave it.
    pub title: String,
    pub started_ms: u64,
    /// None while it goes on.
    pub ended_ms: Option<u64>,
    /// The names given to who spoke, by [`Speaker::key`].
    pub names: BTreeMap<String, String>,
    /// Kept until the owner deletes it, whatever the settings say.
    pub keep: bool,
    /// The titles of the app's windows seen during the call (where the title came from).
    pub windows: Vec<String>,
    /// Whether the title was given by the owner (it is then never replaced by a window's).
    pub titled_by_owner: bool,
}

/// The most window titles remembered for a call.
pub const MAX_WINDOWS: usize = 24;

impl Call {
    pub fn is_live(&self) -> bool {
        self.ended_ms.is_none()
    }

    /// How long it was, or has been so far at `now_ms`.
    pub fn duration_ms(&self, now_ms: u64) -> u64 {
        self.ended_ms.unwrap_or(now_ms).saturating_sub(self.started_ms)
    }

    /// Its end, or `now_ms` while it goes on.
    pub fn end_or(&self, now_ms: u64) -> u64 {
        self.ended_ms.unwrap_or(now_ms).max(self.started_ms)
    }

    /// What the list shows: the title, else the app's call.
    pub fn display_title(&self) -> String {
        let t = self.title.trim();
        if !t.is_empty() {
            return t.to_string();
        }
        if self.app.trim().is_empty() { "Call".to_string() } else { format!("{} call", self.app.trim()) }
    }

    /// The name shown for a speaker.
    pub fn speaker_name(&self, s: Speaker, your_name: &str) -> String {
        match self.names.get(&s.key()).map(|n| n.trim()).filter(|n| !n.is_empty()) {
            Some(n) => n.to_string(),
            None => s.default_name(your_name),
        }
    }

    /// Names a speaker (an empty name takes it back).
    pub fn set_name(&mut self, s: Speaker, name: &str) {
        let name = clip(name.trim(), MAX_NAME);
        if name.is_empty() {
            self.names.remove(&s.key());
        } else {
            self.names.insert(s.key(), name);
        }
    }

    /// Remembers a window title seen during the call.
    pub fn saw_window(&mut self, title: &str) {
        let t = clip(title.trim(), MAX_NAME * 2);
        if t.is_empty() || self.windows.contains(&t) || self.windows.len() >= MAX_WINDOWS {
            return;
        }
        self.windows.push(t);
    }

    /// The call made safe after reading it from a file.
    pub fn checked(mut self) -> Call {
        self.app = clip(&self.app, MAX_NAME);
        self.app_exe = clip(&self.app_exe, MAX_NAME);
        self.title = clip(&self.title, MAX_NAME * 2);
        self.names = self.names.into_iter().filter(|(k, _)| Speaker::from_key(k).is_some()).map(|(k, v)| (k, clip(&v, MAX_NAME))).collect();
        self.windows.truncate(MAX_WINDOWS);
        if let Some(e) = self.ended_ms {
            self.ended_ms = Some(e.max(self.started_ms));
        }
        self
    }

    /// Whether `[from, to)` (ms) overlaps the call, with `margin` either side.
    pub fn overlaps(&self, from_ms: u64, to_ms: u64, margin_ms: u64, now_ms: u64) -> bool {
        let start = self.started_ms.saturating_sub(margin_ms);
        let end = self.end_or(now_ms).saturating_add(margin_ms);
        from_ms < end && start < to_ms
    }
}

/// A folder name made safe from any text: letters, digits and `-`, lower case, at most `max`.
pub fn slug(text: &str, max: usize) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= max {
            break;
        }
    }
    out.trim_end_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_made_safe() {
        let l = Line { start_ms: 10, end_ms: 5, speaker: Speaker::Me, text: "  hi  ".into() }.checked().unwrap();
        assert_eq!((l.start_ms, l.end_ms, l.text.as_str()), (10, 11, "hi"));
        assert!(Line { start_ms: 0, end_ms: 1, speaker: Speaker::Me, text: "   ".into() }.checked().is_none());
        let long = Line { start_ms: 0, end_ms: 1, speaker: Speaker::Others, text: "é".repeat(MAX_TEXT + 50) }.checked().unwrap();
        assert_eq!(long.text.chars().count(), MAX_TEXT);
        assert!(l.contains(10) && !l.contains(11));
    }

    #[test]
    fn a_call_names_its_speakers_and_reads_back() {
        let mut c = Call { id: "x".into(), app: "Zoom".into(), started_ms: 1000, ..Call::default() };
        assert_eq!(c.display_title(), "Zoom call");
        assert!(c.is_live());
        assert_eq!(c.duration_ms(4000), 3000);
        c.set_name(Speaker::Voice(2), "  John Smith ");
        assert_eq!(c.speaker_name(Speaker::Voice(2), ""), "John Smith");
        assert_eq!(c.speaker_name(Speaker::Voice(1), ""), "Speaker 1");
        assert_eq!(c.speaker_name(Speaker::Me, "Bill"), "Bill");
        c.set_name(Speaker::Voice(2), "");
        assert_eq!(c.speaker_name(Speaker::Voice(2), ""), "Speaker 2");
        c.saw_window("Weekly sync | Microsoft Teams");
        c.saw_window("Weekly sync | Microsoft Teams");
        assert_eq!(c.windows.len(), 1);
        let json = serde_json::to_string(&c).unwrap();
        let back: Call = serde_json::from_str::<Call>(&json).unwrap().checked();
        assert_eq!(back, c);
        // Junk names are dropped; a missing field takes its default.
        let odd: Call =
            serde_json::from_str::<Call>(r#"{"id":"y","names":{"nobody":"x","me":"Ann"},"ended_ms":5,"started_ms":9}"#).unwrap().checked();
        assert_eq!(odd.names.len(), 1);
        assert_eq!(odd.ended_ms, Some(9));
        assert!(c.overlaps(0, 1001, 0, 5000));
        assert!(!c.overlaps(0, 999, 0, 5000));
        assert!(c.overlaps(0, 999, 10, 5000));
    }

    #[test]
    fn slugs_are_plain() {
        assert_eq!(slug("Microsoft Teams", 20), "microsoft-teams");
        assert_eq!(slug("  ?? ", 20), "");
        assert_eq!(slug("ms-teams.exe", 4), "ms-t");
    }
}
