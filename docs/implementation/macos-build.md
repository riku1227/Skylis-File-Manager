# macOS local build and permissions

Skylis uses Rust/GPUI Fast and ships here as a non-sandboxed local `.app`.
The bundle identifier is `dev.skylis.file-manager`; its executable is `Skylis`.
This is a locally signed development bundle, not a notarized distribution.
Intel and App Store distribution are not validated.

## Environment

Install full Xcode at `/Applications/Xcode.app`, complete its first-launch setup,
then install stable Rust using the official installer:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh
sh /tmp/rustup-init.sh -y --profile minimal --default-toolchain stable
. "$HOME/.cargo/env"
rustup component add rustfmt
xcodebuild -downloadComponent MetalToolchain
```

The build script uses `DEVELOPER_DIR`, defaulting to the Xcode path above;
it does not change the global `xcode-select` setting. A command named `metal`
is insufficient: the script compiles a small shader to check the component.

## Build and launch

```sh
bash scripts/build-macos.sh debug --launch
bash scripts/build-macos.sh release
cargo test --locked --workspace
```

`t3.json` provides **debug(macOS native .app)**, **release(macOS native .app)**,
and **launch(macOS debug .app)**. Windows commands remain available.
The script can be invoked by absolute path from another working directory,
including paths containing spaces. It builds the native Rust host target explicitly.
`CARGO_TARGET_DIR` and Cargo's configured target directory are resolved through
`cargo metadata`. Default output:

```text
target/aarch64-apple-darwin/debug/Skylis.app
target/aarch64-apple-darwin/release/Skylis.app
```

Open the bundle with Finder or `open /absolute/path/Skylis.app`. SVG assets are
embedded, so Finder's working directory does not affect them. Do not rebuild an
app while it is running: quit it, rebuild, and relaunch. No entitlement grants
Full Disk Access.

## Access permissions

Ordinary accessible folders and the startup Home screen do not require Full Disk
Access (FDA). Settings → **macOSアクセス権限** starts unconfirmed: no sensitive
folder probes, TCC database reads, or private permission APIs are used.

**Files & Folders** controls individual categories such as Desktop, Documents,
Downloads, removable disks, and network volumes. **Full Disk Access** is broader.
A navigation denial can also be caused by POSIX permissions or ACLs; Skylis does
not equate it to missing FDA. A successful folder listing proves access to that
folder only. Unknown remains unknown for FDA even after success.

On a denied listing, choose **アクセス権限の設定を開く**. The permissions category
can open the system privacy pane or reveal the currently running `.app` in Finder.
OS actions run on a worker, with one action in flight across windows; failures
are displayed. Successful dispatch does not prove that System Settings showed
the requested pane. If the deep link does not work, open **System Settings →
Privacy & Security → Full Disk Access** manually. If needed, add the revealed
Skylis.app with **+** and enable it yourself. Quit and relaunch Skylis, navigate
to the affected folder, and press **F5**. This retries only that listing; it is
not an FDA check or a background scan. Navigating elsewhere clears that tab's
old observation, and cancelled listing callbacks cannot apply stale observations.

### Stable development signing

The build script uses `SKYLIS_SIGNING_IDENTITY` when explicitly set (certificate
SHA-1 or an identity accepted by `codesign`). An empty override is an error;
invalid identities fail at signing without downgrading to ad-hoc. Otherwise it
auto-selects a single valid **Apple Development** or **Developer ID Application**
identity from `security find-identity -v -p codesigning`. With multiple eligible
identities it stops: choose one with the override. With none it prints a warning
and falls back to ad-hoc; `SKYLIS_SIGNING_IDENTITY=-` explicitly chooses ad-hoc.
No certificates, trust settings or private-key ACLs are created or modified.

```sh
security find-identity -v -p codesigning
SKYLIS_SIGNING_IDENTITY='<certificate SHA-1>' bash scripts/build-macos.sh debug
```

The bundle ID stays `dev.skylis.file-manager`, and `codesign` generates its normal
certificate-based designated requirement (DR). No custom requirement is supplied.
Using the same certificate lets debug and release satisfy each other's DR even
when their code hashes differ. Apple's [TN3127: Inside Code Signing: Requirements](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements.md)
explains why ad-hoc identity is version-specific and certificate-based identity
can persist across builds. Certificate signing does not itself grant FDA.

Use the settings **Finder** button to identify the exact running bundle; debug
and release live in different directories. After migrating from ad-hoc signing,
remove the old FDA entry and re-add the current bundle yourself if needed, then
quit and relaunch. Stable certificate signing avoids relying on a new code hash
at each rebuild; ad-hoc builds may still need re-adding after code changes.
This workflow never grants/revokes permissions or resets TCC. FDA does not promise
to suppress every macOS privacy prompt, nor fix POSIX/ACL failures.

### Native icons and capacity

macOS file, folder, type and volume icons use NSWorkspace through a small native
adapter on the existing single icon worker. Each job drains an autorelease pool,
copies its NSImage, and draws into a private 16–256 pixel bitmap. Only straight
BGRA bytes cross to GPUI. Type lookups use an extension or generic folder type,
never a real file lookup; extensionless and unknown types use the OS generic
icon. The Windows virtual recycle-bin icon remains unsupported on macOS.
The existing bounded pending queue, image cache and fallback flow apply.
See Apple's [thread-safety guidance](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/Multithreading/ThreadSafetySummary/ThreadSafetySummary.html)
for NSImage and per-thread graphics contexts/autorelease pools.

The existing asynchronous capacity service now uses macOS `statfs`, with checked
64-bit multiplication and available-to-user `f_bavail` bytes. It queries `/` and
the home volume for the home cards; it does not enumerate mounted volumes.
APFS volumes share container free space, so root and home can have the same
available bytes. Finder's purgeable-space estimates can differ from `df`/`statfs`.
Invalid paths preserve OS errors; embedded NUL is rejected. Other unsupported
platforms keep reporting unavailable capacity.

## Verification record

Verified on Apple Silicon arm64 macOS 27.0.1 (26A434), Xcode 27.0
(27A266a), Rust stable 1.100.0 and Apple Metal Toolchain 27A266a:

- `bash scripts/build-macos.sh debug`: locked native build, plist lint,
  ad-hoc signing and strict signature verification passed. Also invoked from
  `/tmp` through a source path containing spaces.
- Launch Services (`open Skylis.app`) launched the actual signed bundle.
  Home, Japanese/Latin text, embedded icons and normal fixture navigation rendered.
  Native text requires the explicitly enabled GPUI `font-kit` feature.
- `cargo test --locked --workspace`: **204 explorer + 42 core tests passed**,
  zero failures; doc tests passed (zero cases). `cargo fmt --all -- --check`,
  `git diff --check` and `bash -n scripts/build-macos.sh` passed.
- A disposable POSIX mode-000 folder showed denial and a settings link.
  Restoring access on that fixture and pressing F5 restored the listing, cleared
  the old refresh error, and showed folder success with FDA still unconfirmed.
- The permissions action opened the actual Full Disk Access System Settings pane;
  the reveal action selected the running Skylis.app in Finder. No privacy switches,
  grants, revokes or TCC resets were performed.

Parent review also verified the optimized release build with
`bash scripts/build-macos.sh release`, including plist and strict signature
validation. The release `.app` launched through Launch Services, rendered Home
and Japanese text, displayed the permissions category with unknown initial state,
and revealed the exact release bundle in Finder. The parent independently repeated
the denied-folder → permissions settings → system FDA pane → restored access/F5
flow using a separate disposable fixture. After simplifying the permission copy
and making the action buttons visible, all **204 explorer + 42 core tests** passed
again. No FDA grants or revocations were required for these checks.

Intel, Developer ID/notarization, and interactive FDA grant/revoke/relaunch
transitions remain unverified. The follow-up below supersedes the earlier ad-hoc
signing and unavailable native icon/capacity behavior. Existing deprecation/dead-code
warnings are present; they do not prevent the debug build or tests.

Archive tests canonicalize their disposable temp root because macOS `/var` is a
system symlink and the archive engine deliberately rejects symlink ancestors.
This does not relax that production safety rule. Quick Look tests use a headless
GPUI window and assert honest native-handle failure; actual Quick Look parity is
outside this verification.

## Follow-up verification (2026-11-16)

- Native worker extraction tests cover visible, bounded root/volume and folder
  icons plus nonexistent Unicode/space/extensionless/unknown-type fallback names;
  invalid NUL/oversized inputs and the unsupported virtual bin are rejected.
- Root and home capacity tests compare against a fresh native `statfs` result,
  allowing concurrent disk writes; invalid/nonexistent paths are tested.
  A separate `df -k` and Python `os.statvfs` reference agreed: total
  494,384,795,648 bytes and approximately 73.85 GB available during verification
  (free space changes while building).
- Shell fixtures exercise zero, one and multiple identities, explicit identity,
  explicit ad-hoc and empty override behavior. Personal identity subjects are
  intentionally omitted from this document.
- Earlier release/debug ad-hoc DRs had different code hashes; the observed running
  app was the debug bundle, so prior release FDA authorization is a plausible
  mismatch. This is not evidence of the exact TCC decision or proof that all
  Documents/Downloads prompts are fixed. No TCC database was inspected and no
  FDA settings were changed.
- Final workspace tests: **205 explorer + 44 core passed**, zero failures; doc
  tests passed. Formatting, diff whitespace, shell syntax and signing fixtures
  passed. Debug and release builds passed with existing unrelated warnings.
- A valid Apple Development certificate signed both profiles without private-key
  authentication blocking. Strict signature verification passed, and each app
  satisfied the other profile's default certificate-based DR, with no `cdhash`
  term. A disposable executable rejected an explicitly invalid signing identity.
- To preserve the existing running debug bundle, builds used
  `CARGO_TARGET_DIR=target/macos-followup`. The final debug binary was packaged and
  signed as `target/macos-followup/aarch64-apple-darwin/debug/Skylis-verified.app`
  (same bundle ID). Release is `target/macos-followup/aarch64-apple-darwin/release/Skylis.app`.
- Launch Services started the separate debug review bundle. Actual native UI
  verification of icons, capacity and settings remains blocked: computer-use
  returned `cgWindowNotFound` for Skylis and Finder. No screen-rendering success
  is claimed from worker tests. The pre-existing running debug app was not
  overwritten or terminated. No FDA grants, revokes or re-add operations were
  performed, so Documents/Downloads prompt behavior after migration is unproved.

### Parent follow-up review

The parent independently reran the workspace suite: **205 explorer + 44 core
passed**, with zero failures. Signing selection fixtures, formatting and shell
syntax checks passed. Both review profiles satisfied each other's certificate
requirements. The normal `bash scripts/build-macos.sh release` command also
passed, replacing the default release artifact at
`target/aarch64-apple-darwin/release/Skylis.app` with the corrected, certificate-signed
build; its strict signature verification passed. The pre-existing debug process
was not overwritten. Quit that older app before opening the updated release.

The parent also encountered `cgWindowNotFound` with both debug and release
window capture. Native display verification remains pending an accessible desktop;
test pixel buffers and successful builds are not treated as proof of on-screen
rendering or of FDA prompt resolution.
