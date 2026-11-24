//! Names from the meeting app: during a call, its window is read as a screen reader reads it
//! ([`recallcraft_calls::uia`]), about twice a second, for who it shows speaking and who's on
//! the call ([`recallcraft_calls::roster`]). The stretches each person was shown speaking go to
//! the call's `speaking.jsonl`, the people to the call; and a voice whose lines keep coming while
//! the same person is shown speaking is named after them ([`voices_named`]).
//!
//! Only the meeting app's own windows are read (the one that appeared with the call, when
//! there is one), nothing in them is changed, and the owner is never one of the names. A name
//! the owner gave, or the call's title gave, is never replaced. Off with
//! [`Settings::names_from_app`](recallcraft_model::Settings).

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use recallcraft_calls::roster::{self, Stretch, Timeline};
use recallcraft_calls::{probe, titles, uia};
use recallcraft_model::{Line, Speaker, Spoken, time};
use recallcraft_store::Layout;
use recallcraft_store::calls::CallStore;

use crate::Shared;

/// What the devices loop tells this thread.
#[derive(Debug)]
pub enum ToNames {
    /// A call started in `app_exe`, whose windows were titled `before` just before it.
    Started {
        id: String,
        app_exe: String,
        before: Vec<String>,
    },
    Ended,
}

/// How often what was found is saved, and the voices named.
const SAVE_EVERY: Duration = Duration::from_secs(5);
/// Captions show what was said a moment after it was said.
const CAPTION_LAG_MS: u64 = 1000;

struct Current {
    id: String,
    app_exe: String,
    before: Vec<String>,
    pids: Vec<u32>,
    pids_at: Option<Instant>,
    own: Vec<String>,
    tiles: Timeline,
    captions: Timeline,
    last_caption: Option<(String, String)>,
    people: Vec<String>,
    saved_at: Instant,
    transcript_len: u64,
}

fn stretches(s: Vec<Stretch>) -> Vec<Spoken> {
    s.into_iter().map(|s| Spoken { name: s.name, from_ms: s.from_ms, to_ms: s.to_ms }).collect()
}

pub fn run(shared: &Shared, layout: &Layout, rx: Receiver<ToNames>) {
    let calls = CallStore::new(&layout.calls());
    // Made here: it sets this thread up for COM.
    let reader = match uia::Reader::new() {
        Ok(r) => Some(r),
        Err(e) => {
            log::info!("names from the meeting app: {e}");
            None
        }
    };
    let mut current: Option<Current> = None;
    let mut poll = Duration::from_millis(500);
    while !shared.stop.load(Ordering::Relaxed) {
        let msg = match rx.recv_timeout(if current.is_some() { poll } else { Duration::from_millis(500) }) {
            Ok(m) => Some(m),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        for m in msg.into_iter().chain(rx.try_iter()) {
            match m {
                ToNames::Started { id, app_exe, before } => {
                    if let Some(c) = current.take() {
                        finish(&calls, c, poll, shared.settings().separate_voices);
                    }
                    current = Some(Current {
                        id,
                        app_exe,
                        before,
                        pids: Vec::new(),
                        pids_at: None,
                        own: Vec::new(),
                        tiles: Timeline::default(),
                        captions: Timeline::default(),
                        last_caption: None,
                        people: Vec::new(),
                        saved_at: Instant::now(),
                        transcript_len: 0,
                    });
                }
                ToNames::Ended => {
                    if let Some(c) = current.take() {
                        finish(&calls, c, poll, shared.settings().separate_voices);
                    }
                }
            }
        }
        let (Some(c), Some(reader)) = (current.as_mut(), reader.as_ref()) else { continue };
        let settings = shared.settings();
        if !settings.names_from_app {
            continue;
        }
        if c.pids_at.is_none_or(|t| t.elapsed() > Duration::from_secs(10)) {
            c.pids_at = Some(Instant::now());
            c.pids = probe::pids_of(&c.app_exe, &probe::processes());
        }
        let started = Instant::now();
        let titles_now = reader.titles(&c.pids);
        // The meeting's window: one that wasn't there before the call, when there is one.
        let meeting: Vec<String> = titles_now.iter().filter(|t| !c.before.contains(t)).cloned().collect();
        let elements = reader.read(&c.pids, &meeting, roster::MAX_ELEMENTS);
        let took = started.elapsed();
        // A slow window is looked at less often.
        poll = (took * 4).clamp(Duration::from_millis(400), Duration::from_secs(3));
        let poll_ms = u64::try_from(poll.as_millis()).unwrap_or(3000);
        let now = time::now_ms();
        let seen = roster::read(&elements);
        for o in seen.own.iter().chain(&titles::own_names(&titles_now)) {
            if !c.own.contains(o) {
                c.own.push(o.clone());
            }
        }
        let own = c.own.clone();
        let is_own = |n: &str| titles::is_own(n, &settings.your_name, &own);
        let speaking: Vec<String> = seen.speaking.iter().filter(|n| !is_own(n)).cloned().collect();
        c.tiles.observe(now, &speaking, poll_ms);
        // The newest caption growing: its speaker is speaking (a moment ago).
        let newest = seen.captions.last().cloned();
        let caption_speaker = match (&newest, &c.last_caption) {
            (Some(n), Some(last)) if n != last && !is_own(&n.0) => vec![n.0.clone()],
            _ => Vec::new(),
        };
        if newest.is_some() {
            c.last_caption = newest;
        }
        c.captions.observe(now.saturating_sub(CAPTION_LAG_MS), &caption_speaker, poll_ms);
        for p in &seen.people {
            if !is_own(p) && !c.people.contains(p) && c.people.len() < 200 {
                c.people.push(p.clone());
            }
        }
        if c.saved_at.elapsed() >= SAVE_EVERY {
            c.saved_at = Instant::now();
            save(&calls, c, now, settings.separate_voices);
        }
    }
    if let Some(c) = current.take() {
        finish(&calls, c, poll, shared.settings().separate_voices);
    }
}

/// The stretches that ended go to the call's file, the people to the call, and voices that are
/// clearly someone are named.
fn save(calls: &CallStore, c: &mut Current, now: u64, separate: bool) {
    let mut ended = stretches(c.tiles.take());
    ended.extend(stretches(c.captions.take()));
    if !ended.is_empty()
        && let Err(e) = calls.append_speaking(&c.id, &ended)
    {
        log::warn!("saving who spoke on {}: {e}", c.id);
    }
    let len = calls.transcript_len(&c.id);
    let named = if len != c.transcript_len || !ended.is_empty() {
        c.transcript_len = len;
        let mut spoken = calls.speaking(&c.id);
        spoken.extend(stretches(c.tiles.open(now)));
        spoken.extend(stretches(c.captions.open(now)));
        voices_named(&calls.lines(&c.id), &spoken, separate)
    } else {
        Vec::new()
    };
    let Some(mut call) = calls.load(&c.id) else { return };
    let before = call.clone();
    for p in &c.people {
        call.saw_person(p);
    }
    for (speaker, name) in named {
        if !call.names.contains_key(&speaker.key()) {
            log::info!("a voice on {} is named from {}", c.id, c.app_exe);
            call.set_name(speaker, &name);
        }
    }
    if call != before
        && let Err(e) = calls.save(&call)
    {
        log::warn!("saving {}: {e}", c.id);
    }
}

/// The call ended: what's still going on ends, and is saved with the rest.
fn finish(calls: &CallStore, mut c: Current, poll: Duration, separate: bool) {
    let poll_ms = u64::try_from(poll.as_millis()).unwrap_or(3000);
    let mut ended = c.tiles.end(poll_ms);
    ended.extend(c.captions.end(poll_ms));
    if !ended.is_empty()
        && let Err(e) = calls.append_speaking(&c.id, &stretches(ended))
    {
        log::warn!("saving who spoke on {}: {e}", c.id);
    }
    c.transcript_len = u64::MAX;
    save(calls, &mut c, time::now_ms(), separate);
    if !c.people.is_empty() {
        log::info!("{} people seen on {}", c.people.len(), c.id);
    }
}

/// The share of a line that has to fall in someone's stretches for it to count for them.
const LINE_COVERED: u64 = 30;

/// Names for the others' voices from who the app showed speaking: a voice is named after the
/// person whose stretches its lines fall in, when at least two of its lines and three seconds do,
/// and seven in ten of its lines that fall in anyone's are theirs. Without separate voices
/// (everyone's lines are the others'), only when nine in ten are, and three lines: a
/// one-to-one call.
pub fn voices_named(lines: &[Line], spoken: &[Spoken], separate: bool) -> Vec<(Speaker, String)> {
    // For each speaker: for each name, the lines that fell in their stretches, and how long.
    let mut tally: BTreeMap<String, (Speaker, BTreeMap<String, (u32, u64)>)> = BTreeMap::new();
    for l in lines {
        let ok = match l.speaker {
            Speaker::Voice(_) => separate,
            Speaker::Others => true,
            Speaker::Me => false,
        };
        if !ok {
            continue;
        }
        let dur = l.end_ms.saturating_sub(l.start_ms).max(1);
        let mut per: BTreeMap<&str, u64> = BTreeMap::new();
        for s in spoken {
            let from = s.from_ms.max(l.start_ms);
            let to = s.to_ms.min(l.end_ms);
            if to > from {
                *per.entry(s.name.as_str()).or_default() += to - from;
            }
        }
        let Some((name, ms)) = per.into_iter().max_by_key(|(_, ms)| *ms) else { continue };
        if ms.saturating_mul(100) < dur.saturating_mul(LINE_COVERED) {
            continue;
        }
        let e = tally.entry(l.speaker.key()).or_insert_with(|| (l.speaker, BTreeMap::new())).1.entry(name.to_string()).or_default();
        e.0 += 1;
        e.1 += ms.min(dur);
    }
    let mut out = Vec::new();
    for (speaker, names) in tally.into_values() {
        let votes: u32 = names.values().map(|(n, _)| n).sum();
        let Some((name, (n, ms))) = names.into_iter().max_by_key(|(_, (n, ms))| (*n, *ms)) else { continue };
        let (min_lines, share) = if speaker == Speaker::Others { (3, 90) } else { (2, 70) };
        if n >= min_lines && ms >= 3000 && u64::from(n) * 100 >= u64::from(votes) * share {
            out.push((speaker, name));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(speaker: Speaker, start_ms: u64, end_ms: u64) -> Line {
        Line { start_ms, end_ms, speaker, text: "words".into() }
    }

    fn spoke(name: &str, from_ms: u64, to_ms: u64) -> Spoken {
        Spoken { name: name.into(), from_ms, to_ms }
    }

    #[test]
    fn a_voice_is_named_after_who_the_app_showed_speaking() {
        let spoken = [spoke("Jane Doe", 0, 10_000), spoke("John Smith", 12_000, 20_000), spoke("Jane Doe", 21_000, 30_000)];
        let lines = [
            line(Speaker::Voice(1), 1_000, 4_000),
            line(Speaker::Voice(1), 5_000, 9_000),
            line(Speaker::Voice(2), 13_000, 16_000),
            line(Speaker::Voice(1), 22_000, 25_000),
            // Voice 2 once, too briefly to say.
            line(Speaker::Me, 10_000, 12_000),
        ];
        let named = voices_named(&lines, &spoken, true);
        assert_eq!(named, vec![(Speaker::Voice(1), "Jane Doe".to_string())]);
        // A second line of voice 2 while John is speaking names it.
        let mut more = lines.to_vec();
        more.push(line(Speaker::Voice(2), 17_000, 19_500));
        let named = voices_named(&more, &spoken, true);
        assert_eq!(named, vec![(Speaker::Voice(1), "Jane Doe".to_string()), (Speaker::Voice(2), "John Smith".to_string())]);
    }

    #[test]
    fn a_voice_that_is_two_people_is_left_alone() {
        let spoken = [spoke("Jane Doe", 0, 10_000), spoke("John Smith", 10_000, 20_000)];
        let lines = [
            line(Speaker::Voice(1), 1_000, 4_000),
            line(Speaker::Voice(1), 5_000, 9_000),
            line(Speaker::Voice(1), 11_000, 15_000),
            line(Speaker::Voice(1), 16_000, 19_000),
        ];
        assert!(voices_named(&lines, &spoken, true).is_empty());
        // Lines that only brush a stretch don't count.
        let brushing = [line(Speaker::Voice(3), 9_500, 14_000), line(Speaker::Voice(3), 9_000, 14_500)];
        assert!(voices_named(&brushing, &[spoke("Jane Doe", 0, 10_000)], true).is_empty());
    }

    #[test]
    fn without_separate_voices_only_a_one_to_one_call_is_named() {
        let spoken = [spoke("Jane Doe", 0, 60_000)];
        let lines: Vec<Line> = (0..4).map(|k| line(Speaker::Others, k * 10_000 + 1_000, k * 10_000 + 6_000)).collect();
        assert_eq!(voices_named(&lines, &spoken, false), vec![(Speaker::Others, "Jane Doe".to_string())]);
        let group = [spoke("Jane Doe", 0, 20_000), spoke("John Smith", 20_000, 60_000)];
        assert!(voices_named(&lines, &group, false).is_empty());
        // Separate voices off: the voices' lines (from before) aren't looked at.
        let voices = [line(Speaker::Voice(1), 1_000, 4_000), line(Speaker::Voice(1), 5_000, 9_000)];
        assert!(voices_named(&voices, &spoken, false).is_empty());
    }

    #[test]
    fn nothing_seen_names_nobody() {
        assert!(voices_named(&[], &[], true).is_empty());
        let lines = [line(Speaker::Voice(1), 1_000, 4_000), line(Speaker::Voice(1), 5_000, 9_000)];
        assert!(voices_named(&lines, &[], true).is_empty());
        let odd = [spoke("Jane Doe", 9_000, 1_000)];
        assert!(voices_named(&lines, &odd, true).is_empty());
    }
}
