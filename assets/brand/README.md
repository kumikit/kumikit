# Brand assets

The Kumikit icon is *asanoha* (hemp leaf), the classic kumiko pattern: a hexagon split
into six triangles, each joined to its centre. Kumiko is Japanese woodwork in which a
lattice is assembled from many small pieces without nails.

- `kumikit-icon.svg`: the full icon, for 64 px and larger (README, app icon).
- `kumikit-icon-small.svg`: heavier lines and an open centre so the pattern stays
  readable at 48 px and smaller (favicon, small ICO entries).

The raster app icons and the favicon are generated from these files. After changing
either SVG, run `sh scripts/icons.sh` from the repository root (needs `rsvg-convert`,
ImageMagick and `python3`) and commit its output.
