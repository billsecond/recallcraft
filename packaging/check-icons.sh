#!/usr/bin/env bash
# Check that every app icon file the packages use is there, and that each PNG is the size its
# folder says. Run by packaging-lint (storytold/craft-common-actions) when present; also by
# packaging/icons.sh after writing them.
#
#   packaging/check-icons.sh
#
# RecallCraft is one program with one icon: recallcraft.ico (Windows), recallcraft.icns (macOS)
# and io.github.billsecond.recallcraft.png in the hicolor sizes (Linux, FreeBSD, the Flatpak and
# the AppImage).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ICONS="$ROOT/assets/app-icon"
APP_ID=io.github.billsecond.recallcraft

failed=0
fail() { echo "error: $*" >&2; failed=1; }

# The first bytes of a file as hex.
magic() { od -An -tx1 -N"$2" "$1" | tr -d ' \n'; }

# Width and height of a PNG from its IHDR chunk ("W H"); nothing when the file isn't a PNG.
png_size() {
  [ "$(magic "$1" 8)" = 89504e470d0a1a0a ] || return 0
  od -An -tu1 -j16 -N8 "$1" | awk '{ print $1 * 16777216 + $2 * 65536 + $3 * 256 + $4, $5 * 16777216 + $6 * 65536 + $7 * 256 + $8 }'
}

# check_file <path> <magic hex>; check_png <path> <size>
check_file() {
  if [ ! -s "$1" ]; then fail "missing ${1#"$ROOT"/}"; return; fi
  [ "$(magic "$1" $((${#2} / 2)))" = "$2" ] || fail "${1#"$ROOT"/} doesn't start like its kind of file"
}
check_png() {
  local got
  if [ ! -s "$1" ]; then fail "missing ${1#"$ROOT"/}"; return; fi
  got="$(png_size "$1")"
  [ "$got" = "$2 $2" ] || fail "${1#"$ROOT"/} is ${got:-not a PNG}, expected $2 x $2"
}

# Windows (the program's resources, the MSI and its shortcut) and macOS (the app bundle).
check_file "$ICONS/recallcraft.ico" 00000100
check_file "$ICONS/recallcraft.icns" 69636e73
# Linux and FreeBSD (hicolor theme), the Flatpak and the AppImage.
for size in 16 24 32 48 64 128 256 512; do
  check_png "$ICONS/hicolor/${size}x${size}/apps/$APP_ID.png" "$size"
done

if [ "$failed" != 0 ]; then
  echo "app icons incomplete: run packaging/icons.sh (cargo xtask icon) and commit the result" >&2
  exit 1
fi
echo "app icons ok"
