# CI Foundation Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan in the current session. Review with user-requested Opus 5.5 medium.

**Goal:** 改善項目の現在地を明示し、Windowsの整形・テスト・リリースビルドを自動検証する。

**Architecture:** 単一Windowsジョブに既存Cargoコマンドを載せる。実装済み・未実装・実機確認待ちを資料で区別する。

**Tech Stack:** GitHub Actions、PowerShell、Rust stable、Markdown。

**Spec:** ../specs/2026-11-17-ci-foundation-design.md

## Constraints and review focus

既存の軽量性、依存、テストのignore条件を維持。fork PRの読み取り権限、古い実行のキャンセル、Windows MSVCのビルド、macOS未検証の表示をレビューする。GUI検証を自動テストの成功から推定しない。

## Task: Windows CIと資料の更新

- [x] 現行設定とビルド要件を調べ、変更前のworkspace testを開始する。
- [x] `.github/workflows/ci.yml`にPR/main push/手動トリガー、読み取り権限、キャンセル、Windows検証を追加する。
- [x] `docs/implementation/daily-driver-roadmap.md`と`research/ui-ux-review/findings.md`に状態・着手順を反映する。
- [x] `docs/implementation/continuous-integration.md`とREADMEの入口を追加する。
- [x] workflowをactionlintで確認し、fmt、workspace test、release build、diffチェックを実行する。
- [x] Opus 5.5 mediumのレビュー指摘を反映する。
- [x] 明示ファイルをConventional Commitでコミットする。

## Execution record

- ユーザーは推奨順での実装と指定モデルによるレビューを承認済み。今回の区切りを追加承認待ちにしない。
- クリーンな通常checkoutから専用ブランチで進める。アプリ本体の変更を含まないため、追加worktreeは作らない。
- Opusはworkflow本体に重大な問題なしと判定。検証結果・チェック欄の更新指摘を反映した。キャッシュとmainのキャンセル条件は初回運用の計測後に再評価する。
- ローカル検証は306件成功・2件ignore、fmt/actionlint/release build成功。GitHubホストでの初回実行は未確認。
