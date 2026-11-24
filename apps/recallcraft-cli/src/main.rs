//! RecallCraft for scripts and agents.
//!
//! ```text
//! recallcraft-cli calls [--json]                     the calls, latest first
//! recallcraft-cli transcript <call|latest> [--format text|md|srt|vtt|json]
//! recallcraft-cli export <call|latest> [--audio mixed|stereo|separate|opus|none]
//!                        [--transcript text,md,srt,vtt,json] [--out <folder>]
//! recallcraft-cli search <words…> [--json]           calls where they were said
//! recallcraft-cli status                             what the service is doing
//! recallcraft-cli hear <file.wav> [--model <file>]   write down what's said in a WAV file
//! ```
//!
//! It reads the folders the settings name; it never records anything itself.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use recallcraft_desktop::{models, paths, settings as prefs, version};
use recallcraft_model::{AudioExport, Call, TranscriptFormat, time};
use recallcraft_store::calls::CallStore;
use recallcraft_store::export::{self, Export};
use recallcraft_store::ring::RingReader;
use recallcraft_store::{Layout, transcript, wav};

const USAGE: &str = "usage: recallcraft-cli calls [--json]
       recallcraft-cli transcript <call|latest> [--format text|md|srt|vtt|json]
       recallcraft-cli export <call|latest> [--audio mixed|stereo|separate|opus|none] [--transcript text,md,…] [--out <folder>]
       recallcraft-cli search <words…> [--json]
       recallcraft-cli status
       recallcraft-cli hear <file.wav> [--model <file.gguf>]
       recallcraft-cli --version";

fn option(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn find(calls: &CallStore, which: &str) -> Result<Call, String> {
    if which == "latest" {
        return calls.list().into_iter().next().ok_or_else(|| "there are no calls".to_string());
    }
    calls.load(which).ok_or_else(|| format!("no call {which} (recallcraft-cli calls lists them)"))
}

fn run(args: &[String]) -> Result<(), String> {
    let settings = prefs::load();
    let layout = Layout::new(paths::data_dir(&settings));
    let calls = CallStore::new(&layout.calls());
    let json = args.iter().any(|a| a == "--json");
    match args.first().map(String::as_str) {
        Some("calls") => {
            let list = calls.list();
            if json {
                println!("{}", serde_json::to_string_pretty(&list).map_err(|e| e.to_string())?);
                return Ok(());
            }
            let now = time::now_ms();
            for c in list {
                let len = if c.is_live() { "going on".to_string() } else { time::duration_words(c.duration_ms(now)) };
                println!(
                    "{}  {} {}  {:<10}  {}  ({})",
                    c.id,
                    time::local_date(c.started_ms),
                    time::local_time(c.started_ms),
                    len,
                    c.display_title(),
                    c.app
                );
            }
            Ok(())
        }
        Some("transcript") => {
            let call = find(&calls, args.get(1).ok_or(USAGE)?)?;
            let format = option(args, "--format")
                .map_or(Some(TranscriptFormat::Text), |f| TranscriptFormat::from_name(&f))
                .ok_or("--format is text, md, srt, vtt or json")?;
            print!("{}", transcript::render(format, &call, &calls.lines(&call.id), &settings.your_name));
            Ok(())
        }
        Some("export") => {
            let call = find(&calls, args.get(1).ok_or(USAGE)?)?;
            let mut s = settings.export.clone();
            if let Some(a) = option(args, "--audio") {
                s.audio = match a.as_str() {
                    "mixed" | "wav" => AudioExport::Mixed,
                    "stereo" => AudioExport::Stereo,
                    "separate" => AudioExport::Separate,
                    "opus" => AudioExport::Opus,
                    "none" => AudioExport::None,
                    _ => return Err("--audio is mixed, stereo, separate, opus or none".into()),
                };
            }
            if let Some(t) = option(args, "--transcript") {
                s.transcripts = t
                    .split(',')
                    .filter(|x| !x.trim().is_empty() && x.trim() != "none")
                    .map(|x| TranscriptFormat::from_name(x).ok_or(format!("{x}: not a transcript format")))
                    .collect::<Result<_, _>>()?;
            }
            let folder = option(args, "--out").map_or_else(|| paths::exports_dir(&settings), PathBuf::from);
            let lines = calls.lines(&call.id);
            let now = time::now_ms();
            let job = Export {
                call: &call,
                lines: &lines,
                your_name: &settings.your_name,
                settings: &s,
                folder,
                from_us: call.started_ms * 1000,
                to_us: call.end_or(now) * 1000,
            };
            let files = export::run(&mut RingReader::new(&layout.ring()), &job, &mut |_| {}, &AtomicBool::new(false)).map_err(|e| e.to_string())?;
            for f in files {
                println!("{}", f.display());
            }
            Ok(())
        }
        Some("search") => {
            let words: Vec<&str> = args.iter().skip(1).filter(|a| !a.starts_with("--")).map(String::as_str).collect();
            if words.is_empty() {
                return Err(USAGE.into());
            }
            let found = calls.search(&words.join(" "));
            if json {
                let v: Vec<serde_json::Value> = found.iter().map(|(c, l)| serde_json::json!({ "call": c, "lines": l })).collect();
                println!("{}", serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?);
                return Ok(());
            }
            for (c, hits) in found {
                println!("{}  {}", c.id, c.display_title());
                for h in hits.iter().take(5) {
                    println!(
                        "    [{}] {}: {}",
                        time::clock(h.start_ms.saturating_sub(c.started_ms)),
                        c.speaker_name(h.speaker, &settings.your_name),
                        h.text
                    );
                }
            }
            Ok(())
        }
        Some("status") => {
            let status = std::fs::read_to_string(layout.status()).ok().and_then(|t| serde_json::from_str::<recallcraft_engine::Status>(&t).ok());
            match status.filter(|s| time::now_ms().saturating_sub(s.at_ms) < 8000) {
                None => println!("RecallCraft isn't running"),
                Some(s) => {
                    println!("recording: {}", if s.recording { "yes" } else { "no" });
                    println!("microphone: {}", if s.mic.is_empty() { "-" } else { &s.mic });
                    println!("computer sound: {}", if s.system.is_empty() { "-" } else { &s.system });
                    if s.in_call() {
                        println!("call: {} ({}, since {})", s.call_title, s.call_app, time::local_time(s.call_started_ms));
                    }
                    println!("hearing: {} ({:.0} s waiting)", s.hearer, s.waiting_s);
                    for p in s.problems {
                        println!("problem: {p}");
                    }
                }
            }
            println!("data: {}", layout.root.display());
            Ok(())
        }
        Some("hear") => {
            let file = args.get(1).ok_or(USAGE)?;
            let model = option(args, "--model").map_or_else(models::speech_model, |m| Ok(PathBuf::from(m)))?;
            let w = wav::read(std::path::Path::new(file))?;
            let sound = wav::mono_at(&w, 16_000);
            let bytes = std::fs::read(&model).map_err(|e| format!("{}: {e}", model.display()))?;
            let mut whisper = recallcraft_speech::load(&bytes).map_err(|e| e.to_string())?;
            let started = std::time::Instant::now();
            let cancel = AtomicBool::new(false);
            let mut lines = Vec::new();
            // A window at a time (30 s), as the service does.
            for (k, chunk) in sound.chunks(16_000 * 28).enumerate() {
                for l in recallcraft_speech::hear(&mut whisper, chunk, &cancel).map_err(|e| e.to_string())? {
                    let off = k as f32 * 28.0;
                    lines.push((l.start + off, l.end + off, l.text));
                }
            }
            for (a, b, t) in &lines {
                println!("[{:>6.2} – {:>6.2}] {t}", a, b);
            }
            eprintln!("{:.1} s of sound heard in {:.1} s", sound.len() as f32 / 16_000.0, started.elapsed().as_secs_f32());
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("{}", version::line("RecallCraft"));
        return ExitCode::SUCCESS;
    }
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
