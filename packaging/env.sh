# shellcheck shell=bash
# Shared setup for the packaging scripts. Source it: `. "$(dirname "$0")/../env.sh"`.
#
# Exports:
#   ROOT                      workspace root
#   VERSION                   [workspace.package] version from Cargo.toml (override: RECALLCRAFT_VERSION)
#   DIST                      output directory for release artifacts (default: $ROOT/dist/release)
#   RECALLCRAFT_BUILD_SHA     git commit of the build (the macOS bundle records it)
#   RECALLCRAFT_BUILD_DATE    UTC build date, YYYY-MM-DD (the AppStream release date)
#   CARGO_TARGET_DIR          cargo's target dir (default: $ROOT/target)
#
# The two programs every package carries (cargo packages of the same names under apps/):
#   recallcraft       the service (tray icon, the recorder, starts at sign-in), its window
#                     (--window) and the worker that writes speech down (--hear), in one program
#   recallcraft-cli   the command-line program
#
# And the speech model, which every package puts where the programs look for it:
#   models/ beside the programs (Windows), ../share/recallcraft/models (Linux, FreeBSD, the
#   Flatpak and the AppImage), Contents/Resources/models (the macOS app).

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT

# The version lives in exactly one place: `[workspace.package] version` in the root Cargo.toml.
# (`cargo xtask version` prints the same thing; awk avoids compiling xtask here.)
workspace_version() {
  awk '
    /^\[/ { in_pkg = ($0 == "[workspace.package]") ; next }
    in_pkg && $1 == "version" { gsub(/[" ]/, "", $3); print $3; exit }
  ' "$ROOT/Cargo.toml"
}

VERSION="${RECALLCRAFT_VERSION:-$(workspace_version)}"
if [ -z "$VERSION" ]; then
  echo "error: could not read [workspace.package] version from $ROOT/Cargo.toml" >&2
  exit 1
fi
export VERSION

DIST="${DIST:-$ROOT/dist/release}"
mkdir -p "$DIST"
export DIST

if [ -z "${RECALLCRAFT_BUILD_SHA:-}" ]; then
  RECALLCRAFT_BUILD_SHA="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || true)"
fi
export RECALLCRAFT_BUILD_SHA
export RECALLCRAFT_BUILD_DATE="${RECALLCRAFT_BUILD_DATE:-$(date -u +%Y-%m-%d)}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

# The speech model's folder in the source tree, and the files that go with it into every package.
MODELS="$ROOT/assets/models"
MODEL_FILES=(whisper-base.en-q8_0.gguf LICENSE-WHISPER.txt README.md)

# Emit a GitHub Actions warning (plain stderr outside Actions).
warn() {
  if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::warning::$*"; else echo "warning: $*" >&2; fi
}

# Copy the speech model and its licence and readme into a folder (made if missing).
copy_models() {
  local dest="$1" f
  mkdir -p "$dest"
  for f in "${MODEL_FILES[@]}"; do
    if [ ! -f "$MODELS/$f" ]; then
      echo "error: a model file is missing: $MODELS/$f" >&2
      exit 1
    fi
    cp "$MODELS/$f" "$dest/$f"
    chmod 644 "$dest/$f"
  done
}

# Copy the licence, attribution and readme files that exist into a package directory.
copy_docs() {
  local dest="$1" f
  for f in README.md PRIVACY.md LICENSE LICENSE-MIT LICENSE-APACHE COPYRIGHT ATTRIBUTION.md NOTICE; do
    if [ -f "$ROOT/$f" ]; then cp "$ROOT/$f" "$dest/"; fi
  done
}

# Portable SHA-256 of a file (prints just the hash).
sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}
