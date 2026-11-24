//! The service as a persistent, per-user service: started when the owner signs in, and started
//! again if it stops unexpectedly. It records the owner's own devices in their own session, so
//! it's a sign-in service, not a system one.
//!
//! - **Windows:** a `Run` entry of the owner's own (`HKCU\…\CurrentVersion\Run\RecallCraft`,
//!   `recallcraft.exe --background`), with Windows' own switch for it
//!   (`HKCU\…\Explorer\StartupApproved\Run`, what Task Manager and Settings › Apps › Startup
//!   change). `--background` is the [`supervise`] loop.
//! - **macOS:** a LaunchAgent with `KeepAlive` on an unsuccessful exit.
//! - **Linux and the BSDs:** an XDG autostart entry running `--background`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

pub const NAME: &str = "RecallCraft";
pub const APP_ID: &str = "io.github.billsecond.recallcraft";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub enabled: bool,
    pub how: String,
}

pub fn program() -> Option<PathBuf> {
    if let Some(appimage) = std::env::var_os("APPIMAGE").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(appimage));
    }
    std::env::current_exe().ok()
}

/// Whether a program is a build in the source tree (it never registers itself).
pub fn development_build(exe: &Path) -> bool {
    let s = exe.to_string_lossy().replace('\\', "/");
    s.contains("/target/") || s.contains("/target-")
}

#[cfg(target_os = "windows")]
mod imp {
    use super::*;
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

    fn approval() -> Option<bool> {
        let k = RegKey::predef(HKEY_CURRENT_USER).open_subkey(APPROVED).ok()?;
        let v = k.get_raw_value(NAME).ok()?;
        // Task Manager writes 02 (or 06) for on and 03 (or 07, with a time) for off.
        v.bytes.first().map(|b| b & 1 == 0)
    }

    fn set_approval(on: bool) -> Result<(), String> {
        let (k, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(APPROVED).map_err(|e| e.to_string())?;
        let mut bytes = vec![0u8; 12];
        bytes[0] = if on { 2 } else { 3 };
        k.set_raw_value(NAME, &winreg::RegValue { bytes: bytes.into(), vtype: winreg::enums::RegType::REG_BINARY }).map_err(|e| e.to_string())
    }

    pub fn status() -> Status {
        let entry = RegKey::predef(HKEY_CURRENT_USER).open_subkey(RUN).ok().and_then(|k| k.get_value::<String, _>(NAME).ok());
        Status { enabled: entry.is_some() && approval().unwrap_or(true), how: "a sign-in entry (Settings › Apps › Startup)".to_string() }
    }

    pub fn enable(exe: &Path) -> Result<String, String> {
        let (k, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN).map_err(|e| e.to_string())?;
        k.set_value(NAME, &format!("\"{}\" --background", exe.display())).map_err(|e| e.to_string())?;
        let _ = set_approval(true);
        Ok("RecallCraft starts when you sign in".to_string())
    }

    pub fn disable() -> Result<String, String> {
        if let Ok(k) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN, winreg::enums::KEY_SET_VALUE) {
            let _ = k.delete_value(NAME);
        }
        Ok("RecallCraft no longer starts when you sign in".to_string())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    fn plist_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join("Library/LaunchAgents").join(format!("{APP_ID}.plist")))
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    pub fn plist(exe: &Path) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key>\n  <string>{APP_ID}</string>\n  <key>ProgramArguments</key>\n  <array>\n    <string>{}</string>\n    <string>--tray</string>\n  </array>\n  <key>RunAtLoad</key>\n  <true/>\n  <key>KeepAlive</key>\n  <dict>\n    <key>SuccessfulExit</key>\n    <false/>\n  </dict>\n  <key>ProcessType</key>\n  <string>Interactive</string>\n</dict>\n</plist>\n",
            xml_escape(&exe.to_string_lossy())
        )
    }

    pub fn status() -> Status {
        Status { enabled: plist_path().is_some_and(|p| p.exists()), how: "a LaunchAgent (~/Library/LaunchAgents)".to_string() }
    }

    pub fn enable(exe: &Path) -> Result<String, String> {
        let p = plist_path().ok_or("no home folder")?;
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        recallcraft_store::write_atomic(&p, plist(exe).as_bytes()).map_err(|e| e.to_string())?;
        Ok("RecallCraft starts when you sign in, and again if it stops".to_string())
    }

    pub fn disable() -> Result<String, String> {
        if let Some(p) = plist_path()
            && p.exists()
        {
            std::fs::remove_file(&p).map_err(|e| e.to_string())?;
        }
        Ok("RecallCraft no longer starts when you sign in".to_string())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use super::*;

    fn user_entry() -> Option<PathBuf> {
        dirs::config_dir().map(|c| c.join("autostart").join(format!("{APP_ID}.desktop")))
    }

    pub fn entry(exe: &Path) -> String {
        let quoted = format!("\"{}\"", exe.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\""));
        format!(
            "[Desktop Entry]\nType=Application\nName=RecallCraft\nComment=Keeps the last two days of what was said\nExec={quoted} --background\nTerminal=false\nNoDisplay=true\n"
        )
    }

    pub fn status() -> Status {
        Status { enabled: user_entry().is_some_and(|u| u.exists()), how: "an autostart entry (~/.config/autostart)".to_string() }
    }

    pub fn enable(exe: &Path) -> Result<String, String> {
        let u = user_entry().ok_or("no configuration folder")?;
        if let Some(dir) = u.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        recallcraft_store::write_atomic(&u, entry(exe).as_bytes()).map_err(|e| e.to_string())?;
        Ok("RecallCraft starts when you sign in".to_string())
    }

    pub fn disable() -> Result<String, String> {
        if let Some(u) = user_entry()
            && u.exists()
        {
            std::fs::remove_file(&u).map_err(|e| e.to_string())?;
        }
        Ok("RecallCraft no longer starts when you sign in".to_string())
    }
}

#[cfg(not(any(target_os = "windows", unix)))]
mod imp {
    use super::*;
    pub fn status() -> Status {
        Status { enabled: false, how: "not available on this system".to_string() }
    }
    pub fn enable(_exe: &Path) -> Result<String, String> {
        Err("not available on this system".to_string())
    }
    pub fn disable() -> Result<String, String> {
        Err("not available on this system".to_string())
    }
}

pub fn status() -> Status {
    imp::status()
}

/// Registers this program to start at sign-in.
pub fn enable() -> Result<String, String> {
    if crate::paths::no_prefs() {
        return Err("not while running without settings (RECALLCRAFT_NO_PREFS)".to_string());
    }
    let exe = program().ok_or("this program's own path is unknown")?;
    imp::enable(&exe)
}

pub fn disable() -> Result<String, String> {
    if crate::paths::no_prefs() {
        return Err("not while running without settings (RECALLCRAFT_NO_PREFS)".to_string());
    }
    imp::disable()
}

/// What the supervisor decides after the service ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum After {
    Stop,
    Restart(Duration),
    GiveUp,
}

const WINDOW: Duration = Duration::from_secs(120);
const MAX_RESTARTS: usize = 5;

/// What follows an exit: `code` None is a crash without a code.
pub fn after(code: Option<i32>, recent: &mut Vec<Instant>, now: Instant) -> After {
    if code == Some(0) {
        return After::Stop;
    }
    recent.retain(|t| now.duration_since(*t) < WINDOW);
    if recent.len() >= MAX_RESTARTS {
        return After::GiveUp;
    }
    recent.push(now);
    After::Restart(Duration::from_secs(1u64 << (recent.len() - 1).min(4)))
}

/// `--background`: runs `exe --tray` and starts it again whenever it ends unexpectedly.
pub fn supervise(exe: &Path) -> i32 {
    let mut recent = Vec::new();
    loop {
        let started = Instant::now();
        let status = Command::new(exe).arg("--tray").status();
        let code = match status {
            Ok(s) => s.code(),
            Err(e) => {
                log::error!("could not start the service: {e}");
                return 1;
            }
        };
        log::info!("the service ended with {code:?} after {} s", started.elapsed().as_secs());
        match after(code, &mut recent, Instant::now()) {
            After::Stop => return 0,
            After::GiveUp => {
                log::error!("the service stopped {MAX_RESTARTS} times in {} s; not starting it again", WINDOW.as_secs());
                return 1;
            }
            After::Restart(wait) => {
                log::warn!("starting the service again in {} s", wait.as_secs());
                std::thread::sleep(wait);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_supervisor_restarts_failures_and_gives_up() {
        let mut recent = Vec::new();
        let t0 = Instant::now();
        assert_eq!(after(Some(0), &mut recent, t0), After::Stop);
        for secs in [1, 2, 4, 8, 16] {
            assert_eq!(after(Some(1), &mut recent, t0), After::Restart(Duration::from_secs(secs)));
        }
        assert_eq!(after(None, &mut recent, t0), After::GiveUp);
        let later = t0 + WINDOW + Duration::from_secs(1);
        assert_eq!(after(Some(1), &mut recent, later), After::Restart(Duration::from_secs(1)));
        assert!(development_build(Path::new(r"N:\Source\RecallCraft\target\release\recallcraft.exe")));
        assert!(!development_build(Path::new(r"C:\Program Files\RecallCraft\recallcraft.exe")));
    }
}
