# アプリ内蔵ファイル操作基盤・統一アクティビティUI 全体設計

日付: 2026-11-19  
状態: 設計確定（ユーザー承認済み・ライン1実装中）  
相談・レビュー: Gemini Orchestrator / Claude Opus 5.5 High（設計相談・第1回設計レビュー完了）

> [!NOTE]
> 本ドキュメントでは、**ユーザー要件**（「ファイル操作のステータス表示をできる限りアプリ内に内蔵し、進捗確認・中止や操作制御を行えるようにすること」「まずはライン 1 で進め、後から独自方式のライン 2 を追加して設定で切り替えられるようにすること」「下部内蔵表示と別ウィンドウ表示の両方を共通コンポーネントで持ち設定切替＋ポップアウト可能にすること」）と、**エンジニアリング提案値**（キュー上限、保持件数、更新間隔、メモリ上限、フェーズ分割など）を区別して記載する。第8.2節の製品判断事項はユーザー承認により確定済み（推奨デフォルト採用）。

---

## 1. 目的・背景・基本思想（三層アーキテクチャ）

### 1.1 現状と課題

現行の Skylis File Manager では、同じ「時間がかかるファイル操作」でありながら設計思想が2系統に分裂している。

1. **圧縮・解凍（すでに自前制御＋アプリ内表示）**
   - コア実装は [`crates/explorer-core/src/archive/mod.rs`](../../../crates/explorer-core/src/archive/mod.rs) および [`crates/explorer-core/src/archive/paths.rs`](../../../crates/explorer-core/src/archive/paths.rs) にあり、有界上限・`CancelToken` による中断・バイト単位進捗・一時領域（`Staging`）から非上書き公開（`rename_no_replace`）・`Zone.Identifier` 伝播・リパースポイント防御を備える。
   - ジョブ管理は [`apps/explorer/src/archive_jobs.rs`](../../../apps/explorer/src/archive_jobs.rs) が単一ワーカーの有界キュー（待機上限4、履歴16）で動かし、[`apps/explorer/src/folder/archives.rs`](../../../apps/explorer/src/folder/archives.rs) が `render_archive_jobs` でアプリ内表示を行っている。
2. **コピー・移動・削除（OS Shell / AppleScript 丸投げ）**
   - [`apps/explorer/src/file_operations.rs`](../../../apps/explorer/src/file_operations.rs) の Windows 実装は COM の `IFileOperation` を標準 UI 付き（`FOF_SILENT` なし）で呼び出しており、進捗・上書き確認・エラー・UAC ダイアログがすべて Windows 側の別ウィンドウとして表示される。
   - 進捗シンク（[`apps/explorer/src/file_operation_progress/sink.rs`](../../../apps/explorer/src/file_operation_progress/sink.rs)）とメールボックス（[`apps/explorer/src/file_operation_progress.rs`](../../../apps/explorer/src/file_operation_progress.rs)）経由でアプリ内にも進行バーを出しているが、取れる情報は Shell の相対値（`work_done / work_total`）に限られ、[`apps/explorer/src/folder.rs`](../../../apps/explorer/src/folder.rs) の描画には「中止はWindowsの進捗ウィンドウから」と表示されるなど、操作の主導権が OS とアプリに分断されている。
   - また `ACTIVE: AtomicBool` によるプロセス全体1件排他であるため、連続した貼り付けや D&D のキューイング・一時停止ができない。さらに `HWND` 所有ウィンドウと密結合しているため、[`apps/explorer/src/explorer/tabs.rs`](../../../apps/explorer/src/explorer/tabs.rs) では操作中の最後のタブ移動やウィンドウ統合を制限している。
   - macOS 実装は `/usr/bin/osascript` で Finder に依頼する暫定方式であり、進捗・キャンセル・置換指定が効かず、Automation 権限を必要とする。また、macOS の完全削除は現行未対応（[`apps/explorer/src/file_operations.rs`](../../../apps/explorer/src/file_operations.rs)）である。
   - UI 面でも、`folder.rs` において `render_status` の下に `transfer_progress` バナーと `render_archive_jobs` パネルが別系統として縦積みされている。

つまり、ファイル操作の進捗や制御を Skylis 側へ取り込む構想は、**圧縮・解凍で確立済みの「軽量・安全・統一 UI・有界ワーカー」という Skylis 本来の思想へファイル操作全体を揃える**試みである。

### 1.2 採用する基本思想：三層分離アーキテクチャ

Skylis の開発規約（[`AGENTS.md`](../../../AGENTS.md)）とデータ安全性を両立するため、すべてをゼロから自前実装するのではなく、**制御・UI は自前、低レベルデータ運搬は OS プリミティブ、特殊な領域は正直に委譲**する三層分離を採用する。

```text
+-----------------------------------------------------------------------+
| 第1層: オーケストレーション & UI 層（Skylis 完全自前）                  |
|   - 統一 Job モデル・有界キュー・パスリース・デバイス別スケジューリング |
|   - 単一パス先読み走査・進捗計測（バイト/項目/速度/ETA）・一時停止/中止 |
|   - アプリ内対話解決（競合・ロック/エラー再試行・完全削除確認）        |
|   - 統一アクティビティ UI（ステータスバー常駐ピル / ポップオーバー）    |
+-----------------------------------+-----------------------------------+
                                    |
          +-------------------------+-------------------------+
          v                                                   v
+-----------------------------------+   +-------------------------------+
| 第2層: データ運搬プリミティブ層   |   | 第3層: 正直な OS 委譲層       |
|   （薄い OS アダプター）          |   |   （フォールバック & 特殊領域）|
|   - Windows: CopyFile2,           |   |   - ゴミ箱移動:               |
|     MoveFileExW,                  |   |     Win サイレント IFileOp    |
|     SetFileInformationByHandle    |   |     macOS trashItemAtURL      |
|   - macOS: copyfile(3),           |   |   - UAC 昇格が必要なパス      |
|     renamex_np(RENAME_EXCL)       |   |   - MTP / 仮想シェル項目      |
|   - ADS・スパース・SMB ODX・      |   |   - ライン1(Phase 0.5)の      |
|     APFS Clone を維持             |   |     サイレント IFileOp コピー |
|   （※ACLはコピーせず保存先親継承）|   |   - UI に委譲状態を正直表示   |
+-----------------------------------+   +-------------------------------+
```

> [!IMPORTANT]
> `CopyFile2` は Explorer のコピーと同様に、ファイルの **ACL（セキュリティ記述子）をコピーせず保存先親フォルダーから継承** する。権限まで複製したい特殊なコピーは本基盤の対象外（標準の OS コピーと同等仕様）とする。

### 1.3 検討したアプローチと 2つの到達ライン

設計上、目的に応じて **2つの明確な到達ライン** が存在する（第6節のロードマップで段階的に進めるため、両者は対立せずライン1を独立した中間完結点にできる）。

| アプローチ | 利点 | 欠点・制約 | 位置づけ |
|---|---|---|---|
| **A. Shell 丸投げ維持（現状）** | Explorer と完全同等の挙動・互換性・`Ctrl+Z` 履歴連携 | 進捗・競合・エラーが常に Windows 別窓になり、アプリ内ステータス統合と根本的に衝突する。macOS の `osascript` 依存も解消しない | **不採用** |
| **B1. `IFileOperation` + `FOF_SILENT`（`FOF_NOCONFIRMATION` なし）＋ Sink 中止 ＋ キュー** | 普段のコピー・移動で Windows 進捗窓を消して Skylis 内に統一表示・中止・待機キューを持てる。**データ損失リスクほぼゼロ**で、UAC・競合・ロック・ADS・OneDrive・`Ctrl+Z` をそのまま維持 | ① 相対値 `work_done/work_total` のみで正確な速度（MB/s）・実バイト数・ETA を出せない。<br>② 衝突・ロック・UAC 発生時は Windows 側ダイアログが出る。<br>③ 一時停止・macOS には使えない | **到達ライン 1（Phase 0.5）として採用**<br>最小コストの現実的到達点 |
| **B2. `IFileOperation` + `FOF_SILENT \| FOF_NOCONFIRMATION` 全面採用** | Windows 窓を完全に隠せる | 競合解決コールバック不在のため「無言上書き」か「無条件リネーム」しかできず、ゴミ箱不可ボリュームで**無言完全削除**が起きる危険がある | **コピー・移動は不採用**<br>（ゴミ箱のみ三層防御付きで条件付き採用） |
| **C. `std::io::copy` / `read`/`write` ループ全自前** | I/O 制御が 100% 完結する | NTFS ADS・スパース・圧縮属性・SMB ODX・APFS クローンを失うか再実装により低速・危険になる | **不採用（禁止）** |
| **D. 三層分離＋自前オーケストレーション** | 統一 UI・キュー・一時停止・競合/エラー解決・正確なバイト進捗・MB/s・ETA を完全内蔵しつつ、データ運搬の高速性・安全性を OS プリミティブに委ねられる。macOS も `osascript` を脱却できる | 競合 UI・エラー再試行 UI・障害回復・安全性テストの実装コストが高い。Explorer `Ctrl+Z` 履歴・`ICopyHook` とは切り離される | **到達ライン 2（Phase 2〜4）として採用** |

> [!TIP]
> **Windows の `IOperationsProgressDialog` 自前実装を採らない理由**: COM の `IFileOperation::SetProgressDialog` に渡す `IOperationsProgressDialog` をアプリ側でゼロから実装する方法は、Microsoft の公式方針上も標準サポート外（`CLSID_ProgressDialog` 前提で非システム実装は `E_NOINTERFACE` 等のリスクがある）であり、モーダルメッセージループやスレッドマーシャリングの保守負荷も高いため採用しない。

---

## 2. 操作種別ごとの責務境界とルーティング方針

### 2.1 操作・ケース別の責務マトリクス

| 操作 / 特殊ケース | 採用方式 | 具体的な実装方針・注意点 |
|---|---|---|
| **圧縮・解凍（ZIP / 7z）** | **自前（実装済み）** | 既存の `explorer_core::archive` を維持し、進捗・履歴・キャンセル窓口を統一 `OperationCenter` へ統合する |
| **コピー（ローカル / SMB / 外部ドライブ）** | **Phase 0.5**: サイレント `IFileOperation`<br>**Phase 2〜**: 自前エンジン ＋ `CopyFile2` / `copyfile(3)` | `CopyFile2`（進捗・キャンセル・ADS/属性維持・SMB オフロード）および `copyfile(3)`（`COPYFILE_ALL \| COPYFILE_CLONE` で APFS クローン活用）。※以下は Windows 実機で要確認：① `COPY_FILE_COPY_SYMLINK` が開発者モードで特権なしに動くか、② スパース/圧縮属性の保持、③ ADS 非対応保存先での `Zone.Identifier` 喪失挙動、④ `COPY_FILE_NO_BUFFERING` と SMB サーバーサイドコピー／キャッシュの相互作用（ベンチマーク後に採用判断） |
| **同一ボリューム内の移動** | **自前 ＋ アトミックリネーム** | 事前にボリューム同一性を推測せず、`MOVEFILE_COPY_ALLOWED` なしの `MoveFileExW`（macOS は `renamex_np` 等）を試し、`ERROR_NOT_SAME_DEVICE`（`EXDEV`）なら異ボリューム移動へ切り替える（`TransferKind::Automatic` の意図判定には引き続き [`volume.rs`](../../../apps/explorer/src/file_operations/volume.rs) を使用）。コピーへ誤フォールバックしない |
| **ボリュームをまたぐ移動** | **自前：コピー → 検証 → ハンドル削除** | `MoveFileWithProgressW` はボリューム跨ぎディレクトリ移動に非対応。コピー先 `FlushFileBuffers` / `F_FULLFSYNC` と転送元同一性・サイズ検証（§4.1）完了後にのみ元を削除し、失敗・スキップ・中止があれば親ディレクトリと未完了元を絶対に残す |
| **ゴミ箱へ移動** | **OS 委譲 ＋ UI は自前** | `$Recycle.Bin` や `.Trash` の内部構造には触らない。Windows は三層防御付きのサイレント `IFileOperation`（§4.8）、macOS は `NSFileManager::trashItemAtURL` |
| **完全削除** | **Windows**: Phase 3〜 自前（ハンドル基準）<br>**macOS**: 現行未対応（要方針確認） | Windows はアプリ内確認のうえ `FILE_FLAG_OPEN_REPARSE_POINT \| FILE_FLAG_BACKUP_SEMANTICS` ハンドルから `FileDispositionInfoEx` で削除（§4.7）。**名前サロゲート（ジャンクション等）内部へは絶対に再帰しない**。macOS 向け完全削除（`unlinkat` 等）を解禁するかは §8.2 と macOS 検証状況に連動 |
| **UAC 昇格が必要なパス** | **OS 委譲（`JobBackend::OsDelegated`）** | `ERROR_ACCESS_DENIED` 等を検知したら、対話シートで「Windows の管理者権限ダイアログで再試行」を選択可能にする（昇格ヘルパー自作は Phase 5 以降で任意検討） |
| **OneDrive / iCloud クラウドプレースホルダー** | **自前可（タグ分類と非ハイドレーション徹底）** | 事前走査では非ハイドレーション属性のみ参照し、`RECALL_ON_OPEN` / `SF_DATALESS` のディレクトリには降りない。実コピー時にダウンロード待ちが見込まれる場合は事前属性から推定して「クラウドからダウンロード中…」を表示する |
| **MTP（スマホ等）・仮想シェルフォルダー・ライブラリ** | **OS 委譲（`JobBackend::OsDelegated`）** | 実パスを持たず `IShellItem` 専用のため自前処理しない。起動時は「Windows のダイアログで進行中」と正直に表示する |
| **シンボリックリンク / ジャンクション（名前サロゲート）** | **自前（転送元・保存先とも No-Follow）** | 辿らないのは **名前サロゲートタグ（`tag & 0x2000_0000`）とシンボリックリンク** であり、OneDrive 等の非サロゲート（`CLOUD_*`）とは厳密に区別する（[`archive/paths.rs`](../../../crates/explorer-core/src/archive/paths.rs) の `is_name_surrogate`・[`name_search/policy.rs`](../../../crates/explorer-core/src/name_search/policy.rs) の `allowed_reparse` と統一）。<br>・`COPY_FILE_COPY_SYMLINK` はシンボリックリンクにのみ効き、**ジャンクションには効かず `CopyFile2` はディレクトリを扱えない**ため、ジャンクションのコピーは既定スキップ＋ユーザー確認とする（再作成時もターゲットを書き換えない）。<br>・シンボリックリンク作成権限不足（`ERROR_PRIVILEGE_NOT_HELD`）時は、無言で実体コピーに化けさせず「実体をコピー / スキップ / Windows に委譲」を提示する |
| **EFS 暗号化ファイル** | **OS プリミティブへ委任（暗黙復号禁止）** | 非暗号化 FS へコピーして失敗した場合、`COPY_FILE_ALLOW_DECRYPTED_DESTINATION` を無断付与せず、確認 UI でユーザーが「復号してコピー」を明示したときだけ許可する |
| **使用中ファイルロック（共有違反）** | **自前 UI で対話解決** | 「再試行 / スキップ / 中止」を提示。Phase 4 以降で Windows Restart Manager（`RmGetList`）により掴んでいるプロセス名を表示する |

### 2.2 バックエンド型とルーティング規則

バックエンド種別は全層で `JobBackend`（旧名称案 `TransferBackend` を廃し統一）として表現する。また設定（`settings_data.rs`）には **`FileTransferEngine`** を設け、ライン 1 とライン 2（および緊急退避用の OS 標準ダイアログ）を安全に切り替えられる構造とする：

- `FileTransferEngine::OsIntegrated`（**ライン 1・既定**）：Windows サイレント `IFileOperation` + Sink 中止 + キュー＋ゴミ箱内蔵（Phase 0.5 / Phase 1 完成時）。
- `FileTransferEngine::OsDialog`：緊急退避・比較用の現行 Windows 標準進捗ダイアログ経路。
- `FileTransferEngine::Builtin`（**将来のライン 2 拡張枠**）：`explorer-core::transfer` による自前オーケストレーション＋OS プリミティブ（`CopyFile2` / `copyfile(3)`）。未実装の段階では設定 UI 上で稼働中と偽装せず、「独自エンジンは今後の段階実装で追加予定」と正直に表示する（`AGENTS.md` 準拠）。
- **切替タイミング**: 設定変更は**新しく開始するジョブから即時適用**され、すでに進行中のジョブは開始時のエンジンのまま安全に完了する。

実行バックエンド `JobBackend` の割り当て：

- `JobBackend::Builtin`: `FileTransferEngine::Builtin` が有効で、かつ対象が通常のファイルシステム絶対パス（ドライブ文字、UNC、マウントポイント、ローカルクラウド同期フォルダー）である場合。
- `JobBackend::OsDelegated { reason }`:
  - 仮想シェルフォルダー、MTP 等
  - ユーザーが「Windows 権限で再試行（UAC）」を選んだ場合
  - 設定が `OsIntegrated`（ライン 1）または `OsDialog` の場合
  - **Phase 2 における `TransferKind::Automatic` バッチ**：[`volume.rs`](../../../apps/explorer/src/file_operations/volume.rs) で項目別にコピー／移動を判定した結果、**移動が 1 件でも含まれるバッチは Phase 3 まで丸ごと `OsDelegated` へ回す**（同一バッチが「自前コピー」と「Shell 移動」に分裂するのを防ぐため）。

**UI スレッド非ブロックとウィンドウ制約の縮小:**
- バックエンド判定・ボリューム判定は必ずワーカー上で実行し、UI スレッドではファイルシステムを `stat` しない。
- [`apps/explorer/src/explorer/tabs.rs`](../../../apps/explorer/src/explorer/tabs.rs) の「操作中は最後のタブを移動・ウィンドウ統合できない」制約は、オーナー `HWND` を持つ `OsDelegated` 実行中のみに適用し、`Builtin` 実行中はタブやウィンドウの組み替えを妨げない。

---

## 3. モジュール境界・統一ジョブモデル・スケジューラー・上限

### 3.1 クレートとモジュール配置

既存の [`crates/explorer-core/Cargo.toml`](../../../crates/explorer-core/Cargo.toml)（UI 非依存、GPUI・Shell COM 持ち込み禁止）と [`apps/explorer/Cargo.toml`](../../../apps/explorer/Cargo.toml) の境界を厳守する。

```text
crates/explorer-core/src/
  fs_safety.rs        [新設] 安全なパス・メタデータ基盤を共通抽出
                        - archive/paths.rs, name_search/policy.rs,
                          file_operations/volume.rs, native_shell::windows_path_wide の
                          重複パス変換・検証を集約
                        - native_path (\\?\, \\?\UNC\), 予約名・文字検証
                        - リパースタグ / 名前サロゲート / クラウドタグ / SF_DATALESS 判定
                        - FileIdentity (FileIdInfo 優先 -> FileIndex -> 不明時はパス正規化比較)
                        - rename_no_replace, 空き容量確認, Staging, Zone.Identifier 読み書き
  transfer.rs         [新設] 自前コピー・移動・完全削除・Restart Manager(Win32) エンジン
  transfer/           [新設] lookahead_walker / progress / conflict / win_io / macos_io / fake_fs(test)
  archive/            [既存] fs_safety を利用するように整理（既存上限・挙動は維持）

apps/explorer/src/
  operation_center.rs [新設] プロセス共有 Global エンティティ
                        - 旧 archive_jobs.rs と file_operation_progress.rs を統合
                        - パスリース調停・有界キュー・履歴・100ms メールボックス集約
                        - PromptId 発行とマルチウィンドウ単一消費調停
  file_operations.rs  [改修] OsDelegated 実行 (FOF_SILENT等), 三層防御サイレントゴミ箱, クリップボード, D&D
  file_operations/
    macos_trash.m     [新設] NSFileManager::trashItemAtURL (ワーカースレッド上は @autoreleasepool で包む)
  explorer/
    activity.rs       [新設] 統一アクティビティ UI（ポップオーバー、確認/競合/エラー解決シート）
  folder.rs           [改修] status bar 右端にコンパクトピルを描画（旧2段バー廃止）
```

**ネイティブバインディング制約の正確な整理:**
- `explorer-core` には COM / `windows` クレート / `gpui` / `AppKit` を持ち込まない。
- Windows の `CopyFile2`、`MoveFileExW`、`SetFileInformationByHandle`、`ReplaceFileW`、および **Restart Manager**（COM ではなく純 C API の `Win32_System_RestartManager`）はすべて `windows-sys` で `explorer-core` に配置できる。
- macOS の `copyfile(3)` / `copyfile_state_*` / `COPYFILE_*` は `libc` クレートでの提供定数・宣言を確認し、不足するシンボルは `explorer-core` 内の最小限の `extern "C"` 宣言で補う。`NSFileManager::trashItemAtURL` と COM (`IFileOperation`) は `apps/explorer` 側の狭いアダプターに閉じる（[`apps/explorer/build.rs`](../../../apps/explorer/build.rs) の `cc` + `Foundation` 構成を活用）。

### 3.2 統一データモデル（型設計案）

```rust
pub struct JobId(pub u64);
pub struct PromptId(pub u64);

pub enum JobKind {
    Copy,
    Move,
    Trash,
    PermanentDelete,
    ArchiveCreate(ArchiveFormat),
    ArchiveExtract,
}

pub enum JobBackend {
    Builtin,
    /// OS標準機能（Windows IFileOperation / macOS Finder等）への委譲
    OsDelegated { reason: DelegateReason },
}

pub enum JobStatus {
    Queued,
    Running,
    Paused,
    WaitingForUser(PromptId),
    Cancelling,
    Completed,
    CompletedWithIssues,
    Failed,
    Cancelled,
}

pub enum TotalState {
    /// 先読み走査中（単調非減少の下限値）
    Counting { lower_bound: u64 },
    /// 走査完了により確定（原則として Unknown へ戻さない）
    Known(u64),
    /// 上限到達・クラウド未取得ディレクトリ等で合計不明
    Unknown,
}

pub enum ProgressMeasure {
    /// 自前エンジン・圧縮解凍が計測したバイト・項目ベース進捗
    Measured {
        bytes_done: u64,
        bytes_total: TotalState,
        items_done: u64,
        items_total: TotalState,
        bytes_per_sec: Option<u64>,
        eta_seconds: Option<u64>,
    },
    /// Windows IFileOperationProgressSink 由来（相対単位のみ）
    Relative {
        work_done: u32,
        work_total: u32,
    },
    /// osascript 等、進捗値を取得できない委譲処理
    Indeterminate,
}

pub struct JobProgress {
    pub measure: ProgressMeasure,
    pub current_item: Option<String>,
    /// CopyFile2 に直接コールバックがないため、事前取得した属性（RECALL_ON_* 等）から推定表示する
    pub hydrating_cloud_likely: bool,
}
```

**進捗・合計表示の不変条件:**
- `ProgressMeasure::Relative` や `Indeterminate` を偽のバイト数や偽の ETA に変換しない。
- `TotalState` が一度 `Known` になった後に `Unknown` へ戻さない（UI のちらつき防止）。
- 転送途中にファイルが増える等で `bytes_done` が `Known` の合計を超えた場合は合計値側を引き上げる。
- 表示パーセントは単調非減少とし、**終端状態（`Completed` 等）に達するまでは最大 99% に抑える**。スキップした項目のサイズ・件数も「処理対象として完了（`done`）」に含め、完了間際で 80% のまま止まる現象を防ぐ。

### 3.3 単一パス先読み走査・パスリース・ロック規律・有界上限

#### 1. 単一パス先読み走査（二重走査の廃止）
「別タスクで見積もり走査を走らせる」方式は同じツリーを2回列挙して HDD や SMB のスラッシングを悪化させるため採らない。
- 実行ワーカー内の **1つの走査器（Lookahead Walker）** がディレクトリを先読み（有限バッファ上限内で先行列挙）し、その列挙結果を **コピー実行と合計見積もりの両方で共有** する。
- ルート入力検証が終わり最初のバッチを先読みしたら、全件走査の完了を待たずに転送を開始する。
- 先読み走査において、`RECALL_ON_OPEN` または `SF_DATALESS` が立っている**ディレクトリには降りない**（列挙自体がクラウドの一覧取得・ダウンロードを誘発するため）。その場合 `bytes_total` は `TotalState::Unknown` とする。ファイルの中身は一切開かず列挙メタデータのみ参照する。

#### 2. 進捗通知と速度計算
- 既存の [`file_operation_progress.rs`](../../../apps/explorer/src/file_operation_progress.rs) と同じく、100ms（10Hz）間隔の最新値メールボックス＋`bounded(1)` シグナルで UI へ通知する。
- 速度は 1 秒窓＋指数移動平均（EMA）で算出し、一時停止中・確認待ち中・`hydrating_cloud_likely` 中・`TotalState` が `Known` 以外のときは `eta_seconds = None` とする。一時停止中や確認待ち時間は経過時間の分母から除外する。

#### 3. パスリースと排他規則（レーン間衝突の防止）
アーカイブレーンと転送レーンが並行動作すると、「圧縮中のフォルダーを移動・削除する」「コピー元を削除する」といった競合が起き得る。そのため `OperationCenter` に **パスリース（祖先・子孫重なり判定）** を導入する：
- 破壊的・変更を伴う操作（`Move` / `Trash` / `PermanentDelete`）は、実行中ジョブ（アーカイブ含む）の転送元または保存先とパスが祖先／子孫／一致の関係にある場合、先行ジョブの後ろに待機させる。
- Phase 0〜3 における `OsDelegated` ジョブは、**転送レーンと厳密に直列（排他）** として曖昧な並列動作を許さない。

#### 4. ロックと待機の規律（デッドロック・UI ブロック絶対禁止）
- `OperationCenter` の Mutex ロックを保持したまま、ファイルシステム呼び出し、チャネルでの待機、COM 呼び出し、GPUI 更新を**一切行わない**。
- UI スレッドはワーカーの処理待ち・ロック待ちで**一切ブロックしない**。
- 確認待ち（`WaitingForUser(PromptId)`）に入ったワーカーは、**応答チャネル（`bounded(1)`）と `CancelToken`（中止・終了シグナル）の両方を同時に待機** し、「中止」「アプリ終了」は必ず待機中のワーカーを即座に起床させる。
- `PromptResponse` は `(JobId, PromptId, Action, apply_to_all)` を持ち、`OperationCenter` が `PromptId` の一致・未消費を原子的に検証することで、マルチウィンドウからの重複応答を先着1回に調停する。

#### 5. 有界リソース上限（エンジニアリング提案値）

| 項目 | 提案上限 | 根拠・超過時の振る舞い |
|---|---|---|
| 待機キュー上限 | **全体 16 件**（圧縮系最大 4 件 / 転送系最大 12 件）<br>＋**完了通知保護**: `未配信完了通知 + 待機 + 実行中 < 保持上限`（[`archive_jobs.rs`](../../../apps/explorer/src/archive_jobs.rs) 継承） | 暴走キューイング防止および完了通知の欠落防止。超過時はエラーメッセージを即時返却 |
| 同時実行ワーカー数（Phase 0–3） | **転送レーン 1 ＋ アーカイブレーン 1**（`OsDelegated` は転送レーンと完全直列、さらにパスリースで衝突回避） | HDD / SMB 帯域の奪い合い防止と、アーカイブ・転送間の破壊的競合防止 |
| 同時実行ワーカー数（Phase 4） | **物理デバイス単位の直列化（異デバイス並列 最大 3〜4）** | 別ドライブ間の独立転送のみ並列化 |
| ジョブ履歴保持数 | **最新 32 件**（実行中・待機中・要確認は退避対象外） | メモリ上限保護。完了済みから FIFO で破棄（一括クリア操作可） |
| 1ジョブあたり `JobIssue` 保持 | **最大 256 件 かつ 総テキストバイト 256 KiB まで** | パス長最大 32 KiB によるメモリ爆発（最悪 268 MB 等）を防ぐため、件数・バイトの二重上限を課す（32件保持で最大 ~8 MiB）。超過後は件数カウンタのみ加算 |
| 先読み・走査上限 | 深さ **128 階層** / 単一パス **32 KiB** / 自作項目 `FileIdentity` 追跡セット最大 **100,000 件**（見積もり打ち切り: 例 2秒または 50,000件） | スタック・メモリ爆発防止。`FileIdentity` セット上限超過時は「自作項目衝突」の安全を保つため以降の競合で自動置換せず常に確認シートへ回す（§4.4） |

---

## 4. データ安全性・エッジケース仕様（必須不変条件）

「1ビットの意図しないデータ損失も起こさない」ため、以下を実装フェーズを問わず破ってはならない不変条件とする。

### 4.1 クロスボリューム移動の検証定義・耐久性・ハンドル削除

コピー完了直後に元を消すと、書き込みキャッシュの未フラッシュ（USB 抜去・電源断）や転送中の元ファイル書き換えでデータが消失する。元ファイルの削除は **以下の 3 条件をすべて満たしたときだけ** 行う：

1. **(a) 永続化フラッシュ**: 保存先ファイルを書き込みハンドルで開き、`FlushFileBuffers`（macOS は `fcntl(F_FULLFSYNC)`、非対応 FS のみ `fsync` へフォールバック）が成功したこと。
2. **(b) 転送元不変の検証**: コピー開始前に記録した転送元の `(FileIdentity, size, LastWriteTime, ChangeTime)` が、コピー完了後の再確認でも一致していること（Windows は `GetFileInformationByHandleEx(FileBasicInfo / FileStandardInfo)` 等）。
3. **(c) サイズ一致**: 保存先のサイズが転送元と一致していること（ハッシュ全量照合は速度影響が大きいため任意の将来設定とする）。

**削除時の TOCTOU 防止とディレクトリ削除規則:**
- ファイル削除はパス文字列ではなく **ハンドル経由** で行う：`FILE_FLAG_OPEN_REPARSE_POINT` 付きで `DELETE` アクセス権付きで開き、そのハンドルで `FileIdentity` を再確認したうえで `FileDispositionInfoEx(FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS)` を実行する。
- ディレクトリ削除は `RemoveDirectoryW` / `rmdir` の **非再帰削除** のみを使い、`remove_dir_all` は絶対に使わない。
- 同じディレクトリを列挙しながら元を削除すると FAT や SMB で列挙が項目を取りこぼすことがあるため、**ディレクトリの元削除はそのディレクトリの列挙が終わってから** 行う。`ERROR_DIR_NOT_EMPTY` が返った場合は再列挙して残存項目（失敗・スキップ・新規作成された別ファイル）を報告する。配下に1件でも失敗・スキップ・中止があれば、そのディレクトリと未処理の元は絶対に残す。

### 4.2 排他新規作成と安全な上書き置換（Atomic Replace）

#### 1. 新規コピー時の排他作成と一時ファイル公開（TOCTOU・破損防止）
- 競合の有無を「事前チェック文字列」で判定してから `CopyFile2` を呼ぶと、その隙間に同名ファイルが現れたときに無言上書きしてしまう（`CopyFile2` は既定で上書きするため）。
- 新規作成は **常に OS の排他フラグ付き** で実行し、OS の失敗コードで衝突を検出する（大文字・小文字差や 8.3 短縮名別名も OS に判定させる）：
  - **ファイル**: まず保存先ディレクトリ内に一時ファイル（隠し属性・明示接頭辞 `.skylis-copy-<pid>-<seq>.tmp`）へ `CopyFile2` / `copyfile(3)` でコピーし、完了後に `rename_no_replace`（Windows は既存時エラーの `MoveFileExW` / `SetFileInformationByHandle`、macOS は `renamex_np(RENAME_EXCL)`、exFAT/SMB の `ENOTSUP` 時は `O_CREAT | O_EXCL` 予約＋`rename`）で最終名へ公開する。
  - **一時ファイルを使う理由**: 異常終了・停電時に不完全なファイルが完成名として残り完成品と見分けがつかなくなるのを防ぐため。なお起動時に過去の残骸を自動走査して削除することはしない（軽量性原則と誤削除防止のため、ユーザー識別可能な接頭辞で残す）。
  - **ディレクトリ**: Windows は `CreateDirectoryW` の `ERROR_ALREADY_EXISTS`、macOS は `mkdir` の `EEXIST` で衝突検出する。

#### 2. 既存ファイルへの上書き置換セマンティクス
既存の保存先ファイルを「置換（上書き）」する意味は、**「転送元の内容・属性・タイムスタンプで置き換え、ACL は保存先親フォルダーから継承する」** に統一する。

- 既存ファイルへ直接上書き書き込みはしない（失敗時に新旧両方が壊れるため）。同ディレクトリの一時ファイルへコピー完了後、アトミック差し替えを行う：
  - **Windows 第一選択**: `SetFileInformationByHandle(FileRenameInfoEx, FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS | FILE_RENAME_FLAG_IGNORE_READONLY_ATTRIBUTE)` または同ディレクトリ内 `MoveFileExW(tmp, dst, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)`。
  - **なぜ `ReplaceFileW` を既定で使わないか**: `ReplaceFileW` は置換される**旧ファイル側**の属性・作成日時・ACL・短い名前・未マージ ADS（古い `Zone.Identifier` 等）を引き継ぎ、読み取り専用で失敗し、さらにエラー `1176`（`ERROR_UNABLE_TO_MOVE_REPLACEMENT`）発生時は**旧ファイルがすでに消えた状態で一時ファイルだけが残る**ため、通常の上書き期待と異なり危険が大きいためである。（万一将来オプション等で `ReplaceFileW` を使う場合は、1175 / 1176 / 1177 のエラー別に復旧手順を分け、1176 / 1177 では絶対に一時ファイルを削除せず `rename_no_replace` を試みて失敗時は一時ファイルパスを報告する。）
  - **macOS**: 同ディレクトリ上での `rename(tmp, dst)`（POSIX アトミック置換）。
- **ハードリンク／シンボリックリンク先への置換**: 保存先がシンボリックリンクや複数ハードリンクを持つファイルの場合、名前差し替えによる置換は「リンク先への上書き」ではなく「リンクの切断・差し替え」になるため、無言置換せず個別に確認シートを出す。

### 4.3 キャンセル・異常終了・一時ファイル後始末の不変条件

1. **一時ファイルを削除してよい絶対条件**: **「保存先の既存ファイルが無傷で残っている（または元々存在しなかった）と確認できたときだけ」** 一時ファイルを削除してよい。差し替え API が曖昧なエラーを返して保存先が消えた可能性がある場合は一時ファイルを削除せず、復旧先パスとしてユーザーに報告する。
2. **キャンセル時の後始末**: 自ジョブが新規作成した未完了の一時ファイル・新規空ディレクトリのみを追跡リストから削除する。既存ファイル・既存ディレクトリ・完了済みファイルには一切触れない。
3. **終了待機と OS シャットダウン**:
   - 「すべて中止して終了」時は、自前ワーカーが上記の一時ファイル後始末を終えるまで待機してからアプリを終了する。
   - `OsDelegated` がアプリから中断できない（または確認ダイアログ待ち等）の場合は、その旨を正直に表示して終了を保留する。
   - Windows の `WM_QUERYENDSESSION` および macOS のアプリケーション終了シグナル時は、進行中の自前ジョブに即座にキャンセルを発行し、一時ファイルの掃除と未完了通知を短いタイムアウト範囲で試みる。

### 4.4 同一実体判定（`FileIdentity`）・自己配下防御・自作項目競合防御

1. **`FileIdentity` と境界条件（SMB / NAS / 9P）**:
   - Windows は `GetFileInformationByHandleEx(FileIdInfo)`（128-bit `FileId` + `VolumeSerialNumber`）を優先し、非対応 FS のみ `GetFileInformationByHandle`（`dwVolumeSerialNumber` + `nFileIndexHigh/Low`）へフォールバック。macOS は `(st_dev, st_ino)`。
   - **SMB・NAS・9P（`\\\\wsl$` 等）の境界**: ファイル ID が `0` または一意性を保証しないファイルシステムでは `FileIdentity::Unknown` として扱い、その場合は正規化パス（`GetFinalPathNameByHandleW` / 大文字小文字ポリシー考慮）の比較へフォールバックし、なお曖昧なら安全側（拒否または確認）に倒す。
2. **自己配下コピー・移動の判定アルゴリズム**:
   - 保存先を `GetFinalPathNameByHandleW`（macOS は実体解決）で解決した親パスからルートまで祖先を順に辿り、各祖先の `FileIdentity` が転送元ディレクトリの `FileIdentity` と一致しないか検査する。これにより `subst`・ハードリンク・UNC/マップドライブ別名・8.3 名を貫通して防御する。
3. **同一ジョブが作成した項目との競合防御（重要）**:
   - 大文字・小文字を区別する FS（WSL、ケースセンシティブ APFS 等）や NFC/NFD 差から、非区別 FS（Windows NTFS、標準 APFS）へ `a.txt` と `A.txt` を移動／コピーする場合、2件目は「**このジョブ自身が先ほど書いたファイル**」と衝突する。
   - ここで「置換」または `apply_to_all` 自動置換を実行すると、移動では1件目の元ファイルがすでに消えているため **1件目の内容が完全に失われる**。
   - **対策**: ジョブ内で新規作成／公開した出力の `FileIdentity` を有界セット（最大 100,000 件）で記録し、競合先が自作項目の `FileIdentity` と一致した場合（またはセット上限超過時）は、**「置換」を選択肢から外し（「両方残す」「スキップ」のみ）、`apply_to_all` の対象からも強制除外** する。

### 4.5 同一フォルダー内ペーストと大小文字のみのリネーム

- **同一フォルダーへのコピー**: 複製（自動連番）とするか競合確認とするかは第8.2節（Q3）でユーザー確認のうえ確定する。
- **大小文字非区別 FS での大文字・小文字のみのリネーム**: APFS 等で `renamex_np(RENAME_EXCL)` を使うと同一実体に対して `EEXIST` が返るため、`FileIdentity` が同一かつ文字表記が異なるリネーム時は通常の `rename`（Windows は `MoveFileExW`）へ分岐する。

### 4.6 リンク・リパースポイント（転送元・保存先双方の No-Follow）

- 走査は **転送元と保存先の両方で No-Follow** とする。
- 名前サロゲート（`tag & 0x2000_0000`）とシンボリックリンクだけを辿らず、OneDrive 等の非サロゲートリパースポイント（`CLOUD_*`）は通常のファイル・ディレクトリとして走査・処理する。
- **保存先マージ時のリンク境界**: フォルダー統合（マージ）中に、保存先側の既存の子が名前サロゲートまたはシンボリックリンクだった場合、その中へ書き込んでツリー外（場合によっては転送元自身）を破壊するのを防ぐため、書き込みを止め「リンク先への統合はできません」と確認を出す。
- **ジャンクションの扱い**: `COPY_FILE_COPY_SYMLINK` はジャンクションに効かないため、ジャンクションのコピーは既定スキップ＋ユーザー確認とし、再作成オプションを許す場合もターゲット側の改変は行わない。

### 4.7 完全削除エンジン・属性復元・macOS 固有エッジケース

1. **完全削除のハンドル基準実行**:
   - 「先に読み取り専用属性を外してからパスで列挙・削除」は属性戻し忘れと TOCTOU を招くため禁止する。
   - **Windows**: `FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS` で開いたハンドル上でリパースタグを確認し、名前サロゲートでなければそのハンドル基準で列挙し、`FileDispositionInfoEx(FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE)` で削除する。名前サロゲートは開いた対象自体のみを削除し、絶対に配下へ降らない。
   - **macOS**: `openat(O_NOFOLLOW | O_DIRECTORY)` → `fdopendir` → `unlinkat` を用い、シンボリックリンクを辿らない。
2. **ディレクトリタイムスタンプ・権限復元**:
   - 更新日時の復元は **「このジョブが新規作成したディレクトリ」だけ** に行い、既存フォルダーへのマージ時は既存ディレクトリの日時を書き換えない。
   - macOS で `0555` 等書き込み不可のディレクトリをコピーする際は、作成時のみ一時的に `u+w` を付与して中身を書き、最後に元のモードへ戻す。
3. **macOS `copyfile(3)` 実装詳細とテストゲート**:
   - `COPYFILE_RECURSIVE` は使わない（競合確認・単位制御ができないため）。ディレクトリは自前で作成し、ファイル単位で `copyfile`（`COPYFILE_ALL | COPYFILE_CLONE | COPYFILE_EXCL` 等、なお `COPYFILE_CLONE` 自体が `EXCL | NOFOLLOW_SRC` を含む点に注意）を呼ぶ。
   - `COPYFILE_QUIT` でコールバック中断した場合は、途中まで書かれた保存先ファイルを必ず `unlink` する。
   - 現行 CI（`.github/workflows/ci.yml`）は `windows-2025` のみであるため、macOS ランナーが CI に追加されるか実機確認が完了するまで、macOS の自前転送エンジンは既定オフ（`osascript` 維持）で安全運用する。

### 4.8 Windows ゴミ箱サイレント実行の三層防御（Phase 1）

`IFileOperation` に `FOFX_RECYCLEONDELETE | FOF_SILENT | FOF_NOERRORUI | FOF_NOCONFIRMATION` を安易に組むと、ゴミ箱に入らない項目（リムーバブル、ネットワーク、大容量ファイル、ゴミ箱無効ドライブ）が**警告なしで即座に完全削除される**。なお、`SHQueryRecycleBinW` は「現在の使用量と件数」しか返さず `NukeOnDelete` や最大容量を返さないうえ、ゴミ箱は古い項目を自動削除して空きを作るため事前使用量比較は無効であり、ディレクトリの事前サイズ集計も軽量性原則に反する。さらに `PreDeleteItem` 時点でそれ以前の項目はすでにゴミ箱へ移動済みであるため「無傷で全停止」は成立しない。

そのため、Phase 1 では現行 [`apps/explorer/src/file_operations.rs`](../../../apps/explorer/src/file_operations.rs) の安全網を後退させないよう、以下の **三層防御** を採用する：

- **フラグ構成**: `FOFX_RECYCLEONDELETE | FOF_ALLOWUNDO | FOFX_ADDUNDORECORD | FOF_SILENT | FOF_NOERRORUI | FOF_NOCONFIRMATION | FOF_WANTNUKEWARNING`
  - `FOF_WANTNUKEWARNING` を**第3防御**として必ず残し、万一第1・第2防御をすり抜けた場合でも「無言の完全削除」ではなく「Windows の警告ダイアログ」が必ず出るようにする。
- **第1防御（安価な事前判定のみ）**:
  - ドライブ種別（リムーバブル・ネットワーク等）、レジストリ `BitBucket`（`NukeOnDelete` / `MaxCapacity`）、単一ファイルのサイズだけを確認する（ディレクトリの事前再帰集計はしない）。確実にゴミ箱へ入らない場合は実行前にアプリ内 `ConfirmPermanentDelete` を出す。
- **第2防御（主防御：`PreDeleteItem` フックによる項目単位遮断）**:
  - `IFileOperationProgressSink::PreDeleteItem` で `dwFlags & TSF_DELETE_RECYCLE_IF_POSSIBLE == 0` を検出したら、**その項目だけ** `COPYENGINE_E_RECYCLE_BIN_NOT_FOUND` を返して失敗記録し、残りのゴミ箱可能な項目は続行させる（注：失敗 HRESULT ごとの正確な継続／全停止挙動は Windows 実機で検証する）。操作完了後、拒否されて残った項目だけを対象にアプリ内 `ConfirmPermanentDelete` シートを出す。
- **事後検証（`PostDeleteItem`）**:
  - `PostDeleteItem` で `hrDelete == S_OK` なのに `psiNewlyCreated == NULL` だった項目は、ゴミ箱に入らなかった可能性があるため正直に結果リストで報告する。

### 4.9 周辺サブシステムとの整合性

1. **応答受信後の再検証**: 確認シートでユーザーが「置換」等を応答してから再開するまでの間に保存先が外部変更されている可能性に備え、再開直前に保存先の `(FileIdentity, size, mtime, kind)` を再検証し、変化していれば再確認する。`apply_to_all` は種別違い（ファイル対フォルダー）、自作項目衝突（§4.4）、リンク競合には適用しない。
2. **クリップボードの切り取り消去（`clear_file_clipboard`）**: [`apps/explorer/src/folder/transfers.rs`](../../../apps/explorer/src/folder/transfers.rs) の切り取り貼り付け完了後処理は、ジョブ結果が **`Completed`（全成功）かつシーケンス一致時のみ** 消去する。`CompletedWithIssues` / `Cancelled` / `Failed` ではクリップボードを保持する。
3. **ディレクトリ監視（[`apps/explorer/src/directory_watch.rs`](../../../apps/explorer/src/directory_watch.rs)）との協調**:
   - `directory_watch.rs` はすでに **非再帰・ローカル固定ドライブ限定・200ms デバウンス / 最大1s 集約** であるため、大量更新で負荷が顕著になるのは「表示中のフラットなフォルダー直下へ大量に書き込む」場合である。
   - コピー用一時ファイル（`.skylis-copy-...`）に隠し属性（または一覧での一時除外）を付けて不要なチラつきを抑えつつ、大量書き込み中の自動リフレッシュを間引き、完了／中止時に影響親ディレクトリを確定更新する。併せて完了時の `SHChangeNotify(SHCNE_UPDATEDIR)` 発行（Explorer やデスクトップ側の同期）を検討する。
4. **アプリ終了保護（`Explorer::request_quit`）**: [`apps/explorer/src/explorer.rs`](../../../apps/explorer/src/explorer.rs) の終了チェックを `OperationCenter` へ一本化し、実行中・待機中・確認待ちジョブが残る場合は終了を保留してアクティビティ UI へ誘導する。

---

## 5. UI / UX 設計（統一アクティビティ UI・確認シート）

### 5.1 レイアウト・所有権・誤操作防止

```text
+-------------------------------------------------------------------+
| タブストリップ                                                    |
+-------------------------------------------------------------------+
| ツールバー / パンくず・アドレスバー                               |
+-------------------------------------------------------------------+
| サイドバー | ファイル一覧 / ギャラリー               | 詳細ペイン |
|            |                                         |            |
+-------------------------------------------------------------------+
| 124 項目 · 3 項目を選択 · 42.1 MB      [コピー中 42% · 128 MB/s]  | <- 28px ステータスバー
+-------------------------------------------------------------------+
                                                   ^
                                                   | クリックでポップオーバー展開
                        +--------------------------+---------------+
                        | アクティビティ (進行中 1 · 待機 1) [整理]  |
                        |------------------------------------------|
                        | [!] 確認が必要: report.pdf が既に存在    |
                        |     [ 確認して選択... ]                  |
                        |------------------------------------------|
                        | コピー → D:\Backups          42%  [||][x]|
                        | ======-----------                        |
                        | photo_0192.raw  · 128 MB/s · 残り約 18秒 |
                        |------------------------------------------|
                        | ZIP作成: assets.zip (待機中)         [x] |
                        |  (※ 前の操作が確認待ちです)              |
                        |------------------------------------------|
                        | 解凍完了: release-v2       [保存先を開く] |
                        +------------------------------------------+
```

1. **通常時の専有面積ゼロ増・デュアル表示モード（下部内蔵 ⇄ 別ウィンドウ）**:
   - [`apps/explorer/src/folder.rs`](../../../apps/explorer/src/folder.rs) の旧縦積み2段バナー（`transfer_progress` と `render_archive_jobs`）を廃止し、28px ステータスバー右端の**コンパクトピル**に一本化する（待機・進行中・未確認エラー 0 件時は非表示）。
   - `settings_data.rs` に **`ActivityDisplay`** 設定を追加し、2つの表示方式を切り替え可能にする：
     - `ActivityDisplay::StatusBar`（**既定**）：下部ステータスバーのピルから `Explorer` ウィンドウ内のポップオーバーを開いて進捗・履歴を操作する。
     - `ActivityDisplay::Window`：Skylis 製の独立ミニ進行ウィンドウ（`ActivityWindow`：[`preview_window.rs`](../../../apps/explorer/src/folder/preview_window.rs) と同じ軽量 GPUI シングルトン別窓）を使用する。一瞬で終わる操作で窓がちらつかないよう、**開始から約 500ms 経過しても実行中の操作があるときだけ自動で開く**（既に開いている場合は前面化）。
   - **共通 `ActivityList` と随時ポップアウト**: 下部ポップオーバーと `ActivityWindow` は、単一の共通 `ActivityList` 描画コンポーネントを共有し、両方ともプロセス共有 `OperationCenter` を購読して二重管理を排する。また下部ポップオーバー右上に **「別ウィンドウで開く（ポップアウト）」** ボタンを備え、下部内蔵モードのままでも随時 `ActivityWindow` へ切り離せる（`ActivityWindow` が開く際は下部ポップオーバーを自動で閉じる）。
   - ピルおよび進捗表示テキスト：`Measured`（ライン2）は `コピー中 42% · 128 MB/s`、`Relative`（ライン1 / Shell 委譲）は `コピー中 42%`（または `Windows で処理中 42%`）、`Indeterminate` は不確定インジケーター、要確認時は警告色バッジで表示。
2. **ウィンドウ（`Explorer`）レベルの所有とマルチウィンドウ規則**:
   - ポップオーバーと確認シートは `FolderView` ではなく `Explorer` ウィンドウレベルで描画し、タブ切替や別フォルダー移動でも維持される。
   - **確認シートの自動表示ルール**: 全ウィンドウで確認シートを同時に開くと別ウィンドウで文字入力中に Enter で「置換」を誤確定する事故が起きる。確認シートを自動で開くのは **起動元ウィンドウだけ**（閉じていれば最後にアクティブだったウィンドウ）とし、他のウィンドウはピル上の「要確認」バッジだけを出す。
   - **フォーカスと誤確定ガード**: 確認シートの初期フォーカスは安全な選択肢（「キャンセル」または「スキップ」）に置き、**シート表示から約 400ms はキーボードでの確定操作を受け付けない**。
   - **待機レーン説明**: 単一転送レーン上で先行ジョブが確認待ち（または一時停止中）のため後続ジョブが進めない場合、待機カードへ「前の操作が確認待ちです」を明記する。
3. **モーション・テーマ整合**:
   - 開閉は [`apps/explorer/src/motion.rs`](../../../apps/explorer/src/motion.rs) / [`apps/explorer/src/motion/presence.rs`](../../../apps/explorer/src/motion/presence.rs) の `Presence<T>`（ポップオーバーに `POPUP`、確認シートに `MODAL`、退場中は `ghost_shield()` でクリック遮断、`reduce_motion` 尊重）を使い、配色は [`apps/explorer/src/theme_tokens.rs`](../../../apps/explorer/src/theme_tokens.rs) の既存セマンティックトークンをそのまま使用する。

### 5.2 ポップオーバーとアプリ内確認シートの操作要素

- **アクティビティポップオーバー**: ① 要確認 → ② 進行中/一時停止 → ③ 待機中 → ④ 最近の完了/一部失敗/中止。現在項目名（ホバーでフルパス）、進捗バー、速度・ETA（またはクラウドダウンロード推定／OS 委譲状態）、一時停止/再開（Phase 4）、中止、完了後「保存先を開く」、失敗項目一覧と再試行を提供。「完了済みを消す」は完了・失敗・中止済みのみ消す。
- **競合解決シート**: 転送元／保存先のアイコン・名前・サイズ・更新日時比較＋パス。「置換（上書き）」「スキップ」「両方残す（連番）」「フォルダーを統合（ディレクトリ衝突時）」＋「以降の競合にも同じ選択を適用（`apply_to_all`）」（※自作項目・種別違い・リンク衝突時は適用外）。
- **エラー / ロック / 権限解決シート**: 日本語エラー理由（Phase 4 で使用中プロセス名）、「再試行」「スキップ」「Windows 権限等で委譲して続行」「実体をコピー / 復号してコピー（該当時のみ）」「中止」。
- **完全削除確認シート**: 対象件数・代表名・合計サイズ（判明分）・保存場所を表示し、破壊ボタンは `danger.solid`。

---

## 6. 段階的ロードマップ（Phase 0〜5）と完了ゲート

「先に統一 UI とプロンプト基盤の器を作り、次に自前エンジンなしのライン1到達点（Phase 0.5 / Phase 1）を安全に完成させ、その後に自前コピー／移動（ライン2：Phase 2〜4）へ拡張する」段階構成を採る。各フェーズが独立してリリース・検証可能である。

### Phase 0（Milestone 1）: 統一ジョブモデル・共通 `ActivityList`・デュアル表示 UI・設定・プロンプト基盤（I/O 層は無変更）
- **内容**:
  - `OperationCenter` と `explorer/activity.rs`（コンパクトピル＋共通 `ActivityList`＋下部ポップオーバー＋「別ウィンドウで開く」ポップアウト対応の軽量 `ActivityWindow`＋**プロンプト基盤 `PromptId` / 確認シート / 安全フォーカス / 400ms 誤確定防止**）を実装。Phase 1 より先にプロンプト基盤をここで整えておく。
  - `settings_data.rs` / `settings_panel.rs` に「ファイル操作」カテゴリを追加し、`file_transfer_engine`（`OsIntegrated`［既定］ / `OsDialog`）および `activity_display`（`StatusBar`［既定］ / `Window`）を永続化・切り替え可能にする（未実装の独自エンジンは今後の段階実装予定であることを正直に記載し、既存カテゴリ添字との不整合を起こさない）。
  - `archive_jobs` と `file_operation_progress`（`IFileOperationProgressSink`）を `OperationCenter` へ統合し、`folder.rs` の旧2段バーを廃止。
  - `OsDelegated` は現行のプロセス排他を維持（実行時 I/O フラグは無変更）。
- **完了ゲート**: データ変更ロジックに触れずに全操作の表示・設定・`ActivityWindow`・プロンプト基盤が統合され、GPUI テスト（タブ移動/閉鎖/分離・マルチウィンドウ `PromptId` 単一消費・`ActivityWindow` ライフサイクル・`Presence`・100ms 間引き・設定カテゴリ遷移）と Windows CI が通過すること。

### Phase 0.5（Milestone 2 / 到達ライン 1）: Windows サイレント `IFileOperation` コピー／移動 ＋ Sink 中止 ＋ キュー化
- **内容**:
  - `file_transfer_engine == FileTransferEngine::OsIntegrated` のとき、Windows の通常コピー・移動を `IFileOperation` + `FOF_SILENT`（**`FOF_NOCONFIRMATION` は絶対に付けない**）＋ `IFileOperationProgressSink` からの中止返却 ＋ キュー待機（ワーカー実行開始時に有効 `HWND` を再解決し、閉じていれば他ウィンドウまたはフォールバック）へ切り替える。`OsDialog` 選択時は従来の標準ダイアログ挙動を維持する。
  - **ユーザー体験**: 普段のコピー・移動で Windows の進捗ウィンドウは一切出ず Skylis 内（下部ピル／`ActivityWindow`）で進捗・中止・キューが完結する。衝突・ロック・UAC 時だけ安全に Windows 標準ダイアログが出る。
- **完了ゲート**: Windows 実機で `IFileOperationProgressSink` からの途中キャンセルが安全に `Aborted` になること・連続操作がキューで順に実行されること・衝突/UAC ダイアログが正常に手前へ出ることを確認する。（※ここで止めても十分に実用的な独立到達点となる）

### Phase 1: ゴミ箱のアプリ内統合 ＋ macOS `osascript` ゴミ箱脱却
- **内容**:
  - Windows: §4.8 の三層防御（安価な事前判定 ＋ `PreDeleteItem` の `TSF_DELETE_RECYCLE_IF_POSSIBLE` 監視 ＋ `FOF_WANTNUKEWARNING` 残置 ＋ `PostDeleteItem` 検証）を伴うサイレント `IFileOperation` ゴミ箱削除。完全削除へフォールバックしてユーザーが `ConfirmPermanentDelete` を承諾した場合は、Phase 3 まで `IFileOperation`（ゴミ箱なし・`FOF_NOCONFIRMATION`）で実行する。
  - macOS: `osascript` ゴミ箱を廃止し、`macos_trash.m`（`NSFileManager::trashItemAtURL`）へ置換（Automation 権限不要化）。
- **完了ゲート**: 実機シナリオ表（リムーバブル、ネットワーク共有、ネットワークリダイレクト既知フォルダー、容量超過単一ファイル、ゴミ箱無効設定、GPO）の通過。

### Phase 2（到達ライン 2 開始）: 自前コピーエンジン（ローカル / SMB）＋ 競合・エラー解決 UI
- **内容**:
  - `explorer-core` に `fs_safety` と `transfer`（`CopyFile2` / `copyfile(3)`、単一パス先読み走査、排他一時作成＋Atomic Replace、`FileIdentity` 自己配下・自作項目衝突防御、ディレクトリ監視協調）を実装。
  - 競合解決・エラー解決シートを本格稼働し、`Settings` トグルで旧 `OsDelegated` 経路へ戻せる安全弁を設ける。
  - `TransferKind::Automatic`（[`volume.rs`](../../../apps/explorer/src/file_operations/volume.rs)）で移動が1件でも混ざるバッチは Phase 3 まで丸ごと `OsDelegated` へ流す。
  - *(選択的拡張)* 同一ボリューム内のアトミックリネーム移動はリスクが低いため、Phase 2 後半に前倒しする構成も採り得る。
- **完了ゲート**: 障害注入テスト（ディスク満杯、共有違反ロック、途中キャンセル、置換失敗時の一時ファイル・元データ保全、自己配下/別名/自作衝突、クラウドタグ、名前サロゲート非追従）全通過＋Opus 完了レビュー＋実機大容量テスト。

### Phase 3: 移動（同一ボリュームリネーム ／ 異ボリューム安全移行）＆ 完全削除の自前化
- **内容**:
  - 同一ボリュームリネーム（`MoveFileExW` → `ERROR_NOT_SAME_DEVICE` 切替）と、§4.1 の3条件（`FlushFileBuffers` / `F_FULLFSYNC` + 転送元不変 + サイズ一致 + ハンドル削除）に基づく異ボリューム安全移動を実装。
  - 完全削除（§4.7 ハンドル基準、`FileDispositionInfoEx` / `unlinkat`）を自前化。
  - 切り取り後の `clear_file_clipboard` を `Completed`（全成功）限定に厳格化。
  - macOS 自前転送は macOS CI ランナー整備・実機検証完了まで既定オフ（`osascript` 維持）とする。
- **完了ゲート**: 異ボリューム移動の強制失敗・部分失敗・キャンセルで元データが1つも消失しない障害注入テスト＆プロパティテスト通過、ジャンクション完全削除安全テスト通過。

### Phase 4: スケジューリング強化・診断性・仕上げ
- **内容**:
  - 物理デバイス単位の直列化キュー（別デバイス間並列 最大 3〜4）。
  - 一時停止 / 再開（ファイルハンドルや共有ロックを握り続けないよう **ファイル境界を基本** とし、`CopyFile2` コールバック内での長時間ブロックは避ける）。
  - Windows Restart Manager（`RmGetList`）によるロック元プロセス名表示。
  - 完了結果からの「失敗した項目だけ再試行」。
  - *(任意・設定)* 上書き置換された旧ファイルをそのまま消さずゴミ箱へ退避するオプション（将来のアプリ内 Undo 基盤にも活用可）。

### Phase 5（任意・別仕様）: 昇格ヘルパー（UAC）
- 保護フォルダー操作までアプリ内 UI へ取り込む必要が生じた場合のみ、別仕様として独立設計する。それまでは `ERROR_ACCESS_DENIED` 時の「Windows に委譲して再試行」で一貫させる。

---

## 7. 検証・テスト戦略

### 7.1 `explorer-core` の決定論テスト・障害注入テスト
`transfer` モジュールにファイルシステム・クロック抽象シーム（フェイク FS＋障害注入アダプター）を持たせ、以下を CI 上で高速に検証する：

- **上書き置換と一時ファイル安全（B2, B3, I5）**:
  - 排他作成により既存ファイルへの無言上書きが起きないこと（8.3 短縮名・大小文字差・一時ファイル書き込み中の割り込み作成を含む）。
  - 置換差し替えに失敗（または `1176` 等曖昧失敗）したとき、一時ファイルを誤削除せず元データ保全／復元パス通知を行うこと。
- **異ボリューム移動と完全削除（B4, B5, B6, I7, I9）**:
  - `FlushFileBuffers` / `F_FULLFSYNC` 失敗時・転送元サイズ/時刻変化時・1項目失敗/スキップ時に、親ディレクトリと未完了元が削除されないこと。
  - 大文字・小文字区別→非区別 FS での自作項目衝突（`a.txt` / `A.txt`）で「置換」が拒否され 1件目が失われないこと。
  - 保存先ディレクトリ内のジャンクション／シンボリックリンク先へ統合書き込み・削除再帰が一切入らないこと。
  - `FileIdentity` が `0`/Unknown（SMB/NAS/9P 相当）のとき安全側へフォールバックすること。
- **プロパティテスト**: ランダム生成ディレクトリ木（通常ファイル、空ディレクトリ、読み取り専用、リンク、非 UTF-8、予約名）に対するコピー・移動の同値性・無損失検証。

### 7.2 `apps/explorer` GPUI 統合テスト
- **ライフサイクル・マルチウィンドウ・ロック規律**:
  - 実行中タブの閉鎖・切替・別ウィンドウ分離（`detach`）と `OperationCenter` 継続、影響タブ自動更新。
  - 確認シートが起動元ウィンドウのみ自動表示され、他ウィンドウはピルバッジに留まること、表示後 400ms のキーボード確定ガード、同一 `PromptId` の先着1回消費。
  - `WaitingForUser` 待機中の `CancelToken` キャンセル・終了要求でワーカーがデッドロックせず即座に抜けること。
  - パスリース衝突（移動/削除が実行中コピー・アーカイブと重なる場合に直列化されること）と `JobIssue` 256件 / 256KiB 上限。
- **アクセシビリティ・モーション・UI**:
  - 100ms メールボックス間引き、`Presence<T>` の `ghost_shield()` 非反応性。
  - アクティビティポップオーバーおよび確認シートのキーボード操作（Tab / 矢印 / Esc）とスクリーンリーダー向け要素ラベル整合。
  - 進行中ジョブ残存時の `request_quit` 終了保留。

### 7.3 実機手動検証項目（OS 別境界の正直な記録）
自動テストでカバーできない実機特性は以下を検証対象とし、現在の Windows 開発環境で実行できない macOS 検証は正直に「未検証（macOS 実機 / CI 待ち）」と記録する：

- **Windows 実機**:
  - ゴミ箱シナリオ表：リムーバブルメディア、ネットワーク共有、ネットワークへリダイレクトされた既知フォルダー、ゴミ箱上限超過の単一ファイル、Windows 側「ゴミ箱に移動しない」設定、グループポリシー。
  - `CopyFile2` 実挙動：`COPY_FILE_COPY_SYMLINK`（開発者モード非昇格）、スパース/圧縮属性保持、FAT32/exFAT 等 ADS 非対応先での `Zone.Identifier` 扱い、`COPY_FILE_NO_BUFFERING` の SMB 速度比較。
  - Phase 0.5 `IFileOperationProgressSink` 中断時の各 HRESULT 挙動、>4GB・10万ファイルコピーと `directory_watch` / フレーム時間、OneDrive ハイドレーション、Restart Manager ロック検出。
- **macOS 実機（未検証領域）**:
  - `copyfile(3)` の APFS クローン速度・`xattr` 保持、`COPYFILE_QUIT` 後始末、`renamex_np` の exFAT/SMB フォールバック、`NSFileManager::trashItemAtURL` の権限不要動作と Finder「戻す」非対応の挙動確認、`SF_DATALESS` 挙動。

---

## 8. 非対象（Non-Goals）と実装前に確認したい製品判断

### 8.1 明確な非対象（やらないこと）
- `read`/`write` ループによる独自バイトコピー実装
- Windows `$Recycle.Bin` や macOS `.Trash` 内部構造の直接操作
- MTP デバイスや Windows 仮想シェルフォルダーの自前実装
- アプリ内 Undo（`Ctrl+Z` / `Cmd+Z`）の本仕様への同時抱き合わせ（まずはファイル操作基盤安定を優先し、Undo は別判断とする）
- Windows サードパーティ `ICopyHook` シェル拡張互換

### 8.2 決定済みの製品判断事項（Q0〜Q9・ユーザー承認反映済み）

2026-11-19 のユーザー確認により、Q0・Q1 の確定および Q2〜Q9 の推奨デフォルト採用が決定した。

#### 最重要（全体の方向性）
- **Q0. 到達ラインの選択 → 【確定：ライン 1 先行 ＋ 設定切替でライン 2 を後から追加】**
  - まず **Phase 0（Milestone 1）→ Phase 0.5 ＋ Phase 1（Milestone 2：ライン 1 完成）** を実装し、「普段 Windows の進捗窓が出ず Skylis 内で完結する体験」を最小リスクで完成させる。
  - その後にライン 2（独自エンジン：Phase 2〜4）を順次実装し、`FileTransferEngine` 設定でライン 1 / ライン 2（および `OsDialog`）を切り替えられるようにする。

#### UI / 動作ポリシー
- **Q1. ステータス表示の位置・方式 → 【確定：下部ステータスバー内蔵（既定）＋ 別ウィンドウ `ActivityWindow` 設定切替＆随時ポップアウト】**
  - 共通コンポーネント `ActivityList` を作り、下部 28px ステータスバー右端のピル＋ポップオーバー（`ActivityDisplay::StatusBar`、既定）と、Skylis 製ミニ進行ウィンドウ `ActivityWindow`（`ActivityDisplay::Window`、500ms 以上で自動表示）の両方を設定で切り替え可能にし、下部ポップオーバーからも随時ポップアウトできるようにする。
- **Q2. 将来 Phase 2（自前コピーエンジン）投入時の初期有効化方針 → 【確定：案 A（オプトイン / 既定オフで投入）】**
  - Phase 2 投入時点では `FileTransferEngine::OsIntegrated`（ライン 1）を既定に保ち、`Builtin`（ベータ）は設定でのオプトインから開始して十分な実戦検証後に既定昇格を検討する。
- **Q3. 同一フォルダー内ペースト時の挙動 → 【確定：案 A（Explorer 互換の自動別名複製）】**
  - 同じフォルダーへコピー＆ペーストしたときだけ、競合シートを出さず自動で別名複製（`file - コピー.txt` 相当）を作る。

#### Opus レビュー提起項目（推奨デフォルト採用）
- **Q4. macOS ゴミ箱の「戻す（Put Back）」トレードオフ → 【確定：推奨案（`NSFileManager::trashItemAtURL` 採用）】**
  - Automation 権限不要化を優先し、`trashItemAtURL` への切り替え（Finder の「戻す」非対応）を許容する。
- **Q5. Windows ゴミ箱削除前の確認表示 → 【確定：推奨案（即時ゴミ箱移動・完全削除フォールバック時のみアプリ内確認）】**
  - ゴミ箱移動自体は即時実行とし、完全削除へフォールバックする場合だけアプリ内 `ConfirmPermanentDelete`（および第3防御 `FOF_WANTNUKEWARNING`）で必ず確認する。
- **Q6. ADS 非対応の保存先（FAT32 / exFAT 等）へダウンロード済みファイルをコピーしたときの扱い → 【確定：推奨案（確認シートで明示確認）】**
  - ライン 2 実装時に `Zone.Identifier` が保持できない保存先へコピーする際は、確認シート（「ダウンロード元の安全情報が失われますが続行しますか？」＋以降適用）で確認する。
- **Q7. 競合やエラー発生時のジョブ進行方式 → 【確定：推奨案（案 A：競合・エラー発生時に一時停止して即座に確認）】**
  - まずは予測しやすい Explorer 互換（競合時にジョブを一時停止して確認）を採用し、大量コピー向けの後回し保留モードは将来拡張候補とする。
- **Q8. 確認シートの自動表示先ウィンドウ → 【確定：案 A（起動元ウィンドウ限定）】**
  - 起動元ウィンドウ（閉じていれば最後にアクティブなウィンドウ）にだけ自動でシートを出し、他ウィンドウはピルバッジのみにする（入力中ウィンドウの誤操作防止）。
- **Q9. macOS 自前エンジンの有効化条件 → 【確定：推奨案（macOS CI / 実機確認完了まで `osascript` 維持）】**
  - macOS CI ランナー整備または macOS 実機での検証が完了するまで、macOS のコピー・移動は安全のため `osascript`（または未有効フラグ）のまま残す。

---

## 9. レビュー反映記録（Opus 5.5 High）

第1回独立設計レビュー（[Opus 5.5 High review of in-app file operations design spec](t3-thread://v2/2c3f18ba-5058-4e57-a821-88c7dd91db2e/thread:delegated-task:command%3Amcp%3A510fb532-7278-4012-910a-fda0e7ac029d%3Adelegate-task%3A93a5c4b8-4ee2-4cf7-b83e-994f4b4330ee)）の指摘を受領し、全項目を本改訂版へ反映した。

- **Blocker 6件の修正（すべて反映済み）**:
  - **B1（§4.8 ゴミ箱防御）**: 誤った `SHQueryRecycleBinW` 前提と「無傷で全停止」表現を撤回し、`FOF_WANTNUKEWARNING` を第3防御として維持したまま、安価な事前判定 ＋ `PreDeleteItem` の `TSF_DELETE_RECYCLE_IF_POSSIBLE` 項目別遮断 ＋ `PostDeleteItem` 事後検証の三層防御へ刷新。
  - **B2（§4.2 排他新規作成）**: 事前文字列チェックによる TOCTOU を防ぐため、ファイル・ディレクトリとも OS の排他作成失敗（`COPY_FILE_FAIL_IF_EXISTS` / `COPYFILE_EXCL` / `EEXIST` 等）を競合検出の起点に統一。
  - **B3（§4.2・§4.3 置換 API 変更）**: 旧ファイルメタデータ引継ぎと `1176` 時のデータ消失罠を持つ `ReplaceFileW` を既定から外し、`SetFileInformationByHandle(FileRenameInfoEx)` / `MoveFileExW` アトミック置換へ変更。「一時ファイル削除は保存先旧ファイルの無傷確認時のみ」を絶対条件に追加し、リンク置換の個別確認を明記。
  - **B4（§4.1 異ボリューム移動の検証定義）**: 削除前の `FlushFileBuffers` / `F_FULLFSYNC`、転送元メタデータ不変確認、サイズ一致、ハンドルベース削除（`FileDispositionInfoEx`）、列挙完了後の非再帰 `RemoveDirectoryW` を規定。
  - **B5（§2.1・§4.6 リンク/リパース分類厳密化）**: 名前サロゲート（`tag & 0x2000_0000`）と `CLOUD_*` タグを明確に区別し、ジャンクション非コピー（スキップ＋確認）と保存先マージ時のリンク経由書き込み遮断を追加。
  - **B6（§4.4 自作項目衝突防御）**: ケース差や NFC/NFD 差で同一ジョブの自作項目と衝突した際に「置換」と `apply_to_all` を禁止する `FileIdentity` 追跡（上限10万件）を追加。
- **Important 12件の修正（すべて反映済み）**:
  - **I1**: プロンプト基盤の Phase 0 前倒し、Phase 1 での完全削除実行経路明記、遅延実行時の `HWND` 再解決を反映。
  - **I2**: `CancelToken`＋応答チャネル同時待機、再開前再検証、`apply_to_all` 例外、起動元ウィンドウ限定シート表示、400ms 誤確定防止、単一レーン待機表示を反映。
  - **I3**: アーカイブ・転送間のパスリース調停、Phase 0–3 の `OsDelegated` 完全直列化を明記。
  - **I4**: 二重走査を廃した単一パス先読み走査、`RECALL_ON_OPEN`/`SF_DATALESS` ディレクトリ非降下、`TotalState`（99% 上限・単調非減少・スキップ計上）、`hydrating_cloud_likely` 推定表現を反映。
  - **I5**: 新規コピーの一時ファイル経由公開、シャットダウン（`WM_QUERYENDSESSION` 等）と後始末完了待機を反映。
  - **I6**: `CopyFile2` が ACL をコピーせず保存先親継承である事実訂正、要実機検証リスト追加、一時停止のファイル境界基準化を反映。
  - **I7**: 完全削除のハンドル基準実装（`FileDispositionInfoEx` / `unlinkat`）と macOS 未対応の明記。
  - **I8**: macOS `copyfile(3)` のディレクトリ自前走査、`COPYFILE_QUIT` 後 `unlink`、`renamex_np` `ENOTSUP` 代替、APFS ケース変更分岐、`@autoreleasepool`、macOS CI 整備前の既定オフ方針を追記。
  - **I9**: SMB/NAS/9P で `FileIdentity` が `0` になる境界と、`GetFinalPathNameByHandleW` 祖先辿りアルゴリズムを明記。
  - **I10**: Phase 2 での `TransferKind::Automatic` 混在バッチの `OsDelegated` 一括委譲を追記。
  - **I11**: `JobIssue` の 256 KiB 総バイト上限と、完了イベント保護条件の継承を明記。
  - **I12**: `OperationCenter` のロック・チャネル待機規律を明文化。
- **Suggestion 12件の反映＆ユーザーとの会話統合**:
  - `JobBackend` 名称統一、`ProgressMeasure::Measured` / `Indeterminate` 整理（S1）、Restart Manager の配置訂正（S2）、`fs_safety` 共通化範囲拡大（S3）、`MoveFileExW` の `ERROR_NOT_SAME_DEVICE` 切替方式（S4）、`directory_watch` 負荷条件の正確な記述と `SHChangeNotify` 検討（S5）、Phase 0.5（到達ライン 1）独立位置づけと Sink 中止の要検証明記（S6）、新規作成ディレクトリのみ日時復元（S7）、同一ボリューム移動の前倒し案注記（S8）、旧ファイルゴミ箱退避オプション案（S9）、キーボード/SR テスト追加（S10）、`COPY_FILE_NO_BUFFERING` のベンチマーク条件（S11）、本記録作成（S12）を反映し、Q0 および Q4〜Q9 を §8.2 へ統合した。
