#!/usr/bin/env bash
# Installs deskstats for this user on Linux: ~/.local/bin/deskstats and a systemd user service
# that starts it at login.
#
#   ./install.sh
#   ./install.sh --uninstall
set -euo pipefail

service="deskstats.service"
unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
unit="$unit_dir/$service"
bin="$HOME/.local/bin/deskstats"
ui="$HOME/.local/bin/deskstats-ui"
apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
icon="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps/deskstats.png"
autostart="${XDG_CONFIG_HOME:-$HOME/.config}/autostart/deskstats-tray.desktop"
data="${XDG_CONFIG_HOME:-$HOME/.config}/deskstats"

fail() { echo "install.sh: $*" >&2; exit 1; }

cd "$(dirname "$0")"
command -v systemctl >/dev/null 2>&1 || fail "systemctl not found: this installer needs systemd"
systemctl --user show-environment >/dev/null 2>&1 || fail "the systemd user manager is not running (log in to a desktop session and try again)"

case "${1:-}" in
  "") ;;
  --uninstall)
    systemctl --user disable --now "$service" 2>/dev/null || true
    pkill -u "$(id -u)" -f "$ui" 2>/dev/null || true
    rm -f "$unit" "$bin" "$bin.new" "$ui" "$ui.new" "$apps/deskstats.desktop" "$icon" "$autostart"
    systemctl --user daemon-reload
    echo "deskstats removed."
    echo "Left in place: the pairing key in $data (delete the folder to forget the phone)."
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
  built_ui=""
  if [ -f ../ui/Cargo.toml ]; then
    (cd ../ui && cargo build --release)
    built_ui=../ui/target/release/deskstats-ui
  fi
else
  built=./deskstats
  built_ui=./deskstats-ui
fi
[ -x "$built" ] || fail "no program to install"

mkdir -p "$(dirname "$bin")" "$unit_dir"
# Written beside the old program and moved over it, so that a running server is never half replaced.
cp "$built" "$bin.new"
chmod 755 "$bin.new"
mv -f "$bin.new" "$bin"

# The service does not get the PATH of this shell, so the adb found here is handed to it by name.
adb_line=""
adb="$(command -v adb || true)"
case "$adb" in
  /*) adb_line="Environment=\"DESKSTATS_ADB=${adb//%/%%}\"" ;;
esac

cat > "$unit" <<UNIT
[Unit]
Description=deskstats: this computer's stats on an Android phone

[Service]
# %h is the home directory; the quotes keep a path with spaces in one piece.
ExecStart="%h/.local/bin/deskstats"
Restart=on-failure
RestartSec=3
# Stop only the server itself: the adb server it may have started is shared with other tools.
KillMode=process
$adb_line

[Install]
WantedBy=default.target
UNIT

systemctl --user daemon-reload
systemctl --user enable "$service" >/dev/null 2>&1 || fail "cannot enable $service"
systemctl --user restart "$service" || fail "the server did not start: journalctl --user -u $service"

running=0
for _ in $(seq 20); do
  if systemctl --user is-active --quiet "$service"; then
    running=1
    break
  fi
  sleep 0.25
done
[ "$running" = 1 ] || fail "the server did not start: journalctl --user -u $service"

# The icon by the clock and its panel: an entry in the menu of applications and one that starts at login.
panel=0
if [ -n "$built_ui" ] && [ -x "$built_ui" ]; then
  pkill -u "$(id -u)" -f "$ui" 2>/dev/null || true
  cp "$built_ui" "$ui.new"
  chmod 755 "$ui.new"
  mv -f "$ui.new" "$ui"
  mkdir -p "$apps" "$(dirname "$icon")" "$(dirname "$autostart")"
  "$ui" --icon "$icon" || true
  # Exec takes the path in quotes, with the characters the specification wants escaped there.
  exec_path="$(printf '%s' "$ui" | sed 's/[\\"`$]/\\&/g')"
  cat > "$apps/deskstats.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Deskstats
Comment=This computer's stats on an Android phone
Exec="$exec_path"
Icon=deskstats
Terminal=false
Categories=Utility;System;
StartupWMClass=deskstats
DESKTOP
  cat > "$autostart" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Deskstats (icon by the clock)
Exec="$exec_path" --tray
Icon=deskstats
Terminal=false
NoDisplay=true
X-GNOME-Autostart-enabled=true
DESKTOP
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$apps" >/dev/null 2>&1 || true
  # Started for this session too, if there is one to show it in.
  if [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then
    (setsid "$ui" --tray >/dev/null 2>&1 &) || true
  fi
  panel=1
fi

echo "deskstats is running and starts at login."
if [ "$panel" = 1 ]; then
  echo "Its icon is by the clock, and a click shows the panel; Deskstats in the menu of applications opens it too (on GNOME the icon needs"
  echo "the AppIndicator extension, which Ubuntu has)."
fi
echo
echo "For the phone to find this computer over Wi-Fi, a firewall must let in TCP 8787 and UDP 8788, e.g.:"
echo "  sudo ufw allow 8787/tcp && sudo ufw allow 8788/udp"
echo "To let a phone in over Wi-Fi, zenity or kdialog must be installed (it shows the question)."
echo "The USB cable is optional: for it, install adb (android-tools-adb or android-tools) and run this again."
