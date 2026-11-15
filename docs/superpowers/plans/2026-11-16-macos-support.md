# macOS support implementation plan

> Worker: GPT-6.1-Sol, reasoning effort High. Implement directly, without further delegation. Parent owns design and review.

**Goal:** Build and launch Skylis on this Apple Silicon Mac, expose macOS build commands in t3.json, and provide an honest, usable Full Disk Access settings flow.

**Architecture:** Keep the Rust/GPUI Fast application and existing platform boundaries. Package a stable-identifier, locally signed, non-sandboxed Skylis.app for direct local use. Permission observations and OS actions belong in a small macOS adapter; filesystem work and subprocess waits stay off the UI thread.

**Design decisions (parent):** Direct local app bundle rather than App Store sandbox/security-scoped bookmarks or bare cargo executable as the normal launcher. The current file-manager model requires arbitrary user-selected navigation; a bundle provides a recognisable target in macOS Privacy settings. Distribution/notarization and unrelated platform feature parity are out of scope. Preserve Windows support.

## Constraints and review focus

- No UI-thread filesystem scans, image decoding, recursive sizes or blocking shell waits. Bound background workers and request queues, reject obsolete navigation results.
- Never claim FDA is granted solely because one directory is readable, or missing solely because an ordinary POSIX permission was denied. No private TCC APIs/database scraping; no silent permission changes or privacy resets.
- No blanket launch-time probes of Mail/Messages/etc. Use real failed navigation as evidence; initially show permission state as unconfirmed. Recheck/retry must have a defined observable meaning and preserve unknown states.
- Explain Files & Folders vs Full Disk Access, offer opening the system privacy pane, and expose the running .app in Finder for manual addition. Include a manual navigation fallback if a deep link fails, and guidance about relaunch and local ad-hoc signature changes.
- Default startup and ordinary accessible folders must work without FDA. File errors must not silently become empty successful folders.
- Build commands must work from a clean checkout and arbitrary cwd/path with spaces; detect required Apple build tools clearly. Verify Metal toolchain availability. Do not change system Xcode selection globally unnecessarily.
- Preserve unrelated changes; explicit git staging; Conventional Commits with intent, behavior, constraints and validation in body. No temporary screenshots/logs in git.

## Task 1: macOS environment, build and bundle

- Inspect Cargo manifests/lockfile, build.rs, existing macOS platform/media code, environment, and AGENTS.md.
- Install Rust stable using official rustup, minimally and noninteractively, if absent. Xcode is already at /Applications/Xcode.app/Contents/Developer. Host is arm64 macOS 27.0.1. Diagnose required compiler/Metal components and obtain official components if needed.
- Run a locked build and fix actual macOS compilation/link errors, keeping changes platform scoped.
- Add a reusable build script and bundle metadata under scripts/ and a suitable packaging directory. Include stable bundle identifier, executable, version, appropriate privacy purpose strings and local signing. Assets must resolve when launched from Finder with arbitrary cwd. Respect CARGO_TARGET_DIR or document its handling.
- Add debug and release macOS app build entries to t3.json (host architecture at minimum; clearly label any extra cross-build entries). Provide a launch command if useful.
- Verify script syntax, bundle plist/signature, actual locked build, launch, and existing tests. Commit independently usable build support.

## Task 2: permissions and recovery UI

- Read apps/explorer/src/{settings.rs,explorer/settings_panel.rs,folder.rs,explorer.rs} and crates/explorer-core/src/listing.rs before editing. Existing listing failures carry ErrorKind; preserve structured permission information through UI.
- Implement a focused macOS permission adapter and a settings category with cautious state text, system FDA settings action, show current bundle action and manual instructions. Guard actions against queue growth/repeated clicks; display failures.
- On permission-denied navigation, show an actionable route into the permissions category. Other errors remain distinct; stale navigation errors must not mark the current folder or permissions incorrectly.
- Add targeted tests for denial/unknown/access-success distinctions, navigation reset/stale handling, and action/path logic where appropriate. Avoid privileged actual permission toggles in tests.
- Verify regular file-manager navigation, settings, and denied-path UX on macOS with a disposable fixture. Do not grant/revoke privacy privileges automatically. State which interactive grant/relaunch cases remain user-only.
- Update README.md and docs/implementation/macos-build.md with reproducible environment setup, t3 commands, bundle path, FDA steps, signing limitations, tested host and actual results. Commit permission work separately.

## Parent acceptance

- Inspect implementation for privacy correctness, bounded background work, bundle identity and cross-platform regressions.
- Independently rerun relevant checks and launch the built app, inspecting actual native UI where tools allow.
- Final report includes artifact location, commands, results and precise remaining manual permission steps. Do not report untested Intel or externally notarized distribution as working.
