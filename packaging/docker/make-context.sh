#!/bin/bash
# Makes a Docker build context for DEP Engine from the release binaries, without the source:
#
#   packaging/docker/make-context.sh     dist/docker/dep-engine-<v>-docker-<arch>.tar.gz
#
# Unpack it anywhere with Docker, and in the folder:  docker build -t dep-engine:<v> .
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
ARCH="$(uname -m)"
REL="${CARGO_TARGET_DIR:-$ROOT/target}/release"
NAME="dep-engine-$VERSION-docker-$ARCH"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
case "$ARCH" in x86_64) DARCH=amd64 ;; aarch64|arm64) DARCH=arm64 ;; *) DARCH="$ARCH" ;; esac
C="$WORK/$NAME"; mkdir -p "$C/bin/$DARCH" "$ROOT/dist/docker"
cp "$REL/dep-engine" "$REL/dep-cli" "$C/bin/$DARCH/"
cp "$HERE/Dockerfile.binary" "$C/Dockerfile"
cp -R "$ROOT/examples" "$C/examples"
cp "$ROOT/NOTICE" "$ROOT/LICENSING.md" "$ROOT/BRAND.md" "$C/"; mkdir -p "$C/LICENSES"
cp "$ROOT/LICENSES/BUSL-1.1.txt" "$ROOT/LICENSES/LicenseRef-Hoopeo-Commercial.txt" "$C/LICENSES/"
if [ -f "$ROOT/THIRD-PARTY-NOTICES" ]; then cp "$ROOT/THIRD-PARTY-NOTICES" "$C/"; else echo "Run cargo about first (see packaging/about.toml)." > "$C/THIRD-PARTY-NOTICES"; fi
sed -n '1,200p' "$HERE/README.md" > "$C/README.md"
cat > "$C/compose.yaml" <<YAML
# In this folder:  echo 'DEP_ADMIN_PASSWORD=change-me' > .env && docker compose up -d
services:
  dep-engine:
    build: .
    image: dep-engine:$VERSION
    restart: unless-stopped
    ports:
      - "127.0.0.1:5433:5433"   # remove 127.0.0.1: to accept connections from the network
    environment:
      DEP_ADMIN_PASSWORD: \${DEP_ADMIN_PASSWORD:?set DEP_ADMIN_PASSWORD in .env}
      DEP_TLS_SELF_SIGNED: "1"
    volumes:
      - dep-data:/var/lib/dep-engine
volumes:
  dep-data:
YAML
tar -C "$WORK" -czf "$ROOT/dist/docker/$NAME.tar.gz" "$NAME"
echo "$ROOT/dist/docker/$NAME.tar.gz"
