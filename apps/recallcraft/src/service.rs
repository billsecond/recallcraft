//! The service: in the tray, recording all the time ([`recallcraft_engine`]), with nothing shown
//! and nothing said when a call starts (a dot on the tray icon, if the settings want it).
//!
//! - The tray icon opens the window (a click), and its menu has Replay, Pause and Settings.
//! - The Replay shortcut plays the last seconds again, at speed, and stops when it has played
//!   them; pressed again, it stops sooner, and so does the replay bar's Stop
//!   (`recallcraft --replay-bar`) or the tray's Stop Replay. What it plays isn't recorded.
//! - The window is a program of its own (`recallcraft --window`), started by the service so its
//!   sound is one of the service's own, left out of the recording.
//! - A later start (`recallcraft --replay`, the Start menu) hands its arguments over
//!   ([`instance`]).
//! - The settings file is looked at twice a second: the window changes it.
//! - Updates ([`crate::update`]): checked now and then (never during a call), offered in the
//!   tray, a notice in the corner and the window's settings.

use std::process::{Child, Command, ExitCode, Stdio};
use std::time::{Duration, Instant, SystemTime};

use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
use recallcraft_audio::playback::{Player, RingSource};
use recallcraft_desktop::icon::{self, Look};
use recallcraft_desktop::{instance, logging, paths, service as signin, settings as prefs, version};
use recallcraft_engine::{Config, Engine};
use recallcraft_model::{RecordMode, Settings, time};
use recallcraft_store::Layout;
use recallcraft_store::ring::RingReader;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use winit::application::ApplicationHandler;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

use crate::hotkeys::{Registered, Target};
use crate::replay::ReplayState;
use crate::speech_model::Chooser;
use crate::update::{Asker, Updates};

/// How long sound stopped a moment ago is still coming out of the speaker (its buffer).
const SPEAKER_TAIL_US: u64 = 500_000;

/// What wakes the service.
#[derive(Debug)]
enum Msg {
    Tray(TrayIconEvent),
    Menu(MenuId),
    Hotkey(u32),
    Wake,
}

struct Items {
    status: MenuItem,
    open: MenuItem,
    replay: MenuItem,
    pause15: MenuItem,
    pause60: MenuItem,
    pause: MenuItem,
    resume: MenuItem,
    calls_only: CheckMenuItem,
    settings: MenuItem,
    check_updates: MenuItem,
    /// Install Update (or Get Update) while one is waiting; in the menu only then.
    update: Option<MenuItem>,
    quit: MenuItem,
    menu: Menu,
}

/// Where the waiting update's item goes in the menu: before Check for Updates….
const UPDATE_AT: usize = 12;

struct ServiceApp {
    engine: Option<Engine>,
    guard: instance::Guard,
    settings: Settings,
    settings_seen: Option<SystemTime>,
    layout: Layout,
    tray: Option<TrayIcon>,
    items: Option<Items>,
    keys: Option<Registered>,
    replay: Option<Player>,
    /// The replay going on, as the replay bar is told it.
    replay_state: ReplayState,
    replay_bar: Option<Child>,
    replay_label: String,
    window: Option<Child>,
    pending: Vec<Vec<String>>,
    look: Option<Look>,
    tip: String,
    status_text: String,
    quitting: bool,
    updates: Updates,
    /// The speech model chosen, downloaded when it isn't here.
    speech: Chooser,
}

fn icon(look: Look) -> Option<tray_icon::Icon> {
    tray_icon::Icon::from_rgba(icon::pixels(32, look), 32, 32).inspect_err(|e| log::warn!("tray icon: {e}")).ok()
}

/// Started by the owner while nothing runs: the service is started under the supervisor
/// (`--background`), so it's started again if it ever stops unexpectedly and stays in the tray
/// until Quit; this copy hands its arguments to it and ends. None when that isn't how it starts
/// (runs without settings, or the supervisor can't be started): this copy is the service.
fn start_supervised(handover: &[String]) -> Option<ExitCode> {
    if paths::no_prefs() || instance::running("service") {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let mut c = Command::new(&exe);
    c.arg("--background").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Its own, outliving this copy and any console it was started from.
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        c.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    if let Err(e) = c.spawn() {
        log::warn!("starting the supervisor: {e}; running the service here");
        return None;
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if instance::tell("service", handover) {
            return Some(ExitCode::SUCCESS);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    log::error!("the service didn't start in time");
    Some(ExitCode::FAILURE)
}

pub fn run(args: &[String]) -> ExitCode {
    let by_system = args.iter().any(|a| a == "--tray");
    let handover: Vec<String> = if by_system {
        vec!["--noop".to_string()]
    } else if args.is_empty() {
        vec!["--open".to_string()]
    } else {
        args.to_vec()
    };
    if !by_system
        && !args.iter().any(|a| a == "--quit")
        && let Some(code) = start_supervised(&handover)
    {
        return code;
    }
    let guard = match instance::start("service", &handover) {
        instance::Start::First(g) => g,
        instance::Start::Handed => return ExitCode::SUCCESS,
        instance::Start::Unreachable(e) => {
            log::error!("another RecallCraft is running but couldn't be reached: {e}");
            return ExitCode::FAILURE;
        }
    };
    if args.iter().any(|a| a == "--quit") {
        return ExitCode::SUCCESS;
    }
    logging::to_file("service");
    log::info!("{} starting", version::line("RecallCraft"));
    let settings = prefs::load();
    // An installed copy arranges to start at sign-in the first time it runs (it's what keeps
    // the last two days); a build in the source tree never does.
    if let (Some(exe), Some(dir)) = (signin::program(), paths::ensure_settings_dir()) {
        let marker = dir.join("signin.arranged");
        if !signin::development_build(&exe) && !marker.exists() && !paths::no_prefs() {
            match signin::enable() {
                Ok(m) => log::info!("{m}"),
                Err(e) => log::warn!("starting at sign-in could not be arranged: {e}"),
            }
            let _ = std::fs::write(&marker, b"");
        }
    }
    let layout = Layout::new(paths::data_dir(&settings));
    let speech = Chooser::new(&layout.root);
    let (hear, hearing_with) = speech.hear_with(settings.speech_model);
    let config = Config {
        data_dir: layout.root.clone(),
        settings_dir: paths::settings_dir().unwrap_or_else(|| layout.root.join("settings")),
        hear,
        own_exe: std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_default(),
    };
    let engine = Engine::start(config, settings.clone());
    engine.shared.update(|s| s.speech_model = hearing_with.to_string());
    let event_loop = match EventLoop::<Msg>::with_user_event().build() {
        Ok(l) => l,
        Err(e) => {
            log::error!("no event loop: {e}");
            engine.stop();
            return ExitCode::FAILURE;
        }
    };
    let proxy = event_loop.create_proxy();
    let p = proxy.clone();
    TrayIconEvent::set_event_handler(Some(move |e| {
        let _ = p.send_event(Msg::Tray(e));
    }));
    let p = proxy.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let _ = p.send_event(Msg::Menu(e.id));
    }));
    let p = proxy.clone();
    GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
        if e.state == HotKeyState::Pressed {
            let _ = p.send_event(Msg::Hotkey(e.id));
        }
    }));
    let p = proxy;
    guard.set_waker(Box::new(move || {
        let _ = p.send_event(Msg::Wake);
    }));
    let pending = if by_system { Vec::new() } else { vec![handover] };
    let mut app = ServiceApp {
        engine: Some(engine),
        guard,
        settings_seen: prefs::modified(),
        settings,
        layout,
        tray: None,
        items: None,
        keys: None,
        replay: None,
        replay_state: ReplayState::default(),
        replay_bar: None,
        replay_label: String::new(),
        window: None,
        pending,
        look: None,
        tip: String::new(),
        status_text: String::new(),
        quitting: false,
        updates: Updates::new(),
        speech,
    };
    let r = event_loop.run_app(&mut app);
    app.shut_down();
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("the event loop: {e}");
            ExitCode::FAILURE
        }
    }
}

impl ServiceApp {
    fn build_tray(&mut self) {
        let items = Items {
            status: MenuItem::new("RecallCraft", false, None),
            open: MenuItem::new("Open RecallCraft", true, None),
            replay: MenuItem::new("Replay", true, None),
            pause15: MenuItem::new("Pause for 15 minutes", true, None),
            pause60: MenuItem::new("Pause for an hour", true, None),
            pause: MenuItem::new("Pause until I resume", true, None),
            resume: MenuItem::new("Resume recording", false, None),
            calls_only: CheckMenuItem::new("Record only during calls", true, self.settings.record_mode == RecordMode::CallsOnly, None),
            settings: MenuItem::new("Settings…", true, None),
            check_updates: MenuItem::new("Check for Updates…", true, None),
            update: None,
            quit: MenuItem::new("Quit RecallCraft", true, None),
            menu: Menu::new(),
        };
        let menu = items.menu.clone();
        let sep = || PredefinedMenuItem::separator();
        for r in [
            menu.append(&items.status),
            menu.append(&sep()),
            menu.append(&items.open),
            menu.append(&items.replay),
            menu.append(&sep()),
            menu.append(&items.pause15),
            menu.append(&items.pause60),
            menu.append(&items.pause),
            menu.append(&items.resume),
            menu.append(&sep()),
            menu.append(&items.calls_only),
            menu.append(&items.settings),
            menu.append(&items.check_updates),
            menu.append(&items.quit),
        ] {
            if let Err(e) = r {
                log::warn!("tray menu: {e}");
            }
        }
        let mut b = TrayIconBuilder::new().with_tooltip("RecallCraft").with_menu(Box::new(menu)).with_menu_on_left_click(false);
        if let Some(i) = icon(Look::Normal) {
            b = b.with_icon(i);
        }
        match b.build() {
            Ok(t) => self.tray = Some(t),
            Err(e) => log::warn!("no tray icon: {e}"),
        }
        self.look = Some(Look::Normal);
        self.items = Some(items);
    }

    fn settings_changed(&mut self) {
        let seen = prefs::modified();
        if seen == self.settings_seen {
            return;
        }
        self.settings_seen = seen;
        self.settings = prefs::load();
        if let Some(e) = &self.engine {
            e.set_settings(self.settings.clone());
        }
        if let Some(k) = self.keys.as_mut() {
            k.apply(&self.settings);
        }
        let data = paths::data_dir(&self.settings);
        if data != self.layout.root {
            log::info!("the data folder changed; RecallCraft records there after it starts again");
        }
    }

    fn change(&mut self, f: impl FnOnce(&mut Settings)) {
        self.settings = prefs::update(f);
        self.settings_seen = prefs::modified();
        if let Some(e) = &self.engine {
            e.set_settings(self.settings.clone());
        }
    }

    /// Opens the window (at its settings), or brings the open one forward.
    fn open(&mut self, settings_page: bool) {
        let running = self.window.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None)));
        let word = if settings_page { "--settings" } else { "--show" }.to_string();
        if running || instance::running("window") {
            std::thread::spawn(move || {
                instance::tell("window", &[word]);
            });
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let mut c = Command::new(exe);
        c.arg("--window").env(crate::FROM_SERVICE, "1");
        if settings_page {
            c.arg("--settings");
        }
        match c.spawn() {
            Ok(child) => self.window = Some(child),
            Err(e) => log::warn!("opening the window: {e}"),
        }
    }

    /// Plays the last seconds again, up to the moment it was asked for, and stops; asked again
    /// while it plays, it stops sooner.
    fn replay(&mut self) {
        if self.replay.is_some() {
            self.stop_replay("stopped");
            return;
        }
        let s = &self.settings;
        let now = time::now_us();
        let from = now.saturating_sub(u64::from(s.replay_seconds) * 1_000_000);
        match Player::new(RingSource::new(RingReader::new(&self.layout.ring()), true)) {
            Ok(p) => {
                p.set_speed(s.replay_speed);
                p.set_live(true);
                p.play_from(from, Some(now));
                log::info!("replaying the last {} s at {}×", s.replay_seconds, s.replay_speed);
                // What it plays is in the computer's sound for that long: it isn't recorded, or
                // it would be in the next replay (cut short when it stops).
                let lasts = (f64::from(s.replay_seconds) / f64::from(s.replay_speed.max(0.5)) * 1_000_000.0) as u64;
                if let Some(e) = &self.engine {
                    e.shared.playing(now, now + lasts + 2_000_000);
                }
                self.replay = Some(p);
                self.replay_state = ReplayState {
                    id: now,
                    playing: true,
                    seconds: s.replay_seconds,
                    speed: s.replay_speed,
                    started_ms: now / 1000,
                    lasts_ms: lasts / 1000,
                };
                crate::replay::write(&self.replay_state);
                self.show_replay_bar();
            }
            Err(e) => log::warn!("replay: {e}"),
        }
    }

    /// The replay ends (it played what it was asked to, or was stopped).
    fn stop_replay(&mut self, how: &str) {
        if self.replay.take().is_none() {
            return;
        }
        log::info!("replay {how}");
        if let Some(e) = &self.engine {
            e.shared.stopped_playing(time::now_us() + SPEAKER_TAIL_US);
        }
        self.replay_state.playing = false;
        crate::replay::write(&self.replay_state);
    }

    /// The bar that shows the replay, with its Stop (a bar that's up follows the next replay).
    fn show_replay_bar(&mut self) {
        if let Some(c) = self.replay_bar.as_mut()
            && matches!(c.try_wait(), Ok(None))
        {
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        match Command::new(exe).arg("--replay-bar").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            Ok(c) => self.replay_bar = Some(c),
            Err(e) => {
                log::warn!("showing the replay bar: {e}");
                self.replay_bar = None;
            }
        }
    }

    /// The tray's Replay item: Stop Replay while one plays.
    fn sync_replay_item(&mut self) {
        let key = self.settings.replay_hotkey.trim();
        let what =
            if self.replay.is_some() { "Stop Replay".to_string() } else { format!("Replay the last {} seconds", self.settings.replay_seconds) };
        let label = if key.is_empty() { what } else { format!("{what}\t{key}") };
        if label != self.replay_label
            && let Some(i) = &self.items
        {
            i.replay.set_text(&label);
            self.replay_label = label;
        }
    }

    fn command(&mut self, args: &[String], el: &ActiveEventLoop) {
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--open" | "--show" => self.open(false),
                "--settings" => self.open(true),
                "--replay" => self.replay(),
                "--stop-replay" => self.stop_replay("stopped"),
                "--fetch-speech-model" => {
                    // Try the chosen model's download again (the window's Try again).
                    if let Some(e) = &self.engine {
                        self.speech.sync(self.settings.speech_model, true, &e.shared);
                    }
                }
                "--pause" => {
                    let minutes: u64 = it.next().and_then(|m| m.parse().ok()).unwrap_or(0);
                    self.pause(minutes);
                }
                "--resume" => self.resume(),
                "--own-sound" => {
                    // The window plays the recording from now until then (µs since 1970).
                    let until: u64 = it.next().and_then(|m| m.parse().ok()).unwrap_or(0);
                    let now = time::now_us();
                    if let Some(e) = &self.engine
                        && until > now
                    {
                        e.shared.playing(now.saturating_sub(500_000), until.min(now + 60_000_000));
                    }
                }
                "--own-sound-stopped" => {
                    // The window stopped playing.
                    if let Some(e) = &self.engine {
                        e.shared.stopped_playing(time::now_us() + SPEAKER_TAIL_US);
                    }
                }
                "--check-updates" => {
                    // From the window's settings (which show the answer), or the command line.
                    let from_settings = it.as_slice().first().is_some_and(|w| w == "settings");
                    if from_settings {
                        it.next();
                    }
                    self.updates.ask(if from_settings { Asker::Settings } else { Asker::Tray });
                }
                "--install-update" => self.install_update(el),
                "--dismiss-update" => self.updates.dismiss(),
                "--quit" => {
                    self.quitting = true;
                    el.exit();
                }
                _ => {}
            }
        }
    }

    fn in_call(&self) -> bool {
        self.engine.as_ref().is_some_and(|e| e.status().in_call())
    }

    /// Update now. A portable copy's helper waits for RecallCraft to close: it quits.
    fn install_update(&mut self, el: &ActiveEventLoop) {
        let in_call = self.in_call();
        self.updates.install_now(in_call);
        if self.updates.quit {
            log::info!("quitting so the update can be put in place");
            self.quitting = true;
            el.exit();
        }
    }

    /// The tray's item for a waiting update, kept as the updates say.
    fn sync_update_item(&mut self) {
        let label = self.updates.tray_label();
        let Some(i) = self.items.as_mut() else { return };
        match (&label, &i.update) {
            (Some(text), Some(item)) => {
                if item.text() != *text {
                    item.set_text(text);
                }
            }
            (Some(text), None) => {
                let item = MenuItem::new(text, true, None);
                if let Err(e) = i.menu.insert(&item, UPDATE_AT) {
                    log::warn!("tray menu: {e}");
                }
                i.update = Some(item);
            }
            (None, Some(item)) => {
                if let Err(e) = i.menu.remove(item) {
                    log::warn!("tray menu: {e}");
                }
                i.update = None;
            }
            (None, None) => {}
        }
    }

    fn pause(&mut self, minutes: u64) {
        if minutes == 0 {
            log::info!("recording paused until resumed");
            self.change(|s| s.recording = false);
        } else {
            log::info!("recording paused for {minutes} minutes");
            let until = time::now_ms() + minutes.min(24 * 60) * 60_000;
            self.change(|s| {
                s.recording = true;
                s.paused_until_ms = until;
            });
        }
    }

    fn resume(&mut self) {
        log::info!("recording resumed");
        self.change(|s| {
            s.recording = true;
            s.paused_until_ms = 0;
        });
    }

    /// Everything looked at twice a second.
    fn tick(&mut self, el: &ActiveEventLoop) {
        for args in std::mem::take(&mut self.pending) {
            self.command(&args, el);
        }
        for args in self.guard.handed() {
            self.command(&args, el);
        }
        self.settings_changed();
        if self.replay.as_ref().is_some_and(Player::finished) {
            self.stop_replay("ended");
        }
        // Still playing (it can take longer than it was meant to): still not recorded.
        if self.replay.is_some()
            && let Some(e) = &self.engine
        {
            let now = time::now_us();
            e.shared.playing(now, now + 2_000_000);
        }
        if let Some(c) = self.replay_bar.as_mut()
            && !matches!(c.try_wait(), Ok(None))
        {
            self.replay_bar = None;
        }
        self.sync_replay_item();
        if let Some(e) = &self.engine {
            self.speech.sync(self.settings.speech_model, false, &e.shared);
            self.speech.tick(&e.shared);
        }
        if let Some(c) = self.window.as_mut()
            && !matches!(c.try_wait(), Ok(None))
        {
            self.window = None;
        }
        let in_call = self.in_call();
        self.updates.tick(in_call, self.settings.check_updates);
        self.sync_update_item();
        if self.updates.quit && !self.quitting {
            log::info!("quitting so the update can be put in place");
            self.quitting = true;
            el.exit();
            return;
        }
        let Some(engine) = &self.engine else { return };
        let st = engine.status();
        let now = time::now_ms();
        let paused = !self.settings.recording_at(now);
        let look = if paused {
            Look::Paused
        } else if st.in_call() && self.settings.tray_dot {
            Look::Call
        } else {
            Look::Normal
        };
        if self.look != Some(look)
            && let Some(t) = &self.tray
        {
            let _ = t.set_icon(icon(look));
            self.look = Some(look);
        }
        let text = if !self.settings.recording {
            "Paused".to_string()
        } else if paused {
            format!("Paused until {}", time::local_time(self.settings.paused_until_ms))
        } else if st.in_call() {
            format!("Recording · {} ({})", st.call_title, time::duration_words(now.saturating_sub(st.call_started_ms)))
        } else if self.replay.is_some() {
            "Replaying…".to_string()
        } else if self.settings.record_mode == RecordMode::CallsOnly {
            "Waiting for a call (recording only during calls)".to_string()
        } else if !st.problems.is_empty() {
            format!("Recording · {}", st.problems.join("; "))
        } else {
            "Recording".to_string()
        };
        if let Some(i) = &self.items {
            // Kept as the settings say (the window can change them too).
            let calls_only = self.settings.record_mode == RecordMode::CallsOnly;
            if i.calls_only.is_checked() != calls_only {
                i.calls_only.set_checked(calls_only);
            }
        }
        if text != self.status_text {
            if let Some(i) = &self.items {
                i.status.set_text(&text);
                i.resume.set_enabled(paused);
                i.pause.set_enabled(!paused || self.settings.recording);
            }
            self.status_text = text.clone();
        }
        let tip = format!("RecallCraft · {text}");
        if tip != self.tip {
            if let Some(t) = &self.tray {
                let _ = t.set_tooltip(Some(tip.chars().take(120).collect::<String>()));
            }
            self.tip = tip;
        }
    }

    /// Quit: the window goes too (it's told to, and ended if it doesn't within two seconds).
    fn close_window(&mut self) {
        if instance::running("window") {
            instance::tell("window", &["--quit".to_string()]);
        }
        if let Some(mut c) = self.window.take() {
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline && matches!(c.try_wait(), Ok(None)) {
                std::thread::sleep(Duration::from_millis(50));
            }
            if matches!(c.try_wait(), Ok(None)) {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }

    fn shut_down(&mut self) {
        self.stop_replay("stopped");
        self.speech.shut_down();
        self.updates.shut_down();
        if self.quitting {
            self.close_window();
        }
        if let Some(e) = self.engine.take() {
            e.stop();
        }
        self.guard.close();
        log::info!("the service stopped");
    }
}

impl ApplicationHandler<Msg> for ServiceApp {
    fn new_events(&mut self, el: &ActiveEventLoop, cause: StartCause) {
        if cause == StartCause::Init {
            self.build_tray();
            let mut keys = Registered::new();
            keys.apply(&self.settings);
            self.keys = Some(keys);
        }
        if !self.quitting {
            self.tick(el);
        }
    }

    fn resumed(&mut self, _el: &ActiveEventLoop) {}

    fn user_event(&mut self, el: &ActiveEventLoop, msg: Msg) {
        match msg {
            Msg::Tray(TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }) => self.open(false),
            Msg::Tray(_) | Msg::Wake => {}
            Msg::Hotkey(id) => match self.keys.as_ref().and_then(|k| k.target(id)) {
                Some(Target::Replay) => self.replay(),
                Some(Target::Open) => self.open(false),
                None => {}
            },
            Msg::Menu(id) => {
                let Some(i) = &self.items else { return };
                if *i.check_updates.id() == id {
                    self.updates.ask(Asker::Tray);
                    self.tick(el);
                    return;
                }
                if i.update.as_ref().is_some_and(|u| *u.id() == id) {
                    self.install_update(el);
                    if !self.quitting {
                        self.tick(el);
                    }
                    return;
                }
                if *i.calls_only.id() == id {
                    // The menu ticked or unticked it already.
                    let mode = if i.calls_only.is_checked() { RecordMode::CallsOnly } else { RecordMode::Always };
                    log::info!("{}", mode.label());
                    self.change(|s| s.record_mode = mode);
                    self.tick(el);
                    return;
                }
                let which =
                    [(&i.open, 0), (&i.replay, 1), (&i.pause15, 2), (&i.pause60, 3), (&i.pause, 4), (&i.resume, 5), (&i.settings, 6), (&i.quit, 7)]
                        .iter()
                        .find(|(item, _)| *item.id() == id)
                        .map(|(_, n)| *n);
                match which {
                    Some(0) => self.open(false),
                    Some(1) => self.replay(),
                    Some(2) => self.pause(15),
                    Some(3) => self.pause(60),
                    Some(4) => self.pause(0),
                    Some(5) => self.resume(),
                    Some(6) => self.open(true),
                    Some(7) => {
                        self.quitting = true;
                        el.exit();
                    }
                    _ => {}
                }
            }
        }
        if !self.quitting {
            self.tick(el);
        }
    }

    fn window_event(&mut self, _el: &ActiveEventLoop, _id: WindowId, _e: WindowEvent) {}

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(500)));
    }
}
