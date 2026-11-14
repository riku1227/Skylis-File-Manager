# Home / address entry / hover verification — 2026-11-15

## Changes
- Default startup and new tabs show virtual Home with drive capacity cards and known-folder shortcuts. `--path` keeps its existing meaning.
- Home is a distinct `Location`, carried by each tab's generic history; the real user-profile directory has its own shortcut.
- Current breadcrumb / address whitespace click and Ctrl+L / Alt+D / Cmd+L enter path editing. Enter validates directory metadata on the background executor; Escape, outside clicks, blur or tab switching cancel. Generation checks and task cancellation discard outdated validation.
- Invalid addresses keep the draft and current location. Supports quoted/Unicode, absolute/relative, UNC, bare Windows drive letters and `~`. URL input is rejected; this is not a command launcher.
- Navigation clears the current location's search; refresh preserves it.
- The list's hover surface now has a stable ID beneath each virtual row. No timer, permanent redraw loop, Fast-specific API or dependency-edition change was added.

## Automated checks
- `cargo test --workspace --locked`: 20 passed (11 app, 9 core).
- `cargo check --workspace --locked`, `cargo build --release -p explorer --locked`, `cargo fmt --all -- --check`, both token generators' `--check`, and `git diff --check`: passed.
- The hover regression test failed before the ID fix with `enter must repaint without scrolling`. It now verifies repaint on enter/exit and no extra rendering when the pointer moves within the same row.
- Address integration checks use real filesystem metadata: file rejection preserves Home and the draft; a directory navigates; cancelled validation cannot navigate later.
- Core tests cover mixed Home/folder history, selection restoration, forward-history invalidation, Unicode/quoted paths, relative paths, `~`, UNC, bare drive roots, NUL/empty/URL rejection and ambiguous drive-relative input rejection.
- Tests use GPUI's test-support feature only as a dev dependency. Real listing/drive workers are permitted in integration tests; hover rendering remains deterministic.

## Windows UI checks
- 1280×800 light/dark: drive capacities, folder cards, sidebar, compact tab/toolbar and status bar render.
- Click address whitespace to edit; type the fixture directory and Enter to navigate.
- Click the current breadcrumb to select/edit the complete path; a nonexistent relative path leaves the directory intact and shows an error; Escape cancels.
- Ctrl+T opens a separate Home tab while retaining the earlier folder tab.
- Double-click a drive card to open its root; Alt+Left returns to Home.
- Resize to 1100×700 and 900×600: cards wrap to two columns; at minimum height, Home and sidebar scroll independently and lower cards remain reachable. Dark theme verified at minimum size.
- Ctrl+L accepts a long fixture path at minimum width. Typing an unsubmitted draft then clicking outside cancels it; Down then selects the first file row, confirming focus returned to the list.

The latest release executable is left open on Home at the initial 1280×800 size. The user's already-running previous build was preserved separately.

## Limits
- macOS build/rendering and actual Japanese IME candidate interactions are not verified on hardware.
- Network-share metadata validation is off the UI thread, but the OS call itself cannot be forcibly interrupted. Cancelling discards its result.
- Drive discovery refreshes on startup/new tab and F5; hotplug notifications are not implemented.
- Recent files, favorites editing, path completion and persistent startup settings are not part of this change.
