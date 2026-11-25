//! The owner's settings. A missing field takes its default, and [`Settings::checked`] clamps
//! every number, so a settings file edited by hand (or damaged) never breaks the program.

use serde::{Deserialize, Serialize};

/// The speech model that writes down what's said: Whisper, English only, in three sizes. Base is
/// installed with RecallCraft; Small and Medium are downloaded when chosen. Kept as `base`,
/// `small` or `medium`; anything else reads as [`SpeechModel::Base`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum SpeechModel {
    #[default]
    Base,
    Small,
    Medium,
}

impl SpeechModel {
    pub const ALL: [SpeechModel; 3] = [SpeechModel::Base, SpeechModel::Small, SpeechModel::Medium];

    pub fn name(self) -> &'static str {
        match self {
            SpeechModel::Base => "Base",
            SpeechModel::Small => "Small",
            SpeechModel::Medium => "Medium",
        }
    }

    /// Its file, in the models folder.
    pub fn file(self) -> &'static str {
        match self {
            SpeechModel::Base => "whisper-base.en-q8_0.gguf",
            SpeechModel::Small => "whisper-small.en-q8_0.gguf",
            SpeechModel::Medium => "whisper-medium.en-q8_0.gguf",
        }
    }

    /// What choosing it means, in a few words.
    pub fn about(self) -> &'static str {
        match self {
            SpeechModel::Base => "installed with RecallCraft; the quickest",
            SpeechModel::Small => "hears better; a 270 MB download, about three times the work",
            SpeechModel::Medium => "hears best; an 830 MB download, about ten times the work (for a fast processor)",
        }
    }

    /// How many of the processor's threads it hears on (of `cores`), at below-normal priority.
    pub fn threads(self, cores: usize) -> usize {
        let most = match self {
            SpeechModel::Base => 2,
            SpeechModel::Small => 3,
            SpeechModel::Medium => 4,
        };
        (cores / 4).clamp(1, most)
    }
}

impl From<String> for SpeechModel {
    fn from(s: String) -> SpeechModel {
        match s.trim() {
            "small" => SpeechModel::Small,
            "medium" => SpeechModel::Medium,
            _ => SpeechModel::Base,
        }
    }
}

impl From<SpeechModel> for String {
    fn from(m: SpeechModel) -> String {
        m.name().to_ascii_lowercase()
    }
}

/// What's recorded. Kept as `always` or `calls_only`; anything else (a mode a later version
/// writes) reads as [`RecordMode::Always`], and the other settings stay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum RecordMode {
    /// Everything, all the time: the last hours are kept, calls or not.
    #[default]
    Always,
    /// Only during calls: the microphone and the computer's sound are opened when a call starts,
    /// and closed when it ends.
    CallsOnly,
}

impl From<String> for RecordMode {
    fn from(s: String) -> RecordMode {
        if s.trim() == "calls_only" { RecordMode::CallsOnly } else { RecordMode::Always }
    }
}

impl From<RecordMode> for String {
    fn from(m: RecordMode) -> String {
        match m {
            RecordMode::Always => "always".to_string(),
            RecordMode::CallsOnly => "calls_only".to_string(),
        }
    }
}

impl RecordMode {
    pub fn label(self) -> &'static str {
        match self {
            RecordMode::Always => "Record all the time",
            RecordMode::CallsOnly => "Record only during calls",
        }
    }
}

/// When the microphone is recorded (while recording all the time).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MicMode {
    /// All the time.
    Always,
    /// Only while a call goes on.
    DuringCalls,
    /// All the time, except a Bluetooth headset's microphone, which is recorded only during
    /// calls: while a program records it, a Bluetooth headset switches to its hands-free mode,
    /// and its sound gets much worse for everything else.
    Auto,
}

/// The sound of an export.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioExport {
    None,
    /// One WAV file, both sides mixed.
    Mixed,
    /// One WAV file, you on the left and the others on the right.
    Stereo,
    /// Two WAV files, yours and theirs.
    Separate,
    /// One Opus file (`.opus`), both sides mixed: a tenth of a WAV's size.
    Opus,
}

impl AudioExport {
    pub const ALL: [AudioExport; 5] = [AudioExport::Mixed, AudioExport::Stereo, AudioExport::Separate, AudioExport::Opus, AudioExport::None];

    pub fn label(self) -> &'static str {
        match self {
            AudioExport::None => "No sound",
            AudioExport::Mixed => "WAV, both sides mixed",
            AudioExport::Stereo => "WAV, you left and them right",
            AudioExport::Separate => "Two WAV files, yours and theirs",
            AudioExport::Opus => "Opus, both sides mixed (small)",
        }
    }
}

/// A transcript's format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptFormat {
    /// Plain text: `[00:01:23] John: …`.
    Text,
    Markdown,
    /// SubRip subtitles.
    Srt,
    /// WebVTT subtitles, with who spoke.
    Vtt,
    /// Everything, for programs.
    Json,
}

impl TranscriptFormat {
    pub const ALL: [TranscriptFormat; 5] =
        [TranscriptFormat::Text, TranscriptFormat::Markdown, TranscriptFormat::Srt, TranscriptFormat::Vtt, TranscriptFormat::Json];

    pub fn extension(self) -> &'static str {
        match self {
            TranscriptFormat::Text => "txt",
            TranscriptFormat::Markdown => "md",
            TranscriptFormat::Srt => "srt",
            TranscriptFormat::Vtt => "vtt",
            TranscriptFormat::Json => "json",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TranscriptFormat::Text => "Text (.txt)",
            TranscriptFormat::Markdown => "Markdown (.md)",
            TranscriptFormat::Srt => "Subtitles (.srt)",
            TranscriptFormat::Vtt => "Subtitles with speakers (.vtt)",
            TranscriptFormat::Json => "JSON (.json)",
        }
    }

    pub fn from_name(name: &str) -> Option<TranscriptFormat> {
        TranscriptFormat::ALL
            .into_iter()
            .find(|f| f.extension().eq_ignore_ascii_case(name.trim()) || (name.eq_ignore_ascii_case("text") && *f == TranscriptFormat::Text))
    }
}

/// How calls are exported.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportSettings {
    pub audio: AudioExport,
    pub transcripts: Vec<TranscriptFormat>,
    /// Where exports go; empty is `Documents/RecallCraft`.
    pub folder: String,
    /// The file name, without its extension: `{date}`, `{time}`, `{title}`, `{app}`.
    pub name: String,
    /// Each export in a folder of its own, named like its files.
    pub own_folder: bool,
}

impl Default for ExportSettings {
    fn default() -> ExportSettings {
        ExportSettings {
            audio: AudioExport::Mixed,
            transcripts: vec![TranscriptFormat::Text],
            folder: String::new(),
            name: "{date} {time} {title}".to_string(),
            own_folder: false,
        }
    }
}

/// Every setting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// What your lines are labelled with; empty is "Me".
    pub your_name: String,
    /// Recording is on (the tray's Pause turns it off for a while: [`Settings::paused_until_ms`]).
    pub recording: bool,
    /// Paused until then (ms since 1970); 0 is not paused.
    pub paused_until_ms: u64,
    /// All the time, or only during calls.
    pub record_mode: RecordMode,
    /// The window was closed once: it was said that RecallCraft goes on in the tray.
    pub told_about_tray: bool,
    /// How long everything heard is kept, in hours.
    pub keep_audio_hours: u32,
    /// How long calls are kept, in days (their sound and transcript); 0 keeps them only as long as
    /// everything else.
    pub keep_calls_days: u32,
    /// Where the recording and the calls are kept; empty is the local app data folder.
    pub data_folder: String,
    pub mic_mode: MicMode,
    /// A microphone by name; empty follows the call (the microphone the calling app uses during
    /// a call, Windows' default otherwise).
    pub microphone: String,
    /// Record what the computer plays.
    pub record_system: bool,
    /// Write down what is said during calls, as it is said.
    pub transcribe_calls: bool,
    /// Tell the other people's voices apart (Speaker 1, Speaker 2, …).
    pub separate_voices: bool,
    /// Name the voices from the meeting app's window during a call (who it shows speaking, its
    /// captions' speakers), read as a screen reader reads it.
    pub names_from_app: bool,
    /// The speech model that writes down what's said.
    pub speech_model: SpeechModel,
    /// A browser using the microphone is a call (Meet, Teams or Zoom in a browser).
    pub browsers_are_calls: bool,
    /// Any program using the microphone is a call.
    pub any_app_is_a_call: bool,
    /// More programs (their program file, `myphone.exe`) whose use of the microphone is a call.
    pub more_call_apps: Vec<String>,
    /// Programs never taken for a call.
    pub never_call_apps: Vec<String>,
    /// How far back Replay goes, in seconds.
    pub replay_seconds: u32,
    /// How fast Replay plays (1 to 3).
    pub replay_speed: f32,
    /// The shortcut for Replay (`Ctrl+Alt+R`), empty for none.
    pub replay_hotkey: String,
    /// The shortcut that opens RecallCraft, empty for none.
    pub open_hotkey: String,
    /// A dot on the tray icon while a call is recorded.
    pub tray_dot: bool,
    pub export: ExportSettings,
    /// Playback speed in the window.
    pub playback_speed: f32,
    /// Ask GitHub for a newer RecallCraft now and then (the only thing that goes to the network).
    pub check_updates: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            your_name: String::new(),
            recording: true,
            paused_until_ms: 0,
            record_mode: RecordMode::Always,
            told_about_tray: false,
            keep_audio_hours: 48,
            keep_calls_days: 30,
            data_folder: String::new(),
            mic_mode: MicMode::Auto,
            microphone: String::new(),
            record_system: true,
            transcribe_calls: true,
            separate_voices: true,
            names_from_app: true,
            speech_model: SpeechModel::Base,
            browsers_are_calls: true,
            any_app_is_a_call: false,
            more_call_apps: Vec::new(),
            never_call_apps: Vec::new(),
            replay_seconds: 30,
            replay_speed: 1.5,
            replay_hotkey: "Ctrl+Alt+R".to_string(),
            open_hotkey: String::new(),
            tray_dot: true,
            export: ExportSettings::default(),
            playback_speed: 1.0,
            check_updates: true,
        }
    }
}

pub const MIN_KEEP_HOURS: u32 = 1;
pub const MAX_KEEP_HOURS: u32 = 24 * 30;
pub const MAX_KEEP_DAYS: u32 = 3650;
pub const SPEEDS: [f32; 6] = [0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

fn clean_list(list: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in list.into_iter().take(64) {
        let i = crate::call::clip(item.trim(), 120).to_ascii_lowercase();
        if !i.is_empty() && !out.contains(&i) {
            out.push(i);
        }
    }
    out
}

fn speed(s: f32, default: f32) -> f32 {
    if s.is_finite() { s.clamp(0.5, 3.0) } else { default }
}

impl Settings {
    /// Every value in range.
    pub fn checked(mut self) -> Settings {
        self.your_name = crate::call::clip(self.your_name.trim(), crate::call::MAX_NAME);
        self.keep_audio_hours = self.keep_audio_hours.clamp(MIN_KEEP_HOURS, MAX_KEEP_HOURS);
        self.keep_calls_days = self.keep_calls_days.min(MAX_KEEP_DAYS);
        self.replay_seconds = self.replay_seconds.clamp(5, 600);
        self.replay_speed = speed(self.replay_speed, 1.5);
        self.playback_speed = speed(self.playback_speed, 1.0);
        self.more_call_apps = clean_list(self.more_call_apps);
        self.never_call_apps = clean_list(self.never_call_apps);
        self.data_folder = self.data_folder.trim().to_string();
        self.microphone = crate::call::clip(self.microphone.trim(), 400);
        self.export.transcripts.dedup();
        self.export.transcripts.truncate(TranscriptFormat::ALL.len());
        if self.export.name.trim().is_empty() {
            self.export.name = ExportSettings::default().name;
        }
        self.export.name = crate::call::clip(&self.export.name, 200);
        self
    }

    /// Whether recording is on at `now_ms` (switched on and not paused).
    pub fn recording_at(&self, now_ms: u64) -> bool {
        self.recording && now_ms >= self.paused_until_ms
    }

    /// How long everything heard is kept, in ms.
    pub fn keep_audio_ms(&self) -> u64 {
        u64::from(self.keep_audio_hours.clamp(MIN_KEEP_HOURS, MAX_KEEP_HOURS)) * 3_600_000
    }

    /// How long calls are kept, in ms (never less than everything else).
    pub fn keep_calls_ms(&self) -> u64 {
        (u64::from(self.keep_calls_days.min(MAX_KEEP_DAYS)) * 86_400_000).max(self.keep_audio_ms())
    }

    /// The settings from a settings file's text (the defaults where it can't be read).
    pub fn from_json(text: &str) -> Settings {
        serde_json::from_str::<Settings>(text).unwrap_or_default().checked()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_fill_in_and_junk_is_clamped() {
        let s = Settings::from_json(
            r#"{"keep_audio_hours": 0, "replay_speed": 99, "more_call_apps": [" Foo.EXE ", "foo.exe", ""], "export": {"name": " "}}"#,
        );
        assert_eq!(s.keep_audio_hours, MIN_KEEP_HOURS);
        assert_eq!(s.replay_speed, 3.0);
        assert_eq!(s.more_call_apps, vec!["foo.exe"]);
        assert_eq!(s.export.name, "{date} {time} {title}");
        assert!(s.transcribe_calls, "a missing field takes its default");
        assert_eq!(Settings::from_json("not json"), Settings::default().checked());
        assert_eq!(Settings::from_json("{}").keep_audio_ms(), 48 * 3_600_000);
        let d = Settings { keep_calls_days: 0, ..Settings::default() };
        assert_eq!(d.keep_calls_ms(), d.keep_audio_ms(), "calls are never kept for less than everything");
    }

    #[test]
    fn the_recording_mode_reads_back_and_defaults_to_always() {
        assert_eq!(Settings::from_json("{}").record_mode, RecordMode::Always, "an older settings file records all the time");
        let s = Settings::from_json(r#"{"record_mode": "calls_only"}"#);
        assert_eq!(s.record_mode, RecordMode::CallsOnly);
        let back = Settings::from_json(&serde_json::to_string(&s).unwrap());
        assert_eq!(back.record_mode, RecordMode::CallsOnly);
        let odd = Settings::from_json(r#"{"record_mode": "sometimes", "keep_audio_hours": 12}"#);
        assert_eq!((odd.record_mode, odd.keep_audio_hours), (RecordMode::Always, 12), "a mode it doesn't know is Always; the rest stays");
    }

    #[test]
    fn pausing_stops_recording_until_then() {
        let s = Settings { paused_until_ms: 5000, ..Settings::default() };
        assert!(!s.recording_at(4999));
        assert!(s.recording_at(5000));
        let off = Settings { recording: false, ..Settings::default() };
        assert!(!off.recording_at(u64::MAX));
    }

    #[test]
    fn the_speech_model_reads_back_and_defaults_to_base() {
        for m in SpeechModel::ALL {
            let s = Settings { speech_model: m, ..Settings::default() };
            assert_eq!(Settings::from_json(&serde_json::to_string(&s).unwrap()).speech_model, m);
        }
        assert_eq!(Settings::default().speech_model, SpeechModel::Base);
        let odd = Settings::from_json(r#"{"speech_model": "huge", "keep_audio_hours": 12}"#);
        assert_eq!((odd.speech_model, odd.keep_audio_hours), (SpeechModel::Base, 12), "one it doesn't know is Base; the rest stays");
        assert_eq!(SpeechModel::Medium.threads(24), 4);
        assert_eq!(SpeechModel::Base.threads(24), 2);
        assert_eq!(SpeechModel::Small.threads(2), 1);
    }

    #[test]
    fn formats_by_name() {
        assert_eq!(TranscriptFormat::from_name("SRT"), Some(TranscriptFormat::Srt));
        assert_eq!(TranscriptFormat::from_name("text"), Some(TranscriptFormat::Text));
        assert_eq!(TranscriptFormat::from_name("doc"), None);
    }
}
