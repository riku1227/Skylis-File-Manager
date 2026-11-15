# Feature expansion progress

Plan/spec: feature-expansion-design.md. User delegated specification and execution; no intermediate approval gate. UX consultation completed with Claude Opus 5.5.

- [x] Icons: bounded latest-demand scheduler, shared type icons and stable LRU image identity.
- [x] Settings: versioned atomic preferences, shared live settings screen, corruption/round-trip tests.
- [x] Downloads: newest-first local-calendar groups, nonselectable virtual headers and date-boundary tests.
- [x] Scrollbars: reserved gutter, draggable thumb, page and Shift-jump interactions.
- [x] Context menu: custom selection-aware popup, Windows classic Shell host, macOS Finder/Share/Services adapters.
- [x] File drag/drop: frozen multi-selection, native clipboard and transfer workers, actual folder drop targets, cross-window refresh.
- [x] Tab drag/drop: reorder, detach, merge, retained entity/history, overlapping-window regression.
- [x] Preview: Space/Escape, bounded image/text loading, native macOS Quick Look ownership.
- [x] WSL: registry discovery without VM startup, validated UNC sidebar locations.
- [x] Gallery: 80% image classification, virtualized tile rows, progressive Shell thumbnails and stable cache.
- [x] Folder sizes: manual/automatic modes, two bounded workers, partial totals and navigation cancellation.
- [x] Integration review and regression tests; temporary self-contained graphical HTML guide.

## Verification on Windows

`cargo test --workspace --locked`: 103 passed, 0 failed, 2 ignored (environment-dependent native Shell and WSL smoke tests). `cargo fmt --all --check` and `git diff --check` pass. `cargo build -p explorer --locked` builds the debug application.

Native GUI checks used generated files under `target/ui-fixtures`, leaving ordinary user files unchanged:

- Windows Shell menu displayed installed PowerToys, 7-Zip and editor commands. The forward-slash CLI path failure was reproduced and fixed at the Shell API boundary.
- Automatic gallery thumbnails and the Space image preview displayed correctly; the preview remained inside its frame.
- Manual folder size returned 16 KiB for the text fixtures.
- Dragging Documents into Destination copied all 240 files; source/destination byte totals both equalled 16,210.
- Scrollbar dragging moved from the beginning to entries around 200; rows and icons rendered at the new position.
- Tab reordering, shortcut detachment into two native windows and menu-based merge succeeded. State-preserving transfers and overlapping-window destination selection also have GPUI tests.
- Settings displayed all five preferences. The live GUI check did not alter saved preferences. A discovered Alt+F4 interception was fixed, regression tested and verified to close the rebuilt native window.

## Boundaries and follow-up verification

macOS adapters are implemented but Objective-C compilation and native behavior require a macOS host. Finder Sync extension menus cannot be fully hosted by a third-party file manager; the supported Reveal/Share/Services entry points are explicit. Windows uses the classic Shell menu, not Explorer's private Windows 11 modern menu.

The automation driver cannot drag outside the selected window's bounds. Native outbound file drag and physical tab tear-off/merge across window boundaries therefore have implementation/unit coverage, but still need a hands-on platform pass. Windows internal file D&D, native Shell menu, native window detach/merge and shared completion updates were checked as described above.

Cancellation stops queued work and discards late results; an OS call already running may finish before its worker can stop. Requested copy/move operations deliberately outlive navigation and are cancelled in the OS progress dialog. External Windows drops conservatively copy; internal Shift-drop moves. Unsupported previews fall back to metadata/default app; no undocumented PowerToys integration is invoked.

Temporary guide: `target/feature-guide/index.html` (not committed). Previewed at 728px and 390px with no console errors. Contains all 11 feature explanations and interactive diagrams. Native menu evidence: `target/feature-guide/native-menu.jpg`.

## Workspace

Feature branch: `feat/explorer-workflows`. Each feature has a Conventional Commit with a detailed body; shared Shell foundation and integration fixes are separate commits. Existing user changes to README.md and docs/implementation/windows-build.md remain uncommitted and untouched. No product specification decision requires user approval at this point.
