# Skylis app icon

`skylis.svg` is the adopted J2 front-view design reconstructed by Astra 6 medium, selected by the project owner on 2026-11-17, and refined with C2 crystal-glass contrast on 2026-11-23. `skylis-small.svg` provides the small-size optical variant (`16-40px`) for Windows taskbar/titlebar and small macOS icon sizes. Both files are editable pure vectors with transparent backgrounds and contain no embedded raster or external assets. Keep the low-saturation sea-glass aqua body and abstract pearlescent fold intact when regenerating.

The C2 refinement preserves the clear icy sea-glass aesthetic while deepening the back tab, adding a subtle fold cast shadow, and introducing dual glass-cut rims so the silhouette remains legible on both light and dark backgrounds. At `<= 40px`, `skylis-small.svg` tightens canvas margins, raises body opacity, sharpens crease contrast, and thickens the dual rims.

PNG sizes, `skylis.ico` (Windows resource 1) and `skylis.icns` (macOS bundle) are generated offline and checked in. Normal application builds need neither Node nor an SVG renderer and perform no icon conversion at runtime.

To regenerate after an intentional SVG edit, use Node with Sharp available:

```sh
node scripts/generate-app-icons.cjs
# Or supply an existing Sharp package path:
node scripts/generate-app-icons.cjs /absolute/path/to/sharp
```

PNGs use straight transparency; test against light and dark backgrounds when changing artwork.
