# T3 Code の Reveal と Explorer からの切り替え

調査日: 2026-11-15。T3 Code 公開ソース `649d9fc1ab16fd28f8d927155c4315ab8034963a` と、ローカルにインストールされている server.asar 内の同等実装を読み取りで確認した。インストール済みアプリは変更していない。

## T3 Code の経路

現在の T3 Code は、Windows の Reveal を一律に Electron `shell.showItemInFolder` へ渡してはいない。サーバー側の `buildFileManagerRevealPowerShellSource` が HKCR の `Directory` / `Folder` の既定 verb と command を調べる（ドライブ直下の場合は `Drive` も対象）。Windows Explorer 以外の既定ハンドラーを検出すると、対象ファイルの**親フォルダー**に `Invoke-Item -LiteralPath` を実行する。

Skylis の既存 `Directory\shell` の既定値 `Skylis.Open` はこの検出対象になる。したがって、Skylis のフォルダー連携を有効にすると、この T3 Code 実装からの Reveal は親フォルダーを Skylis で開ける。ただし、この分岐で T3 Code は**選択するファイル名を渡していない**。Skylis 側の `--select` や Files 型の COM 選択ブリッジだけでは、この分岐で捨てられた情報は復元できない。

通常設定の `Directory\shell` 既定値が `none` の場合、T3 Code は短時間キャッシュした判定から `explorer.exe /select,"対象"` を直接起動する高速経路を持つ。設定変更直後はキャッシュの影響も考慮する必要がある。すべての T3 Code の版・他アプリの直接起動を置き換えられる保証とは区別する。

出典:

- [置き換え先を判定する PowerShell](https://github.com/pingdotgg/t3code/blob/649d9fc1ab16fd28f8d927155c4315ab8034963a/apps/server/src/process/externalLauncher.ts#L592-L643)
- [Windows の Reveal 高速経路](https://github.com/pingdotgg/t3code/blob/649d9fc1ab16fd28f8d927155c4315ab8034963a/apps/server/src/process/externalLauncher.ts#L728-L795)

## 今回の Skylis の受け口

- `--select "ファイルまたはフォルダーの絶対パス"`: 親を開いて、その名前を選択する。外部アプリが明示的に Skylis を指定する連携の受け口。
- Win+E: `HKCU\Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\opennewwindow\command` を独立して登録・復元する。
- Explorer のフォルダー背景: 従来型の右クリックメニュー（Windows 11 では「その他のオプションを確認」内）に「Skylisへ切り替え」を追加する。
- `Folder` クラスは変更しない。ゴミ箱などの仮想場所は対象外。

既存の FolderIntegrationBackupV1 の plan、値番号、キー番号は変更しない。Win+E と切り替えメニューはそれぞれ別の復元ジャーナルを持ち、設定UIでも独立して有効化・復元する。

## 元の Explorer を閉じる条件

背景メニューの `DelegateExecute` は、短時間だけ動作する COM LocalServer を呼ぶ。`IExecuteCommand` / `IObjectWithSite` / `IObjectWithSelection` を実装し、呼び出し元の site から `SID_SFolderView` → `IShellView` を取得し、`SID_STopLevelBrowser` → `IShellBrowser` の現在のビューと同一 COM identity か照合する。現在のアクティブビューだけを呼び出し元として採用しない。フォアグラウンドのウィンドウや、同じパスを開いたウィンドウを探して推測する方式ではない。常駐監視、プロセス終了、Explorer 全体の強制終了は行わない。

サーバーは元の ShellView の COM identity、ウィンドウ HWND、Explorer の PID とプロセス作成時刻、フォルダー PIDL を保持し、ランダムな要求IDで Skylis を起動する。Skylis が対象フォルダーの一覧読み込みに成功したときだけ、名前付きイベントで完了を返す。読み込み失敗・読み込み完了前の移動・起動失敗なら元を残す。完了通知後もガードを保持し、閉じる条件を確認している間の移動でもキャンセルできる。

その後、同じ HWND/PID/作成時刻、同じ ShellView、同じフォルダーを再照合する。さらに、`ShellTabWindowClass` 子ウィンドウが1つ、UI Automation の TabItem が1つ、の両方を必要条件とする。複数タブ、Windows版によるUI差異、取得不能などで確認できない場合は元を残し、Skylisに通知する。両方の観測を使う理由は、非アクティブのタブが ShellWindows に列挙されるかなどの版依存挙動を単独の根拠にしないため。

確認できた場合だけ、その HWND に通常の `WM_CLOSE` を送る。ウィンドウ消滅も確認し、成功を通知する。静的コマンドとして `--switch-explorer` が起動された場合は site がないため、フォルダーを開くだけで元を残す。

COMのsiteからフォルダーを取得できない場合は、まず選択配列を確認し、それでも取得できなければ `SetParameters` の明示的なパス、引数がなければ `SetDirectory` の作業ディレクトリを使う。引数は `--switch-explorer "パス"` または単一のパスだけを受理し、文字数、引用符、絶対ファイルシステムパスを検証する。補助プロセス内で実在するディレクトリか確認してから起動する。相対パス、デバイス名前空間、不正な引数、通常ファイルは拒否する。これらの代替情報は呼び出し元ビューの証明にならないため、元を閉じるためのトークンは発行しない。`SetDirectory` はあくまで作業ディレクトリというAPI契約であり、背景メニューの場所と常に一致する保証ではない。[SetDirectory](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-iexecutecommand-setdirectory)、[SetParameters](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-iexecutecommand-setparameters)

COMのsite取得・再照合は各6秒の期限で `CoCancelCall` を要求し、さらに2秒応答がない場合は一時helper自身だけを終了する。GPUI側やExplorerを強制終了する処理ではない。helper全体の120秒期限も別に監視し、COM解放・終了処理の停止で残留しないようにする。UI Automationにも個別の接続／呼び出し期限がある。閉じる直前に同じ検査を改めて実行し、途中で期限を超えた要求は閉じる処理へ進めない。

`WM_CLOSE` 送信後1.5秒で消滅を確認できない場合、または完了応答が失われた場合は「閉じたか確認できない」と通知する。終了要求が後から処理される可能性があるため、この場合に「元を残した」と断定しない。

これは未確認の Windows バージョンでも自動クローズを保証する実装ではない。安全条件が成立しない場合も「Skylisで開く」は利用できる。COM・ウィンドウ状態の実機確認は、独立したテスト用 Explorer ウィンドウを使って行う。

出典:

- [Shell verb 実装方式と IExecuteCommand](https://learn.microsoft.com/en-us/windows/win32/shell/shortcut-choose-method)
- [IExecuteCommand](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-iexecutecommand)
- [IShellBrowser::QueryActiveShellView](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellbrowser-queryactiveshellview)
- [SID_SFolderView でコンテキストメニューのサイトを取得](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-icontextmenusite)

## 残す範囲

### 2026-11-15の実機検証

生成したテスト用フォルダーと専用Explorerウィンドウで確認した。背景メニューからCOMサーバーが呼び出され、`--handoff-token`付きでSkylisが起動した。単一タブでは一覧読み込み後に元のウィンドウだけが閉じ、成功通知が出た。2タブでは元の2タブが残り、保留理由が表示された。復元操作後は背景メニューとCOM登録が消えた。Win+Eも設定からの登録・復元とコマンド文字列を確認したが、Win+Eの実キー入力は未検証。既存の通常フォルダー関連付けは変更していない。全Windows版で同じ結果を保証する検証ではない。

Files の `IShellWindows` / `IShellView::SelectItem` 選択ブリッジは別機能として扱う。今回の explicit command server はその置き換えではない。T3 Code の現在の既定ハンドラー分岐にはファイル名がないため、T3 側の連携変更なしに完全な「開いて選択」を保証しない。ファイル選択・保存ダイアログも置き換えない。

## 選択ブリッジの追加調査と最小案

**判断: 技術的には `Folder` 全体の上書きやアプリ全体の単一インスタンス化をせずに、通常フォルダーの選択ブリッジを追加できる。ただし、現在の実装にCOM登録を数行足せば任意アプリのRevealが完全に動く、という機能ではない。今回は追加せず、以下の受信経路と実機検証を独立した変更として扱う。**

Files v4.3 の経路は次の3段階であり、役割が異なる。

1. `Folder\shell\open` / `explore` がランチャーを起動する。これは起動の関連付け。公開登録スクリプトでは `Directory` の旧登録はコメントアウトされている。[登録スクリプト](https://github.com/files-community/Files/blob/v4.3/src/Files.App/Assets/FilesOpenDialog/SetFilesAsDefault.reg)
2. ランチャーが対象フォルダーのPIDLを作り、`IShellWindows::RegisterPending` → `Register` → `OnNavigate` により一時的な非表示Shellウィンドウを登録する。COMオブジェクトは `IWebBrowserApp::get_Document`、`IServiceProvider::QueryService`、`IShellView::SelectItem` を提供する。選択された子PIDLと親PIDLを合わせ、選択先を取得する。[OpenInFolder.cpp](https://github.com/files-community/Files/blob/v4.3/src/Files.App.Launcher/OpenInFolder.cpp)
3. 取得済みのパスを `files-dev:?cmd=... -select ...` にしてアプリへ渡す。`files-dev:` プロトコルはパッケージアプリへの輸送経路であり、Windowsの選択要求を捕捉するフックではない。Filesアプリ側には `AppInstance` のactivation redirectもある。[ランチャー](https://github.com/files-community/Files/blob/v4.3/src/Files.App.Launcher/FilesLauncher.cpp)、[Package.appxmanifest](https://github.com/files-community/Files/blob/v4.3/src/Files.App/Package.appxmanifest)、[Program.cs](https://github.com/files-community/Files/blob/v4.3/src/Files.App/Program.cs)

Filesの実装は最初に500ms待ち、選択がなければフォルダーを先に開く。その後も10秒間受信を継続し、遅れて届いた選択を追加activationで転送する。初期の実装ではショートカットから何も開かなくなる報告もあり、プロトコルactivationの配送を待つ修正が必要だった。Skylisで単にこの「フォルダー起動→遅延選択で再起動」をコピーすると、現在の複数プロセス方式では二つのウィンドウになり得る。[ランチャーの遅延受信処理](https://github.com/files-community/Files/blob/v4.3/src/Files.App.Launcher/FilesLauncher.cpp)、[Files #18818](https://github.com/files-community/Files/issues/18818)

### Directory / Drive だけで可能な範囲

`IShellWindows` の登録そのものは、`Folder` クラスの既定変更を要求するAPIではない。指定PIDLでShellウィンドウを登録・検索する仕組みであり、既に起動した受信先には利用できる。起動前の要求を受けるには、まずその呼び出し経路がSkylisの通常フォルダー関連付けに到達する必要がある。[RegisterPending](https://learn.microsoft.com/en-us/windows/win32/api/exdisp/nf-exdisp-ishellwindows-registerpending)、[FindWindowSW](https://learn.microsoft.com/en-us/windows/win32/api/exdisp/nf-exdisp-ishellwindows-findwindowsw)

OneCommander開発者は2022年、`Directory` と `Drive` のopenだけを変更し、ブラウザーからフォルダーパスを受け取れているがファイル名を受信できていない、と説明している。これは狭い関連付けでも起動経路が存在する一次証拠である。一方、現在のすべてのWindows版・既知フォルダー・呼び出し形式を保証する証拠ではない。[開発者の説明](https://groups.google.com/g/onecommander/c/-wTQarNGRP8)

Filesの選択対応PRはChrome/Edgeでの動作を示すが、登録条件はFiles側の広い関連付けであり、その検証結果をそのままSkylisのDirectory/Drive限定へ一般化しない。Chromiumの該当コードは `SHOpenFolderAndSelectItems` を呼び、失敗時にフォルダーだけを開くフォールバックを持つ。[Files PR #18784](https://github.com/files-community/Files/pull/18784)、[Chromium実装](https://github.com/chromium/chromium/blob/d86b833661437c96234c43b172f83561e583ee0d/chrome/browser/platform_util_win.cc#L73-L85)

### Skylisに適した最小の追加構成

- `--shell-folder` で起動した**そのプロセス**内に、期限付きのSTA ShellWindow proxyを1つ作る。対象のPIDL、登録cookie、非表示HWNDを専有する。今回の背景切り替え用 `IExecuteCommand` とは別の受信インターフェースになる。
- GPUI起動を一律500ms遅らせる代わりに、proxyからbounded channelで起動対象のFolderViewへ選択通知を送る。遅れて届いても同じFolderViewに反映し、新規プロセスを追加起動しない。URI登録や全体の単一インスタンス化は必須ではない。
- 読み込み完了前／後の選択を扱い、表示対象が変わったら通知を破棄して `Revoke` する。選択先をファイルシステムの子要素として検証し、元のフォルダーを勝手に別の場所へ変更しない。
- 同じ場所の既存Explorer・複数Skylis・同時Revealが競合する場合の方針を決める。`FindWindowSW` はPIDLによる検索であり、`SelectItem` に起動要求IDはない。「特定のcallerに対応するproxyへ必ず配送される」とは保証できない。同じフォルダーのproxyを無制限に重ねない調停が必要。
- `SHOpenFolderAndSelectItems` は複数選択と `OFASI_EDIT` を持つ。Files型の最初の1項目だけで終了する受信コードをそのまま採用せず、選択フラグと複数通知、対応外のrename要求を明示的に扱う。[APIの契約](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shopenfolderandselectitems)

実装を承認する前の最小検証は、隔離ユーザーまたはVMでDirectory/Driveだけを登録し、専用テストフォルダーに対する `SHOpenFolderAndSelectItems`（単一・複数・cidl=0）、Chrome/Edge/Firefox、通常の `ShellExecute("open")`、同じパスの既存Explorerあり／なし、同時2要求、遅延選択、ナビゲーション中断を確認すること。既知フォルダーとUNCは別ケースで検証する。未対応の仮想Shell場所へ対象を広げるなら、その後に `Folder` の登録と再帰しないExplorerフォールバックを検討する。

このブリッジを追加しても、T3 Codeの既定ハンドラー分岐で既に省かれたファイル名、任意の `explorer.exe` 直接起動、ファイル選択／保存ダイアログは補完できない。
