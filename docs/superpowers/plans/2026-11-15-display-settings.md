# Display Settings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement the approved Sol v2 display popup and its state scopes.
**Architecture:** FolderView owns the popup and tab sorting. Preferences owns runtime hidden visibility; Explorer owns window pane visibility and receives tab events. Existing listing/filter/media paths remain authoritative.
**Tech Stack:** Rust, GPUI Fast, existing Preferences entities.
**Spec:** docs/implementation/display-settings.md

## Global Constraints
- No extra filesystem scans or image work on the UI thread.
- No new dependencies; runtime visibility is not persisted.
- Explicit staging and Conventional Commits with intent and validation bodies.

## Review Focus
- New and transferred tabs inherit destination window pane state.
- Hidden filtering propagates to inactive tabs and new windows without rescanning.
- Popup input cannot trigger file open/preview/search clearing.
- Outside/trigger clicks cannot immediately reopen a dismissed popup.
- Narrow/short windows retain access to all controls.

### Task 1: Share display state
Files: settings.rs, explorer.rs, explorer/tabs.rs, folder.rs, folder/tests.rs.
- [x] Add regression tests for hidden visibility across windows/new tabs and pane visibility across tabs/window transfers; run and observe failure.
- [x] Add runtime Preferences.show_hidden with setter; subscribe FolderView filtering to it. Keep FolderView.pane as rendered projection of Explorer.pane; emit SetPane events for changes and apply destination state when attaching tabs.
- [x] Run targeted GPUI tests and cargo check; include shared state in the complete popup feature commit.

### Task 2: Popup and sorting
Files: folder/display_settings.rs, folder.rs, folder/context_menu.rs, main.rs, folder/tests.rs, explorer.rs.
- [x] Add tests for popup opening/continuous edits, idempotent sorting and selected path retention, Esc/outside/trigger/Tab dismissal and bounds; observe failure.
- [x] Build scoped popup with current theme tokens, explicit sort controls and shared state setters. Replace existing toolbar/status controls. Close obsolete popups during navigation, window/tab changes and settings/context menu opening.
- [x] Run meaningful targeted tests, full workspace tests/check, formatting and diff checks. Review final diff and commit with validation.

## Execution rulings and results
- User approved Sol v2 and requested implementation; execute inline in the existing isolated T3 worktree without a further design approval gate.
- Hidden visibility remains runtime-only; new window pane starts enabled, matching existing behavior.
- Shared state and popup are committed together as one usable feature; tests exercise them together.
- New tests initially failed on missing state/events/popup. The 320px test exposed a toolbar overflow (trigger at x=399); narrow toolbar dimensions now keep it reachable.
- Fresh review found incoming-tab dismissal and outside-click focus gaps. Incoming-tab regression reproduced the failure and passes after the fix; outside dismissal now explicitly returns focus before target mouse handling. Refresh also restores focus on dismissal.
- Final verification: `cargo test --workspace --locked --offline` passed 231 explorer tests and 49 core tests (2 existing ignored). `cargo check --workspace --locked --offline`, touched-file rustfmt check and `git diff --check` passed.
- Native Windows light/dark smoke: popup opens, gallery updates underneath while remaining open. Temporary APPDATA isolated saved preferences; verification process closed afterward. GPUI tests cover 320x240, bound file-key containment, Tab exit, sort selection, cross-window hidden and pane transfer scopes.
