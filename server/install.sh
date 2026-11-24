#!/usr/bin/env bash
# Installs deskstats as ~/Applications/Deskstats.app and starts it at login.
#
#   ./install.sh
#   ./install.sh --uninstall
set -euo pipefail

label="app.deskstats.server"
plist="$HOME/Library/LaunchAgents/$label.plist"
app="$HOME/Applications/Deskstats.app"
stage="$HOME/Applications/.Deskstats.new.app"
bin="$app/Contents/MacOS/deskstats"
data="$HOME/Library/Application Support/deskstats"
old_logs=("$HOME/Library/Logs/deskstats.log" "$HOME/Library/Logs/deskstats-ax.log")
domain="gui/$(id -u)"
identity="${DESKSTATS_SIGN_IDENTITY:--}"

unload() {
  launchctl bootout "$domain/$label" 2>/dev/null || true
  for _ in $(seq 200); do
    launchctl print "$domain/$label" >/dev/null 2>&1 || return 0
    sleep 0.1
  done
  return 1
}

cd "$(dirname "$0")"

case "${1:-}" in
  "") ;;
  --uninstall)
    unload || { echo "install.sh: the server does not stop; try again in a moment" >&2; exit 1; }
    rm -f "$plist" "$plist.new" "${old_logs[@]}" "$data/installed-build"
    rm -rf "$app" "$stage"
    echo "deskstats removed."
    echo "Left in place: the pairing key in $data (delete the folder to forget the phone)"
    echo "and the Deskstats entries in System Settings > Privacy & Security."
    exit 0
    ;;
  *)
    echo "usage: ./install.sh [--uninstall]" >&2
    exit 2
    ;;
esac

if [ -f Cargo.toml ]; then
  cargo build --release
  built=target/release/deskstats
  version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
else
  built=./deskstats
  version="$(cat VERSION 2>/dev/null || true)"
fi
if [ -z "$version" ] || [ ! -x "$built" ]; then
  echo "install.sh: no program or version to install" >&2
  exit 1
fi

info="$(cat <<INFO
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>$label</string>
  <key>CFBundleName</key><string>Deskstats</string>
  <key>CFBundleDisplayName</key><string>Deskstats</string>
  <key>CFBundleExecutable</key><string>deskstats</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSUIElement</key><true/>
</dict></plist>
INFO
)"

fingerprint() { { shasum -a 256 "$built" icon/AppIcon.icns; printf '%s\n%s\n' "$info" "$identity"; } | shasum -a 256 | cut -d' ' -f1; }
build="$(fingerprint)"
if [ -x "$bin" ] && [ "$(cat "$data/installed-build" 2>/dev/null)" = "$build" ] && codesign --verify "$app" 2>/dev/null; then
  same_bundle=1
else
  same_bundle=0
  rm -rf "$stage"
  mkdir -p "$stage/Contents/MacOS" "$stage/Contents/Resources"
  cp "$built" "$stage/Contents/MacOS/deskstats"
  cp icon/AppIcon.icns "$stage/Contents/Resources/AppIcon.icns"
  printf '%s\n' "$info" > "$stage/Contents/Info.plist"
  plutil -lint "$stage/Contents/Info.plist" >/dev/null
  if ! codesign --force --options runtime --sign "$identity" --identifier "$label" "$stage"; then
    if [ "$identity" = "-" ]; then
      echo "install.sh: cannot sign the application" >&2
      rm -rf "$stage"
      exit 1
    fi
    echo "install.sh: cannot sign as \"$identity\"; signing without a certificate instead" >&2
    identity="-"
    build="$(fingerprint)"
    codesign --force --options runtime --sign - --identifier "$label" "$stage"
  fi
  codesign --verify "$stage"
fi

mkdir -p "$(dirname "$plist")" "$data"
chmod 700 "$data"
cat > "$plist.new" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>$label</string>
  <key>ProgramArguments</key><array>
    <string>$bin</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ThrottleInterval</key><integer>30</integer>
  <key>ProcessType</key><string>Background</string>
</dict></plist>
PLIST
plutil -lint "$plist.new" >/dev/null

if ! unload; then
  rm -rf "$plist.new" "$stage"
  echo "install.sh: the running server does not stop; it keeps running as it was" >&2
  exit 1
fi
trap 'echo "install.sh: failed after the server was stopped; it is not running. Run install.sh again." >&2' ERR
rm -f "${old_logs[@]}"
if [ "$same_bundle" = 0 ]; then
  rm -rf "$app"
  mv "$stage" "$app"
  touch "$app"
fi
mv "$plist.new" "$plist"
launchctl bootstrap "$domain" "$plist"
printf '%s' "$build" > "$data/installed-build"
trap - ERR

running=0
for _ in $(seq 20); do
  if launchctl print "$domain/$label" 2>/dev/null | grep -q 'state = running'; then
    running=1
    break
  fi
  sleep 0.25
done
if [ "$running" = 0 ]; then
  echo "install.sh: the server did not start" >&2
  exit 1
fi

echo "deskstats is running and starts at login."
if [ "$same_bundle" = 0 ]; then
  echo
  echo "In System Settings > Privacy & Security allow Deskstats ($app):"
  echo "  - Full Disk Access (notifications of Mac apps),"
  echo "  - Accessibility (notifications forwarded from the iPhone)."
  echo "After an update, allow them again: macOS ties them to the exact program."
fi
