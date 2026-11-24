# RecallCraft

**Never miss what was said: the last two days of what you heard and said, every call written down
as it happens, all on your own computer. Written from scratch in Rust.**

RecallCraft sits in the tray and keeps recording, quietly: your microphone and what the computer
plays. It keeps the last 48 hours (you choose how long) and throws away what's older. When a call
starts in Teams, Zoom, Slack, Webex, Discord, WhatsApp, Phone Link, a browser meeting or another
calling app, it notices on its own, nothing pops up, and it writes down what everyone says, a few
seconds after they say it:

```text
Speaker 1  0:00   Hi, thanks for joining. Are you the developer on the reporting project?
Me         0:07   Yes, that's me. I have been working on the export feature this week.
Speaker 2  0:13   Great. Can you walk us through what is left before the release on Friday?
```

> **Status: early alpha (0.1).** Recording, call detection, the live transcript, playback and
> export are in place and tested; see [ROADMAP.md](ROADMAP.md) for what's next.

## What it does

- **Always recording, compactly, or only during calls.** Both sides are kept as Opus (24–32
  kbit/s), in files of five minutes; silence isn't kept. A day of a busy office is a few hundred
  megabytes. Older than the setting (48 hours by default) is deleted, except the calls you keep.
  Or choose **Record only during calls** (Settings, or the tray menu): the microphone and the
  computer's sound are opened when a call starts and closed when it ends.
- **Always in the tray.** Closing or minimising the window puts it in the tray; RecallCraft keeps
  recording until you choose **Quit** (in the tray's menu, or Settings). It runs under a
  supervisor that starts it again if it ever stops unexpectedly.
- **Knows when you're on a call.** A call is a calling app holding a microphone open. Its title
  comes from the call's window ("Weekly sync | Microsoft Teams" is *Weekly sync*), never from your
  own account's window, and a one-to-one call's title names the other person, who then labels
  their side of the transcript.
- **Hears both sides, whatever you listen on.** During a call RecallCraft records the speaker the
  calling app is actually playing on: a headset, when the computer's default is its speakers.
- **Knows who said what.** The microphone is you. The computer's sound is the others, told apart
  by their voices (*Speaker 1*, *Speaker 2*); name one once and RecallCraft knows that voice on
  later calls. With speakers instead of headphones, the others heard by your microphone aren't
  taken for you.
- **Names from the meeting app.** During a call RecallCraft reads the meeting app's window as a
  screen reader would: who's on the call, who it shows speaking, and its live captions' speakers.
  A voice whose lines keep coming while the app shows the same person speaking is named after
  them (*Speaker 2* becomes *Jane Doe*). Turning on the app's live captions helps.
  `recallcraft-cli call-window`, run during a call, shows what the app's window tells
  RecallCraft.
- **Live transcript.** Whisper runs on your computer, in a program of its own at below-normal
  priority, only on what's said during calls. A minute of call takes about four seconds to hear.
- **Replay.** Ctrl+Alt+R (or the tray menu) plays the last 30 seconds again at 1.5×, without
  changing the voices' pitch; pressed again, it stops. In the window, **Catch up** plays a live
  call from 30 seconds back, fast, until it's caught up; **Live** listens from a few seconds ago.
- **Browse.** The window lists the calls by day. A call shows who was on it (click to name them),
  its sound with each side's activity along it, and its transcript: click a line to play from
  there; the line playing is marked. Search finds what was said in any call. **Everything** shows
  every minute of the last two days, to play or export any hour.
- **Export.** Sound as one WAV, a stereo WAV (you left, them right), two WAVs, or a small Opus
  file; transcripts as text, Markdown, SubRip, WebVTT (with who spoke) or JSON. The folder and the
  file name pattern (`{date} {time} {title}`) are yours.

Nothing pops up when a call starts. There's a red dot on the tray icon while a call is recorded
(you can turn it off), and Windows shows its own microphone-in-use mark while RecallCraft records.

## Privacy, and the people on your calls

Everything recorded stays on this computer: there's no account, no cloud, no telemetry, and the
speech is written down here too. The only thing RecallCraft asks the network for is whether
there's a newer version (Settings › Updates; [PRIVACY.md](PRIVACY.md) says exactly what's sent). **In many places everyone on a call must agree to it being recorded.**
That's yours to arrange. Pause recording from the tray (15 minutes, an hour, or until you resume)
whenever you need to. See [PRIVACY.md](PRIVACY.md).

## Running it

Windows 10 (version 2004) or 11. Build it with a current Rust:

```powershell
cargo build --release
.\target\release\recallcraft.exe
```

The first start of an installed copy arranges to start at sign-in; a build in `target/` never
does. `recallcraft service install` arranges it by hand.

```text
recallcraft                      start (or open the window of the one running)
recallcraft --replay             play the last seconds again
recallcraft --pause <minutes>    stop recording for a while (0: until --resume)
recallcraft --resume | --quit
recallcraft service install|uninstall|status
```

For scripts and agents, `recallcraft-cli`:

```text
recallcraft-cli calls [--json]
recallcraft-cli transcript <call|latest> [--format text|md|srt|vtt|json]
recallcraft-cli export <call|latest> [--audio mixed|stereo|separate|opus|none] [--transcript text,srt] [--out <folder>]
recallcraft-cli search <words…>
recallcraft-cli status
recallcraft-cli hear <file.wav>
```

Settings, the log and the voices you named are in `%APPDATA%\RecallCraft`; the recording and the
calls in `%LOCALAPPDATA%\RecallCraft` (or a folder you choose).

## Bluetooth headsets

While any program records a Bluetooth headset's microphone, the headset switches to its
hands-free mode, and music and everything else sounds much worse through it. So by default a
Bluetooth headset's microphone is recorded only during calls (the calling app has switched it
already). The computer's sound is always recorded.

## Elsewhere

macOS and Linux build and record the microphone; recording what the computer plays there, and
finding calls, aren't done yet.

## How it's made

A Cargo workspace in the shape of ScreenGrabCraft's, its sibling: see
[docs/architecture.md](docs/architecture.md). The speech engine (Whisper on candle, in Rust) and
the device, single-instance and sign-in code come from ScreenGrabCraft.

```powershell
cargo xtask ci
```

## Licence

MIT or Apache-2.0, at your choice. The speech model is OpenAI's Whisper (MIT); see
[ATTRIBUTION.md](ATTRIBUTION.md).
