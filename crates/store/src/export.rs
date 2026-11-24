//! A call (or any stretch of the recording) exported: its sound as WAV or Opus, and its
//! transcript in the formats chosen, into a folder. Sound is read and written a few seconds at
//! a time, so a long call needs little memory. Nothing is ever written over: a name that's
//! taken gets ` (2)`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use recallcraft_model::{AudioExport, Call, ExportSettings, Line, Stream, time};

use crate::StoreError;
use crate::ogg::OggOpus;
use crate::opus::{Encoder, FRAME, RATE};
use crate::ring::{RingReader, samples_to_us};
use crate::transcript;
use crate::wav::WavWriter;

/// Sound read at a time: five seconds.
const CHUNK: usize = RATE as usize * 5;

/// What to export.
pub struct Export<'a> {
    pub call: &'a Call,
    pub lines: &'a [Line],
    pub your_name: &'a str,
    pub settings: &'a ExportSettings,
    /// The folder the files go in.
    pub folder: PathBuf,
    /// The stretch of sound, in µs since 1970.
    pub from_us: u64,
    pub to_us: u64,
}

/// A file name made safe on every system: no `<>:"/\|?*`, no control characters, not too
/// long, never empty.
pub fn safe_name(text: &str) -> String {
    let cleaned: String = text.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { ' ' } else { c }).collect();
    let words: String = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed: String = words.chars().take(120).collect::<String>().trim_matches(|c: char| c == '.' || c == ' ').to_string();
    if trimmed.is_empty() { "RecallCraft".to_string() } else { trimmed }
}

/// The file name of an export (no extension), from the settings' pattern.
pub fn file_stem(pattern: &str, call: &Call) -> String {
    let date = time::local_date(call.started_ms);
    let clock = time::local_time(call.started_ms).replace(':', ".");
    let text = pattern
        .replace("{date}", &date)
        .replace("{time}", &clock)
        .replace("{title}", &call.display_title())
        .replace("{app}", if call.app.is_empty() { "Recording" } else { &call.app });
    safe_name(&text)
}

/// `folder/stem.ext`, or `folder/stem (2).ext` and on if it's taken.
fn free_path(folder: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = folder.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    for n in 2..1000 {
        let p = folder.join(format!("{stem} ({n}).{ext}"));
        if !p.exists() {
            return p;
        }
    }
    folder.join(format!("{stem} ({}).{ext}", recallcraft_model::time::now_ms()))
}

/// Two sounds made one, never past full scale (a gentle knee near the top).
pub fn mix(a: f32, b: f32) -> f32 {
    let x = a + b;
    let m = x.abs();
    if m <= 0.9 { x } else { x.signum() * (0.9 + 0.1 * ((m - 0.9) / 0.1).tanh()) }
}

/// Exports; returns the files written. `progress` hears 0 to 1.
pub fn run(reader: &mut RingReader, job: &Export<'_>, progress: &mut dyn FnMut(f32), cancel: &AtomicBool) -> Result<Vec<PathBuf>, StoreError> {
    let mut folder = job.folder.clone();
    let stem = file_stem(&job.settings.name, job.call);
    if job.settings.own_folder {
        folder = free_path(&folder, &stem, "").with_extension("");
    }
    std::fs::create_dir_all(&folder)?;
    let mut written = Vec::new();
    for f in &job.settings.transcripts {
        let path = free_path(&folder, &stem, f.extension());
        std::fs::write(&path, transcript::render(*f, job.call, job.lines, job.your_name))?;
        written.push(path);
    }
    let total = job.to_us.saturating_sub(job.from_us);
    if job.settings.audio == AudioExport::None || total == 0 {
        progress(1.0);
        return Ok(written);
    }
    #[allow(clippy::large_enum_variant)]
    enum Out {
        Mixed(WavWriter),
        Stereo(WavWriter),
        Separate(WavWriter, WavWriter),
        Opus(OggOpus<std::io::BufWriter<std::fs::File>>, Encoder, Vec<f32>),
    }
    let mut paths = Vec::new();
    let mut out = match job.settings.audio {
        AudioExport::Mixed | AudioExport::None => {
            let p = free_path(&folder, &stem, "wav");
            let w = WavWriter::create(&p, 1, RATE)?;
            paths.push(p);
            Out::Mixed(w)
        }
        AudioExport::Stereo => {
            let p = free_path(&folder, &stem, "wav");
            let w = WavWriter::create(&p, 2, RATE)?;
            paths.push(p);
            Out::Stereo(w)
        }
        AudioExport::Separate => {
            let (you, them) = (free_path(&folder, &format!("{stem} - you"), "wav"), free_path(&folder, &format!("{stem} - them"), "wav"));
            let w = (WavWriter::create(&you, 1, RATE)?, WavWriter::create(&them, 1, RATE)?);
            paths.push(you);
            paths.push(them);
            Out::Separate(w.0, w.1)
        }
        AudioExport::Opus => {
            let p = free_path(&folder, &stem, "opus");
            let file = std::io::BufWriter::new(std::fs::File::create(&p)?);
            let serial = (job.call.started_ms as u32) ^ 0x5243_4352;
            paths.push(p);
            Out::Opus(OggOpus::new(file, serial)?, Encoder::new(48_000, false)?, Vec::new())
        }
    };
    let mut at = job.from_us;
    while at < job.to_us {
        if cancel.load(Ordering::Relaxed) {
            for p in &paths {
                let _ = std::fs::remove_file(p);
            }
            return Err(StoreError::Other("cancelled".into()));
        }
        let left = crate::ring::us_to_samples(job.to_us - at, RATE) as usize;
        let n = left.clamp(1, CHUNK);
        let mic = reader.read(Stream::Mic, at, n, RATE);
        let sys = reader.read(Stream::System, at, n, RATE);
        match &mut out {
            Out::Mixed(w) => {
                let m: Vec<f32> = mic.iter().zip(&sys).map(|(a, b)| mix(*a, *b)).collect();
                w.write(&m)?;
            }
            Out::Stereo(w) => {
                let s: Vec<f32> = mic.iter().zip(&sys).flat_map(|(a, b)| [*a, *b]).collect();
                w.write(&s)?;
            }
            Out::Separate(a, b) => {
                a.write(&mic)?;
                b.write(&sys)?;
            }
            Out::Opus(ogg, enc, pending) => {
                pending.extend(mic.iter().zip(&sys).map(|(a, b)| mix(*a, *b)));
                let whole = pending.len() / FRAME * FRAME;
                for frame in pending.get(..whole).unwrap_or_default().as_chunks::<FRAME>().0 {
                    ogg.packet(enc.encode(frame)?)?;
                }
                pending.drain(..whole);
            }
        }
        at += samples_to_us(n as u64, RATE).max(1);
        progress((at.saturating_sub(job.from_us)) as f32 / total as f32);
    }
    match out {
        Out::Mixed(w) | Out::Stereo(w) => w.finish()?,
        Out::Separate(a, b) => {
            a.finish()?;
            b.finish()?;
        }
        Out::Opus(mut ogg, mut enc, mut pending) => {
            // The last frame filled out, and the encoder's warm-up after it.
            pending.resize(pending.len().div_ceil(FRAME) * FRAME + FRAME, 0.0);
            for frame in pending.as_chunks::<FRAME>().0 {
                ogg.packet(enc.encode(frame)?)?;
            }
            ogg.finish()?;
        }
    }
    written.extend(paths);
    progress(1.0);
    log::info!("exported {} files", written.len());
    Ok(written)
}

#[cfg(test)]
mod tests {
    use recallcraft_model::{Speaker, TranscriptFormat};

    use super::*;
    use crate::ring::RingWriter;

    #[test]
    fn names_are_safe_and_patterns_fill_in() {
        assert_eq!(safe_name(" a/b:c*d?.  "), "a b c d");
        assert_eq!(safe_name("..."), "RecallCraft");
        assert_eq!(safe_name(&"x".repeat(300)).len(), 120);
        let call = Call { title: "Weekly: sync".into(), app: "Zoom".into(), started_ms: 1_764_000_000_000, ..Call::default() };
        let stem = file_stem("{date} {time} {title} ({app})", &call);
        assert!(stem.ends_with("Weekly sync (Zoom)"), "{stem}");
        assert!(!stem.contains(':'));
        assert!((mix(0.95, 0.95) - 1.0).abs() < 0.01 && mix(0.95, 0.95) <= 1.0);
        assert_eq!(mix(0.2, 0.3), 0.5);
    }

    #[test]
    fn a_call_exports_in_every_way() {
        let root = crate::temp_dir("export");
        let ring = root.join("ring");
        let t0 = 1_764_000_000_000_000u64;
        let mut w = RingWriter::new(&ring);
        for k in 0..100u64 {
            let mic: Vec<f32> = (0..4800).map(|i| ((k as usize * 4800 + i) as f32 * 0.03).sin() * 0.3).collect();
            w.push(Stream::Mic, t0 + k * 100_000, &mic).unwrap();
            if k >= 50 {
                let sys: Vec<f32> = (0..4800).map(|i| ((k as usize * 4800 + i) as f32 * 0.07).sin() * 0.3).collect();
                w.push(Stream::System, t0 + k * 100_000, &sys).unwrap();
            }
        }
        w.close();
        let call = Call {
            id: "c".into(),
            title: "Test".into(),
            app: "Zoom".into(),
            started_ms: t0 / 1000,
            ended_ms: Some(t0 / 1000 + 10_000),
            ..Call::default()
        };
        let lines = vec![Line { start_ms: t0 / 1000 + 1000, end_ms: t0 / 1000 + 2000, speaker: Speaker::Me, text: "Hello".into() }];
        let out = root.join("out");
        let cancel = AtomicBool::new(false);
        for audio in AudioExport::ALL {
            let settings = ExportSettings {
                audio,
                transcripts: vec![TranscriptFormat::Text, TranscriptFormat::Srt],
                name: "{title}".into(),
                ..ExportSettings::default()
            };
            let job =
                Export { call: &call, lines: &lines, your_name: "", settings: &settings, folder: out.clone(), from_us: t0, to_us: t0 + 10_000_000 };
            let mut r = RingReader::new(&ring);
            let mut last = 0.0;
            let files = run(&mut r, &job, &mut |p| last = p, &cancel).unwrap();
            assert_eq!(last, 1.0);
            let sound = match audio {
                AudioExport::None => 0,
                AudioExport::Separate => 2,
                _ => 1,
            };
            assert_eq!(files.len(), 2 + sound, "{audio:?}: {files:?}");
            for f in &files {
                assert!(f.exists() && std::fs::metadata(f).unwrap().len() > 10, "{f:?}");
            }
            if audio == AudioExport::Stereo {
                let wav = files.iter().find(|f| f.extension().unwrap() == "wav").unwrap();
                // Ten seconds, two channels, two bytes, and the header.
                assert_eq!(std::fs::metadata(wav).unwrap().len(), 44 + 48_000 * 10 * 4);
            }
            if audio == AudioExport::Opus {
                let opus = files.iter().find(|f| f.extension().unwrap() == "opus").unwrap();
                let size = std::fs::metadata(opus).unwrap().len();
                assert!(size < 48_000 * 10 / 4, "small: {size}");
            }
        }
        // Names already taken get a number.
        assert!(out.join("Test (2).txt").exists());
        // Cancelled: the sound file goes.
        cancel.store(true, Ordering::Relaxed);
        let settings = ExportSettings { audio: AudioExport::Mixed, transcripts: vec![], name: "cancelled".into(), ..ExportSettings::default() };
        let job = Export { call: &call, lines: &lines, your_name: "", settings: &settings, folder: out.clone(), from_us: t0, to_us: t0 + 10_000_000 };
        assert!(run(&mut RingReader::new(&ring), &job, &mut |_| {}, &cancel).is_err());
        assert!(!out.join("cancelled.wav").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
