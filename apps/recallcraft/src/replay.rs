//! The replay going on, as the service tells it to the replay bar (`recallcraft --replay-bar`):
//! a small file in the temporary folder, written when a replay starts and when it ends.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReplayState {
    /// Another for every replay.
    pub id: u64,
    pub playing: bool,
    /// How far back it goes, in seconds.
    pub seconds: u32,
    pub speed: f32,
    /// When it started, ms since 1970.
    pub started_ms: u64,
    /// How long it plays, ms.
    pub lasts_ms: u64,
}

impl ReplayState {
    /// How much of it has played at `now_ms`, 0 to 1.
    pub fn done(&self, now_ms: u64) -> f32 {
        if self.lasts_ms == 0 {
            return 1.0;
        }
        (now_ms.saturating_sub(self.started_ms) as f32 / self.lasts_ms as f32).clamp(0.0, 1.0)
    }

    /// The seconds of the recording played by `now_ms`.
    pub fn played_seconds(&self, now_ms: u64) -> u32 {
        (self.done(now_ms) * self.seconds as f32).round() as u32
    }
}

pub fn state_file() -> PathBuf {
    std::env::temp_dir().join("RecallCraft").join("replay.json")
}

pub fn write(s: &ReplayState) {
    let path = state_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let r = serde_json::to_vec(s).map_err(std::io::Error::other).and_then(|b| recallcraft_store::write_atomic(&path, &b));
    if let Err(e) = r {
        log::warn!("{}: {e}", path.display());
    }
}

/// What the service last wrote; None when there's nothing readable.
pub fn read() -> Option<ReplayState> {
    let text = std::fs::read_to_string(state_file()).ok()?;
    if text.len() > 4096 {
        return None;
    }
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replay_s_progress_is_told_from_its_start() {
        let r = ReplayState { id: 1, playing: true, seconds: 30, speed: 1.5, started_ms: 10_000, lasts_ms: 20_000 };
        assert_eq!(r.done(10_000), 0.0);
        assert_eq!(r.played_seconds(20_000), 15);
        assert_eq!(r.done(99_000), 1.0);
        assert_eq!(r.played_seconds(5_000), 0, "a clock that went back");
        assert_eq!(ReplayState::default().done(1), 1.0);
    }
}
