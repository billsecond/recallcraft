//! What's going on just now: which programs have a microphone open, and the programs' windows.
//!
//! Windows: every capture device's audio sessions (WASAPI), with the process of each one that's
//! active; every process with its parent (a tool-help snapshot), so a webview's microphone is put
//! down to the app that started it; and every visible top-level window's title. Elsewhere there
//! is nothing to look at yet, so calls aren't found by themselves.

use std::collections::HashMap;

use crate::detector::Capturer;

/// A process: its parent and program file (lower case).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub parent: u32,
    pub exe: String,
}

/// Every process.
pub type Processes = HashMap<u32, Process>;

/// The app a process belongs to: the nearest of it and its parents that's a known calling app
/// or browser (a webview's is the app that started it), else the process itself.
pub fn app_of(pid: u32, procs: &Processes) -> String {
    let own = procs.get(&pid).map(|p| p.exe.clone()).unwrap_or_default();
    let mut at = pid;
    for _ in 0..8 {
        let Some(p) = procs.get(&at) else { break };
        if crate::apps::known(&p.exe) {
            return p.exe.clone();
        }
        if p.parent == 0 || p.parent == at {
            break;
        }
        at = p.parent;
    }
    own
}

/// A window: its process and title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub pid: u32,
    pub title: String,
}

#[cfg(target_os = "windows")]
mod imp {
    use super::*;
    use wasapi::{DeviceEnumerator, Direction, SessionState};
    use winsafe::co::TH32CS;
    use winsafe::{EnumWindows, HPROCESSLIST};

    pub fn processes() -> Processes {
        let mut out = Processes::new();
        let Ok(mut snap) = HPROCESSLIST::CreateToolhelp32Snapshot(TH32CS::SNAPPROCESS, None) else { return out };
        for p in snap.iter_processes().flatten().take(20_000) {
            out.insert(p.th32ProcessID, Process { parent: p.th32ParentProcessID, exe: p.szExeFile().to_ascii_lowercase() });
        }
        out
    }

    pub fn capturing(procs: &Processes, own: &[u32]) -> Vec<Capturer> {
        let _ = wasapi::initialize_mta();
        let mut out = Vec::new();
        let Ok(e) = DeviceEnumerator::new() else { return out };
        let Ok(devices) = e.get_device_collection(Direction::Capture) else { return out };
        for dev in (&devices).into_iter().flatten() {
            let id = dev.get_id().unwrap_or_default();
            let Ok(manager) = dev.get_iaudiosessionmanager() else { continue };
            let Ok(sessions) = manager.get_audiosessionenumerator() else { continue };
            let count = sessions.get_count().unwrap_or(0).clamp(0, 500);
            for i in 0..count {
                let Ok(s) = sessions.get_session(i) else { continue };
                if s.get_state().ok() != Some(SessionState::Active) {
                    continue;
                }
                let Ok(pid) = s.get_process_id() else { continue };
                if pid == 0 || own.contains(&pid) {
                    continue;
                }
                let exe = procs.get(&pid).map(|p| p.exe.clone()).unwrap_or_default();
                out.push(Capturer { pid, app_exe: app_of(pid, procs), exe, device_id: id.clone() });
            }
        }
        out
    }

    pub fn windows() -> Vec<Window> {
        let mut out = Vec::new();
        let _ = EnumWindows(|hwnd| {
            if out.len() < 2000 && hwnd.IsWindowVisible() {
                let title = hwnd.GetWindowText().unwrap_or_default();
                if !title.trim().is_empty() {
                    let (_, pid) = hwnd.GetWindowThreadProcessId();
                    out.push(Window { pid, title });
                }
            }
            true
        });
        out
    }

    pub fn playing(procs: &Processes, app_exe: &str) -> Vec<Playing> {
        let _ = wasapi::initialize_mta();
        let mut out: Vec<Playing> = Vec::new();
        let Ok(e) = DeviceEnumerator::new() else { return out };
        let Ok(devices) = e.get_device_collection(Direction::Render) else { return out };
        for dev in (&devices).into_iter().flatten() {
            let Ok(id) = dev.get_id() else { continue };
            let Ok(manager) = dev.get_iaudiosessionmanager() else { continue };
            let Ok(sessions) = manager.get_audiosessionenumerator() else { continue };
            let count = sessions.get_count().unwrap_or(0).clamp(0, 500);
            for i in 0..count {
                let Ok(s) = sessions.get_session(i) else { continue };
                if s.get_state().ok() != Some(SessionState::Active) {
                    continue;
                }
                let Ok(pid) = s.get_process_id() else { continue };
                if pid == 0 || app_of(pid, procs) != app_exe {
                    continue;
                }
                let peak = s.get_audiometerinformation().and_then(|m| m.get_peak_value()).unwrap_or(0.0);
                let peak = if peak.is_finite() { peak.clamp(0.0, 1.0) } else { 0.0 };
                match out.iter_mut().find(|p| p.device_id == id) {
                    Some(p) => p.peak = p.peak.max(peak),
                    None => out.push(Playing { device_id: id.clone(), peak }),
                }
            }
        }
        out
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use super::*;

    pub fn processes() -> Processes {
        Processes::new()
    }

    pub fn capturing(_procs: &Processes, _own: &[u32]) -> Vec<Capturer> {
        Vec::new()
    }

    pub fn playing(_procs: &Processes, _app_exe: &str) -> Vec<Playing> {
        Vec::new()
    }

    pub fn windows() -> Vec<Window> {
        Vec::new()
    }
}

/// Every process.
pub fn processes() -> Processes {
    imp::processes()
}

/// The programs with a microphone open, but for `own` (RecallCraft's own processes).
pub fn capturing(procs: &Processes, own: &[u32]) -> Vec<Capturer> {
    imp::capturing(procs, own)
}

/// Every visible top-level window with a title.
pub fn windows() -> Vec<Window> {
    imp::windows()
}

/// A speaker an app is playing through just now, and how loud it is (0 to 1).
#[derive(Clone, Debug, PartialEq)]
pub struct Playing {
    pub device_id: String,
    pub peak: f32,
}

/// The speakers the app `app_exe` (and the programs it started) is playing through just now.
pub fn playing(procs: &Processes, app_exe: &str) -> Vec<Playing> {
    imp::playing(procs, app_exe)
}

/// Follows which speaker a call is played on: the one it plays loudest through, held for a
/// while so a moment of quiet (or a notification sound elsewhere) doesn't switch it.
#[derive(Clone, Debug, Default)]
pub struct CallSpeaker {
    /// Each speaker's loudness, decaying (so the one used lately wins).
    seen: Vec<(String, f32)>,
}

impl CallSpeaker {
    /// What was seen this time; returns the speaker the call is on, if any.
    pub fn observe(&mut self, playing: &[Playing]) -> Option<String> {
        for s in &mut self.seen {
            s.1 *= 0.85;
        }
        for p in playing {
            // Just being there counts a little; sound counts more.
            let score = 0.05 + p.peak;
            match self.seen.iter_mut().find(|s| s.0 == p.device_id) {
                Some(s) => s.1 += score,
                None => self.seen.push((p.device_id.clone(), score)),
            }
        }
        self.seen.retain(|s| s.1 > 0.01);
        self.seen.iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|s| s.0.clone())
    }

    pub fn reset(&mut self) {
        self.seen.clear();
    }
}

/// The processes of an app: those whose program is `app_exe`, and those they started.
pub fn pids_of(app_exe: &str, procs: &Processes) -> Vec<u32> {
    procs.keys().copied().filter(|pid| app_of(*pid, procs) == app_exe || procs.get(pid).is_some_and(|p| p.exe == app_exe)).collect()
}

/// RecallCraft's own processes: this one, and those it started (the window, the transcriber).
pub fn own_tree(procs: &Processes) -> Vec<u32> {
    let me = std::process::id();
    let mut out = vec![me];
    for (pid, p) in procs {
        if p.parent == me {
            out.push(*pid);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(id: &str, peak: f32) -> Playing {
        Playing { device_id: id.into(), peak }
    }

    #[test]
    fn the_call_s_speaker_is_the_one_it_plays_on() {
        let mut s = CallSpeaker::default();
        assert_eq!(s.observe(&[]), None);
        // It rings on the speakers, then the call is answered on the headset.
        assert_eq!(s.observe(&[at("speakers", 0.6)]).as_deref(), Some("speakers"));
        for _ in 0..6 {
            s.observe(&[at("headset", 0.3)]);
        }
        assert_eq!(s.observe(&[at("headset", 0.3)]).as_deref(), Some("headset"));
        // A moment of quiet, or a notification on the speakers, doesn't move it.
        assert_eq!(s.observe(&[at("headset", 0.0)]).as_deref(), Some("headset"));
        assert_eq!(s.observe(&[at("headset", 0.0), at("speakers", 0.2)]).as_deref(), Some("headset"));
        // Gone a long while: forgotten.
        for _ in 0..40 {
            s.observe(&[]);
        }
        assert_eq!(s.observe(&[]), None);
        s.reset();
    }

    #[test]
    fn a_webview_s_microphone_is_its_app_s() {
        let mut procs = Processes::new();
        procs.insert(4, Process { parent: 1, exe: "explorer.exe".into() });
        procs.insert(10, Process { parent: 4, exe: "ms-teams.exe".into() });
        procs.insert(11, Process { parent: 10, exe: "msedgewebview2.exe".into() });
        procs.insert(12, Process { parent: 11, exe: "msedgewebview2.exe".into() });
        procs.insert(20, Process { parent: 4, exe: "obs64.exe".into() });
        procs.insert(30, Process { parent: 30, exe: "loop.exe".into() });
        assert_eq!(app_of(12, &procs), "ms-teams.exe");
        assert_eq!(app_of(20, &procs), "obs64.exe");
        assert_eq!(app_of(30, &procs), "loop.exe", "a process that's its own parent ends the walk");
        assert_eq!(app_of(99, &procs), "");
        let mut teams = pids_of("ms-teams.exe", &procs);
        teams.sort();
        assert_eq!(teams, vec![10, 11, 12]);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn this_computer_can_be_looked_at() {
        let procs = processes();
        assert!(procs.len() > 5, "{}", procs.len());
        assert!(procs.contains_key(&std::process::id()));
        // What has a microphone open, and the windows: whatever they are, looking doesn't fail.
        let _ = capturing(&procs, &own_tree(&procs));
        let _ = windows();
    }
}
