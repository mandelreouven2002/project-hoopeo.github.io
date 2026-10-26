#!/bin/bash
# Builds the macOS installers of Project Hoopeo. Run it on a Mac, from anywhere:
#
#   packaging/macos/build-pkg.sh                 # this Mac's architecture
#   ARCH=universal packaging/macos/build-pkg.sh  # Apple silicon and Intel in one binary
#
# Output, in dist/macos/:
#   Project-Hoopeo-<version>-macos-<arch>.pkg   everything, with choices (Customize)
#   Hoopeo-Studio-<version>-macos-<arch>.pkg    Hoopeo Studio only
#   DEP-CLI-<version>-macos-<arch>.pkg          DEP CLI only
#   DEP-Engine-<version>-macos-<arch>.pkg       DEP Engine server, with the background service
#   Hoopeo Studio.app, DEP CLI.app              the app bundles, for a quick look
#
# Optional signing and notarization (Apple Developer ID):
#   SIGN_APP="Developer ID Application: Name (TEAMID)"
#   SIGN_PKG="Developer ID Installer: Name (TEAMID)"
#   NOTARY_PROFILE=<a profile stored with: xcrun notarytool store-credentials>
#
# Requirements: Xcode command line tools, Rust (rustup), Python 3 with Pillow only if the
# icons must be regenerated (they are committed in packaging/icons).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
ARCH="${ARCH:-$(uname -m)}"
DIST="$ROOT/dist/macos"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

case "$ARCH" in
  arm64|aarch64) TARGETS=(aarch64-apple-darwin); ARCHS="arm64"; ARCH=arm64 ;;
  x86_64)        TARGETS=(x86_64-apple-darwin);  ARCHS="x86_64" ;;
  universal)     TARGETS=(aarch64-apple-darwin x86_64-apple-darwin); ARCHS="arm64,x86_64" ;;
  *) echo "ARCH must be arm64, x86_64 or universal" >&2; exit 2 ;;
esac

echo "Project Hoopeo $VERSION for macOS ($ARCH)"
mkdir -p "$DIST"

# ------------------------------------------------------------------ build
export MACOSX_DEPLOYMENT_TARGET=11.0
for t in "${TARGETS[@]}"; do
  rustup target add "$t" >/dev/null
  cargo build --release --locked --manifest-path "$ROOT/Cargo.toml" --target "$t" -p dep-engine -p dep-cli -p hoopeo-studio
done
BIN="$WORK/bin"; mkdir -p "$BIN"
for b in dep-engine dep-cli hoopeo-studio; do
  if [ "${#TARGETS[@]}" -gt 1 ]; then
    lipo -create -output "$BIN/$b" $(for t in "${TARGETS[@]}"; do echo "$ROOT/target/$t/release/$b"; done)
  else
    cp "$ROOT/target/${TARGETS[0]}/release/$b" "$BIN/$b"
  fi
done

sign() {  # sign <path> : hardened runtime, if a Developer ID is set
  if [ -n "${SIGN_APP:-}" ]; then codesign --force --timestamp --options runtime --sign "$SIGN_APP" "$1"; fi
}

licenses() {  # licenses <dir>: the license documents that go beside the binaries
  mkdir -p "$1/LICENSES"
  cp "$ROOT/NOTICE" "$ROOT/LICENSING.md" "$ROOT/public/BRAND.md" "$1/"
  cp "$ROOT/LICENSES/LicenseRef-Hoopeo-Evaluation.txt" "$ROOT/LICENSES/LicenseRef-Hoopeo-Commercial.txt" "$1/LICENSES/"
  if [ -f "$ROOT/THIRD-PARTY-NOTICES" ]; then cp "$ROOT/THIRD-PARTY-NOTICES" "$1/"; fi
  return 0
}

plist() {  # plist <file> <name> <id> <exe> <icon> <category>
  cat > "$1" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$2</string>
  <key>CFBundleDisplayName</key><string>$2</string>
  <key>CFBundleIdentifier</key><string>$3</string>
  <key>CFBundleExecutable</key><string>$4</string>
  <key>CFBundleIconFile</key><string>$5</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSApplicationCategoryType</key><string>$6</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>(c) Reuven Mandelovich, lp.reuven.click</string>
</dict>
</plist>
EOF
}

# ------------------------------------------------------------------ Hoopeo Studio.app
STUDIO_ROOT="$WORK/studio-root"
APP="$STUDIO_ROOT/Applications/Hoopeo Studio.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN/hoopeo-studio" "$APP/Contents/MacOS/hoopeo-studio"
cp "$ROOT/packaging/icons/hoopeo-studio/icon.icns" "$APP/Contents/Resources/icon.icns"
licenses "$APP/Contents/Resources"
plist "$APP/Contents/Info.plist" "Hoopeo Studio" click.reuven.hoopeo.studio hoopeo-studio icon.icns public.app-category.developer-tools
sign "$APP/Contents/MacOS/hoopeo-studio"; sign "$APP"

# ------------------------------------------------------------------ DEP CLI: binary, app, examples
CLI_ROOT="$WORK/cli-root"
mkdir -p "$CLI_ROOT/usr/local/bin" "$CLI_ROOT/usr/local/libexec/project-hoopeo" "$CLI_ROOT/usr/local/share/project-hoopeo"
cp "$BIN/dep-cli" "$CLI_ROOT/usr/local/bin/dep-cli"
cp "$HERE/files/dep-cli-shell.command" "$CLI_ROOT/usr/local/libexec/project-hoopeo/"
cp -R "$ROOT/public/examples" "$CLI_ROOT/usr/local/share/project-hoopeo/examples"
cp "$HERE/files/uninstall.sh" "$CLI_ROOT/usr/local/share/project-hoopeo/uninstall.sh"
licenses "$CLI_ROOT/usr/local/share/project-hoopeo"
CLIAPP="$CLI_ROOT/Applications/DEP CLI.app"
mkdir -p "$CLIAPP/Contents/MacOS" "$CLIAPP/Contents/Resources"
cp "$HERE/files/dep-cli-launcher" "$CLIAPP/Contents/MacOS/dep-cli-launcher"
cp "$ROOT/packaging/icons/dep-cli/icon.icns" "$CLIAPP/Contents/Resources/icon.icns"
plist "$CLIAPP/Contents/Info.plist" "DEP CLI" click.reuven.hoopeo.dep-cli.app dep-cli-launcher icon.icns public.app-category.developer-tools
sign "$CLI_ROOT/usr/local/bin/dep-cli"; sign "$CLIAPP"

# ------------------------------------------------------------------ DEP Engine and its service
ENGINE_ROOT="$WORK/engine-root"
mkdir -p "$ENGINE_ROOT/usr/local/bin" "$ENGINE_ROOT/usr/local/share/project-hoopeo"
cp "$BIN/dep-engine" "$ENGINE_ROOT/usr/local/bin/dep-engine"
cp "$HERE/files/uninstall.sh" "$ENGINE_ROOT/usr/local/share/project-hoopeo/uninstall.sh"
licenses "$ENGINE_ROOT/usr/local/share/project-hoopeo"
sign "$ENGINE_ROOT/usr/local/bin/dep-engine"
SERVICE_ROOT="$WORK/service-root"
mkdir -p "$SERVICE_ROOT/Library/LaunchAgents" "$SERVICE_ROOT/usr/local/libexec/project-hoopeo"
cp "$HERE/files/click.reuven.hoopeo.dep-engine.plist" "$SERVICE_ROOT/Library/LaunchAgents/"
cp "$HERE/files/dep-engine-agent" "$SERVICE_ROOT/usr/local/libexec/project-hoopeo/"

# ------------------------------------------------------------------ component packages
COMP="$WORK/components"; mkdir -p "$COMP"
component() {  # component <root> <id> <file> [scripts]
  local scripts=()
  [ -n "${4:-}" ] && scripts=(--scripts "$4")
  pkgbuild --root "$1" --identifier "$2" --version "$VERSION" --install-location / --ownership recommended ${scripts[@]+"${scripts[@]}"} "$COMP/$3"
}
# Keep the app bundles where they are installed (no relocation to a copy elsewhere).
for r in "$STUDIO_ROOT" "$CLI_ROOT"; do
  pkgbuild --analyze --root "$r" "$WORK/$(basename "$r").plist" >/dev/null
  /usr/libexec/PlistBuddy -c "Set :0:BundleIsRelocatable false" "$WORK/$(basename "$r").plist" 2>/dev/null || true
done
pkgbuild --root "$STUDIO_ROOT" --identifier click.reuven.hoopeo.studio --version "$VERSION" --install-location / --component-plist "$WORK/studio-root.plist" "$COMP/hoopeo-studio.pkg"
pkgbuild --root "$CLI_ROOT" --identifier click.reuven.hoopeo.dep-cli --version "$VERSION" --install-location / --component-plist "$WORK/cli-root.plist" --scripts "$HERE/scripts/cli" "$COMP/dep-cli.pkg"
component "$ENGINE_ROOT" click.reuven.hoopeo.dep-engine dep-engine.pkg
component "$SERVICE_ROOT" click.reuven.hoopeo.dep-engine-service dep-engine-service.pkg "$HERE/scripts/engine-service"

# ------------------------------------------------------------------ products
RES="$WORK/resources"; mkdir -p "$RES"
for f in welcome.html conclusion.html; do sed "s/__VERSION__/$VERSION/g" "$HERE/resources/$f" > "$RES/$f"; done
cp "$HERE/resources/background.png" "$HERE/resources/background-dark.png" "$RES/"
# The license page of the installer: the Evaluation License, then the NOTICE.
{ cat "$ROOT/LICENSES/LicenseRef-Hoopeo-Evaluation.txt"; printf '\n\n--------------------------------------------------------------------------------\n\n'; cat "$ROOT/NOTICE"; } > "$RES/LICENSE.txt"

product() {  # product <distribution.xml> <output name>
  local out="$DIST/$2"
  local sign_args=()
  [ -n "${SIGN_PKG:-}" ] && sign_args=(--sign "$SIGN_PKG")
  productbuild --distribution "$1" --resources "$RES" --package-path "$COMP" ${sign_args[@]+"${sign_args[@]}"} "$out"
  if [ -n "${NOTARY_PROFILE:-}" ]; then
    xcrun notarytool submit "$out" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$out"
  fi
  echo "  $out"
}

sed -e "s/__VERSION__/$VERSION/g" -e "s/__ARCHS__/$ARCHS/g" "$HERE/distribution.xml" > "$WORK/all.xml"
product "$WORK/all.xml" "Project-Hoopeo-$VERSION-macos-$ARCH.pkg"

single() {  # single <title> <pkg-ref ids...> -> a distribution with fixed choices
  local title="$1"; shift
  {
    echo '<?xml version="1.0" encoding="utf-8"?>'
    echo '<installer-gui-script minSpecVersion="2">'
    echo "  <title>$title</title>"
    echo '  <welcome file="welcome.html" mime-type="text/html"/><license file="LICENSE.txt" mime-type="text/plain"/><conclusion file="conclusion.html" mime-type="text/html"/>'
    echo "  <options customize=\"never\" require-scripts=\"false\" hostArchitectures=\"$ARCHS\"/>"
    echo '  <domains enable_localSystem="true"/>'
    echo '  <choices-outline>'
    for id in "$@"; do echo "    <line choice=\"$id\"/>"; done
    echo '  </choices-outline>'
    for id in "$@"; do
      local file; file="$(sed -n "s/.*id=\"$id\" version=\"__VERSION__\">\(.*\)<\/pkg-ref>.*/\1/p" "$HERE/distribution.xml")"
      echo "  <choice id=\"$id\" title=\"$title\"><pkg-ref id=\"$id\"/></choice>"
      echo "  <pkg-ref id=\"$id\" version=\"$VERSION\">$file</pkg-ref>"
    done
    echo '</installer-gui-script>'
  } > "$WORK/single.xml"
  product "$WORK/single.xml" "$(echo "$title" | tr ' ' '-')-$VERSION-macos-$ARCH.pkg"
}
single "Hoopeo Studio" click.reuven.hoopeo.studio
single "DEP CLI" click.reuven.hoopeo.dep-cli
single "DEP Engine" click.reuven.hoopeo.dep-engine click.reuven.hoopeo.dep-engine-service

rm -rf "$DIST/Hoopeo Studio.app" "$DIST/DEP CLI.app"
cp -R "$APP" "$CLIAPP" "$DIST/"
echo "Done: $DIST"
