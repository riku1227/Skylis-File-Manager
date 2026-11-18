# Recursive Name Search Implementation Plan

**Goal:** 現在のフォルダーと配下を名前で検索し、直下の既存操作と軽快さを維持する。
**Spec:** ../specs/2026-11-17-recursive-name-search-design.md
**Workflow:** ユーザー指定Astra設計→Opus設計レビュー→親が実装→Opus完成レビュー。設計レビューの回帰指摘をAstraが反映済み。

## Constraints

UIスレッドに走査・Shell操作・画像デコードを追加しない。探索は固定2レーン、pending最大8・ownerごと1件、結果5000件／8MiB、深さ64、走査10万entry、共通期限30秒。直下フィルターは無制限で既存操作を残す。古いquery/root/tabの結果を拒否する。探索結果のメディア処理を起動しない。長いnative I/Oが即中断できるとは報告しない。

## Tasks

- [x] **perf:** Entryキャッシュ名による一致と入力伸長時の前結果絞り込み。入力以外の更新は全再構築。日付グループを検索時だけ省く。失敗テスト→実装→100k件の実モデル診断。
- [x] **core:** name_searchの型・上限・no-follow/cloud/hidden/volumeポリシー・DFS・FIFOバッチをテスト先行で実装。tempdirと狭い制御seamで境界・cancelを検証。
- [x] **service:** 固定local/remoteレーン、owner置換、query/root世代、最大8pending/subscription、full-queue cancellation、共通期限を実装。blocked仕事でスレッド数が増えないことをテスト。
- [x] **UI:** 250ms debounce、IME pair、直下／配下の仮想行、独立した配下選択、open/reveal/copy-path、stop/retry/partialと小さな共有drain予算。直下操作維持・配下操作ガード・tab/navigation lifecycleを検証。
- [x] **remote:** 分類後のlocal routingと明示開始、queryごとの認可、未対応・クラウドskipを表示。classificationブロックは既知localレーンを塞がない。
- [x] **verify:** 全テスト・fmt・release build・再現可能な性能診断・Opus完成レビュー。未確認のネイティブframe/IME/macOSを明記し、指摘を修正。
- [x] **finish:** 利用仕様と実測・制約を記録し、独立して使用可能なConventional Commitに分割する。

## Design review rulings

直下の読み取り専用化と件数上限は撤回。byte/path上限は平均パス長では保証できないため維持。固定ワーカー数だけでは未drainビュー数を制限できないため、小さな全窓共有drain quotaを維持する。フレームドロップがないことをユニットテストだけから断定しない。

## Final validation

361 passed / 3 ignored。Windows release（明示MSVCターゲット）、macOS coreのcross-check、fmtとdiff checkが成功。直下10万件の実モデル診断は統合後の最大3.923ms（描画除外）。Opusの完成レビューと修正後再確認で重要指摘なし。ネイティブ検索中のframe時間、OSドロップ、cloud/SMB実物、macOS GUIは未確認として実装資料へ記載。
