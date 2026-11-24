# Architecture

```text
 microphone ─┐                      ┌─ ring/mic, ring/system   (Opus, five-minute files)
             ├─ blocks ── recorder ─┤
 computer  ──┘   (48 kHz mono,      └─ utterances (16 kHz) ── hearer ── calls/<id>/transcript.jsonl
                  µs since 1970)                                  │
                                                                  └─ worker: recallcraft --hear
 devices loop (every second): captures kept going · calls found · titles · retention · status.json
```

## Processes

- **The service** (`recallcraft --tray`, or `--background` under the supervisor): the tray, the
  shortcuts, and the engine's threads. No window of its own: a winit event loop.
- **The window** (`recallcraft --window`), started by the service: eframe. It reads the files the
  service writes and plays the recording.
- **The worker** (`recallcraft --hear <model>`), started by the service when there's speech to
  hear and let go after five minutes without a call: Whisper, below-normal priority, two threads.
- The computer's sound is a speaker's loopback, as ScreenGrabCraft records it: the default
  speaker, or during a call the one the call plays on. RecallCraft's own playback is in it, so the
  service notes when it plays (the window tells it every two seconds while it plays) and the
  hearer leaves those stretches out of the transcript.

## The recording (`crates/store/src/ring.rs`)

One folder per stream, a file per five minutes of the clock, named by its first moment (ms since
1970). A file is a header and records: `R` (a run starts at this µs) and `P` (a 20 ms Opus packet
and its loudness). Silence (below −72 dBFS for more than a second) ends a run. Files are flushed
every second and read while they're written; a record cut short is left out. A closed file has a
`.lvl` beside it, its loudness each second, for the two-day overview.

Times are the system clock. Each device's blocks are numbered from its start and placed again on
the clock when they drift by more than 80 ms (`crates/audio/src/place.rs`), so the two streams line
up for echo and playback.

## Calls (`crates/calls`)

Every second: WASAPI's audio sessions on every capture device, with their processes; a webview's
or a browser's audio process is put down to the app that started it. A known calling app (or a
browser, or one the owner listed) holding a microphone for 1.5 s starts a call (dated 3 s
earlier); none holding one for 15 s ends it. A call shorter than 20 s with nothing said is dropped.
Every 3 s, the app's windows: the one that wasn't there before the call names it.

## Hearing (`crates/engine/src/hearer.rs`, `crates/speech`)

The recorder cuts each stream into utterances at pauses of 0.7 s (the background level is
followed). The hearer queues those of the call going on (and the minute before it was noticed),
packs a few of one stream into one 30-second window, hears it in the worker, and takes each line
back to its utterance and time. The microphone's utterances during which the computer was louder
are the speakers heard again, and aren't heard; a line of yours with the others' words at the same
time is dropped too. The computer's utterances get a voice fingerprint (16 cepstral coefficients
and the pitch, each as a mean and spread); a voice is the nearest by Bhattacharyya distance, or a
new one past 0.05.

## Playback (`crates/audio/src/playback.rs`)

A feeder thread reads 100 ms ahead from the recording, mixes the two sides, time-stretches by WSOLA
(32 ms slices, matched within 8 ms) so speed keeps pitch, and queues it for the speaker. Live, it
never passes what's been written; at speed, it drops to 1× once it has caught up.
