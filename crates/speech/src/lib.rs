//! Hearing what was said, on this computer: Whisper (OpenAI, MIT) run on the processor by
//! candle, written in Rust. Nothing leaves the computer.
//!
//! The Whisper engine ([`gguf`], [`model`], [`mel`], [`decode`]) is ScreenGrabCraft's, as it is.
//! Around it, for hearing a call as it goes on:
//!
//! - [`resample`]: the recording's 48 kHz made the 16 kHz Whisper hears.
//! - [`vad`]: the sound of each stream cut into utterances, at the pauses.
//! - [`pack`]: a few utterances end to end in one window, and the way back to when each was said.
//! - [`echo`]: the others' voices heard again by the microphone (speakers, not headphones) are
//!   not taken for yours.
//! - [`voices`]: the others told apart by their voices.
//!
//! Layer L2: no devices and no files. Whatever a model file or the sound holds, a bad one is an
//! error, never a crash: everything candle runs is under `catch_unwind`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod decode;
pub mod echo;
pub mod gguf;
pub mod mel;
pub mod model;
pub mod pack;
pub mod resample;
pub mod vad;
pub mod voices;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::AtomicBool;

pub use decode::Line;
pub use model::{Model, Whisper};

/// The model installed with RecallCraft: Whisper base, English only, its large weights in 8 bits.
pub const BUNDLED: &str = "whisper-base.en-q8_0.gguf";
/// The largest model file read.
pub const MAX_MODEL_BYTES: usize = 2 * 1024 * 1024 * 1024;
/// The longest line kept.
pub const MAX_TEXT: usize = 1000;

#[derive(Debug, thiserror::Error)]
pub enum SpeechError {
    #[error("the speech model: {0}")]
    Model(String),
    #[error("cancelled")]
    Cancelled,
}

/// Runs `f`, turning a panic in it (in candle, on a file it didn't expect) into an error.
fn guarded<T>(what: &str, f: impl FnOnce() -> Result<T, SpeechError>) -> Result<T, SpeechError> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        log::error!("{what} panicked");
        Err(SpeechError::Model(format!("{what} failed unexpectedly")))
    })
}

/// Reads a Whisper model from its file's bytes.
pub fn load(bytes: &[u8]) -> Result<Whisper, SpeechError> {
    if bytes.len() > MAX_MODEL_BYTES {
        return Err(SpeechError::Model("the model file is too large".to_string()));
    }
    guarded("reading the speech model", || Whisper::load(bytes))
}

/// Text on one line: words separated by single spaces, not too long.
pub fn clean_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_TEXT * 4));
    for word in text.split(|c: char| c.is_whitespace() || c.is_control()).filter(|w| !w.is_empty()) {
        if out.chars().count() + word.chars().count() + 1 > MAX_TEXT {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// What the model writes for silence or noise, that nobody says on a call.
const NOT_SAID: [&str; 8] = [
    "you",
    "thanks for watching!",
    "thank you for watching.",
    "thanks for watching.",
    "please subscribe.",
    "subtitles by the amara.org community",
    "so",
    "...",
];

/// Whether a line is speech, not a sound written down (`[Music]`, `(laughs)`, `♪`) or what the
/// model writes for noise.
pub fn is_speech(text: &str) -> bool {
    let t = text.trim();
    let bracketed = (t.starts_with('[') && t.ends_with(']')) || (t.starts_with('(') && t.ends_with(')')) || (t.starts_with('*') && t.ends_with('*'));
    let lower = t.to_lowercase();
    !bracketed && t.chars().any(char::is_alphanumeric) && !NOT_SAID.contains(&lower.as_str())
}

/// What was said in some 16 kHz mono sound: its lines, in seconds from its start.
pub fn hear(model: &mut dyn Model, sound: &[f32], cancel: &AtomicBool) -> Result<Vec<Line>, SpeechError> {
    guarded("hearing speech", || {
        let mut read = |start: usize, n: usize| sound.get(start..start.saturating_add(n).min(sound.len())).unwrap_or_default().to_vec();
        let lines = decode::transcribe(model, sound.len(), &mut read, &mut |_| {}, cancel)?;
        let mut out: Vec<Line> = Vec::new();
        for l in lines {
            let text = clean_text(&l.text);
            if !is_speech(&text) {
                continue;
            }
            // The same line again straight after is the model stuck, not someone repeating it.
            if out.last().is_some_and(|p: &Line| p.text == text && l.start <= p.end + 2.0) {
                continue;
            }
            out.push(Line { start: l.start.max(0.0), end: l.end.max(l.start), text });
        }
        Ok(out)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_speech_is_kept() {
        assert!(is_speech("Are you the developer?"));
        assert!(!is_speech("[Music]"));
        assert!(!is_speech("(laughs)"));
        assert!(!is_speech("♪ ♪"));
        assert!(!is_speech(" you "));
        assert!(!is_speech("Thanks for watching!"));
        assert!(is_speech("Thank you."));
        assert_eq!(clean_text("  a \n b\t\tc "), "a b c");
        assert!(clean_text(&"word ".repeat(1000)).chars().count() <= MAX_TEXT);
    }

    #[test]
    fn a_bad_model_file_is_an_error() {
        assert!(load(b"").is_err());
        assert!(load(b"GGUF\x03\0\0\0junk").is_err());
        assert!(load(&[0u8; 4096]).is_err());
    }
}
