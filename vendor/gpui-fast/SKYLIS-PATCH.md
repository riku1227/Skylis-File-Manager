# Skylis GPUI core patch

Origin: the crates.io release `gpui-fast 0.1.15`, from
<https://github.com/longbridge/gpui-fast>. The upstream Apache-2.0 license is
preserved in `LICENSE`. The upstream crate did not include a `NOTICE` file.
Published crate checksum (SHA-256):
`a069272b90d908d15152711b38153edabcf67a5dcc490123197af7fd9ad78d9d`.

Both the workspace dependency and this crate remain pinned to `0.1.15`.
The workspace replaces only the core package through `[patch.crates-io]`;
platform backends, macros and other dependencies retain their registry versions.
The published `src`, `resources`, `build.rs`, normalized `Cargo.toml`, `README.md`
and license are retained. Cargo cache metadata, the upstream lockfile and original
workspace-dependent manifest are omitted. No external benchmark asset tree is
required by the published manifest.

Local changes:

- `Window::set_frame_rate_limit(Option<f64>) -> anyhow::Result<()>` and its getter
  provide a per-window runtime limit without changing `WindowOptions`.
- A shared cell connects the window to the existing frame-request closure.
  The longest applicable interval wins over the runtime, inactive, low-power and
  thermal limits. `None` preserves native cadence and existing power policy.
- The existing `FramePacer` gates ordinary drawing and VRR maintenance presents.
  Required OS presentation remains exempt. Deferred dirty state, forced rendering
  and next-frame callbacks remain pending; changing the limit re-arms the native
  frame source. No timer or periodic idle work is added.
- Profiler output identifies runtime-limit skips separately from thermal skips.
- Regression tests cover runtime validation, pending demand, native cadence,
  multiple source refresh rates, jitter, stalls, power policy and required presents.
- `FramePacer` and interval composition reside in a private `window/frame_pacing`
  module so deterministic tests can include the actual production source. The
  existing pacing algorithm and tolerance are unchanged.
- The test-support utilities expose the existing `TestAppContext::test_window`
  lookup, add `use_platform_frame_requests` to exercise production callbacks, and
  count platform draw submissions with `TestWindow::presentation_count`.
- `Window::set_frame_diagnostics_view(Option<EntityId>)` enables a lightweight
  per-window submission counter; `frame_diagnostics_count()` returns it, or
  `None` while disabled. Setting the same view does not reset it; changing the
  view or disabling it drops previous accounting. Enabling never requests a
  frame, starts a timer, or enables the full profiler.
- The diagnostic counter excludes draws caused only by the selected HUD view.
  Coalesced application, forced-refresh, mandatory OS and high-rate-input demand
  still counts. Application attribution survives intervening HUD/clean draws
  until a presentation. Counting happens at the platform submission boundary,
  not during synchronous CPU draws or native frame callbacks. Since the
  platform draw API returns no result, this is neither a GPU-success count nor
  physical display FPS. The private `window/frame_diagnostics` module holds only
  a view ID, a counter and pending attribution, without histories or clocks.

Run the focused patched-core tests from the workspace root (selecting `explorer`
enables its existing `test-support` dependency feature):

```text
cargo test -p gpui-fast -p explorer --test frame_rate_limit
```

The published upstream crate's full `--lib` test suite cannot compile standalone:
it references an unpublished JPEG, two fonts outside the crate, and `util_macros`
without a packaged development dependency. The focused target compiles the real
library and exercises its real frame-request closure using test-support, without
removing upstream tests or bundling unrelated upstream asset trees. Passing this
target does not imply the complete upstream test suite passed.

Remove this patch when the selected upstream release offers equivalent runtime
frame-limit and opt-in diagnostic APIs with compatible demand retention, native
cadence, power-policy, required-presentation and diagnostic-exclusion behavior.
Re-run the regression tests against that implementation before removing the
vendor directory and Cargo patch.
