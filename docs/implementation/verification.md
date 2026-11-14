# Native UI verification — 2026-11-15

## Result
Windows / GPUI Fast 0.1.15で採用UIをネイティブ実装。タブとウィンドウ操作ボタンを最上段32pxへ統合し、別のOSタイトルバーを撤去した。描画にはWebViewや背景ガラス効果を使わない。

## Automated checks
- `cargo check --workspace`: success, warningsなし。
- `cargo test --workspace`: 13 passed（アプリ7、core6）。タブ閉鎖時のインデックス、UTF16/日本語/絵文字の境界、長文入力の横スクロール、Windowsアイコンの取得とalpha、実ドライブ容量等。
- `cargo build --release -p explorer`: success。`target/release/explorer.exe` は約14 MB。
- `cargo fmt --all -- --check`: success。
- `node docs/design/explorer-design-system/tools/build-rust-tokens.cjs --check`: success。

## Windows GUI checks
Computer Useでビルド済みexeを操作して確認した。
- 1280×800相当で統合タイトルバー、タブ、ツールバー、一覧、サイドバー、詳細、全幅情報バーを表示。
- ライト/ダークの切替とWindowsファイルアイコンの表示。
- 最小化（最小化状態の取得エラーで確認）と復帰、最大化と復元、閉じる（ウィンドウ/プロセス消滅）。
- タブ横の空白からドラッグ移動、左辺と上辺をドラッグして1100×700相当に縮小。狭幅では種類列を省略し、主要操作と詳細を維持。
- Ctrl+Fで検索にフォーカスし「企画」と入力。9件→1件へ絞り込み。
- Ctrl+Tで新しいタブ（9件）へ。Ctrl+Shift+Tabで元の「企画」検索（1件）が復元。
- タブ内×で対象タブだけを閉じる。
- フォルダーのダブルクリック、空状態、Alt+左で戻り選択が復元。
- 詳細の長い場所は1行省略に修正。実ドライブ容量バーの取得・表示を確認。

## Limits / next work
- 本人の追加レビューで、タブ段を32pxから38pxへ拡張し、タブと追加ボタンの上側に6pxの余裕を追加。ウィンドウ操作ボタンは上端に残し、一覧の行高等は変更しない。HTML時点の32px仕様より、このネイティブ側の調整を優先する。
- HTMLとピクセル単位の一致ではない。OS実アイコンと文字描画の差、初期選択なしの状態などがある。
- 大きいアイコンは標準shell iconを拡大している。高解像度アイコン/サムネイル取得は未対応。
- 複数選択・グリッド表示・列幅変更・右クリックメニュー・ファイル変更操作・設定保存は今後。
- 日本語文字列入力とUTF16処理は確認済みだが、日本語IMEの変換候補の全操作は実機で未検証。
- Snapレイアウトのホバーポップアップは未検証。最大化ボタンはWindowsのHTMAXBUTTON経路を使用する。
- macOSの実機ビルド/描画/OSアイコン取得、スクリーンリーダー、ハイコントラスト、10,000件のフレームタイムは未検証。性能向上の数値は主張しない。
- テーマは採用済みトークンに合わせたため、デザインリファレンスに記録した一部コントラスト未達は残る。
