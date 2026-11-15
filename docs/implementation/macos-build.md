# macOS local build and permissions

Skylis uses Rust/GPUI Fast and ships here as a non-sandboxed local `.app`.
The bundle identifier is `dev.skylis.file-manager`; its executable is `Skylis`.
This is a development bundle with an ad-hoc signature, not a notarized distribution.
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

Ad-hoc signatures change when code changes. The OS may require manually removing
and adding the current bundle again after a rebuild. For stable distribution,
Developer ID signing and notarization remain future work. This workflow never
grants/revokes permissions or resets TCC.

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

Release commands are provided but the optimized release build has not been run.
Intel, Developer ID/notarization, and interactive FDA grant/revoke/relaunch
transitions remain unverified. Existing unrelated platform gaps remain, including
unavailable drive-capacity reporting on this host. Existing deprecation/dead-code
warnings are present; they do not prevent the debug build or tests.

Archive tests canonicalize their disposable temp root because macOS `/var` is a
system symlink and the archive engine deliberately rejects symlink ancestors.
This does not relax that production safety rule. Quick Look tests use a headless
GPUI window and assert honest native-handle failure; actual Quick Look parity is
outside this verification.
