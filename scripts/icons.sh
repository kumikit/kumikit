#!/bin/sh
# Regenerates the raster icons from the brand SVGs in assets/brand/:
#   kumikit-icon.svg        full mark, used for 64 px and larger
#   kumikit-icon-small.svg  heavier lines and an open centre, for 48 px and smaller
# Writes crates/kumikit-shell/icons/icon.png (512 px) and icon.ico (16-256 px, PNG
# entries), and copies the small mark to web/kernel/public/favicon.svg.
# Needs rsvg-convert, ImageMagick (magick) and python3.
# Usage: sh scripts/icons.sh   (run from the repository root)
set -eu

brand=assets/brand
icons=crates/kumikit-shell/icons
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

png() { # size svg
  rsvg-convert -w "$1" -h "$1" "$2" -o "$tmp/raw$1.png"
  magick "$tmp/raw$1.png" -strip -define png:color-type=6 -define png:compression-level=9 "$tmp/$1.png"
}
for s in 16 24 32 48; do png "$s" "$brand/kumikit-icon-small.svg"; done
for s in 64 128 256 512; do png "$s" "$brand/kumikit-icon.svg"; done

cp "$tmp/512.png" "$icons/icon.png"
cp "$brand/kumikit-icon-small.svg" web/kernel/public/favicon.svg

# ImageMagick stores large ICO entries uncompressed; write PNG entries instead.
python3 -I - "$tmp" "$icons/icon.ico" <<'PY'
import struct, sys
src, out = sys.argv[1], sys.argv[2]
sizes = [16, 24, 32, 48, 64, 128, 256]
data = [open(f"{src}/{s}.png", "rb").read() for s in sizes]
head = struct.pack("<HHH", 0, 1, len(sizes))
offset = 6 + 16 * len(sizes)
entries = b""
for s, b in zip(sizes, data):
    entries += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(b), offset)
    offset += len(b)
with open(out, "wb") as f:
    f.write(head + entries + b"".join(data))
PY
echo "wrote $icons/icon.png, $icons/icon.ico, web/kernel/public/favicon.svg"
