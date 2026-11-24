//! The two sound streams and who said a line.
//!
//! What the microphone hears is you; what the computer plays is the others. The others can be
//! told apart by their voices ([`Speaker::Voice`]), each given a name later.

use std::fmt;

use serde::{Deserialize, Serialize};

/// One of the two sounds recorded all the time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    /// The microphone: you.
    Mic,
    /// What the computer plays: the other people on a call (and anything else that plays).
    System,
}

impl Stream {
    pub const ALL: [Stream; 2] = [Stream::Mic, Stream::System];

    /// The folder its sound is kept in.
    pub fn dir_name(self) -> &'static str {
        match self {
            Stream::Mic => "mic",
            Stream::System => "system",
        }
    }

    /// The byte that names it in a file.
    pub fn id(self) -> u8 {
        match self {
            Stream::Mic => 0,
            Stream::System => 1,
        }
    }

    pub fn from_id(id: u8) -> Option<Stream> {
        match id {
            0 => Some(Stream::Mic),
            1 => Some(Stream::System),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Stream::Mic => "Microphone",
            Stream::System => "Computer sound",
        }
    }

    /// Who a line heard in this stream is, before voices are told apart.
    pub fn speaker(self) -> Speaker {
        match self {
            Stream::Mic => Speaker::Me,
            Stream::System => Speaker::Others,
        }
    }
}

/// Who said a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum Speaker {
    /// The microphone.
    Me,
    /// The computer's sound, voices not told apart.
    Others,
    /// One of the others, by voice: 1, 2, …
    Voice(u8),
}

/// The most voices told apart on one call.
pub const MAX_VOICES: u8 = 12;

impl Speaker {
    /// The key it's kept under (`me`, `others`, `voice-2`).
    pub fn key(self) -> String {
        match self {
            Speaker::Me => "me".to_string(),
            Speaker::Others => "others".to_string(),
            Speaker::Voice(n) => format!("voice-{n}"),
        }
    }

    pub fn from_key(key: &str) -> Option<Speaker> {
        match key.trim() {
            "me" => Some(Speaker::Me),
            "others" | "them" => Some(Speaker::Others),
            k => {
                let n: u8 = k.strip_prefix("voice-")?.parse().ok()?;
                (1..=MAX_VOICES).contains(&n).then_some(Speaker::Voice(n))
            }
        }
    }

    /// The name shown when nobody has named it.
    pub fn default_name(self, your_name: &str) -> String {
        match self {
            Speaker::Me => {
                let n = your_name.trim();
                if n.is_empty() { "Me".to_string() } else { n.to_string() }
            }
            Speaker::Others => "Them".to_string(),
            Speaker::Voice(n) => format!("Speaker {n}"),
        }
    }

    pub fn is_me(self) -> bool {
        self == Speaker::Me
    }
}

impl From<Speaker> for String {
    fn from(s: Speaker) -> String {
        s.key()
    }
}

impl TryFrom<String> for Speaker {
    type Error = String;
    fn try_from(s: String) -> Result<Speaker, String> {
        Speaker::from_key(&s).ok_or_else(|| format!("not a speaker: {s:?}"))
    }
}

impl fmt::Display for Speaker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.key())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speakers_round_trip_and_junk_is_refused() {
        for s in [Speaker::Me, Speaker::Others, Speaker::Voice(1), Speaker::Voice(MAX_VOICES)] {
            assert_eq!(Speaker::from_key(&s.key()), Some(s));
            let json = serde_json::to_string(&s).unwrap();
            assert_eq!(serde_json::from_str::<Speaker>(&json).unwrap(), s);
        }
        for junk in ["", "voice-0", "voice-99", "voice-x", "you", "voice-"] {
            assert_eq!(Speaker::from_key(junk), None, "{junk}");
        }
        assert!(serde_json::from_str::<Speaker>("\"nobody\"").is_err());
        assert_eq!(Speaker::Me.default_name("  "), "Me");
        assert_eq!(Speaker::Me.default_name("Bill"), "Bill");
        assert_eq!(Speaker::Voice(3).default_name(""), "Speaker 3");
    }

    #[test]
    fn streams_have_ids_and_speakers() {
        for s in Stream::ALL {
            assert_eq!(Stream::from_id(s.id()), Some(s));
        }
        assert_eq!(Stream::from_id(7), None);
        assert_eq!(Stream::Mic.speaker(), Speaker::Me);
        assert_eq!(Stream::System.speaker(), Speaker::Others);
    }
}
