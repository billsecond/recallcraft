# Privacy

RecallCraft records sound around the clock, so it's built to keep that sound yours.

## What's recorded, and where it goes

- **Your microphone and what the computer plays**, all the time while recording is on (a
  Bluetooth headset's microphone only during calls, by default), or **only during calls** if you
  choose that (Settings, or the tray menu): then nothing is opened, let alone kept, between
  calls. What the computer plays is what the speaker plays, mixed (during a call, the speaker the
  call plays on), as ScreenGrabCraft records it, so RecallCraft's own replays are in it too; they
  aren't written down as the others' words.
- **Kept** in `%LOCALAPPDATA%\RecallCraft` (or the folder you choose), as Opus files of five
  minutes. Silence isn't kept. Everything older than the setting (48 hours by default) is
  deleted, every ten minutes; calls are kept as long as their own setting (30 days by default),
  and those you keep (📌) until you delete them.
- **Transcripts** are written only during calls, by the speech model on this computer, into the
  call's folder. Voices you name are kept as numbers describing the voice (no sound) in
  `%APPDATA%\RecallCraft\voices.json`.
- **Nothing goes to the network.** No account, no cloud, no telemetry, no update check, no crash
  reports. Sharing is you exporting a file.
- **The log** has times, device and program names, and how long things took; never what was said.

## Telling, and pausing

Nothing pops up when a call starts. The tray icon is always there, with a red dot while a call is
recorded (unless you turn it off), and Windows shows that a program is using the microphone.

In many places everyone on a call has to agree to it being recorded, and workplaces may have
rules of their own. That's yours to arrange. Pause from the tray menu (15 minutes, an hour, or
until you resume), or turn recording off in Settings. **Settings › Storage › Delete everything
recorded** deletes the whole recording and every call at once.

## What it can't protect

The recording is protected the way your other files are: by your Windows account. Anyone who can
read your files can read it. It isn't encrypted on disk yet ([ROADMAP.md](ROADMAP.md)).
