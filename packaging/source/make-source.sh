#!/bin/bash
# Makes the source archive of Project Hoopeo:
#
#   packaging/source/make-source.sh           dist/source/project-hoopeo-<v>-src.tar.gz
#   VENDOR=1 packaging/source/make-source.sh  also includes every dependency (cargo vendor),
#                                             so the archive builds without network access
#
# Build from the archive:  cargo build --release   (add --offline with VENDOR=1)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
NAME="project-hoopeo-$VERSION"
DIST="$ROOT/dist/source"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
mkdir -p "$DIST" "$WORK/$NAME"
tar -C "$ROOT" --exclude=./target --exclude=./dist --exclude=./.git --exclude='*/node_modules' --exclude=./studio/gen -cf - . | tar -C "$WORK/$NAME" -xf -
SUFFIX=src
if [ -n "${VENDOR:-}" ]; then
  (cd "$WORK/$NAME" && mkdir -p .cargo && cargo vendor --locked vendor > .cargo/config.toml)
  SUFFIX=src-vendored
fi
tar -C "$WORK" -czf "$DIST/$NAME-$SUFFIX.tar.gz" "$NAME"
echo "$DIST/$NAME-$SUFFIX.tar.gz"
