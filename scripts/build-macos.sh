#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
[[ "$(uname -s)" == Darwin ]] || { echo 'macOS is required.' >&2; exit 1; }
PROFILE="${1:-debug}"
[[ "$PROFILE" == debug || "$PROFILE" == release ]] || { echo 'Usage: build-macos.sh [debug|release] [--launch]' >&2; exit 1; }
if ! command -v cargo >/dev/null; then
  [[ -f "$HOME/.cargo/env" ]] && source "$HOME/.cargo/env"
fi
command -v cargo >/dev/null || { echo 'Install Rust stable with official rustup (https://rustup.rs).' >&2; exit 1; }
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
xcrun --find clang >/dev/null
xcrun --find metal >/dev/null || { echo 'Install the Xcode Metal Toolchain: xcodebuild -downloadComponent MetalToolchain' >&2; exit 1; }
METAL_CHECK="$(mktemp -d "${TMPDIR:-/tmp}/skylis-metal.XXXXXX")"
trap 'rm -rf "$METAL_CHECK"' EXIT
printf '#include <metal_stdlib>\n' > "$METAL_CHECK/check.metal"
xcrun -sdk macosx metal -c "$METAL_CHECK/check.metal" -o "$METAL_CHECK/check.air" || {
  echo 'Install the Xcode Metal Toolchain: xcodebuild -downloadComponent MetalToolchain' >&2; exit 1;
}
HOST="$(rustc -vV | sed -n 's/^host: //p')"
ARGS=(--locked -p explorer --target "$HOST")
[[ "$PROFILE" == release ]] && ARGS+=(--release)
cargo build "${ARGS[@]}"
# Cargo metadata resolves relative target directories and configured target-dir.
TARGET="$(cargo metadata --locked --no-deps --format-version 1 | /usr/bin/plutil -extract target_directory raw -o - -)"
APP="$TARGET/$HOST/$PROFILE/Skylis.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$TARGET/$HOST/$PROFILE/explorer" "$APP/Contents/MacOS/Skylis"
cp "$ROOT/packaging/macos/Info.plist" "$APP/Contents/Info.plist"
/usr/bin/plutil -lint "$APP/Contents/Info.plist"
/usr/bin/codesign --force --sign - "$APP"
/usr/bin/codesign --verify --deep --strict "$APP"
printf '%s\n' "$APP"
if [[ "${2:-}" == --launch ]]; then /usr/bin/open "$APP"; fi
