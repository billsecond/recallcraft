//! The settings, saved as they're changed (the service picks them up within a second).

use std::sync::{Arc, Mutex, PoisonError};

use egui::RichText;
use recallcraft_audio::devices::{self, Device};
use recallcraft_desktop::service::{self as signin, Status as SignIn};
use recallcraft_desktop::{models, paths, version};
use recallcraft_model::settings::{MAX_KEEP_DAYS, MAX_KEEP_HOURS, MIN_KEEP_HOURS, SPEEDS};
use recallcraft_model::{AudioExport, MicMode, RecordMode, Settings, TranscriptFormat};
use recallcraft_store::Layout;

use super::Action;
use crate::update::Shared;

#[derive(Default)]
pub struct State {
    mics: Option<Vec<Device>>,
    size: Option<Arc<Mutex<Option<u64>>>>,
    signin: Option<SignIn>,
    more: Option<String>,
    never: Option<String>,
    confirm: Option<Confirm>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Confirm {
    DeleteAll,
    ForgetVoices,
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(14.0);
    ui.label(RichText::new(title).strong().size(16.0));
    ui.separator();
}

fn list_text(list: &[String]) -> String {
    list.join(", ")
}

fn list_of(text: &str) -> Vec<String> {
    text.split([',', ';', '\n']).map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()).collect()
}

/// The Updates part: the switch, Check now, and what the service found (`upd`, while it runs).
fn updates(ui: &mut egui::Ui, s: &mut Settings, upd: Option<&Shared>, in_call: bool, out: &mut Vec<Action>) {
    section(ui, "Updates");
    ui.checkbox(&mut s.check_updates, "Check for updates automatically");
    ui.label(
        RichText::new(
            "A minute after it starts and then every six hours (never during a call), RecallCraft asks GitHub for its latest release. \
             Nothing about you or what's recorded is sent.",
        )
        .small()
        .weak(),
    );
    let Some(u) = upd else {
        ui.label(RichText::new("Updates are checked while RecallCraft runs in the tray.").small().weak());
        return;
    };
    if u.flatpak {
        ui.label(RichText::new("The Flatpak has no network access, so RecallCraft can't check for updates here.").small().weak());
        ui.hyperlink_to("The releases page", format!("{}/releases", recallcraft_model::HOMEPAGE));
        return;
    }
    ui.horizontal(|ui| {
        if ui.add_enabled(!u.checking, egui::Button::new("Check now")).clicked() {
            out.push(Action::Tell(vec!["--check-updates".to_string(), "settings".to_string()]));
        }
        if !u.status.is_empty() {
            if u.failed {
                ui.colored_label(super::style::AMBER, &u.status);
            } else {
                ui.label(&u.status);
            }
        }
    });
    let Some(o) = &u.offer else { return };
    ui.add_space(4.0);
    if o.ready {
        ui.label(RichText::new(format!("RecallCraft {} is ready to install.", o.version)).strong());
        ui.horizontal(|ui| {
            let label = if u.installing {
                "Installing…".to_string()
            } else if u.after_call {
                "Installs when the call ends".to_string()
            } else {
                format!("Update now to {}", o.version)
            };
            if ui.add_enabled(!u.installing && !u.after_call, egui::Button::new(label)).clicked() {
                out.push(Action::Tell(vec!["--install-update".to_string()]));
            }
            ui.hyperlink_to("What's new", &o.page);
        });
        let note =
            if in_call { "During a call it's installed when the call ends, so the call's recording isn't cut short." } else { &u.install_note };
        ui.label(RichText::new(note).small().weak());
    } else {
        ui.label(RichText::new(format!("RecallCraft {} is available.", o.version)).strong());
        if let Some(p) = &o.problem {
            ui.colored_label(super::style::AMBER, format!("The download didn't work: {p}."));
        }
        ui.hyperlink_to("Get it from its release page", &o.page);
    }
}

/// Shows the settings; true when one changed.
pub fn show(ui: &mut egui::Ui, s: &mut Settings, st: &mut State, layout: &Layout, upd: Option<&Shared>, in_call: bool) -> (bool, Vec<Action>) {
    let before = s.clone();
    let mut out = Vec::new();
    egui::ScrollArea::vertical().id_salt("settings").auto_shrink([false, false]).show(ui, |ui| {
        ui.set_max_width(720.0);
        ui.heading("Settings");

        section(ui, "You");
        ui.horizontal(|ui| {
            ui.label("Your name");
            ui.add(egui::TextEdit::singleline(&mut s.your_name).hint_text("Me").desired_width(220.0));
        });
        ui.label(RichText::new("What you say into the microphone is labelled with it.").small().weak());

        section(ui, "Recording");
        ui.checkbox(&mut s.recording, "Recording is on").on_hover_text("Off is the same as the tray's Pause until I resume");
        ui.radio_value(&mut s.record_mode, RecordMode::Always, "Record all the time (keeps the last hours, calls or not)");
        ui.radio_value(&mut s.record_mode, RecordMode::CallsOnly, "Record only during calls");
        ui.label(
            RichText::new(
                "Only during calls, the microphone and the computer's sound are opened when a call starts and closed when it ends, \
                 so the first second or two of a call may be missed, and Replay has nothing outside calls.",
            )
            .small()
            .weak(),
        );
        ui.horizontal(|ui| {
            ui.label("Keep everything heard for");
            ui.add(egui::DragValue::new(&mut s.keep_audio_hours).range(MIN_KEEP_HOURS..=MAX_KEEP_HOURS).suffix(" hours"));
        });
        let mics = st.mics.get_or_insert_with(devices::microphones);
        ui.horizontal(|ui| {
            ui.label("Microphone");
            let shown = if s.microphone.is_empty() { "Follow the call (recommended)".to_string() } else { s.microphone.clone() };
            egui::ComboBox::from_id_salt("mic").width(320.0).selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut s.microphone, String::new(), "Follow the call (recommended)");
                for m in mics.iter() {
                    ui.selectable_value(&mut s.microphone, m.name.clone(), &m.name);
                }
            });
        });
        ui.label(RichText::new("Following the call records the microphone the calling app uses, and Windows' default one otherwise.").small().weak());
        ui.add_enabled_ui(s.record_mode == RecordMode::Always, |ui| {
            ui.label("Record the microphone");
            ui.radio_value(&mut s.mic_mode, MicMode::Auto, "All the time, but a Bluetooth headset's only during calls (recommended)");
            ui.radio_value(&mut s.mic_mode, MicMode::Always, "All the time");
            ui.radio_value(&mut s.mic_mode, MicMode::DuringCalls, "Only during calls");
        });
        ui.label(
            RichText::new("While any program records a Bluetooth headset's microphone, the headset switches to its hands-free mode and everything else it plays sounds worse.")
                .small()
                .weak(),
        );
        ui.checkbox(&mut s.record_system, "Record the computer's sound (what the others say on a call)");

        section(ui, "Calls");
        ui.horizontal(|ui| {
            ui.label("Keep calls for");
            ui.add(egui::DragValue::new(&mut s.keep_calls_days).range(0..=MAX_KEEP_DAYS).suffix(" days"));
        });
        ui.label(RichText::new("Their sound and transcript. 0 keeps them only as long as everything else; a call you keep (📌) stays until you delete it.").small().weak());
        ui.checkbox(&mut s.transcribe_calls, "Write down what's said during calls, as it's said");
        ui.checkbox(&mut s.separate_voices, "Tell the other people's voices apart (Speaker 1, Speaker 2, …)");
        ui.checkbox(&mut s.names_from_app, "Name them from the meeting app (who it shows speaking, its captions)");
        ui.label(
            RichText::new(
                "During a call, RecallCraft reads the meeting app's window as a screen reader would, about twice a second, for who's on                  the call and who's speaking, and names the voices after them. Turning on the app's live captions helps.",
            )
            .small()
            .weak(),
        );
        ui.checkbox(&mut s.browsers_are_calls, "A browser using the microphone is a call (Meet, Teams or Zoom in a browser)");
        ui.checkbox(&mut s.any_app_is_a_call, "Any program using the microphone is a call");
        ui.checkbox(&mut s.tray_dot, "A red dot on the tray icon while a call is recorded");
        let more = st.more.get_or_insert_with(|| list_text(&s.more_call_apps));
        ui.horizontal(|ui| {
            ui.label("Also calling apps");
            if ui.add(egui::TextEdit::singleline(more).hint_text("myphone.exe, other.exe").desired_width(300.0)).changed() {
                s.more_call_apps = list_of(more);
            }
        });
        let never = st.never.get_or_insert_with(|| list_text(&s.never_call_apps));
        ui.horizontal(|ui| {
            ui.label("Never a call");
            if ui.add(egui::TextEdit::singleline(never).hint_text("chrome.exe").desired_width(300.0)).changed() {
                s.never_call_apps = list_of(never);
            }
        });
        ui.label(RichText::new("Teams, Zoom, Slack, Webex, Discord, WhatsApp, Signal, Phone Link and other calling apps are known already.").small().weak());

        section(ui, "Replay");
        ui.horizontal(|ui| {
            ui.label("Replay goes back");
            ui.add(egui::DragValue::new(&mut s.replay_seconds).range(5..=600).suffix(" seconds"));
            ui.label("at");
            egui::ComboBox::from_id_salt("replay-speed").width(70.0).selected_text(format!("{}×", s.replay_speed)).show_ui(ui, |ui| {
                for x in SPEEDS {
                    ui.selectable_value(&mut s.replay_speed, x, format!("{x}×"));
                }
            });
        });
        for (label, key) in [("Shortcut for Replay", &mut s.replay_hotkey), ("Shortcut to open RecallCraft", &mut s.open_hotkey)] {
            ui.horizontal(|ui| {
                ui.label(label);
                ui.add(egui::TextEdit::singleline(key).hint_text("none").desired_width(160.0));
                if !key.trim().is_empty()
                    && let Err(e) = crate::hotkeys::parse(key)
                {
                    ui.colored_label(super::style::AMBER, e);
                }
            });
        }
        ui.label(RichText::new("Like Ctrl+Alt+R. Pressed again while it plays, Replay stops.").small().weak());

        section(ui, "Export");
        ui.horizontal(|ui| {
            ui.label("Sound");
            egui::ComboBox::from_id_salt("export-audio").width(280.0).selected_text(s.export.audio.label()).show_ui(ui, |ui| {
                for a in AudioExport::ALL {
                    ui.selectable_value(&mut s.export.audio, a, a.label());
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Transcript");
            for f in TranscriptFormat::ALL {
                let mut on = s.export.transcripts.contains(&f);
                if ui.checkbox(&mut on, f.label()).changed() {
                    if on {
                        s.export.transcripts.push(f);
                    } else {
                        s.export.transcripts.retain(|x| *x != f);
                    }
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Folder");
            ui.add(egui::TextEdit::singleline(&mut s.export.folder).hint_text(paths::exports_dir(&Settings::default()).display().to_string()).desired_width(330.0));
            if ui.button("Browse…").clicked()
                && let Some(f) = rfd::FileDialog::new().pick_folder()
            {
                s.export.folder = f.display().to_string();
            }
        });
        ui.horizontal(|ui| {
            ui.label("File name");
            ui.add(egui::TextEdit::singleline(&mut s.export.name).desired_width(260.0)).on_hover_text("{date}, {time}, {title} and {app} are filled in");
        });

        section(ui, "Storage");
        ui.horizontal(|ui| {
            ui.label("Kept in");
            ui.add(egui::TextEdit::singleline(&mut s.data_folder).hint_text(paths::default_data_dir().display().to_string()).desired_width(330.0));
            if ui.button("Browse…").clicked()
                && let Some(f) = rfd::FileDialog::new().pick_folder()
            {
                s.data_folder = f.display().to_string();
            }
        });
        ui.label(RichText::new("A new folder is used once RecallCraft starts again; what's in the old one stays there.").small().weak());
        let size = st.size.get_or_insert_with(|| {
            let slot = Arc::new(Mutex::new(None));
            let (s2, root) = (slot.clone(), layout.root.clone());
            std::thread::spawn(move || {
                let n = recallcraft_store::folder_size(&root, 200_000);
                *s2.lock().unwrap_or_else(PoisonError::into_inner) = Some(n);
            });
            slot
        });
        let used = size.lock().unwrap_or_else(PoisonError::into_inner).map_or("…".to_string(), recallcraft_store::size_words);
        ui.horizontal(|ui| {
            ui.label(format!("Using {used} in {}", layout.root.display()));
            if ui.button("Open the folder").clicked() {
                let _ = opener::open(&layout.root);
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Delete everything recorded…").clicked() {
                st.confirm = Some(Confirm::DeleteAll);
            }
            if ui.button("Forget the voices I named…").clicked() {
                st.confirm = Some(Confirm::ForgetVoices);
            }
        });

        section(ui, "Starting");
        let status = st.signin.get_or_insert_with(signin::status);
        let mut on = status.enabled;
        if ui.checkbox(&mut on, "Start RecallCraft when I sign in").changed() {
            let r = if on { signin::enable() } else { signin::disable() };
            match r {
                Ok(m) => out.push(Action::Note(m)),
                Err(e) => out.push(Action::Note(e)),
            }
            st.signin = Some(signin::status());
        }
        ui.label(RichText::new("It keeps the last hours only while it runs.").small().weak());
        ui.label(
            RichText::new("Closing this window keeps RecallCraft in the tray, recording; it stops only when you quit it (here, or in the tray's menu).")
                .small()
                .weak(),
        );
        if ui.button("Quit RecallCraft").on_hover_text("Stops recording and closes RecallCraft until you start it again").clicked() {
            out.push(Action::Quit);
        }

        updates(ui, s, upd, in_call, &mut out);

        section(ui, "About");
        ui.label(version::line("RecallCraft"));
        ui.label(match models::speech_model() {
            Ok(p) => format!("Speech model: {}", p.display()),
            Err(e) => format!("Speech model: {e}"),
        });
        ui.label(
            RichText::new(
                "What's recorded stays on this computer, and the speech is written down here too. The only thing RecallCraft asks \
                 the network for is whether there's a newer version (Updates, above). In many places everyone on a call has to \
                 agree to it being recorded; that's up to you.",
            )
            .small()
            .weak(),
        );
        if let Some(logs) = paths::log_dir()
            && ui.link("The log folder").clicked()
        {
            let _ = opener::open(&logs);
        }
        ui.add_space(20.0);
    });
    if let Some(c) = st.confirm {
        let mut close = false;
        egui::Modal::new(egui::Id::new("settings-confirm")).show(ui.ctx(), |ui| {
            ui.set_width(400.0);
            match c {
                Confirm::DeleteAll => {
                    ui.heading("Delete everything recorded?");
                    ui.label("The whole recording and every call, kept or not, are deleted. This can't be undone.");
                }
                Confirm::ForgetVoices => {
                    ui.heading("Forget the voices you named?");
                    ui.label("Names already given on calls stay; new calls won't know the voices again.");
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(if c == Confirm::DeleteAll { "Delete everything" } else { "Forget them" }).clicked() {
                    match c {
                        Confirm::DeleteAll => match recallcraft_store::retention::delete_everything(&layout.ring(), &layout.calls()) {
                            Ok(()) => out.push(Action::Note("Everything recorded is deleted".to_string())),
                            Err(e) => out.push(Action::Note(format!("Not everything could be deleted: {e}"))),
                        },
                        Confirm::ForgetVoices => {
                            if let Some(d) = paths::settings_dir() {
                                recallcraft_engine::known::forget_all(&d);
                            }
                            out.push(Action::Note("The voices are forgotten".to_string()));
                        }
                    }
                    st.size = None;
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if close {
            st.confirm = None;
        }
    }
    (*s != before, out)
}
