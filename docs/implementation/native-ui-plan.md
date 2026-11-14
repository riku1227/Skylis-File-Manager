# Prism Canopy Native UI Implementation Plan

**Goal:** 採用済みのコンパクトUIをWindows上のネイティブGPUIで再現し、実フォルダーで操作する。
**Architecture:** explorer-coreの非同期列挙・ソート・履歴を維持。UIは共通GPUI APIと意味トークンで構成し、OSアイコンを別モジュールへ隔離する。
**Spec:** ../design/explorer-design-system/DESIGN.md と ../design/explorer-directions/argon/concept-1.html

## Scope
- [x] 公式GPUI Fastの依存と互換性を確認し、固定リビジョンへ移行。
- [x] 意味トークンの色、32pxタブ/44pxツールバー/28px行/6px間隔/28px情報バー、サイドバーと詳細ペインを再現。
- [x] タブごとの場所・履歴・検索・選択、テーマ切替、詳細ペイン開閉を操作可能にする。
- [x] Windowsのファイルアイコンは非同期取得し、元の色のまま表示。
- [x] 追加の本人指定: Windowsエクスプローラーのように、最上段のタブと最小化・最大化・閉じるを一体化。空白部分のドラッグ移動とサイズ変更を維持。
- [x] cargo check/test/release build と実画面確認。ライト/ダーク、フォルダー移動、タブ、検索、空状態を確認。エラー表示経路はコード確認、実画面未検証。

## Constraints
背景のガラス/ぼかし/グラデーションを戻さない。ダーク背景は無彩色。大量一覧の仮想化を維持する。WebViewは使わない。削除/移動等の破壊的ファイル操作は今回追加しない。未実装の機能を動くように見せない。

## Review focus
バックグラウンド列挙中のタブ切替、閉じたタブへの結果配送、検索中の日本語入力、狭幅での列/検索欄、選択のアクティブ/非アクティブ表示、アイコン取得のUIスレッド非ブロック。

## Baseline
Git管理外のワークスペース。既存Rustアプリをpoc-source-backupへ退避。HTMLの正本は変更しない。
