# GPUI系によるクロスプラットフォーム・ファイルエクスプローラー調査

調査日: 2026-11-14。オンラインのREADME、GitHub API、crates.io APIを参照。実機性能測定・全APIのソース差分検証は未実施。以下の「推奨」「評価」は調査からの設計判断であり、ベンチマーク結果ではない。main/HEADのリンクは将来変わる。

## エグゼクティブサマリー

1. **GPUI採用は条件付きで推奨**。共通デザインと大量行の仮想表示には適するが、Windows/macOSのシェル統合まで完成品として提供する枠組みではない。[本家](https://github.com/zed-industries/zed/tree/main/crates/gpui)
2. 初期の基準実装を本家に近いGPUIに置き、同一画面をFASTで比較する。FASTを最初から排除する理由はないが、実験段階という自己評価を受け入れる必要がある。[FAST](https://github.com/longbridge/gpui-fast)
3. CEは独立コミュニティ版で、README自身がAPI互換性の変化を明記する。「CEなら将来も互換」とは扱えない。[CE](https://github.com/gpui-ce/gpui-ce)
4. 最も重要なのは非同期列挙、段階的メタデータ取得、可視行だけの描画、サムネイルの優先度制御。GPU描画だけではディスク待ちを消せない。[Yazi](https://github.com/sxyazi/yazi)
5. FASTのWebView合成は追加機能として隔離し、初期版はWebViewなしを推奨。画像・テキスト・PDFのページ画像は別の仕組みで扱える。[FAST](https://github.com/longbridge/gpui-fast)
6. GPUI製ファイラーはすでに存在する。Nexus、Skwir、Atlasなどを再利用候補・実装研究対象にする。ただし宣伝上の「高速」を実測と混同しない。[Nexus](https://github.com/Augani/nexus-explorer)、[Skwir](https://github.com/skwir-org/skwir)
7. gpui-componentの旧URLは現在GPUI Kitへ転送される。古い紹介の依存関係をそのまま使わない。[Kit](https://github.com/longbridge/gpui-kit)
8. UI非依存のdomain、filesystem、jobs、search、preview、platform crateを中心に組む。GPUI型をコアへ持ち込まない。
9. 全GUIを抽象化するのは割に合わない。依存窓口とOSサービスだけを薄く包み、移行時にはUIの修正を許容する。
10. 本採用前に10万行、低速NAS、日本語IME、外部D&D、スクリーンリーダーを両OSで検証する。短納期で完成度を優先するならiced/Slintも比較対象。

## 1. GPUI系の現状

### 1.1 系譜・配布・活動

GitHub APIから取得した2026-11-14時点の概数。最終コミットはUTC。Zedのスター数はエディタ全体の値で、GPUI単体の人気ではない。

|系統|主体・正体|スター / 最終コミット|配布とライセンス|対応・注意|
|---|---|---|---|---|
|本家GPUI|Zed Industries、エディタと同居|94,983 / 11-14 01:23|gpui 0.2.2、crate更新10-30。GPUIはApache-2.0、Zed全体と区別|Windows/macOS/Linux。エディタの出荷実績と任意アプリの完成度は別|
|GPUI CE|gpui-ce organization、独立フォーク|1,610 / 11-13 07:08|gpui-ce 0.3.0、更新10-20、Apache-2.0|3OSの実装を持つが、本調査でWindows品質の定量比較は未確認|
|GPUI FAST|Longbridge、保持・増分描画とネイティブ合成の実験|609 / 11-13 04:29|gpui-fast 0.1.15、更新11-13、Apache-2.0|3OS。READMEはexperimentalと明記|
|GPUI Kit（旧gpui-component）|Longbridgeのコンポーネント／アプリ基盤|20,634 / 11-14 02:27|gpui-kit 0.9.1、更新11-09。構成crateごとのLICENSEを確認|3OS、75以上の部品、AccessKit対応をREADMEで表明。GPUIの別エンジンとは区別|

出典: [本家API](https://api.github.com/repos/zed-industries/zed)、[CE API](https://api.github.com/repos/gpui-ce/gpui-ce)、[FAST API](https://api.github.com/repos/longbridge/gpui-fast)、[Kit API](https://api.github.com/repos/longbridge/gpui-kit)、[gpui配布](https://crates.io/crates/gpui)、[CE配布](https://crates.io/crates/gpui-ce)、[FAST配布](https://crates.io/crates/gpui-fast)、[Kit配布](https://crates.io/crates/gpui-kit)。リリース間隔の統計は未集計。更新日だけから毎週リリース等とは断定しない。

その他: gpui-preは本家の配布スナップショット、gpui-unofficial/GPUI Archipelagoも配布・拡張の別ルート。JcodeのフォークはFASTに独自パッチを追加する。依存源を増やすほど型・バージョン統一の負担が増す。[Kit](https://github.com/longbridge/gpui-kit)、[Archipelago](https://gpui-archipelago.github.io/)、[Jcode](https://github.com/1jehuang/gpui)

### 1.2 API互換性と分岐

CEの現行READMEは「mostly API compatible ... but that is changing」と明記。Entity<T>、cx.notify、Element、自前描画、uniform_listを例示している。古いModel/View時代のチュートリアルを現在のEntity APIに混ぜない。[CE README](https://github.com/gpui-ce/gpui-ce)

|領域|共通部分・移植方針|確認できた分岐／未確認|
|---|---|---|
|Element / Render|既成要素とRender中心にし、独自Elementを最少に|全traitシグネチャの同一性は未検証。独自レイアウトほど移植負担|
|Entity / 状態|UI状態に限定、コアは通常のRustデータ|FASTの追跡外状態変更ではcx.notifyが必要|
|テキスト・レイアウト|共通スタイルの小部分を利用|FASTはレイアウトノード・文字計測の再利用。CEにはblur、transition等の例|
|uniform_list / list|固定高一覧はuniform_list、可変高はlist系を個別評価|CEの例は範囲指定で可視行を要求。全版のスクロール状態互換は未確認|
|画像|コアは画像バイト／独自ID、GPU画像はUI内|画像ハンドル、キャッシュ無効化の完全互換は未確認|
|D&D|アプリ内とOS間を別の受入条件にする|内部ドラッグAPIがあることは、Explorer/Finderとの相互運用保証にならない|
|メニュー・IME・装飾|OSサービスとUI表示を分離|日本語変換、候補位置、ネイティブメニュー、DPI、タイトルバーの版別品質は実機未確認|
|WebView|機能境界の外へ隔離|FASTはWKWebView、WebView2 composition、Linux子surface合成を明示|

出典: [本家ソース](https://github.com/zed-industries/zed/tree/main/crates/gpui/src)、[CE](https://github.com/gpui-ce/gpui-ce)、[FAST docs](https://github.com/longbridge/gpui-fast/tree/main/docs)。互換性保証の百分率や網羅的差分表は公開資料から確定できなかった。

FASTの性能資料は保持／スクロールキャッシュの有効性を示す開発元測定だが、Linuxの部品ギャラリー等の条件であり、Windowsの巨大フォルダや起動時間の優位を直接証明しない。上記READMEの数値を製品の速度見積もりには使わない。[測定条件・再現手順](https://github.com/longbridge/gpui-fast#performance-vs-gpui)

## 2. 類似ソフト

### GPUI製の直接競合

|プロジェクト|確認した構成・完成度|参考にする点|
|---|---|---|
|[Nexus Explorer](https://github.com/Augani/nexus-explorer)|Rust/GPUI、キャッシュ即時表示と背景照合を説明。小規模公開プロジェクト|ディスク待ちと描画を分離。ただしsub-16msは作者の主張|
|[Skwir](https://github.com/skwir-org/skwir)|Rust/GPUI、プレビュー・タグ・コピー計画。Linuxが実際の配布対象、Win/Macビルドは未実行と明記|「cross-platform」の表示だけで成熟度を判断しない好例|
|[Atlas](https://github.com/neondeex/atlas-filemanager)|Windows向けRust/GPUI、コマンドパレット|Windows専用統合の研究対象|
|[rust-explorer](https://github.com/Mingcodeingmiao/rust-explorer)|Windows原生ファイラーとして公開|Windows側の比較対象、Mac対応の証拠にはしない|
|[macOS fuzzy explorer](https://github.com/situmorang-com/file-explorer)|macOS、GPUI、fuzzy/content search|検索中心の小さい製品境界|
|[pifile](https://github.com/raythurman2386/pifile)|GPUI Kit、Linux/Pi向け、fsコア分離|UI非依存構成の参考|
|[rustyfiles](https://github.com/gelenmedya/rustyfiles)|Windows、GPUI、windows-rs、notify、Velopack|OS統合と配布を含む研究対象|

全プロジェクトをビルド・操作したわけではなく、完成度はREADME上の範囲。日常利用の安全性、データ損失耐性は未検証。

### 他スタックと既存製品

|対象|技術・設計上の参考|評価・限界|
|---|---|---|
|[Spacedrive](https://github.com/spacedriveapp/spacedrive)|Rust+TypeScript、分散／統合ストレージと索引|現在のREADMEは2.0 Betaを11月15日予定、初回Mac/Linux、Windows nightlyと説明。古いTauri紹介を現行全体の説明に流用しない|
|[COSMIC Files](https://github.com/pop-os/cosmic-files)|Rust、COSMIC/libcosmic（iced系）、Linuxデスクトップ統合|実用OS統合の参考。Windows/Mac共通製品の直接テンプレートではない|
|[Yazi](https://github.com/sxyazi/yazi)|Rust TUI、非同期I/O、優先度・キャンセル・先読み|高速化の設計が明示される。端末操作・画像プロトコル依存はGUIと条件が異なる|
|[Files](https://github.com/files-community/Files)|Windows、C#/WinUI、Fluent系の豊富な機能|デザインの参考。フォルダ表示遅延とウィンドウ移動のstutterのIssueあり|
|[One Commander](https://onecommander.com/)|Windows、列表示・二画面・プレビュー|情報密度と写真整理を研究。内部最適化・現行スタックの詳細は未確認|
|[Directory Opus](https://www.gpsoft.com.au/)|Windows、柔軟なLister・コピーキュー・高度な操作|操作設計を参考。内部実装と速度優位は公開情報だけでは確定しない|
|[Path Finder](https://www.cocoatech.io/)|macOS、モジュール型・二画面|高機能による設定量の増加は設計上の検討点|
|[ForkLift](https://binarynights.com/)|macOS、二画面・リモート接続・同期|ローカル／リモートを一貫して操作するUX|
|[Commander One](https://mac.eltima.com/file-manager.html)|macOS、二画面・各種接続|キーボード主体の運用と拡張機能の整理|
|[Marta](https://marta.sh/)|macOS、キーボード・テーマ・拡張|共通アクションと配色の参考|
|[Double Commander](https://github.com/doublecmd/doublecmd)|Free Pascal/Lazarus、二画面・プラグイン|成熟した一括操作を参考。現代的な見た目とは別軸|

ユーザー不満の一次資料: [Filesの遅延報告 #17567](https://github.com/files-community/Files/issues/17567)、[移動stutter #19028](https://github.com/files-community/Files/issues/19028)。個別報告であり全ユーザーの評価ではない。上記商用製品の満足度・不満の代表性を裏づける調査は未実施。公式機能紹介をユーザーの高評価として数えない。

## 3. 推奨技術スタック表

以下は採用候補。各リンクはAPI・制約の確認先であり、組み合わせ済みの保証ではない。

|機能|推奨候補／OS API|実装上の判断|
|---|---|---|
|単一フォルダ列挙|[std::fs::read_dir](https://doc.rust-lang.org/std/fs/fn.read_dir.html)、Windows FindFirstFileExW|最初は名前と最低限の種別をストリーム。全件stat完了を待たない|
|再帰走査|[jwalk](https://github.com/Byron/jwalk)、[ignore](https://github.com/BurntSushi/ripgrep/tree/master/crates/ignore)、Rayon|jwalkの並列化は再帰に有効。単一フォルダで自動的に速いとは限らない。NASの同時数を抑える|
|監視|[notify](https://github.com/notify-rs/notify)|イベントを真実の全ログとせず再照合。ネットワークFSはpolling fallback、rename・overflow処理|
|アイコン|[SHGetFileInfoW](https://learn.microsoft.com/windows/win32/api/shellapi/nf-shellapi-shgetfileinfow)、[IShellItemImageFactory](https://learn.microsoft.com/windows/win32/api/shobjidl_core/nn-shobjidl_core-ishellitemimagefactory)、[NSWorkspace](https://developer.apple.com/documentation/appkit/nsworkspace)|低優先度ジョブ、拡張子キャッシュ、可視行優先。COM/AppKitのスレッド要件を守る|
|サムネイル|OSキャッシュ、[image](https://github.com/image-rs/image)、[fast_image_resize](https://github.com/Cykooz/fast_image_resize)|パス・mtime・サイズ・要求解像度で鍵。デコード上限とLRU、古い要求破棄|
|ゴミ箱|[trash](https://github.com/Byron/trash)|永久削除と別コマンド。ネットワーク／外付けの対応失敗を明示|
|コピー・移動|独自Job+チャンクI/O、[IFileOperation](https://learn.microsoft.com/windows/win32/api/shobjidl_core/nn-shobjidl_core-ifileoperation)|進捗・協調キャンセル、同一volume rename。別volumeはcopy後delete。ACL/xattr、symlink、部分ファイル、競合方針を定義|
|外部D&D|Windows OLE IDataObject/IDropTarget、macOS NSDragging/NSPasteboard|ファイルURLだけでなく仮想ファイルの必要性を切り分け。UI内ドラッグと分離|
|シェルメニュー|[IContextMenu](https://learn.microsoft.com/windows/win32/api/shobjidl_core/nn-shobjidl_core-icontextmenu)|初版は自前共通メニュー、後から「OSメニュー」。第三者シェル拡張の遅延・停止をUIスレッドに持ち込まない設計|
|ネットワーク／クラウド|OSにマウントされたパスから開始、[OpenDAL](https://github.com/apache/opendal)、rcloneは追加候補|UNC・切断・認証・placeholderを扱う。一覧だけでクラウド実体をdownloadしない。VFS全面導入は後回し|
|検索|名前: [nucleo](https://github.com/helix-editor/nucleo)、内容: ripgrep、索引: [Tantivy](https://github.com/quickwit-oss/tantivy)/SQLite|名前検索と本文索引を分離。更新・削除・権限変更・DB再構築を設計|
|OS検索|[Everything SDK](https://www.voidtools.com/support/everything/sdk/)、macOS Spotlight/Core Spotlight|Everythingは外部サービス依存、対応範囲とIPCタイムアウトを隔離。Spotlightも全パスを保証しない|
|文字プレビュー|[ropey](https://github.com/cessen/ropey)、[syntect](https://github.com/trishume/syntect)、tree-sitter|先頭の有限バイトだけ読み、巨大／バイナリ／文字コードを判定|
|画像／PDF／動画|image、[pdfium-render](https://github.com/ajrcarey/pdfium-render)、[MuPDF](https://mupdf.com/)、[FFmpeg](https://ffmpeg.org/)|PDFをページ画像として表示。動画は初版静止画+外部起動。動的ライブラリ配布とライセンスは採用時に精査|
|OS binding|[windows-rs](https://github.com/microsoft/windows-rs)、[objc2](https://github.com/madsmtm/objc2)|raw handleとunsafeをplatform crate内に閉じる|

### WebViewを使わない範囲

テキスト、Markdownの限定構文、画像、PDFページ、動画サムネイル、音声メタデータまではWebViewなしで設計できる。GPUI Kitはnative Markdown/HTML renderingを表明するが、完全なブラウザ互換を意味しない。[Kit](https://github.com/longbridge/gpui-kit)

Officeの完全再現、任意HTML/JS、DRM動画を初版で抱えず外部アプリへ渡す。PDF・動画decoderはhelper process化するとクラッシュ隔離しやすい。FAST固有の合成を後から足す場合はpreview-host境界だけを差し替える。[FAST composition](https://github.com/longbridge/gpui-fast#window-composition)

## 4. エディション非依存の設計

### 推奨crate構成図（提案）

```text
workspace/
  apps/explorer          起動、DI、配布設定
  crates/domain         EntryId、Location、Command、Event（GPUI依存なし）
  crates/fs-local       列挙、stat、監視の再照合
  crates/jobs           コピー計画、進捗、取消、競合、履歴
  crates/search         名前検索、索引、OS検索adapter
  crates/preview        decoder、thumbnail cache、helper protocol
  crates/platform       OSサービスtraitと共通DTO
  crates/platform-win   COM、Shell、OLE、Everything
  crates/platform-mac   AppKit、pasteboard、Spotlight
  crates/app-state      tabs、selection、navigation、snapshot/reducer
  crates/ui-gpui        Entity、Render、仮想リスト、フォーカス
  crates/ui-controls    テーマ、行、パンくず、コマンドパレット
  crates/gpui-bridge    依存のre-export、差が出た小関数のみ
```

OS handle、Entity、Image、SharedStringをコアの公開APIに含めない。PathBuf/OsStringを保持し、表示用文字列と分離する。バックグラウンドはbounded channelでbatchを送り、navigation generationが変わった結果は捨てる。選択は行番号ではなく安定IDで管理する。これらは本調査の設計提案。

feature切り替えは**ビルド時に一系統だけ**とする。異なるpackageからのEntityは同名でも異なる型。Kitと本体の依存が別のGPUIを引くと薄いre-exportだけでは解決しない。Cargo.lock、git rev、Kit互換設定をまとめて管理する。[FAST互換入口](https://github.com/longbridge/gpui-fast/tree/main/compat)、[Kitの依存説明](https://github.com/longbridge/gpui-kit)

全ElementやRenderを独自traitへ再実装するのは避ける。OSサービス、preview、actionとtheme tokenだけを安定境界にして、画面はGPUIに素直に書く。三版常時対応より、採用版+移行候補一版の小画面コンパイル確認に留める方が保守負担を抑えられる。

### 採用判断と代替

|候補|この製品への評価（設計判断）|一次情報|
|---|---|---|
|GPUI|自由な共通描画と仮想一覧に好適。API追従とOS統合を自分で引き受ける|[GPUI](https://gpui.rs/)|
|Slint|宣言的UI、ツール群を重視する場合に有力。現行のroyalty-free条件等を確認|[README](https://github.com/slint-ui/slint)、[LICENSE](https://github.com/slint-ui/slint/blob/master/LICENSE.md)|
|iced|Rust中心、状態更新モデルが明確。Shell統合は別途必要|[iced](https://github.com/iced-rs/iced)|
|egui|試作・内部ツールに強い。大規模一覧、IME、外部D&Dと製品らしい操作を個別評価|[egui](https://github.com/emilk/egui)|
|Makepad|GPU描画の自由度が高い候補。GPUIからの移行が容易という意味ではない|[Makepad](https://github.com/makepad/makepad)|
|Freya / Dioxus|Freyaはネイティブ描画系、Dioxus desktopはWebView系という違いを区別|[Freya](https://github.com/marc2332/freya)、[Dioxus](https://github.com/DioxusLabs/dioxus)|
|Tauri|Web UI資産とプレビューを優先するなら強い。今回のWebViewを減らす方針とはトレードオフ|[Tauri](https://github.com/tauri-apps/tauri)|

率直な結論: 個人開発でデザインと操作感を磨く意欲があり、Windows統合の検証時間を確保できるならGPUIは筋がよい。ファイラーの性能はコア設計に支配されるため「FASTだから速い」を採用理由の中心にはしない。スクリーンリーダーとShell完全互換が初日から必須なら、採用判断を実機試作まで保留する。

## 5. デザイン方向性（提案）

[Zed](https://zed.dev/)の静かな高密度、[Raycast](https://www.raycast.com/)の検索とアクション、[Linear](https://linear.app/)の階層、[Arc](https://arc.net/)のサイドバー整理、[Warp](https://www.warp.dev/)のまとまりを参考にする。外観の模倣より、選択・フォーカス・作業中の区別を明確にする。

共通theme tokenで背景3段階、境界線、アクセント1色、選択色を定義。日本語と英数字のfallback、DPIごとの行高を確認し、ファイル名は読みやすいsans、サイズ・日付はtabular digits。共通アイコンには[Lucide](https://lucide.dev/)等を検討し、OS取得のファイルアイコンと混ぜる際は寸法を統一する。

固定高の詳細表示を最初に磨き、gridは可視範囲だけサムネイル取得。アニメーションは短い選択・panel transition中心、無限spinnerや全面blurを避ける。CEはblur/transition例、FASTは保持描画があるが、採用版での性能検証が必要。[CE](https://github.com/gpui-ce/gpui-ce)、[FAST](https://github.com/longbridge/gpui-fast)

CSDで見た目を揃えても、Macのtraffic lights、Windows snap、resize edge、最大化、Alt+Space、DPI変更、非アクティブ状態はOS契約として維持する。見た目の共通化とOS操作規約は両立させる。完全独自タイトルバーは最後に実施する。

## 6. リスク・未確認事項・採用ゲート

- API差分: trait/関数の網羅的機械比較、各版の同一アプリコンパイルは未実施。互換保証はしない。
- Windows成熟度: GPU driver、複数monitor DPI、IME、外部D&D、touchpad、accessibilityの実機確認が必要。KitのAccessKit表明を全フォークの保証に拡張しない。[Kit](https://github.com/longbridge/gpui-kit)
- ファイル操作: 再parse point、symlink、ACL、xattr、case-only rename、長いパス、非UTF-8名、切断、disk full、キャンセル後の残骸が最重要。
- メモリ: 仮想描画でも全件の文字列・metadataは残る。Entry構造、キャッシュ上限、索引サイズを測る。
- 計測提案: 1千/1万/10万件、cold/warm、SSD/USB/NASを分け、初回可視行までの時間、全列挙時間、p95/p99 frame、RSS、idle CPU、起動から入力可能までを記録。FASTは同じデータ・画面・release設定で比較。
- 未確認: 商用ファイラーの内部アルゴリズム、代表性あるユーザー満足度、各版のIME/アクセシビリティ網羅性、長期保守体制。READMEの将来予定は出荷済み機能ではない。
- 時点差: Spacedriveの11月15日Betaは調査日の翌日の予定であり、リリース済みと書かない。古いgpui-component記事と現行Kitを区別する。

## 7. 出典一覧・再確認入口

本文各表のリンクを個別主張の出典とする。主要な更新確認先:

- 本家: https://github.com/zed-industries/zed/tree/main/crates/gpui / https://github.com/zed-industries/zed/releases
- CE: https://github.com/gpui-ce/gpui-ce / https://github.com/gpui-ce/gpui-ce/releases
- FAST: https://github.com/longbridge/gpui-fast / https://github.com/longbridge/gpui-fast/tree/main/docs
- Kit: https://github.com/longbridge/gpui-kit / https://gpui-kit.com/
- crates.io API: https://crates.io/api/v1/crates/gpui / https://crates.io/api/v1/crates/gpui-ce / https://crates.io/api/v1/crates/gpui-fast / https://crates.io/api/v1/crates/gpui-kit
- 直接競合・OS API・crate・代替GUIの一次情報は各節のリンクを参照。

調査の限界: 広範囲の探索と一次資料の読み取りによる比較であり、ベンチマーク、ソース監査、製品品質保証ではない。未確認の部分を性能優位や互換性の断定で埋めていない。
