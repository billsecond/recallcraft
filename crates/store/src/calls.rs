//! The calls: a folder each (`calls/<id>/`), with `call.json` (when, which app, the title, the
//! names) and `transcript.jsonl` (a line of JSON for each line said, appended as it's heard).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use recallcraft_model::call::slug;
use recallcraft_model::{Call, Line, Spoken, time};

const CALL: &str = "call.json";
const TRANSCRIPT: &str = "transcript.jsonl";
/// Who the meeting app showed speaking, and when.
const SPEAKING: &str = "speaking.jsonl";
const MAX_SPEAKING: u64 = 8 * 1024 * 1024;
/// The largest call file or transcript read.
const MAX_CALL: u64 = 1024 * 1024;
const MAX_TRANSCRIPT: u64 = 64 * 1024 * 1024;
/// The most calls listed.
const MAX_CALLS: usize = 20_000;

/// A new call's id: when it started, locally, and the app.
pub fn new_id(started_ms: u64, app: &str) -> String {
    let s = slug(app, 24);
    if s.is_empty() { time::local_stamp(started_ms) } else { format!("{}_{s}", time::local_stamp(started_ms)) }
}

/// Whether an id is one of ours (letters, digits, `-` and `_`): nothing else is ever a path.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[derive(Clone, Debug)]
pub struct CallStore {
    root: PathBuf,
}

impl CallStore {
    pub fn new(root: &Path) -> CallStore {
        CallStore { root: root.to_path_buf() }
    }

    pub fn dir(&self, id: &str) -> Option<PathBuf> {
        valid_id(id).then(|| self.root.join(id))
    }

    /// Every call, the latest first.
    pub fn list(&self) -> Vec<Call> {
        let Ok(entries) = std::fs::read_dir(&self.root) else { return Vec::new() };
        let mut calls: Vec<Call> = entries
            .flatten()
            .take(MAX_CALLS)
            .filter_map(|e| {
                let id = e.file_name().to_str()?.to_string();
                self.load(&id)
            })
            .collect();
        calls.sort_by(|a, b| b.started_ms.cmp(&a.started_ms).then_with(|| b.id.cmp(&a.id)));
        calls
    }

    pub fn load(&self, id: &str) -> Option<Call> {
        let path = self.dir(id)?.join(CALL);
        let meta = std::fs::metadata(&path).ok()?;
        if meta.len() > MAX_CALL {
            log::warn!("{} is too large to be a call; left out", path.display());
            return None;
        }
        let text = std::fs::read_to_string(&path).ok()?;
        match serde_json::from_str::<Call>(&text) {
            Ok(mut c) => {
                c.id = id.to_string();
                Some(c.checked())
            }
            Err(e) => {
                log::warn!("{}: {e}; left out", path.display());
                None
            }
        }
    }

    pub fn save(&self, call: &Call) -> std::io::Result<()> {
        let dir = self.dir(&call.id).ok_or_else(|| std::io::Error::other("not a call id"))?;
        std::fs::create_dir_all(&dir)?;
        let json = serde_json::to_string_pretty(call).map_err(std::io::Error::other)?;
        crate::write_atomic(&dir.join(CALL), json.as_bytes())
    }

    /// Adds lines to a call's transcript.
    pub fn append(&self, id: &str, lines: &[Line]) -> std::io::Result<()> {
        let dir = self.dir(id).ok_or_else(|| std::io::Error::other("not a call id"))?;
        std::fs::create_dir_all(&dir)?;
        let mut text = String::new();
        for l in lines {
            if let Ok(j) = serde_json::to_string(l) {
                text.push_str(&j);
                text.push('\n');
            }
        }
        let mut f = OpenOptions::new().create(true).append(true).open(dir.join(TRANSCRIPT))?;
        f.write_all(text.as_bytes())
    }

    /// A call's transcript, in the order it was said.
    pub fn lines(&self, id: &str) -> Vec<Line> {
        let Some(dir) = self.dir(id) else { return Vec::new() };
        let path = dir.join(TRANSCRIPT);
        let Ok(meta) = std::fs::metadata(&path) else { return Vec::new() };
        if meta.len() > MAX_TRANSCRIPT {
            log::warn!("{} is too large; left out", path.display());
            return Vec::new();
        }
        let Ok(text) = std::fs::read_to_string(&path) else { return Vec::new() };
        let mut lines: Vec<Line> = text.lines().filter_map(|l| serde_json::from_str::<Line>(l).ok()).filter_map(Line::checked).collect();
        lines.sort_by_key(|l| (l.start_ms, l.end_ms));
        lines
    }

    /// Adds stretches the meeting app showed someone speaking.
    pub fn append_speaking(&self, id: &str, spoken: &[Spoken]) -> std::io::Result<()> {
        let dir = self.dir(id).ok_or_else(|| std::io::Error::other("not a call id"))?;
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(SPEAKING);
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_SPEAKING) {
            return Ok(());
        }
        let mut text = String::new();
        for s in spoken {
            if let Ok(j) = serde_json::to_string(s) {
                text.push_str(&j);
                text.push('\n');
            }
        }
        let mut f = OpenOptions::new().create(true).append(true).open(path)?;
        f.write_all(text.as_bytes())
    }

    /// The stretches the meeting app showed someone speaking, in order.
    pub fn speaking(&self, id: &str) -> Vec<Spoken> {
        let Some(dir) = self.dir(id) else { return Vec::new() };
        let path = dir.join(SPEAKING);
        if std::fs::metadata(&path).map_or(true, |m| m.len() > MAX_SPEAKING) {
            return Vec::new();
        }
        let Ok(text) = std::fs::read_to_string(&path) else { return Vec::new() };
        let mut out: Vec<Spoken> = text
            .lines()
            .filter_map(|l| serde_json::from_str::<Spoken>(l).ok())
            .filter(|s| !s.name.trim().is_empty() && s.to_ms >= s.from_ms)
            .collect();
        out.sort_by_key(|s| (s.from_ms, s.to_ms));
        out
    }

    /// The transcript's size: it changes when lines are added.
    pub fn transcript_len(&self, id: &str) -> u64 {
        self.dir(id).and_then(|d| std::fs::metadata(d.join(TRANSCRIPT)).ok()).map_or(0, |m| m.len())
    }

    /// Writes the whole transcript again (the owner edited it).
    pub fn replace_lines(&self, id: &str, lines: &[Line]) -> std::io::Result<()> {
        let dir = self.dir(id).ok_or_else(|| std::io::Error::other("not a call id"))?;
        std::fs::create_dir_all(&dir)?;
        let mut text = String::new();
        for l in lines {
            if let Ok(j) = serde_json::to_string(l) {
                text.push_str(&j);
                text.push('\n');
            }
        }
        crate::write_atomic(&dir.join(TRANSCRIPT), text.as_bytes())
    }

    pub fn delete(&self, id: &str) -> std::io::Result<()> {
        let dir = self.dir(id).ok_or_else(|| std::io::Error::other("not a call id"))?;
        if dir.exists() { std::fs::remove_dir_all(dir) } else { Ok(()) }
    }

    /// Calls whose transcript has all of `words` (any case), the latest first, with the lines
    /// that matched.
    pub fn search(&self, words: &str) -> Vec<(Call, Vec<Line>)> {
        let needles: Vec<String> = words.split_whitespace().map(str::to_lowercase).collect();
        if needles.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for call in self.list() {
            let title = call.display_title().to_lowercase();
            let lines = self.lines(&call.id);
            let all: String = lines.iter().map(|l| l.text.to_lowercase()).collect::<Vec<_>>().join(" ");
            let names: String = call.names.values().chain(&call.people).map(|n| n.to_lowercase()).collect::<Vec<_>>().join(" ");
            if !needles.iter().all(|n| all.contains(n) || title.contains(n) || names.contains(n)) {
                continue;
            }
            let hits: Vec<Line> = lines.into_iter().filter(|l| needles.iter().any(|n| l.text.to_lowercase().contains(n))).collect();
            out.push((call, hits));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use recallcraft_model::Speaker;

    use super::*;

    #[test]
    fn calls_are_kept_listed_searched_and_deleted() {
        let root = crate::temp_dir("calls");
        let store = CallStore::new(&root);
        let a = Call {
            id: new_id(1_764_000_000_000, "Microsoft Teams"),
            app: "Microsoft Teams".into(),
            started_ms: 1_764_000_000_000,
            ..Call::default()
        };
        let b = Call {
            id: new_id(1_764_000_900_000, "Zoom"),
            app: "Zoom".into(),
            title: "Budget".into(),
            started_ms: 1_764_000_900_000,
            ..Call::default()
        };
        assert!(a.id.ends_with("_microsoft-teams") && valid_id(&a.id), "{}", a.id);
        store.save(&a).unwrap();
        store.save(&b).unwrap();
        let line = |s: u64, sp: Speaker, t: &str| Line { start_ms: s, end_ms: s + 1000, speaker: sp, text: t.into() };
        store.append(&a.id, &[line(1_764_000_005_000, Speaker::Others, "Are you the developer on this?")]).unwrap();
        store.append(&a.id, &[line(1_764_000_002_000, Speaker::Me, "Hello everyone")]).unwrap();
        let lines = store.lines(&a.id);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Hello everyone", "in the order said");
        let list = store.list();
        assert_eq!(list.iter().map(|c| c.id.clone()).collect::<Vec<_>>(), vec![b.id.clone(), a.id.clone()]);
        let found = store.search("DEVELOPER");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1.len(), 1);
        assert_eq!(store.search("budget").len(), 1, "the title counts");
        assert!(store.search("  ").is_empty());
        store.replace_lines(&a.id, &lines[..1]).unwrap();
        assert_eq!(store.lines(&a.id).len(), 1);
        store.delete(&a.id).unwrap();
        assert_eq!(store.list().len(), 1);
        // Ids that aren't ours are never paths.
        for bad in ["", "..", "../x", "a/b", "c:\\x", "x y"] {
            assert!(store.dir(bad).is_none(), "{bad}");
            assert!(store.load(bad).is_none());
            assert!(store.delete(bad).is_err());
        }
        // Damaged files are left out.
        let junk = root.join("junk");
        std::fs::create_dir_all(&junk).unwrap();
        std::fs::write(junk.join(CALL), "{nope").unwrap();
        std::fs::write(root.join(&b.id).join(TRANSCRIPT), "{\"bad\"\n\n{\"start_ms\":1,\"end_ms\":2,\"speaker\":\"me\",\"text\":\"ok\"}\n").unwrap();
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.lines(&b.id).len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }
}
