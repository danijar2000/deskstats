#!/usr/bin/env bash
# Renders icon.svg into AppIcon.icns (needs rsvg-convert: brew install librsvg). The result is
# kept in the repository, so installing the server does not need this.
set -euo pipefail
cd "$(dirname "$0")"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
set_dir="$tmp/AppIcon.iconset"
mkdir -p "$set_dir"
for size in 16 32 128 256 512; do
  rsvg-convert -w "$size" -h "$size" icon.svg -o "$set_dir/icon_${size}x${size}.png"
  rsvg-convert -w "$((size * 2))" -h "$((size * 2))" icon.svg -o "$set_dir/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$set_dir" -o AppIcon.icns
