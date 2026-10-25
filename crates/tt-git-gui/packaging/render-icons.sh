#!/bin/sh
# Renders the app icon (../assets/icon.svg) into the PNG sizes the packages
# use: icons/<size>.png, for the Linux hicolor theme and the macOS .icns.
# The PNGs are committed, so the release jobs need no SVG renderer; run this
# again after changing icon.svg. Needs rsvg-convert (librsvg).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
command -v rsvg-convert >/dev/null 2>&1 || { echo "render-icons: rsvg-convert is required" >&2; exit 1; }
for size in 16 32 48 64 128 256 512 1024; do
    rsvg-convert -w "$size" -h "$size" "$here/../assets/icon.svg" -o "$here/icons/$size.png"
done
echo "Rendered $(ls "$here/icons" | wc -l) icons into $here/icons"
