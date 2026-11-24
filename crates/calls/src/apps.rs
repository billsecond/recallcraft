//! The calling apps, by program file (lower case).

/// What kind of app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppKind {
    /// A meeting or chat app with calls.
    Calling,
    /// A browser: a meeting in a tab (or anything else that asked for the microphone).
    Browser,
    /// Another program the owner said is a calling app, or any program when they said so.
    Other,
}

/// A calling app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallApp {
    /// Its program file, lower case (`ms-teams.exe`).
    pub exe: String,
    /// As people call it.
    pub name: String,
    pub kind: AppKind,
}

/// The known ones: (program files, name).
const CALLING: &[(&[&str], &str)] = &[
    (&["ms-teams.exe", "msteams.exe"], "Microsoft Teams"),
    (&["teams.exe"], "Microsoft Teams (classic)"),
    (&["zoom.exe", "zoomworkplace.exe", "cpthost.exe"], "Zoom"),
    (&["slack.exe"], "Slack"),
    (&["discord.exe", "discordptb.exe", "discordcanary.exe"], "Discord"),
    (&["ciscocollabhost.exe", "webexmta.exe", "atmgr.exe", "webexhost.exe", "webex.exe", "ptoneclk.exe"], "Webex"),
    (&["skype.exe", "skypeapp.exe", "skypehost.exe"], "Skype"),
    (&["lync.exe"], "Skype for Business"),
    (&["whatsapp.exe", "whatsapp.root.exe"], "WhatsApp"),
    (&["signal.exe"], "Signal"),
    (&["telegram.exe"], "Telegram"),
    (&["ringcentral.exe", "rcmeetings.exe", "ringcentralmeetings.exe"], "RingCentral"),
    (&["g2mcomm.exe", "g2mstart.exe", "gotomeeting.exe", "goto.exe"], "GoTo"),
    (&["chime.exe"], "Amazon Chime"),
    (&["bluejeans.exe"], "BlueJeans"),
    (&["phoneexperiencehost.exe"], "Phone Link"),
    (&["messenger.exe"], "Messenger"),
    (&["element.exe"], "Element"),
    (&["mattermost.exe"], "Mattermost"),
    (&["dialpad.exe"], "Dialpad"),
    (&["aircall.exe"], "Aircall"),
    (&["zoiper.exe", "zoiper5.exe"], "Zoiper"),
    (&["microsip.exe"], "MicroSIP"),
    (&["tuple.exe"], "Tuple"),
    (&["facetime.exe"], "FaceTime"),
    (&["jabber.exe", "ciscojabber.exe"], "Cisco Jabber"),
    (&["8x8 work.exe", "8x8-work.exe"], "8x8 Work"),
    (&["vonage business.exe"], "Vonage"),
    (&["webcamoid.exe"], "Webcamoid"),
];

const BROWSERS: &[(&str, &str)] = &[
    ("chrome.exe", "Chrome"),
    ("msedge.exe", "Edge"),
    ("firefox.exe", "Firefox"),
    ("brave.exe", "Brave"),
    ("opera.exe", "Opera"),
    ("vivaldi.exe", "Vivaldi"),
    ("arc.exe", "Arc"),
    ("chromium.exe", "Chromium"),
    ("thorium.exe", "Thorium"),
    ("librewolf.exe", "LibreWolf"),
];

/// Programs that hold a microphone open for other programs, or for themselves, and are never a
/// call by themselves (a webview is looked through to the app that owns it).
pub const HOSTS: &[&str] = &["msedgewebview2.exe", "audiodg.exe", "svchost.exe", "runtimebroker.exe", "applicationframehost.exe"];

/// The app a program file is, by the rules: a known calling app, a browser (if browsers count),
/// one the owner added, or any program (if any counts). None for one that's never a call.
pub fn classify(exe: &str, browsers: bool, any_app: bool, more: &[String], never: &[String]) -> Option<CallApp> {
    let exe = exe.trim().to_ascii_lowercase();
    if exe.is_empty() || never.contains(&exe) || HOSTS.contains(&exe.as_str()) {
        return None;
    }
    if let Some((_, name)) = CALLING.iter().find(|(files, _)| files.contains(&exe.as_str())) {
        return Some(CallApp { exe, name: (*name).to_string(), kind: AppKind::Calling });
    }
    if let Some((_, name)) = BROWSERS.iter().find(|(file, _)| *file == exe) {
        return browsers.then(|| CallApp { exe, name: (*name).to_string(), kind: AppKind::Browser });
    }
    if more.contains(&exe) || any_app {
        let name = exe.trim_end_matches(".exe").to_string();
        return Some(CallApp { exe, name, kind: AppKind::Other });
    }
    None
}

/// Whether a program file is a known calling app or browser (looking up through parents stops
/// at one).
pub fn known(exe: &str) -> bool {
    let exe = exe.to_ascii_lowercase();
    CALLING.iter().any(|(files, _)| files.contains(&exe.as_str())) || BROWSERS.iter().any(|(f, _)| *f == exe)
}

/// Whether a program file is a browser.
pub fn is_browser(exe: &str) -> bool {
    let exe = exe.to_ascii_lowercase();
    BROWSERS.iter().any(|(f, _)| *f == exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apps_are_known_by_their_program() {
        let c = |e: &str| classify(e, true, false, &[], &[]).map(|a| a.name);
        assert_eq!(c("ms-teams.exe").as_deref(), Some("Microsoft Teams"));
        assert_eq!(c("Zoom.exe").as_deref(), Some("Zoom"));
        assert_eq!(c("slack.exe").as_deref(), Some("Slack"));
        assert_eq!(c("chrome.exe").as_deref(), Some("Chrome"));
        assert_eq!(c("obs64.exe"), None);
        assert_eq!(c("msedgewebview2.exe"), None, "looked through, never a call by itself");
        assert_eq!(classify("chrome.exe", false, false, &[], &[]), None, "browsers off");
        assert_eq!(classify("obs64.exe", true, true, &[], &[]).map(|a| a.kind), Some(AppKind::Other));
        assert_eq!(classify("myphone.exe", true, false, &["myphone.exe".into()], &[]).map(|a| a.name).as_deref(), Some("myphone"));
        assert_eq!(classify("zoom.exe", true, true, &[], &["zoom.exe".into()]), None, "never wins");
        assert!(known("Discord.exe") && known("firefox.exe") && !known("explorer.exe"));
        assert!(is_browser("MSEDGE.EXE") && !is_browser("slack.exe"));
    }
}
