//! Who's on the call and who's speaking, from what the meeting app's window tells screen readers
//! (UI Automation on Windows: [`crate::uia`]).
//!
//! Meeting apps describe their participants' tiles and lists for screen readers: "Jane Doe,
//! Speaking, Video on", "John Smith (Guest), Muted", "Jane Doe is speaking", "Active speaker:
//! Jane Doe". Their captions, when they're on, name who said each line. [`read`] takes the
//! window's elements (each with its depth, kind and name) and finds:
//! - **speaking**: the people the window marks as speaking just now;
//! - **people**: everyone it names as on the call;
//! - **captions**: the captions on screen, each with who said it;
//! - **own**: the names it marks as the owner ("(You)").
//!
//! [`Timeline`] turns the moments someone was seen speaking into stretches of time.
//!
//! English words only, for now. Plain logic, tested without a window.

use std::collections::BTreeMap;

/// One element of the meeting window, as UI Automation describes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Element {
    /// How deep in the window's tree (the window is 0).
    pub depth: u16,
    /// Its control type ("Button", "Text", "ListItem", …).
    pub kind: String,
    /// What a screen reader says for it.
    pub name: String,
    /// Its automation id, if it has one.
    pub id: String,
}

/// What the meeting window says about the people on the call, at one moment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    pub speaking: Vec<String>,
    pub people: Vec<String>,
    /// (who, what they said), in the order shown; the last is the newest.
    pub captions: Vec<(String, String)>,
    pub own: Vec<String>,
}

/// The most elements looked at, and the longest name.
pub const MAX_ELEMENTS: usize = 8000;
const MAX_NAME_TEXT: usize = 400;

/// Words that, after a comma, mark the one before it as speaking.
const SPEAKING: &[&str] = &["speaking", "is speaking", "currently speaking", "talking", "is talking", "active speaker", "now speaking"];

/// Words that, after a comma or in brackets, mark a participant: their microphone, camera, role.
const STATUS: &[&str] = &[
    "muted",
    "unmuted",
    "mic muted",
    "mic unmuted",
    "mic on",
    "mic off",
    "microphone muted",
    "microphone on",
    "microphone off",
    "audio muted",
    "audio unmuted",
    "audio on",
    "audio off",
    "video on",
    "video off",
    "video is on",
    "video is off",
    "camera on",
    "camera off",
    "camera is on",
    "camera is off",
    "host",
    "co-host",
    "cohost",
    "organizer",
    "organiser",
    "presenter",
    "attendee",
    "participant",
    "guest",
    "external",
    "unverified",
    "hand raised",
    "raised hand",
    "pinned",
    "spotlighted",
    "in this meeting",
    "you",
    "me",
];

/// Short texts that are the app's own words, never a person.
const NOT_PEOPLE: &[&str] = &[
    "about",
    "activity",
    "add people",
    "admit",
    "apps",
    "audio",
    "back",
    "breakout rooms",
    "calendar",
    "call",
    "calls",
    "camera",
    "captions",
    "chat",
    "chats",
    "close",
    "copilot",
    "copy link",
    "devices",
    "end",
    "end call",
    "everyone",
    "files",
    "forward",
    "gallery",
    "hang up",
    "help",
    "home",
    "invite",
    "join",
    "leave",
    "live captions",
    "maximize",
    "meet",
    "meet now",
    "meeting",
    "meeting chat",
    "meeting notes",
    "mic",
    "microphone",
    "minimize",
    "more",
    "more actions",
    "more options",
    "mute",
    "mute all",
    "new chat",
    "notes",
    "people",
    "participants",
    "raise",
    "raise hand",
    "react",
    "reactions",
    "record",
    "recording",
    "restore",
    "rooms",
    "search",
    "security",
    "settings",
    "share",
    "share content",
    "share screen",
    "speaker",
    "speaker view",
    "speakers",
    "speaking",
    "stop video",
    "start video",
    "subtitles",
    "summary",
    "teams",
    "microsoft teams",
    "transcript",
    "unmute",
    "video",
    "view",
    "waiting",
    "waiting room",
    "whiteboard",
    "zoom",
    "zoom meeting",
    "google meet",
    "webex",
    "slack",
    "huddle",
    "discord",
];

fn plain(s: &str) -> String {
    let s: String = s.chars().take(MAX_NAME_TEXT).map(|c| if c.is_control() { ' ' } else { c }).collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A bracketed word at the end, and the rest: "Jane Doe (Guest)" is ("Jane Doe", "guest").
fn bracket(s: &str) -> (&str, Option<String>) {
    let t = s.trim_end();
    if let Some(rest) = t.strip_suffix(')')
        && let Some(open) = rest.rfind('(')
    {
        let inner = rest.get(open + 1..).unwrap_or("").trim().to_lowercase();
        let before = rest.get(..open).unwrap_or("").trim_end();
        if !before.is_empty() && inner.len() <= 24 {
            return (before, Some(inner));
        }
    }
    (t, None)
}

/// The person a short text names, without what's bracketed after it; None for anything that
/// doesn't look like a name (a sentence, a button, a number, an address).
pub fn person_name(text: &str) -> Option<String> {
    let mut t = plain(text);
    // "Jane Doe (Guest) (You)": every bracket at the end goes.
    for _ in 0..3 {
        let (before, inner) = bracket(&t);
        if inner.is_none() {
            break;
        }
        t = before.to_string();
    }
    let t = t.trim().trim_end_matches(',').trim();
    let words: Vec<&str> = t.split(' ').filter(|w| !w.is_empty()).collect();
    if words.is_empty() || words.len() > 5 || t.chars().count() < 2 || t.chars().count() > 60 {
        return None;
    }
    if t.chars().any(|c| c.is_ascii_digit() || matches!(c, '@' | ':' | '/' | '\\' | '?' | '!' | '#' | '|' | '"' | '“' | '”')) {
        return None;
    }
    if t.ends_with('.') && !(words.len() > 1 && words.last().is_some_and(|w| w.chars().count() <= 3)) {
        // A sentence ("Yes."), not "Jane Doe Jr."
        return None;
    }
    let first = words.first().and_then(|w| w.chars().next())?;
    if !first.is_alphabetic() || first.is_lowercase() {
        return None;
    }
    if words.iter().any(|w| !w.chars().next().is_some_and(char::is_alphabetic)) {
        return None;
    }
    let low = t.to_lowercase();
    if NOT_PEOPLE.contains(&low.as_str()) || STATUS.contains(&low.as_str()) || SPEAKING.contains(&low.as_str()) {
        return None;
    }
    Some(t.to_string())
}

/// What one element's name says: the person, whether they're speaking, whether it's the owner,
/// and whether it describes a participant at all.
#[derive(Debug, Default, PartialEq, Eq)]
struct Said {
    person: String,
    speaking: bool,
    own: bool,
}

fn said(name: &str) -> Option<Said> {
    let name = plain(name);
    if name.is_empty() {
        return None;
    }
    let low = name.to_lowercase();
    // "Speaking: Jane Doe", "Active speaker: Jane Doe".
    for prefix in ["speaking:", "now speaking:", "active speaker:", "talking:"] {
        if let Some(rest) = low.strip_prefix(prefix) {
            let rest_at = name.len() - rest.len();
            let who = name.get(rest_at..).unwrap_or("");
            let who = who.split(',').next().unwrap_or("");
            return person_name(who).map(|person| Said { person, speaking: true, own: false });
        }
    }
    // "Jane Doe is speaking", "Jane Doe is talking".
    for suffix in [" is speaking", " is talking", " is currently speaking"] {
        if let Some(before) = low.strip_suffix(suffix) {
            let who = name.get(..before.len()).unwrap_or("");
            return person_name(who).map(|person| Said { person, speaking: true, own: false });
        }
    }
    // "Jane Doe, Speaking, Video on", "John Smith (Guest), Muted".
    let mut parts = name.split(',').map(str::trim).filter(|p| !p.is_empty());
    let head = parts.next()?;
    let (core, mark) = bracket(head);
    let mut s = Said::default();
    let mut participant = false;
    for m in mark.iter().map(String::as_str).chain(bracket(core).1.as_deref()) {
        participant |= STATUS.contains(&m) || m == "you";
        s.own |= m == "you" || m == "me";
    }
    for p in parts {
        let p = p.to_lowercase();
        let p = p.trim_end_matches('.');
        if SPEAKING.contains(&p) {
            s.speaking = true;
            participant = true;
        } else if STATUS.contains(&p) || p.starts_with("muted") || p.starts_with("unmuted") {
            participant = true;
            s.own |= p == "you" || p == "me";
        }
    }
    if !participant {
        return None;
    }
    s.person = person_name(head)?;
    Some(s)
}

/// "Jane Doe: what she said", as some apps' captions are.
fn caption_line(text: &str) -> Option<(String, String)> {
    let t = plain(text);
    let (who, what) = t.split_once(": ")?;
    let what = what.trim();
    (!what.is_empty()).then_some(())?;
    Some((person_name(who)?, what.to_string()))
}

fn push_unique(list: &mut Vec<String>, name: &str) {
    if list.len() < 200 && !list.iter().any(|n| n.eq_ignore_ascii_case(name)) {
        list.push(name.to_string());
    }
}

/// What the window's elements say about the people on the call.
pub fn read(elements: &[Element]) -> Seen {
    let mut seen = Seen::default();
    let mut in_captions: Option<u16> = None;
    // A caption's author, waiting for the text after it.
    let mut author: Option<String> = None;
    for e in elements.iter().take(MAX_ELEMENTS) {
        if in_captions.is_some_and(|d| e.depth <= d) {
            in_captions = None;
            author = None;
        }
        let name = plain(&e.name);
        let low = name.to_lowercase();
        if in_captions.is_none()
            && (low.contains("caption") || low.contains("subtitle"))
            && !matches!(e.kind.as_str(), "Button" | "MenuItem" | "CheckBox")
        {
            // The captions' area: what's inside it is captions, not tiles.
            in_captions = Some(e.depth);
            continue;
        }
        if in_captions.is_some() {
            if name.is_empty() {
                continue;
            }
            // An author, then what they said; or "Author: what they said" in one.
            if let Some(who) = author.take() {
                seen.captions.push((who, name));
            } else if let Some(c) = caption_line(&name) {
                seen.captions.push(c);
            } else if let Some(who) = person_name(&name) {
                author = Some(who);
            }
            continue;
        }
        if let Some(s) = said(&name) {
            push_unique(&mut seen.people, &s.person);
            if s.own {
                push_unique(&mut seen.own, &s.person);
            }
            if s.speaking {
                push_unique(&mut seen.speaking, &s.person);
            }
        }
    }
    // Only the newest captions matter.
    let drop = seen.captions.len().saturating_sub(20);
    seen.captions.drain(..drop);
    for (who, _) in seen.captions.clone() {
        push_unique(&mut seen.people, &who);
    }
    seen
}

/// A stretch of time someone was seen speaking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stretch {
    pub name: String,
    pub from_ms: u64,
    pub to_ms: u64,
}

/// The moments people were seen speaking, made into stretches: one goes on while they're seen
/// again within `gap_ms`, and ends a little after they were last seen.
#[derive(Debug, Default)]
pub struct Timeline {
    open: BTreeMap<String, (u64, u64)>,
    closed: Vec<Stretch>,
}

impl Timeline {
    /// At `at_ms`, these were seen speaking; `poll_ms` is how often the window is looked at.
    pub fn observe(&mut self, at_ms: u64, speaking: &[String], poll_ms: u64) {
        for name in speaking.iter().take(50) {
            let e = self.open.entry(name.clone()).or_insert((at_ms, at_ms));
            e.1 = e.1.max(at_ms);
        }
        let gap = poll_ms.saturating_mul(3).max(1500);
        let ended: Vec<String> =
            self.open.iter().filter(|(n, (_, last))| !speaking.contains(n) && at_ms.saturating_sub(*last) > gap).map(|(n, _)| n.clone()).collect();
        for n in ended {
            if let Some((from, last)) = self.open.remove(&n) {
                self.closed.push(Stretch { name: n, from_ms: from, to_ms: last.saturating_add(poll_ms / 2) });
            }
        }
    }

    /// The stretches that ended since the last take.
    pub fn take(&mut self) -> Vec<Stretch> {
        std::mem::take(&mut self.closed)
    }

    /// The stretches still going on at `at_ms`, as they are so far.
    pub fn open(&self, at_ms: u64) -> Vec<Stretch> {
        self.open
            .iter()
            .map(|(n, (from, last))| Stretch { name: n.clone(), from_ms: *from, to_ms: (*last).max(at_ms.min(last.saturating_add(1000))) })
            .collect()
    }

    /// The call ended: everything still going on ends.
    pub fn end(&mut self, poll_ms: u64) -> Vec<Stretch> {
        for (n, (from, last)) in std::mem::take(&mut self.open) {
            self.closed.push(Stretch { name: n, from_ms: from, to_ms: last.saturating_add(poll_ms / 2) });
        }
        self.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(depth: u16, kind: &str, name: &str) -> Element {
        Element { depth, kind: kind.to_string(), name: name.to_string(), id: String::new() }
    }

    #[test]
    fn names_are_people_and_buttons_are_not() {
        assert_eq!(person_name("Jane Doe").as_deref(), Some("Jane Doe"));
        assert_eq!(person_name("  John   Smith (Guest) ").as_deref(), Some("John Smith"));
        assert_eq!(person_name("Ann O'Neil-Brown (External) (You)").as_deref(), Some("Ann O'Neil-Brown"));
        assert_eq!(person_name("Ludwig van Beethoven").as_deref(), Some("Ludwig van Beethoven"));
        assert_eq!(person_name("José Álvarez").as_deref(), Some("José Álvarez"));
        assert_eq!(person_name("山田 太郎").as_deref(), Some("山田 太郎"));
        for not in [
            "Mute",
            "Leave",
            "Live captions",
            "Share",
            "More actions",
            "Microsoft Teams",
            "jane@contoso.com",
            "Meeting 3",
            "Hi everyone, can you hear me?",
            "I think we should ship it on Monday and see what happens.",
            "",
            "x",
            "lowercase name",
            "10:32",
            "Yes.",
            "Okay.",
        ] {
            assert_eq!(person_name(not), None, "{not}");
        }
    }

    #[test]
    fn tiles_say_who_is_on_the_call_and_who_is_speaking() {
        let window = [
            el(0, "Window", "Meeting with Jane | Microsoft Teams"),
            el(3, "Button", "Mute"),
            el(5, "Group", "Jane Doe, Speaking, Video on"),
            el(5, "Group", "John Smith (Guest), Muted, Video off"),
            el(5, "Group", "William Daugherty (You), Unmuted"),
            el(6, "Text", "Ann Lee is speaking"),
            el(6, "Text", "Active speaker: Bob Ray"),
            el(6, "Button", "Share"),
            // A chat message that happens to mention a name isn't a tile.
            el(7, "Text", "Jane Doe, could you share the deck"),
        ];
        let s = read(&window);
        assert_eq!(s.speaking, vec!["Jane Doe", "Ann Lee", "Bob Ray"]);
        assert_eq!(s.people, vec!["Jane Doe", "John Smith", "William Daugherty", "Ann Lee", "Bob Ray"]);
        assert_eq!(s.own, vec!["William Daugherty"]);
        assert!(s.captions.is_empty());
    }

    #[test]
    fn captions_say_who_said_each_line() {
        let window = [
            el(0, "Window", "Zoom Meeting"),
            el(2, "Group", "Live Captions"),
            el(3, "ListItem", ""),
            el(4, "Text", "Jane Doe"),
            el(4, "Text", "So the numbers came in lower than we hoped"),
            el(3, "ListItem", ""),
            el(4, "Text", "John Smith: Did we include the refunds?"),
            el(2, "Button", "Leave"),
            el(2, "Text", "Jane Doe"),
        ];
        let s = read(&window);
        assert_eq!(
            s.captions,
            vec![
                ("Jane Doe".to_string(), "So the numbers came in lower than we hoped".to_string()),
                ("John Smith".to_string(), "Did we include the refunds?".to_string())
            ]
        );
        assert_eq!(s.people, vec!["Jane Doe", "John Smith"]);
        assert!(s.speaking.is_empty(), "captions alone don't say who's speaking now");
    }

    #[test]
    fn junk_is_safe() {
        let long = "A".repeat(100_000);
        let window = vec![el(0, "Window", &long); 20_000];
        let s = read(&window);
        assert!(s.people.is_empty());
        assert_eq!(read(&[]), Seen::default());
        assert_eq!(person_name("\u{0}\u{1}"), None);
        assert_eq!(said("(Guest)"), None);
        assert_eq!(said(", , ,"), None);
    }

    #[test]
    fn moments_become_stretches() {
        let mut t = Timeline::default();
        let jane = vec!["Jane Doe".to_string()];
        t.observe(10_000, &jane, 500);
        t.observe(10_500, &jane, 500);
        t.observe(11_000, &jane, 500);
        // Missed once (a slow look): still the same stretch.
        t.observe(12_000, &jane, 500);
        assert!(t.take().is_empty());
        assert_eq!(t.open(12_200), vec![Stretch { name: "Jane Doe".into(), from_ms: 10_000, to_ms: 12_200 }]);
        // Quiet for a while: it ended a little after she was last seen.
        t.observe(12_500, &[], 500);
        t.observe(14_000, &[], 500);
        assert_eq!(t.take(), vec![Stretch { name: "Jane Doe".into(), from_ms: 10_000, to_ms: 12_250 }]);
        t.observe(20_000, &jane, 500);
        assert_eq!(t.end(500), vec![Stretch { name: "Jane Doe".into(), from_ms: 20_000, to_ms: 20_250 }]);
        assert!(t.open(30_000).is_empty());
    }
}
