//! Voices: those the owner named, known again on later calls (`voices.json` in the settings
//! folder), and those heard on each call (`voices.json` in the call's folder, so a name given to
//! one in the window can be learned).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use recallcraft_speech::voices::{self, KnownVoice};
use serde::{Deserialize, Serialize};

const FILE: &str = "voices.json";
const MAX: u64 = 4 * 1024 * 1024;

fn read<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    let Ok(meta) = std::fs::metadata(path) else { return T::default() };
    if meta.len() > MAX {
        return T::default();
    }
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write<T: Serialize>(path: &Path, v: &T) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string(v) {
        Ok(t) => {
            if let Err(e) = recallcraft_store::write_atomic(path, t.as_bytes()) {
                log::warn!("{}: {e}", path.display());
            }
        }
        Err(e) => log::warn!("voices: {e}"),
    }
}

pub fn library_path(settings_dir: &Path) -> PathBuf {
    settings_dir.join(FILE)
}

/// The voices the owner named.
pub fn library(settings_dir: &Path) -> Vec<KnownVoice> {
    read::<Vec<KnownVoice>>(&library_path(settings_dir)).into_iter().filter(|k| k.v.len() == voices::DIMS && !k.name.trim().is_empty()).collect()
}

pub fn save_library(settings_dir: &Path, known: &[KnownVoice]) {
    write(&library_path(settings_dir), &known);
}

/// A voice heard on a call: its fingerprint and how much speech it's from.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CallVoice {
    pub v: Vec<f32>,
    pub frames: f32,
}

/// The voices of a call, by number.
pub fn call_voices(call_dir: &Path) -> BTreeMap<u8, CallVoice> {
    read(&call_dir.join(FILE))
}

pub fn save_call_voices(call_dir: &Path, v: &BTreeMap<u8, CallVoice>) {
    write(&call_dir.join(FILE), v);
}

/// The owner named voice `n` of the call in `call_dir`: it's learned, to be known again.
pub fn learn_from_call(settings_dir: &Path, call_dir: &Path, n: u8, name: &str) {
    let Some(cv) = call_voices(call_dir).remove(&n) else { return };
    let mut known = library(settings_dir);
    voices::learn(&mut known, name, &cv.v, cv.frames);
    save_library(settings_dir, &known);
}

/// Forgets every voice learned.
pub fn forget_all(settings_dir: &Path) {
    let _ = std::fs::remove_file(library_path(settings_dir));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_voice_is_learned_and_known_again() {
        let base = std::env::temp_dir().join(format!("recallcraft-known-{}", std::process::id()));
        let (settings, call) = (base.join("settings"), base.join("call"));
        let fp: Vec<f32> = (0..voices::DIMS).map(|i| i as f32 / 10.0).collect();
        let mut cv = BTreeMap::new();
        cv.insert(2u8, CallVoice { v: fp.clone(), frames: 500.0 });
        save_call_voices(&call, &cv);
        assert_eq!(call_voices(&call), cv);
        learn_from_call(&settings, &call, 2, "John Smith");
        learn_from_call(&settings, &call, 7, "Nobody");
        let lib = library(&settings);
        assert_eq!(lib.len(), 1);
        assert_eq!(voices::recognise(&lib, &fp).map(|k| k.name.as_str()), Some("John Smith"));
        std::fs::write(library_path(&settings), "junk").unwrap();
        assert!(library(&settings).is_empty());
        forget_all(&settings);
        let _ = std::fs::remove_dir_all(&base);
    }
}
