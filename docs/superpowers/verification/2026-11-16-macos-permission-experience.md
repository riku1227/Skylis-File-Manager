# macOS permission experience verification

Implemented navigation decoration without exact-path shell requests on macOS, and a small nonmodal folder access notice with settings, retry and dismiss actions. The native adapter already supports path-independent folder/content-type icons, so no Objective-C bridge change was needed. Bundled semantic icons preserve Home/favorites/drive identity; pins use native generic folder icons. Native file-entry icons inside opened directories and Windows request behavior remain unchanged.

The notice records only the current requested listing. Dismissal survives rendering and watcher failures (including intervening non-permission failures), until explicit retry, navigation or successful access. Existing error/status settings links and F5 remain available after dismissal. Listing receivers and watcher completions retain their cancellation checks; stale success/denial cannot update a later visit.

## Automated checks

- `cargo test --locked --workspace --target aarch64-apple-darwin`: 212 explorer and 44 core tests pass, plus doc tests. Baseline was 207 + 44.
- Added tests cover navigation request queue keys, actual Home/sidebar/address rendering with protected-looking/nonexistent pins, per-visit dismissal/retry/success/non-permission transitions, watcher dismissal, cancelled old listing success/denial, and 900x600 layout bounds plus actual dismiss hit testing.
- `cargo fmt --all -- --check` and `git diff --check` pass.
- `bash scripts/build-macos.sh release` builds and verifies the configured certificate signature. Existing deprecation/unused-code warnings remain.

Local logs: `/tmp/skylis-permissions-tests.log`, `/tmp/skylis-permissions-build.log`, `/tmp/skylis-permissions-format.log`, `/tmp/skylis-permissions-diff-check.log`. Initial failing request-boundary and notice-state compilation logs: `/tmp/skylis-permissions-icons-red.log`, `/tmp/skylis-permissions-notice-red.log`; focused passing icon test: `/tmp/skylis-permissions-icons-green.log`.

## Native acceptance

Used CUA with the exact normal bundle `target/aarch64-apple-darwin/release/Skylis.app`. T3 Nightly was minimized so Skylis remained exposed and continued drawing. No backend refresh workaround or temporary overlay was introduced.

Created owned `/tmp/skylis-permission-acceptance-k44e8nnw` with `recovered.txt`, denied access with mode 000, then restored mode 700. Confirmed:

- Home semantic icons and root/Home capacity meters render normally.
- Tabs switch, new tab opens Home, sidebar Home navigation works, address and tab folder icons remain native generic folders.
- Explicitly opening the denied fixture shows the folder-specific notice, without an FDA-disabled claim.
- Dismiss removes the notice; subsequent redraw and an attribute-change watch refresh did not reopen it. The persistent error/status recovery links remain.
- F5 and the notice retry button can report denial again.
- The settings action opens the existing macOS access guide. No privacy setting was changed.
- Restoring fixture permissions and pressing F5 restores the file row and clears the notice/error.

Evidence screenshots under `/tmp`:

- `skylis-permissions-home.png`, `skylis-permissions-tabs-sidebar.png`
- `skylis-permissions-denied.png`, `skylis-permissions-dismissed.png`
- `skylis-permissions-retry-denied.png`, `skylis-permissions-button-retry-denied.png`
- `skylis-permissions-settings.png`, `skylis-permissions-recovered.png`
- `skylis-permissions-final-denied.png`, `skylis-permissions-final-dismissed.png`, `skylis-permissions-final-retry-denied.png`, `skylis-permissions-final-recovered.png`

Native resizing through CUA edge drags did not change the window size; minimum-size geometry and dismiss hit testing are verified by the 900x600 GPUI visual test instead. The normal signed app is left running, with no instrumentation.

## Inspection and limits

Startup favorites are lexical paths; Home does not enumerate them. macOS drives are root only, with capacity workers querying root and Home. Watches attach to the requested location. Address candidate metadata checks are tied to user-entered paths on a worker. Thumbnail requests originate from opened-directory entries/selected previews; file metadata checks occur on workers. Navigation icon selection adds no filesystem checks or package traversal. Worker/cache limits remain unchanged.

The removed `NSWorkspace iconForFile` call was a confirmed speculative path lookup, not a proven historical TCC trigger. The chmod test verifies filesystem denial recovery; it does not prove absence of fresh-install OS TCC prompts. No TCC database reads, grants, revokes, resets or global FDA probes were performed. Windows behavior was preserved by conditional routing but was not executed on this macOS host.


## Parent review

Reviewed the complete `fa80499..3d379ce` implementation diff and the surrounding listing/watch lifecycle. Home, sidebar, tab and address decoration now use semantic assets or the type-only request route on macOS. Remaining exact-path icon calls are for entries in opened directories or their previews. Navigation and watch cancellation guards remain in place; dismissal survives automatic failures and resets on an explicit retry, navigation or successful access. No additional application changes were needed.

Inspected the recorded 212 explorer + 44 core passing test results, release build output, and native denial/recovery screenshots. Independently verified the final bundle with `codesign --verify --deep --strict --verbose=2` and checked the repository diff for whitespace errors. Also inspected the running normal release Home screen: icons/capacity rendered and no permission notice appeared. The documented TCC, native minimum-window-size, and Windows validation limits still apply. No temporary instrumentation or privacy setting changes were added during review.
