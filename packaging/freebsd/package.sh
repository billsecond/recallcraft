#!/usr/bin/env bash
# Build and package RecallCraft for FreeBSD:
#
#   $DIST/recallcraft-<version>-freebsd-<arch>.tar.gz
#       bin/    recallcraft (the service, its window and its worker), recallcraft-cli
#       share/  the desktop entry, metainfo, icons, docs, the speech model
#               (share/recallcraft/models, where the programs look for it), and the autostart
#               entry (share/recallcraft/autostart/, to copy into ~/.config/autostart;
#               RecallCraft also writes it itself the first time it runs)
#
# Usage: packaging/freebsd/package.sh [--skip-build]
#
# Needs: bash, cargo, alsa-lib (the microphone and playback, through cpal), and the libraries the
# window loads at run time (libxkbcommon, wayland or libX11, mesa-libs or vulkan-loader). Run by
# .github/workflows/package.yml inside a FreeBSD VM.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
LINUX="$ROOT/packaging/linux"
APP_ID=io.github.billsecond.recallcraft

SKIP_BUILD=0
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '2,15p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

ARCH="$(uname -m)"
case "$ARCH" in
  amd64 | x86_64) ARCH=x86_64 ;;
  arm64 | aarch64) ARCH=aarch64 ;;
  *) echo "unsupported architecture $ARCH" >&2; exit 2 ;;
esac
BASENAME="recallcraft-$VERSION-freebsd-$ARCH"

echo "==> RecallCraft $VERSION for FreeBSD $ARCH"

if [ "$SKIP_BUILD" = 0 ]; then
  (cd "$ROOT" && cargo build --release --locked -p recallcraft -p recallcraft-cli)
fi
BIN="$CARGO_TARGET_DIR/release"
WORK="$CARGO_TARGET_DIR/freebsd-package"
STAGE="$WORK/$BASENAME"
rm -rf "$WORK"

# BSD install has no -D: make the directories first.
mkdir -p "$STAGE/bin" "$STAGE/share/applications" "$STAGE/share/metainfo" "$STAGE/share/icons" \
  "$STAGE/share/doc/recallcraft" "$STAGE/share/recallcraft/autostart"
for bin in recallcraft recallcraft-cli; do
  install -m 755 "$BIN/$bin" "$STAGE/bin/$bin"
  strip "$STAGE/bin/$bin" 2>/dev/null || true
done
install -m 644 "$LINUX/$APP_ID.desktop" "$STAGE/share/applications/$APP_ID.desktop"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$RECALLCRAFT_BUILD_DATE/g" \
  "$LINUX/$APP_ID.metainfo.xml.in" >"$STAGE/share/metainfo/$APP_ID.metainfo.xml"
install -m 644 "$LINUX/autostart/$APP_ID.desktop" "$STAGE/share/recallcraft/autostart/$APP_ID.desktop"
cp -R "$ROOT/assets/app-icon/hicolor" "$STAGE/share/icons/"
# The speech model (Whisper base.en, MIT), its licence and readme, where the programs look for it
# (../share/recallcraft/models from bin/).
copy_models "$STAGE/share/recallcraft/models"
copy_docs "$STAGE/share/doc/recallcraft"

tar -C "$WORK" -czf "$DIST/$BASENAME.tar.gz" "$BASENAME"
echo "wrote $DIST/$BASENAME.tar.gz"
for bin in recallcraft recallcraft-cli; do
  "$STAGE/bin/$bin" --version
done
echo "==> done"
