#!/usr/bin/env bash
# Installs deskstats from the latest GitHub release: the server on this Mac or Linux computer and,
# when a phone is connected over USB, the app on the phone. On Windows, use setup.ps1.
#
#   curl -fsSL https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.sh | bash -s -- --uninstall
#
# Without adb or without a phone on the cable it shows the link to the app instead.
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

# Shows the link to the phone app, to open on the phone.
show_link() {
  echo "Open this link on the phone, install the app and open it:"
  echo
  echo "  $base/deskstats.apk"
  echo
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

# Checks a download against the SHA256SUMS of the release. Releases before it was published have none: then the
# download is used as it is, with a warning.
sums=""
check() {
  local file="$1" name="$2" want got
  if [ -z "$sums" ]; then
    # Only a release without the file (404) is let through unchecked; any other failure stops here.
    local code
    code="$(curl -sSL -o "$work/SHA256SUMS" -w '%{http_code}' "$base/SHA256SUMS" 2>/dev/null || true)"
    case "$code" in
      200) sums="$work/SHA256SUMS" ;;
      404) sums="none" ;;
      *) fail "cannot download the checksums of the release (HTTP ${code:-no answer}); try again" ;;
    esac
  fi
  if [ "$sums" = none ]; then
    echo "setup.sh: this release has no checksums; $name is not checked" >&2
    return 0
  fi
  want="$(awk -v n="$name" '$2 == n || $2 == "*" n { print $1 }' "$sums")"
  [ -n "$want" ] || fail "$name is not in the checksums of the release"
  if command -v sha256sum >/dev/null 2>&1; then got="$(sha256sum "$file" | cut -d' ' -f1)"; else got="$(shasum -a 256 "$file" | cut -d' ' -f1)"; fi
  [ "$got" = "$want" ] || fail "$name does not match its checksum (got $got, want $want); not installing it"
}

say "Downloading the server"
curl -fsSL "$base/$archive" -o "$work/server.tar.gz" || fail "cannot download $base/$archive (if the latest release has no build for this system yet, build from source: https://github.com/danijar2000/deskstats#build-from-source)"
check "$work/server.tar.gz" "$archive"
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
  echo "For the USB cable, install adb and run setup.sh again:"
  echo "  $adb_hint"
  echo
  echo "To put the app on the phone without a cable:"
  show_link
  exit 0
fi
if ! "$adb" get-state >/dev/null 2>&1; then
  echo
  echo "No phone over USB (USB debugging must be on, with one phone connected)."
  echo "Connect it and run setup.sh again, or put the app on the phone without a cable:"
  show_link
  exit 0
fi

say "Downloading the phone app"
curl -fsSL "$base/deskstats.apk" -o "$work/deskstats.apk" || fail "cannot download the app"
check "$work/deskstats.apk" deskstats.apk
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
