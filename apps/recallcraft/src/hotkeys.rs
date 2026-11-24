//! The global shortcuts: Replay, and opening RecallCraft. Written like `Ctrl+Alt+R`.

use global_hotkey::GlobalHotKeyManager;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use recallcraft_model::Settings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Replay,
    Open,
}

fn code(name: &str) -> Option<Code> {
    let n = name.trim().to_ascii_uppercase();
    let c = match n.as_str() {
        "SPACE" => Code::Space,
        "PRINTSCREEN" | "PRTSC" => Code::PrintScreen,
        "INSERT" | "INS" => Code::Insert,
        "HOME" => Code::Home,
        "END" => Code::End,
        "PAGEUP" => Code::PageUp,
        "PAGEDOWN" => Code::PageDown,
        "PAUSE" => Code::Pause,
        "SCROLLLOCK" => Code::ScrollLock,
        "BACKSPACE" => Code::Backspace,
        "ENTER" => Code::Enter,
        "TAB" => Code::Tab,
        "LEFT" => Code::ArrowLeft,
        "RIGHT" => Code::ArrowRight,
        "UP" => Code::ArrowUp,
        "DOWN" => Code::ArrowDown,
        "F1" => Code::F1,
        "F2" => Code::F2,
        "F3" => Code::F3,
        "F4" => Code::F4,
        "F5" => Code::F5,
        "F6" => Code::F6,
        "F7" => Code::F7,
        "F8" => Code::F8,
        "F9" => Code::F9,
        "F10" => Code::F10,
        "F11" => Code::F11,
        "F12" => Code::F12,
        "," => Code::Comma,
        "." => Code::Period,
        "/" => Code::Slash,
        ";" => Code::Semicolon,
        "'" => Code::Quote,
        "[" => Code::BracketLeft,
        "]" => Code::BracketRight,
        "-" => Code::Minus,
        "=" => Code::Equal,
        "`" => Code::Backquote,
        other => {
            let mut chars = other.chars();
            let (Some(ch), None) = (chars.next(), chars.next()) else { return None };
            const LETTERS: [Code; 26] = [
                Code::KeyA,
                Code::KeyB,
                Code::KeyC,
                Code::KeyD,
                Code::KeyE,
                Code::KeyF,
                Code::KeyG,
                Code::KeyH,
                Code::KeyI,
                Code::KeyJ,
                Code::KeyK,
                Code::KeyL,
                Code::KeyM,
                Code::KeyN,
                Code::KeyO,
                Code::KeyP,
                Code::KeyQ,
                Code::KeyR,
                Code::KeyS,
                Code::KeyT,
                Code::KeyU,
                Code::KeyV,
                Code::KeyW,
                Code::KeyX,
                Code::KeyY,
                Code::KeyZ,
            ];
            const DIGITS: [Code; 10] = [
                Code::Digit0,
                Code::Digit1,
                Code::Digit2,
                Code::Digit3,
                Code::Digit4,
                Code::Digit5,
                Code::Digit6,
                Code::Digit7,
                Code::Digit8,
                Code::Digit9,
            ];
            if ch.is_ascii_uppercase() {
                return LETTERS.get((ch as u8 - b'A') as usize).copied();
            }
            if ch.is_ascii_digit() {
                return DIGITS.get((ch as u8 - b'0') as usize).copied();
            }
            return None;
        }
    };
    Some(c)
}

/// A shortcut from its text (`Ctrl+Alt+R`); an error says what's wrong with it.
pub fn parse(text: &str) -> Result<HotKey, String> {
    let mut mods = Modifiers::empty();
    let mut key = None;
    for part in text.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "alt" | "option" => mods |= Modifiers::ALT,
            "shift" => mods |= Modifiers::SHIFT,
            "win" | "super" | "cmd" | "meta" => mods |= Modifiers::SUPER,
            _ => {
                if key.is_some() {
                    return Err(format!("{text}: one key only, with Ctrl, Alt, Shift or Win"));
                }
                key = Some(code(part).ok_or_else(|| format!("{part} isn't a key RecallCraft knows"))?);
            }
        }
    }
    let key = key.ok_or_else(|| format!("{text}: which key?"))?;
    if mods.is_empty()
        && !matches!(
            key,
            Code::PrintScreen
                | Code::Pause
                | Code::ScrollLock
                | Code::F1
                | Code::F2
                | Code::F3
                | Code::F4
                | Code::F5
                | Code::F6
                | Code::F7
                | Code::F8
                | Code::F9
                | Code::F10
                | Code::F11
                | Code::F12
        )
    {
        return Err(format!("{text}: add Ctrl, Alt or Shift, or it would take that key from every program"));
    }
    Ok(HotKey::new(Some(mods), key))
}

/// The shortcuts registered with the system.
pub struct Registered {
    manager: Option<GlobalHotKeyManager>,
    keys: Vec<(HotKey, Target)>,
    texts: (String, String),
    pub problems: Vec<String>,
}

impl Registered {
    pub fn new() -> Registered {
        let manager = GlobalHotKeyManager::new().inspect_err(|e| log::warn!("no global shortcuts: {e}")).ok();
        Registered { manager, keys: Vec::new(), texts: (String::new(), String::new()), problems: Vec::new() }
    }

    /// Registers the settings' shortcuts (again, if they changed).
    pub fn apply(&mut self, s: &Settings) {
        let texts = (s.replay_hotkey.trim().to_string(), s.open_hotkey.trim().to_string());
        if texts == self.texts && (!self.keys.is_empty() || (texts.0.is_empty() && texts.1.is_empty())) {
            return;
        }
        let Some(m) = &self.manager else { return };
        for (k, _) in self.keys.drain(..) {
            let _ = m.unregister(k);
        }
        self.problems.clear();
        for (text, target) in [(&texts.0, Target::Replay), (&texts.1, Target::Open)] {
            if text.is_empty() {
                continue;
            }
            match parse(text).and_then(|k| m.register(k).map(|()| k).map_err(|e| format!("{text}: {e} (another program may have it)"))) {
                Ok(k) => self.keys.push((k, target)),
                Err(e) => {
                    log::warn!("shortcut: {e}");
                    self.problems.push(e);
                }
            }
        }
        self.texts = texts;
    }

    pub fn target(&self, id: u32) -> Option<Target> {
        self.keys.iter().find(|(k, _)| k.id() == id).map(|(_, t)| *t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_are_read_and_bad_ones_explained() {
        assert_eq!(parse("Ctrl+Alt+R").unwrap(), HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyR));
        assert_eq!(parse(" shift + f9 ").unwrap(), HotKey::new(Some(Modifiers::SHIFT), Code::F9));
        assert!(parse("F8").is_ok(), "a function key alone");
        assert!(parse("R").is_err(), "a letter alone would take it from every program");
        assert!(parse("Ctrl+Alt").is_err());
        assert!(parse("Ctrl+R+T").is_err());
        assert!(parse("Ctrl+Banana").is_err());
        assert!(parse("").is_err());
    }
}
