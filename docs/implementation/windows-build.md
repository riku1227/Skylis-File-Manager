# Windowsのビルド環境構築

Windows x64向けのローカル開発手順です。依存ライブラリのバージョンはリポジトリの`Cargo.lock`を使います。

## 必要なもの

- Git
- rustupで管理するRustのMSVCツールチェーン（`x86_64-pc-windows-msvc`）
- Visual StudioまたはVisual Studio Build Toolsの「C++によるデスクトップ開発」。MSVCのx64/x86ビルドツールとWindows SDKを含めてください。
- 初回の依存取得に使うインターネット接続と、ビルド成果物を保存する空き容量

Rustは[公式のインストール案内](https://www.rust-lang.org/tools/install)、C++ビルドツールは[Visual Studioの配布ページ](https://visualstudio.microsoft.com/downloads/)から用意できます。すでにインストール済みなら、以下の確認から進めてください。

```powershell
rustup show
rustc -Vv
cargo -V
```

`rustup show`のactive toolchainとinstalled targetsに`x86_64-pc-windows-msvc`が含まれることを確認します。通常のPowerShellからCargoを実行できます。`cl.exe`がPowerShellのPATH上になくても、それだけでは環境不足ではありません。

## ビルドとテスト

リポジトリのルートで実行します。

```powershell
cargo build --release --locked -p explorer
cargo test --workspace --locked
cargo fmt --all -- --check
```

初回はGPUIや画像処理ライブラリなどもコンパイルするため、時間がかかります。`--locked`は依存の意図しない更新を防ぎます。通常のビルドで`cargo update`を実行する必要はありません。

成功すると`target/release/skylis.exe`が生成されます。

```powershell
.\target\release\skylis.exe
.\target\release\skylis.exe --theme dark --path "C:\Users"
```

開発中の型チェックには`cargo check --workspace --locked`、デバッグ実行には`cargo run --locked -p explorer`を使えます。

RustアプリのビルドにNode.js、Playwright、Chromeは不要です。HTML試作やデザイントークンの生成・確認スクリプトを扱うときだけ別途用意します。

## 環境不足の切り分け

- `cargo`が見つからない場合：Rustのインストール後にPowerShellを開き直し、`%USERPROFILE%\.cargo\bin`がPATHに含まれているか確認します。
- `link.exe`が見つからない場合：Visual Studio InstallerでMSVCのx64/x86ビルドツールが入っているか確認します。
- `kernel32.lib`などが見つからない場合：Visual Studio InstallerでWindows SDKが入っているか確認します。
- 依存取得に失敗する場合：Cargo出力にある通信先やプロキシ設定を確認して再実行します。`Cargo.lock`は削除しません。

macOSのビルドと描画は、このWindows環境での確認範囲には含みません。

## 構築確認（2026-11-15）

`E:\github_projects\Skylis-File-Manager`で既存のツールを使って確認しました。追加インストールやシステム設定の変更は行っていません。

- Rust / Cargo: 1.100.0、`stable-x86_64-pc-windows-msvc`
- Visual Studio Community 2026: 18.7.3、MSVC 14.51.36231
- Windows SDK: 10.0.22621.0 / 10.0.26100.0のライブラリが存在（実際にリンクに使われた版の特定は未実施）
- `cargo build --release --locked -p explorer`: 成功、初回3分27秒。`target/release/explorer.exe`は14,487,040バイト。
- `cargo test --workspace --locked`: アプリ13件・core9件、計22件成功。
- `cargo fmt --all -- --check`、`git diff --check`: 成功。

この確認はビルドと自動テストまでです。今回生成した実行ファイルのGUI操作は行っていません。
