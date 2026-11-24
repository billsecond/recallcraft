//! RecallCraft's files.
//!
//! - [`ring`]: everything heard, two streams (the microphone and the computer's sound), kept as
//!   Opus packets in files of five minutes each. Silence isn't kept. Written by the service,
//!   read by the window (while it's written) and the command-line program.
//! - [`calls`]: a folder for each call: `call.json` and `transcript.jsonl`.
//! - [`retention`]: what goes when it's older than the settings keep it.
//! - [`export`]: a call (or any stretch) as WAV or Opus files and transcripts.
//!
//! Layer L2: files only, no devices, no windows. Everything read from a file is hostile: a
//! damaged file is cut short or left out, never a crash.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod calls;
pub mod export;
pub mod ogg;
pub mod opus;
pub mod retention;
pub mod ring;
pub mod transcript;
pub mod wav;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("sound: {0}")]
    Codec(String),
    #[error("{0}")]
    Other(String),
}

/// Writes `bytes` to `path` by way of a temporary file beside it, so a crash mid-write never
/// leaves half a file behind.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let n = WRITES.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(format!(".tmp-{}-{n}", std::process::id()));
    let tmp: PathBuf = path.with_file_name(name);
    std::fs::write(&tmp, bytes)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// The folders of a data folder.
#[derive(Clone, Debug)]
pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    pub fn new(root: impl Into<PathBuf>) -> Layout {
        Layout { root: root.into() }
    }

    /// The recording.
    pub fn ring(&self) -> PathBuf {
        self.root.join("ring")
    }

    /// The calls.
    pub fn calls(&self) -> PathBuf {
        self.root.join("calls")
    }

    /// What the service says it is doing, for the window.
    pub fn status(&self) -> PathBuf {
        self.root.join("status.json")
    }
}

/// How much a folder holds, in bytes (at most `max_files` files looked at).
pub fn folder_size(dir: &Path, max_files: usize) -> u64 {
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            seen += 1;
            if seen > max_files {
                return total;
            }
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                stack.push(e.path());
            } else {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

/// A size for people: `512 KB`, `3.4 MB`, `1.2 GB`.
pub fn size_words(bytes: u64) -> String {
    let b = bytes as f64;
    if b < 1024.0 * 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else if b < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB", b / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", b / (1024.0 * 1024.0 * 1024.0))
    }
}

#[cfg(test)]
pub(crate) fn temp_dir(name: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let d = std::env::temp_dir().join(format!("recallcraft-test-{name}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::create_dir_all(&d);
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_writes_and_sizes() {
        let d = temp_dir("atomic");
        let f = d.join("a.json");
        write_atomic(&f, b"one").unwrap();
        write_atomic(&f, b"two!").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"two!");
        assert_eq!(folder_size(&d, 100), 4);
        assert_eq!(size_words(2048), "2 KB");
        assert_eq!(size_words(5 * 1024 * 1024), "5.0 MB");
        let _ = std::fs::remove_dir_all(&d);
    }
}
