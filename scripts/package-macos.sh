#!/usr/bin/env bash
#
# Package mise-hub as a macOS .app bundle.
#
#   scripts/package-macos.sh                 # build + bundle into dist/
#   scripts/package-macos.sh --dmg           # also produce dist/mise-hub-<version>.dmg
#   scripts/package-macos.sh --target aarch64-apple-darwin
#   scripts/package-macos.sh --no-build      # reuse an existing release build
#
# The bundle is NOT signed and NOT notarized: without a Developer ID
# certificate macOS Gatekeeper blocks the first launch, and the user has
# to open it via right-click → Open (or `xattr -dr com.apple.quarantine`).
# Add signing to this script once a certificate is available.
#
# The version is read from the workspace Cargo.toml — the bundle can never
# claim a version the binary does not have.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

APP_NAME="mise-hub"
BUNDLE_ID="dev.misehub.app"
TARGET=""
MAKE_DMG=0
DO_BUILD=1

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dmg)
      MAKE_DMG=1
      shift
      ;;
    --target)
      TARGET="$2"
      shift 2
      ;;
    --no-build)
      DO_BUILD=0
      shift
      ;;
    -h | --help)
      sed -n '2,20p' "${BASH_SOURCE[0]}"
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

# --- version ---------------------------------------------------------------
VERSION="$(awk '
  /^\[workspace\.package\]/ { in_pkg = 1; next }
  /^\[/ { in_pkg = 0 }
  in_pkg && /^version[[:space:]]*=/ {
    if (match($0, /"[0-9][^"]*"/)) { print substr($0, RSTART + 1, RLENGTH - 2); exit }
  }
' Cargo.toml)"

if [[ -z "$VERSION" ]]; then
  echo "could not read the workspace version from Cargo.toml" >&2
  exit 1
fi

# --- build -----------------------------------------------------------------
if [[ $DO_BUILD -eq 1 ]]; then
  if [[ -n "$TARGET" ]]; then
    cargo build --release -p mise-hub --target "$TARGET"
  else
    cargo build --release -p mise-hub
  fi
fi

BIN_DIR="target/release"
if [[ -n "$TARGET" ]]; then
  BIN_DIR="target/$TARGET/release"
fi
BINARY="$BIN_DIR/$APP_NAME"

if [[ ! -x "$BINARY" ]]; then
  echo "binary not found: $BINARY" >&2
  exit 1
fi

# --- bundle ----------------------------------------------------------------
DIST="dist"
APP="$DIST/$APP_NAME.app"
CONTENTS="$APP/Contents"

rm -rf "$APP"
mkdir -p "$CONTENTS/MacOS" "$CONTENTS/Resources"
cp "$BINARY" "$CONTENTS/MacOS/$APP_NAME"
chmod +x "$CONTENTS/MacOS/$APP_NAME"

cat >"$CONTENTS/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>$APP_NAME</string>
    <key>CFBundleDisplayName</key>
    <string>$APP_NAME</string>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleExecutable</key>
    <string>$APP_NAME</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleVersion</key>
    <string>$VERSION</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

echo "bundled: $APP (version $VERSION)"

# --- optional dmg ----------------------------------------------------------
if [[ $MAKE_DMG -eq 1 ]]; then
  DMG="$DIST/$APP_NAME-$VERSION.dmg"
  rm -f "$DMG"
  # A symlink to /Applications makes the "drag to install" gesture work.
  ln -s /Applications "$DIST/Applications"
  hdiutil create -volname "$APP_NAME $VERSION" -srcfolder "$DIST" -ov -format UDZO "$DMG"
  rm -f "$DIST/Applications"
  echo "dmg: $DMG"
fi

echo "done."
