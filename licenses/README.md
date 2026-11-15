# Third-party source

`apps/explorer/src/search.rs` adapts the text input example shipped with
`gpui-pre 0.3.12` (`examples/input.rs`), published under Apache-2.0.
The bundled license is `GPUI-APACHE-2.0.txt`. The adaptation adds the Explorer
theme, search behavior and UI integration. GPUI Fast dependencies retain their
own upstream license notices; Cargo.lock records exact dependencies.

`vendor/gpui-fast` contains the published `gpui-fast 0.1.15` core sources,
licensed under Apache-2.0. Its complete license is preserved at
`vendor/gpui-fast/LICENSE`. Skylis adds a per-window runtime presentation limit;
origin, local changes, validation commands and the removal condition are recorded
in `vendor/gpui-fast/SKYLIS-PATCH.md`. Platform crates remain registry dependencies.
