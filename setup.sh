#!/usr/bin/env bash
# Installs deskstats from the latest GitHub release: the server on this Mac or Linux computer and,
# when a phone is connected over USB, the app on the phone. On Windows, use setup.ps1.
#
#   curl -fsSL https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.sh | bash -s -- --uninstall
#
# Without adb or without a phone on the cable it shows a QR code with the link to the app instead.
# --no-phone skips the phone.
set -euo pipefail

base="https://github.com/danijar2000/deskstats/releases/latest/download"
phone=1
uninstall=0
for arg in "$@"; do
  case "$arg" in
    --no-phone) phone=0 ;;
    --uninstall) uninstall=1 ;;
    *) echo "usage: setup.sh [--no-phone] [--uninstall]" >&2; exit 2 ;;
  esac
done

# Shows the link to the phone app as a QR code, drawn by the server that was just installed.
show_qr() {
  local server
  for server in "$HOME/Applications/Deskstats.app/Contents/MacOS/deskstats" "$HOME/.local/bin/deskstats"; do
    if [ -x "$server" ] && "$server" --qr "$base/deskstats.apk" 2>/dev/null; then
      break
    fi
  done
  echo "  $base/deskstats.apk"
  echo
  echo "Scan the code with the phone's camera (or open the link on the phone), install the app and open it."
  echo "With the phone on the same Wi-Fi as this computer, the app finds it: tap it there and allow it here."
}

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
fail() { printf 'setup.sh: %s\n' "$*" >&2; exit 1; }

case "$(uname -s)" in
  Darwin)
    archive="deskstats-server-macos.tar.gz"
    adb_hint="brew install android-platform-tools"
    ;;
  Linux)
    case "$(uname -m)" in
      x86_64 | amd64) arch=x86_64 ;;
      aarch64 | arm64) arch=aarch64 ;;
      *) fail "no build for $(uname -m); build from source: https://github.com/danijar2000/deskstats#build-from-source" ;;
    esac
    archive="deskstats-server-linux-$arch.tar.gz"
    adb_hint="sudo apt install android-tools-adb   (or your distribution's android-tools package)"
    ;;
  *) fail "this script is for macOS and Linux; on Windows use setup.ps1" ;;
esac

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

say "Downloading the server"
curl -fsSL "$base/$archive" -o "$work/server.tar.gz" || fail "cannot download $base/$archive (if the latest release has no build for this system yet, build from source: https://github.com/danijar2000/deskstats#build-from-source)"
tar -xzf "$work/server.tar.gz" -C "$work"
if [ "$uninstall" = 1 ]; then
  "$work/deskstats-server/install.sh" --uninstall
  exit 0
fi
"$work/deskstats-server/install.sh"

[ "$phone" = 1 ] || exit 0
adb="$(command -v adb || true)"
for candidate in "$HOME/Library/Android/sdk/platform-tools/adb" "$HOME/Android/Sdk/platform-tools/adb" /opt/homebrew/bin/adb /usr/local/bin/adb /usr/bin/adb; do
  [ -n "$adb" ] && break
  [ -x "$candidate" ] && adb="$candidate"
done
if [ -z "$adb" ]; then
  echo
  echo "To put the app on the phone without a cable:"
  echo
  show_qr
  echo
  echo "For the USB cable instead, install adb and run setup.sh again:"
  echo "  $adb_hint"
  exit 0
fi
if ! "$adb" get-state >/dev/null 2>&1; then
  echo
  echo "No phone over USB (USB debugging must be on, with one phone connected)."
  echo "Connect it and run setup.sh again, or put the app on the phone without a cable:"
  echo
  show_qr
  exit 0
fi

say "Downloading the phone app"
curl -fsSL "$base/deskstats.apk" -o "$work/deskstats.apk" || fail "cannot download the app"
say "Installing the app on the phone"
if ! out="$("$adb" install -r "$work/deskstats.apk" 2>&1)"; then
  echo "$out"
  if [[ "$out" == *INSTALL_FAILED_UPDATE_INCOMPATIBLE* || "$out" == *INSTALL_FAILED_VERSION_DOWNGRADE* ]]; then
    echo
    echo "The phone has a build with another signature or a newer version. Remove it"
    echo "(its settings are lost) and run setup.sh again:"
    echo "  $adb uninstall app.deskstats"
  fi
  exit 1
fi
echo
echo "Done. In a few seconds the computer opens the panel on the phone and pairs it."
