#!/usr/bin/env bash
# One-time asset generation on macOS; ordinary packaging copies the committed ICNS.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
work=$(mktemp -d "${TMPDIR:-/tmp}/flicknote-icon.XXXXXX")
trap 'rm -rf -- "$work"' EXIT
mkdir "$work/FlickNote.iconset"
# 824px silhouette with transparent exterior; centered glyph at iOS's 1.1 scale.
magick -size 1024x1024 gradient:'#222222-#080808' \
  \( -size 1024x1024 xc:none -fill white \
     -draw 'roundrectangle 100,100 924,924 185,185' \) \
  -alpha off -compose CopyOpacity -composite \
  \( glyph.png -resize 906x906 \) -gravity center -compose Over -composite \
  -strip PNG32:"$work/master.png"
for size in 16 32 128 256 512; do
  magick "$work/master.png" -resize "${size}x${size}" -strip \
    PNG32:"$work/FlickNote.iconset/icon_${size}x${size}.png"
  retina=$((size * 2))
  magick "$work/master.png" -resize "${retina}x${retina}" -strip \
    PNG32:"$work/FlickNote.iconset/icon_${size}x${size}@2x.png"
done
iconutil --convert icns "$work/FlickNote.iconset" --output FlickNote.icns
