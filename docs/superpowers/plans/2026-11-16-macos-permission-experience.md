# macOS Permission Experience Implementation Plan

> **For agentic workers:** Use the approved delegated workflow, implementing the tasks below sequentially without further delegation. The parent owns design and review.

**Goal:** Reduce unsolicited macOS privacy prompts and provide a dismissible, contextual recovery guide when an explicitly opened folder is inaccessible.

**Architecture:** Navigation chrome uses path-independent icons on macOS. Existing asynchronous directory-listing results drive a small nonmodal permission notice; no global FDA probe or automatic permission request is introduced.

**Tech Stack:** Rust, GPUI Fast 0.1.15, existing bounded icon worker and AppKit adapter.

**Spec:** The approved conversation and the design below. The user approved proceeding with reduced speculative access, contextual requests, and a small settings/retry/dismiss guide. Continue GPT-6.1-Sol High implementation with parent review.

## Design and scope

The confirmed code path is Home/sidebar -> object_icon -> ShellIcons::get -> NSWorkspace iconForFile with a real path. This is an avoidable access candidate, not a proven explanation of the user's original TCC prompts. The existing Home listing does not enumerate favorites, and capacity requests target root/home only. Preserve that behavior.

Prefer eliminating speculative native path lookups over adding an unconditional startup permission wizard. A wizard would demand broad access before it is needed; a notice alone would not remove decorative accesses. Use path-independent navigation icons plus a contextual, dismissible notice. Do not infer consent or full-disk status from cached successful reads.

On macOS, Home cards, sidebar favorites/pins/drives, tabs and address chrome must not issue exact-path icon requests. Use a small semantic icon interface, native named/type icons where supported and existing bundled semantic SVG assets as fallback. Preserve recognizable folder/drive icons and retain normal native icons for entries in the folder the user actually opened. Check directory/package icon paths for accidental child traversal; do not add directory probing to choose an icon. Windows behavior stays unchanged.

When the current explicit listing fails with PermissionDenied, show one small nonmodal notice inside that folder view: "このフォルダーにアクセスできません", with settings, retry and dismiss actions. Explain that access settings or the folder's own permissions may be responsible; do not claim FDA is disabled. Reuse OpenPermissions to reach the existing settings guide and F5/Refresh behavior for retry. Dismissal must survive rerenders and automatic refreshes for that visit; an explicit retry can report another denial. Navigation to another location and a successful retry clear stale notice state. Preserve the main error/recovery surface so dismissal never traps the user.

## Global constraints

- No filesystem scans, native icon extraction, decoding or recursive calculations on the UI thread.
- Keep existing worker/cache bounds and obsolete-result cancellation.
- No TCC database reads, privacy permission toggles/resets, privileged helpers, new dependencies or FDA guessing probes.
- Do not promise removal of all OS permission dialogs; standard OS consent can still occur for a user-requested operation.
- No blanket FDA prerequisite, repeated automatic retries, startup permission modal, or automatic reopening of a dismissed notice.
- Use Conventional Commits with explanatory bodies; stage explicit files and preserve unrelated work. No push or PR.

## Review focus

1. Home/sidebar rendering with protected or nonexistent pinned paths must enqueue no exact native icon lookup.
2. Archived/nonexistent tab paths and breadcrumbs must not become native path probes.
3. Dismissal plus automatic refresh must not repeatedly show the same notice.
4. Late results from a previous location must not show a notice or clear a denial in the new location.
5. Success, non-permission errors and non-macOS builds must not show misleading permission UI.

## Task 1: Path-independent navigation icons

**Files:** `apps/explorer/src/shell_icons.rs`, `shell_icons_macos.m`, `folder.rs`, `folder/home.rs`, `explorer.rs`; add a small focused module only if useful.

- [ ] Add regression coverage at the icon request boundary proving real protected/pinned paths are not queued by macOS navigation rendering. Include absent paths and archive/tab cases without performing real protected-folder reads.
- [ ] Implement a path-independent navigation icon API and migrate navigation chrome. Keep visible file-entry native icons and Windows behavior intact.
- [ ] Inspect startup, capacity, watcher and thumbnail paths for speculative protected-directory access. Fix only concrete paths relevant to this task; record remaining uncertainty.
- [ ] Run focused tests, format and diff checks; commit the independently usable icon change with its intent and validation.

## Task 2: Contextual recovery notice

**Files:** `apps/explorer/src/macos_permissions.rs`, `folder.rs`, an optional `folder/permissions.rs`, and existing permission settings only as needed.

- [ ] Add state-transition tests for denied -> dismiss -> rerender/automatic refresh; explicit retry -> denied/success; navigation cancellation; non-permission errors. Use existing GPUI tests for action routing where feasible.
- [ ] Implement the notice, settings/retry/dismiss actions, and per-visit dismissal behavior. Maintain existing error access and keyboard navigation. Fit minimum 900x600 windows without covering navigation controls.
- [ ] Run `cargo test --locked --workspace --target aarch64-apple-darwin`, `cargo fmt --all -- --check`, and `git diff --check` once final code is ready.
- [ ] Build/sign `bash scripts/build-macos.sh release`. Verify actual UI with the exact bundle, keeping it uncovered. Use an owned disposable chmod-denied folder for denial/dismiss/retry/recovery, restoring its permissions afterwards; do not change TCC/FDA or access unrelated user data.
- [ ] Commit the notice and concise verification documentation, including this plan. Report precisely which prompt behavior was tested and which remains unproven without resetting privacy permissions.

## Parent acceptance

- [ ] Review full implementation diff against the boundary and lifecycle constraints.
- [ ] Inspect test/build evidence and native UI; fix small findings directly.
- [ ] Leave the normal signed release running with no temporary instrumentation; report commit hashes and verification limits.
