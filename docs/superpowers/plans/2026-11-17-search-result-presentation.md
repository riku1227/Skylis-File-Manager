# Search Result Presentation Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans for inline execution.

**Goal:** 一致文字の背景マーカーと、欠けて見えない配下アイコンを軽量に表示する。
**Architecture:** ArgonとOpus 5.5 mediumへ相談後、アンバー背景のみの強調を採用。既存matchトークンとGPUI StyledTextを使い、元のUnicode文字範囲へ写像する。配下は可視行の種類別アイコンだけ既存ワーカーへ要求する。
**Tech Stack:** Rust / GPUI Fast 0.1.15 / existing semantic tokens.
**Spec:** 本ファイルのDesign decisionsを短い仕様とする。ユーザーは両モデルへの相談後の実装を依頼済み。

## Design decisions

背景マーカー、青文字＋下線、太字を比較し、両モデルとも背景マーカーを推奨。太字／パディングを追加せず既存の選択ウェイトを維持する。親パスと編集中の名前は強調しない。既存match.bgを再利用する。ライトの青い選択文字を継承するとマーカー上で約3.83:1だったため、match.fgのライト値を濃色へ明示化する（親判断）。

大小文字判定は現在の全体to_lowercaseと同じ。伸長（İ）・縮小（ẞ）・文脈的な終端シグマを扱い、同じ元文字に写る範囲は統合。非重複一致を最大128範囲まで強調し、描画する行だけ計算する。queryは描画バッチごとに正規化する。フォントと行寸法は維持し、GPUIの末尾省略記号が一致背景を継承する場合だけ小さな修正を加える。

配下は18px・gap10pxを直下へ揃え、フォーカス状態も共有する。実パスのアイコン取得・サムネイル・一覧全件のアイコン要求は行わず、get_typeのダミー拡張子キーと既存上限付きキャッシュを使う。

## Tasks

- [x] coreのNameMatcherへ元UTF-8範囲の算出を追加。ASCII、日本語、重複、İ、ẞ、シグマ、範囲上限の失敗テスト→実装。
- [x] マーカー描画を一覧／配下／ギャラリーで共有し、名前編集中を除外。配下アイコン・名前位置・フォーカス色を統一。可視行の要求だけであることをテスト。
- [x] GPUIの末尾省略がマーカーを継承する回帰テスト→必要な狭いvendor修正。選択／非選択、両テーマ、狭幅、clearの描画テストを行う。
- [x] 全テスト、token生成整合、release build、性能診断、Opus完成レビュー。結果と限界を資料／Craftへ残し、明示ファイルでコミットする。

## Verification

workspace 368 passed / 3 ignored、GPUI marker2件と既存frame pacing11件成功。Windows release／macOS core cross-check／token整合／fmtが成功。両テーマの実機で一致色、アイコン、選択・非アクティブ、長い名前、clearを確認。Opusのmanifest指摘を修正。詳細と計測の限界は docs/implementation/search-result-presentation.md。
