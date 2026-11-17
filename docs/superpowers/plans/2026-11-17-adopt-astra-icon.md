# Adopt Astra icon implementation plan

**Goal:** Adopt the user-selected Astra SVG without changing its design, and use derived icons for Windows executable/window identity and macOS bundles.

**Architecture:** Keep one editable SVG in `apps/explorer/assets/branding`. Generate PNG/ICO/ICNS offline. Windows resource ID 1 is loaded by GPUI Fast's existing platform adapter; macOS bundles copy ICNS and declare it in Info.plist. No runtime image work or new runtime dependencies.

**Spec:** User selected the Astra J2 front SVG on 2026-11-17. Preserve the exact artwork and transparency. Do not commit exploration sheets or comparison artifacts.

**Tech stack:** SVG, optional Node/Sharp developer regeneration, embed-resource (already locked transitively), existing macOS shell packaging.

- [x] Copy the selected SVG and generate PNG sizes, multi-image ICO and ICNS; decode the generated containers to verify sizes/alpha.
- [x] Embed Windows icon resource 1; declare/copy the macOS bundle icon. Keep application logic unchanged.
- [x] Build the Windows executable and inspect its actual icon resource, run formatting/diff checks and validate macOS plist/package wiring. macOS execution cannot be tested here.
- [x] Document provenance/regeneration; commit scope excludes exploration sheets and comparison artifacts.

Review focus: no raster embedded in source SVG; transparent edges; resource ID matches GPUI; rebuild triggers when ICO changes; ICNS copied before signing. Small icon detail is derived directly from the approved artwork; no unapproved redesign.

## Verification

- Adopted SVG SHA-256 equals the selected Astra source: `346c1dce9f233785ba91ff771afbcb4994363fda3de93fba0bc967e666f0550d`.
- Pillow decoded all 12 PNG dimensions with alpha, all 10 ICO sizes and the 1024px ICNS image.
- Windows release build passed. The resulting EXE's group-icon resource 1 contains all 10 sizes, each byte-identical to its generated PNG. Win32 LoadImage succeeded at 16/32/48/256px.
- Formatting, diff whitespace, macOS shell syntax, plist parse and existing signing-selection fixtures passed.
- Focused independent review found no important issues. macOS app build/signing/display and live taskbar appearance remain untested here. Existing unrelated Rust warnings were left unchanged.
