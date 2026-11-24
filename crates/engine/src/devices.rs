//! The devices and the calls, looked at every second:
//!
//! - which programs have a microphone open: a call starts and ends ([`Detector`]);
//! - the microphone recorded: the one the settings name, else the one the call uses (during a
//!   call), else the system's default; a Bluetooth headset's only during calls (unless the
//!   settings say always); none while paused;
//! - the computer's sound recorded (unless the settings say not);
//! - a device that stopped (unplugged) started again, a moment later;
//! - the call's title, from its app's windows, every few seconds;
//! - what's kept long enough deleted, every ten minutes;
//! - what's going on written to `status.json`, for the window.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use recallcraft_audio::capture::{Capture, Mic, Source};
use recallcraft_audio::{Block, devices as audio_devices};
use recallcraft_calls::{CallApp, Detector, Event, Observation, Rules, apps, probe, titles};
use recallcraft_model::{Call, MicMode, Settings, Speaker, time};
use recallcraft_store::Layout;
use recallcraft_store::calls::{CallStore, new_id};

use crate::Shared;
use crate::recorder::ToHear;

/// A call shorter than this, with nothing said, wasn't a call (an app trying its microphone).
const MIN_CALL_MS: u64 = 20_000;
/// A device that won't start is tried again after this.
const RETRY: Duration = Duration::from_secs(5);
const SWEEP_EVERY: Duration = Duration::from_secs(600);
const WINDOWS_EVERY: Duration = Duration::from_secs(3);
/// The meeting window is the one that wasn't there this long before the call.
const BEFORE: Duration = Duration::from_secs(8);

fn rules(s: &Settings, own_exe: &str) -> Rules {
    let mut never = s.never_call_apps.clone();
    never.push(own_exe.to_ascii_lowercase());
    never.push("recallcraft.exe".to_string());
    Rules { browsers: s.browsers_are_calls, any_app: s.any_app_is_a_call, more: s.more_call_apps.clone(), never }
}

/// A device being recorded, or waiting to be tried again.
struct Slot {
    capture: Option<Capture>,
    want: Option<Source>,
    retry_at: Option<Instant>,
    problem: Option<String>,
}

impl Slot {
    fn new() -> Slot {
        Slot { capture: None, want: None, retry_at: None, problem: None }
    }

    /// Records `want` (nothing when None), starting or changing the device as needed.
    fn keep(&mut self, want: Option<Source>, tx: &Sender<Block>) {
        let alive = self.capture.as_ref().is_some_and(Capture::is_alive);
        let same = self.want == want;
        if same && (alive || want.is_none()) {
            return;
        }
        if !same {
            self.retry_at = None;
        }
        if let Some(c) = self.capture.take() {
            if !alive {
                log::warn!("{} stopped; starting it again", c.name);
            }
            c.stop();
        }
        self.want = want.clone();
        let Some(source) = want else {
            self.problem = None;
            return;
        };
        if self.retry_at.is_some_and(|t| Instant::now() < t) {
            return;
        }
        match Capture::start(source.clone(), tx.clone()) {
            Ok(c) => {
                self.problem = None;
                self.retry_at = None;
                self.capture = Some(c);
            }
            Err(e) => {
                if self.problem.as_deref() != Some(&e.to_string()) {
                    log::warn!("{}: {e}", source.stream().label());
                }
                self.problem = Some(format!("{}: {e}", source.stream().label()));
                self.retry_at = Some(Instant::now() + RETRY);
            }
        }
    }

    fn stop(&mut self) {
        if let Some(c) = self.capture.take() {
            c.stop();
        }
        self.want = None;
    }
}

/// The call going on.
struct Current {
    id: String,
    app: CallApp,
    before: Vec<String>,
    during: Vec<String>,
    windows_at: Option<Instant>,
}

/// Window titles of each calling app, a few seconds at a time.
type TitleHistory = VecDeque<(Instant, HashMap<String, Vec<String>>)>;

/// Calls left going on by a program that stopped without ending them: ended when they were
/// last written to.
fn close_stale(calls: &CallStore) {
    for mut c in calls.list().into_iter().filter(Call::is_live) {
        let Some(dir) = calls.dir(&c.id) else { continue };
        let last = ["transcript.jsonl", "call.json"]
            .iter()
            .filter_map(|f| std::fs::metadata(dir.join(f)).ok()?.modified().ok())
            .filter_map(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .max()
            .unwrap_or(c.started_ms);
        c.ended_ms = Some(last.max(c.started_ms));
        log::info!("call {} was left going on; ended it", c.id);
        let _ = calls.save(&c);
    }
}

/// Changes a call as it is on disk now (the window may have changed it meanwhile).
fn update_call(calls: &CallStore, id: &str, f: impl FnOnce(&mut Call)) -> Option<Call> {
    let mut c = calls.load(id)?;
    let before = c.clone();
    f(&mut c);
    if c != before
        && let Err(e) = calls.save(&c)
    {
        log::warn!("saving call {id}: {e}");
    }
    Some(c)
}

fn titles_by_app(procs: &probe::Processes) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for w in probe::windows() {
        let app = probe::app_of(w.pid, procs);
        if apps::known(&app) {
            let list = out.entry(app).or_default();
            if !list.contains(&w.title) && list.len() < 50 {
                list.push(w.title);
            }
        }
    }
    out
}

pub fn run(shared: &Shared, layout: &Layout, own_exe: &str, tx: Sender<Block>, hear: Sender<ToHear>) {
    let calls = CallStore::new(&layout.calls());
    close_stale(&calls);
    let mut mic = Slot::new();
    let mut system = Slot::new();
    let mut detector = Detector::new(rules(&shared.settings(), own_exe));
    let mut current: Option<Current> = None;
    let mut history: TitleHistory = VecDeque::new();
    let mut windows_at = Instant::now() - WINDOWS_EVERY;
    let mut swept = Instant::now() - SWEEP_EVERY + Duration::from_secs(30);
    let mut status_at = Instant::now() - Duration::from_secs(5);
    let mut bluetooth: (Instant, bool) = (Instant::now() - Duration::from_secs(60), false);
    let started = Instant::now();
    log::info!("recording started");
    while !shared.stopping() {
        let tick = Instant::now();
        let s = shared.settings();
        let now = time::now_ms();
        let recording = s.recording_at(now);
        detector.rules = rules(&s, own_exe);

        // Calls.
        let procs = probe::processes();
        let mut obs = Observation { at_ms: now, capturing: Vec::new() };
        if recording {
            obs.capturing = probe::capturing(&procs, &probe::own_tree(&procs));
        }
        let mut events = detector.observe(&obs);
        if !recording && let Some(e) = detector.end_now(now) {
            events.push(e);
        }
        if windows_at.elapsed() >= WINDOWS_EVERY {
            windows_at = Instant::now();
            history.push_back((Instant::now(), titles_by_app(&procs)));
            while history.len() > 20 {
                history.pop_front();
            }
        }
        for e in events {
            match e {
                Event::Started { app, at_ms, .. } => {
                    let before = history
                        .iter()
                        .rev()
                        .find(|(t, _)| t.elapsed() >= BEFORE)
                        .or(history.front())
                        .and_then(|(_, m)| m.get(&app.exe).cloned())
                        .unwrap_or_default();
                    let mut id = new_id(at_ms, &app.name);
                    if calls.load(&id).is_some() {
                        id = format!("{id}-{}", now % 1000);
                    }
                    let call = Call { id: id.clone(), app: app.name.clone(), app_exe: app.exe.clone(), started_ms: at_ms, ..Call::default() };
                    match calls.save(&call) {
                        Ok(()) => log::info!("a call started in {}: {id}", app.name),
                        Err(e) => log::warn!("saving the call: {e}"),
                    }
                    let _ = hear.send(ToHear::CallStarted { id: id.clone(), started_ms: at_ms });
                    current = Some(Current { id, app, before, during: Vec::new(), windows_at: None });
                }
                Event::Device { device_id } => log::info!("the call moved to another microphone ({device_id})"),
                Event::Ended { at_ms } => {
                    if let Some(c) = current.take() {
                        let ended = update_call(&calls, &c.id, |call| call.ended_ms = Some(at_ms.max(call.started_ms)));
                        let short = ended.as_ref().is_some_and(|call| call.duration_ms(at_ms) < MIN_CALL_MS);
                        if short && calls.transcript_len(&c.id) == 0 {
                            log::info!("{} lasted less than {} s with nothing said; not kept", c.id, MIN_CALL_MS / 1000);
                            let _ = calls.delete(&c.id);
                        } else {
                            log::info!("the call {} ended", c.id);
                        }
                        let _ = hear.send(ToHear::CallEnded { id: c.id, ended_ms: at_ms });
                    }
                }
            }
        }
        // The call's title, from its app's windows.
        if let Some(c) = current.as_mut()
            && c.windows_at.is_none_or(|t| t.elapsed() >= WINDOWS_EVERY)
        {
            c.windows_at = Some(Instant::now());
            let titles = history.back().and_then(|(_, m)| m.get(&c.app.exe).cloned()).unwrap_or_default();
            for t in titles {
                if !c.during.contains(&t) && c.during.len() < 50 {
                    c.during.push(t);
                }
            }
            let browser = apps::is_browser(&c.app.exe);
            let (title, service) = if browser {
                let t = c.during.iter().find(|t| titles::browser_app(t).is_some()).cloned();
                (t.as_deref().and_then(titles::clean), t.as_deref().and_then(titles::browser_app))
            } else {
                (titles::pick(&c.before, &c.during), None)
            };
            let seen: Vec<String> =
                if browser { c.during.iter().filter(|t| titles::browser_app(t).is_some()).cloned().collect() } else { c.during.clone() };
            update_call(&calls, &c.id, |call| {
                for t in &seen {
                    call.saw_window(t);
                }
                if let Some(service) = service {
                    call.app = service.to_string();
                }
                if let Some(t) = &title
                    && !call.titled_by_owner
                {
                    call.title = t.clone();
                    if let Some(person) = titles::person(t) {
                        call.names.entry(Speaker::Others.key()).or_insert_with(|| person.clone());
                        call.names.entry(Speaker::Voice(1).key()).or_insert(person);
                    }
                }
            });
        }

        // The devices.
        let in_call = detector.in_call();
        let mic_want = if !recording {
            None
        } else {
            let source = if !s.microphone.is_empty() {
                Mic::Named(s.microphone.clone())
            } else if let Some(d) = detector.device().filter(|d| in_call && !d.is_empty()) {
                Mic::Id(d.to_string())
            } else {
                Mic::Default
            };
            if bluetooth.0.elapsed() > Duration::from_secs(10) {
                let name = audio_devices::default_microphone(false).map(|d| d.name).unwrap_or_default();
                bluetooth = (Instant::now(), audio_devices::is_bluetooth(&name));
            }
            // The microphone that would be recorded outside a call: the one named, else the default.
            let bt = if s.microphone.is_empty() { bluetooth.1 } else { audio_devices::is_bluetooth(&s.microphone) };
            let on = match s.mic_mode {
                MicMode::Always => true,
                MicMode::DuringCalls => in_call,
                MicMode::Auto => in_call || !bt,
            };
            on.then_some(Source::Mic(source))
        };
        mic.keep(mic_want, &tx);
        let system_want = (recording && s.record_system && recallcraft_audio::capture::system_sound_supported()).then_some(Source::System);
        system.keep(system_want, &tx);

        // What's going on.
        let call = current.as_ref().and_then(|c| calls.load(&c.id));
        shared.update(|st| {
            st.recording = recording && (mic.capture.is_some() || system.capture.is_some());
            st.paused_until_ms = if s.recording { s.paused_until_ms } else { u64::MAX };
            st.mic = mic.capture.as_ref().map(|c| c.name.clone()).unwrap_or_default();
            st.mic_level = mic.capture.as_ref().map_or(0.0, Capture::level);
            st.system = system.capture.as_ref().map(|c| c.name.clone()).unwrap_or_default();
            st.system_level = system.capture.as_ref().map_or(0.0, Capture::level);
            st.call_id = call.as_ref().map(|c| c.id.clone()).unwrap_or_default();
            st.call_title = call.as_ref().map(Call::display_title).unwrap_or_default();
            st.call_app = call.as_ref().map(|c| c.app.clone()).unwrap_or_default();
            st.call_started_ms = call.as_ref().map_or(0, |c| c.started_ms);
            st.problems = [&mic.problem, &system.problem].into_iter().flatten().cloned().collect();
            st.at_ms = now;
            st.pid = std::process::id();
        });
        if status_at.elapsed() >= Duration::from_secs(2) {
            status_at = Instant::now();
            if let Ok(json) = serde_json::to_string(&shared.status()) {
                let _ = std::fs::create_dir_all(&layout.root);
                let _ = recallcraft_store::write_atomic(&layout.status(), json.as_bytes());
            }
        }

        // What's kept long enough goes.
        if swept.elapsed() >= SWEEP_EVERY && started.elapsed() > Duration::from_secs(20) {
            swept = Instant::now();
            recallcraft_store::retention::sweep(&layout.ring(), &calls, s.keep_audio_ms(), s.keep_calls_ms(), now);
        }

        let spent = tick.elapsed();
        if spent > Duration::from_millis(500) {
            log::debug!("looking at the devices took {} ms", spent.as_millis());
        }
        std::thread::sleep(Duration::from_millis(1000).saturating_sub(spent).max(Duration::from_millis(100)));
    }
    // Stopping: the call going on ends now.
    if let Some(c) = current.take() {
        let now = time::now_ms();
        update_call(&calls, &c.id, |call| call.ended_ms = Some(now.max(call.started_ms)));
        let _ = hear.send(ToHear::CallEnded { id: c.id, ended_ms: now });
    }
    mic.stop();
    system.stop();
    let _ = std::fs::remove_file(layout.status());
    log::info!("recording stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recallcraft_is_never_a_call_and_the_settings_count() {
        let s = Settings { browsers_are_calls: false, more_call_apps: vec!["myphone.exe".into()], ..Settings::default() };
        let r = rules(&s, "D:\\x\\RecallCraft.exe");
        assert!(!r.browsers);
        assert!(r.never.contains(&"recallcraft.exe".to_string()));
        assert!(r.more.contains(&"myphone.exe".to_string()));
    }

    #[test]
    fn a_call_left_going_on_is_ended() {
        let root = std::env::temp_dir().join(format!("recallcraft-stale-{}", std::process::id()));
        let calls = CallStore::new(&root);
        let c = Call { id: "left".into(), started_ms: 1000, ..Call::default() };
        calls.save(&c).unwrap();
        close_stale(&calls);
        let back = calls.load("left").unwrap();
        assert!(back.ended_ms.is_some_and(|e| e >= 1000));
        let _ = std::fs::remove_dir_all(&root);
    }
}
