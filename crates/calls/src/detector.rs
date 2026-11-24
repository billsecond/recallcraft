//! When a call starts and ends, from which programs have a microphone open, looked at every
//! second or so.
//!
//! A call starts when a calling app has had a microphone open for [`START_AFTER_MS`] (an app
//! checking its devices for a moment isn't a call); it's taken to have started a moment before
//! ([`PRE_ROLL_MS`]: the recording has that sound anyway). It ends when no calling app has had
//! one open for [`END_AFTER_MS`] (rejoining, or changing headset, doesn't end it).

use crate::apps::{self, CallApp};

pub const START_AFTER_MS: u64 = 1500;
pub const PRE_ROLL_MS: u64 = 3000;
pub const END_AFTER_MS: u64 = 15_000;

/// A program with a microphone open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capturer {
    pub pid: u32,
    /// Its program file, lower case.
    pub exe: String,
    /// The app it belongs to: itself, or the calling app it was started by (a webview's).
    pub app_exe: String,
    /// The microphone's device id.
    pub device_id: String,
}

/// What was seen at a moment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Observation {
    pub at_ms: u64,
    pub capturing: Vec<Capturer>,
}

/// Which programs are calls.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rules {
    pub browsers: bool,
    pub any_app: bool,
    pub more: Vec<String>,
    pub never: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A call started (at `at_ms`), in `app`, on the microphone `device_id`.
    Started {
        app: CallApp,
        at_ms: u64,
        device_id: String,
    },
    /// The call's app moved to another microphone.
    Device {
        device_id: String,
    },
    Ended {
        at_ms: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    Idle,
    Starting { app: CallApp, since_ms: u64 },
    InCall { app: CallApp, last_seen_ms: u64, device_id: String },
}

#[derive(Clone, Debug)]
pub struct Detector {
    state: State,
    pub rules: Rules,
}

impl Detector {
    pub fn new(rules: Rules) -> Detector {
        Detector { state: State::Idle, rules }
    }

    pub fn in_call(&self) -> bool {
        matches!(self.state, State::InCall { .. })
    }

    /// The app of the call going on.
    pub fn app(&self) -> Option<&CallApp> {
        match &self.state {
            State::InCall { app, .. } => Some(app),
            _ => None,
        }
    }

    /// The microphone of the call going on.
    pub fn device(&self) -> Option<&str> {
        match &self.state {
            State::InCall { device_id, .. } => Some(device_id),
            _ => None,
        }
    }

    /// The calling app with a microphone open, and the device: the one the call is in first,
    /// then a calling app before a browser before anything else.
    fn calling(&self, obs: &Observation) -> Option<(CallApp, String)> {
        let r = &self.rules;
        let mut found: Vec<(CallApp, String)> = obs
            .capturing
            .iter()
            .filter_map(|c| {
                let app = apps::classify(&c.app_exe, r.browsers, r.any_app, &r.more, &r.never)
                    .or_else(|| apps::classify(&c.exe, r.browsers, r.any_app, &r.more, &r.never))?;
                Some((app, c.device_id.clone()))
            })
            .collect();
        let current = self.app().map(|a| a.exe.clone());
        found.sort_by_key(|(a, _)| (Some(&a.exe) != current.as_ref(), a.kind as u8));
        found.into_iter().next()
    }

    pub fn observe(&mut self, obs: &Observation) -> Vec<Event> {
        let now = obs.at_ms;
        let calling = self.calling(obs);
        let mut events = Vec::new();
        let state = std::mem::replace(&mut self.state, State::Idle);
        self.state = match (state, calling) {
            (State::Idle, None) => State::Idle,
            (State::Idle, Some((app, _))) => State::Starting { app, since_ms: now },
            (State::Starting { .. }, None) => State::Idle,
            (State::Starting { app, since_ms }, Some((seen, device_id))) => {
                if seen.exe != app.exe {
                    State::Starting { app: seen, since_ms: now }
                } else if now.saturating_sub(since_ms) >= START_AFTER_MS {
                    events.push(Event::Started { app: app.clone(), at_ms: since_ms.saturating_sub(PRE_ROLL_MS), device_id: device_id.clone() });
                    State::InCall { app, last_seen_ms: now, device_id }
                } else {
                    State::Starting { app, since_ms }
                }
            }
            (State::InCall { app, last_seen_ms, device_id }, None) => {
                if now.saturating_sub(last_seen_ms) >= END_AFTER_MS {
                    events.push(Event::Ended { at_ms: last_seen_ms + 1000 });
                    State::Idle
                } else {
                    State::InCall { app, last_seen_ms, device_id }
                }
            }
            (State::InCall { app, device_id, .. }, Some((_, seen_device))) => {
                if seen_device != device_id && !seen_device.is_empty() {
                    events.push(Event::Device { device_id: seen_device.clone() });
                    State::InCall { app, last_seen_ms: now, device_id: seen_device }
                } else {
                    State::InCall { app, last_seen_ms: now, device_id }
                }
            }
        };
        events
    }

    /// Ends the call going on now (the service is stopping, or recording was paused).
    pub fn end_now(&mut self, at_ms: u64) -> Option<Event> {
        let was = std::mem::replace(&mut self.state, State::Idle);
        matches!(was, State::InCall { .. }).then_some(Event::Ended { at_ms })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(at_ms: u64, who: &[(&str, &str, &str)]) -> Observation {
        Observation {
            at_ms,
            capturing: who
                .iter()
                .enumerate()
                .map(|(i, (exe, app, dev))| Capturer {
                    pid: i as u32 + 10,
                    exe: exe.to_string(),
                    app_exe: app.to_string(),
                    device_id: dev.to_string(),
                })
                .collect(),
        }
    }

    fn rules() -> Rules {
        Rules { browsers: true, ..Rules::default() }
    }

    #[test]
    fn a_call_starts_after_a_moment_and_ends_after_a_while() {
        let mut d = Detector::new(rules());
        let recallcraft = ("recallcraft.exe", "recallcraft.exe", "mic-a");
        let teams = ("msedgewebview2.exe", "ms-teams.exe", "mic-a");
        assert!(d.observe(&obs(0, &[recallcraft])).is_empty(), "RecallCraft itself is never a call");
        assert!(d.observe(&obs(1000, &[recallcraft, teams])).is_empty());
        let started = d.observe(&obs(2000, &[recallcraft, teams]));
        assert!(started.is_empty(), "not yet: a second only");
        let started = d.observe(&obs(3000, &[recallcraft, teams]));
        match started.as_slice() {
            [Event::Started { app, at_ms, device_id }] => {
                assert_eq!(app.name, "Microsoft Teams");
                assert_eq!(*at_ms, 1000 - PRE_ROLL_MS.min(1000));
                assert_eq!(device_id, "mic-a");
            }
            other => panic!("{other:?}"),
        }
        assert!(d.in_call());
        // A headset plugged in: the call goes on, on the new microphone.
        let teams_b = ("msedgewebview2.exe", "ms-teams.exe", "mic-b");
        assert_eq!(d.observe(&obs(60_000, &[teams_b])), vec![Event::Device { device_id: "mic-b".into() }]);
        // A few seconds without the microphone (rejoining) don't end it.
        assert!(d.observe(&obs(65_000, &[recallcraft])).is_empty());
        assert!(d.observe(&obs(70_000, &[teams_b])).is_empty());
        // Gone for good.
        assert!(d.observe(&obs(75_000, &[])).is_empty());
        assert_eq!(d.observe(&obs(85_000, &[])), vec![Event::Ended { at_ms: 71_000 }]);
        assert!(!d.in_call());
    }

    #[test]
    fn a_moment_s_device_check_isnt_a_call_and_rules_count() {
        let mut d = Detector::new(rules());
        let zoom = ("zoom.exe", "zoom.exe", "m");
        assert!(d.observe(&obs(0, &[zoom])).is_empty());
        assert!(d.observe(&obs(1000, &[])).is_empty());
        assert!(d.observe(&obs(5000, &[])).is_empty());
        assert!(!d.in_call());
        // Browsers don't count when the owner says so; any app does when they say that.
        let mut d = Detector::new(Rules::default());
        let chrome = ("chrome.exe", "chrome.exe", "m");
        for t in [0, 2000, 4000] {
            assert!(d.observe(&obs(t, &[chrome])).is_empty());
        }
        let mut d = Detector::new(Rules { any_app: true, never: vec!["recallcraft.exe".into()], ..Rules::default() });
        let obs_app = ("obs64.exe", "obs64.exe", "m");
        d.observe(&obs(0, &[obs_app]));
        assert!(matches!(d.observe(&obs(2000, &[obs_app])).as_slice(), [Event::Started { .. }]));
        assert_eq!(d.end_now(3000), Some(Event::Ended { at_ms: 3000 }));
        assert_eq!(d.end_now(4000), None);
    }

    #[test]
    fn a_calling_app_is_chosen_before_a_browser() {
        let mut d = Detector::new(rules());
        let both = [("chrome.exe", "chrome.exe", "m"), ("slack.exe", "slack.exe", "m")];
        d.observe(&obs(0, &both));
        let e = d.observe(&obs(2000, &both));
        assert!(matches!(e.as_slice(), [Event::Started { app, .. }] if app.name == "Slack"), "{e:?}");
    }
}
