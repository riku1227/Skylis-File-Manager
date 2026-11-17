# Skylis app icon

`skylis.svg` is the adopted J2 front-view design reconstructed by Astra 6 medium and selected by the project owner on 2026-11-17. It is an editable vector with transparent background; it contains no embedded raster or external assets. Keep its aqua glass body and abstract pearlescent fold intact when regenerating.

PNG sizes, `skylis.ico` (Windows resource 1) and `skylis.icns` (macOS bundle) are generated offline and checked in. Normal application builds need neither Node nor an SVG renderer and perform no icon conversion at runtime.

To regenerate after an intentional SVG edit, use Node with Sharp available:

```sh
node scripts/generate-app-icons.cjs
# Or supply an existing Sharp package path:
node scripts/generate-app-icons.cjs /absolute/path/to/sharp
```

The source canvas and margins are preserved at all sizes. Fine glass detail simplifies on small taskbar icons; no separate small-size redesign has been introduced. PNGs use straight transparency; test against light and dark backgrounds when changing artwork.
