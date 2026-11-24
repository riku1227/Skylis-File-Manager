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

## リリースに同梱するライセンス

PowerShell 7 でリポジトリのルートから実行します。

```powershell
cargo +stable install --locked cargo-about --version 0.9.3 --features cli
./scripts/generate-third-party-licenses.ps1
```

Windows MSVC と macOS の x64／ARM64 を対象に、Cargo.lock で固定した依存グラフから `target/dist/THIRD_PARTY_LICENSES.html` を生成します。リリースパッケージにはこのファイルを実行ファイルまたは `Skylis.app` と一緒に同梱してください。生成物は Git に追加しません。

ローカルでパッチを適用した `gpui-fast` と `sevenz-rust2` も収録します。後者のライセンスと変更内容は `vendor/sevenz-rust2/LICENSE` と `vendor/sevenz-rust2/SKYLIS-PATCH.md` にあります。

対象グラフの MPL-2.0 は、ファイル単位のコピーレフトです。CI では `option-ext` 0.2.0（macOS のフォント依存）、`cbindgen` 0.28.0（Apple 向けビルド時依存）、`dwrote` 0.11.5（複数ターゲットを合わせた監査グラフに現れるフォント依存）のバージョンを指定した例外として明記しています。配布する実行ファイルに MPL-2.0 のコードを含む場合は、対応するソースの入手先が必要です。同梱する通知には各 crate のバージョンごとのソースへのリンクが入ります。リリース時にはリンク先から該当ソースを入手できることを確認してください。
