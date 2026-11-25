//! Everything heard in the last hours: an hour a row, a cell a minute (you in the top half, the
//! computer's sound in the bottom half), calls marked under it. Click a minute to play from there;
//! export any hour. Hours with nothing heard are left out.

use std::time::{Duration, Instant};

use egui::{RichText, Sense, vec2};
use recallcraft_audio::playback::Player;
use recallcraft_model::{Call, Settings, Speaker, Stream, time};
use recallcraft_store::ring::{RingReader, SILENT_DB};

use super::{Action, style};

#[derive(Default)]
pub struct State {
    /// The loudest moment of each minute: (you, the computer).
    minutes: Vec<(f32, f32)>,
    from_ms: u64,
    at: Option<Instant>,
    hours: u32,
}

const HOUR: u64 = 3_600_000;
const MINUTE: u64 = 60_000;

fn shade(db: f32) -> f32 {
    if db <= SILENT_DB + 1.0 { 0.0 } else { ((db + 65.0) / 50.0).clamp(0.12, 1.0) }
}

pub fn show(ui: &mut egui::Ui, st: &mut State, reader: &mut RingReader, settings: &Settings, calls: &[Call], player: Option<&Player>) -> Vec<Action> {
    let mut out = Vec::new();
    let now = time::now_ms();
    let dark = ui.visuals().dark_mode;
    if st.at.is_none_or(|t| t.elapsed() > Duration::from_secs(20)) || st.hours != settings.keep_audio_hours {
        st.hours = settings.keep_audio_hours;
        let from = (now.saturating_sub(u64::from(st.hours) * HOUR)) / HOUR * HOUR;
        let mic = reader.levels(Stream::Mic, from * 1000, now * 1000, MINUTE * 1000);
        let sys = reader.levels(Stream::System, from * 1000, now * 1000, MINUTE * 1000);
        st.minutes = mic.into_iter().zip(sys).collect();
        st.from_ms = from;
        st.at = Some(Instant::now());
    }
    ui.heading(format!("Everything heard in the last {} hours", settings.keep_audio_hours));
    ui.label(RichText::new("Each row is an hour and each cell a minute: you above, the computer's sound below, calls marked in amber. Click a minute to play from there.").weak());
    let pos = player.filter(|p| p.is_playing()).map(|p| p.position_us() / 1000);
    if let Some(p) = pos {
        ui.horizontal(|ui| {
            if ui.button("⏹ Stop").clicked() {
                out.push(Action::Stop);
            }
            ui.label(format!("Playing {} {}", time::local_date(p), time::local_time(p)));
        });
    }
    ui.add_space(6.0);
    let hours = st.minutes.len().div_ceil(60);
    egui::ScrollArea::vertical().id_salt("everything").auto_shrink([false, false]).show(ui, |ui| {
        let mut quiet = 0usize;
        let mut day = String::new();
        for h in (0..hours).rev() {
            let hour_ms = st.from_ms + h as u64 * HOUR;
            let cells: Vec<(f32, f32)> = (0..60).map(|m| st.minutes.get(h * 60 + m).copied().unwrap_or((SILENT_DB, SILENT_DB))).collect();
            if cells.iter().all(|(a, b)| *a <= SILENT_DB + 1.0 && *b <= SILENT_DB + 1.0) {
                quiet += 1;
                continue;
            }
            if quiet > 0 {
                ui.label(RichText::new(format!("{quiet} quiet hour{}", if quiet == 1 { "" } else { "s" })).small().weak());
                quiet = 0;
            }
            let heading = time::day_heading(hour_ms, now);
            if heading != day {
                ui.add_space(4.0);
                ui.label(RichText::new(&heading).strong());
                day = heading;
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new(time::local_time(hour_ms)).monospace());
                let width = (ui.available_width() - 80.0).max(120.0);
                let (rect, resp) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
                let p = ui.painter_at(rect);
                p.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
                let w = rect.width() / 60.0;
                let mid = rect.top() + 13.0;
                for (m, (a, b)) in cells.iter().enumerate() {
                    let x = rect.left() + m as f32 * w;
                    let (sa, sb) = (shade(*a), shade(*b));
                    if sa > 0.0 {
                        p.rect_filled(
                            egui::Rect::from_min_size(egui::pos2(x, rect.top() + 1.0), vec2(w - 1.0, 11.0)),
                            1.0,
                            style::speaker(Speaker::Me, dark).gamma_multiply(sa),
                        );
                    }
                    if sb > 0.0 {
                        p.rect_filled(egui::Rect::from_min_size(egui::pos2(x, mid + 1.0), vec2(w - 1.0, 11.0)), 1.0, style::TEAL.gamma_multiply(sb));
                    }
                }
                for c in calls.iter().filter(|c| c.overlaps(hour_ms, hour_ms + HOUR, 0, now)) {
                    let a = c.started_ms.max(hour_ms) - hour_ms;
                    let b = c.end_or(now).min(hour_ms + HOUR) - hour_ms;
                    let (xa, xb) = (rect.left() + a as f32 / HOUR as f32 * rect.width(), rect.left() + b as f32 / HOUR as f32 * rect.width());
                    p.rect_filled(
                        egui::Rect::from_min_max(egui::pos2(xa, rect.bottom() - 4.0), egui::pos2(xb.max(xa + 2.0), rect.bottom())),
                        1.0,
                        style::AMBER,
                    );
                }
                if let Some(t) = pos.filter(|t| (hour_ms..hour_ms + HOUR).contains(t)) {
                    let x = rect.left() + (t - hour_ms) as f32 / HOUR as f32 * rect.width();
                    p.line_segment(
                        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                        egui::Stroke::new(2.0, ui.visuals().strong_text_color()),
                    );
                }
                let at = |x: f32| hour_ms + (((x - rect.left()) / rect.width()).clamp(0.0, 0.999) * HOUR as f32) as u64;
                if let Some(h) = resp.hover_pos() {
                    let t = at(h.x);
                    let call = calls.iter().find(|c| c.overlaps(t, t + 1, 0, now)).map(|c| format!(" · {}", c.display_title())).unwrap_or_default();
                    resp.clone().on_hover_text_at_pointer(format!("{}{call}", time::local_time_s(t)));
                }
                if resp.clicked()
                    && let Some(p) = resp.interact_pointer_pos()
                {
                    let t = at(p.x) / MINUTE * MINUTE;
                    out.push(Action::Play { at_ms: t, end_ms: None, live: now.saturating_sub(t) < 5 * MINUTE });
                }
                if ui.small_button("Export…").on_hover_text("Export this hour").clicked() {
                    let end = (hour_ms + HOUR).min(now);
                    let call = Call {
                        id: String::new(),
                        title: format!("Recording {} {}", time::local_date(hour_ms), time::local_time(hour_ms)),
                        started_ms: hour_ms,
                        ended_ms: Some(end),
                        ..Call::default()
                    };
                    out.push(Action::Export { call, lines: Vec::new(), from_ms: hour_ms, to_ms: end });
                }
            });
        }
        if quiet > 0 {
            ui.label(RichText::new(format!("{quiet} quiet hour{}", if quiet == 1 { "" } else { "s" })).small().weak());
        }
        if st.minutes.iter().all(|(a, b)| *a <= SILENT_DB + 1.0 && *b <= SILENT_DB + 1.0) {
            ui.label(RichText::new("Nothing has been heard yet.").weak());
        }
    });
    out
}
