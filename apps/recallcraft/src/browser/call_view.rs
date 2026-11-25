//! A call: its title and who was on it (each can be renamed), its sound (play, skip, speed, the
//! activity of each side along the call to click into), and its transcript, the line being
//! played marked and kept in view. During a call, lines appear a few seconds after they're said.

use std::path::Path;
use std::time::{Duration, Instant};

use egui::{Align, Color32, Key, Label, Margin, RichText, Sense, TextEdit, vec2};
use recallcraft_audio::playback::{Mix, Player};
use recallcraft_desktop::settings as prefs;
use recallcraft_model::settings::SPEEDS;
use recallcraft_model::{Call, Line, Settings, Speaker, Stream, TranscriptFormat, time};
use recallcraft_store::calls::CallStore;
use recallcraft_store::ring::{RingReader, SILENT_DB};

use super::{Action, Page, style};

pub struct CallView {
    pub call: Call,
    pub lines: Vec<Line>,
    len: u64,
    checked: Instant,
    call_read: Instant,
    rename: Option<(Speaker, String)>,
    title: Option<String>,
    activity: Vec<(f32, f32)>,
    activity_for: (u64, u64, usize),
    activity_at: Option<Instant>,
    scrolled_to: Option<usize>,
    mix: Mix,
}

impl CallView {
    pub fn open(calls: &CallStore, id: &str) -> Option<CallView> {
        let call = calls.load(id)?;
        let lines = calls.lines(id);
        Some(CallView {
            len: calls.transcript_len(id),
            call,
            lines,
            checked: Instant::now(),
            call_read: Instant::now(),
            rename: None,
            title: None,
            activity: Vec::new(),
            activity_for: (0, 0, 0),
            activity_at: None,
            scrolled_to: None,
            mix: Mix::default(),
        })
    }

    /// What the service wrote since (lines heard, the title found, the call ended).
    pub fn refresh(&mut self, calls: &CallStore) {
        if self.checked.elapsed() >= Duration::from_millis(700) {
            self.checked = Instant::now();
            let len = calls.transcript_len(&self.call.id);
            if len != self.len {
                self.len = len;
                self.lines = calls.lines(&self.call.id);
            }
        }
        if self.call_read.elapsed() >= Duration::from_secs(2) && self.title.is_none() && self.rename.is_none() {
            self.call_read = Instant::now();
            if let Some(c) = calls.load(&self.call.id) {
                self.call = c;
            }
        }
    }

    fn save(&mut self, calls: &CallStore) -> Option<Action> {
        // As it is on disk now, with this change: the service may have changed it meanwhile.
        let mut on_disk = calls.load(&self.call.id).unwrap_or_else(|| self.call.clone());
        on_disk.title = self.call.title.clone();
        on_disk.titled_by_owner = self.call.titled_by_owner;
        on_disk.names = self.call.names.clone();
        on_disk.keep = self.call.keep;
        self.call = on_disk;
        calls.save(&self.call).err().map(|e| Action::Note(format!("Couldn't save: {e}")))
    }

    /// Who spoke, in the order they first did (you first).
    fn people(&self) -> Vec<Speaker> {
        let mut out = vec![Speaker::Me];
        for l in &self.lines {
            if !out.contains(&l.speaker) {
                out.push(l.speaker);
            }
        }
        out
    }
}

fn level_height(db: f32) -> f32 {
    if db <= SILENT_DB + 1.0 {
        return 0.0;
    }
    ((db + 65.0) / 55.0).clamp(0.04, 1.0)
}

pub fn show(
    ui: &mut egui::Ui,
    v: &mut CallView,
    player: Option<&Player>,
    settings: &mut Settings,
    calls: &CallStore,
    reader: &mut RingReader,
    settings_dir: &Path,
) -> Vec<Action> {
    let mut out = Vec::new();
    let now = time::now_ms();
    let dark = ui.visuals().dark_mode;
    let (start, end) = (v.call.started_ms, v.call.end_or(now));
    let live = v.call.is_live();

    // The title, and what can be done with the call.
    ui.horizontal(|ui| {
        if let Some(t) = v.title.as_mut() {
            let r = ui.add(TextEdit::singleline(t).desired_width(420.0).font(egui::TextStyle::Heading));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if ui.button("Save").clicked() || enter {
                v.call.title = t.trim().to_string();
                v.call.titled_by_owner = !v.call.title.is_empty();
                v.title = None;
                out.extend(v.save(calls));
            } else if ui.button("Cancel").clicked() {
                v.title = None;
            }
        } else {
            let r = ui
                .add(Label::new(RichText::new(v.call.display_title()).size(21.0).strong()).sense(Sense::click()))
                .on_hover_text("Double-click to rename it");
            if r.double_clicked() {
                v.title = Some(v.call.display_title());
            }
        }
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if ui.button("🗑").on_hover_text("Delete this call").clicked() {
                out.push(Action::DeleteCall(v.call.id.clone()));
            }
            if ui.button("Export…").clicked() {
                out.push(Action::Export { call: v.call.clone(), lines: v.lines.clone(), from_ms: start, to_ms: end });
            }
            if ui.button("Copy text").on_hover_text("Copy the transcript").clicked() {
                let text = recallcraft_store::transcript::render(TranscriptFormat::Text, &v.call, &v.lines, &settings.your_name);
                ui.ctx().copy_text(text);
                out.push(Action::Note("The transcript is copied".to_string()));
            }
            let keep = v.call.keep;
            if ui.selectable_label(keep, "📌 Keep").on_hover_text("Kept until you delete it, however long the settings keep calls").clicked() {
                v.call.keep = !keep;
                out.extend(v.save(calls));
            }
        });
    });
    ui.horizontal(|ui| {
        let when = format!("{} {}", time::day_heading(start, now), time::local_time(start));
        let len = time::duration_words(end.saturating_sub(start));
        ui.label(RichText::new(format!("{} · {when} · {len}", v.call.app)).weak());
        if live {
            ui.label(RichText::new("● Live").color(style::RED).strong());
        }
    });

    // Who was on it.
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("People").weak());
        for s in v.people() {
            let name = v.call.speaker_name(s, &settings.your_name);
            let tip = match s {
                Speaker::Me => "You (the microphone). Click to change your name",
                Speaker::Others => "The others (the computer's sound). Click to name them",
                Speaker::Voice(_) => "One of the others, told apart by voice. Click to name them; RecallCraft knows the voice again next time",
            };
            if ui.button(RichText::new(&name).color(style::speaker(s, dark))).on_hover_text(tip).clicked() {
                v.rename = Some((
                    s,
                    if matches!(s, Speaker::Voice(_) | Speaker::Others) && !v.call.names.contains_key(&s.key()) { String::new() } else { name },
                ));
            }
        }
    });
    if !v.call.people.is_empty() {
        let shown: Vec<&str> = v.call.people.iter().take(30).map(String::as_str).collect();
        let more = v.call.people.len().saturating_sub(shown.len());
        let list = if more > 0 { format!("{} and {more} more", shown.join(", ")) } else { shown.join(", ") };
        let app = if v.call.app.trim().is_empty() { "the app" } else { v.call.app.trim() };
        ui.label(RichText::new(format!("In the meeting, as {app} showed it: {list}")).small().weak());
    }
    let mut done_renaming = false;
    if let Some((s, text)) = v.rename.as_mut() {
        let s = *s;
        ui.horizontal(|ui| {
            ui.label(format!("Name for {}:", v.call.speaker_name(s, &settings.your_name)));
            let r = ui.add(TextEdit::singleline(text).desired_width(220.0).hint_text("Their name"));
            r.request_focus();
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if ui.button("Save").clicked() || enter {
                let name = text.trim().to_string();
                if s == Speaker::Me {
                    *settings = prefs::update(|x| x.your_name = name.clone());
                } else {
                    v.call.set_name(s, &name);
                    if let (Speaker::Voice(n), false) = (s, name.is_empty())
                        && let Some(dir) = calls.dir(&v.call.id)
                    {
                        recallcraft_engine::known::learn_from_call(settings_dir, &dir, n, &name);
                    }
                }
                done_renaming = true;
            }
            if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                done_renaming = true;
            }
        });
    }
    if done_renaming {
        v.rename = None;
        out.extend(v.save(calls));
    }
    ui.add_space(4.0);

    // The sound.
    let pos = player.map(|p| p.position_us() / 1000).filter(|p| (start.saturating_sub(1000)..=end + 5000).contains(p));
    let playing = pos.is_some() && player.is_some_and(Player::is_playing);
    let at = pos.unwrap_or(start).clamp(start, end);
    let play_from = |t: u64| Action::Play { at_ms: t, end_ms: v.call.ended_ms, live };
    ui.horizontal(|ui| {
        if ui.button("⏪ 15 s").clicked() {
            out.push(if pos.is_some() { Action::Seek(at.saturating_sub(15_000).max(start)) } else { play_from(start) });
        }
        if ui.button(if playing { "⏸ Pause" } else { "▶ Play" }).clicked() {
            out.push(if pos.is_some() { Action::Toggle } else { play_from(start) });
        }
        if ui.button("15 s ⏩").clicked() {
            out.push(if pos.is_some() { Action::Seek((at + 15_000).min(end)) } else { play_from(start) });
        }
        ui.label(RichText::new(format!("{} / {}", time::clock(at - start), time::clock(end - start))).monospace());
        let mut speed = settings.playback_speed;
        egui::ComboBox::from_id_salt("speed").width(70.0).selected_text(format!("{speed}×")).show_ui(ui, |ui| {
            for s in SPEEDS {
                ui.selectable_value(&mut speed, s, format!("{s}×"));
            }
        });
        if (speed - settings.playback_speed).abs() > f32::EPSILON {
            out.push(Action::Speed(speed));
        }
        let mut mix = v.mix;
        let mut me_on = mix.mic > 0.0;
        let mut them_on = mix.system > 0.0;
        ui.checkbox(&mut me_on, v.call.speaker_name(Speaker::Me, &settings.your_name)).on_hover_text("Hear your side");
        ui.checkbox(&mut them_on, "Them").on_hover_text("Hear the others");
        mix.mic = if me_on { 1.0 } else { 0.0 };
        mix.system = if them_on { 1.0 } else { 0.0 };
        if mix != v.mix {
            v.mix = mix;
            out.push(Action::Mix(mix));
        }
        if live {
            if ui
                .button("⏪ Catch up")
                .on_hover_text(format!("Play the last {} seconds, faster, until it's caught up", settings.replay_seconds))
                .clicked()
            {
                out.push(Action::Speed(settings.replay_speed.max(settings.playback_speed)));
                out.push(Action::Play { at_ms: now.saturating_sub(u64::from(settings.replay_seconds) * 1000), end_ms: None, live: true });
            }
            if ui.button(RichText::new("● Live").color(style::RED)).on_hover_text("Listen from a few seconds ago").clicked() {
                out.push(Action::Speed(1.0));
                out.push(Action::Play { at_ms: now.saturating_sub(4000), end_ms: None, live: true });
            }
        }
    });

    // Activity along the call: you above the line, them below; click or drag to go there.
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click_and_drag());
    let buckets = (rect.width() / 3.0).clamp(40.0, 600.0) as usize;
    let fresh = v.activity_at.is_some_and(|t| t.elapsed() < Duration::from_secs(if live { 3 } else { 600 }));
    if !fresh || v.activity_for != (start, if live { 0 } else { end }, buckets) {
        let (a, b) = (start * 1000, end.max(start + 1000) * 1000);
        let step = ((b - a) / buckets as u64).max(20_000);
        let mic = reader.levels(Stream::Mic, a, b, step);
        let sys = reader.levels(Stream::System, a, b, step);
        v.activity = mic.into_iter().zip(sys).collect();
        v.activity_for = (start, if live { 0 } else { end }, buckets);
        v.activity_at = Some(Instant::now());
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, ui.visuals().extreme_bg_color);
    let mid = rect.center().y;
    let n = v.activity.len().max(1) as f32;
    let w = rect.width() / n;
    for (i, (m, s)) in v.activity.iter().enumerate() {
        let x = rect.left() + i as f32 * w;
        let (hm, hs) = (level_height(*m) * (rect.height() / 2.0 - 3.0), level_height(*s) * (rect.height() / 2.0 - 3.0));
        if hm > 0.0 {
            painter.rect_filled(
                egui::Rect::from_min_max(egui::pos2(x, mid - hm), egui::pos2(x + (w - 1.0).max(1.0), mid)),
                1.0,
                style::speaker(Speaker::Me, dark),
            );
        }
        if hs > 0.0 {
            painter.rect_filled(egui::Rect::from_min_max(egui::pos2(x, mid), egui::pos2(x + (w - 1.0).max(1.0), mid + hs)), 1.0, style::TEAL);
        }
    }
    let span = end.saturating_sub(start).max(1) as f32;
    let px = rect.left() + (at - start) as f32 / span * rect.width();
    painter.line_segment([egui::pos2(px, rect.top()), egui::pos2(px, rect.bottom())], egui::Stroke::new(2.0, ui.visuals().strong_text_color()));
    let to_time = |x: f32| start + (((x - rect.left()) / rect.width()).clamp(0.0, 1.0) * span) as u64;
    if let Some(p) = resp.hover_pos() {
        resp.clone().on_hover_text_at_pointer(time::clock(to_time(p.x) - start));
    }
    if (resp.clicked() || resp.drag_stopped())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let t = to_time(p.x);
        out.push(if playing { Action::Seek(t) } else { play_from(t) });
    }
    ui.add_space(6.0);

    // The transcript.
    let current = if pos.is_some() { v.lines.iter().rposition(|l| l.start_ms <= at) } else { None };
    egui::ScrollArea::vertical().id_salt(("transcript", v.call.id.as_str())).auto_shrink([false, false]).stick_to_bottom(live && !playing).show(
        ui,
        |ui| {
            if v.lines.is_empty() {
                let why = if !settings.transcribe_calls {
                    "Writing down calls is off in Settings."
                } else if live {
                    "Listening… what's said shows up here a few seconds after it's said."
                } else {
                    "Nothing was written down for this call."
                };
                ui.label(RichText::new(why).weak());
            }
            let mut last: Option<Speaker> = None;
            let mut last_end = 0u64;
            for (i, l) in v.lines.iter().enumerate() {
                if last != Some(l.speaker) || l.start_ms > last_end + 30_000 {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(v.call.speaker_name(l.speaker, &settings.your_name)).strong().color(style::speaker(l.speaker, dark)));
                        ui.label(RichText::new(time::clock(l.start_ms.saturating_sub(start))).small().weak());
                    });
                }
                let is_current = current == Some(i);
                let fill = if is_current { ui.visuals().selection.bg_fill.gamma_multiply(0.35) } else { Color32::TRANSPARENT };
                let r = egui::Frame::NONE
                    .fill(fill)
                    .inner_margin(Margin::symmetric(6, 2))
                    .corner_radius(4.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(Label::new(&l.text).wrap());
                    })
                    .response
                    .interact(Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if r.clicked() {
                    out.push(play_from(l.start_ms));
                }
                r.context_menu(|ui| {
                    if ui.button("Play from here").clicked() {
                        out.push(play_from(l.start_ms));
                        ui.close();
                    }
                    if ui.button("Copy this line").clicked() {
                        ui.ctx().copy_text(format!("{}: {}", v.call.speaker_name(l.speaker, &settings.your_name), l.text));
                        ui.close();
                    }
                });
                if is_current && playing && v.scrolled_to != Some(i) {
                    r.scroll_to_me(Some(Align::Center));
                    v.scrolled_to = Some(i);
                }
                last = Some(l.speaker);
                last_end = l.end_ms;
            }
            if live && !v.lines.is_empty() {
                ui.label(RichText::new("…").weak());
            }
        },
    );
    let _ = Page::Start;
    out
}
