#!/bin/sh
# Installs the latest release of Project Hoopeo: DEP Engine, DEP CLI and Hoopeo Studio.
#
#   curl -fsSL https://mandelreouven2002.github.io/project-hoopeo.github.io/install.sh | sh -s -- [PRODUCT...]
#
# PRODUCT is cli, engine, studio or all (default: cli). On Linux it uses the .deb packages where apt exists,
# and the archives otherwise; on macOS, the .pkg installers. Every file is checked against the release's
# SHA256SUMS before it is installed. Project Hoopeo is an experimental, conceptual project: see the terms of
# use on the website. Licensed under the Apache License 2.0.
set -eu

REPO="mandelreouven2002/project-hoopeo.github.io"
BASE="${HOOPEO_RELEASE_URL:-https://github.com/$REPO/releases/latest/download}"

say() { printf '%s\n' "$*"; }
die() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "$1 is needed"; }

need curl
[ "$#" -gt 0 ] || set -- cli
WANT=""
for p in "$@"; do
  case "$p" in
    cli|engine|studio) WANT="$WANT $p" ;;
    all) WANT=" engine cli studio" ;;
    *) die "unknown product '$p' (choose cli, engine, studio or all)" ;;
  esac
done

SUDO=""
if [ "$(id -u)" != 0 ]; then need sudo; SUDO="sudo"; fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
cd "$TMP"

curl -fsSL -o SHA256SUMS "$BASE/SHA256SUMS" || die "could not download the release's SHA256SUMS"

fetch() {   # fetch FILE: download it and check it against SHA256SUMS
  say "Downloading $1"
  curl -fL --progress-bar -o "$1" "$BASE/$1" || die "could not download $1"
  sum="$(grep " [* ]*$1\$" SHA256SUMS | awk '{print $1}')"
  [ -n "$sum" ] || die "$1 is not listed in SHA256SUMS"
  if command -v sha256sum >/dev/null 2>&1; then got="$(sha256sum "$1" | awk '{print $1}')"
  else got="$(shasum -a 256 "$1" | awk '{print $1}')"; fi
  [ "$got" = "$sum" ] || die "$1 does not match SHA256SUMS; nothing was installed"
}

case "$(uname -s)" in
  Darwin)
    if [ "$(echo "$WANT" | wc -w)" -gt 1 ]; then PKG="Project-Hoopeo-macos-universal.pkg"
    else case "$WANT" in
      *cli) PKG="DEP-CLI-macos-universal.pkg" ;;
      *engine) PKG="DEP-Engine-macos-universal.pkg" ;;
      *studio) PKG="Hoopeo-Studio-macos-universal.pkg" ;;
    esac; fi
    fetch "$PKG"
    say "Installing $PKG (your password may be asked)"
    $SUDO installer -pkg "$PKG" -target /
    ;;
  Linux)
    case "$(uname -m)" in
      x86_64|amd64) DEB=amd64; TAR=x86_64 ;;
      aarch64|arm64) DEB=arm64; TAR=aarch64 ;;
      *) die "no build for $(uname -m)" ;;
    esac
    if command -v apt-get >/dev/null 2>&1; then
      FILES=""
      for p in $WANT; do
        case "$p" in
          cli) f="dep-cli_$DEB.deb" ;; engine) f="dep-engine_$DEB.deb" ;; studio) f="hoopeo-studio_$DEB.deb" ;;
        esac
        fetch "$f"; FILES="$FILES ./$f"
      done
      say "Installing$FILES"
      # shellcheck disable=SC2086
      $SUDO apt-get install -y $FILES
    else
      for p in $WANT; do
        case "$p" in
          engine)
            fetch "dep-engine-linux-$TAR.tar.gz"; tar xzf "dep-engine-linux-$TAR.tar.gz"
            $SUDO sh ./dep-engine-*-linux-"$TAR"/install.sh ;;
          cli)
            fetch "dep-cli-linux-$TAR.tar.gz"; tar xzf "dep-cli-linux-$TAR.tar.gz"
            $SUDO install -m 0755 ./dep-cli-*-linux-"$TAR"/dep-cli /usr/local/bin/dep-cli
            $SUDO mkdir -p /usr/local/share/project-hoopeo
            $SUDO cp -R ./dep-cli-*-linux-"$TAR"/examples /usr/local/share/project-hoopeo/
            say "DEP CLI is in /usr/local/bin/dep-cli, the examples in /usr/local/share/project-hoopeo/examples" ;;
          studio) die "Hoopeo Studio is packaged for Debian and Ubuntu (apt) only" ;;
        esac
      done
    fi
    ;;
  *) die "no build for $(uname -s); see the Install page for Docker and the playground" ;;
esac

say ""
say "Done. Project Hoopeo is experimental: see the terms of use on the website."
case "$WANT" in *cli*) say "Try it:  dep-cli --help" ;; esac
