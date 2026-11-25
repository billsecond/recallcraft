//! The recorder: every block of sound from the devices goes into the recording, and is cut into
//! utterances (at 16 kHz) for the hearer. Each utterance of the microphone says how much of it
//! the computer's sound was the louder ([`recallcraft_speech::echo::louder_share`]): near all of it,
//! it's the others heard through the speakers.

use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use recallcraft_audio::Block;
use recallcraft_model::Stream;
use recallcraft_speech::echo::{self, Levels};
use recallcraft_speech::resample::Down3;
use recallcraft_speech::vad::{Segmenter, Utterance};
use recallcraft_store::ring::{RingWriter, rms_db};

use crate::Shared;

/// What the hearer is told.
#[derive(Debug)]
pub enum ToHear {
    /// Something said, and (for the microphone) how much of it was the speakers.
    Utterance {
        stream: Stream,
        utt: Utterance,
        echo: f32,
    },
    CallStarted {
        id: String,
        started_ms: u64,
    },
    CallEnded {
        id: String,
        ended_ms: u64,
    },
}

struct StreamState {
    down: Down3,
    seg: Segmenter,
    levels: Levels,
    expect_us: Option<u64>,
}

impl StreamState {
    fn new() -> StreamState {
        StreamState { down: Down3::new(), seg: Segmenter::new(), levels: Levels::new(), expect_us: None }
    }
}

/// A block's sound as 20 ms loudness steps.
fn levels_of(start_us: u64, samples: &[f32], into: &mut Levels) {
    for (k, frame) in samples.chunks(960).enumerate() {
        into.push(start_us + k as u64 * 20_000, rms_db(frame));
    }
}

/// What the speaker played while RecallCraft was playing sound (`own`, µs since 1970) made
/// silent: a replay recorded would be played again in the next one, round and round.
fn silence_own(start_us: u64, samples: &mut [f32], own: &[(u64, u64)]) {
    let n = samples.len() as u64;
    let at = |us: u64| (us.saturating_sub(start_us) * 48_000 / 1_000_000).min(n) as usize;
    for (a, b) in own {
        let (from, to) = (at(*a), at(*b));
        if let Some(s) = samples.get_mut(from..to) {
            s.fill(0.0);
        }
    }
}

/// Runs until the devices' channel closes or the engine stops.
pub fn run(shared: &Shared, ring: &Path, rx: Receiver<Block>, tx: Sender<ToHear>) {
    let mut writer = RingWriter::new(ring);
    let mut states = [StreamState::new(), StreamState::new()];
    let mut last_error: Option<Instant> = None;
    loop {
        let mut block = match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(b) => b,
            Err(RecvTimeoutError::Timeout) => {
                writer.flush();
                if shared.stopping() {
                    break;
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let i = usize::from(block.stream.id());
        // How loud it was as it came: the microphone's echo is told by the speakers' loudness,
        // RecallCraft's own playback included.
        if let Some(st) = states.get_mut(i) {
            levels_of(block.start_us, &block.samples, &mut st.levels);
        }
        if block.stream == Stream::System {
            let end_us = block.start_us + block.samples.len() as u64 * 1_000_000 / 48_000;
            silence_own(block.start_us, &mut block.samples, &shared.own_sound_in(block.start_us, end_us));
        }
        if let Err(e) = writer.push(block.stream, block.start_us, &block.samples)
            && last_error.is_none_or(|t| t.elapsed() > Duration::from_secs(60))
        {
            log::warn!("writing the recording: {e}");
            last_error = Some(Instant::now());
        }
        let utts = {
            let Some(st) = states.get_mut(i) else { continue };
            if st.expect_us.is_some_and(|e| e.abs_diff(block.start_us) > 100_000) {
                st.down.reset();
            }
            st.expect_us = Some(block.start_us + block.samples.len() as u64 * 1_000_000 / 48_000);
            let s16 = st.down.push(&block.samples);
            st.seg.push(block.start_us, &s16)
        };
        for utt in utts {
            let share = match block.stream {
                Stream::Mic => {
                    let [mic, system] = &states;
                    echo::louder_share(utt.start_us, utt.end_us(), &mic.levels, &system.levels)
                }
                Stream::System => 0.0,
            };
            if tx.send(ToHear::Utterance { stream: block.stream, utt, echo: share }).is_err() {
                break;
            }
        }
    }
    for (i, st) in states.iter_mut().enumerate() {
        if let (Some(utt), Some(stream)) = (st.seg.flush(), Stream::from_id(i as u8)) {
            let _ = tx.send(ToHear::Utterance { stream, utt, echo: 0.0 });
        }
    }
    writer.close();
    log::info!("the recorder stopped ({} bytes written)", writer.written());
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    use recallcraft_model::{Settings, time};
    use recallcraft_store::ring::RingReader;

    use super::*;

    #[test]
    fn only_the_stretches_recallcraft_played_in_are_silenced() {
        let mut s = vec![0.5f32; 48_000];
        silence_own(1_000_000, &mut s, &[(0, 1_100_000), (1_250_000, 1_500_000), (3_000_000, 4_000_000)]);
        assert!(s[..4800].iter().all(|v| *v == 0.0), "from before the block into it");
        assert!(s[4800..12_000].iter().all(|v| *v == 0.5));
        assert!(s[12_000..24_000].iter().all(|v| *v == 0.0));
        assert!(s[24_000..].iter().all(|v| *v == 0.5), "a stretch after the block changes nothing");
    }

    #[test]
    fn what_recallcraft_plays_is_left_out_of_the_recording() {
        let root = std::env::temp_dir().join(format!("recallcraft-recorder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let shared = Shared::new(Settings::default());
        // A whole second, a minute ago; RecallCraft played its second second.
        let t0 = (time::now_us() / 1_000_000 - 60) * 1_000_000;
        shared.playing(t0 + 1_000_000, t0 + 2_000_000);
        let (tx, rx) = channel();
        let (hear_tx, _hear_rx) = channel();
        let tone: Vec<f32> = (0..48_000 * 3).map(|i| (i as f32 * 0.05).sin() * 0.3).collect();
        tx.send(Block { stream: Stream::System, start_us: t0, samples: tone.clone() }).unwrap();
        tx.send(Block { stream: Stream::Mic, start_us: t0, samples: tone }).unwrap();
        drop(tx);
        run(&shared, &root, rx, hear_tx);
        let mut reader = RingReader::new(&root);
        let mut loudest = |stream, at_us| reader.read(stream, at_us, 9600, 48_000).iter().map(|v| v.abs()).fold(0.0f32, f32::max);
        assert!(loudest(Stream::System, t0 + 300_000) > 0.1, "before it played");
        assert!(loudest(Stream::System, t0 + 1_300_000) < 0.01, "while it played");
        assert!(loudest(Stream::System, t0 + 2_500_000) > 0.1, "after");
        assert!(loudest(Stream::Mic, t0 + 1_300_000) > 0.1, "the microphone is kept");
        let _ = std::fs::remove_dir_all(&root);
    }
}
