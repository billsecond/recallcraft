//! Updates in the service: the automatic check (a minute after the start, then every six hours,
//! never during a call), the notice when a newer release is found, the tray's items, and what
//! the window's settings show. The network and the installing are
//! `recallcraft_desktop::update`'s; this is when, and what the owner sees.
//!
//! On Windows, in a copy the installer put there or a portable copy, a newer release's MSI (or
//! portable zip) is downloaded and checked before the owner hears of it, and Update now installs
//! it (for a copy installed for everyone, Windows asks for permission; the service starts again
//! afterwards). Never during a call: Update now pressed during one installs it when the call
//! ends. Elsewhere the notice opens the release page.
//!
//! The service has no window, so it writes what it knows to [`state_file`] for the window's
//! settings and for the notice ([`crate::browser::notice`]), a small window of its own in the
//! corner; they answer with commands handed to the service (`--check-updates`,
//! `--install-update`, `--dismiss-update`).

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use recallcraft_desktop::update::{self, Found, Installer, Installing, Schedule, Version, Way};
use recallcraft_desktop::{paths, version};
use serde::{Deserialize, Serialize};

/// How long the notice of an update stays up (longer while the pointer is on it).
const OFFER_SECONDS: u64 = 30;
/// And a notice that only tells something.
const INFO_SECONDS: u64 = 12;
/// How long an install is waited for before Update now may be pressed again.
const INSTALL_WAIT: Duration = Duration::from_secs(30 * 60);

/// What the window's settings and the notice read: written by the service as it changes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shared {
    /// This program's version.
    pub current: String,
    /// This copy installs its updates itself (else the release page is the way).
    pub can_install: bool,
    /// Windows asks for permission to install them (installed for everyone on this PC).
    pub asks_permission: bool,
    /// What happens when an update is installed, in words.
    pub install_note: String,
    /// The Flatpak: no network at all, so no checks.
    pub flatpak: bool,
    /// A check is being made, or waits for the call to end.
    pub checking: bool,
    /// What the last check found, in words ("" before one).
    pub status: String,
    /// It's a failure.
    pub failed: bool,
    pub offer: Option<SharedOffer>,
    /// The installer is running.
    pub installing: bool,
    /// Update now was pressed during a call: it's installed when the call ends.
    pub after_call: bool,
    /// The notice up in the corner, if one is (never during a call).
    pub notice: Option<Notice>,
}

/// A newer release, as the window shows it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SharedOffer {
    pub version: String,
    pub page: String,
    /// It's downloaded and checked: Update now installs it.
    pub ready: bool,
    /// Why it isn't, although this copy installs its updates (a failed download).
    pub problem: Option<String>,
}

/// The notice in the corner.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notice {
    /// Each notice its own number: the notice's window closes when the service's is another.
    pub id: u64,
    pub title: String,
    pub detail: String,
    /// An update to install or get (Later puts it off), rather than something to know.
    pub offer: bool,
    /// Update now installs it (else Get it opens its page).
    pub ready: bool,
    /// The release page, when there's one to see.
    pub page: Option<String>,
    /// How long it stays up, in seconds (longer while the pointer is on it).
    pub seconds: u64,
}

/// Where the service writes [`Shared`]: beside the downloads, in the temporary folder.
pub fn state_file() -> PathBuf {
    update::updates_dir().join("state.json")
}

/// What the service last wrote; None when there's nothing (or nothing readable).
pub fn read_shared() -> Option<Shared> {
    let text = std::fs::read_to_string(state_file()).ok()?;
    if text.len() > 64 * 1024 {
        return None;
    }
    serde_json::from_str(&text).ok()
}

/// Who asked for a check, and so who hears the answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asker {
    /// The automatic check: the owner hears only of an update.
    Timer,
    /// Check for Updates… in the tray (or `recallcraft --check-updates`): the answer is a
    /// notice, whatever it is.
    Tray,
    /// Check now in the settings: the answer is shown there.
    Settings,
}

/// What the check's thread reports.
enum Report {
    Downloading(Version),
    Done(Result<Outcome, String>),
}

enum Outcome {
    UpToDate(Option<Version>),
    Offer(Offer),
}

/// A newer release, waiting for the owner.
#[derive(Clone, Debug)]
struct Offer {
    version: Version,
    page: String,
    /// The checked installer; without it the release page is the way.
    installer: Option<Installer>,
    /// Why there is no installer although this copy installs its updates.
    problem: Option<String>,
}

/// What the settings say of the last check.
#[derive(Clone, Debug, Default)]
enum Status {
    #[default]
    Idle,
    Checking,
    Downloading(Version),
    UpToDate(Option<Version>),
    Failed(String),
}

pub struct Updates {
    schedule: Schedule,
    job: Option<(Asker, Receiver<Report>)>,
    /// A check asked for, started once no call is going on.
    wanted: Option<Asker>,
    status: Status,
    offer: Option<Offer>,
    /// Later was chosen for this version: no notice of it again in this run.
    dismissed: Option<Version>,
    installing: Option<(Installing, Instant)>,
    /// How this copy updates itself, if it does.
    way: Option<Way>,
    asks_permission: bool,
    /// Checks happen by themselves (not in a run without settings, nor in the Flatpak).
    automatic: bool,
    /// The Flatpak: no network at all.
    flatpak: bool,
    /// The version an update installed just before this start, to be told once.
    just_updated: Option<Version>,
    /// The version an update was to install just before this start, and didn't.
    not_updated: Option<Version>,
    /// Update now was pressed during a call.
    after_call: bool,
    /// The notice to show (held back during a call), and the window showing it.
    notice: Option<Notice>,
    notice_window: Option<Child>,
    next_id: u64,
    written: Option<Shared>,
    /// The helper waits for RecallCraft to close (a portable copy): the service quits now.
    pub quit: bool,
}

/// What happens when an update is installed.
fn install_note(way: Option<&Way>, asks: bool) -> &'static str {
    match way {
        Some(Way::Portable(_)) => "RecallCraft closes for a moment while it's updated, then starts again.",
        _ if asks => "Windows asks for permission, then RecallCraft starts again.",
        _ => "RecallCraft closes while it installs, then starts again.",
    }
}

fn current_text() -> String {
    update::current().map(|v| v.to_string()).unwrap_or_else(|| version::version().to_string())
}

/// What the tray's item for an offer says.
fn tray_label(offer: Option<&Offer>, after_call: bool) -> Option<String> {
    offer.map(|o| match (&o.installer, after_call) {
        (Some(_), true) => format!("Install Update {} (after this call)", o.version),
        (Some(_), false) => format!("Install Update {}", o.version),
        (None, _) => format!("Get Update {}…", o.version),
    })
}

/// The check itself (on its own thread): the latest release, and what updates this copy.
fn run(way: Option<Way>, tx: &Sender<Report>) -> Result<Outcome, String> {
    let current = update::current().ok_or("this program's version can't be compared with a release's")?;
    match update::check(&current)? {
        Found::UpToDate(latest) => {
            match &latest {
                Some(v) => log::info!("update check: up to date ({current}; the latest release is {v})"),
                None => log::info!("update check: up to date ({current}; no release to offer)"),
            }
            Ok(Outcome::UpToDate(latest))
        }
        Found::Newer(release) => {
            log::info!("update check: RecallCraft {} is available (this is {current})", release.version);
            let mut offer = Offer { version: release.version.clone(), page: release.page(), installer: None, problem: None };
            if let Some(way) = &way {
                let _ = tx.send(Report::Downloading(release.version.clone()));
                let dir = update::updates_dir();
                match update::download(&release, &dir, std::env::consts::OS, std::env::consts::ARCH, way) {
                    Ok(i) => {
                        update::prune(&dir, Some(&i.path));
                        offer.installer = Some(i);
                    }
                    Err(e) => {
                        log::warn!("downloading the update: {e}");
                        offer.problem = Some(e);
                    }
                }
            }
            Ok(Outcome::Offer(offer))
        }
    }
}

impl Updates {
    pub fn new() -> Updates {
        let way = update::install_way();
        let dir = update::updates_dir();
        let (mut just_updated, mut not_updated) = (None, None);
        if let Some(current) = update::current() {
            match update::take_marker(&dir) {
                Some(v) if v == current => just_updated = Some(v),
                Some(v) => {
                    log::warn!("the update to {v} wasn't installed; this is still {current}");
                    not_updated = Some(v);
                }
                None => {}
            }
            update::tidy(&dir, &current);
        }
        let mut u = Updates::blank(way);
        u.flatpak = std::env::var_os("FLATPAK_ID").is_some();
        u.automatic = !paths::no_prefs() && !u.flatpak;
        u.just_updated = just_updated;
        u.not_updated = not_updated;
        u
    }

    /// Nothing checked yet, nothing to tell; no automatic checks.
    fn blank(way: Option<Way>) -> Updates {
        let asks_permission = match &way {
            Some(Way::Msi(scope)) => scope.asks_permission(),
            _ => false,
        };
        Updates {
            schedule: Schedule::new(Instant::now()),
            job: None,
            wanted: None,
            status: Status::Idle,
            offer: None,
            dismissed: None,
            installing: None,
            way,
            asks_permission,
            automatic: false,
            flatpak: false,
            just_updated: None,
            not_updated: None,
            after_call: false,
            notice: None,
            notice_window: None,
            next_id: 1,
            written: None,
            quit: false,
        }
    }

    /// The tray's item for a waiting update, if there's one.
    pub fn tray_label(&self) -> Option<String> {
        tray_label(self.offer.as_ref(), self.after_call)
    }

    fn info(&mut self, title: impl Into<String>, detail: impl Into<String>, page: Option<String>) {
        self.set_notice(Notice { title: title.into(), detail: detail.into(), page, seconds: INFO_SECONDS, ..Notice::default() });
    }

    fn set_notice(&mut self, mut n: Notice) {
        n.id = self.next_id;
        self.next_id += 1;
        self.notice = Some(n);
    }

    fn offer_notice(&mut self) {
        let Some(o) = self.offer.clone() else { return };
        let ready = o.installer.is_some();
        let title = if ready { format!("RecallCraft {} is ready to install", o.version) } else { format!("RecallCraft {} is available", o.version) };
        let detail = match (&o.problem, ready) {
            (Some(_), _) => format!("You have {}. The download didn't work, so get it from its release page.", current_text()),
            (None, true) => format!("You have {}. {}", current_text(), install_note(self.way.as_ref(), self.asks_permission)),
            (None, false) => format!("You have {}. Get it from its release page.", current_text()),
        };
        self.set_notice(Notice { title, detail, offer: true, ready, page: Some(o.page), seconds: OFFER_SECONDS, ..Notice::default() });
    }

    /// A check is asked for by the owner.
    pub fn ask(&mut self, asker: Asker) {
        if self.flatpak {
            self.info("RecallCraft can't check for updates here", "The Flatpak has no network access. New versions are on the releases page.", None);
            return;
        }
        match &mut self.job {
            // One is running: its answer goes to whoever asked last.
            Some((a, _)) => *a = asker,
            None => self.wanted = Some(asker),
        }
    }

    fn start_job(&mut self, asker: Asker) {
        let (tx, rx) = channel();
        let way = self.way.clone();
        let spawned = std::thread::Builder::new().name("update".into()).spawn(move || {
            let r = run(way, &tx);
            let _ = tx.send(Report::Done(r));
        });
        match spawned {
            Ok(_) => {
                self.job = Some((asker, rx));
                self.status = Status::Checking;
            }
            Err(e) => {
                log::warn!("the update check couldn't start: {e}");
                self.status = Status::Failed(e.to_string());
            }
        }
    }

    fn finish(&mut self, asker: Asker, result: Result<Outcome, String>) {
        match result {
            Ok(Outcome::UpToDate(latest)) => {
                self.status = Status::UpToDate(latest);
                // An offer the release no longer stands behind goes.
                self.offer = None;
                self.after_call = false;
                if self.notice.as_ref().is_some_and(|n| n.offer) {
                    self.notice = None;
                }
                if asker == Asker::Tray {
                    self.info("RecallCraft is up to date", format!("Version {} is the latest.", current_text()), None);
                }
            }
            Ok(Outcome::Offer(offer)) => {
                self.status = Status::Idle;
                // The automatic check tells of an update once it's ready, and once a run if put off.
                let complete = offer.installer.is_some() || self.way.is_none();
                let tell = match asker {
                    Asker::Tray => true,
                    Asker::Settings => false,
                    Asker::Timer => complete && self.dismissed.as_ref() != Some(&offer.version),
                };
                self.offer = Some(offer);
                if tell {
                    self.offer_notice();
                }
            }
            Err(e) => {
                log::warn!("the update check failed: {e}");
                if asker == Asker::Tray {
                    self.info("RecallCraft couldn't check for updates", e.clone(), None);
                }
                self.status = Status::Failed(e);
            }
        }
    }

    /// Twice a second, from the service: answers, the timer, the install waited for, the notice.
    pub fn tick(&mut self, in_call: bool, check_updates: bool) {
        let now = Instant::now();
        if let Some(v) = self.just_updated.take() {
            self.info(format!("RecallCraft {v} is installed"), "What's new is on its release page.", Some(update::page(&v)));
        }
        if let Some(v) = self.not_updated.take() {
            self.info(format!("RecallCraft wasn't updated to {v}"), "Install Update in the tray tries again once it's found again.", None);
        }
        let mut done = None;
        if let Some((asker, rx)) = &self.job {
            loop {
                match rx.try_recv() {
                    Ok(Report::Downloading(v)) => self.status = Status::Downloading(v),
                    Ok(Report::Done(r)) => {
                        done = Some((*asker, r));
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        done = Some((*asker, Err("the check stopped unexpectedly".to_string())));
                        break;
                    }
                }
            }
        }
        if let Some((asker, r)) = done {
            self.job = None;
            self.finish(asker, r);
        }
        // A check to start: asked for, or the timer's. Never during a call.
        if self.job.is_none() && !in_call {
            let auto = self.automatic && check_updates;
            let asker = self.wanted.take().or_else(|| self.schedule.due(now, auto, in_call).then_some(Asker::Timer));
            if let Some(a) = asker {
                self.schedule.started(now);
                self.start_job(a);
            }
        }
        // Update now was pressed during the call that just ended.
        if self.after_call && !in_call {
            self.after_call = false;
            self.install_now(false);
        }
        // Still here when the helper ended: the install didn't happen.
        let mut not_installed = false;
        if let Some((helper, at)) = &mut self.installing {
            if helper.ended() {
                not_installed = !helper.quit_now;
            } else if now.saturating_duration_since(*at) > INSTALL_WAIT {
                self.installing = None;
            }
        }
        if not_installed {
            self.installing = None;
            // Its note goes too, or the next start would tell of it again.
            let _ = update::take_marker(&update::updates_dir());
            log::warn!("the update installer ended and this program is still running: it wasn't updated");
            self.info("RecallCraft wasn't updated", "The install was cancelled or didn't finish. Install Update in the tray tries again.", None);
        }
        self.keep_notice(in_call);
        self.write(in_call);
    }

    /// The notice's window: started when there's a notice and no call, closed (by the notice
    /// going from [`state_file`]) during a call and shown again after it.
    fn keep_notice(&mut self, in_call: bool) {
        if let Some(c) = self.notice_window.as_mut()
            && !matches!(c.try_wait(), Ok(None))
        {
            self.notice_window = None;
            // Closed by the owner, or its time ran out; one held back during a call comes again.
            if !in_call {
                self.notice = None;
            }
        }
        if self.notice.is_none() || in_call || self.notice_window.is_some() || self.quit {
            return;
        }
        // Written first: the window reads it as it starts.
        self.write(in_call);
        let Ok(exe) = std::env::current_exe() else { return };
        match Command::new(exe).arg("--notice").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            Ok(c) => self.notice_window = Some(c),
            Err(e) => {
                log::warn!("showing the update notice: {e}");
                self.notice = None;
            }
        }
    }

    /// Update now (the notice, the tray's Install Update, the settings).
    pub fn install_now(&mut self, in_call: bool) {
        let Some(offer) = self.offer.clone() else { return };
        let Some(installer) = offer.installer.clone() else {
            // Nothing to install here: the release page has it.
            if let Err(e) = opener::open_browser(&offer.page) {
                log::warn!("opening {}: {e}", offer.page);
            }
            self.notice = None;
            return;
        };
        if self.installing.is_some() {
            let detail = if self.asks_permission { "Windows may be asking for permission." } else { "RecallCraft starts again when it's done." };
            self.info("The update is being installed", detail, None);
            return;
        }
        self.notice = None;
        if in_call {
            // Installing closes RecallCraft, which would end the call's recording.
            log::info!("the update to {} is installed when the call ends", offer.version);
            self.after_call = true;
            return;
        }
        match update::install(&installer, &offer.version) {
            Ok(helper) => {
                self.quit = helper.quit_now;
                self.installing = Some((helper, Instant::now()));
                if !self.quit {
                    self.info(format!("Installing RecallCraft {}", offer.version), install_note(self.way.as_ref(), self.asks_permission), None);
                }
            }
            Err(e) => {
                log::warn!("installing the update: {e}");
                if !installer.path.is_file() {
                    // It was deleted (it no longer matched): downloaded again.
                    if let Some(o) = &mut self.offer {
                        o.installer = None;
                    }
                    self.wanted = Some(Asker::Timer);
                }
                self.info("The update couldn't be installed", e, None);
            }
        }
    }

    /// Later, on the notice: no notice of this version again in this run.
    pub fn dismiss(&mut self) {
        self.notice = None;
        self.dismissed = self.offer.as_ref().map(|o| o.version.clone());
    }

    fn shared(&self, in_call: bool) -> Shared {
        let current = current_text();
        let (status, failed) = match &self.status {
            _ if self.wanted.is_some() && in_call => ("The check waits for the call to end.".to_string(), false),
            Status::Checking => ("Checking…".to_string(), false),
            Status::Downloading(v) => (format!("Downloading {v}…"), false),
            Status::UpToDate(Some(v)) if v.to_string() != current => (format!("Up to date (the latest release is {v})."), false),
            Status::UpToDate(_) => (format!("Up to date: {current} is the latest."), false),
            Status::Failed(e) => (format!("Couldn't check: {e}."), true),
            Status::Idle => (String::new(), false),
        };
        Shared {
            current,
            can_install: self.way.is_some(),
            asks_permission: self.asks_permission,
            install_note: install_note(self.way.as_ref(), self.asks_permission).to_string(),
            flatpak: self.flatpak,
            checking: self.job.is_some() || self.wanted.is_some(),
            status,
            failed,
            offer: self.offer.as_ref().map(|o| SharedOffer {
                version: o.version.to_string(),
                page: o.page.clone(),
                ready: o.installer.is_some(),
                problem: o.problem.clone(),
            }),
            installing: self.installing.is_some(),
            after_call: self.after_call,
            notice: if in_call { None } else { self.notice.clone() },
        }
    }

    /// Writes what the window and the notice read, when it changed.
    fn write(&mut self, in_call: bool) {
        let s = self.shared(in_call);
        if self.written.as_ref() == Some(&s) {
            return;
        }
        let path = state_file();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("json.part");
        let ok = serde_json::to_vec_pretty(&s)
            .map_err(|e| e.to_string())
            .and_then(|b| std::fs::write(&tmp, b).map_err(|e| e.to_string()))
            .and_then(|()| std::fs::rename(&tmp, &path).map_err(|e| e.to_string()));
        match ok {
            Ok(()) => self.written = Some(s),
            Err(e) => log::warn!("writing {}: {e}", path.display()),
        }
    }

    /// The service stops: the notice goes.
    pub fn shut_down(&mut self) {
        self.notice = None;
        self.write(false);
        if let Some(mut c) = self.notice_window.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tray_says_what_a_waiting_update_does() {
        let v = Version::new(0, 3, 0);
        let ready =
            Offer { version: v.clone(), page: update::page(&v), installer: Some(Installer { path: "x.msi".into(), sha256: [0; 32] }), problem: None };
        assert_eq!(tray_label(Some(&ready), false).as_deref(), Some("Install Update 0.3.0"));
        assert_eq!(tray_label(Some(&ready), true).as_deref(), Some("Install Update 0.3.0 (after this call)"));
        let page_only = Offer { installer: None, ..ready };
        assert_eq!(tray_label(Some(&page_only), false).as_deref(), Some("Get Update 0.3.0…"));
        assert_eq!(tray_label(None, false), None);
    }

    #[test]
    fn only_an_install_for_everyone_mentions_the_permission_prompt() {
        assert!(install_note(Some(&Way::Msi(update::Scope::Machine)), true).contains("permission"));
        assert!(!install_note(Some(&Way::Msi(update::Scope::User)), false).contains("permission"));
        assert!(install_note(Some(&Way::Portable(PathBuf::from("D:\\x"))), false).contains("closes for a moment"));
    }

    #[test]
    fn the_shared_state_reads_back_and_junk_is_nothing() {
        let s = Shared {
            current: "0.2.0".into(),
            offer: Some(SharedOffer { version: "0.3.0".into(), page: "p".into(), ready: true, problem: None }),
            notice: Some(Notice { id: 3, title: "t".into(), offer: true, ready: true, seconds: 30, ..Notice::default() }),
            ..Shared::default()
        };
        let back: Shared = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        let old: Shared = serde_json::from_str(r#"{"current":"0.2.0","something_new":1}"#).unwrap();
        assert_eq!(old.current, "0.2.0");
        assert_eq!(old.notice, None);
    }

    #[test]
    fn a_notice_is_held_back_during_a_call() {
        let mut u = Updates::blank(None);
        u.info("t", "d", None);
        assert!(u.shared(false).notice.is_some());
        assert!(u.shared(true).notice.is_none(), "never shown during a call");
        // Each notice its own number.
        let first = u.notice.as_ref().map(|n| n.id);
        u.info("t", "d", None);
        assert_ne!(u.notice.as_ref().map(|n| n.id), first);
        // A check asked for during a call waits for it to end.
        u.ask(Asker::Settings);
        let s = u.shared(true);
        assert!(s.checking);
        assert!(s.status.contains("call"));
    }
}
