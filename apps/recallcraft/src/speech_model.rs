//! The speech model the owner chose (Settings › Calls): Base is installed; Small and Medium are
//! downloaded when chosen, on a thread of their own, checked against the SHA-256 they were
//! published with, and handed to the hearer once they're here (it takes them up between calls).
//! Until then the installed one hears. How a download goes is in the status the window reads.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use recallcraft_desktop::models;
use recallcraft_engine::worker::HearWith;
use recallcraft_engine::{NextModel, Shared};
use recallcraft_model::SpeechModel;

struct Fetch {
    model: SpeechModel,
    got: Arc<AtomicU64>,
    whole: u64,
    cancel: Arc<AtomicBool>,
    done: Receiver<Result<PathBuf, String>>,
}

pub struct Chooser {
    data_dir: PathBuf,
    /// The choice last acted on.
    synced: Option<SpeechModel>,
    fetch: Option<Fetch>,
    cores: usize,
}

/// `n` bytes in MB.
fn mb(n: u64) -> u64 {
    (n + 500_000) / 1_000_000
}

/// How a download is going, in words.
pub fn progress_words(m: SpeechModel, got: u64, whole: u64) -> String {
    let pct = (got.min(whole) * 100).checked_div(whole).unwrap_or(0);
    format!("Downloading {}: {pct}% ({} of {} MB)", m.name(), mb(got), mb(whole))
}

impl Chooser {
    pub fn new(data_dir: &Path) -> Chooser {
        let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
        Chooser { data_dir: data_dir.to_path_buf(), synced: None, fetch: None, cores }
    }

    /// How the engine hears at the start: the chosen model if it's here, else the installed one;
    /// and the name of the one it is.
    pub fn hear_with(&self, want: SpeechModel) -> (HearWith, &'static str) {
        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(e) => return (HearWith::Off(e.to_string()), ""),
        };
        match models::speech_model_for(want, &self.data_dir) {
            Ok((model, used)) => (HearWith::Program { exe, args: vec!["--hear".to_string()], model, threads: used.threads(self.cores) }, used.name()),
            Err(e) => {
                log::warn!("{e}; what's said won't be written down");
                (HearWith::Off("no speech model".to_string()), "")
            }
        }
    }

    /// Follows the setting: the chosen model is used if it's here, downloaded if not. `again`
    /// tries a download that failed once more.
    pub fn sync(&mut self, want: SpeechModel, again: bool, shared: &Shared) {
        if self.synced == Some(want) && !again {
            return;
        }
        self.synced = Some(want);
        if let Some(f) = self.fetch.take_if(|f| f.model != want) {
            f.cancel.store(true, Ordering::SeqCst);
            log::info!("the {} speech model's download stopped (another was chosen)", f.model.name());
        }
        models::tidy(want, &self.data_dir);
        if want == SpeechModel::Base || models::downloaded(want, &self.data_dir).is_some() {
            self.hand_over(want, shared);
            shared.update(|s| s.speech_download.clear());
            return;
        }
        if self.fetch.is_some() {
            return;
        }
        // Until it's here, the installed one hears.
        self.hand_over(SpeechModel::Base, shared);
        let Some(p) = models::published(want) else { return };
        let (tx, rx) = channel();
        let got = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (g, c, data) = (got.clone(), cancel.clone(), self.data_dir.clone());
        log::info!("downloading the {} speech model", want.name());
        let started = std::thread::Builder::new().name("speech model".into()).spawn(move || {
            let r = models::download(want, &data, &mut |n, _| g.store(n, Ordering::Relaxed), &c);
            let _ = tx.send(r);
        });
        match started {
            Ok(_) => {
                shared.update(|s| s.speech_download = progress_words(want, 0, p.bytes));
                self.fetch = Some(Fetch { model: want, got, whole: p.bytes, cancel, done: rx });
            }
            Err(e) => shared.update(|s| s.speech_download = format!("Couldn't download {}: {e}", want.name())),
        }
    }

    /// The hearer hears with `m` (the installed one if `m` isn't here) from the next quiet moment.
    fn hand_over(&self, m: SpeechModel, shared: &Shared) {
        match models::speech_model_for(m, &self.data_dir) {
            Ok((file, used)) => shared.use_model(NextModel { file, threads: used.threads(self.cores), name: used.name().to_string() }),
            Err(e) => log::warn!("{e}"),
        }
    }

    /// Looks at the download going on (twice a second).
    pub fn tick(&mut self, shared: &Shared) {
        let Some(f) = &self.fetch else { return };
        match f.done.try_recv() {
            Err(TryRecvError::Empty) => {
                let words = progress_words(f.model, f.got.load(Ordering::Relaxed), f.whole);
                shared.update(|s| s.speech_download = words);
            }
            Ok(Ok(_)) => {
                let m = f.model;
                self.fetch = None;
                self.hand_over(m, shared);
                shared.update(|s| s.speech_download.clear());
            }
            Ok(Err(e)) => {
                let m = f.model;
                log::warn!("the {} speech model couldn't be downloaded: {e}", m.name());
                self.fetch = None;
                shared.update(|s| s.speech_download = format!("Couldn't download {}: {e}", m.name()));
            }
            Err(TryRecvError::Disconnected) => {
                self.fetch = None;
                shared.update(|s| s.speech_download.clear());
            }
        }
    }

    /// Stops a download going on (the service stops).
    pub fn shut_down(&mut self) {
        if let Some(f) = self.fetch.take() {
            f.cancel.store(true, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_s_progress_is_told_in_words() {
        assert_eq!(progress_words(SpeechModel::Small, 134_000_000, 268_599_712), "Downloading Small: 49% (134 of 269 MB)");
        assert_eq!(progress_words(SpeechModel::Medium, 0, 0), "Downloading Medium: 0% (0 of 0 MB)");
        assert_eq!(progress_words(SpeechModel::Medium, 900, 100), "Downloading Medium: 100% (0 of 0 MB)");
    }
}
