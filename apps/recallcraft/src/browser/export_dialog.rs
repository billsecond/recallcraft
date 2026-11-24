//! Exporting a call (or an hour): the sound and transcript formats, the folder and the file name,
//! remembered for next time. The export runs on a thread of its own, with a bar and Cancel.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use recallcraft_desktop::paths;
use recallcraft_model::{AudioExport, Call, ExportSettings, Line, Settings, TranscriptFormat};
use recallcraft_store::Layout;
use recallcraft_store::export::{self, Export};
use recallcraft_store::ring::RingReader;

type Done = Arc<Mutex<Option<Result<Vec<PathBuf>, String>>>>;

struct Running {
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    done: Done,
}

pub struct Dialog {
    call: Call,
    lines: Vec<Line>,
    from_ms: u64,
    to_ms: u64,
    settings: ExportSettings,
    folder: String,
    running: Option<Running>,
    result: Option<Result<Vec<PathBuf>, String>>,
}

pub enum Outcome {
    Open,
    /// Closed; the settings to remember, if it exported.
    Closed(Option<ExportSettings>),
}

impl Dialog {
    pub fn new(call: Call, lines: Vec<Line>, from_ms: u64, to_ms: u64, settings: &Settings) -> Dialog {
        let mut s = settings.export.clone();
        if lines.is_empty() && call.id.is_empty() {
            // An hour of the recording has no transcript.
            s.transcripts.clear();
            if s.audio == AudioExport::None {
                s.audio = AudioExport::Mixed;
            }
        }
        Dialog { folder: paths::exports_dir(settings).display().to_string(), call, lines, from_ms, to_ms, settings: s, running: None, result: None }
    }

    fn start(&mut self, layout: &Layout, your_name: &str) {
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let done: Done = Arc::new(Mutex::new(None));
        let (p, c, d) = (progress.clone(), cancel.clone(), done.clone());
        let (call, lines, settings, folder, ring, your) =
            (self.call.clone(), self.lines.clone(), self.settings.clone(), PathBuf::from(self.folder.trim()), layout.ring(), your_name.to_string());
        let (from_us, to_us) = (self.from_ms * 1000, self.to_ms * 1000);
        let spawned = std::thread::Builder::new().name("export".into()).spawn(move || {
            let mut reader = RingReader::new(&ring);
            let job = Export { call: &call, lines: &lines, your_name: &your, settings: &settings, folder, from_us, to_us };
            let r = export::run(&mut reader, &job, &mut |f| p.store((f * 1000.0) as u32, Ordering::Relaxed), &c).map_err(|e| e.to_string());
            *d.lock().unwrap_or_else(PoisonError::into_inner) = Some(r);
        });
        match spawned {
            Ok(_) => self.running = Some(Running { progress, cancel, done }),
            Err(e) => self.result = Some(Err(e.to_string())),
        }
    }
}

pub fn show(ctx: &egui::Context, d: &mut Dialog, layout: &Layout, your_name: &str) -> Outcome {
    let mut outcome = Outcome::Open;
    let finished = d.running.as_ref().and_then(|r| r.done.lock().unwrap_or_else(PoisonError::into_inner).take());
    if let Some(result) = finished {
        d.result = Some(result);
        d.running = None;
    }
    egui::Modal::new(egui::Id::new("export")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.heading(format!("Export “{}”", d.call.display_title()));
        ui.add_space(6.0);
        if let Some(result) = &d.result {
            match result {
                Ok(files) => {
                    ui.label(format!("Exported {} file{}:", files.len(), if files.len() == 1 { "" } else { "s" }));
                    for f in files {
                        ui.label(egui::RichText::new(f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).monospace());
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Show in folder").clicked()
                            && let Some(f) = files.first()
                        {
                            let _ = opener::reveal(f);
                        }
                        if ui.button("Done").clicked() {
                            outcome = Outcome::Closed(Some(d.settings.clone()));
                        }
                    });
                }
                Err(e) => {
                    ui.colored_label(super::style::RED, format!("The export failed: {e}"));
                    if ui.button("Close").clicked() {
                        outcome = Outcome::Closed(None);
                    }
                }
            }
            return;
        }
        if let Some(r) = &d.running {
            let f = r.progress.load(Ordering::Relaxed) as f32 / 1000.0;
            ui.add(egui::ProgressBar::new(f).show_percentage());
            if ui.button("Cancel").clicked() {
                r.cancel.store(true, Ordering::Relaxed);
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        ui.label(egui::RichText::new("Sound").strong());
        for a in AudioExport::ALL {
            ui.radio_value(&mut d.settings.audio, a, a.label());
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Transcript").strong());
        if d.lines.is_empty() {
            ui.label(egui::RichText::new("Nothing was written down for this.").weak());
        }
        ui.horizontal_wrapped(|ui| {
            for f in TranscriptFormat::ALL {
                let mut on = d.settings.transcripts.contains(&f);
                if ui.checkbox(&mut on, f.label()).changed() {
                    if on {
                        d.settings.transcripts.push(f);
                    } else {
                        d.settings.transcripts.retain(|x| *x != f);
                    }
                }
            }
        });
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Where").strong());
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut d.folder).desired_width(330.0));
            if ui.button("Browse…").clicked()
                && let Some(f) = rfd::FileDialog::new().set_directory(d.folder.trim()).pick_folder()
            {
                d.folder = f.display().to_string();
            }
        });
        ui.horizontal(|ui| {
            ui.label("File name");
            ui.add(egui::TextEdit::singleline(&mut d.settings.name).desired_width(260.0))
                .on_hover_text("{date}, {time}, {title} and {app} are filled in");
        });
        ui.checkbox(&mut d.settings.own_folder, "Put the files in a folder of their own");
        ui.label(egui::RichText::new(format!("Named like: {}", export::file_stem(&d.settings.name, &d.call))).small().weak());
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let nothing = d.settings.audio == AudioExport::None && d.settings.transcripts.is_empty();
            if ui.add_enabled(!nothing && !d.folder.trim().is_empty(), egui::Button::new("Export")).clicked() {
                d.settings.folder = if std::path::Path::new(d.folder.trim()) == paths::exports_dir(&Settings::default()) {
                    String::new()
                } else {
                    d.folder.trim().to_string()
                };
                d.start(layout, your_name);
            }
            if ui.button("Cancel").clicked() {
                outcome = Outcome::Closed(None);
            }
        });
    });
    outcome
}
