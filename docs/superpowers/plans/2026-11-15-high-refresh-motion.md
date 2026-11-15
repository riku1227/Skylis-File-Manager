# High refresh motion implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development or superpowers:executing-plans. Track completion here and in target/high-refresh-work/progress.md.

**Goal:** Native high-refresh presentation with optional 60fps cap and responsive bounded scroll motion.
**Architecture:** Minimal pinned GPUI core patch; shared preferences and OS policy; one active folder motion driver per window, using native frame callbacks and pure interpolation/classification.
**Tech Stack:** Rust, GPUI Fast =0.1.15, Windows/macOS adapters.
**Spec:** ../specs/2026-11-15-high-refresh-motion-design.md (user approved).

## Global constraints

- Keep GPUI =0.1.15; preserve demand-driven idle and existing power/thermal policies.
- No filesystem scans, image decoding, shell calls or recursive sizes on UI thread.
- High refresh and smooth scroll default on. OS reduce motion overrides smoothing.
- Never wait for motion before opening files/navigation; preserve last painted hit-test positions.
- Existing README/windows-build changes in original checkout are unrelated and must survive.

## Review focus

- Continuous wheel input represented as Lines must not be delayed: classification tests in task 2.
- A click between frames must select the painted row: integration test in task 2.
- Hidden/detached tabs must not retain callbacks: lifecycle test in task 2.
- Initial and detached windows must inherit limit: settings integration test in task 3.
- Dirty/forced presentation must survive throttling: frame gate tests in task 1.

## Task 1: Runtime presentation cap

Files: vendor/gpui-fast, Cargo.toml, Cargo.lock, licenses/README.md.
Interface: Window::set_frame_rate_limit(Option<f64>) and frame_rate_limit()->Option<f64>; reject nonfinite/nonpositive values without changing prior limit (Result preferred).
- [x] Write and run failing frame limit tests against actual frame gate/pacer.
- [x] Vendor fixed core with license; expose runtime setter without changing WindowOptions; longest applicable interval wins, mandatory presentation exempt, setter wakes pending demand.
- [x] Test None compatibility, 60 cap under multiple callback Hz, change while pending, power limits and mandatory present; run cargo check/test using patched core.
- [x] Commit independently after parent review.

## Task 2: Folder scroll motion

Files: apps/explorer/src/motion.rs (or motion/), folder.rs, folder/{marquee,scrollbar,selection_geometry,grouping,media_view,tests}.rs.
Consumes: Settings.high_refresh_rate/smooth_scroll, App::reduce_motion(), crate::motion_preferences::wheel_scroll_lines(&App)->Option<u32>.
Produces: FolderView::cancel_scroll_motion(&mut self) for outer lifecycle. One callback chain for visible folder per window; motion and marquee share it.
- [x] Pure failing tests for monotone retargeted Hermite, painted-position freeze, frequency-independent motion and conservative wheel classification.
- [x] Implement 100ms bounded motion, integer notch/25ms dense input classifier/150ms idle reset, public capture hook, direct precise input and no double scroll.
- [x] Replace fixed marquee timer with elapsed seconds (1125px/s maximum, dt capped 32ms), retain selection geometry.
- [x] Integration tests for hit-test, nested/modal input, direct scroll, navigation/layout/tab cancellation and pending callback bounds; run workspace tests.
- [x] Commit independently after parent review.

## Task 3: Settings and OS policy

Files: settings_data.rs, explorer.rs, explorer/settings_panel.rs, main.rs, motion_preferences.rs and platform adapter if needed.
Consumes: Task 1 frame setter; task 2 cancellation hook.
- [x] Test old settings/defaults/new round trips/invalid booleans before implementation.
- [x] Add persistent toggles and runtime root setup; retain display subscription, show approximate Hz without claiming FPS; apply limit before first draw including detached windows.
- [x] OS reduce-motion/scroll-lines adapter; startup/activation/settings-open refresh, retain last successful value and report unknown honestly.
- [x] Verify preference changes stop obsolete motion; workspace tests/fmt/check.
- [x] Commit independently after review.

## Task 4: Release verification and review

Files: docs/implementation/high-refresh-motion-verification.md.
- [x] Build release, run appropriate workspace and vendor tests, formatting and diff checks.
- [x] Native UI checks for settings, wheel, click, marquee, navigation, idle and cap. Measure actual presentation cadence where tools permit, distinguish unavailable measurements.
- [x] Fresh whole-change review; fix material issues and rerun affected checks.
- [x] Record measurements and limits, commit verification, integrate onto original feature branch without staging unrelated files.

## Execution decisions

The user's explicit instruction to implement authorizes proceeding without another plan approval. Parallel workers have disjoint ownership (GPUI dependency vs folder motion); parent owns settings and integration. This overrides the generic skill's sequential implementer convention to satisfy the active parallel-work instruction while avoiding file conflicts. Worktree branch starts at 521602b. No push or merge to main is part of this task.

## Completion evidence

Completed 2026-11-15. Runtime core commit d00488b and app commit 866e08a are integrated by fast-forward into feat/explorer-workflows. Tasks 2 and 3 share one independently usable app commit because their module/settings/driver interfaces require each other. All 163 tests passed (two existing ignored); final release and independent review passed. Hardware results and explicitly unavailable checks are recorded in ../../implementation/high-refresh-motion-verification.md. Original unrelated files are preserved. The worktree is retained for local verification artifacts and the tested binary.
