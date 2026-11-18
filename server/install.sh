#!/usr/bin/env bash
# Builds deskstats, installs it as ~/Applications/Deskstats.app and starts it at login through
# launchd. It is an application bundle only to have a name and an icon in Activity Monitor and
# in the privacy settings; it has no window and no Dock icon.
#
#   ./install.sh [deskstats options]     e.g. ./install.sh --interval 1 --city Bishkek
#   ./install.sh --uninstall
#   ./install.sh --help
#
# Run it again with other options to change them. The log is ~/Library/Logs/deskstats.log.
#
# macOS ties the privacy permissions to the code signature. Without a certificate that is the
# exact binary, so every rebuild needs them granted again. With a code-signing certificate in
# the keychain they stay:  DESKSTATS_SIGN_IDENTITY="<name or hash>" ./install.sh …
# (see `security find-identity -v -p codesigning`).
set -euo pipefail

label="app.deskstats.server"
plist="$HOME/Library/LaunchAgents/$label.plist"
app="$HOME/Applications/Deskstats.app"
stage="$HOME/Applications/.Deskstats.new.app"
bin="$app/Contents/MacOS/deskstats"
# Where versions before the application bundle lived.
old_bin="$HOME/.local/bin/deskstats"
log="$HOME/Library/Logs/deskstats.log"
data="$HOME/Library/Application Support/deskstats"
domain="gui/$(id -u)"
identity="${DESKSTATS_SIGN_IDENTITY:--}"

# Stops the server and waits until launchd has let go of it: bootstrapping too early fails
# with error 5. A server sampling at a long interval may need a few seconds to notice.
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
  --help|-h)
    # After the cd above "$0" may no longer lead to this file; its name does.
    sed -n '2,15p' "${BASH_SOURCE[0]##*/}" | sed 's/^# \{0,1\}//'
    for program in target/release/deskstats ./deskstats; do
      [ -x "$program" ] || continue
      echo
      "$program" --help
      break
    done
    exit 0
    ;;
  --uninstall)
    unload || { echo "install.sh: the server does not stop; try again in a moment" >&2; exit 1; }
    rm -f "$plist" "$old_bin" "$log" "$HOME/Library/Logs/deskstats-ax.log" "$data/installed-build"
    rm -rf "$app" "$stage"
    echo "deskstats removed, with its logs."
    echo "Left in place: the token in $data (delete the folder to forget the phone),"
    echo "and the Deskstats entries under System Settings > Privacy & Security > Full Disk Access"
    echo "and Accessibility, which only you can remove there (select the entry, press the minus)."
    exit 0
    ;;
esac

for arg in "$@"; do
  if [[ "$arg" =~ [[:cntrl:]] ]]; then
    echo "install.sh: an option contains a control character" >&2
    exit 1
  fi
done

xml_escape() { printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'; }

# From the sources (Cargo.toml here) it is built; a release archive brings the program built.
if [ -f Cargo.toml ]; then
  cargo build --release
  built=target/release/deskstats
  version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
else
  built=./deskstats
  version="$(cat VERSION 2>/dev/null || true)"
fi
# Wrong options would make launchd restart the server in a loop; check them first. --check goes
# first so that an option missing its value cannot swallow it.
"$built" --check "$@" >/dev/null

if [ -z "$version" ]; then
  echo "install.sh: no version in Cargo.toml or VERSION" >&2
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
  <key>NSHumanReadableCopyright</key><string>Shows this Mac's load on a phone on the desk.</string>
</dict></plist>
INFO
)"

# Signing changes the installed file, so what was installed is remembered by a checksum of its
# sources, kept outside the bundle and written only after the installation went through.
build="$( { shasum -a 256 "$built" icon/AppIcon.icns; printf '%s\n%s\n' "$info" "$identity"; } | shasum -a 256 | cut -d' ' -f1)"
if [ -x "$bin" ] && [ "$(cat "$data/installed-build" 2>/dev/null)" = "$build" ] && codesign --verify "$app" 2>/dev/null; then
  same_bundle=1
else
  same_bundle=0
  # The new bundle is put together and signed aside; the running server is not touched until
  # it is complete.
  rm -rf "$stage"
  mkdir -p "$stage/Contents/MacOS" "$stage/Contents/Resources"
  cp "$built" "$stage/Contents/MacOS/deskstats"
  cp icon/AppIcon.icns "$stage/Contents/Resources/AppIcon.icns"
  printf '%s\n' "$info" > "$stage/Contents/Info.plist"
  plutil -lint "$stage/Contents/Info.plist" >/dev/null
  # The hardened runtime keeps other programs from loading their code into the server, which
  # holds the right to read the whole disk.
  if ! codesign --force --options runtime --sign "$identity" --identifier "$label" "$stage"; then
    if [ "$identity" = "-" ]; then
      echo "install.sh: cannot sign the application" >&2
      rm -rf "$stage"
      exit 1
    fi
    echo "install.sh: cannot sign as \"$identity\"; signing without a certificate instead" >&2
    identity="-"
    # Remember what was really installed, so the next run tries the certificate again.
    build="$( { shasum -a 256 "$built" icon/AppIcon.icns; printf '%s\n%s\n' "$info" "$identity"; } | shasum -a 256 | cut -d' ' -f1)"
    codesign --force --options runtime --sign - --identifier "$label" "$stage"
  fi
  codesign --verify "$stage"
fi

arguments="    <string>$(xml_escape "$bin")</string>"
for arg in "$@"; do
  arguments="$arguments
    <string>$(xml_escape "$arg")</string>"
done

mkdir -p "$(dirname "$plist")" "$(dirname "$log")" "$data"
chmod 700 "$data"
cat > "$plist.new" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>$label</string>
  <key>ProgramArguments</key><array>
$arguments
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ThrottleInterval</key><integer>30</integer>
  <key>ProcessType</key><string>Background</string>
  <key>StandardErrorPath</key><string>$(xml_escape "$log")</string>
</dict></plist>
PLIST
plutil -lint "$plist.new" >/dev/null

# From here on the server is down until the last step; say so if anything goes wrong.
trap 'echo "install.sh: failed after the server was stopped — it is NOT running. Fix the problem above and run install.sh again." >&2' ERR
if ! unload; then
  echo "install.sh: the running server does not stop" >&2
  false
fi
rm -f "$old_bin"
if [ "$same_bundle" = 0 ]; then
  rm -rf "$app"
  mv "$stage" "$app"
  # Finder and the privacy settings cache an application's icon by its path.
  touch "$app"
fi
mv "$plist.new" "$plist"
launchctl bootstrap "$domain" "$plist"
printf '%s' "$build" > "$data/installed-build"
trap - ERR

echo "deskstats is running with: $*"
echo "it starts at login; log: $log"
if [ "$same_bundle" = 0 ]; then
  echo
  if [ "$identity" = "-" ]; then
    echo "The application changed. macOS ties its permissions to the exact binary, so check in"
    echo "System Settings > Privacy & Security that Deskstats ($app) has"
  else
    echo "The application changed and is signed as \"$identity\": its permissions stay. If this is"
    echo "the first installation signed this way, grant them once more in"
    echo "System Settings > Privacy & Security, for Deskstats ($app):"
  fi
  echo "  - Full Disk Access (notifications of Mac apps),"
  echo "  - Accessibility (notifications forwarded from the iPhone)."
fi
