# 継続的インテグレーション

`.github/workflows/ci.yml`の`Windows build and tests`で、以下を順に実行する。

1. `cargo +stable fmt --all -- --check`
2. `cargo +stable test --workspace --locked`
3. `cargo +stable build --release --locked -p explorer`

PR、mainへのpush、GitHub Actions画面からの手動実行が対象。Windows 2025のホストランナーとRust stable（rustfmt込み）を使う。依存はCargo.lockで固定し、toolchainはstableの更新に追従する。初期段階ではキャッシュ・ビルド成果物のアップロード・配布を行わない。

トークンはcontentsの読み取りだけとし、checkoutの資格情報は保持しない。同一PR／ブランチへの新しい実行で古い実行をキャンセルし、1ジョブは60分で停止する。ワークフロー追加だけではブランチ保護の必須チェックにはならない。

## 検証の境界

- 通常の自動テストを実行する。デスクトップのShell拡張と本人のWSLレジストリに依存する既存の2件はignoreのまま維持する。
- GUI実機操作、Windows連携の登録・復元、実際の表示速度、macOSのコンパイル・テストはこのCIの対象外。
- macOS CIはXcode／Metal環境の準備とともに後続で扱う。署名・公証・インストーラー・自動更新も別項目。
- この変更のGitHubホスト上での初回実行は未確認。ローカルでの同等コマンドの結果は以下に記録する。

## 今回の検証

2026-11-17、Windowsのローカル環境（Rust stable 1.100.0）で確認。

- `cargo +stable fmt --all -- --check`：成功。
- `cargo +stable test --workspace --locked`：アプリ257件・core49件、計306件成功、失敗0件、既存2件ignore。doc-testは0件。
- `cargo +stable build --release --locked -p explorer`：成功。既存の未使用import（tabs.rsのsize）とWindowsで未使用のbundle_for_executableに関する警告2件あり。
- `actionlint 1.7.12 .github/workflows/ci.yml`：成功。検証ツールは公式配布のSHA-256照合後に使用し、target以下だけに保存した。
- `git diff --check`と追加・更新したMarkdownのローカルリンク確認：成功。
- Opus 5.5・mediumの静的レビュー：workflow本体に重大な指摘なし。検証記録と計画のチェック欄を更新し、残作業の重複記述も整理した。

GitHubホスト上の初回実行と実行時間は未確認。キャッシュ未使用でdebug/releaseを両方ビルドするため、初回の所要時間を見てタイムアウトやキャッシュ導入を判断する。同一対象の古いmain実行もキャンセルされるため、全コミットの検証履歴を残す必要が出た場合はキャンセル条件をPRに限定する。
