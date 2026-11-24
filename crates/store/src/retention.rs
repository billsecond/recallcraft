//! What goes when it's old: files of the recording older than the settings keep everything
//! (unless a call still kept needs them), and calls older than the settings keep calls (unless
//! the owner keeps them). The newest file of each stream is never deleted (it may be being
//! written).

use std::path::Path;

use recallcraft_model::Stream;

use crate::calls::CallStore;
use crate::ring::{SegmentInfo, segments};

/// What a sweep deleted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Swept {
    pub files: usize,
    pub calls: usize,
    pub bytes: u64,
}

/// The margin of sound kept either side of a call.
const CALL_MARGIN_MS: u64 = 60_000;

/// Deletes what is older than kept. `keep_audio_ms` and `keep_calls_ms` are how long each is
/// kept; `now_ms` is now.
pub fn sweep(ring: &Path, calls: &CallStore, keep_audio_ms: u64, keep_calls_ms: u64, now_ms: u64) -> Swept {
    let mut swept = Swept::default();
    let calls_from = now_ms.saturating_sub(keep_calls_ms);
    let mut kept_calls = Vec::new();
    for call in calls.list() {
        let ended = call.end_or(now_ms);
        if !call.keep && !call.is_live() && ended < calls_from {
            if calls.delete(&call.id).is_ok() {
                swept.calls += 1;
            }
        } else {
            kept_calls.push(call);
        }
    }
    let audio_from_us = now_ms.saturating_sub(keep_audio_ms).saturating_mul(1000);
    for stream in Stream::ALL {
        let segs = segments(ring, stream);
        let n = segs.len();
        for (i, seg) in segs.iter().enumerate() {
            if i + 1 >= n {
                break;
            }
            let end_us = segs.get(i + 1).map_or(seg.end_us(), |next: &SegmentInfo| next.start_us.min(seg.end_us()));
            if end_us >= audio_from_us {
                break;
            }
            let (from_ms, to_ms) = (seg.start_us / 1000, end_us / 1000);
            if kept_calls.iter().any(|c| c.overlaps(from_ms, to_ms, CALL_MARGIN_MS, now_ms)) {
                continue;
            }
            let size = std::fs::metadata(&seg.path).map_or(0, |m| m.len());
            if std::fs::remove_file(&seg.path).is_ok() {
                let _ = std::fs::remove_file(seg.path.with_extension("lvl"));
                swept.files += 1;
                swept.bytes += size;
            }
        }
    }
    if swept.files > 0 || swept.calls > 0 {
        log::info!("kept for long enough: {} files of the recording and {} calls deleted ({} bytes)", swept.files, swept.calls, swept.bytes);
    }
    swept
}

/// Deletes the whole recording and every call (the owner asked).
pub fn delete_everything(ring: &Path, calls_root: &Path) -> std::io::Result<()> {
    for d in [ring, calls_root] {
        if d.exists() {
            std::fs::remove_dir_all(d)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use recallcraft_model::Call;

    use super::*;
    use crate::ring::{RingWriter, SEGMENT_US};

    #[test]
    fn old_sound_goes_unless_a_kept_call_needs_it() {
        let root = crate::temp_dir("retention");
        let ring = root.join("ring");
        let calls = CallStore::new(&root.join("calls"));
        let hour_ms = 3_600_000u64;
        let now_ms = 1_764_000_000_000 + 100 * hour_ms;
        let mut w = RingWriter::new(&ring);
        let tone: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.06).sin() * 0.3).collect();
        // Sound every five minutes for the last four days.
        let first_ms = now_ms - 96 * hour_ms;
        let mut t = first_ms;
        while t < now_ms {
            w.push(Stream::Mic, t * 1000, &tone).unwrap();
            t += SEGMENT_US / 1000;
        }
        w.close();
        let before = segments(&ring, Stream::Mic).len();
        // A call three days ago, kept by the owner; one two days ago, not; one an hour ago.
        let kept = Call {
            id: "kept".into(),
            started_ms: now_ms - 72 * hour_ms,
            ended_ms: Some(now_ms - 72 * hour_ms + 600_000),
            keep: true,
            ..Call::default()
        };
        let old = Call { id: "old".into(), started_ms: now_ms - 60 * hour_ms, ended_ms: Some(now_ms - 60 * hour_ms + 600_000), ..Call::default() };
        let recent = Call { id: "recent".into(), started_ms: now_ms - hour_ms, ended_ms: Some(now_ms - hour_ms + 60_000), ..Call::default() };
        for c in [&kept, &old, &recent] {
            calls.save(c).unwrap();
        }
        // Everything for 48 hours, calls for 2 days too.
        let s = sweep(&ring, &calls, 48 * hour_ms, 48 * hour_ms, now_ms);
        assert_eq!(s.calls, 1, "the old call went");
        assert!(calls.load("old").is_none() && calls.load("kept").is_some() && calls.load("recent").is_some());
        let after = segments(&ring, Stream::Mic);
        assert!(after.len() < before && s.files == before - after.len());
        // What's left: the last 48 hours, and the files around the kept call.
        let cutoff_us = (now_ms - 48 * hour_ms) * 1000;
        // (The file that runs past the cut-off is kept whatever is in it.)
        let around: Vec<&SegmentInfo> = after.iter().filter(|x| x.start_us + SEGMENT_US < cutoff_us).collect();
        assert!(!around.is_empty() && around.len() <= 5, "{}", around.len());
        assert!(around.iter().all(|x| kept.overlaps(x.start_us / 1000, x.end_us() / 1000, 61_000, now_ms)));
        // Sweeping again deletes nothing more.
        assert_eq!(sweep(&ring, &calls, 48 * hour_ms, 48 * hour_ms, now_ms), Swept::default());
        delete_everything(&ring, &root.join("calls")).unwrap();
        assert!(segments(&ring, Stream::Mic).is_empty() && calls.list().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
