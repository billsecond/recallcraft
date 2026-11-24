//! The service: in the tray, recording all the time ([`recallcraft_engine`]), with nothing shown
//! and nothing said when a call starts (a dot on the tray icon, if the settings want it).
//!
//! - The tray icon opens the window (a click), and its menu has Replay, Pause and Settings.
//! - The Replay shortcut plays the last seconds again, at speed; pressed again, it stops.
//! - The window is a program of its own (`recallcraft --window`), started by the service so its
//!   sound is one of the service's own, left out of the recording.
//! - A later start (`recallcraft --replay`, the Start menu) hands its arguments over
//!   ([`instance`]).
//! - The settings file is looked at twice a second: the window changes it.

use std::process::{Child, Command, ExitCode};
use std::time::{Duration, Instant, SystemTime};

use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
use recallcraft_audio::playback::{Player, RingSource};
use recallcraft_desktop::icon::{self, Look};
use recallcraft_desktop::{instance, logging, models, paths, service as signin, settings as prefs, version};
use recallcraft_engine::worker::HearWith;
use recallcraft_engine::{Config, Engine};
use recallcraft_model::{Settings, time};
use recallcraft_store::Layout;
use recallcraft_store::ring::RingReader;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use winit::application::ApplicationHandler;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

use crate::hotkeys::{Registered, Target};

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
    settings: MenuItem,
    quit: MenuItem,
}

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
    window: Option<Child>,
    pending: Vec<Vec<String>>,
    look: Option<Look>,
    tip: String,
    status_text: String,
    quitting: bool,
}

fn icon(look: Look) -> Option<tray_icon::Icon> {
    tray_icon::Icon::from_rgba(icon::pixels(32, look), 32, 32).inspect_err(|e| log::warn!("tray icon: {e}")).ok()
}

/// How the engine hears: the worker program, if there's a model.
fn hear_with() -> HearWith {
    match (std::env::current_exe(), models::speech_model()) {
        (Ok(exe), Ok(model)) => {
            let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
            HearWith::Program { exe, args: vec!["--hear".to_string()], model, threads: (cores / 4).clamp(1, 2) }
        }
        (_, Err(e)) => {
            log::warn!("{e}; what's said won't be written down");
            HearWith::Off("no speech model".to_string())
        }
        (Err(e), _) => HearWith::Off(e.to_string()),
    }
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
    let config = Config {
        data_dir: layout.root.clone(),
        settings_dir: paths::settings_dir().unwrap_or_else(|| layout.root.join("settings")),
        hear: hear_with(),
        own_exe: std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_default(),
    };
    let engine = Engine::start(config, settings.clone());
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
        window: None,
        pending,
        look: None,
        tip: String::new(),
        status_text: String::new(),
        quitting: false,
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
            settings: MenuItem::new("Settings…", true, None),
            quit: MenuItem::new("Quit RecallCraft", true, None),
        };
        let menu = Menu::new();
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
            menu.append(&items.settings),
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

    /// Plays the last seconds again; a second press stops it.
    fn replay(&mut self) {
        if self.replay.take().is_some() {
            log::info!("replay stopped");
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
                self.replay = Some(p);
            }
            Err(e) => log::warn!("replay: {e}"),
        }
    }

    fn command(&mut self, args: &[String], el: &ActiveEventLoop) {
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--open" | "--show" => self.open(false),
                "--settings" => self.open(true),
                "--replay" => self.replay(),
                "--pause" => {
                    let minutes: u64 = it.next().and_then(|m| m.parse().ok()).unwrap_or(0);
                    self.pause(minutes);
                }
                "--resume" => self.resume(),
                "--quit" => {
                    self.quitting = true;
                    el.exit();
                }
                _ => {}
            }
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
            self.replay = None;
        }
        if let Some(c) = self.window.as_mut()
            && !matches!(c.try_wait(), Ok(None))
        {
            self.window = None;
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
        } else if !st.problems.is_empty() {
            format!("Recording · {}", st.problems.join("; "))
        } else if self.replay.is_some() {
            "Replaying…".to_string()
        } else {
            "Recording".to_string()
        };
        if text != self.status_text {
            if let Some(i) = &self.items {
                i.status.set_text(&text);
                i.resume.set_enabled(paused);
                i.pause.set_enabled(!paused || self.settings.recording);
                let replay = format!("Replay the last {} seconds", self.settings.replay_seconds);
                let key = self.settings.replay_hotkey.trim();
                i.replay.set_text(if key.is_empty() { replay } else { format!("{replay}\t{key}") });
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

    fn shut_down(&mut self) {
        self.replay = None;
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
