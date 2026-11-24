//! A call played through the recorder and the hearer, from WAV files, with the real speech model:
//! what RecallCraft would write down, and how fast.
//!
//! ```sh
//! cargo run --release -p recallcraft-engine --example simulate -- <script> <data folder> [--echo 0.08] [--model <file>]
//! ```
//!
//! Each line of the script is `<mic|system> <pause before, s> <file.wav>`: the utterances one
//! after another, yours on the microphone, the others' on the computer's sound. `--echo` adds the
//! computer's sound to the microphone, that much quieter and a little late (speakers, not
//! headphones).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::channel;
use std::time::Instant;

use recallcraft_audio::Block;
use recallcraft_engine::recorder::{self, ToHear};
use recallcraft_engine::worker::{Hear, HearWith};
use recallcraft_engine::{Shared, hearer};
use recallcraft_model::{Call, Settings, Stream, time};
use recallcraft_store::Layout;
use recallcraft_store::calls::CallStore;
use recallcraft_store::wav;

struct Whisper(recallcraft_speech::Whisper, AtomicBool);

impl Hear for Whisper {
    fn hear(&mut self, sound: &[f32]) -> Result<Vec<recallcraft_speech::Line>, String> {
        recallcraft_speech::hear(&mut self.0, sound, &self.1).map_err(|e| e.to_string())
    }
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(script), Some(out)) = (args.first(), args.get(1)) else {
        return Err("usage: simulate <script> <data folder> [--echo <gain>] [--model <file>]".into());
    };
    let opt = |n: &str| args.iter().position(|a| a == n).and_then(|i| args.get(i + 1)).cloned();
    let echo: f32 = opt("--echo").and_then(|e| e.parse().ok()).unwrap_or(0.0);
    let model = opt("--model").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("assets/models/whisper-base.en-q8_0.gguf"));
    let base = Path::new(script).parent().unwrap_or(Path::new("."));
    // The two tracks, at 48 kHz.
    let (mut mic, mut sys): (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
    let mut at = 0usize;
    let mut said: Vec<(String, f32, f32)> = Vec::new();
    for line in std::fs::read_to_string(script).map_err(|e| e.to_string())?.lines().filter(|l| !l.trim().is_empty()) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let [stream, pause, file] = parts.as_slice() else { return Err(format!("a script line: {line}")) };
        let pause: f32 = pause.parse().map_err(|_| format!("a pause: {pause}"))?;
        at += (pause * 48_000.0) as usize;
        let w = wav::read(&base.join(file))?;
        let s = wav::mono_at(&w, 48_000);
        let track = if *stream == "mic" { &mut mic } else { &mut sys };
        if track.len() < at + s.len() {
            track.resize(at + s.len(), 0.0);
        }
        for (i, v) in s.iter().enumerate() {
            track[at + i] += v * 0.7;
        }
        said.push((stream.to_string(), at as f32 / 48_000.0, (at + s.len()) as f32 / 48_000.0));
        at += s.len();
    }
    let total = at + 48_000 * 2;
    mic.resize(total, 0.0);
    sys.resize(total, 0.0);
    // A quiet room in the microphone, and the speakers heard by it.
    let mut x = 12345u32;
    for (i, m) in mic.iter_mut().enumerate() {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *m += (x as f32 / u32::MAX as f32 - 0.5) * 0.002;
        if echo > 0.0 && i >= 2400 {
            *m += sys[i - 2400] * echo;
        }
    }
    let secs = total as f32 / 48_000.0;
    println!("{secs:.1} s of call: {} utterances", said.len());

    let bytes = std::fs::read(&model).map_err(|e| format!("{}: {e}", model.display()))?;
    let whisper = recallcraft_speech::load(&bytes).map_err(|e| e.to_string())?;
    let layout = Layout::new(out);
    let _ = std::fs::remove_dir_all(&layout.root);
    let calls = CallStore::new(&layout.calls());
    let t0 = time::now_ms() - (secs * 1000.0) as u64 - 5000;
    calls
        .save(&Call { id: "simulated".into(), app: "Simulated".into(), title: "A simulated call".into(), started_ms: t0, ..Call::default() })
        .map_err(|e| e.to_string())?;
    let shared = Arc::new(Shared::new(Settings::default()));
    let (block_tx, block_rx) = channel::<Block>();
    let (hear_tx, hear_rx) = channel::<ToHear>();
    let _ = hear_tx.send(ToHear::CallStarted { id: "simulated".into(), started_ms: t0 });
    let started = Instant::now();
    let (s1, ring, h1) = (shared.clone(), layout.ring(), hear_tx.clone());
    let rec = std::thread::spawn(move || recorder::run(&s1, &ring, block_rx, h1));
    let (s2, l2) = (shared.clone(), layout.clone());
    let settings_dir = layout.root.join("settings");
    let hearing =
        std::thread::spawn(move || hearer::run(&s2, &l2, &settings_dir, HearWith::Here(Box::new(Whisper(whisper, AtomicBool::new(false)))), hear_rx));
    for k in 0..total / 480 {
        let us = t0 * 1000 + k as u64 * 10_000;
        let r = k * 480..(k + 1) * 480;
        let _ = block_tx.send(Block { stream: Stream::Mic, start_us: us, samples: mic[r.clone()].to_vec() });
        let _ = block_tx.send(Block { stream: Stream::System, start_us: us, samples: sys[r].to_vec() });
    }
    drop(block_tx);
    let _ = rec.join();
    let _ = hear_tx.send(ToHear::CallEnded { id: "simulated".into(), ended_ms: t0 + (secs * 1000.0) as u64 });
    drop(hear_tx);
    let _ = hearing.join();
    let took = started.elapsed().as_secs_f32();
    let lines = calls.lines("simulated");
    let call = calls.load("simulated").unwrap_or_default();
    for l in &lines {
        println!(
            "[{:>5.1} – {:>5.1}] {:<10} {}",
            (l.start_ms - t0) as f32 / 1000.0,
            (l.end_ms - t0) as f32 / 1000.0,
            call.speaker_name(l.speaker, "Me"),
            l.text
        );
    }
    println!("{} lines; {secs:.1} s of call heard in {took:.1} s ({:.1}× faster than it was said)", lines.len(), secs / took.max(0.001));
    Ok(())
}
