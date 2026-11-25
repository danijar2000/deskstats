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
data="${XDG_CONFIG_HOME:-$HOME/.config}/deskstats"

fail() { echo "install.sh: $*" >&2; exit 1; }

cd "$(dirname "$0")"
command -v systemctl >/dev/null 2>&1 || fail "systemctl not found: this installer needs systemd"
systemctl --user show-environment >/dev/null 2>&1 || fail "the systemd user manager is not running (log in to a desktop session and try again)"

case "${1:-}" in
  "") ;;
  --uninstall)
    systemctl --user disable --now "$service" 2>/dev/null || true
    rm -f "$unit" "$bin" "$bin.new"
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
else
  built=./deskstats
fi
[ -x "$built" ] || fail "no program to install"

mkdir -p "$(dirname "$bin")" "$unit_dir"
# Written beside the old program and moved over it, so that a running server is never half replaced.
cp "$built" "$bin.new"
chmod 755 "$bin.new"
mv -f "$bin.new" "$bin"

cat > "$unit" <<UNIT
[Unit]
Description=deskstats: this computer's stats on an Android phone

[Service]
# %h is the home directory; the quotes keep a path with spaces in one piece.
ExecStart="%h/.local/bin/deskstats"
Restart=on-failure
RestartSec=30
# Stop only the server itself: the adb server it may have started is shared with other tools.
KillMode=process

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

echo "deskstats is running and starts at login."
echo
echo "For the phone to find this computer over Wi-Fi, a firewall must let in TCP 8787 and UDP 8788, e.g.:"
echo "  sudo ufw allow 8787/tcp && sudo ufw allow 8788/udp"
echo "To let a phone in over Wi-Fi, zenity or kdialog must be installed (it shows the question)."
echo "The USB cable is optional: for it, install adb (android-tools-adb or android-tools) and run this again."
