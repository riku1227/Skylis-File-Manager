# Skylis 開発者ガイド

[English](README_DEV.md) · **日本語**

Skylis のビルド・テスト・リポジトリ構成・開発ルールをまとめたドキュメントです。アプリの紹介と使い方は [README.ja.md](README.ja.md) を参照してください。

- [技術スタック](#技術スタック)
- [リポジトリ構成](#リポジトリ構成)
- [Windows での環境構築](#windows-での環境構築)
- [macOS での環境構築](#macos-での環境構築)
- [ビルドと実行](#ビルドと実行)
- [テストと静的チェック](#テストと静的チェック)
- [CI と依存ポリシー](#ci-と依存ポリシー)
- [リリース成果物](#リリース成果物)
- [開発ルール](#開発ルール)
- [設計資料](#設計資料)

## 技術スタック

| 項目 | 内容 |
| --- | --- |
| 言語 | Rust（edition 2024、stable） |
| UI | [GPUI Fast](https://crates.io/crates/gpui-fast) 0.1.15（WebView なしの GPU 描画） |
| 対象 OS | Windows 10/11 x64（主開発環境）、macOS Apple Silicon（ローカル `.app`） |
| OS 連携 | Windows: `windows` / `windows-sys` crate による Shell API。macOS: Objective-C アダプター（`shell_icons_macos.m`） |
| 圧縮 | `zip` 9.6.0、`sevenz-rust2` 0.25.0（ローカルパッチ適用） |

GPUI の依存窓口はルートの `Cargo.toml` に集約しています。`gpui-fast` と `sevenz-rust2` は `vendor/` にパッチ済みソースを置き、`[patch.crates-io]` で差し替えています。変更内容と削除条件は各ディレクトリの `SKYLIS-PATCH.md` に記録しています。

## リポジトリ構成

```text
.
├── apps/explorer/          # GUI アプリ本体（パッケージ名 explorer、実行ファイル名 skylis）
│   ├── assets/             # アプリアイコン（branding/）と UI アイコン（icons/）
│   ├── src/                # 画面・入力・OS 連携・ワーカー
│   ├── app.rc / build.rs   # Windows リソース（アイコン・バージョン情報）
├── crates/explorer-core/   # UI 非依存のロジック（列挙・ソート・履歴・検索・圧縮）
├── vendor/                 # パッチ適用済みの依存ソース（gpui-fast, sevenz-rust2）
├── packaging/              # Windows インストーラー、macOS Info.plist、ライセンス通知テンプレート
├── scripts/                # Windows・macOS 向けパッケージ作成、アイコン生成、ライセンス生成
├── docs/                   # 設計・実装記録・検証結果
├── research/               # 調査メモ
└── licenses/               # 取り込んだ第三者ソースのライセンス
```

責任の分け方の原則:

- **`explorer-core`** はファイル列挙・ソート・履歴・名前検索・圧縮など、GPUI に依存しないロジックを持ちます。単体テストはここに集めます。
- **`apps/explorer`** は表示・入力・ワーカーの調停・OS アダプターを持ちます。OS 固有の処理は `native_shell/`、`shell_icons*`、`folder_integration.rs` などの小さなアダプターに閉じ込めます。

## Windows での環境構築

必要なもの:

- Git
- rustup 管理の Rust stable（MSVC ツールチェーン `x86_64-pc-windows-msvc`）
- Visual Studio 2022 以降、または Build Tools の「C++ によるデスクトップ開発」（MSVC x64/x86 ビルドツールと Windows SDK を含める）

```powershell
# Rust のインストール後に確認
rustup show          # active toolchain が *-pc-windows-msvc であること
rustc -Vv
cargo -V
rustup component add rustfmt
```

`cl.exe` が PowerShell の PATH 上になくても問題ありません。Cargo が MSVC を自動で検出します。

トラブルシューティング:

| 症状 | 確認すること |
| --- | --- |
| `cargo` が見つからない | PowerShell を開き直し、`%USERPROFILE%\.cargo\bin` が PATH にあるか |
| `link.exe` が見つからない | Visual Studio Installer で MSVC x64/x86 ビルドツールが入っているか |
| `kernel32.lib` などが見つからない | Visual Studio Installer で Windows SDK が入っているか |
| 依存の取得に失敗する | プロキシ設定と通信先。`Cargo.lock` は削除しない |

詳細と確認記録は [docs/implementation/windows-build.md](docs/implementation/windows-build.md) にあります。

## macOS での環境構築

1. `/Applications/Xcode.app` にフル版の Xcode をインストールし、初回起動のセットアップを完了します。
2. Rust と Metal ツールチェーンを用意します。

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh
sh /tmp/rustup-init.sh -y --profile minimal --default-toolchain stable
. "$HOME/.cargo/env"
rustup component add rustfmt
xcodebuild -downloadComponent MetalToolchain
```

ビルドスクリプトは `DEVELOPER_DIR`（既定は上記の Xcode）を使い、グローバルな `xcode-select` は変更しません。

署名には有効な Apple Development / Developer ID Application 証明書が 1 つだけあれば自動で選びます。複数ある場合は `SKYLIS_SIGNING_IDENTITY` で指定し、ない場合は警告を出してアドホック署名にします。権限（フルディスクアクセス）の扱いを含む詳細は [docs/implementation/macos-build.md](docs/implementation/macos-build.md) を参照してください。

## ビルドと実行

### Windows

```powershell
# デバッグ実行
cargo run --locked -p explorer

# 引数付きで起動（cargo run では -- の後ろがアプリへの引数）
cargo run --release --locked -p explorer -- --theme dark --path "C:\Users"

# リリースビルド → target/release/skylis.exe
cargo build --release --locked -p explorer
```

初回は GPUI や画像処理ライブラリのコンパイルに数分かかります。MSVC の C ランタイムは静的リンク（`.cargo/config.toml`）しているため、配布先に Visual C++ 再頒布可能パッケージは不要です。

### macOS

```sh
bash scripts/build-macos.sh debug --launch   # ビルドして起動
bash scripts/build-macos.sh release
# 出力: target/aarch64-apple-darwin/{debug,release}/Skylis.app
```

実行中のアプリを上書きビルドしないでください。終了 → ビルド → 再起動の順で行います。

### T3 Code のスクリプト

[`t3.json`](t3.json) に Windows / macOS のビルドスクリプトを登録しています（`release(Windows x64)`、`launch(macOS debug .app)` など）。

### コマンドライン引数

| 引数 | 動作 |
| --- | --- |
| `--path <PATH>` / `<PATH>` | 指定フォルダーを開く |
| `--home` | ホーム画面で開く（`--path` とは併用不可） |
| `--select <PATH>` | 親フォルダーを開き、対象を選択する |
| `--theme light\|dark\|system` | テーマを指定（省略時は保存済み設定または OS 設定） |
| `--restore-integration` | Windows 連携を画面なしで復元し、結果を標準エラーへ出力して終了（単独で指定） |
| `--` | 以降を位置引数として扱う（`-` で始まるパス用） |

`--switch-explorer`、`--shell-folder`、`--handoff-token`、`--shell-command-server` は Windows 連携からの内部呼び出し用です。

### 設定・ログの保存場所

| 種類 | Windows | macOS |
| --- | --- | --- |
| 設定・セッション | `%APPDATA%\Skylis\` | `~/Library/Application Support/Skylis/` |
| クラッシュログ | `%LOCALAPPDATA%\Skylis\logs\` | `~/Library/Logs/Skylis/` |

開発中に普段使いの設定を汚したくない場合は、`APPDATA` を一時ディレクトリへ向けて起動できます。

```powershell
$env:APPDATA = "$PWD\target\dev-appdata"; cargo run --locked -p explorer
```

## テストと静的チェック

```powershell
cargo test --workspace --locked        # 全テスト
cargo test --locked -p explorer-core   # コアのみ
cargo check --workspace --locked       # 型チェック
cargo fmt --all -- --check             # フォーマット
git diff --check                       # 空白エラー
```

インストール済みの Shell 拡張を読み込むテストなど、デスクトップ環境に依存するものは `#[ignore]` にしています。必要なときに `cargo test --locked -p explorer -- --ignored` で明示的に実行してください。

`docs/design/` の HTML 試作の検証スクリプトは Node.js・Playwright・Google Chrome を使います（`PLAYWRIGHT_PATH` で既存のインストールを指定可能）。Rust アプリのビルドには不要です。

## CI と依存ポリシー

[`.github/workflows/ci.yml`](.github/workflows/ci.yml) が `main` への push と Pull Request で実行されます。

| ジョブ | 内容 |
| --- | --- |
| Windows build and tests | `cargo fmt --check` → `cargo test --workspace --locked` → リリースビルド |
| Dependency policy | `cargo deny check licenses sources bans` と、ローカル path 依存が許可リストに限られることの確認 |

依存を追加・更新したときは、手元でも確認してください。

```powershell
cargo +stable install --locked cargo-deny --version 0.22.0
cargo +stable deny --locked check licenses sources bans
```

許可するライセンスと例外は [`deny.toml`](deny.toml) に記載しています。新しい path 依存を追加する場合は CI の許可リストも更新が必要です。詳細は [docs/implementation/continuous-integration.md](docs/implementation/continuous-integration.md)。

## リリース成果物

- Windows: 通常のリリースビルドは `target/release/skylis.exe`、インストーラーのビルドは `target/installer/Skylis-<version>-x64-setup.exe`
- macOS: `Skylis.app`（バンドル ID `dev.skylis.file-manager`、ローカル署名。公証は未対応）
- 第三者ライセンス通知: PowerShell 7 で以下を実行し、生成された `target/dist/THIRD_PARTY_LICENSES.html` を実行ファイルと一緒に同梱します。

```powershell
cargo +stable install --locked cargo-about --version 0.9.3 --features cli
./scripts/generate-third-party-licenses.ps1
```

### Windows インストーラーのビルド

前述の Windows 向け MSVC ビルド環境に加え、PowerShell 7 と Inno Setup 6.3 以降が必要です。リポジトリのルートで次を実行します。

```powershell
winget install JRSoftware.InnoSetup
rustup target add x86_64-pc-windows-msvc
cargo +stable install --locked cargo-about --version 0.9.3 --features cli
./scripts/build-windows-installer.ps1
```

スクリプトは x64 のリリースをビルドし、第三者ライセンス通知を再生成して、ワークスペースのバージョンを使った `target/installer/Skylis-<version>-x64-setup.exe` を出力します。アップグレード時は既存のインストール先を再利用します。ユーザー単位のアンインストーラーは、ファイルの削除前に `skylis.exe --restore-integration` を実行し、ユーザー設定を残します。

アプリアイコンを更新する場合は `apps/explorer/assets/branding/skylis.svg`（小サイズは `skylis-small.svg`）を編集し、`node scripts/generate-app-icons.cjs` で PNG / ICO / ICNS を再生成します（要 `sharp`）。通常のビルドではチェックイン済みの成果物を使います。

## 開発ルール

[AGENTS.md](AGENTS.md) が正本です。要点:

- **軽さを守る**: UI スレッドでファイルシステム走査、Shell 拡張呼び出し、画像デコード、再帰的なサイズ計算をしない。
- **上限を設ける**: ワーカーの並列数・待機キュー・画像メモリーに上限を持たせる。ナビゲーションで不要になった要求はキャンセルし、表示中の有効な画像をローディング表示で置き換えない。
- **アダプターに閉じ込める**: OS 連携は小さく明示的なアダプターにまとめ、未対応の機能は正直に未対応と表示する。
- **コミット**: [Conventional Commits](https://www.conventionalcommits.org/)（`feat:` / `fix:` / `perf:` / `docs:` など）。独立して使える機能ごとにコミットし、本文に意図・ユーザーから見た挙動・制約・検証内容を書く。
- 一時的な説明用 HTML やローカル検証の成果物はコミットしない。

## 設計資料

| 資料 | 内容 |
| --- | --- |
| [デザインリファレンス](docs/design/explorer-design-system/index.html) | Prism Canopy Compact のデザインシステム |
| [ネイティブ UI 実装計画](docs/implementation/native-ui-plan.md) | 画面構成と実装方針 |
| [GPUI Fast 移行記録](docs/implementation/gpui-fast.md) | GPUI Fast の採用とパッチ |
| [日常利用向けのファイル操作](docs/implementation/daily-use.md) | コピー・移動・圧縮・監視の責任範囲 |
| [ドラッグとドロップ](docs/implementation/drag-drop.md) | D&D の規則と制約 |
| [フォルダー連携とピン留め](docs/implementation/folder-integration-and-pins.md) | Windows 連携の登録と復元 |
| [セッション復元](docs/implementation/session-restore.md) | タブとウィンドウの保存 |
| [キーボード操作](docs/implementation/keyboard-tab-workflows.md) | 閉じたタブの復元・複製 |
| [モーション](docs/implementation/motion.md) | アニメーションと高リフレッシュレート対応 |
| [改善ロードマップ](docs/implementation/daily-driver-roadmap.md) | 今後の着手順 |
| [検証結果と未実装項目](docs/implementation/verification.md) | 実画面・テストの確認結果 |
| [圧縮バックエンド](crates/explorer-core/src/archive/README.md) | 対応形式・上限・安全対策 |
| [第三者ソースのライセンス](licenses/README.md) | 取り込んだソースの出典 |
