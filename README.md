# Skylis File Manager

Windows / macOS向けの、軽快でコンパクトなファイルマネージャーを目指しています。RustとGPUI Fastを使い、WebViewなしでネイティブUIを描画します。

現在はWindows向けの初期実装です。採用したPrism Canopy Compactのデザインをもとに、タブ・ファイル一覧・検索・OSアイコン表示などを実装しています。macOSは今後の検証対象です。

正式リポジトリ: https://github.com/riku1227/Skylis-File-Manager

## 起動

初めてビルドする場合は、[Windowsのビルド環境構築](docs/implementation/windows-build.md)を参照してください。RustのMSVCツールチェーン、Visual StudioのC++ビルドツール、Windows SDKが必要です。

```powershell
cargo run --release -p explorer -- --theme light --path "C:\Users"
cargo run --release -p explorer -- --theme dark --path "C:\Users"
```

ビルド後のアプリは `target/release/explorer.exe`。内部のcrate名・実行ファイル名は現時点では `explorer` です。`--theme` は `light` / `dark` / `system`、省略時はOS設定。`--path` を省略すると、ドライブの空き容量と主要フォルダーを並べたホーム画面を開きます。

## 操作

- Ctrl+T / Ctrl+W: ホーム画面のタブを追加・閉じる（最後の1枚は残す）
- Ctrl+Tab / Ctrl+Shift+Tab: タブ切替
- Ctrl+F: 現在のフォルダー内を名前で絞り込み、Esc: 解除
- 現在のパンくず／アドレス欄の空白をクリック、Ctrl+L / Alt+D: パスを直接入力（macOSはCmd+Lも対応）
- パス入力中のEnter: フォルダーへ移動、Esc／欄外クリック: キャンセル。無効なパスは入力を保持してエラーを表示
- Alt+左 / 右 / 上: 戻る・進む・親へ
- Enter / ダブルクリック: フォルダーを開く、ファイルは既定アプリへ
- F5: 再読込、Ctrl+H: 隠しファイル
- Ctrl+Shift+L: ライト → ダーク → システム

右の詳細ペインと下端の密度・隠しファイル設定も操作できます。検索は再帰検索ではありません。ホームではドライブ・主要フォルダーを絞り込みます。ホームのカードはダブルクリック、または選択後のEnterで開けます。ホームもタブごとの戻る・進む履歴に含まれます。

直接入力は絶対パス、相対パス、UNCパス、`~`、引用符で囲まれたパスに対応します。Windowsの `D:` は `D:\` として扱います。コマンド実行やURLの起動は行いません。

Windowsではホームに「ゴミ箱」カードがあります。ダブルクリックまたは選択後のEnterでWindows標準のゴミ箱を開きます。復元や空にする操作は、開いたWindows側の画面で行います。

## 設計と開発

- [デザインリファレンス](docs/design/explorer-design-system/index.html)
- [ネイティブUI実装計画](docs/implementation/native-ui-plan.md)
- [GPUI Fast移行記録](docs/implementation/gpui-fast.md)
- [実画面・テストの確認結果と未実装項目](docs/implementation/verification.md)
- [取り込んだソースのライセンス](licenses/README.md)

ファイル列挙・ソート・履歴は `explorer-core`、表示と入力は `apps/explorer`。Windowsのアイコンはワーカースレッドで取得し、色を変えずに表示します。GPUI Fastの依存入口はルートCargo.tomlに集約しています。

```powershell
cargo test --workspace --locked
cargo check --workspace --locked
cargo build --release --locked -p explorer
```

HTML試作の検証スクリプトを実行する場合は、別途Node.js・Playwright・Google Chromeが必要です。Playwrightは通常のNode.jsモジュール解決を使い、既存のインストールを指定する場合は `PLAYWRIGHT_PATH` を設定できます。これらはRustアプリのビルドには不要です。

## 今回の範囲

UI再現を優先した開発段階です。ファイルの削除・移動・名前変更、サムネイルプレビュー、設定の永続化は未実装。macOS実機のビルド・描画・OSアイコン取得は未検証です。大きな詳細アイコンはWindows標準アイコンの拡大で、高解像度取得は今後の改善点です。

## ライセンス

[Apache License 2.0](LICENSE)。取り込んだ第三者ソースの出典とライセンスは [licenses/README.md](licenses/README.md) を参照してください。
