//! The speech model's file. Base (`whisper-base.en-q8_0.gguf`) is installed with RecallCraft;
//! Small and Medium are downloaded from the repository's `speech-models` release when the owner
//! chooses them ([`download`]), into `models` in the data folder, and checked against the SHA-256
//! they were published with. The chosen one is used once it's there ([`speech_model_for`]);
//! until then, the installed one, looked for in order: the file `RECALLCRAFT_SPEECH_MODEL` names;
//! the `models` folder in the settings folder (where the owner can put another `whisper….gguf`);
//! `models` beside the program; `../share/recallcraft/models` (Linux) and `../Resources/models`
//! (a macOS app); and, for a build in `target/`, `assets/models` in the source tree.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use recallcraft_model::SpeechModel;

/// Where the larger models are published.
pub const MODELS_RELEASE: &str = "https://github.com/billsecond/recallcraft/releases/download/speech-models/";

/// A model that's downloaded: its size and SHA-256 as published.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Published {
    pub bytes: u64,
    pub sha256: &'static str,
}

/// What a model that's downloaded was published as; None for the installed one.
pub fn published(m: SpeechModel) -> Option<Published> {
    match m {
        SpeechModel::Base => None,
        SpeechModel::Small => Some(Published { bytes: 268_599_712, sha256: "8fc18e0736d8b486dfe65a6f780cb75a8ca2684ab782e1d381c73160662ff326" }),
        SpeechModel::Medium => Some(Published { bytes: 830_393_856, sha256: "eccef047c9a63db6e6c38ae511c0771df8563d41c294ca762755a6b156d7aea5" }),
    }
}

/// Where downloaded models are kept: `models` in the data folder.
pub fn downloads_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("models")
}

/// The chosen model's file, if it has been downloaded (whole: its size is checked; its SHA-256
/// was, as it came).
pub fn downloaded(m: SpeechModel, data_dir: &Path) -> Option<PathBuf> {
    let p = published(m)?;
    let f = downloads_dir(data_dir).join(m.file());
    std::fs::metadata(&f).is_ok_and(|md| md.is_file() && md.len() == p.bytes).then_some(f)
}

/// The model to hear with: the chosen one if it's here, else the installed one. Says which it
/// is.
pub fn speech_model_for(m: SpeechModel, data_dir: &Path) -> Result<(PathBuf, SpeechModel), String> {
    if std::env::var_os("RECALLCRAFT_SPEECH_MODEL").is_none_or(|f| f.is_empty())
        && let Some(f) = downloaded(m, data_dir)
    {
        return Ok((f, m));
    }
    speech_model().map(|f| (f, SpeechModel::Base))
}

/// Downloads the model `m` into the data folder and checks it, telling `progress` the bytes so
/// far and the whole; stopped when `cancel` is set. One already there is kept.
pub fn download(m: SpeechModel, data_dir: &Path, progress: &mut dyn FnMut(u64, u64), cancel: &AtomicBool) -> Result<PathBuf, String> {
    let p = published(m).ok_or("that model is installed with RecallCraft")?;
    if let Some(f) = downloaded(m, data_dir) {
        return Ok(f);
    }
    let expected = parse_sha(p.sha256).ok_or("a model's checksum is malformed")?;
    let dir = downloads_dir(data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let dest = dir.join(m.file());
    // Others of the larger models aren't kept: one is used at a time.
    for other in SpeechModel::ALL.into_iter().filter(|o| *o != m && published(*o).is_some()) {
        let _ = std::fs::remove_file(dir.join(other.file()));
    }
    let url = format!("{MODELS_RELEASE}{}", m.file());
    // A slow line may need a while; each read still has a minute.
    let agent = crate::update::agent(Duration::from_secs(4 * 60 * 60));
    let mut resp = agent.get(&url).header("Accept", "application/octet-stream").call().map_err(crate::update::plain)?;
    let reader = resp.body_mut().with_config().limit(p.bytes).reader();
    let whole = p.bytes;
    let bytes = crate::update::save_verified_with(reader, &dest, whole, &expected, "speech model", &mut |n| progress(n, whole), cancel)?;
    log::info!("downloaded the {} speech model ({bytes} bytes), SHA-256 {}", m.name(), p.sha256);
    Ok(dest)
}

/// Deletes the larger models downloaded but not chosen.
pub fn tidy(chosen: SpeechModel, data_dir: &Path) {
    let dir = downloads_dir(data_dir);
    for m in SpeechModel::ALL.into_iter().filter(|m| *m != chosen && published(*m).is_some()) {
        for name in [m.file().to_string(), format!("{}.part", m.file())] {
            let f = dir.join(name);
            if f.is_file() {
                match std::fs::remove_file(&f) {
                    Ok(()) => log::info!("removed {} (another speech model is chosen)", f.display()),
                    Err(e) => log::warn!("{}: {e}", f.display()),
                }
            }
        }
    }
}

fn parse_sha(hex: &str) -> Option<[u8; 32]> {
    let mut out = [0u8; 32];
    if hex.len() != 64 {
        return None;
    }
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

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
    fn a_chosen_model_is_used_once_it_s_whole() {
        let data = std::env::temp_dir().join(format!("recallcraft-chosen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let dir = downloads_dir(&data);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(downloaded(SpeechModel::Small, &data), None);
        assert_eq!(downloaded(SpeechModel::Base, &data), None, "Base is never downloaded");
        // Half a file isn't used.
        std::fs::write(dir.join(SpeechModel::Small.file()), b"half").unwrap();
        assert_eq!(downloaded(SpeechModel::Small, &data), None);
        let f = std::fs::File::create(dir.join(SpeechModel::Small.file())).unwrap();
        f.set_len(published(SpeechModel::Small).unwrap().bytes).unwrap();
        drop(f);
        assert_eq!(downloaded(SpeechModel::Small, &data), Some(dir.join(SpeechModel::Small.file())));
        if std::env::var_os("RECALLCRAFT_SPEECH_MODEL").is_none() {
            assert_eq!(speech_model_for(SpeechModel::Small, &data).unwrap().1, SpeechModel::Small);
            // Not downloaded: the installed one, and it says so.
            assert_eq!(speech_model_for(SpeechModel::Medium, &data).unwrap().1, SpeechModel::Base);
        }
        // Choosing another clears the one not chosen.
        tidy(SpeechModel::Medium, &data);
        assert_eq!(downloaded(SpeechModel::Small, &data), None);
        let _ = std::fs::remove_dir_all(&data);
    }

    /// The published Small model downloads and checks out: a 270 MB download, so it's run by
    /// hand (`cargo test -p recallcraft-desktop --lib -- --ignored the_small_model_downloads`).
    #[test]
    #[ignore = "downloads 270 MB from GitHub"]
    fn the_small_model_downloads() {
        let data = std::env::temp_dir().join(format!("recallcraft-download-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let whole = published(SpeechModel::Small).unwrap().bytes;
        let mut last = 0;
        let f = download(
            SpeechModel::Small,
            &data,
            &mut |n, w| {
                assert!(n <= w && w == whole);
                last = n;
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(downloaded(SpeechModel::Small, &data), Some(f));
        assert_eq!(last, whole);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn the_published_checksums_are_well_formed() {
        for m in SpeechModel::ALL {
            if let Some(p) = published(m) {
                assert!(parse_sha(p.sha256).is_some(), "{m:?}");
                assert!(p.bytes > 100_000_000);
            }
        }
        assert_eq!(parse_sha("zz"), None);
    }

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
