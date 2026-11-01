#!/bin/bash
# Builds the Linux packages of Project Hoopeo, for this machine's architecture:
#
#   packaging/linux/build.sh
#
# Output, in dist/linux/:
#   dep-engine_<v>_<arch>.deb              DEP Engine as a systemd service (Debian, Ubuntu)
#   dep-cli_<v>_<arch>.deb                 DEP CLI, with DEP Engine built in
#   hoopeo-studio_<v>_<arch>.deb           Hoopeo Studio (needs WebKitGTK 4.1)
#   dep-engine-<v>-linux-<arch>.tar.gz     DEP Engine and DEP CLI, with install.sh (any systemd distribution)
#   dep-cli-<v>-linux-<arch>.tar.gz        DEP CLI only
#
# SKIP_STUDIO=1 skips Hoopeo Studio (a server build machine without WebKitGTK).
# NO_BUILD=1 packages the binaries already in target/release.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
DEB_ARCH="$(dpkg --print-architecture 2>/dev/null || echo amd64)"
ARCH="$(uname -m)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
DIST="$ROOT/dist/linux"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$DIST"

PKGS=(-p dep-engine -p dep-cli)
[ -z "${SKIP_STUDIO:-}" ] && PKGS+=(-p hoopeo-studio)
if [ -z "${NO_BUILD:-}" ]; then
  cargo build --release --locked --manifest-path "$ROOT/Cargo.toml" "${PKGS[@]}"
fi
REL="$TARGET_DIR/release"
echo "Project Hoopeo $VERSION for Linux ($ARCH)"

control() {  # control <dir> <package> <description> <depends>
  mkdir -p "$1/DEBIAN"
  local size; size="$(du -sk "$1" | cut -f1)"
  {
    echo "Package: $2"
    echo "Version: $VERSION"
    echo "Architecture: $DEB_ARCH"
    echo "Maintainer: Reuven Mandelovich <https://lp.reuven.click>"
    echo "Installed-Size: $size"
    [ -n "$4" ] && echo "Depends: $4"
    echo "Section: database"
    echo "Priority: optional"
    echo "Homepage: https://lp.reuven.click"
    echo "Description: $3"
  } > "$1/DEBIAN/control"
}

licenses() {  # licenses <dir>: the license documents that go beside every binary
  mkdir -p "$1/LICENSES"
  cp "$ROOT/NOTICE" "$ROOT/LICENSING.md" "$ROOT/BRAND.md" "$1/"
  cp "$ROOT/LICENSES/BUSL-1.1.txt" "$ROOT/LICENSES/LicenseRef-Hoopeo-Commercial.txt" "$1/LICENSES/"
  if [ -f "$ROOT/THIRD-PARTY-NOTICES" ]; then cp "$ROOT/THIRD-PARTY-NOTICES" "$1/"; fi
  return 0
}

docs() {  # docs <root> <package> [examples dir inside the package]
  licenses "$1/usr/share/doc/$2"
  # A machine-readable Debian copyright file (https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/)
  {
    echo "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/"
    echo "Upstream-Name: Project Hoopeo ($2)"
    echo "Upstream-Contact: Reuven Mandelovich <https://lp.reuven.click>"
    echo "Source: https://github.com/mandelreouven2002/project-hoopeo.github.io"
    echo "Comment: The source code of DEP Engine, DEP CLI and Hoopeo Studio is public, at"
    echo " the address above, under the Business Source License 1.1. The third-party"
    echo " components in the binaries keep their own licenses: see THIRD-PARTY-NOTICES"
    echo " in this directory."
    echo
    echo "Files: *"
    echo "Copyright: 2026 Reuven Mandelovich"
    echo "License: BUSL-1.1"
    if [ -n "${3:-}" ]; then
      echo
      echo "Files: $3/*"
      echo "Copyright: 2026 Reuven Mandelovich"
      echo "License: Apache-2.0"
    fi
    echo
    echo "License: BUSL-1.1"
    sed -e 's/^$/./' -e 's/^/ /' "$ROOT/LICENSES/BUSL-1.1.txt"
    if [ -n "${3:-}" ]; then
      echo
      echo "License: Apache-2.0"
      echo " On Debian systems, the full text of the Apache License 2.0 is in"
      echo " /usr/share/common-licenses/Apache-2.0."
    fi
  } > "$1/usr/share/doc/$2/copyright"
  return 0
}

deb() {  # deb <root> <file>
  find "$1" -type d -exec chmod 0755 {} +
  dpkg-deb --root-owner-group --build "$1" "$DIST/$2" >/dev/null
  echo "  $DIST/$2"
}

# ------------------------------------------------------------------ dep-engine.deb
R="$WORK/dep-engine"
install -Dm0755 "$REL/dep-engine" "$R/usr/bin/dep-engine"
install -Dm0644 "$HERE/files/dep-engine.service" "$R/lib/systemd/system/dep-engine.service"
install -Dm0640 "$HERE/files/dep-engine.env" "$R/etc/dep-engine/dep-engine.env"
mkdir -p "$R/usr/share/dep-engine"; cp -R "$ROOT/examples" "$R/usr/share/dep-engine/examples"
docs "$R" dep-engine usr/share/dep-engine/examples
control "$R" dep-engine "DEP Engine, the Database-Enforced Privacy server
 A SQL database server that enforces privacy rules in the engine: protected
 columns are stored as blind indexes and ciphertexts, matched only by equality,
 and released only through declared functions. Speaks the PostgreSQL wire
 protocol, with TLS. Part of Project Hoopeo." "adduser, libc6 (>= 2.34)"
echo "/etc/dep-engine/dep-engine.env" > "$R/DEBIAN/conffiles"
for s in postinst prerm postrm; do install -m0755 "$HERE/debian/dep-engine.$s" "$R/DEBIAN/$s"; done
deb "$R" "dep-engine_${VERSION}_${DEB_ARCH}.deb"

# ------------------------------------------------------------------ dep-cli.deb
R="$WORK/dep-cli"
install -Dm0755 "$REL/dep-cli" "$R/usr/bin/dep-cli"
mkdir -p "$R/usr/share/dep-cli"; cp -R "$ROOT/examples" "$R/usr/share/dep-cli/examples"
docs "$R" dep-cli usr/share/dep-cli/examples
control "$R" dep-cli "DEP CLI, the command-line tool of Project Hoopeo
 A shell for DEP SQL, with DEP Engine built in: it opens a local database
 directly, or connects to a DEP Engine server over the PostgreSQL wire
 protocol, with TLS. It prints the DEP notes of each statement." "libc6 (>= 2.34)"
deb "$R" "dep-cli_${VERSION}_${DEB_ARCH}.deb"

# ------------------------------------------------------------------ hoopeo-studio.deb
if [ -z "${SKIP_STUDIO:-}" ]; then
  R="$WORK/hoopeo-studio"
  install -Dm0755 "$REL/hoopeo-studio" "$R/usr/bin/hoopeo-studio"
  install -Dm0644 "$HERE/files/hoopeo-studio.desktop" "$R/usr/share/applications/hoopeo-studio.desktop"
  for s in 32 64 128 256 512; do
    install -Dm0644 "$ROOT/packaging/icons/hoopeo-studio/${s}x${s}.png" "$R/usr/share/icons/hicolor/${s}x${s}/apps/hoopeo-studio.png"
  done
  docs "$R" hoopeo-studio
  control "$R" hoopeo-studio "Hoopeo Studio, the developer application for DEP SQL
 An IDE for Database-Enforced Privacy, in the spirit of PL/SQL Developer: SQL
 windows, an object browser, result grids, explain plans, the privacy matrix,
 and the DEP notes of every statement. DEP Engine is built in, for local
 databases; it also connects to DEP Engine servers, with TLS." "libwebkit2gtk-4.1-0, libgtk-3-0, libc6 (>= 2.34)"
  deb "$R" "hoopeo-studio_${VERSION}_${DEB_ARCH}.deb"
fi

# ------------------------------------------------------------------ tarballs
T="$WORK/dep-engine-$VERSION-linux-$ARCH"
mkdir -p "$T/bin" "$T/etc"
cp "$REL/dep-engine" "$REL/dep-cli" "$T/bin/"
cp "$HERE/files/dep-engine.service" "$HERE/files/dep-engine.env" "$T/etc/"
cp "$HERE/files/install.sh" "$T/install.sh"
cp -R "$ROOT/examples" "$T/examples"
licenses "$T"
cat > "$T/README.txt" <<EOF
DEP Engine $VERSION for Linux ($ARCH), Project Hoopeo

  sudo ./install.sh              install as a systemd service and start it
  ./bin/dep-engine --help        or run it yourself, for example:
  ./bin/dep-engine --data-dir ./data --listen 127.0.0.1:5433

DEP CLI is included: ./bin/dep-cli --help
(c) Reuven Mandelovich, lp.reuven.click
EOF
tar -C "$WORK" -czf "$DIST/dep-engine-$VERSION-linux-$ARCH.tar.gz" "$(basename "$T")"
echo "  $DIST/dep-engine-$VERSION-linux-$ARCH.tar.gz"

T="$WORK/dep-cli-$VERSION-linux-$ARCH"
mkdir -p "$T"
cp "$REL/dep-cli" "$T/"
cp -R "$ROOT/examples" "$T/examples"
licenses "$T"
printf 'DEP CLI %s for Linux (%s), Project Hoopeo\n\n  ./dep-cli --memory --init examples/clinic.sql   try the demo clinic\n  ./dep-cli -h HOST -p 5433 -U admin -d operational --sslmode require\n\nCopy dep-cli to a directory on your PATH, for example /usr/local/bin.\n(c) Reuven Mandelovich, lp.reuven.click\n' "$VERSION" "$ARCH" > "$T/README.txt"
tar -C "$WORK" -czf "$DIST/dep-cli-$VERSION-linux-$ARCH.tar.gz" "$(basename "$T")"
echo "  $DIST/dep-cli-$VERSION-linux-$ARCH.tar.gz"
echo "Done: $DIST"
