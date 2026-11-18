#!/usr/bin/env bash
# Устанавливает deskstats из готовых сборок на GitHub: сервер на этот Mac и, если телефон
# подключён по USB, приложение на телефон. Rust и Android SDK не нужны.
#
#   curl -fsSL https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.sh | bash -s -- --city Bishkek --lan
#
# Параметры после «--» уходят серверу (см. «Параметры сервера» в README). Свои:
#   --no-phone    не ставить приложение на телефон
#   --uninstall   удалить сервер с этого Mac
set -euo pipefail

repo="danijar2000/deskstats"
base="https://github.com/$repo/releases/latest/download"
phone=1
args=()
for arg in "$@"; do
  case "$arg" in
    --no-phone) phone=0 ;;
    *) args+=("$arg") ;;
  esac
done

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
fail() { printf 'setup.sh: %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = Darwin ] || fail "сервер работает только на macOS"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

say "Скачиваю сервер"
curl -fsSL "$base/deskstats-server-macos.tar.gz" -o "$work/server.tar.gz" || fail "не удалось скачать сервер с $base"
tar -xzf "$work/server.tar.gz" -C "$work"
"$work/deskstats-server/install.sh" ${args[@]+"${args[@]}"}
for arg in ${args[@]+"${args[@]}"}; do
  [ "$arg" = --uninstall ] && exit 0
done

[ "$phone" = 1 ] || exit 0
adb="$(command -v adb || true)"
for candidate in "$HOME/Library/Android/sdk/platform-tools/adb" /opt/homebrew/bin/adb /usr/local/bin/adb; do
  [ -n "$adb" ] && break
  [ -x "$candidate" ] && adb="$candidate"
done
if [ -z "$adb" ]; then
  echo
  echo "adb не найден, приложение на телефон не ставлю. Поставьте его и запустите setup.sh снова:"
  echo "  brew install android-platform-tools"
  exit 0
fi
if ! "$adb" get-state >/dev/null 2>&1; then
  echo
  echo "Телефон по USB не виден (нужна «Отладка по USB» и один подключённый телефон)."
  echo "Подключите его и запустите setup.sh снова, или поставьте APK сами:"
  echo "  $base/deskstats.apk"
  exit 0
fi

say "Скачиваю приложение для телефона"
curl -fsSL "$base/deskstats.apk" -o "$work/deskstats.apk" || fail "не удалось скачать APK"
say "Ставлю приложение на телефон"
if ! out="$("$adb" install -r "$work/deskstats.apk" 2>&1)"; then
  echo "$out"
  if [[ "$out" == *INSTALL_FAILED_UPDATE_INCOMPATIBLE* ]]; then
    echo
    echo "На телефоне стоит сборка с другой подписью. Удалите её (настройки блоков и городов сбросятся)"
    echo "и запустите setup.sh снова:"
    echo "  $adb uninstall app.deskstats"
  fi
  exit 1
fi
echo
echo "Готово. Через несколько секунд сервер сам откроет панель на телефоне и передаст ей ключ."
