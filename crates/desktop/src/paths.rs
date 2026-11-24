//! Where things are kept.
//!
//! - Settings and the log: `%APPDATA%\RecallCraft` on Windows, `~/Library/Application
//!   Support/RecallCraft` on macOS, `~/.config/recallcraft` elsewhere.
//! - The recording and the calls (they're large, and stay on this computer): `%LOCALAPPDATA%\RecallCraft`
//!   on Windows, `~/Library/Application Support/RecallCraft/Data` on macOS,
//!   `~/.local/share/recallcraft` elsewhere, unless the settings name another folder.
//! - Exports: `Documents/RecallCraft`, unless the settings name another folder.
//!
//! `RECALLCRAFT_NO_PREFS=1` makes a run that keeps no settings (tests, CI): nothing is registered
//! to start at sign-in, no instance lock is taken, and everything is in a temporary folder.

use std::path::PathBuf;

use recallcraft_model::Settings;

pub const APP: &str = "RecallCraft";

/// Whether this run keeps no settings.
pub fn no_prefs() -> bool {
    std::env::var_os("RECALLCRAFT_NO_PREFS").is_some_and(|v| !v.is_empty() && v != "0")
}

fn temp() -> PathBuf {
    std::env::temp_dir().join("RecallCraft-no-prefs")
}

/// The settings folder (not created).
pub fn settings_dir() -> Option<PathBuf> {
    if no_prefs() {
        return Some(temp().join("settings"));
    }
    let base = dirs::config_dir()?;
    Some(if cfg!(any(target_os = "windows", target_os = "macos")) { base.join(APP) } else { base.join("recallcraft") })
}

/// The settings folder, created if it isn't there.
pub fn ensure_settings_dir() -> Option<PathBuf> {
    let dir = settings_dir()?;
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("could not create {}: {e}", dir.display());
        return None;
    }
    Some(dir)
}

/// The data folder the system gives (where the settings don't name one).
pub fn default_data_dir() -> PathBuf {
    if no_prefs() {
        return temp().join("data");
    }
    if cfg!(target_os = "macos") {
        return dirs::data_dir().map(|d| d.join(APP).join("Data")).unwrap_or_else(|| temp().join("data"));
    }
    match dirs::data_local_dir() {
        Some(d) if cfg!(target_os = "windows") => d.join(APP),
        Some(d) => d.join("recallcraft"),
        None => temp().join("data"),
    }
}

/// Where the recording and the calls are kept.
pub fn data_dir(settings: &Settings) -> PathBuf {
    let f = settings.data_folder.trim();
    if f.is_empty() { default_data_dir() } else { PathBuf::from(f) }
}

/// Where exports go.
pub fn exports_dir(settings: &Settings) -> PathBuf {
    let f = settings.export.folder.trim();
    if !f.is_empty() {
        return PathBuf::from(f);
    }
    dirs::document_dir().or_else(|| dirs::home_dir().map(|h| h.join("Documents"))).unwrap_or_else(std::env::temp_dir).join(APP)
}

/// The log folder.
pub fn log_dir() -> Option<PathBuf> {
    settings_dir().map(|d| d.join("logs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_follow_the_settings() {
        let s = Settings { data_folder: "  D:\\Recall  ".into(), ..Settings::default() };
        assert_eq!(data_dir(&s), PathBuf::from("D:\\Recall"));
        assert_eq!(data_dir(&Settings::default()), default_data_dir());
        let e = Settings { export: recallcraft_model::ExportSettings { folder: "E:\\Out".into(), ..Default::default() }, ..Settings::default() };
        assert_eq!(exports_dir(&e), PathBuf::from("E:\\Out"));
        assert!(exports_dir(&Settings::default()).ends_with(APP));
    }
}
