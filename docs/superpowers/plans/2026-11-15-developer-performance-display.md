# Developer performance display implementation plan

> **For agentic workers:** Use superpowers:executing-plans. Keep unrelated README/windows-build changes intact; stage explicit files.

**Goal:** A developer option shows FPS and process memory while using Skylis.
**Design:** Default-off persisted `performance_overlay` setting under 開発者向け. A small dedicated footer avoids covering files and uses the existing theme. It shows per-window submission FPS (excluding HUD-only refresh), approximate display Hz, and app-wide resident memory in MiB. These are not physical display completions, private/GPU memory, or CPU draw-time reciprocals.
**Architecture:** Opt-in lightweight GPUI present counter plus one isolated child entity per root. Only enabled, visible windows run a one-second sample task; hidden/disabled/dropped views cancel it. Memory uses a small OS adapter and one app-wide one-second cache. The child notifies only itself and only when formatted values change. No forced continuous animation or global profiler feature.
**Tech stack:** Rust/GPUI Fast =0.1.15, existing Windows/macOS OS bindings.

## Decisions and constraints

- User authorized the feature; placement, refresh interval and labels are implementation choices. Proceed without another permission flow under the active developer instructions.
- Existing GPUI profiler overlay was considered but measures CPU draw times, lacks memory, and requires broader instrumentation. Rendering/callback counts in app code also misreport synchronous draws. Use the actual submission boundary with explicit labeling.
- HUD-only frames must not create fake idle FPS. Coalesced real work must still count once; CPU-only draws do not count.
- Memory failure/unsupported is 取得不可, never a fabricated zero. Windows resident working set includes shared pages. macOS RSS is implemented only through existing bindings.
- Root renders the HUD entity directly, never observes its sample state. Font/style updates use the same preferences as the rest of the app.
- Shared preferences update all windows and new/detached roots. Old configs default off; no settings version bump.
- No filesystem scans, subprocesses, image decoding, unbounded history, or independent render loop. Disabled option owns no timer or memory polling.

## Tasks and review focus

- [x] GPUI API `set_frame_diagnostics_view(Option<EntityId>)`, `frame_diagnostics_count()->Option<u64>`. Test actual native callback, HUD-only exclusion, coalescence, synchronous draws, disable/reset and window independence. Preserve pacing semantics.
- [x] OS adapter `diagnostics::memory::resident_bytes()->Option<u64>` and bounded app-wide caching. Validate current Windows process and fail/unknown display behavior.
- [x] Persist setting and add row8 / shortcut8. Implement child footer, actual elapsed-time sample math, hidden/off cancellation, restart baselines and theme consistency. Test meaningful lifecycle/formatting behavior.
- [x] Workspace and vendor tests, formatting, release build, independent review, native UI check (off/on, idle0, movingFPS, memory, restart/persistence), then explicit-file Conventional Commits and updated executable.

The parent owns app/settings/integration. Parallel workers own only vendor files or memory.rs; no overlapping edits or child checkouts. This implementation uses the existing checkout and preserves unrelated local edits.

## Completion evidence

177 applicable tests passed;2 pre-existing ignored. Release and independent review passed. Native UI confirms toggle, scrollingFPS, idle0, memory and both themes. Settings persistence and visibility restart have automated coverage; interactive restart was not performed because the user began operating the verification window, which was retained. See ../../implementation/developer-performance-display.md for precise metric semantics and verification scope.
