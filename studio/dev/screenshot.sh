#!/bin/bash
# Runs Hoopeo Studio on a virtual display (Xvfb), drives it with a self-test script, and
# saves a screenshot. Linux only; needs Xvfb and ImageMagick (import).
#
#   studio/dev/screenshot.sh output           # see dev/selftest.py for the scenarios
#   BIN=target/release/hoopeo-studio studio/dev/screenshot.sh matrix
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
NAME="$1"; SECS="${SECS:-7}"
BIN="${BIN:-$ROOT/target/debug/hoopeo-studio}"
OUT="${OUT:-$ROOT/target/screenshots}"; mkdir -p "$OUT"
DISP=":${DISPNUM:-91}"
Xvfb "$DISP" -screen 0 1440x900x24 >/dev/null 2>&1 & XPID=$!
trap 'kill $XPID 2>/dev/null || true' EXIT
sleep 1
export HOME="$OUT/home-$NAME"; mkdir -p "$HOME"
if [ "$NAME" = splash ]; then EXTRA=(HOOPEO_STUDIO_KEEP_SPLASH=1); else EXTRA=("HOOPEO_STUDIO_SELFTEST=$(python3 "$HERE/selftest.py" "$NAME")"); fi
env DISPLAY="$DISP" WEBKIT_DISABLE_COMPOSITING_MODE=1 "${EXTRA[@]}" "$BIN" > "$OUT/$NAME.log" 2>&1 & APID=$!
sleep "$SECS"
DISPLAY="$DISP" import -window root "$OUT/$NAME.png"
kill $APID 2>/dev/null || true
echo "$OUT/$NAME.png"
