#!/usr/bin/env bash
# Build, sign and (optionally) notarize the macOS release artifacts:
#
#   $DIST/recallcraft-<version>-macos-<arch>.dmg       RecallCraft.app (the service, a menu-bar app,
#                                                      with its window and its worker) on a
#                                                      drag-to-Applications DMG
#   $DIST/recallcraft-cli-<version>-macos-<arch>.zip   the command-line program, with the model
#
# Usage: packaging/macos/package.sh [--arch universal|aarch64|x86_64] [--skip-build]
#
# Signing (env):
#   MACOS_SIGN_IDENTITY   codesign identity (name or SHA-1). Default "-" = ad-hoc (local testing;
#                         Gatekeeper will reject the result on other Macs). In CI, import-cert.sh sets it.
#   MACOS_KEYCHAIN        keychain holding the identity (optional)
# Notarization (env; all three needed, and a real identity):
#   APPLE_ID, APPLE_PASSWORD (app-specific password), APPLE_TEAM_ID
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/macos"
ICONS="$ROOT/assets/app-icon"

ARCH=universal
SKIP_BUILD=0
while [ $# -gt 0 ]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '2,17p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$ARCH" in
  universal) TARGETS=(aarch64-apple-darwin x86_64-apple-darwin) ;;
  aarch64) TARGETS=(aarch64-apple-darwin) ;;
  x86_64) TARGETS=(x86_64-apple-darwin) ;;
  *) echo "unknown --arch $ARCH" >&2; exit 2 ;;
esac

# Keep in sync with LSMinimumSystemVersion in Info.plist.in.
export MACOSX_DEPLOYMENT_TARGET=11.0
IDENTITY="${MACOS_SIGN_IDENTITY:--}"
SHORT_VERSION="${VERSION%%-*}"
WORK="$CARGO_TARGET_DIR/macos-package"
APPS="$WORK/apps"
APP="$APPS/RecallCraft.app"
DMG="$DIST/recallcraft-$VERSION-macos-$ARCH.dmg"
CLI_ZIP="$DIST/recallcraft-cli-$VERSION-macos-$ARCH.zip"

NOTARIZE=0
if [ "$IDENTITY" = "-" ]; then
  warn "macOS: ad-hoc signing (no MACOS_SIGN_IDENTITY); artifacts are not notarized"
elif [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
  NOTARIZE=1
else
  warn "macOS: APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID incomplete; signed but not notarized"
fi

echo "==> RecallCraft $VERSION for macOS ($ARCH), identity: $IDENTITY, notarize: $NOTARIZE"

# ---- build -------------------------------------------------------------------------------------
if [ "$SKIP_BUILD" = 0 ]; then
  args=()
  for t in "${TARGETS[@]}"; do args+=(--target "$t"); done
  (cd "$ROOT" && cargo build --release --locked -p recallcraft -p recallcraft-cli "${args[@]}")
fi

rm -rf "$WORK"
mkdir -p "$WORK/bin" "$APPS"
for bin in recallcraft recallcraft-cli; do
  inputs=()
  for t in "${TARGETS[@]}"; do inputs+=("$CARGO_TARGET_DIR/$t/release/$bin"); done
  lipo -create -output "$WORK/bin/$bin" "${inputs[@]}"
  lipo -info "$WORK/bin/$bin"
done

# ---- signing helpers ---------------------------------------------------------------------------
sign() {
  # Hardened runtime + secure timestamp for a real identity; ad-hoc can't be timestamped.
  local ts=(--timestamp)
  [ "$IDENTITY" = "-" ] && ts=(--timestamp=none)
  local kc=()
  [ -n "${MACOS_KEYCHAIN:-}" ] && kc=(--keychain "$MACOS_KEYCHAIN")
  codesign --force --sign "$IDENTITY" ${kc[@]+"${kc[@]}"} "${ts[@]}" "$@"
}

notarize() {
  local file="$1" out id status
  echo "==> notarizing $(basename "$file") (this can take a few minutes)"
  out="$(xcrun notarytool submit "$file" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" --wait --timeout 1h --output-format json)" || true
  echo "$out"
  id="$(printf '%s' "$out" | plutil -extract id raw -o - - 2>/dev/null || true)"
  status="$(printf '%s' "$out" | plutil -extract status raw -o - - 2>/dev/null || true)"
  if [ "$status" != "Accepted" ]; then
    if [ -n "$id" ]; then
      xcrun notarytool log "$id" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" || true
    fi
    echo "error: notarization of $(basename "$file") failed (status: ${status:-unknown})" >&2
    exit 1
  fi
}

# ---- the app -----------------------------------------------------------------------------------
# The executable and the icon carry the display name (CFBundleExecutable / CFBundleIconFile). The
# speech model, its licence and its readme go in Resources/models, where the program looks for
# them. The window (--window) and the worker (--hear) are the same executable, started by the
# service, so the bundle holds one program.
echo "==> assembling $APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$WORK/bin/recallcraft" "$APP/Contents/MacOS/RecallCraft"
cp "$ICONS/recallcraft.icns" "$APP/Contents/Resources/RecallCraft.icns"
copy_models "$APP/Contents/Resources/models"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHORT_VERSION@/$SHORT_VERSION/g" \
  -e "s/@BUILD_SHA@/${RECALLCRAFT_BUILD_SHA:-unknown}/g" \
  "$HERE/Info.plist.in" >"$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
printf 'APPL????' >"$APP/Contents/PkgInfo"
# Sign inside-out: nested code first, then the bundle itself (no --deep on the final signature).
# Today the only nested code is the main executable.
sign --options runtime --entitlements "$HERE/entitlements.plist" "$APP/Contents/MacOS/RecallCraft"
sign --options runtime --entitlements "$HERE/entitlements.plist" "$APP"
codesign --verify --strict --deep --verbose=2 "$APP"

if [ "$NOTARIZE" = 1 ]; then
  ditto -c -k --keepParent "$APP" "$WORK/RecallCraft-notarize.zip"
  notarize "$WORK/RecallCraft-notarize.zip"
  xcrun stapler staple "$APP"
  xcrun stapler validate "$APP"
  spctl --assess --type execute -vvv "$APP"
fi

# ---- DMG ---------------------------------------------------------------------------------------
echo "==> building $DMG"
STAGE="$WORK/dmg"
mkdir -p "$STAGE"
ditto "$APP" "$STAGE/RecallCraft.app"
ln -s /Applications "$STAGE/Applications"
# Finder window layout: background, icon size and positions (packaging/macos/dmg/README.md).
# RecallCraft has no DMG artwork yet: without it the DMG opens as a plain Finder window.
if [ -f "$HERE/dmg/background.tiff" ] && [ -f "$HERE/dmg/dmg-layout.DS_Store" ]; then
  mkdir -p "$STAGE/.background"
  cp "$HERE/dmg/background.tiff" "$STAGE/.background/background.tiff"
  cp "$HERE/dmg/dmg-layout.DS_Store" "$STAGE/.DS_Store"
fi
rm -f "$DMG"
# `create -srcfolder` copies the staged folder as it is. (`makehybrid -hfs`, which builds an image
# without attaching a device, gives every file and folder Finder information on current macOS,
# and the app's signature doesn't hold with that attached: `codesign --verify --strict` on the
# mounted image says "detritus not allowed".) Attaching can fail with "Resource busy" on a CI
# runner, so it is tried a few times.
# The volume name has no version: .DS_Store finds the background through an alias that includes it.
made=0
for attempt in 1 2 3 4 5; do
  if hdiutil create -volname "RecallCraft" -srcfolder "$STAGE" -fs HFS+ -format UDZO -imagekey zlib-level=9 -ov "$DMG"; then
    made=1
    break
  fi
  echo "hdiutil create failed (attempt $attempt of 5); trying again" >&2
  sleep $((attempt * 5))
done
if [ "$made" != 1 ]; then
  echo "error: could not build $DMG" >&2
  exit 1
fi
sign "$DMG"
codesign --verify --strict --verbose=2 "$DMG"
if [ "$NOTARIZE" = 1 ]; then
  notarize "$DMG"
  xcrun stapler staple "$DMG"
  xcrun stapler validate "$DMG"
  spctl --assess --type open --context context:primary-signature -vvv "$DMG"
fi

# ---- CLI ---------------------------------------------------------------------------------------
# With the model in models/ beside it, where `recallcraft-cli hear` looks for it.
echo "==> building $CLI_ZIP"
CLI_DIR="$WORK/recallcraft-cli-$VERSION-macos-$ARCH"
mkdir -p "$CLI_DIR"
cp "$WORK/bin/recallcraft-cli" "$CLI_DIR/"
copy_models "$CLI_DIR/models"
copy_docs "$CLI_DIR"
sign --options runtime "$CLI_DIR/recallcraft-cli"
codesign --verify --strict --verbose=2 "$CLI_DIR/recallcraft-cli"
rm -f "$CLI_ZIP"
ditto -c -k --norsrc --noextattr --keepParent "$CLI_DIR" "$CLI_ZIP"
# A bare Mach-O can't carry a stapled ticket; Gatekeeper looks the notarization up online.
if [ "$NOTARIZE" = 1 ]; then notarize "$CLI_ZIP"; fi

"$WORK/bin/recallcraft-cli" --version
echo "==> done"
ls -lh "$DMG" "$CLI_ZIP"
