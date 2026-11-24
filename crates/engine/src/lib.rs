//! RecallCraft's recorder, which the service runs all the time.
//!
//! ```text
//!  microphone ─┐                     ┌─ the recording (ring)
//!              ├─ blocks ─ recorder ─┤
//!  computer  ──┘                     └─ utterances ─ hearer ─ transcripts (calls/<id>)
//!                                                      ▲
//!  devices: keeps the captures going, finds calls ─────┘ (calls started and ended)
//! ```
//!
//! - [`devices`]: starts and restarts the captures (the microphone a call uses, during a call),
//!   finds calls ([`recallcraft_calls`]), keeps each call's record, deletes what's kept long
//!   enough.
//! - [`recorder`]: writes the sound into the recording, and cuts it into utterances.
//! - [`hearer`]: hears the utterances of calls (in a program of its own, [`worker`]), tells the
//!   others' voices apart, leaves echoes out, and writes the lines.
//!
//! Layer L4: no windows. Everything runs on threads of its own; [`Engine::stop`] ends them.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod devices;
pub mod hearer;
pub mod known;
pub mod recorder;
pub mod worker;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::thread::JoinHandle;

use recallcraft_model::Settings;
use serde::{Deserialize, Serialize};

/// Where things are, and how to hear.
pub struct Config {
    /// The recording and the calls.
    pub data_dir: PathBuf,
    /// The settings folder (known voices are kept there).
    pub settings_dir: PathBuf,
    /// How lines are heard: a program of its own, normally.
    pub hear: worker::HearWith,
    /// This program's file, never taken for a calling app.
    pub own_exe: String,
}

/// What the service is doing, for the tray and the window (`status.json` in the data folder).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub recording: bool,
    /// Recording only during calls, and no call just now.
    pub waiting_for_call: bool,
    pub paused_until_ms: u64,
    pub mic: String,
    pub mic_level: f32,
    pub system: String,
    pub system_level: f32,
    /// The call going on.
    pub call_id: String,
    pub call_title: String,
    pub call_app: String,
    pub call_started_ms: u64,
    /// The hearer: "ready", "starting", "off", or why it can't.
    pub hearer: String,
    /// Seconds of speech waiting to be heard.
    pub waiting_s: f32,
    /// Something the owner should know (a device that won't start).
    pub problems: Vec<String>,
    /// When this was written (ms since 1970).
    pub at_ms: u64,
    pub pid: u32,
}

impl Status {
    pub fn in_call(&self) -> bool {
        !self.call_id.is_empty()
    }
}

/// What every thread shares.
pub struct Shared {
    pub settings: RwLock<Settings>,
    pub status: Mutex<Status>,
    pub stop: AtomicBool,
    /// When RecallCraft itself played sound (replays, the window's playback), µs since 1970: the
    /// computer's sound then has it in it, and isn't written down as the others'.
    own_sound: Mutex<Vec<(u64, u64)>>,
}

/// The most stretches of RecallCraft's own sound remembered.
const OWN_SOUND_KEPT: usize = 64;

impl Shared {
    pub fn new(settings: Settings) -> Shared {
        Shared {
            settings: RwLock::new(settings.checked()),
            status: Mutex::new(Status::default()),
            stop: AtomicBool::new(false),
            own_sound: Mutex::new(Vec::new()),
        }
    }

    /// RecallCraft plays sound from `from_us` to `until_us` (joined to the stretch before when they
    /// touch).
    pub fn playing(&self, from_us: u64, until_us: u64) {
        if until_us <= from_us {
            return;
        }
        let mut own = self.own_sound.lock().unwrap_or_else(PoisonError::into_inner);
        match own.last_mut() {
            Some(last) if from_us <= last.1 + 1_000_000 && from_us >= last.0 => last.1 = last.1.max(until_us),
            _ => own.push((from_us, until_us)),
        }
        if own.len() > OWN_SOUND_KEPT {
            own.remove(0);
        }
    }

    /// Whether RecallCraft was playing sound at any time in `[from, to)`.
    pub fn was_playing(&self, from_us: u64, to_us: u64) -> bool {
        self.own_sound.lock().unwrap_or_else(PoisonError::into_inner).iter().any(|(a, b)| *a < to_us && from_us < *b)
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn update(&self, f: impl FnOnce(&mut Status)) {
        f(&mut self.status.lock().unwrap_or_else(PoisonError::into_inner));
    }

    pub fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

/// The recorder, running.
pub struct Engine {
    pub shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

impl Engine {
    pub fn start(config: Config, settings: Settings) -> Engine {
        let shared = Arc::new(Shared::new(settings));
        let (block_tx, block_rx) = channel();
        let (hear_tx, hear_rx) = channel();
        let mut threads = Vec::new();
        let layout = recallcraft_store::Layout::new(&config.data_dir);
        let spawn = |name: &str, f: Box<dyn FnOnce() + Send>| {
            std::thread::Builder::new().name(name.into()).spawn(f).inspect_err(|e| log::error!("starting {name}: {e}")).ok()
        };
        {
            let (s, ring, tx) = (shared.clone(), layout.ring(), hear_tx.clone());
            threads.extend(spawn("recorder", Box::new(move || recorder::run(&s, &ring, block_rx, tx))));
        }
        {
            let (s, l, settings_dir, hear) = (shared.clone(), layout.clone(), config.settings_dir.clone(), config.hear);
            threads.extend(spawn("hearer", Box::new(move || hearer::run(&s, &l, &settings_dir, hear, hear_rx))));
        }
        {
            let (s, l, own) = (shared.clone(), layout, config.own_exe);
            threads.extend(spawn("devices", Box::new(move || devices::run(&s, &l, &own, block_tx, hear_tx))));
        }
        Engine { shared, threads }
    }

    pub fn set_settings(&self, s: Settings) {
        *self.shared.settings.write().unwrap_or_else(PoisonError::into_inner) = s.checked();
    }

    pub fn status(&self) -> Status {
        self.shared.status()
    }

    /// Stops everything: the call going on ends, the recording's files are closed.
    pub fn stop(self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        for t in self.threads {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !t.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if t.is_finished() {
                let _ = t.join();
            }
        }
    }
}
