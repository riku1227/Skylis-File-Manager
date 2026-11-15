# 開発者向けパフォーマンス表示

2026-11-15実装。設定（Ctrl+,）の「開発者向け → パフォーマンス表示」で切替。設定を開いている間は8キーでも切替可能。既定はオフで、全ウィンドウに共有・保存され、新規ウィンドウにも反映する。

## 表示と計測の意味

ファイルに重ならない下端の24pxの行に表示する。

- **FPS（描画）**: このウィンドウの描画送信回数を実際の経過時間で割ったもの。CPU draw、native callback、画面Hz、GPU完了、物理的な画面表示回数ではない。HUDだけの更新を除く。実作業やOS必須描画と同時になった送信は一度だけ数える。
- **メモリ**: Skylisプロセス全体の常駐メモリをMiBで表示。Windowsは共有ページも含むWorkingSetSize、macOSはresident_size。private bytesやGPUメモリではない。取得失敗・未対応は「取得不可」、最初のFPS計測前は「—」。
- **画面**: ウィンドウの画面情報から取得したおおよそのHz。実FPSとは別。

表示中・可視ウィンドウのみ1秒間隔でサンプルを採る。メモリはアプリ全体のキャッシュで共有し、失敗時も含めて最短1秒の再照会間隔を守る。文字列が変わったときだけ表示用の子entityをnotifyする。静止中はFPSが0へ戻り、HUDの更新で偽のFPSを生まない。

オフ・非表示・ウィンドウ破棄でタイマーと計測を停止。復帰時は新しい時刻・カウンターを基準にし、停止中の時間を引き継がない。常時アニメーションや全体profiler機能を有効にしない。常駐量は軽量なOS APIで読み、ファイル走査・外部コマンド・WMI・画像decodeは行わない。

## 検証

- `cargo test --workspace --locked -j 4`: explorer140件、core26件成功。既存ignored各1件。
- `cargo test -p gpui-fast -p explorer --test frame_rate_limit`: 11件成功。実際のproduction frame closureでHUDのみの更新、実作業との合流、同期draw、設定リセット、ウィンドウ別計測を検証。既存の60fps上限8件も継続成功。
- 合計177件成功。集計は実経過時間・無操作・ゼロ時間・カウンターリセット、メモリキャッシュは複数読取と失敗からの回復を検証。Windows実プロセスの正の常駐量と無効ハンドルのNoneも確認。
- `cargo fmt --all -- --check`と変更ファイルの`git diff --check`成功。
- `cargo build --release --locked -p explorer -j 4 --target-dir E:/github_projects/Skylis-high-refresh/target`成功。実行中の旧exeを上書きしないため既存の検証targetを再利用。
- 独立した読み取りレビューで、弱参照タスク、キャンセル、親を再通知しない構造、グローバル取得上限と計測の意味を確認。未解決の重大な指摘なし。

最終releaseのWindows実画面で、設定末尾へのスクロール、クリック/8キーによる切替、表示の有無、ホイール操作中のFPS更新、静止時0、メモリの正値、約160Hzの画面表示、ダーク/ライト両テーマでの収まりを確認。隔離APPDATAの保存内容がperformance_overlay=trueになることも確認した。利用者の操作を検出したため検証画面は閉じずに残した。実画面での再起動・最小化からの復帰は未実施で、設定往復と非表示復帰は自動テストによる確認。macOSはソース実装のみで実機未検証。

検証・試用バイナリ: `target/release/explorer-diagnostics.exe`

SHA-256: `315E69BEE45DAC207FA0735254A1CA9506EFC3A0AE5131F5FFD4F114C76DE858`

テスト・ビルドログはGit対象外の`target/diagnostics-work/`、OSアダプターの独立した検証は`target/diagnostics-memory-check/`。検証ウィンドウのAPPDATAは`target/diagnostics-work/appdata/`で、本人の通常設定とは独立している。元からあったREADME変更・Windowsビルド文書は変更/コミットしない。
