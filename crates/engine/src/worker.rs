//! Hearing in a program of its own: the service starts itself again as the worker
//! (`recallcraft --hear <model>`), at a low priority, with no window. The worker reads the model
//! once and then hears whatever it's sent, until it's told to stop (its input closes). Whatever
//! goes wrong in it (a damaged model, memory), the service goes on, and starts it again.
//!
//! Over its input and output:
//!
//! ```text
//! request   "RCH1"  samples u32  samples f32 × n      16 kHz mono, at most 30 s
//! answer    one line of JSON: {"lines":[{"start":0.4,"end":2.1,"text":"…"}]} or {"error":"…"}
//! ```
//!
//! Its first line is `{"ready":true,"model":"…"}` once the model is read (or `{"error":…}`).

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use recallcraft_speech::Line;
use serde::{Deserialize, Serialize};

const MAGIC: &[u8; 4] = b"RCH1";
/// The most samples in a request: 30 seconds and a little.
pub const MAX_SAMPLES: usize = 16_000 * 31;
/// How long the model may take to read.
const READY_WAIT: Duration = Duration::from_secs(90);
/// How long hearing one window may take before the worker is taken to be stuck.
const ANSWER_WAIT: Duration = Duration::from_secs(180);

/// Hears 16 kHz sound.
pub trait Hear: Send {
    fn hear(&mut self, sound: &[f32]) -> Result<Vec<Line>, String>;
}

/// How the engine hears.
pub enum HearWith {
    /// A program of its own: `exe` with `args` (and the model's path at the end).
    Program { exe: PathBuf, args: Vec<String>, model: PathBuf, threads: usize },
    /// In this process (tests).
    Here(Box<dyn Hear>),
    /// Not at all (no model).
    Off(String),
}

#[derive(Serialize, Deserialize)]
struct WireLine {
    start: f32,
    end: f32,
    text: String,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Answer {
    ready: bool,
    model: String,
    lines: Vec<WireLine>,
    error: Option<String>,
}

pub fn write_request(w: &mut impl Write, sound: &[f32]) -> std::io::Result<()> {
    let n = sound.len().min(MAX_SAMPLES);
    let mut bytes = Vec::with_capacity(8 + n * 4);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(n as u32).to_le_bytes());
    for s in sound.iter().take(n) {
        bytes.extend_from_slice(&(if s.is_finite() { *s } else { 0.0 }).to_le_bytes());
    }
    w.write_all(&bytes)?;
    w.flush()
}

/// The next request; None when the input has closed.
pub fn read_request(r: &mut impl Read) -> std::io::Result<Option<Vec<f32>>> {
    let mut head = [0u8; 8];
    match r.read_exact(&mut head) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    if head.get(..4) != Some(MAGIC.as_slice()) {
        return Err(std::io::Error::other("not a request"));
    }
    let n = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
    if n > MAX_SAMPLES {
        return Err(std::io::Error::other("a request too long"));
    }
    let mut data = vec![0u8; n * 4];
    r.read_exact(&mut data)?;
    Ok(Some(data.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).map(|v| if v.is_finite() { v } else { 0.0 }).collect()))
}

fn answer(w: &mut impl Write, a: &Answer) -> std::io::Result<()> {
    let line = serde_json::to_string(a).unwrap_or_else(|_| "{\"error\":\"answer\"}".to_string());
    w.write_all(line.as_bytes())?;
    w.write_all(b"\n")?;
    w.flush()
}

/// The worker: reads the model at `model`, then answers requests until its input closes.
/// Returns the exit code.
pub fn serve(model: &Path, input: &mut impl Read, output: &mut impl Write) -> i32 {
    let started = Instant::now();
    let bytes = match std::fs::metadata(model) {
        Ok(m) if m.len() as usize > recallcraft_speech::MAX_MODEL_BYTES => Err("the model file is too large".to_string()),
        Ok(_) => std::fs::read(model).map_err(|e| format!("{}: {e}", model.display())),
        Err(e) => Err(format!("{}: {e}", model.display())),
    };
    let mut whisper = match bytes.and_then(|b| recallcraft_speech::load(&b).map_err(|e| e.to_string())) {
        Ok(w) => w,
        Err(e) => {
            log::error!("the speech model: {e}");
            let _ = answer(output, &Answer { error: Some(e), ..Answer::default() });
            return 2;
        }
    };
    log::info!("the speech model {} read in {} ms", whisper.name, started.elapsed().as_millis());
    if answer(output, &Answer { ready: true, model: whisper.name.clone(), ..Answer::default() }).is_err() {
        return 1;
    }
    let cancel = AtomicBool::new(false);
    loop {
        let sound = match read_request(input) {
            Ok(Some(s)) => s,
            Ok(None) => return 0,
            Err(e) => {
                log::warn!("a request: {e}");
                return 1;
            }
        };
        let t = Instant::now();
        let a = match recallcraft_speech::hear(&mut whisper, &sound, &cancel) {
            Ok(lines) => {
                Answer { lines: lines.into_iter().map(|l| WireLine { start: l.start, end: l.end, text: l.text }).collect(), ..Answer::default() }
            }
            Err(e) => Answer { error: Some(e.to_string()), ..Answer::default() },
        };
        log::info!("heard {:.1} s of sound in {} ms: {} lines", sound.len() as f32 / 16_000.0, t.elapsed().as_millis(), a.lines.len());
        if answer(output, &a).is_err() {
            return 1;
        }
    }
}

struct Running {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
}

/// Hears through the worker program, starting it when needed and again when it fails.
pub struct ChildHear {
    exe: PathBuf,
    args: Vec<String>,
    model: PathBuf,
    threads: usize,
    running: Option<Running>,
    /// Why it last failed, and when (it isn't started again straight away).
    pub failed: Option<(String, Instant)>,
    pub model_name: String,
}

impl ChildHear {
    pub fn new(exe: PathBuf, args: Vec<String>, model: PathBuf, threads: usize) -> ChildHear {
        ChildHear { exe, args, model, threads: threads.clamp(1, 8), running: None, failed: None, model_name: String::new() }
    }

    fn spawn(&mut self) -> Result<(), String> {
        let mut c = Command::new(&self.exe);
        c.args(&self.args).arg(&self.model);
        let n = self.threads.to_string();
        c.env("CANDLE_NUM_THREADS", &n).env("RAYON_NUM_THREADS", &n);
        c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // Below normal priority (calls and everything else come first), and no window.
            const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            c.creation_flags(BELOW_NORMAL_PRIORITY_CLASS | CREATE_NO_WINDOW);
        }
        let mut child = c.spawn().map_err(|e| format!("starting the hearer: {e}"))?;
        let stdin = child.stdin.take().ok_or("the hearer has no input")?;
        let stdout = child.stdout.take().ok_or("the hearer has no output")?;
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("hearer output".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let first = rx.recv_timeout(READY_WAIT).map_err(|_| "the hearer didn't start".to_string())?;
        let a: Answer = serde_json::from_str(&first).map_err(|e| format!("the hearer: {e}"))?;
        if let Some(e) = a.error {
            let _ = child.kill();
            return Err(e);
        }
        self.model_name = a.model;
        log::info!("the hearer started ({})", self.model_name);
        self.running = Some(Running { child, stdin, lines: rx });
        Ok(())
    }

    fn stop_child(&mut self) {
        if let Some(mut r) = self.running.take() {
            drop(r.stdin);
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                if matches!(r.child.try_wait(), Ok(Some(_))) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let _ = r.child.kill();
            let _ = r.child.wait();
        }
    }

    /// Hears with the model `model` on `threads` threads from now on; false if that's already it.
    pub fn set_model(&mut self, model: PathBuf, threads: usize) -> bool {
        let threads = threads.clamp(1, 8);
        if model == self.model && threads == self.threads {
            return false;
        }
        self.stop_child();
        self.model = model;
        self.threads = threads;
        self.failed = None;
        self.model_name.clear();
        true
    }

    /// Whether it may be started (not within a minute of failing).
    pub fn may_start(&self) -> bool {
        self.failed.as_ref().is_none_or(|(_, at)| at.elapsed() > Duration::from_secs(60))
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Lets the worker go (no call for a while: its memory goes with it).
    pub fn rest(&mut self) {
        self.stop_child();
    }
}

impl Drop for ChildHear {
    fn drop(&mut self) {
        self.stop_child();
    }
}

impl Hear for ChildHear {
    fn hear(&mut self, sound: &[f32]) -> Result<Vec<Line>, String> {
        if self.running.is_none() {
            if !self.may_start() {
                return Err(self.failed.as_ref().map(|(e, _)| e.clone()).unwrap_or_default());
            }
            if let Err(e) = self.spawn() {
                log::warn!("{e}");
                self.failed = Some((e.clone(), Instant::now()));
                return Err(e);
            }
        }
        let result = (|| {
            let r = self.running.as_mut().ok_or("the hearer isn't running")?;
            write_request(&mut r.stdin, sound).map_err(|e| format!("the hearer: {e}"))?;
            let line = r.lines.recv_timeout(ANSWER_WAIT).map_err(|e| match e {
                RecvTimeoutError::Timeout => "the hearer took too long".to_string(),
                RecvTimeoutError::Disconnected => "the hearer stopped".to_string(),
            })?;
            let a: Answer = serde_json::from_str(&line).map_err(|e| format!("the hearer: {e}"))?;
            if let Some(e) = a.error {
                return Err(e);
            }
            Ok(a.lines.into_iter().map(|l| Line { start: l.start, end: l.end, text: l.text }).collect())
        })();
        if let Err(e) = &result {
            log::warn!("{e}; starting it again");
            self.stop_child();
            self.failed = Some((e.clone(), Instant::now()));
        } else {
            self.failed = None;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_go_through_and_junk_is_refused() {
        let mut buf = Vec::new();
        write_request(&mut buf, &[0.5, -0.25, f32::NAN]).unwrap();
        write_request(&mut buf, &[]).unwrap();
        let mut r = std::io::Cursor::new(buf);
        assert_eq!(read_request(&mut r).unwrap(), Some(vec![0.5, -0.25, 0.0]));
        assert_eq!(read_request(&mut r).unwrap(), Some(vec![]));
        assert_eq!(read_request(&mut r).unwrap(), None);
        assert!(read_request(&mut std::io::Cursor::new(b"XXXX\0\0\0\0".to_vec())).is_err());
        let mut huge = b"RCH1".to_vec();
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(read_request(&mut std::io::Cursor::new(huge)).is_err());
        let mut short = b"RCH1".to_vec();
        short.extend_from_slice(&4u32.to_le_bytes());
        short.extend_from_slice(&[0u8; 5]);
        assert!(read_request(&mut std::io::Cursor::new(short)).is_err());
    }

    #[test]
    fn another_model_is_taken_up_only_when_it_differs() {
        let mut c = ChildHear::new(PathBuf::from("recallcraft"), Vec::new(), PathBuf::from("base.gguf"), 2);
        assert!(!c.set_model(PathBuf::from("base.gguf"), 2));
        c.failed = Some(("it failed".to_string(), Instant::now()));
        assert!(c.set_model(PathBuf::from("small.gguf"), 3));
        assert!(c.failed.is_none() && c.may_start(), "a new model is tried straight away");
        assert!(c.set_model(PathBuf::from("small.gguf"), 4), "more threads");
    }

    #[test]
    fn a_worker_without_its_model_says_so() {
        let mut out = Vec::new();
        let code = serve(Path::new("no/such/model.gguf"), &mut std::io::empty(), &mut out);
        assert_eq!(code, 2);
        let a: Answer = serde_json::from_slice(out.trim_ascii()).unwrap();
        assert!(a.error.is_some() && !a.ready);
    }
}
