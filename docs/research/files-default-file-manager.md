# Filesの標準ファイルマネージャー連携の調査

調査日: 2026-11-15。Filesの公開版タグ `v4.3` と、調査時のmain `f6f35e798a81664d73aa0fbfdc6a223bd753909e` を確認。Skylis側は `9bb376c` 時点。公開ソース・公式説明を読んだ調査であり、Filesのインストールや実レジストリ変更は行っていない。

## 結論

Filesも関連付けとWindows Shellの仕組みを利用する方式で、すべての `explorer.exe` 起動を取り込むものではない。ただしSkylisより対象が広く、Win+Eと、外部アプリからの「フォルダーを開いて対象ファイルを選択する」要求に追加対応している。「現在のSkylisとまったく同じ制限」と説明するのは不正確。

## 確認した実装

### フォルダーを開く

`SetFilesAsDefault.reg` は、現在のユーザーの次の登録先を書き換える。

- `Software\Classes\Folder\shell\open\command`
- `Software\Classes\Folder\shell\explore\command`
- `Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\opennewwindow\command`（Win+E）

`DelegateExecute` を空にし、`%LOCALAPPDATA%\Files\Files.App.Launcher.exe` に渡す。旧方式の `Directory\shell\openinfiles` はコメントとして残っている。

出典: [Files v4.3の登録スクリプト](https://github.com/files-community/Files/blob/v4.3/src/Files.App/Assets/FilesOpenDialog/SetFilesAsDefault.reg)、[公式Advanced設定](https://www.files.community/docs/customize-settings/advanced)。

### 管理者権限の要求

設定画面は登録・解除用の `regedit.exe /s` を `Verb = "runas"` で起動する。したがって、ユーザーが記憶していたUAC要求は実装上も確認できる。一方、有効化スクリプトの登録先はHKCUであり、「管理者権限によってすべてのプロセス起動を横取りしている」という意味ではない。解除スクリプトにはHKCRへの書き込みも含まれる。現在のコードから、昇格を選んだ開発者の意図までは断定しない。

出典: [設定ハンドラー](https://github.com/files-community/Files/blob/v4.3/src/Files.App/ViewModels/Settings/AdvancedViewModel.cs)、[解除スクリプト](https://github.com/files-community/Files/blob/v4.3/src/Files.App/Assets/FilesOpenDialog/UnsetFilesAsDefault.reg)。

### 外部アプリの「フォルダー内で表示」

専用ランチャーが、一時的な非表示ウィンドウとCOM実装を `IShellWindows` に登録する。`SHOpenFolderAndSelectItems` による選択要求を `IShellView::SelectItem` で受け、Filesへ `-select` 付きで転送する。単なるフォルダーパスの受け渡しより一段広い対応。

[PR #18784](https://github.com/files-community/Files/pull/18784) は2026-08-13にマージ済み。Chromium系ブラウザーのダウンロードから「フォルダーで表示」を扱うための変更で、公開版v4.3のソースにも存在する。ランチャーは選択通知が遅れる場合の待機・追加転送も持つ。

出典: [FilesLauncher.cpp](https://github.com/files-community/Files/blob/v4.3/src/Files.App.Launcher/FilesLauncher.cpp)、[OpenInFolder.cpp](https://github.com/files-community/Files/blob/v4.3/src/Files.App.Launcher/OpenInFolder.cpp)。

### 残る制限

Filesメンテナーは2026-08-05、Explorerを固定で呼ぶ一部アプリでは置き換えが効かないことを説明している。すべての `explorer.exe` 直接起動・すべての `/select` 形式が同じ結果になるとまでは、今回の証拠から断定しない。呼び出し経路、Windowsの版、既存Shellウィンドウなどを含む実機確認が必要。

また、ランチャーは未対応のShell名前空間などをExplorerへ戻す処理を持つ。ファイル選択／保存ダイアログの置き換えは別の設定・COM実装で、v4.3の設定UIでは開発環境向けに制限されている。標準ファイルマネージャー設定を有効にすれば、すべての「開く」「保存」画面まで置き換わるわけではない。

出典: [メンテナーの説明](https://github.com/files-community/Files/issues/18798#issuecomment-5192879600)、[Shellフックを伴わない案を求める追記](https://github.com/files-community/Files/issues/18798#issuecomment-5192890776)、[AdvancedPage.xaml](https://github.com/files-community/Files/blob/v4.3/src/Files.App/Views/Settings/AdvancedPage.xaml)。

## Skylisへの示唆

Skylisの現状は `Directory` と `Drive` の既定／open登録であり、Win+E登録と選択要求を仲介するCOMランチャーを持たない。

実装を広げるなら、まずWin+Eの明示的な登録・復元、次に「開いて選択」の受け口と既存ウィンドウへの転送を独立して追加するのが妥当。`Folder` へ対象を広げる場合は、未対応の仮想フォルダーをExplorerへ戻す処理も必要になる。Skylisの既存の復元ジャーナルと競合保護は維持する。

外部アプリ→SkylisのD&Dについては、ユーザーからコピー固定を許容する案が出た。コピー印と実処理を揃える方向の候補として記録する。今回の依頼は調査であり、D&Dの動作変更は行っていない。送信元の許可effectや完了通知が伝わらない問題全体が、コピー固定だけで解決したとは扱わない。
