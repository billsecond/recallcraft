//! The notice in the corner of the screen (`recallcraft --notice`), started by the service when
//! there's something to say about updates and no call is going on: a newer RecallCraft ready to
//! install (Update now, Later, What's new), or a word on how a check or an install went.
//!
//! It shows the notice the service wrote ([`crate::update::state_file`]) and closes when its
//! time is up (not while the pointer is on it), when a button is pressed, or when the service's
//! notice is another or none (a call started, the service stopped). It never takes the focus.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Color32, RichText, vec2};
use recallcraft_desktop::icon::{self, Look};
use recallcraft_desktop::{instance, logging};

use crate::graphics;
use crate::update::{Notice, read_shared};

const SIZE: egui::Vec2 = vec2(390.0, 118.0);
const DARK: Color32 = Color32::from_rgb(0x1F, 0x1F, 0x25);

pub fn run() -> ExitCode {
    let guard = match instance::start("notice", &["--show".to_string()]) {
        instance::Start::First(g) => g,
        instance::Start::Handed => return ExitCode::SUCCESS,
        instance::Start::Unreachable(e) => {
            log::warn!("another notice is up but couldn't be reached: {e}");
            return ExitCode::FAILURE;
        }
    };
    logging::to_file("notice");
    let Some(notice) = read_shared().and_then(|s| s.notice) else {
        return ExitCode::SUCCESS;
    };
    let icon = egui::IconData { rgba: icon::pixels(64, Look::Normal), width: 64, height: 64 };
    // Hidden until it's placed in the corner of the screen it opens on.
    let viewport = egui::ViewportBuilder::default()
        .with_title("RecallCraft")
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
        "RecallCraft notice",
        options,
        Box::new(move |cc| {
            graphics::log_errors(cc);
            super::style::setup(&cc.egui_ctx);
            Ok(Box::new(NoticeApp::new(guard, notice)))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("the notice could not open: {e}");
            ExitCode::FAILURE
        }
    }
}

struct NoticeApp {
    guard: instance::Guard,
    notice: Notice,
    until: Instant,
    started: Instant,
    placed: bool,
    hovered: bool,
    read: Instant,
    closing: bool,
}

/// Where the notice goes on a screen of `monitor` points: its bottom right corner, above the
/// taskbar.
fn corner(monitor: egui::Vec2) -> egui::Pos2 {
    let x = (monitor.x - SIZE.x - 24.0).max(0.0);
    let y = (monitor.y - SIZE.y - 72.0).max(0.0);
    egui::pos2(x, y)
}

/// Tells the service (the notice closes right after, so it isn't left to a thread).
fn tell(word: &str) {
    if !instance::tell("service", &[word.to_string()]) {
        log::warn!("the service didn't hear {word}");
    }
}

impl NoticeApp {
    fn new(guard: instance::Guard, notice: Notice) -> NoticeApp {
        let now = Instant::now();
        NoticeApp {
            guard,
            until: now + Duration::from_secs(notice.seconds.clamp(3, 120)),
            notice,
            started: now,
            placed: false,
            hovered: false,
            read: now,
            closing: false,
        }
    }

    fn close(&mut self, ctx: &egui::Context) {
        self.closing = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for NoticeApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.closing {
            return;
        }
        if !self.placed {
            let monitor = ctx.input(|i| i.viewport().monitor_size);
            // Shown where it belongs once the screen's size is known (or in a moment anyway).
            if monitor.is_some() || self.started.elapsed() > Duration::from_millis(1500) {
                let at = monitor.map_or(egui::pos2(40.0, 40.0), corner);
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
        if self.hovered {
            self.until = self.until.max(now + Duration::from_secs(2));
        }
        if now >= self.until {
            self.close(ctx);
            return;
        }
        // The service's notice is another, or none: a call started, or the service stopped.
        if now.saturating_duration_since(self.read) > Duration::from_millis(400) {
            self.read = now;
            if read_shared().and_then(|s| s.notice).is_none_or(|n| n.id != self.notice.id) {
                self.close(ctx);
                return;
            }
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let n = self.notice.clone();
        let mut act: Option<&str> = None;
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(DARK).inner_margin(egui::Margin::same(12))).show(ui, |ui| {
            ui.style_mut().visuals = egui::Visuals::dark();
            self.hovered = ui.ui_contains_pointer();
            ui.label(RichText::new(&n.title).strong().size(15.0).color(Color32::WHITE));
            ui.add(egui::Label::new(RichText::new(&n.detail).size(12.0).color(Color32::from_gray(190))).wrap());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if n.offer {
                    if n.ready {
                        if ui.button(RichText::new("Update now").strong()).clicked() {
                            act = Some("install");
                        }
                    } else if ui.button(RichText::new("Get it").strong()).clicked() {
                        act = Some("page-close");
                    }
                    if ui.button("Later").clicked() {
                        act = Some("later");
                    }
                    if n.ready && n.page.is_some() && ui.link("What's new").clicked() {
                        act = Some("page");
                    }
                } else {
                    if ui.button("OK").clicked() {
                        act = Some("close");
                    }
                    if n.page.is_some() && ui.link("What's new").clicked() {
                        act = Some("page-close");
                    }
                }
            });
        });
        let open_page = || {
            if let Some(p) = &n.page
                && let Err(e) = opener::open_browser(p)
            {
                log::warn!("opening {p}: {e}");
            }
        };
        match act {
            Some("install") => {
                tell("--install-update");
                self.close(&ctx);
            }
            Some("later") => {
                tell("--dismiss-update");
                self.close(&ctx);
            }
            Some("page") => open_page(),
            Some("page-close") => {
                open_page();
                if n.offer {
                    tell("--dismiss-update");
                }
                self.close(&ctx);
            }
            Some(_) => self.close(&ctx),
            None => {}
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
    fn the_notice_goes_in_the_bottom_right_corner() {
        let at = corner(vec2(1920.0, 1080.0));
        assert_eq!(at, egui::pos2(1920.0 - SIZE.x - 24.0, 1080.0 - SIZE.y - 72.0));
        assert_eq!(corner(vec2(100.0, 100.0)), egui::pos2(0.0, 0.0), "never off the screen");
    }
}
