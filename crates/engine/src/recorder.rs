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

/// Runs until the devices' channel closes or the engine stops.
pub fn run(shared: &Shared, ring: &Path, rx: Receiver<Block>, tx: Sender<ToHear>) {
    let mut writer = RingWriter::new(ring);
    let mut states = [StreamState::new(), StreamState::new()];
    let mut last_error: Option<Instant> = None;
    loop {
        let block = match rx.recv_timeout(Duration::from_millis(500)) {
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
        if let Err(e) = writer.push(block.stream, block.start_us, &block.samples)
            && last_error.is_none_or(|t| t.elapsed() > Duration::from_secs(60))
        {
            log::warn!("writing the recording: {e}");
            last_error = Some(Instant::now());
        }
        let i = usize::from(block.stream.id());
        let utts = {
            let Some(st) = states.get_mut(i) else { continue };
            levels_of(block.start_us, &block.samples, &mut st.levels);
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
