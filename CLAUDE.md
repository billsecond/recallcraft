# RecallCraft — instructions for agents

RecallCraft is an open-source, Rust-native recall utility: a tray service that keeps the last
hours of the microphone and the computer's sound, finds calls on its own, and writes them down as
they happen, with a window to browse, replay and export them. It follows the conventions of its
sibling `../ScreenGrabCraft` (read its `CLAUDE.md`); the speech engine and much of the desktop
code come from there.

## Start every session here
1. Read `plan/STATUS.md` if it exists (`plan/` is gitignored), else `ROADMAP.md`.
2. Don't stop to ask unless a decision is genuinely the owner's (licensing, publishing, pushing to
   new remotes, secrets, anything that records or plays sound on the owner's computer beyond a
   short test they know about).

## Never crash
It runs all the time; a crash loses what was being said.
- **No panics in non-test code:** no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`,
  `unimplemented!`; no `unsafe` (`unsafe_code = "forbid"`). Crate roots carry the `deny` list.
- **Everything from outside is hostile:** the recording's files, call files, settings, WAV files,
  devices' answers, the worker's answers. `get()` instead of `[i]`, checked arithmetic, capped
  sizes and loops. Codecs and the speech model run under `catch_unwind`.
- Every crash fix lands with a regression test.

## Recordings are private
- **No network at all.** No update check, no telemetry, nothing. Adding a network feature is the
  owner's decision.
- Nothing of what was said is logged (times, sizes, device and program names only).
- **Never record or play sound on the owner's computer without saying so first**, and never commit
  a recording. Tests use made-up sound; `crates/engine/examples/simulate.rs` plays a call made
  with Windows' text-to-speech voices through the real recorder and hearer, without devices.

## Architecture (see `docs/architecture.md`)
| Layer | Crate | Job |
|---|---|---|
| L1 | `model` | settings, calls, lines, speakers, times |
| L2 | `store` | the recording (Opus in five-minute files), calls, retention, export, WAV/Ogg |
| L2 | `speech` | Whisper (ScreenGrabCraft's), utterances, echo, packing, voices |
| L3 | `audio` | capture (WASAPI / cpal), placing blocks in time, playback, time-stretch |
| L3 | `calls` | calling apps, the call detector, window titles, probing Windows |
| L4 | `engine` | the recorder, the devices loop, the hearer and its worker |
| L5 | `desktop` | paths, settings file, log, one instance, sign-in, supervisor, icon |
| apps | `recallcraft`, `recallcraft-cli` | the service + window + worker, and the CLI |

- Nothing below `apps/` depends on eframe/egui/winit/tray-icon/global-hotkey/rfd (`cargo xtask layers`).
- The service starts the window (`--window`) and the hearing worker (`--hear`) as its own child
  processes: the process loopback leaves the service's process tree out of the recording.
- The window only reads what the service writes, plus the settings and owner edits to a call.

## Quality gates
Before every commit: `cargo xtask ci` (fmt, clippy -D warnings, tests, layers). Commit after every
arc of work that builds, with a task id (`M1.2: …`).

## Running
- `cargo run --release -p recallcraft` (the service and the window); `RECALLCRAFT_NO_PREFS=1`
  runs with temporary settings and data, no sign-in entry and no instance lock.
- Logs: `%APPDATA%\RecallCraft\logs\service.log`, `window.log`, `hearer.log`, `supervisor.log`.
- `cargo run -p recallcraft-calls --example probe` shows which programs have a microphone open.
- `cargo run -p recallcraft-audio --example devices` lists microphones and checks each source for
  three seconds (prints loudness only).
- `cargo run --release -p recallcraft-speech --example voices -- a.wav b.wav` shows how far apart
  voices are.
