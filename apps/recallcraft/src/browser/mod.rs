//! The window: the calls of the last days, each with its transcript (as it's written, during a
//! call) and its sound, to play back at any speed; everything heard in the last two days; the
//! settings; exports.
//!
//! It reads what the service writes (the recording, the calls, `status.json`) and writes only the
//! settings and what the owner changes in a call (its title, names, keeping it, deleting it). It's
//! started by the service (`recallcraft --window`), and tells it while it plays, so what it plays
//! isn't written down as the others'.

mod call_view;
mod everything;
mod export_dialog;
pub mod notice;
mod settings_page;
mod style;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use recallcraft_audio::playback::{Mix, Player, RingSource};
use recallcraft_desktop::icon::{self, Look};
use recallcraft_desktop::{instance, logging, paths, settings as prefs};
use recallcraft_engine::Status;
use recallcraft_model::{Call, Line, Settings, time};
use recallcraft_store::Layout;
use recallcraft_store::calls::CallStore;
use recallcraft_store::ring::RingReader;

use crate::graphics;

pub fn run(args: &[String]) -> ExitCode {
    let settings_page = args.iter().any(|a| a == "--settings");
    // Opened by the owner rather than by the service: the service opens it, so what it plays is
    // the service's own sound and isn't recorded.
    if std::env::var_os(crate::FROM_SERVICE).is_none() && instance::running("service") {
        let word = if settings_page { "--settings" } else { "--open" };
        if instance::tell("service", &[word.to_string()]) {
            return ExitCode::SUCCESS;
        }
    }
    let handover = vec![if settings_page { "--settings" } else { "--show" }.to_string()];
    let guard = match instance::start("window", &handover) {
        instance::Start::First(g) => g,
        instance::Start::Handed => return ExitCode::SUCCESS,
        instance::Start::Unreachable(e) => {
            log::error!("another window is open but couldn't be reached: {e}");
            return ExitCode::FAILURE;
        }
    };
    logging::to_file("window");
    let icon = egui::IconData { rgba: icon::pixels(256, Look::Normal), width: 256, height: 256 };
    let viewport = egui::ViewportBuilder::default()
        .with_title("RecallCraft")
        .with_app_id("io.github.billsecond.recallcraft")
        .with_inner_size([1200.0, 780.0])
        .with_min_inner_size([780.0, 480.0])
        .with_icon(Arc::new(icon));
    let mut options = eframe::NativeOptions { viewport, ..Default::default() };
    let backends = eframe::wgpu::Backends::from_env().or_else(graphics::fallback_backends);
    graphics::configure(&mut options, backends);
    let opening = graphics::Opening::start();
    let result = eframe::run_native(
        "RecallCraft",
        options,
        Box::new(move |cc| {
            graphics::log_errors(cc);
            style::setup(&cc.egui_ctx);
            let wake = cc.egui_ctx.clone();
            guard.set_waker(Box::new(move || wake.request_repaint()));
            Ok(Box::new(Browser::new(guard, if settings_page { Page::Settings } else { Page::Start }, opening)))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("the window could not open: {e}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Page {
    /// The live call, else the latest call, else everything.
    Start,
    Call(String),
    Everything,
    Settings,
}

/// What a page asks the window to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Play from a moment (ms), until `end` if given; `live` follows the recording.
    Play {
        at_ms: u64,
        end_ms: Option<u64>,
        live: bool,
    },
    Toggle,
    Seek(u64),
    Speed(f32),
    Mix(Mix),
    Export {
        call: Call,
        lines: Vec<Line>,
        from_ms: u64,
        to_ms: u64,
    },
    DeleteCall(String),
    Open(Page),
    Note(String),
    /// Hand these to the service (Check now, Update now).
    Tell(Vec<String>),
    /// Quit RecallCraft (the service, and the window).
    Quit,
}

enum Confirm {
    DeleteCall(String),
}

pub struct Browser {
    guard: instance::Guard,
    layout: Layout,
    calls: CallStore,
    settings: Settings,
    settings_seen: Option<SystemTime>,
    list: Vec<Call>,
    listed: Option<Instant>,
    status: Option<Status>,
    status_read: Option<Instant>,
    page: Page,
    view: Option<call_view::CallView>,
    player: Option<Player>,
    player_error: Option<String>,
    search: String,
    searched: String,
    search_at: Option<Instant>,
    found: Option<Vec<(Call, Vec<Line>)>>,
    export: Option<export_dialog::Dialog>,
    confirm: Option<Confirm>,
    note: Option<(String, Instant)>,
    everything: everything::State,
    settings_state: settings_page::State,
    reader: RingReader,
    opening: graphics::Opening,
    /// Hidden in the tray (closed or minimised while the service runs).
    hidden: bool,
    /// Closing for real (Quit).
    quitting: bool,
    /// Saying, the first time, that RecallCraft goes on in the tray.
    tray_note: bool,
    /// The window was minimised when last looked at (it says so for a frame after it's shown).
    minimised: bool,
    /// When the service was last told this window is playing (its sound is in the recording).
    told_playing: Option<Instant>,
    /// What the service says of updates (while it runs).
    updates: Option<crate::update::Shared>,
}

/// Whether the window can go back to a tray: it was opened by the service, and the service is
/// running.
fn has_tray(status: Option<&Status>) -> bool {
    std::env::var_os(crate::FROM_SERVICE).is_some() && (status.is_some() || instance::running("service"))
}

/// Asks the service to quit (it closes the window too).
fn quit_service() {
    std::thread::spawn(|| {
        instance::tell("service", &["--quit".to_string()]);
    });
}

impl Browser {
    fn new(guard: instance::Guard, page: Page, opening: graphics::Opening) -> Browser {
        let settings = prefs::load();
        let layout = Layout::new(paths::data_dir(&settings));
        Browser {
            guard,
            calls: CallStore::new(&layout.calls()),
            reader: RingReader::new(&layout.ring()),
            layout,
            settings_seen: prefs::modified(),
            settings,
            list: Vec::new(),
            listed: None,
            status: None,
            status_read: None,
            page,
            view: None,
            player: None,
            player_error: None,
            search: String::new(),
            searched: String::new(),
            search_at: None,
            found: None,
            export: None,
            confirm: None,
            note: None,
            everything: everything::State::default(),
            settings_state: settings_page::State::default(),
            opening,
            hidden: false,
            quitting: false,
            tray_note: false,
            minimised: false,
            told_playing: None,
            updates: None,
        }
    }

    /// While it plays, the service hears so every two seconds: what's played is in the computer's
    /// sound, and isn't written down as the others'.
    fn tell_playing(&mut self) {
        let playing = self.player.as_ref().is_some_and(Player::is_playing);
        if !playing || self.told_playing.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
            return;
        }
        self.told_playing = Some(Instant::now());
        let until = time::now_us() + 4_000_000;
        std::thread::spawn(move || {
            instance::tell("service", &["--own-sound".to_string(), until.to_string()]);
        });
    }

    fn show(&mut self, ctx: &egui::Context) {
        self.hidden = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }

    fn hide(&mut self, ctx: &egui::Context) {
        self.hidden = true;
        self.tray_note = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    fn quit(&mut self, ctx: &egui::Context) {
        self.quitting = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Closing or minimising the window puts it in the tray, while there's a tray to go back to;
    /// only Quit ends it.
    fn hide_when_closed(&mut self, ctx: &egui::Context) {
        let (close, minimised) = ctx.input(|i| (i.viewport().close_requested(), i.viewport().minimized == Some(true)));
        // Only the moment it's minimised counts.
        let just_minimised = minimised && !self.minimised && !self.hidden;
        self.minimised = minimised;
        if self.quitting || !(close || just_minimised) {
            return;
        }
        if !has_tray(self.status.as_ref()) {
            // Started on its own, with no service running: closing ends it.
            return;
        }
        if close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        if close && !self.settings.told_about_tray {
            self.tray_note = true;
            return;
        }
        self.hide(ctx);
    }

    /// The first time the window is closed: RecallCraft goes on in the tray.
    fn tray_note(&mut self, ctx: &egui::Context) {
        if !self.tray_note {
            return;
        }
        let mut choice = None;
        egui::Modal::new(egui::Id::new("tray-note")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading("RecallCraft keeps going in the tray");
            ui.add_space(4.0);
            let what = match self.settings.record_mode {
                recallcraft_model::RecordMode::Always => "It goes on recording",
                recallcraft_model::RecordMode::CallsOnly => "It goes on recording your calls",
            };
            ui.label(format!(
                "{what} while this window is closed. Click its icon in the tray, by the clock (it may be under the ^), to open it again. Quit is in its menu."
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    choice = Some(false);
                }
                if ui.button("Quit RecallCraft").clicked() {
                    choice = Some(true);
                }
            });
        });
        if let Some(quit) = choice {
            self.tray_note = false;
            self.settings = prefs::update(|x| x.told_about_tray = true);
            self.settings_seen = prefs::modified();
            if quit {
                quit_service();
                self.quit(ctx);
            } else {
                self.hide(ctx);
            }
        }
    }

    fn note(&mut self, text: impl Into<String>) {
        self.note = Some((text.into(), Instant::now()));
    }

    /// What changed on disk, and what was handed over.
    fn poll(&mut self, ctx: &egui::Context) {
        for args in self.guard.handed() {
            if args.iter().any(|a| a == "--quit") {
                self.quit(ctx);
                return;
            }
            if args.iter().any(|a| a == "--settings") {
                self.page = Page::Settings;
            }
            self.show(ctx);
        }
        let seen = prefs::modified();
        if seen != self.settings_seen {
            self.settings_seen = seen;
            self.settings = prefs::load();
        }
        if self.status_read.is_none_or(|t| t.elapsed() > Duration::from_millis(900)) {
            self.status_read = Some(Instant::now());
            self.status = std::fs::read_to_string(self.layout.status())
                .ok()
                .and_then(|t| serde_json::from_str::<Status>(&t).ok())
                .filter(|s| time::now_ms().saturating_sub(s.at_ms) < 8000);
            // The service's word on updates, while it runs and the settings are open.
            self.updates = if self.status.is_some() && self.page == Page::Settings { crate::update::read_shared() } else { None };
        }
        let live = self.status.as_ref().is_some_and(Status::in_call);
        if self.listed.is_none_or(|t| t.elapsed() > Duration::from_secs(if live { 2 } else { 5 })) {
            self.listed = Some(Instant::now());
            self.list = self.calls.list();
        }
        if self.page == Page::Start {
            self.page = match (self.status.as_ref().filter(|s| s.in_call()), self.list.first()) {
                (Some(s), _) => Page::Call(s.call_id.clone()),
                (None, Some(c)) => Page::Call(c.id.clone()),
                (None, None) => Page::Everything,
            };
        }
        if let Page::Call(id) = &self.page {
            match self.view.as_mut() {
                Some(v) if &v.call.id == id => v.refresh(&self.calls),
                _ => {
                    if let Some(p) = &self.player {
                        p.set_playing(false);
                    }
                    self.view = call_view::CallView::open(&self.calls, id);
                    if self.view.is_none() {
                        self.page = Page::Start;
                    }
                }
            }
        }
        let typed = self.search.trim().to_string();
        if typed != self.searched {
            if typed.chars().count() < 2 {
                self.found = None;
                self.searched = typed;
            } else if self.search_at.is_none_or(|t| t.elapsed() > Duration::from_millis(300)) {
                self.found = Some(self.calls.search(&typed));
                self.searched = typed;
            }
            self.search_at = Some(Instant::now());
        }
        if self.note.as_ref().is_some_and(|(_, t)| t.elapsed() > Duration::from_secs(5)) {
            self.note = None;
        }
    }

    fn player(&mut self) -> Option<&Player> {
        if self.player.is_none() {
            match Player::new(RingSource::new(RingReader::new(&self.layout.ring()), true)) {
                Ok(p) => {
                    p.set_speed(self.settings.playback_speed);
                    self.player = Some(p);
                    self.player_error = None;
                }
                Err(e) => {
                    log::warn!("playback: {e}");
                    self.player_error = Some(e.to_string());
                }
            }
        }
        self.player.as_ref()
    }

    fn act(&mut self, a: Action) {
        match a {
            Action::Play { at_ms, end_ms, live } => {
                if let Some(p) = self.player() {
                    p.set_live(live);
                    p.play_from(at_ms * 1000, end_ms.map(|e| e * 1000));
                }
            }
            Action::Toggle => {
                if let Some(p) = self.player() {
                    p.set_playing(!p.is_playing());
                }
            }
            Action::Seek(ms) => {
                if let Some(p) = self.player() {
                    p.seek(ms * 1000);
                }
            }
            Action::Speed(s) => {
                self.settings = prefs::update(|x| x.playback_speed = s);
                self.settings_seen = prefs::modified();
                if let Some(p) = &self.player {
                    p.set_speed(s);
                }
            }
            Action::Mix(m) => {
                if let Some(p) = self.player() {
                    p.set_mix(m);
                }
            }
            Action::Export { call, lines, from_ms, to_ms } => {
                self.export = Some(export_dialog::Dialog::new(call, lines, from_ms, to_ms, &self.settings));
            }
            Action::DeleteCall(id) => self.confirm = Some(Confirm::DeleteCall(id)),
            Action::Open(p) => self.page = p,
            Action::Note(n) => self.note(n),
            Action::Tell(args) => {
                std::thread::spawn(move || {
                    if !instance::tell("service", &args) {
                        log::warn!("the service didn't hear {args:?}");
                    }
                });
            }
            // Handled where the window's context is at hand.
            Action::Quit => {}
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) -> Vec<Action> {
        let mut out = Vec::new();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("RecallCraft").strong().size(17.0));
            ui.add_space(12.0);
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search what was said…").desired_width(260.0));
            if !self.search.is_empty() && ui.small_button("✕").on_hover_text("Clear the search").clicked() {
                self.search.clear();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.selectable_label(self.page == Page::Settings, "⚙ Settings").clicked() {
                    out.push(Action::Open(Page::Settings));
                }
                let now = time::now_ms();
                let s = self.settings.clone();
                if s.recording_at(now) {
                    ui.menu_button("Pause", |ui| {
                        for (label, minutes) in [("For 15 minutes", 15u64), ("For an hour", 60), ("Until I resume", 0)] {
                            if ui.button(label).clicked() {
                                self.settings = prefs::update(|x| {
                                    if minutes == 0 {
                                        x.recording = false;
                                    } else {
                                        x.recording = true;
                                        x.paused_until_ms = now + minutes * 60_000;
                                    }
                                });
                                self.settings_seen = prefs::modified();
                                ui.close();
                            }
                        }
                    });
                } else if ui.button("▶ Resume recording").clicked() {
                    self.settings = prefs::update(|x| {
                        x.recording = true;
                        x.paused_until_ms = 0;
                    });
                    self.settings_seen = prefs::modified();
                }
                let (dot, text) = match &self.status {
                    None => (egui::Color32::GRAY, "Not running".to_string()),
                    Some(_) if !s.recording => (egui::Color32::GRAY, "Paused".to_string()),
                    Some(_) if !s.recording_at(now) => (egui::Color32::GRAY, format!("Paused until {}", time::local_time(s.paused_until_ms))),
                    Some(st) if st.in_call() => {
                        (style::RED, format!("{} · {}", st.call_title, time::duration_words(now.saturating_sub(st.call_started_ms))))
                    }
                    Some(st) if st.waiting_for_call => (egui::Color32::GRAY, "Waiting for a call (recording only during calls)".to_string()),
                    Some(st) if !st.problems.is_empty() => (style::AMBER, st.problems.join("; ")),
                    Some(st) if st.recording => (style::TEAL, "Recording".to_string()),
                    Some(_) => (egui::Color32::GRAY, "Starting…".to_string()),
                };
                if self.status.is_none()
                    && ui.button("Start recording").on_hover_text("Starts RecallCraft's service").clicked()
                    && let Ok(exe) = std::env::current_exe()
                {
                    let _ = std::process::Command::new(exe).arg("--tray").spawn();
                }
                ui.label(egui::RichText::new(text).small());
                ui.label(egui::RichText::new("●").color(dot));
            });
        });
        out
    }

    fn call_list(&mut self, ui: &mut egui::Ui) -> Vec<Action> {
        let mut out = Vec::new();
        let now = time::now_ms();
        let dark = ui.visuals().dark_mode;
        egui::ScrollArea::vertical().id_salt("calls").auto_shrink([false, false]).show(ui, |ui| {
            if let Some(found) = &self.found {
                ui.label(egui::RichText::new(format!("{} calls with “{}”", found.len(), self.searched)).strong());
                ui.add_space(4.0);
                for (call, hits) in found {
                    let selected = self.page == Page::Call(call.id.clone());
                    let r = ui.selectable_label(
                        selected,
                        format!("{}\n{} {}", call.display_title(), time::local_date(call.started_ms), time::local_time(call.started_ms)),
                    );
                    if r.clicked() {
                        out.push(Action::Open(Page::Call(call.id.clone())));
                    }
                    for h in hits.iter().take(3) {
                        let who = call.speaker_name(h.speaker, &self.settings.your_name);
                        let r = ui.add(
                            egui::Label::new(egui::RichText::new(format!("  {who}: {}", h.text)).small().weak())
                                .truncate()
                                .sense(egui::Sense::click()),
                        );
                        if r.clicked() {
                            out.push(Action::Open(Page::Call(call.id.clone())));
                            out.push(Action::Play { at_ms: h.start_ms, end_ms: None, live: false });
                        }
                    }
                    ui.separator();
                }
                return;
            }
            if ui.selectable_label(self.page == Page::Everything, format!("🕘  Everything, last {} hours", self.settings.keep_audio_hours)).clicked()
            {
                out.push(Action::Open(Page::Everything));
            }
            ui.separator();
            if self.list.is_empty() {
                ui.label(
                    egui::RichText::new("No calls yet. When a call starts in Teams, Zoom, Slack or another calling app, it shows up here.").weak(),
                );
            }
            let mut day = String::new();
            for c in &self.list {
                let heading = time::day_heading(c.started_ms, now);
                if heading != day {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(&heading).small().strong());
                    day = heading;
                }
                let selected = self.page == Page::Call(c.id.clone());
                let frame = egui::Frame::NONE
                    .fill(if selected { ui.visuals().selection.bg_fill.gamma_multiply(0.35) } else { egui::Color32::TRANSPARENT })
                    .inner_margin(egui::Margin::symmetric(6, 4))
                    .corner_radius(6.0);
                let r = frame
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            if c.is_live() {
                                ui.label(egui::RichText::new("●").color(style::RED)).on_hover_text("Going on now");
                            }
                            if c.keep {
                                ui.label("📌").on_hover_text("Kept until you delete it");
                            }
                            ui.add(egui::Label::new(egui::RichText::new(c.display_title()).strong()).truncate());
                        });
                        let len = if c.is_live() { "now".to_string() } else { time::duration_words(c.duration_ms(now)) };
                        ui.label(egui::RichText::new(format!("{} · {} · {len}", c.app, time::local_time(c.started_ms))).small().color(if dark {
                            egui::Color32::from_gray(160)
                        } else {
                            egui::Color32::from_gray(90)
                        }));
                    })
                    .response
                    .interact(egui::Sense::click());
                if r.clicked() {
                    out.push(Action::Open(Page::Call(c.id.clone())));
                }
            }
        });
        out
    }

    fn confirm(&mut self, ctx: &egui::Context) {
        let Some(Confirm::DeleteCall(id)) = &self.confirm else { return };
        let id = id.clone();
        let title = self.calls.load(&id).map(|c| c.display_title()).unwrap_or_default();
        let mut close = false;
        egui::Modal::new(egui::Id::new("confirm-delete")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading("Delete this call?");
            ui.label(format!("“{title}” and its transcript are deleted. Its sound goes when the recording around it does."));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Delete").clicked() {
                    match self.calls.delete(&id) {
                        Ok(()) => {
                            self.note("Deleted");
                            self.view = None;
                            self.page = Page::Start;
                            self.listed = None;
                        }
                        Err(e) => self.note(format!("Couldn't delete it: {e}")),
                    }
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if close {
            self.confirm = None;
        }
    }
}

impl eframe::App for Browser {
    /// Runs while the window is hidden too: what's handed over (Show, Quit), and closing or
    /// minimising into the tray.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        self.hide_when_closed(ctx);
        self.tell_playing();
        if self.hidden {
            // Hidden in the tray: a look now and then for the tray's Show.
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let mut actions = Vec::new();
        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(4.0);
            actions.extend(self.top_bar(ui));
            ui.add_space(2.0);
        });
        egui::Panel::left("calls").resizable(true).default_size(300.0).min_size(220.0).show(ui, |ui| {
            ui.add_space(4.0);
            actions.extend(self.call_list(ui));
        });
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(e) = &self.player_error {
                ui.colored_label(style::AMBER, format!("Playback isn't possible: {e}"));
            }
            match self.page.clone() {
                Page::Settings => {
                    let layout = self.layout.clone();
                    let in_call = self.status.as_ref().is_some_and(Status::in_call);
                    let (changed, acts) =
                        settings_page::show(ui, &mut self.settings, &mut self.settings_state, &layout, self.updates.as_ref(), in_call);
                    if changed {
                        let s = self.settings.clone();
                        self.settings = prefs::update(|x| *x = s);
                        self.settings_seen = prefs::modified();
                    }
                    actions.extend(acts);
                }
                Page::Everything => {
                    actions.extend(everything::show(ui, &mut self.everything, &mut self.reader, &self.settings, &self.list, self.player.as_ref()));
                }
                Page::Call(_) | Page::Start => {
                    if let Some(v) = self.view.as_mut() {
                        let settings_dir = paths::settings_dir().unwrap_or_else(|| PathBuf::from("."));
                        actions.extend(call_view::show(
                            ui,
                            v,
                            self.player.as_ref(),
                            &mut self.settings,
                            &self.calls,
                            &mut self.reader,
                            &settings_dir,
                        ));
                    } else {
                        ui.label("Nothing to show yet.");
                    }
                }
            }
        });
        if let Some((n, _)) = &self.note {
            egui::Area::new(egui::Id::new("note")).anchor(egui::Align2::CENTER_BOTTOM, [0.0, -16.0]).show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.label(n);
                });
            });
        }
        if let Some(d) = self.export.as_mut() {
            let your = self.settings.your_name.clone();
            match export_dialog::show(&ctx, d, &self.layout, &your) {
                export_dialog::Outcome::Open => {}
                export_dialog::Outcome::Closed(remember) => {
                    if let Some(e) = remember {
                        self.settings = prefs::update(|x| x.export = e);
                        self.settings_seen = prefs::modified();
                    }
                    self.export = None;
                }
            }
        }
        self.confirm(&ctx);
        self.tray_note(&ctx);
        for a in actions {
            if a == Action::Quit {
                quit_service();
                self.quit(&ctx);
                continue;
            }
            self.act(a);
        }
        self.opening.opened();
        let playing = self.player.as_ref().is_some_and(Player::is_playing);
        ctx.request_repaint_after(Duration::from_millis(if playing { 60 } else { 500 }));
    }

    fn on_exit(&mut self) {
        self.player = None;
        self.guard.close();
    }
}
