//! The meeting's title from the app's windows, and the other person's name when it says it.
//!
//! The meeting window is the one that appeared when the call started (an app's main window was
//! there before); its title is cleaned of the app's name. A title like "Call with Ann Lee", or
//! one that is just a person's name (a one-to-one call), names the other person.

/// What browsers and apps put after a window's title.
const SUFFIXES: &[&str] = &[
    " | Microsoft Teams",
    " - Microsoft Teams",
    " | Microsoft Teams (work or school)",
    " | Microsoft Teams (free)",
    " - Zoom",
    " - Slack",
    " | Slack",
    " - Discord",
    " - Webex",
    " - Google Chrome",
    " - Microsoft\u{200b} Edge",
    " - Microsoft Edge",
    " — Mozilla Firefox",
    " - Mozilla Firefox",
    " - Brave",
    " - Opera",
    " - Vivaldi",
    " - Chromium",
];

/// Window titles that say nothing about the meeting.
const GENERIC: &[&str] = &[
    "",
    "microsoft teams",
    "microsoft teams (work or school)",
    "zoom",
    "zoom meeting",
    "zoom workplace",
    "zoom cloud meetings",
    "slack",
    "discord",
    "webex",
    "cisco webex meetings",
    "skype",
    "whatsapp",
    "signal",
    "telegram",
    "phone link",
    "meeting",
    "meeting controls",
    "meeting compact view",
    "sharing control bar",
    "screen share",
    "new tab",
    "settings",
    "chat",
    "activity",
    "calendar",
    "calls",
    "teams",
    "files",
    "notification",
    "notifications",
    "msctfime ui",
    "default ime",
    "gdi+ window",
    "hidden window",
];

/// The parts of a Teams main window's title: `Chat | Contoso | ann@contoso.com`.
const SECTIONS: &[&str] = &["chat", "activity", "calendar", "teams", "calls", "files", "apps", "onedrive", "copilot", "communities", "assignments"];

/// A window title cleaned of the app's and browser's names; None if it says nothing.
pub fn clean(title: &str) -> Option<String> {
    let mut t: String = title.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string();
    // A browser profile: "Meet - abc - Work - Google Chrome".
    for _ in 0..3 {
        let before = t.clone();
        for s in SUFFIXES {
            if let Some(rest) = t.strip_suffix(s) {
                t = rest.trim().to_string();
            }
        }
        if t == before {
            break;
        }
    }
    // Teams' main window: its first part is one of its sections.
    if let Some((first, _)) = t.split_once(" | ")
        && SECTIONS.contains(&first.trim().to_lowercase().as_str())
    {
        return None;
    }
    // Zoom's title for a meeting has the meeting's name after "Zoom Meeting - " at times.
    for p in ["Zoom Meeting - ", "Zoom - ", "Meet - ", "Meet – "] {
        if let Some(rest) = t.strip_prefix(p) {
            t = rest.trim().to_string();
        }
    }
    if GENERIC.contains(&t.to_lowercase().as_str()) || t.len() < 2 || !t.chars().any(char::is_alphanumeric) {
        return None;
    }
    Some(t.chars().take(160).collect())
}

/// The meeting's title: of the app's windows during the call (`during`), one that wasn't there
/// before it (`before`) first, then any that says something.
pub fn pick(before: &[String], during: &[String]) -> Option<String> {
    let new = during.iter().filter(|t| !before.contains(t)).find_map(|t| clean(t));
    new.or_else(|| during.iter().find_map(|t| clean(t)))
}

/// A meeting in a browser tab, by the window's title (the tab showing): the meeting service's
/// name. None for a tab that isn't a known meeting service.
pub fn browser_app(title: &str) -> Option<&'static str> {
    let t = title.to_lowercase();
    let services: [(&str, &str); 10] = [
        ("meet.google", "Google Meet"),
        ("meet -", "Google Meet"),
        ("meet –", "Google Meet"),
        ("google meet", "Google Meet"),
        ("microsoft teams", "Microsoft Teams"),
        ("zoom", "Zoom"),
        ("webex", "Webex"),
        ("jitsi", "Jitsi Meet"),
        ("whereby", "Whereby"),
        ("discord", "Discord"),
    ];
    services.iter().find(|(k, _)| t.starts_with(k) || t.contains(&format!(" {k}")) || t.contains(&format!("| {k}"))).map(|(_, n)| *n)
}

/// Words a meeting's title has that a person's name doesn't.
const MEETING_WORDS: &[&str] = &[
    "meeting",
    "sync",
    "standup",
    "stand-up",
    "review",
    "call",
    "weekly",
    "daily",
    "team",
    "1:1",
    "huddle",
    "planning",
    "retro",
    "demo",
    "interview",
    "chat",
    "update",
    "check-in",
    "catch",
    "project",
    "sprint",
    "kickoff",
    "all-hands",
    "webinar",
    "training",
    "session",
    "workshop",
    "office",
    "hours",
    "general",
    "random",
    "channel",
    "meet",
    "zoom",
    "teams",
    "slack",
    "discord",
    "with",
    "and",
    "the",
    "for",
    "&",
];

/// Whether some text looks like a person's name: two or three words, each starting with a
/// capital, letters (and `'-.`) only.
pub fn looks_like_a_name(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    if !(2..=4).contains(&words.len()) || text.len() > 48 {
        return false;
    }
    words.iter().all(|w| {
        let lower = w.to_lowercase();
        w.chars().next().is_some_and(char::is_uppercase)
            && w.chars().all(|c| c.is_alphabetic() || "'-.".contains(c))
            && !MEETING_WORDS.contains(&lower.as_str())
    })
}

/// The other person's name, when a one-to-one call's title says it.
pub fn person(title: &str) -> Option<String> {
    let t = title.trim();
    for p in [
        "Call with ",
        "call with ",
        "Meeting with ",
        "Huddle with ",
        "Chat with ",
        "Video call with ",
        "Audio call with ",
        "Calling ",
        "In a call with ",
    ] {
        if let Some(rest) = t.strip_prefix(p) {
            let name = rest.split([',', '|', '(']).next().unwrap_or_default().trim();
            return looks_like_a_name(name)
                .then(|| name.to_string())
                .or_else(|| (name.split_whitespace().count() == 1 && name.len() > 1).then(|| name.to_string()));
        }
    }
    looks_like_a_name(t).then(|| t.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_cleaned_and_generic_ones_dropped() {
        assert_eq!(clean("Weekly sync | Microsoft Teams").as_deref(), Some("Weekly sync"));
        assert_eq!(clean("Chat | Contoso | ann@contoso.com | Microsoft Teams"), None);
        assert_eq!(clean("Microsoft Teams"), None);
        assert_eq!(clean("Zoom Meeting"), None);
        assert_eq!(clean("Zoom Meeting - Budget review").as_deref(), Some("Budget review"));
        assert_eq!(clean("Meet - abc-defg-hij - Work - Google Chrome").as_deref(), Some("abc-defg-hij - Work"));
        assert_eq!(clean("Huddle: #design - Slack").as_deref(), Some("Huddle: #design"));
        assert_eq!(clean("  \u{1}  "), None);
    }

    #[test]
    fn the_meeting_window_is_the_new_one() {
        let before = vec!["Chat | Contoso | Microsoft Teams".to_string(), "Planning | Microsoft Teams".to_string()];
        let during = vec!["Planning | Microsoft Teams".to_string(), "Budget review | Microsoft Teams".to_string()];
        assert_eq!(pick(&before, &during).as_deref(), Some("Budget review"));
        assert_eq!(pick(&during, &during).as_deref(), Some("Planning"));
        assert_eq!(pick(&[], &["Microsoft Teams".into()]), None);
    }

    #[test]
    fn meetings_in_a_browser_are_known_by_their_service() {
        assert_eq!(browser_app("Meet - abc-defg-hij - Google Chrome"), Some("Google Meet"));
        assert_eq!(browser_app("Weekly sync | Microsoft Teams - Microsoft Edge"), Some("Microsoft Teams"));
        assert_eq!(browser_app("Zoom Meeting - Google Chrome"), Some("Zoom"));
        assert_eq!(browser_app("Inbox (3) - Gmail - Google Chrome"), None);
        assert_eq!(browser_app("Amazoom shop"), None);
    }

    #[test]
    fn a_person_is_named_by_a_one_to_one_title() {
        assert_eq!(person("Call with John Smith").as_deref(), Some("John Smith"));
        assert_eq!(person("Huddle with Priya").as_deref(), Some("Priya"));
        assert_eq!(person("Ann Lee").as_deref(), Some("Ann Lee"));
        assert_eq!(person("Mary-Jane O'Neil").as_deref(), Some("Mary-Jane O'Neil"));
        assert_eq!(person("Weekly Sync"), None);
        assert_eq!(person("Budget review"), None, "not capitalised");
        assert_eq!(person("Design Team Standup"), None);
        assert_eq!(person("John"), None, "one word alone could be anything");
        assert_eq!(person("Q3 Planning"), None);
        assert_eq!(person(""), None);
    }
}
