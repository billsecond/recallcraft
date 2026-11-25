# Privacy

RecallCraft records sound around the clock, so it's built to keep that sound yours.

## What's recorded, and where it goes

- **Your microphone and what the computer plays**, all the time while recording is on (a
  Bluetooth headset's microphone only during calls, by default), or **only during calls** if you
  choose that (Settings, or the tray menu): then nothing is opened, let alone kept, between
  calls. What the computer plays is what the speaker plays, mixed (during a call, the speaker the
  call plays on), as ScreenGrabCraft records it. While RecallCraft itself plays sound (a replay,
  the window's playback), what the speaker plays isn't recorded, so its own sound isn't kept and
  played again; what the others say in those seconds isn't recorded either.
- **Kept** in `%LOCALAPPDATA%\RecallCraft` (or the folder you choose), as Opus files of five
  minutes. Silence isn't kept. Everything older than the setting (48 hours by default) is
  deleted, every ten minutes; calls are kept as long as their own setting (30 days by default),
  and those you keep (📌) until you delete them.
- **Transcripts** are written only during calls, by the speech model on this computer, into the
  call's folder. Voices you name are kept as numbers describing the voice (no sound) in
  `%APPDATA%\RecallCraft\voices.json`.
- **The meeting app's window**, during a call only, is read the way a screen reader reads it
  (Windows' UI Automation): only the calling app's windows, about twice a second, and nothing in
  them is clicked or changed. Kept, in the call's folder: the names of the people it showed on
  the call, and when it showed each one speaking. The words of its captions aren't kept. Turn it
  off with "Name them from the meeting app" (Settings › Calls).
- **Nothing of what's recorded goes to the network.** No account, no cloud, no telemetry, no
  crash reports. Sharing is you exporting a file.
- **The one network request is the update check.** A minute after RecallCraft starts, and then
  every six hours (never during a call), it asks GitHub for its latest release
  (`https://api.github.com/repos/billsecond/recallcraft/releases/latest`). It's a plain HTTPS
  request that says only "RecallCraft" and its version: no identifier, no cookie, nothing about
  you or what was recorded. GitHub sees your IP address, as any website does. On Windows, when a
  newer version is out, its installer (or, for a portable copy, its zip) is downloaded from the
  same release into your temporary folder and checked against the release's checksums; it's
  installed only when you choose Update now, and never during a call. Elsewhere you're told, and
  the release page opens if you ask. Turn it off with "Check for updates automatically"
  (Settings › Updates); Check now still works when you ask. The Flatpak has no network access, so
  it never checks.
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
