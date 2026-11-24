#!/usr/bin/env bash
# Write RecallCraft's icon in every size and format the packages use: the Windows .ico, the macOS
# .icns and the Linux hicolor tree, in assets/app-icon/.
#
# The drawing is in code (crates/desktop/src/icon.rs, which the tray and the window use too), so
# this needs nothing but cargo. The outputs are committed, so builds and packaging never need to
# run it. Change the drawing, run this, check the result with packaging/check-icons.sh, and commit
# what changed (every file is listed in ATTRIBUTION.md).
#
#   packaging/icons.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
cargo run -q -p xtask -- icon
bash packaging/check-icons.sh
