//! A device being recorded, on a thread of its own, handing its sound over as [`Block`]s.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Sender, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use recallcraft_model::Stream;

use crate::{AudioError, Block};

/// Which microphone.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Mic {
    /// The system's default microphone.
    Default,
    /// The system's default microphone for calls (Windows lets it be another).
    Communications,
    /// A device by its id (a call's microphone, found by which device it uses).
    Id(String),
    /// A device by name (the settings).
    Named(String),
}

/// What to record.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Mic(Mic),
    /// What the default speaker plays (its loopback, as ScreenGrabCraft records it).
    System,
    /// What one speaker plays, by its id: the one a call is played on when it isn't the default
    /// (a headset), whose sound the default speaker's loopback doesn't have.
    Speaker(String),
}

impl Source {
    pub fn stream(&self) -> Stream {
        match self {
            Source::Mic(_) => Stream::Mic,
            Source::System | Source::Speaker(_) => Stream::System,
        }
    }
}

/// What the device thread shares with whoever started it.
pub(crate) struct Shared {
    pub stop: AtomicBool,
    pub alive: AtomicBool,
    pub level: AtomicU32,
    /// When sound last came in (ms since `started`).
    pub heard_ms: AtomicU32,
    pub started: Instant,
}

impl Shared {
    pub fn heard(&self) {
        let ms = self.started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
        self.heard_ms.store(ms, Ordering::Relaxed);
    }

    pub fn set_level(&self, peak: f32) {
        self.level.store(peak.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
}

/// A device being recorded.
pub struct Capture {
    pub source: Source,
    /// The device's name, for the owner.
    pub name: String,
    /// The device's id, if the system has one.
    pub id: String,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

/// What a device thread says once it has started (or why it couldn't).
pub(crate) type Ready = std::sync::mpsc::SyncSender<Result<(String, String), String>>;

impl Capture {
    /// Starts recording; its sound goes to `tx`.
    pub fn start(source: Source, tx: Sender<Block>) -> Result<Capture, AudioError> {
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            alive: AtomicBool::new(true),
            level: AtomicU32::new(0),
            heard_ms: AtomicU32::new(0),
            started: Instant::now(),
        });
        let (ready_tx, ready_rx) = sync_channel::<Result<(String, String), String>>(1);
        let (s, src) = (shared.clone(), source.clone());
        let name = match source {
            Source::Mic(_) => "microphone",
            Source::System | Source::Speaker(_) => "computer sound",
        };
        let thread = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                run(src, tx, ready_tx, &s);
                s.alive.store(false, Ordering::SeqCst);
            })
            .map_err(|e| AudioError::Device(e.to_string()))?;
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok((name, id))) => Ok(Capture { source, name, id, shared, thread: Some(thread) }),
            Ok(Err(e)) => Err(AudioError::Device(e)),
            Err(_) => {
                shared.stop.store(true, Ordering::SeqCst);
                Err(AudioError::Device("the sound device didn't start".to_string()))
            }
        }
    }

    /// Whether it's still recording (a device unplugged ends it).
    pub fn is_alive(&self) -> bool {
        self.shared.alive.load(Ordering::SeqCst)
    }

    /// How loud it is just now, 0 to 1.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.shared.level.load(Ordering::Relaxed))
    }

    /// How long since sound last came in.
    pub fn silent_for(&self) -> Duration {
        let heard = Duration::from_millis(u64::from(self.shared.heard_ms.load(Ordering::Relaxed)));
        self.shared.started.elapsed().saturating_sub(heard)
    }

    /// Stops it; a device stuck in its driver isn't waited for longer than a moment.
    pub fn stop(mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let deadline = Instant::now() + Duration::from_millis(1500);
            while !t.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            if t.is_finished() {
                let _ = t.join();
            } else {
                log::warn!("the {} thread didn't stop in time; left to end on its own", self.name);
            }
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
    }
}

#[cfg(target_os = "windows")]
fn run(source: Source, tx: Sender<Block>, ready: Ready, shared: &Shared) {
    crate::wasapi_capture::run(source, tx, ready, shared);
}

#[cfg(not(target_os = "windows"))]
fn run(source: Source, tx: Sender<Block>, ready: Ready, shared: &Shared) {
    crate::cpal_capture::run(source, tx, ready, shared);
}

/// Whether what the computer plays can be recorded here.
pub fn system_sound_supported() -> bool {
    cfg!(target_os = "windows")
}
