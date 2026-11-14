# Home, address entry and hover updates

## Scope
- Keep the approved compact, opaque light/dark design and 38px integrated tab strip.
- Default startup and new tabs open a virtual Home page: drive cards with real capacity and shortcuts to known folders. Explicit `--path` still opens that folder.
- Home participates in each tab's back/forward history; it is not the user's profile directory. Keep that directory as a separate shortcut.
- Click the current breadcrumb or address whitespace, or press Ctrl+L / Alt+D (Cmd+L on macOS), to edit a selected full path. Enter validates a directory off the UI thread and navigates; Esc or focus loss cancels. Invalid paths keep the draft and show an error. No commands are executed.
- Give the file-row hover surface a stable element ID. GPUI Fast 0.1.15 only records hover state and requests repaint for stateful elements; an anonymous inner row only changes during another repaint.

## Implementation
1. Extend UI-independent history to carry a typed Home/Folder location; test navigation and address parsing.
2. Reuse the existing IME-capable text input for address entry. Cancel stale validation after editing, navigating or switching tabs.
3. Add Home rendering under `folder/home.rs`, using existing platform places, icon cache and asynchronous drive capacity queries. Support selection, Enter/double-click, filtering and refresh.
4. Reproduce the hover regression using the GPUI test harness, then fix the row surface without continuous redraw or edition-specific APIs.
5. Run workspace tests, formatting/token checks, release build and Windows UI checks; commit and push to the existing repository.

No dependency edition changes, WebView, filesystem mutations, recent-file tracking or settings persistence are included.
