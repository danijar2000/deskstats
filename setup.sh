#!/usr/bin/env bash
# Installs deskstats from the latest GitHub release: the server on this Mac and, when a phone
# is connected over USB, the app on the phone.
#
#   curl -fsSL https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.sh | bash -s -- --uninstall
#
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

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
fail() { printf 'setup.sh: %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = Darwin ] || fail "the server runs on macOS only"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

say "Downloading the server"
curl -fsSL "$base/deskstats-server-macos.tar.gz" -o "$work/server.tar.gz" || fail "cannot download $base/deskstats-server-macos.tar.gz"
tar -xzf "$work/server.tar.gz" -C "$work"
if [ "$uninstall" = 1 ]; then
  "$work/deskstats-server/install.sh" --uninstall
  exit 0
fi
"$work/deskstats-server/install.sh"

[ "$phone" = 1 ] || exit 0
adb="$(command -v adb || true)"
for candidate in "$HOME/Library/Android/sdk/platform-tools/adb" /opt/homebrew/bin/adb /usr/local/bin/adb; do
  [ -n "$adb" ] && break
  [ -x "$candidate" ] && adb="$candidate"
done
if [ -z "$adb" ]; then
  echo
  echo "adb not found, so the phone app is not installed. Install adb and run setup.sh again:"
  echo "  brew install android-platform-tools"
  exit 0
fi
if ! "$adb" get-state >/dev/null 2>&1; then
  echo
  echo "No phone over USB (USB debugging must be on, with one phone connected)."
  echo "Connect it and run setup.sh again, or install the app yourself:"
  echo "  $base/deskstats.apk"
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
echo "Done. In a few seconds the Mac opens the panel on the phone and pairs it."
