# Explorer workflow expansion

User delegates ordinary specification decisions and asks for execution without approval gates (2026-11-15). UX consultation: Claude Opus 5.5. Unavoidable product decisions will be collected at the end, not block unrelated work.

## Architecture and priorities

Retain Rust + GPUI Fast, virtualized lists and semantic theme tokens. Extend focused modules rather than adding a general framework. Background work has bounded queues/cache/concurrency and cancellation. Window/tab ownership must remain explicit. Never follow directory links during recursive operations by default.

1. Stabilize icon demand scheduling and LRU retention before adding thumbnails.
2. Persist versioned settings (system theme, grouped Downloads enabled, auto gallery enabled, manual folder sizes by default). Share live settings between windows.
3. Downloads use known-folder identity, modification date descending, local-calendar nonoverlapping date sections. Group rows are not selectable. User sorting exits automatic grouped mode for that view.
4. Context menu provides common file operations and Windows classic Shell menu. macOS uses supported integrations, never promises a complete Finder extension host.
5. File D&D uses explicit copy/move semantics, rejects self/descendant destinations and never silently overwrites. Tab D&D preserves the same folder entity and history.
6. Scrollbars provide visible position, track clicks and thumb dragging with bounded virtualized rendering.
7. Space preview loads only selected content; close/cancel on Escape/navigation. Bounded built-in image/text previews, native macOS Quick Look as appropriate. No assumed undocumented PowerToys invocation.
8. WSL locations discovered asynchronously without starting arbitrary distributions during paint.
9. Auto gallery after enumeration when image files / non-directory files >= 80%, excluding hidden filtered files; empty folders stay lists. Manual view override. Request visible thumbnails only, keep stable fallback until high quality arrives.
10. Folder size defaults to explicit request. Automatic mode is opt-in, bounded, cancellable on navigation/drop, reports partial/error counts instead of falsely exact totals.

## Alternatives

Custom menus with native OS entry points were selected over re-rendering every third-party extension (compatibility) or exclusively native menus (Skylis-specific UX). On-demand metadata and previews are preferred over eager tree scans (latency/memory). In-process bounded workers initially; native calls already in flight cannot always be interrupted, so cancel queued work and discard stale results.

## Verification

Pure tests cover date boundaries, selection mapping, settings corruption/round trip, cache eviction/scheduler, recursive scan cancellation/link safety and file-operation collision safety. GPUI integration tests and manual native-app checks cover interactions. Build/test the Windows target here; clearly identify macOS runtime work that cannot be validated on this host.

## Workspace

Work on feature branch in the existing checkout to preserve already available builds and shared-agent file ownership. Pre-existing README.md and docs/implementation/windows-build.md changes are not ours and will not be committed.

## Concrete choices after implementation

Preferences use a small versioned `settings.conf` format with atomic replacement and 300ms debounce. An invalid existing file is preserved and writes are disabled with a visible error instead of overwriting it. Defaults remain system theme, normal density, grouped Downloads, automatic gallery and manual folder sizes.

Date sections are Today, Yesterday, This week, Last week, This month, Last month, Earlier and Unknown; calendar boundaries are local and nonoverlapping. Gallery decisions run after enumeration; manual view selection survives refresh within the visit. The 80% rule has no extra minimum file count.

Icons use a 16 MiB/2,048-entry cache and a bounded 128-request demand queue. Media uses a 64 MiB/384-entry cache and 96 pending requests, with one internal decoder and one Windows Shell worker. Recursive size scans use two shared workers, 128 waiting requests and a 128-level traversal cap; logical sizes and incomplete lower bounds are explicit.

Native Shell, transfer, WSL, media and size services are focused modules. Windows Shell calls normalize slash-separated absolute paths without an added filesystem probe. macOS integrations are compile-gated, with runtime verification outstanding on a Mac. See the progress document for exact validation and platform boundaries.
