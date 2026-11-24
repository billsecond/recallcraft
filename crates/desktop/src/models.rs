//! The speech model's file: `whisper-base.en-q8_0.gguf`, installed with RecallCraft and never
//! downloaded. Looked for, in order: the file `RECALLCRAFT_SPEECH_MODEL` names; the `models`
//! folder in the settings folder (where the owner can put another `whisper….gguf`); `models`
//! beside the program; `../share/recallcraft/models` (Linux) and `../Resources/models` (a macOS
//! app); and, for a build in `target/`, `assets/models` in the source tree.

use std::ffi::OsStr;
use std::path::PathBuf;

/// The model installed with RecallCraft.
pub const SPEECH: &str = "whisper-base.en-q8_0.gguf";

/// The folders a model is looked for in, in order.
pub fn model_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(s) = crate::paths::settings_dir() {
        dirs.push(s.join("models"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(bin) = exe.parent()
    {
        dirs.push(bin.join("models"));
        dirs.push(bin.join("..").join("share").join("recallcraft").join("models"));
        dirs.push(bin.join("..").join("Resources").join("models"));
        // A build in the source tree: target/release, target/<triple>/release, or deeper where a
        // test runner keeps its programs (target/debug/build/<crate>/out).
        for up in bin.ancestors().skip(1).take(6) {
            dirs.push(up.join("assets").join("models"));
        }
    }
    dirs
}

/// The first `whisper….gguf` in `dirs` (in a folder, the installed one first, then by name).
pub fn find_speech(dirs: &[PathBuf]) -> Option<PathBuf> {
    for d in dirs {
        let Ok(entries) = std::fs::read_dir(d) else { continue };
        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("gguf")))
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_lowercase().starts_with("whisper")))
            .collect();
        files.sort();
        files.sort_by_key(|p| p.file_name() != Some(OsStr::new(SPEECH)));
        if let Some(f) = files.into_iter().next() {
            return Some(f);
        }
    }
    None
}

/// The speech model's file, or why there's none.
pub fn speech_model() -> Result<PathBuf, String> {
    if let Some(f) = std::env::var_os("RECALLCRAFT_SPEECH_MODEL").filter(|f| !f.is_empty()) {
        return Ok(PathBuf::from(f));
    }
    find_speech(&model_dirs()).ok_or_else(|| format!("the speech model isn't installed with this copy of RecallCraft ({SPEECH})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_installed_model_is_found_first() {
        let base = std::env::temp_dir().join(format!("recallcraft-models-{}", std::process::id()));
        let (a, b) = (base.join("a"), base.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("llama.gguf"), b"").unwrap();
        std::fs::write(b.join("whisper-tiny.gguf"), b"").unwrap();
        std::fs::write(b.join(SPEECH), b"").unwrap();
        let dirs = vec![base.join("missing"), a.clone(), b.clone()];
        assert_eq!(find_speech(&dirs), Some(b.join(SPEECH)));
        std::fs::remove_file(b.join(SPEECH)).unwrap();
        assert_eq!(find_speech(&dirs), Some(b.join("whisper-tiny.gguf")));
        assert_eq!(find_speech(&[a]), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_source_tree_s_model_is_found() {
        // Wherever the test program itself runs from (GitHub's runners keep it deeper than
        // target/…/deps), the source tree's models folder has the installed model.
        let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("assets").join("models");
        let found = find_speech(&[assets]);
        assert!(found.as_ref().is_some_and(|p| p.ends_with(SPEECH) && p.exists()), "{found:?}");
    }
}
