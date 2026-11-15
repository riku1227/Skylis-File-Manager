# Drag range selection

Follow-up to the file drag/drop feature. UX reviewed with Claude Opus 5.5.

## Interaction

- Start with the left button on blank content: list gutters, trailing space, date headers, gallery gaps or empty cells. Rows and tiles retain their existing file drag/drop behavior.
- A 4 px movement threshold separates a click from a rectangle. Plain blank clicks clear selection; modified blank clicks retain it.
- Plain drag replaces selection, Shift adds, and Ctrl/Command toggles against the selection captured at mouse-down. Shrinking a rectangle restores items according to that original baseline.
- Escape cancels the gesture. Selection keyboard commands are suspended during the gesture, so they cannot overwrite its baseline.
- The start stays in content coordinates during scrolling. A single 16 ms task scrolls in the top/bottom 24 px zones, capped at 18 px per tick, including while the pointer is stationary. Releasing or cancelling stops the task.
- The closest hit to the start becomes the Shift-selection anchor. The closest hit to the end becomes the focused item if still selected; otherwise focus falls back to a remaining selected item. Horizontal gallery movement uses both coordinates.
- Navigation, refreshed listing batches, filtering, sorting, grouping, density/view changes, viewport changes and focus loss cancel the in-progress gesture. Existing loading/selection reconciliation then applies normally.

## Lightweight implementation

`folder/selection_geometry.rs` contains pure content-space rectangle/virtual-row geometry. `folder/marquee.rs` owns the short-lived gesture, overlay and edge-scroll task. The list has 12 px side gutters and two empty trailing virtual rows. Gallery hit tests use actual tile boxes and exclude gaps and headers.

Mouse movement updates coordinates only. Rendered rows derive their highlight from the rectangle and baseline; offscreen files are not traversed on every move. Paths are materialized once at release, using only candidate virtual rows. Row/tile identities and image handles stay stable, and the inspector updates after the selection commits.

The selection rectangle is a painted overlay rather than a GPUI drag payload. It never promotes a native file drag or activates folder drop targets. Platform modifier flags support Windows Ctrl and macOS Command through the same code.

## Verification

Pure geometry tests cover all directions, precise row/tile boundaries, compact layout, empty cells, bounded candidate ranges for million-row lists, and edge-scroll speed. GPUI input tests exercise modifiers and shrink, lazy path commit, blank clicks, Escape, sorting, date headers, gallery columns, stationary edge scrolling and file drag/drop coexistence. Additional regression checks cover keyboard input and viewport resizing during a gesture.

Windows native checks use generated files in `target/ui-fixtures`: reverse selection from lower blank space, selection through the left gutter of a 240-file list, and a rectangle selecting four images across two gallery rows. macOS native pointer behavior still requires a macOS host; Command modifier behavior is covered by the shared GPUI tests.

Temporary interactive explanation: `target/feature-guide/range-selection.html`, linked from the main feature guide. It is not committed.
