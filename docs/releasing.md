# Releasing RecallCraft

> **Last reviewed:** 2026-11-24 · **Last updated:** 2026-11-24 · **Change:** major (first version: the packaging scripts and the release workflow, ported from ScreenGrabCraft's) · **Status:** the release workflow ran for 0.2.0: every system's packages built, and the Windows x64 install test stopped the publishing (it read a sign-in key the uninstall had removed; fixed for 0.3.0). Automatic updates are described below; the first release that has them is 0.3.0.

The canonical recipe is [craftrules `release/playbook.md`](https://github.com/storytold/craftrules/blob/main/release/playbook.md);
this file has the RecallCraft specifics. The scripts in `packaging/` are ScreenGrabCraft's with
RecallCraft's names, and differ from them only where the program does: two programs instead of
three (the service, its window and its worker are one program), no file types, no web build, a
speech model in every package, starting at sign-in kept per person (a recorder must never start
for someone who didn't start it), and an installer identity of its own. So a release has the
files a Craft app's has, under the same names.

RecallCraft lives outside the storytold organisation, so it cannot use the organisation's shared
release workflow (that one needs the organisation's runners and its `release` environment).
`.github/workflows/release.yml` and `package.yml` are RecallCraft's own and run on GitHub's
standard runners in any repository, a fork included. If the repository moves into the
organisation, call `storytold/craft-common-actions/.github/workflows/release.yml` instead. The
quick checks do use the organisation's shared workflows, which work from outside it:
`packaging-lint.yml`, `freebsd.yml` (build and test in a FreeBSD VM) and `windows-arm64.yml`
(install the ARM64 MSI on Windows 11 ARM64).

The two programs, in every package, with the speech model:

| Program | What | Windows subsystem |
|---|---|---|
| `recallcraft` | The service: tray icon (menu bar on macOS), the recorder, the replay shortcut; its window (`--window`) and the worker that writes speech down (`--hear`) are copies of it. Starts at sign-in and is restarted if it crashes. | GUI |
| `recallcraft-cli` | The command-line program: the calls, their transcripts, exports, search | console |
| `models/` | `whisper-base.en-q8_0.gguf`, `LICENSE-WHISPER.txt`, `README.md` (`assets/models`), where the programs look for them (`crates/desktop/src/models.rs`): beside the programs on Windows, `../share/recallcraft/models` on Linux, FreeBSD, the AppImage and the Flatpak, `Contents/Resources/models` in the macOS app | |

1. Bump the version: `cargo xtask version set 0.2.0` (the one source of truth is
   `[workspace.package] version`; the programs show it in `--version`), commit, and get
   `cargo xtask ci` green.
2. Tag that commit `v0.2.0` and push the tag. The workflow refuses a tag that isn't the version in
   `Cargo.toml`, then builds:

| Platform | Artifacts | Signing |
|---|---|---|
| macOS | universal `RecallCraft.app` in a `.dmg`, universal CLI zip (with the model) | Developer ID + notarization (`APPLE_*` secrets) |
| Windows | x64, x86 and arm64 `.msi` + portable `.zip` (both programs and the model) | Azure Trusted Signing (`AZURE_*` secrets) or a certificate (`WINDOWS_CERTIFICATE*`) |
| Linux | x86_64 and aarch64 `.AppImage` (+ `.zsync` for AppImageUpdate), `.flatpak` bundle, `.deb`, `.rpm`, `.tar.gz`; Flathub manifest | checksums |
| FreeBSD | x86_64 `.tar.gz` | checksums |

3. The run publishes ONE GitHub release, `RecallCraft v<version>` on that tag, with every file and
   a `SHA256SUMS.txt` (`sha256sum` format; `packaging/release-notes.md` is its text, so update it
   for each release). Running the same tag again replaces the files of that release. A version
   with a suffix (`0.2.0-rc.1`) is published as a pre-release.

The release files, 22 of them with `SHA256SUMS.txt`:

```
recallcraft-<ver>-windows-{x64,x86,arm64}.msi
recallcraft-<ver>-windows-{x64,x86,arm64}-portable.zip
recallcraft-<ver>-macos-universal.dmg
recallcraft-cli-<ver>-macos-universal.zip
recallcraft-<ver>-linux-{x86_64,aarch64}.{AppImage,AppImage.zsync,deb,rpm,tar.gz,flatpak}
recallcraft-<ver>-freebsd-x86_64.tar.gz
SHA256SUMS.txt
```

Each package carries the 84 MB speech model, so each is about that size.

Signing secrets (`APPLE_*`, `WINDOWS_CERTIFICATE*` or `AZURE_*`, the same names as
ScreenGrabCraft's and MailCraft's) are repository secrets; without them the builds are unsigned
(ad-hoc on macOS) and the scripts say so, so forks still build. A FreeBSD build that fails doesn't
hold the release back: it then has no FreeBSD tarball.

Dry run: `gh workflow run release.yml --ref <branch>` builds everything on any branch and keeps
the files as workflow artifacts without publishing anything (FreeBSD only when asked for: it is
slow).

## Automatic updates

The service checks `GET /repos/billsecond/recallcraft/releases/latest` a minute after it starts
and every six hours (never during a call; "Check for updates automatically" in Settings › Updates
turns it off; `crates/desktop/src/update.rs`). It relies on what the workflow above produces, so
keep it so:

- The tag is `v<version>` and the version is semver. Drafts and pre-releases are never offered
  (GitHub's "latest" skips them anyway).
- The Windows MSIs are named `recallcraft-<ver>-windows-{x64,x86,arm64}.msi` and the portable
  zips `recallcraft-<ver>-windows-{x64,x86,arm64}-portable.zip`, with one folder in each zip that
  holds `recallcraft.exe`; `SHA256SUMS.txt` lists them. A copy updates from the file of its own
  architecture and kind, downloaded into `%TEMP%\RecallCraft\updates`, checked against
  `SHA256SUMS.txt` (and GitHub's own digest of the file, when it gives one) and deleted on a
  mismatch. A file that isn't listed, or a release without `SHA256SUMS.txt`, is never installed.
- **A copy the MSI installed:** Update now runs `msiexec /i <msi> /passive /norestart /l*v
  install.log` through a detached `cmd.exe` that then starts `recallcraft.exe --background` from
  where this copy is installed: the MSI closes RecallCraft while it installs
  (`util:CloseApplication`), and a passive install shows no finish page to start it again. An
  update keeps the copy's scope, read from where the program is: in `%LocalAppData%\Programs` it
  was installed just for the owner, so `msiexec` gets `ALLUSERS=2 MSIINSTALLPERUSER=1` and nothing
  is asked; anywhere else it is everyone's, and Windows asks for permission (UAC). If the owner
  says no, nothing was closed, the new start hands over to the running service and ends, and the
  update stays offered.
- **A portable copy** (unzipped, `models\whisper*.gguf` beside `recallcraft.exe`): Update now
  starts Windows PowerShell, detached and hidden, with a script that waits up to two minutes for
  every RecallCraft program in that folder to close, unzips the new version over it (files only
  in the folder stay) and starts `recallcraft.exe --background`; the service quits as soon as the
  script has started. If anything is still running after two minutes, nothing is copied.
- Never during a call: a check waits for the call to end, and Update now pressed during one
  installs when it ends.
- macOS, Linux, FreeBSD and builds in `target/` only tell the owner and open the release page. The
  Flatpak has no network permission and never checks.

`cargo test -p recallcraft-desktop github -- --ignored --nocapture` asks the real API as version
0.0.1; with `RECALLCRAFT_TEST_DOWNLOAD=1` it also downloads and checks the x64 portable zip,
without installing.

## What was left out, compared with ScreenGrabCraft

- **The web build** (`packaging/web`, the wasm job, `cargo xtask wasm`): RecallCraft has no web
  version.
- **The editor** (its app bundle, desktop entry, metainfo, Start menu entries, MIME type): the
  window is the same program as the service.
- **File types** (the MSI's ProgIds and Default apps capabilities, the Linux MIME package, the
  macOS document types): RecallCraft opens no files from Explorer or Finder.
- **The camera and screen recording permissions** (macOS): RecallCraft asks for the microphone only.
- **The 1024 px icon PNGs**: the icon is drawn by `crates/desktop/src/icon.rs`, which goes up to
  512 px; the `.icns` stops there too.

## ArtCraft Launcher

The launcher installs a Craft app from its GitHub releases, finding the right file by name
(`<app>-<version>-<os>-<arch>.<ext>`) and checking it against `SHA256SUMS.txt`. What it needs of
RecallCraft, and where each part stands:

| Needs | State |
|---|---|
| One release per version with Craft file names and `SHA256SUMS.txt` | The workflow above produces it; no release has been published yet |
| An icon in the Craft style | `cargo xtask icon` (`packaging/icons.sh`) writes it in every size the packages use into `assets/app-icon/`; `packaging/check-icons.sh` checks them |
| A row in the launcher's catalog (`crates/core/src/catalog.rs` in `storytold/craft-launcher`): id, name, tagline, colours, `owner/repo`; for an app outside the organisation also its own reverse-DNS id (`io.github.billsecond.recallcraft`) and publisher (the "community apps" group) | A pull request on that repository, after the first release; the 128 px icon goes with it |

## What the installers do

- **Windows (`.msi`, WiX 5, `packaging/windows/recallcraft.wxs`):** one package that installs
  either for everyone on this PC (per-machine: Program Files\RecallCraft, the all-users Start
  menu, HKLM; an administrator has to allow it) or just for one person (per-user:
  `%LocalAppData%\Programs\RecallCraft`, their Start menu, HKCU; no administrator). The first page
  shows the licence and two buttons, "Just for me" and "Everyone on this PC"; when an earlier
  version is installed only its way is offered, since Windows upgrades only a product installed
  the same way. On the command line:

  | | Command |
  |---|---|
  | Everyone on this PC (the default) | `msiexec /i recallcraft-<ver>-windows-x64.msi` (add `/passive` or `/qn`) |
  | Just for me, no administrator | `msiexec /i recallcraft-<ver>-windows-x64.msi ALLUSERS=2 MSIINSTALLPERUSER=1` (add `/passive` or `/qn`) |

  Both properties are needed for a per-user install (the package's own `ALLUSERS` is 1, and
  Windows reads `MSIINSTALLPERUSER` only when `ALLUSERS` is 2). A silent install the other way than
  the installed copy stops with a message instead of installing a second copy beside it; to
  switch, uninstall first. One Start Menu shortcut, "RecallCraft" (it starts RecallCraft and opens
  its window, or opens the running one's); an App Paths entry for `recallcraft.exe`. The speech
  model in `models\` beside the programs.

  **Starting at sign-in is per person, in both scopes** (unlike ScreenGrabCraft, whose per-machine
  install starts its service for every user from HKLM): the `RecallCraft` value under
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (`"…\recallcraft.exe" --background`) for
  the person who installs it. It's the value RecallCraft writes itself on its first run, and the
  one its "Start RecallCraft when I sign in" switch removes, so an HKLM value would turn the switch
  back on; and a recorder mustn't start for an account that never started it. Anyone else on the
  PC gets the same value when they first start RecallCraft (the program on its first run, and the
  advertised shortcut writing the package's per-user part). The Run value isn't the component's
  key path (a marker under `HKCU\Software\RecallCraft\Installer` is), so Windows never repairs it
  back after the owner turned it off. The installer removes the installing person's value on
  uninstall; another account's stays, pointing at the removed program, until that person removes
  it (Windows skips a missing program at sign-in).

  The finish page's "Start RecallCraft" (ticked) starts it the way a sign-in does. Upgrading or
  uninstalling closes it first: WM_CLOSE to every `recallcraft.exe` (the window closes), then
  whatever is left (the service, the worker, the windowless supervisor) is terminated, with exit
  code 0 and no reboot prompt; that step is moved to the start of the install script so it runs
  before any file is removed. The recording loses at most its last second. The licence page is
  built from `LICENSE-MIT`, `LICENSE-APACHE` and the model's `LICENSE-WHISPER.txt`, with a line on
  recording consent. The portable `.zip` holds both programs, the model and the docs; its first
  start arranges to start at sign-in like an installed copy (a build in `target/` never does).
- **macOS (`.dmg`):** `RecallCraft.app` (`io.github.billsecond.recallcraft`, a menu-bar app:
  `LSUIElement`, no Dock icon), universal (arm64 + x86_64), hardened runtime with the microphone
  entitlement and `NSMicrophoneUsageDescription`, macOS 11 or later, next to an `Applications`
  link. RecallCraft registers its own LaunchAgent (`--tray`, KeepAlive on a failure) on first run;
  the DMG installs nothing of the kind. macOS asks the owner once for the microphone.
- **Linux (`.deb`, `.rpm`):** `/usr/bin/recallcraft` and `recallcraft-cli`; the desktop entry
  `io.github.billsecond.recallcraft.desktop`, AppStream metadata, the icons, the model in
  `/usr/share/recallcraft/models`. **No `/etc/xdg/autostart` entry:** RecallCraft writes
  `~/.config/autostart/io.github.billsecond.recallcraft.desktop` for the person running it, the
  first time (its switch removes it again), and a system entry would start a recorder for every
  account and come back when the switch is off. The same entry is in
  `/usr/share/recallcraft/autostart/` to copy by hand, and
  `/usr/lib/systemd/user/recallcraft.service` (`--tray`, `Restart=on-failure`, part of
  `graphical-session.target`) is installed as the alternative and not enabled:
  `systemctl --user enable --now recallcraft.service` after turning the app's own start at sign-in
  off. The binaries need glibc 2.35 or later and ALSA (through PipeWire or PulseAudio on a
  desktop); the tray needs a StatusNotifierItem host (KDE, Xfce, Cinnamon, MATE; GNOME with the
  AppIndicator extension).
- **AppImage:** starts RecallCraft; the CLI and the model are inside too. No autostart entry or
  unit: RecallCraft registers the AppImage itself (`$APPIMAGE`) on its first run.
- **Flatpak (`io.github.billsecond.recallcraft`):** one app holding both programs; the command is
  RecallCraft. Permissions: Wayland (X11 as fallback), IPC, the GPU, PulseAudio (the microphone),
  Documents (exports go to Documents/RecallCraft), and `org.kde.StatusNotifierWatcher` for the
  tray; no network. **It doesn't start at sign-in yet:** the entry RecallCraft writes stays inside
  the sandbox; that needs the Background portal. The two manifests in `packaging/linux/flatpak/`
  (from source for Flathub, and the release bundle repackaging the tarball) must agree on
  runtime, SDK and permissions; packaging-lint checks it.
- **`.tar.gz` (Linux, FreeBSD):** `bin/` with both programs; `share/` with the desktop entry,
  metainfo, icons, docs, the model in `share/recallcraft/models/`, and the autostart entry under
  `share/recallcraft/autostart/`; on Linux also `lib/systemd/user/recallcraft.service`.
- **Product identity:** the MSI's UpgradeCode and component GUIDs are RecallCraft's own. Never copy
  them from or into a sibling Craft app (Windows would treat the two as one product).

## What the packages expect of the programs

- `--version` on both, printing and exiting without a display, a tray host, D-Bus or sound devices
  (`package.sh` and the CI checks call it; on Windows only the CLI is run, the service being a GUI
  program).
- `recallcraft --background` (the sign-in entry point: a supervisor running the service,
  restarting it after an abnormal exit, exiting when the service exits normally) and `--tray`
  (the service alone, for launchd and systemd). The MSI ends both by WM_CLOSE and, failing that,
  TerminateProcess with exit code 0, so exit code 0 must mean "don't restart".
- `recallcraft --window` starts the window on its own (the Linux CI starts it under Xvfb).
- `RECALLCRAFT_NO_PREFS=1`: temporary settings and data, no sign-in registration, no
  single-instance lock (the CI smoke tests start the programs with it).
- `RECALLCRAFT_REQUIRE_WINRES=1` (set by `package.ps1`): the build fails if the Windows icon or
  VERSIONINFO can't be embedded.
- The model is found beside the program or in `../share/recallcraft/models` or
  `../Resources/models` (`crates/desktop/src/models.rs`).
- A program outside `target/` (and `target-*/`) registers itself to start at sign-in on its first
  run (`crates/desktop/src/service.rs`, `apps/recallcraft/src/service.rs`).

## Building the packages on your own machine

Windows (needs the MSVC Rust toolchain, the Windows SDK and the .NET SDK):

```
dotnet tool install --global wix --version 5.0.2
wix extension add -g WixToolset.UI.wixext/5.0.2 WixToolset.Util.wixext/5.0.2
pwsh packaging/windows/package.ps1 -Arch x64
```

Linux (build on Ubuntu 22.04 so the result runs on newer systems; these are the packages the
release workflow installs):

```
sudo apt-get install -y build-essential pkg-config libxkbcommon-dev libwayland-dev libasound2-dev \
  libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev desktop-file-utils appstream zsync
# nfpm 2.47.0 from https://github.com/goreleaser/nfpm/releases, then:
packaging/linux/package.sh
```

Both write to `dist/release/`. Also: `packaging/macos/package.sh --arch universal`,
`packaging/linux/flatpak-bundle.sh` (after `package.sh`) and `packaging/freebsd/package.sh` (on
FreeBSD, with `alsa-lib`). `packaging/check-icons.sh` checks the app icons.

## What has been checked, and what has not

Checked on 2026-11-24, on a Windows 11 machine:

- `cargo xtask ci` (clippy, tests, assets, names, layers), `cargo xtask icon` and
  `packaging/check-icons.sh` (Git Bash).
- The PowerShell scripts parse (`[System.Management.Automation.Language.Parser]::ParseFile`), the
  shell scripts pass `bash -n`; the workflow YAML, the Flatpak and nfpm YAML, the `.wxs`, the
  plists and the metainfo parse; the two Flatpak manifests agree; the `.wxs` has no GUID twice
  and its icon id ends in `.ico` (the checks packaging-lint makes).
- `packaging/windows/package.ps1 -Arch x64` from the real programs: the PE checks (GUI service,
  console CLI), the licence RTF, the MSI (`wix build` without warnings) and the portable zip, the
  CLI's `--version`. `wix msi validate`: only ICE57 (the App Paths value, HKMU, beside the
  program: expected in a package that installs either way) and ICE61 (same-version upgrades, on
  purpose). The decompiled MSI has the HKCU Run value and its separate key path, the shortcut,
  the finish-page action and the close step after InstallInitialize; an administrative extract
  (`msiexec /a`) lays out both programs and `models\` with the model, its licence and readme; the
  zip holds the same and the docs. `wix build` for x86 and arm64 succeeds too. Nothing was
  installed.

Not checked yet: the MSI installed and uninstalled for real (the workflow does it on its runner,
with RecallCraft running, and checks the uninstall closed it), the finish page's launch, the x86
and arm64 Windows builds on their machines; macOS (signing, notarization, the DMG; and it has no
DMG artwork yet: `packaging/macos/dmg/README.md`), the Linux packages, the AppImage, the Flatpak
bundle and FreeBSD, which only GitHub's runners build; shellcheck, actionlint, desktop-file-validate
and appstreamcli (packaging-lint runs them); signing; the release workflow on GitHub, which will
need fixing on its first run.

## Revision history

| Date | Change |
|---|---|
| 2026-11-24 | Automatic updates: what the update check relies on in a release, how an MSI copy and a portable copy update and start again |
| 2026-11-24 | First version: packaging for Windows, macOS, Linux and FreeBSD from ScreenGrabCraft's; the speech model in every package; starting at sign-in per person; a release workflow of RecallCraft's own; what has and hasn't been checked |
