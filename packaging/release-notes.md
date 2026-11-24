## RecallCraft 0.1

The first version: the last two days of what you heard and said, every call written down as it happens, all on your own computer.

- **Always recording, compactly.** Your microphone and what the computer plays, as Opus in five-minute files; silence isn't kept. Older than the setting (48 hours by default) is deleted, except the calls you keep.
- **Knows when you're on a call** in Teams, Zoom, Slack, Webex, Discord, WhatsApp, Phone Link, a browser meeting or another calling app, and names it from the meeting's window.
- **Live transcript.** Whisper runs on your computer, in a program of its own at below-normal priority, only on what's said during calls; the others are told apart by their voices.
- **Replay** the last 30 seconds at 1.5× (Ctrl+Alt+R), **browse** the calls by day, **search** what was said, and **export** the sound (WAV, stereo WAV, Opus) and the transcript (text, Markdown, SubRip, WebVTT, JSON).

**Early alpha.** The foundations are in place and tested; see the [README](https://github.com/GITHUB_REPOSITORY#readme) and the roadmap for what's next. **In many places everyone on a call must agree to it being recorded: that is yours to arrange.** Nothing leaves your computer: there's no account, no cloud, no telemetry, and RecallCraft has no network features at all (not even an update check).

RecallCraft for Windows, macOS, Linux and FreeBSD. Every file is listed with its SHA-256 in `SHA256SUMS.txt`. **Windows is where RecallCraft records both sides of a call and finds calls on its own;** on macOS, Linux and FreeBSD it records the microphone, and the rest isn't done there yet.

RecallCraft is two programs: `recallcraft`, which lives in the system tray (the menu bar on macOS), records, and opens the window where the calls are listed; and the command-line program `recallcraft-cli`, for scripts and agents (the calls, their transcripts, exports and search). The speech model (OpenAI's Whisper base.en, MIT) is in every package.

| Platform | File | |
|---|---|---|
| Windows 10 (2004) / 11 (x64, x86, ARM64) | `recallcraft-<ver>-windows-<arch>.msi` | Installer, just for you (no administrator: `%LocalAppData%\Programs\RecallCraft`; on the command line `msiexec /i <msi> ALLUSERS=2 MSIINSTALLPERUSER=1`) or for everyone on this PC (Program Files\RecallCraft, the default), with a Start menu entry. RecallCraft starts at every sign-in for the person who installed it (and right away from the finish page), and for anyone else on the PC once they've started it themselves; turn that off in its settings or in Task Manager › Startup apps. Upgrading or uninstalling closes the running RecallCraft. `recallcraft-cli.exe` is installed next to it. |
| | `recallcraft-<ver>-windows-<arch>-portable.zip` | Portable: unzip anywhere and run `recallcraft.exe`; includes `recallcraft-cli.exe` and the model. Its first start arranges to start at sign-in (turn it off in its settings). |
| macOS 11+ (Apple silicon and Intel) | `recallcraft-<ver>-macos-universal.dmg` | Drag RecallCraft to Applications. It runs in the menu bar (no Dock icon), asks once for the microphone, and starts itself at sign-in. |
| | `recallcraft-cli-<ver>-macos-universal.zip` | The command-line program alone, with the model. |
| Linux x86_64 and aarch64 (glibc 2.35+) | `recallcraft-<ver>-linux-<arch>.AppImage` | `chmod +x` and run (if it does not start, add `--appimage-extract-and-run`); the CLI is inside too. The `.zsync` next to it enables delta updates. |
| | `recallcraft-<ver>-linux-<arch>.deb` / `.rpm` | `sudo apt install ./recallcraft-*.deb` or `sudo dnf install ./recallcraft-*.rpm`. RecallCraft arranges to start at sign-in, for you, the first time you run it (`~/.config/autostart`); a systemd user unit, `recallcraft.service`, is there as the alternative. |
| | `recallcraft-<ver>-linux-<arch>.flatpak` | `flatpak install --user recallcraft-*.flatpak` (the freedesktop runtime comes from Flathub). It doesn't start at sign-in yet. |
| | `recallcraft-<ver>-linux-<arch>.tar.gz` | `bin/` with both programs; the desktop entry, metainfo, icons, the model and an autostart entry under `share/`; the systemd user unit under `lib/`. |
| FreeBSD 15 (x86_64) | `recallcraft-<ver>-freebsd-x86_64.tar.gz` | Both programs, the desktop entry, the icons, the model and an autostart entry. |

On Linux and FreeBSD the tray icon uses the StatusNotifierItem protocol: KDE Plasma, Xfce, Cinnamon and MATE show it; GNOME needs the AppIndicator extension.

The builds are not signed with a publisher certificate yet: Windows SmartScreen and macOS Gatekeeper will ask before running them.
