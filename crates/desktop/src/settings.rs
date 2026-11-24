//! The settings file (`settings.json` in the settings folder). A file that's missing or can't be
//! read gives the defaults; writing goes through a temporary file. The service and the window
//! both write it, each changing only what it means to: [`update`] reads it again first.

use std::path::PathBuf;
use std::time::SystemTime;

use recallcraft_model::Settings;

use crate::paths;

const FILE: &str = "settings.json";
const MAX: u64 = 4 * 1024 * 1024;

pub fn path() -> Option<PathBuf> {
    paths::settings_dir().map(|d| d.join(FILE))
}

pub fn load() -> Settings {
    let Some(p) = path() else { return Settings::default() };
    match std::fs::metadata(&p) {
        Ok(m) if m.len() <= MAX => std::fs::read_to_string(&p).map(|t| Settings::from_json(&t)).unwrap_or_default(),
        Ok(_) => {
            log::warn!("{} is too large to be settings; the defaults are used", p.display());
            Settings::default()
        }
        Err(_) => Settings::default(),
    }
}

pub fn save(s: &Settings) {
    let Some(dir) = paths::ensure_settings_dir() else { return };
    let p = dir.join(FILE);
    match serde_json::to_string_pretty(s) {
        Ok(json) => {
            if let Err(e) = recallcraft_store::write_atomic(&p, json.as_bytes()) {
                log::warn!("saving {}: {e}", p.display());
            }
        }
        Err(e) => log::warn!("settings: {e}"),
    }
}

/// Changes the settings as they are on disk now, and saves them.
pub fn update(f: impl FnOnce(&mut Settings)) -> Settings {
    let mut s = load();
    f(&mut s);
    let s = s.checked();
    save(&s);
    s
}

/// When the file was last written (to notice another program changing it).
pub fn modified() -> Option<SystemTime> {
    std::fs::metadata(path()?).ok()?.modified().ok()
}
