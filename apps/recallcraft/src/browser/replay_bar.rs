//! The replay bar (`recallcraft --replay-bar`): while the Replay shortcut's replay plays, a small
//! bar at the bottom of the screen says how far it has got, with a Stop. The service starts it
//! with the replay and it closes when the replay ends ([`crate::replay`]), or when Stop is
//! pressed. It never takes the focus, so the call's window keeps it.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Color32, RichText, vec2};
use recallcraft_desktop::icon::{self, Look};
use recallcraft_desktop::{instance, logging};
use recallcraft_model::time;

use crate::graphics;
use crate::replay::{self, ReplayState};

const SIZE: egui::Vec2 = vec2(400.0, 78.0);
const DARK: Color32 = Color32::from_rgb(0x1F, 0x1F, 0x25);
/// How long it stays once the replay has ended, so its end is seen.
const LINGER: Duration = Duration::from_millis(600);

pub fn run() -> ExitCode {
    let guard = match instance::start("replay-bar", &["--show".to_string()]) {
        instance::Start::First(g) => g,
        instance::Start::Handed => return ExitCode::SUCCESS,
        instance::Start::Unreachable(e) => {
            log::warn!("another replay bar is up but couldn't be reached: {e}");
            return ExitCode::FAILURE;
        }
    };
    logging::to_file("replay-bar");
    let Some(state) = replay::read().filter(|s| s.playing) else {
        return ExitCode::SUCCESS;
    };
    let icon = egui::IconData { rgba: icon::pixels(64, Look::Normal), width: 64, height: 64 };
    // Hidden until it's placed at the bottom of the screen it opens on.
    let viewport = egui::ViewportBuilder::default()
        .with_title("RecallCraft replay")
        .with_app_id("io.github.billsecond.recallcraft")
        .with_decorations(false)
        .with_resizable(false)
        .with_taskbar(false)
        .with_window_level(egui::WindowLevel::AlwaysOnTop)
        .with_active(false)
        .with_visible(false)
        .with_inner_size(SIZE)
        .with_icon(Arc::new(icon));
    let mut options = eframe::NativeOptions { viewport, ..Default::default() };
    let backends = eframe::wgpu::Backends::from_env().or_else(graphics::fallback_backends);
    graphics::configure(&mut options, backends);
    let result = eframe::run_native(
        "RecallCraft replay",
        options,
        Box::new(move |cc| {
            graphics::log_errors(cc);
            super::style::setup(&cc.egui_ctx);
            Ok(Box::new(Bar::new(guard, state)))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("the replay bar could not open: {e}");
            ExitCode::FAILURE
        }
    }
}

struct Bar {
    guard: instance::Guard,
    state: ReplayState,
    started: Instant,
    placed: bool,
    read: Instant,
    /// When the replay was seen to have ended.
    ended: Option<Instant>,
    closing: bool,
}

/// Where the bar goes on a screen of `monitor` points: the middle of its bottom, above the
/// taskbar.
fn bottom_middle(monitor: egui::Vec2) -> egui::Pos2 {
    let x = ((monitor.x - SIZE.x) / 2.0).max(0.0);
    let y = (monitor.y - SIZE.y - 72.0).max(0.0);
    egui::pos2(x, y)
}

/// `m:ss`.
fn clock(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

impl Bar {
    fn new(guard: instance::Guard, state: ReplayState) -> Bar {
        let now = Instant::now();
        Bar { guard, state, started: now, placed: false, read: now, ended: None, closing: false }
    }

    fn close(&mut self, ctx: &egui::Context) {
        self.closing = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for Bar {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.closing {
            return;
        }
        if !self.placed {
            let monitor = ctx.input(|i| i.viewport().monitor_size);
            if monitor.is_some() || self.started.elapsed() > Duration::from_millis(1500) {
                let at = monitor.map_or(egui::pos2(40.0, 40.0), bottom_middle);
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(at));
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                self.placed = true;
            }
        }
        for args in self.guard.handed() {
            if args.iter().any(|a| a == "--quit") {
                self.close(ctx);
                return;
            }
        }
        let now = Instant::now();
        // What the service says: this replay ended, or another started (followed).
        if now.saturating_duration_since(self.read) > Duration::from_millis(200) {
            self.read = now;
            match replay::read() {
                Some(s) if s.playing => {
                    if s.id != self.state.id {
                        self.ended = None;
                    }
                    self.state = s;
                }
                _ => {
                    self.ended.get_or_insert(now);
                }
            }
        }
        // It never stays up long after its replay should have ended (the service stopped).
        let overdue = time::now_ms() > self.state.started_ms + self.state.lasts_ms + 5_000;
        if self.ended.is_some_and(|t| now.saturating_duration_since(t) >= LINGER) || overdue {
            self.close(ctx);
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let s = self.state.clone();
        let now = time::now_ms();
        let done = if self.ended.is_some() { 1.0 } else { s.done(now) };
        let played = if self.ended.is_some() { s.seconds } else { s.played_seconds(now) };
        let mut stop = false;
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(DARK).inner_margin(egui::Margin::same(12))).show(ui, |ui| {
            ui.style_mut().visuals = egui::Visuals::dark();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(SIZE.x - 120.0);
                    let speed = if (s.speed - 1.0).abs() < 0.01 { String::new() } else { format!(" at {}×", s.speed) };
                    ui.label(RichText::new(format!("⏪ Replaying the last {} seconds{speed}", s.seconds)).strong().size(14.0).color(Color32::WHITE));
                    ui.add_space(2.0);
                    ui.add(egui::ProgressBar::new(done).desired_height(8.0));
                    ui.label(RichText::new(format!("{} of {}", clock(played), clock(s.seconds))).size(11.0).color(Color32::from_gray(190)));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(RichText::new("⏹ Stop").strong()).on_hover_text("Stop the replay").clicked() {
                        stop = true;
                    }
                });
            });
        });
        if stop {
            if !instance::tell("service", &["--stop-replay".to_string()]) {
                log::warn!("the service didn't hear --stop-replay");
            }
            self.close(&ctx);
        }
    }

    fn on_exit(&mut self) {
        self.guard.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_goes_at_the_bottom_in_the_middle() {
        assert_eq!(bottom_middle(vec2(1920.0, 1080.0)), egui::pos2((1920.0 - SIZE.x) / 2.0, 1080.0 - SIZE.y - 72.0));
        assert_eq!(bottom_middle(vec2(100.0, 100.0)), egui::pos2(0.0, 0.0), "never off the screen");
        assert_eq!(clock(75), "1:15");
        assert_eq!(clock(30), "0:30");
    }
}
