# Skylis developer guide

**English** · [日本語](README_DEV.ja.md)

How to build, test and work on Skylis. For an introduction to the app and how to use it, see [README.md](README.md).

- [Tech stack](#tech-stack)
- [Repository layout](#repository-layout)
- [Setting up on Windows](#setting-up-on-windows)
- [Setting up on macOS](#setting-up-on-macos)
- [Building and running](#building-and-running)
- [Tests and checks](#tests-and-checks)
- [CI and dependency policy](#ci-and-dependency-policy)
- [Release artifacts](#release-artifacts)
- [Development rules](#development-rules)
- [Design documents](#design-documents)

> [!NOTE]
> Most of the design and implementation notes under `docs/` are written in Japanese.

## Tech stack

| Item | Details |
| --- | --- |
| Language | Rust (edition 2024, stable) |
| UI | [GPUI Fast](https://crates.io/crates/gpui-fast) 0.1.15 (GPU rendering, no WebView) |
| Targets | Windows 10/11 x64 (primary), macOS Apple Silicon (local `.app`) |
| OS integration | Windows: Shell APIs via the `windows` / `windows-sys` crates. macOS: an Objective-C adapter (`shell_icons_macos.m`) |
| Archives | `zip` 9.6.0, `sevenz-rust2` 0.25.0 (locally patched) |

All GPUI dependencies are declared once in the root `Cargo.toml`. `gpui-fast` and `sevenz-rust2` are vendored with patches under `vendor/` and swapped in through `[patch.crates-io]`. Each directory's `SKYLIS-PATCH.md` records the changes and when the patch can be removed.

## Repository layout

```text
.
├── apps/explorer/          # The GUI app (package: explorer, executable: skylis)
│   ├── assets/             # App icon (branding/) and UI icons (icons/)
│   ├── src/                # Views, input, OS integration and workers
│   ├── app.rc / build.rs   # Windows resources (icon and version info)
├── crates/explorer-core/   # UI-independent logic (listing, sorting, history, search, archives)
├── vendor/                 # Patched dependency sources (gpui-fast, sevenz-rust2)
├── packaging/              # macOS Info.plist and the license notice template
├── scripts/                # macOS build and signing, icon and license generation
├── docs/                   # Design, implementation notes and verification records
├── research/               # Research notes
└── licenses/               # Licenses of vendored third-party sources
```

How responsibilities are split:

- **`explorer-core`** holds logic that does not depend on GPUI: file listing, sorting, history, name search and archives. Unit tests live here.
- **`apps/explorer`** holds rendering, input, worker coordination and OS adapters. Platform-specific code stays in small adapters such as `native_shell/`, `shell_icons*` and `folder_integration.rs`.

## Setting up on Windows

Requirements:

- Git
- Rust stable managed by rustup, with the MSVC toolchain (`x86_64-pc-windows-msvc`)
- Visual Studio 2022 or later (or the Build Tools) with the “Desktop development with C++” workload, including the MSVC x64/x86 build tools and a Windows SDK

```powershell
# After installing Rust
rustup show          # the active toolchain should be *-pc-windows-msvc
rustc -Vv
cargo -V
rustup component add rustfmt
```

`cl.exe` does not need to be on your PowerShell `PATH`; Cargo finds MSVC on its own.

Troubleshooting:

| Symptom | What to check |
| --- | --- |
| `cargo` not found | Reopen PowerShell and make sure `%USERPROFILE%\.cargo\bin` is on `PATH` |
| `link.exe` not found | The MSVC x64/x86 build tools are installed in the Visual Studio Installer |
| `kernel32.lib` etc. not found | A Windows SDK is installed in the Visual Studio Installer |
| Fetching dependencies fails | Proxy settings and network access. Do not delete `Cargo.lock` |

More details and a record of a verified setup are in [docs/implementation/windows-build.md](docs/implementation/windows-build.md) (Japanese).

## Setting up on macOS

1. Install the full Xcode at `/Applications/Xcode.app` and finish its first-launch setup.
2. Install Rust and the Metal toolchain:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh
sh /tmp/rustup-init.sh -y --profile minimal --default-toolchain stable
. "$HOME/.cargo/env"
rustup component add rustfmt
xcodebuild -downloadComponent MetalToolchain
```

The build script uses `DEVELOPER_DIR` (defaulting to the Xcode above) and never changes the global `xcode-select` setting.

For signing, the script picks the only valid Apple Development or Developer ID Application certificate automatically. If there are several, choose one with `SKYLIS_SIGNING_IDENTITY`. If there are none, it warns and falls back to ad-hoc signing. See [docs/implementation/macos-build.md](docs/implementation/macos-build.md) for details, including Full Disk Access.

## Building and running

### Windows

```powershell
# Debug run
cargo run --locked -p explorer

# With arguments (everything after -- goes to the app)
cargo run --release --locked -p explorer -- --theme dark --path "C:\Users"

# Release build → target/release/skylis.exe
cargo build --release --locked -p explorer
```

The first build takes a few minutes while GPUI and the image libraries compile. The MSVC C runtime is linked statically (`.cargo/config.toml`), so the Visual C++ Redistributable is not needed on target machines.

### macOS

```sh
bash scripts/build-macos.sh debug --launch   # build and launch
bash scripts/build-macos.sh release
# Output: target/aarch64-apple-darwin/{debug,release}/Skylis.app
```

Do not rebuild over a running app: quit it, rebuild, then relaunch.

### T3 Code scripts

[`t3.json`](t3.json) registers build scripts for Windows and macOS (`release(Windows x64)`, `launch(macOS debug .app)` and others).

### Command-line arguments

| Argument | Behavior |
| --- | --- |
| `--path <PATH>` / `<PATH>` | Open the given folder |
| `--home` | Open the Home screen (cannot be combined with `--path`) |
| `--select <PATH>` | Open the parent folder and select the item |
| `--theme light\|dark\|system` | Choose a theme (defaults to the saved setting or the OS theme) |
| `--restore-integration` | Restore the Windows integration without opening a window, report to stderr and exit (must be the only argument) |
| `--` | Treat everything after it as positional (for paths starting with `-`) |

`--switch-explorer`, `--shell-folder`, `--handoff-token` and `--shell-command-server` are for internal use by the Windows integration.

### Where settings and logs live

| Kind | Windows | macOS |
| --- | --- | --- |
| Settings and session | `%APPDATA%\Skylis\` | `~/Library/Application Support/Skylis/` |
| Crash logs | `%LOCALAPPDATA%\Skylis\logs\` | `~/Library/Logs/Skylis/` |

To keep your everyday settings untouched while developing, point `APPDATA` at a scratch directory:

```powershell
$env:APPDATA = "$PWD\target\dev-appdata"; cargo run --locked -p explorer
```

## Tests and checks

```powershell
cargo test --workspace --locked        # all tests
cargo test --locked -p explorer-core   # core only
cargo check --workspace --locked       # type check
cargo fmt --all -- --check             # formatting
git diff --check                       # whitespace errors
```

Tests that depend on a desktop environment, such as those that load installed Shell extensions, are marked `#[ignore]`. Run them explicitly with `cargo test --locked -p explorer -- --ignored` when needed.

The verification scripts for the HTML prototypes in `docs/design/` use Node.js, Playwright and Google Chrome (`PLAYWRIGHT_PATH` can point to an existing install). None of these are needed to build the Rust app.

## CI and dependency policy

[`.github/workflows/ci.yml`](.github/workflows/ci.yml) runs on pushes to `main` and on pull requests.

| Job | What it does |
| --- | --- |
| Windows build and tests | `cargo fmt --check` → `cargo test --workspace --locked` → release build |
| Dependency policy | `cargo deny check licenses sources bans`, plus a check that local path dependencies are limited to an allow list |

When you add or update dependencies, run the same check locally:

```powershell
cargo +stable install --locked cargo-deny --version 0.22.0
cargo +stable deny --locked check licenses sources bans
```

Allowed licenses and exceptions are listed in [`deny.toml`](deny.toml). Adding a new path dependency also requires updating the allow list in CI. See [docs/implementation/continuous-integration.md](docs/implementation/continuous-integration.md) for details.

## Release artifacts

- Windows: `target/release/skylis.exe` (the icon and version info are embedded from the version in `Cargo.toml`)
- macOS: `Skylis.app` (bundle ID `dev.skylis.file-manager`, locally signed; not notarized yet)
- Third-party license notices: run the following in PowerShell 7 and ship the generated `target/dist/THIRD_PARTY_LICENSES.html` alongside the executable.

```powershell
cargo +stable install --locked cargo-about --version 0.9.3 --features cli
./scripts/generate-third-party-licenses.ps1
```

If you build an uninstaller, have it run `skylis.exe --restore-integration` as the user who enabled the integration, before deleting any files.

To update the app icon, edit `apps/explorer/assets/branding/skylis.svg` (and `skylis-small.svg` for small sizes), then regenerate the PNG / ICO / ICNS files with `node scripts/generate-app-icons.cjs` (requires `sharp`). Regular builds use the checked-in outputs.

## Development rules

[AGENTS.md](AGENTS.md) is the source of truth. In short:

- **Stay light**: never scan the filesystem, call Shell extensions, decode images or compute recursive sizes on the UI thread.
- **Bound everything**: cap worker concurrency, pending queues and image memory. Cancel requests that navigation made obsolete, and never replace a valid displayed image with a loading placeholder.
- **Keep integrations behind adapters**: put platform integrations behind small, explicit adapters, and report unsupported capabilities honestly.
- **Commits**: follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:` / `fix:` / `perf:` / `docs:` …). Commit each independently usable feature separately, with a body explaining intent, user-visible behavior, constraints and validation.
- Do not commit temporary explanation HTML or local verification artifacts.

## Design documents

Most of these documents are written in Japanese.

| Document | Contents |
| --- | --- |
| [Design reference](docs/design/explorer-design-system/index.html) | The Prism Canopy Compact design system |
| [Native UI plan](docs/implementation/native-ui-plan.md) | Screen structure and implementation approach |
| [GPUI Fast migration](docs/implementation/gpui-fast.md) | Adopting and patching GPUI Fast |
| [Everyday file operations](docs/implementation/daily-use.md) | Copy, move, archives and file watching |
| [Drag and drop](docs/implementation/drag-drop.md) | Drag-and-drop rules and limits |
| [Folder integration and pins](docs/implementation/folder-integration-and-pins.md) | Registering and restoring the Windows integration |
| [Session restore](docs/implementation/session-restore.md) | Saving tabs and windows |
| [Keyboard workflows](docs/implementation/keyboard-tab-workflows.md) | Reopening and duplicating tabs |
| [Motion](docs/implementation/motion.md) | Animation and high refresh rates |
| [Roadmap](docs/implementation/daily-driver-roadmap.md) | What comes next |
| [Verification and gaps](docs/implementation/verification.md) | Results of UI and test verification |
| [Archive backend](crates/explorer-core/src/archive/README.md) | Supported formats, limits and safeguards |
| [Third-party sources](licenses/README.md) | Origins of vendored sources |
