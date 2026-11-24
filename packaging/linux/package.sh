#!/usr/bin/env bash
# Build and package RecallCraft for Linux (<arch> is x86_64 or aarch64):
#
#   $DIST/recallcraft-<version>-linux-<arch>.AppImage        any distro with glibc >= the build host's
#   $DIST/recallcraft-<version>-linux-<arch>.AppImage.zsync  delta updates (needs zsyncmake)
#   $DIST/recallcraft-<version>-linux-<arch>.deb             Debian, Ubuntu, Mint, Pop!_OS, ...
#   $DIST/recallcraft-<version>-linux-<arch>.rpm             Fedora, openSUSE, RHEL, ...
#   $DIST/recallcraft-<version>-linux-<arch>.tar.gz          plain FHS-style tree (bin/, share/, lib/)
#
# Usage: packaging/linux/package.sh [--skip-build] [--formats "appimage deb rpm tar"]
#
# Needs: cargo; nfpm for deb/rpm (https://nfpm.goreleaser.com); appimagetool for the AppImage
# (downloaded into $CARGO_TARGET_DIR if missing). Build on an old distro (CI: Ubuntu 22.04,
# glibc 2.35) so the binaries run on newer ones. Optional: desktop-file-validate, appstreamcli,
# zsyncmake (the zsync package) for the AppImage's .zsync.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/linux"
APP_ID=io.github.billsecond.recallcraft

SKIP_BUILD=0
FORMATS="appimage deb rpm tar"
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --formats) FORMATS="$2"; shift 2 ;;
    -h | --help) sed -n '2,16p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) DEB_ARCH=amd64 ;;
  aarch64 | arm64) ARCH=aarch64; DEB_ARCH=arm64 ;;
  *) echo "unsupported architecture $ARCH" >&2; exit 2 ;;
esac
export RECALLCRAFT_MAINTAINER="${RECALLCRAFT_MAINTAINER:-RecallCraft contributors <31995458+billsecond@users.noreply.github.com>}"
BASENAME="recallcraft-$VERSION-linux-$ARCH"

echo "==> RecallCraft $VERSION for Linux $ARCH ($FORMATS)"

if [ "$SKIP_BUILD" = 0 ]; then
  (cd "$ROOT" && cargo build --release --locked -p recallcraft -p recallcraft-cli)
fi
BIN="$CARGO_TARGET_DIR/release"
WORK="$CARGO_TARGET_DIR/linux-package"
STAGE="$WORK/root"
rm -rf "$WORK"

# ---- stage an FHS tree (shared by every format) -------------------------------------------------
for bin in recallcraft recallcraft-cli; do
  install -Dm755 "$BIN/$bin" "$STAGE/usr/bin/$bin"
  strip "$STAGE/usr/bin/$bin" 2>/dev/null || true
done
install -Dm644 "$HERE/$APP_ID.desktop" "$STAGE/usr/share/applications/$APP_ID.desktop"
mkdir -p "$STAGE/usr/share/metainfo"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$RECALLCRAFT_BUILD_DATE/g" \
  "$HERE/$APP_ID.metainfo.xml.in" >"$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
# The icon (io.github.billsecond.recallcraft.png in every size).
mkdir -p "$STAGE/usr/share/icons"
cp -R "$ROOT/assets/app-icon/hicolor" "$STAGE/usr/share/icons/"
# The speech model (Whisper base.en, MIT), its licence and readme, where the programs look for it:
# ../share/recallcraft/models from their bin/.
copy_models "$STAGE/usr/share/recallcraft/models"
mkdir -p "$STAGE/usr/share/doc/recallcraft"
copy_docs "$STAGE/usr/share/doc/recallcraft"
# Starting at sign-in. RecallCraft arranges it itself, per person, the first time it runs (an entry
# in ~/.config/autostart, which its "Start RecallCraft when I sign in" switch turns off by removing
# it). So no package installs an entry in /etc/xdg/autostart: that would start a recorder for every
# account on the computer, and would come back whenever the owner turned the switch off (the user
# entry hides the system one only while it's there). The entry is kept under
# share/recallcraft/autostart/ to copy by hand, and the systemd user unit is installed as the
# alternative (not enabled).
install -Dm644 "$HERE/autostart/$APP_ID.desktop" "$STAGE/usr/share/recallcraft/autostart/$APP_ID.desktop"
install -Dm644 "$HERE/systemd/recallcraft.service" "$STAGE/usr/lib/systemd/user/recallcraft.service"

if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$STAGE/usr/share/applications/$APP_ID.desktop" "$STAGE/usr/share/recallcraft/autostart/$APP_ID.desktop"
fi
if command -v appstreamcli >/dev/null; then
  appstreamcli validate --no-net --explain "$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
fi

has() { case " $FORMATS " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

# ---- .tar.gz ------------------------------------------------------------------------------------
# The /usr tree (bin/, share/, lib/systemd/user/), with the autostart entry under
# share/recallcraft/autostart/ to copy into ~/.config/autostart.
if has tar; then
  mkdir -p "$WORK/tar"
  cp -R "$STAGE/usr" "$WORK/tar/$BASENAME"
  tar -C "$WORK/tar" -czf "$DIST/$BASENAME.tar.gz" "$BASENAME"
  echo "wrote $DIST/$BASENAME.tar.gz"
fi

# ---- .deb / .rpm --------------------------------------------------------------------------------
if has deb || has rpm; then
  command -v nfpm >/dev/null || { echo "error: nfpm not found (https://nfpm.goreleaser.com/install/)" >&2; exit 1; }
  export VERSION
  export NFPM_ARCH="$DEB_ARCH"
  # nfpm expands env vars in fields like `version` and `arch`, but not in `contents[].src`.
  sed "s|\${STAGE}|$STAGE|g" "$HERE/nfpm.yaml" >"$WORK/nfpm.yaml"
  for fmt in deb rpm; do
    if has "$fmt"; then (cd "$ROOT" && nfpm package -f "$WORK/nfpm.yaml" -p "$fmt" -t "$DIST/$BASENAME.$fmt"); fi
  done
fi

# ---- AppImage -----------------------------------------------------------------------------------
# Starts RecallCraft (the service, and its window); the CLI rides along in usr/bin. No autostart
# entry, systemd unit or docs inside: RecallCraft registers the AppImage itself (its path, from
# $APPIMAGE) when it first runs.
if has appimage; then
  APPDIR="$WORK/RecallCraft.AppDir"
  mkdir -p "$APPDIR"
  cp -R "$STAGE/usr" "$APPDIR/usr"
  rm -rf "$APPDIR/usr/share/doc" "$APPDIR/usr/lib" "$APPDIR/usr/share/recallcraft/autostart"
  ln -s usr/bin/recallcraft "$APPDIR/AppRun"
  cp "$HERE/$APP_ID.desktop" "$APPDIR/$APP_ID.desktop"
  cp "$ROOT/assets/app-icon/hicolor/256x256/apps/$APP_ID.png" "$APPDIR/$APP_ID.png"
  ln -s "$APP_ID.png" "$APPDIR/.DirIcon"

  TOOL="${APPIMAGETOOL:-$(command -v appimagetool || true)}"
  if [ -z "$TOOL" ]; then
    TOOL="$CARGO_TARGET_DIR/appimagetool-$ARCH.AppImage"
    if [ ! -x "$TOOL" ]; then
      curl -fsSL -o "$TOOL" "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$ARCH.AppImage"
      chmod +x "$TOOL"
    fi
  fi
  # Absolute, because appimagetool runs in $DIST below (CARGO_TARGET_DIR or APPIMAGETOOL may be
  # relative, e.g. target/agent-<name>).
  OUT="$(cd "$DIST" && pwd)/$BASENAME.AppImage"
  APPDIR="$(cd "$APPDIR" && pwd)"
  TOOL="$(cd "$(dirname "$TOOL")" && pwd)/$(basename "$TOOL")"
  # A .zsync left by an earlier run would hide a missing zsyncmake and describe another file.
  rm -f "$OUT.zsync"
  # Update information: AppImageUpdate, AppImageLauncher and the like read it from the file and
  # fetch only the blocks that changed in a newer release, through the .zsync published next to
  # each AppImage on GitHub Releases. `latest` is the newest published release that is not a
  # pre-release. A fork's builds point at its own releases through GITHUB_REPOSITORY.
  # (RecallCraft itself never looks: it has no network features.)
  REPO="${GITHUB_REPOSITORY:-billsecond/recallcraft}"
  UPDATE_INFO="gh-releases-zsync|${REPO%%/*}|${REPO#*/}|latest|recallcraft-*-linux-$ARCH.AppImage.zsync"
  # Extract-and-run: works without FUSE (containers, CI). The output embeds the static runtime,
  # so users don't need libfuse2 either. With zsyncmake on the host (CI installs the zsync
  # package) appimagetool also writes the .zsync, into its working directory, hence the cd.
  (cd "$DIST" && ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream -u "$UPDATE_INFO" "$APPDIR" "$OUT")
  echo "wrote $OUT"
  if [ -s "$OUT.zsync" ]; then
    echo "wrote $OUT.zsync"
  else
    warn "zsyncmake not found, so $OUT.zsync was not written; AppImage delta updates need it"
  fi
fi

# Both programs answer --version without a display.
for bin in recallcraft recallcraft-cli; do
  "$STAGE/usr/bin/$bin" --version
done
echo "==> done"
ls -lh "$DIST"
