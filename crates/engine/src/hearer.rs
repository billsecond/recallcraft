//! The hearer: what's said during calls, written down as it's said.
//!
//! Utterances come from the recorder. Those of a call are queued; the last minute of the others
//! is kept, so what was said just before a call was noticed is heard too. The queue is heard a
//! window at a time (a few utterances of one stream end to end, [`Packed`]), oldest first:
//!
//! - the microphone's utterances that were the speakers (the others heard again) aren't heard;
//! - the computer's are told apart by voice ([`VoiceBook`]), and a voice the owner named on an
//!   earlier call is named again ([`crate::known`]);
//! - a line of yours whose words are the others' at the same time is their echo, and goes.
//!
//! The worker is let go when there's been no call for a while (its memory goes with it), and
//! started again for the next.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use recallcraft_model::{Line, Speaker, Stream, time};
use recallcraft_speech::echo;
use recallcraft_speech::pack::Packed;
use recallcraft_speech::vad::Utterance;
use recallcraft_speech::voices::{self, Analyzer, KnownVoice, VoiceBook};
use recallcraft_store::Layout;
use recallcraft_store::calls::CallStore;

use crate::Shared;
use crate::known::{self, CallVoice};
use crate::recorder::ToHear;
use crate::worker::{ChildHear, Hear, HearWith};

/// The microphone's utterance was the speakers if the computer was the louder this much of it.
const ECHO_SHARE: f32 = 0.6;
/// Kept of what was said outside a call, in case a call is noticed late.
const KEEP_BEFORE_MS: u64 = 60_000;
/// The most speech waiting (the worker can't keep up): older goes.
const MAX_WAITING_S: f32 = 30.0 * 60.0;
/// The worker is let go after this long without a call.
const REST_AFTER: Duration = Duration::from_secs(300);
/// A voice heard for this long (10 ms steps) is compared with those the owner named.
const RECOGNISE_AFTER: f32 = 600.0;

struct Item {
    call: String,
    stream: Stream,
    utt: Utterance,
}

struct CallState {
    started_ms: u64,
    ended_ms: Option<u64>,
    book: VoiceBook,
    /// The lines of the last few minutes, for leaving echoes out.
    lines: VecDeque<Line>,
    named: Vec<u8>,
    voices_saved: Instant,
}

impl CallState {
    fn new(started_ms: u64) -> CallState {
        CallState { started_ms, ended_ms: None, book: VoiceBook::new(), lines: VecDeque::new(), named: Vec::new(), voices_saved: Instant::now() }
    }

    fn holds(&self, utt: &Utterance) -> bool {
        let (s, e) = (utt.start_us / 1000, utt.end_us() / 1000);
        e > self.started_ms && self.ended_ms.is_none_or(|end| s < end)
    }
}

#[allow(clippy::large_enum_variant)]
enum Hearer {
    Child(ChildHear),
    Here(Box<dyn Hear>),
    Off(String),
}

impl Hearer {
    fn hear(&mut self, sound: &[f32]) -> Result<Vec<recallcraft_speech::Line>, String> {
        match self {
            Hearer::Child(c) => c.hear(sound),
            Hearer::Here(h) => h.hear(sound),
            Hearer::Off(why) => Err(why.clone()),
        }
    }

    fn state(&self) -> String {
        match self {
            Hearer::Child(c) if c.is_running() => "ready".to_string(),
            Hearer::Child(c) => match &c.failed {
                Some((e, _)) => e.clone(),
                None => "resting".to_string(),
            },
            Hearer::Here(_) => "ready".to_string(),
            Hearer::Off(why) => why.clone(),
        }
    }

    /// Whether it can't hear just now, but may in a while (so the queue waits).
    fn waiting(&self) -> bool {
        matches!(self, Hearer::Child(c) if !c.may_start())
    }

    fn rest(&mut self) {
        if let Hearer::Child(c) = self
            && c.is_running()
        {
            log::info!("no call for a while: the hearer rests");
            c.rest();
        }
    }
}

fn waiting_s(queue: &VecDeque<Item>) -> f32 {
    queue.iter().map(|i| i.utt.seconds()).sum::<f32>().max(0.0)
}

pub fn run(shared: &Shared, layout: &Layout, settings_dir: &Path, with: HearWith, rx: Receiver<ToHear>) {
    let calls = CallStore::new(&layout.calls());
    let mut hearer = match with {
        HearWith::Program { exe, args, model, threads } => Hearer::Child(ChildHear::new(exe, args, model, threads)),
        HearWith::Here(h) => Hearer::Here(h),
        HearWith::Off(why) => Hearer::Off(why),
    };
    let analyzer = Analyzer::new();
    let mut queue: VecDeque<Item> = VecDeque::new();
    let mut before: VecDeque<(Stream, Utterance)> = VecDeque::new();
    let mut states: HashMap<String, CallState> = HashMap::new();
    let mut live: Option<String> = None;
    let mut known: Vec<KnownVoice> = known::library(settings_dir);
    let mut last_call = Instant::now();
    loop {
        // Everything that came in; wait a moment when there's nothing to do.
        let first = if queue.is_empty() || hearer.waiting() {
            rx.recv_timeout(Duration::from_millis(1000))
        } else {
            rx.try_recv().map_err(|_| RecvTimeoutError::Timeout)
        };
        let mut msgs = Vec::new();
        match first {
            Ok(m) => msgs.push(m),
            Err(RecvTimeoutError::Disconnected) if queue.is_empty() => break,
            Err(_) => {}
        }
        msgs.extend(rx.try_iter());
        for m in msgs {
            match m {
                ToHear::Utterance { stream, utt, echo } => {
                    if stream == Stream::Mic && echo >= ECHO_SHARE {
                        log::debug!("an utterance of the microphone was the speakers; not heard");
                        continue;
                    }
                    if stream == Stream::System && shared.was_playing(utt.start_us, utt.end_us()) {
                        log::debug!("an utterance of the computer's sound was RecallCraft's own playback; not heard");
                        continue;
                    }
                    match live.as_ref().and_then(|id| states.get(id).map(|s| (id, s))) {
                        Some((id, st)) if st.holds(&utt) => queue.push_back(Item { call: id.clone(), stream, utt }),
                        _ => {
                            before.push_back((stream, utt));
                            let now = time::now_ms();
                            while before.front().is_some_and(|(_, u)| u.end_us() / 1000 + KEEP_BEFORE_MS < now) {
                                before.pop_front();
                            }
                        }
                    }
                }
                ToHear::CallStarted { id, started_ms } => {
                    known = known::library(settings_dir);
                    let st = CallState::new(started_ms);
                    // What was said since the call started, before it was noticed.
                    let mut early: Vec<Item> =
                        before.drain(..).filter(|(_, u)| st.holds(u)).map(|(stream, utt)| Item { call: id.clone(), stream, utt }).collect();
                    early.sort_by_key(|i| i.utt.start_us);
                    queue.extend(early);
                    states.insert(id.clone(), st);
                    live = Some(id);
                    last_call = Instant::now();
                }
                ToHear::CallEnded { id, ended_ms } => {
                    if let Some(st) = states.get_mut(&id) {
                        st.ended_ms = Some(ended_ms);
                    }
                    if live.as_deref() == Some(&id) {
                        live = None;
                    }
                    last_call = Instant::now();
                }
            }
        }
        let s = shared.settings();
        if !s.transcribe_calls {
            queue.clear();
        }
        // Speech of calls that were deleted meanwhile isn't heard.
        queue.retain(|i| calls.dir(&i.call).is_some_and(|d| d.exists()));
        while waiting_s(&queue) > MAX_WAITING_S {
            queue.pop_front();
        }
        // Calls that ended and have nothing left to hear are done with.
        let done: Vec<String> =
            states.iter().filter(|(id, st)| st.ended_ms.is_some() && !queue.iter().any(|i| &i.call == *id)).map(|(id, _)| id.clone()).collect();
        for id in done {
            if let Some(st) = states.remove(&id) {
                save_voices(&calls, &id, &st.book);
            }
        }
        shared.update(|st| {
            st.hearer = if s.transcribe_calls { hearer.state() } else { "off".to_string() };
            st.waiting_s = waiting_s(&queue);
        });
        if queue.is_empty() {
            if live.is_none() && last_call.elapsed() > REST_AFTER {
                hearer.rest();
            }
            if shared.stopping() {
                break;
            }
            continue;
        }
        if hearer.waiting() {
            continue;
        }
        if shared.stopping() {
            break;
        }
        hear_some(&mut queue, &mut states, &mut hearer, &analyzer, &calls, &known, s.separate_voices, settings_dir);
    }
    for (id, st) in &states {
        save_voices(&calls, id, &st.book);
    }
    log::info!("the hearer stopped");
}

fn save_voices(calls: &CallStore, id: &str, book: &VoiceBook) {
    let Some(dir) = calls.dir(id) else { return };
    if !dir.exists() || book.is_empty() {
        return;
    }
    let mut out = BTreeMap::new();
    for n in 1..=voices::MAX_VOICES {
        if let Some((v, w)) = book.mean(n) {
            out.insert(n, CallVoice { v: v.to_vec(), frames: w });
        }
    }
    known::save_call_voices(&dir, &out);
}

/// Hears the oldest utterances of one stream of one call, in one window.
fn hear_some(
    queue: &mut VecDeque<Item>,
    states: &mut HashMap<String, CallState>,
    hearer: &mut Hearer,
    analyzer: &Analyzer,
    calls: &CallStore,
    known: &[KnownVoice],
    separate: bool,
    _settings_dir: &Path,
) {
    let Some(head) = queue.front() else { return };
    let (call_id, stream) = (head.call.clone(), head.stream);
    // The utterances of this call and stream, oldest first, as many as fit in a window.
    let picks: Vec<usize> = queue.iter().enumerate().filter(|(_, i)| i.call == call_id && i.stream == stream).map(|(k, _)| k).take(40).collect();
    let utts: Vec<&Utterance> = picks.iter().filter_map(|k| queue.get(*k).map(|i| &i.utt)).collect();
    let (packed, taken) = Packed::new(&utts);
    let taken_idx: Vec<usize> = picks.into_iter().take(taken.max(1)).collect();
    let mut items: Vec<Item> = Vec::new();
    for k in taken_idx.iter().rev() {
        if let Some(i) = queue.remove(*k) {
            items.push(i);
        }
    }
    items.reverse();
    if packed.is_empty() {
        return;
    }
    // Who said each.
    let speakers: Vec<Speaker> = match stream {
        Stream::Mic => vec![Speaker::Me; items.len()],
        Stream::System if !separate => vec![Speaker::Others; items.len()],
        Stream::System => {
            let Some(st) = states.get_mut(&call_id) else { return };
            items
                .iter()
                .map(|i| {
                    analyzer.fingerprint(&i.utt.samples).and_then(|fp| st.book.assign(&fp, i.utt.start_us)).map_or(Speaker::Others, Speaker::Voice)
                })
                .collect()
        }
    };
    let started = Instant::now();
    let heard = match hearer.hear(&packed.sound) {
        Ok(h) => h,
        Err(e) => {
            if hearer.waiting() {
                // It'll be started again in a while: the speech waits.
                for i in items.into_iter().rev() {
                    queue.push_front(i);
                }
            } else {
                log::warn!("hearing: {e}");
            }
            return;
        }
    };
    log::debug!("heard {:.1} s in {} ms", packed.sound.len() as f32 / 16_000.0, started.elapsed().as_millis());
    let Some(st) = states.get_mut(&call_id) else { return };
    let mut new_lines: Vec<Line> = Vec::new();
    let mut echoes: Vec<Line> = Vec::new();
    for l in heard {
        let Some(placed) = packed.place(l.start, l.end) else { continue };
        let speaker = speakers.get(placed.piece).copied().unwrap_or(stream.speaker());
        let Some(line) = (Line { start_ms: placed.start_us / 1000, end_ms: placed.end_us / 1000, speaker, text: l.text }).checked() else { continue };
        let theirs: Vec<(u64, u64, &str)> =
            st.lines.iter().filter(|x| !x.speaker.is_me()).map(|x| (x.start_ms * 1000, x.end_ms * 1000, x.text.as_str())).collect();
        if line.speaker.is_me() && echo::is_echo(line.start_ms * 1000, line.end_ms * 1000, &line.text, &theirs) {
            log::debug!("a line of the microphone was the others' heard again; left out");
            continue;
        }
        if !line.speaker.is_me() {
            for mine in st.lines.iter().filter(|x| x.speaker.is_me()) {
                if echo::is_echo(
                    mine.start_ms * 1000,
                    mine.end_ms * 1000,
                    &mine.text,
                    &[(line.start_ms * 1000, line.end_ms * 1000, line.text.as_str())],
                ) {
                    echoes.push(mine.clone());
                }
            }
        }
        new_lines.push(line);
    }
    if !echoes.is_empty() {
        st.lines.retain(|l| !echoes.contains(l));
        let kept: Vec<Line> = calls.lines(&call_id).into_iter().filter(|l| !echoes.contains(l)).collect();
        if let Err(e) = calls.replace_lines(&call_id, &kept) {
            log::warn!("the transcript of {call_id}: {e}");
        }
        log::debug!("{} lines of the microphone were the others' heard again; taken out", echoes.len());
    }
    if !new_lines.is_empty() {
        if let Err(e) = calls.append(&call_id, &new_lines) {
            log::warn!("the transcript of {call_id}: {e}");
        }
        st.lines.extend(new_lines);
        while st.lines.len() > 200 {
            st.lines.pop_front();
        }
    }
    // Voices heard long enough are compared with those the owner named.
    if stream == Stream::System && separate {
        let mut names: Vec<(u8, String)> = Vec::new();
        for n in 1..=voices::MAX_VOICES {
            if st.named.contains(&n) {
                continue;
            }
            if let Some((v, w)) = st.book.mean(n)
                && w >= RECOGNISE_AFTER
            {
                st.named.push(n);
                if let Some(k) = voices::recognise(known, v) {
                    names.push((n, k.name.clone()));
                }
            }
        }
        if !names.is_empty()
            && let Some(mut c) = calls.load(&call_id)
        {
            for (n, name) in names {
                if !c.names.contains_key(&Speaker::Voice(n).key()) {
                    log::info!("a voice on {call_id} is one named before");
                    c.set_name(Speaker::Voice(n), &name);
                }
            }
            let _ = calls.save(&c);
        }
        if st.voices_saved.elapsed() > Duration::from_secs(20) {
            st.voices_saved = Instant::now();
            save_voices(calls, &call_id, &st.book);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    use recallcraft_model::{Call, Settings};

    use super::*;

    /// Hears every window as one line per utterance in it, saying how long it was.
    struct Fake;

    impl Hear for Fake {
        fn hear(&mut self, sound: &[f32]) -> Result<Vec<recallcraft_speech::Line>, String> {
            // Utterances are the stretches that aren't silent.
            let mut out = Vec::new();
            let mut start: Option<usize> = None;
            for (i, s) in sound.iter().enumerate().chain(std::iter::once((sound.len(), &0.0))) {
                match (start, *s != 0.0) {
                    (None, true) => start = Some(i),
                    (Some(a), false) => {
                        let secs = (i - a) as f32 / 16_000.0;
                        out.push(recallcraft_speech::Line {
                            start: a as f32 / 16_000.0,
                            end: i as f32 / 16_000.0,
                            text: format!("about {secs:.0} seconds"),
                        });
                        start = None;
                    }
                    _ => {}
                }
            }
            Ok(out)
        }
    }

    fn utt(start_ms: u64, secs: f32) -> Utterance {
        let n = (secs * 16_000.0) as usize;
        Utterance { start_us: start_ms * 1000, samples: vec![0.1; n], levels: vec![-20.0; n / 320], voiced: (n / 320) as u32 }
    }

    #[test]
    fn a_call_s_speech_is_written_down_with_what_came_just_before_it() {
        let root = std::env::temp_dir().join(format!("recallcraft-hearer-{}", std::process::id()));
        let layout = Layout::new(&root);
        let calls = CallStore::new(&layout.calls());
        let shared = Shared::new(Settings { separate_voices: false, ..Settings::default() });
        let (tx, rx) = channel();
        let t0 = time::now_ms();
        calls.save(&Call { id: "c1".into(), started_ms: t0, ..Call::default() }).unwrap();
        // Said before the call was noticed, and before it started.
        tx.send(ToHear::Utterance { stream: Stream::Mic, utt: utt(t0 - 30_000, 2.0), echo: 0.0 }).unwrap();
        tx.send(ToHear::Utterance { stream: Stream::Mic, utt: utt(t0 + 500, 3.0), echo: 0.0 }).unwrap();
        tx.send(ToHear::CallStarted { id: "c1".into(), started_ms: t0 }).unwrap();
        tx.send(ToHear::Utterance { stream: Stream::System, utt: utt(t0 + 5000, 4.0), echo: 0.0 }).unwrap();
        // RecallCraft replaying something: in the computer's sound, but not the others'.
        shared.playing((t0 + 11_000) * 1000, (t0 + 14_000) * 1000);
        tx.send(ToHear::Utterance { stream: Stream::System, utt: utt(t0 + 11_500, 2.0), echo: 0.0 }).unwrap();
        // The speakers heard by the microphone: not heard.
        tx.send(ToHear::Utterance { stream: Stream::Mic, utt: utt(t0 + 5100, 3.5), echo: 0.9 }).unwrap();
        tx.send(ToHear::CallEnded { id: "c1".into(), ended_ms: t0 + 20_000 }).unwrap();
        tx.send(ToHear::Utterance { stream: Stream::Mic, utt: utt(t0 + 30_000, 2.0), echo: 0.0 }).unwrap();
        drop(tx);
        run(&shared, &layout, &root.join("settings"), HearWith::Here(Box::new(Fake)), rx);
        let lines = calls.lines("c1");
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!((lines[0].speaker, lines[0].text.as_str()), (Speaker::Me, "about 3 seconds"));
        assert_eq!(lines[0].start_ms, t0 + 500);
        assert_eq!((lines[1].speaker, lines[1].start_ms), (Speaker::Others, t0 + 5000));
        let _ = std::fs::remove_dir_all(&root);
    }
}
