# Skylis File Manager UI/UX・アニメーション調査報告

- **対象リポジトリ**: `E:\github_projects\Skylis-File-Manager`
- **調査範囲（ソースコード／トークン／設計文書）**:
  - 表示設定ポップアップ: [display_settings.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/display_settings.rs)
  - 設定モーダル: [settings_panel.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/explorer/settings_panel.rs), [settings.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/settings.rs), [settings_data.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/settings_data.rs)
  - メインUI・ツールバー・サイドバー・インスペクタ: [folder.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder.rs), [explorer.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/explorer.rs), [scrollbar.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/scrollbar.rs)
  - テーマトークン: [theme_tokens.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/theme_tokens.rs), [theme.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/theme.rs)
  - アニメーション基盤: [animation.rs](file:///E:/github_projects/Skylis-File-Manager/vendor/gpui-fast/src/elements/animation.rs), [context_menu.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/context_menu.rs), [scroll_motion.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/scroll_motion.rs)
  - 設計仕様・規約: [display-settings.md](file:///E:/github_projects/Skylis-File-Manager/docs/implementation/display-settings.md), [settings-navigation.md](file:///E:/github_projects/Skylis-File-Manager/docs/implementation/settings-navigation.md), [AGENTS.md](file:///E:/github_projects/Skylis-File-Manager/AGENTS.md)

> [!NOTE]
> **検証方法に関する注記**: 参照スクリーンショット3枚（いずれも `1924×1202`）について、本作業環境のファイル読込ツールがPNG画像の視覚レンダリングに非対応であったため、ピクセル単位の直接目視検証は行えていません。ただしオーナーが観察された症状はすべて上記Rustソースおよびデザイントークンの実装と1対1で一致しており、本報告の指摘はコード上の確定事実に基づいています。

---

## 1. 全体総評

Skylis File Manager のベースUI（`surface.canvas` / `surface.chrome` / `surface.content` のカード分離、10px角丸のワークスペース構成、明確なステータスバー、アクセント青 `nav.current.bg` のアクティブ表示）は、軽量で情報の見通しが良く、拡張の土台として優れています。

一方で、**「後から追加されたオーバーレイUI（表示設定ポップアップ・設定モーダル）」** において、トークンの使い分けルール・階層表現・ボタンスタイルがベースUIと噛み合っておらず、「何が選択中か」「どこが押せるか」「どこまでが同じグループか」が読み取りにくくなっています。また、コンテキストメニューやスクロールには `100〜180ms` の丁寧なモーションが入っているにもかかわらず、ポップアップやモーダル・スイッチ類が `0ms` で瞬時に切り替わるため、全体として「硬い・荒い」印象を与えています。

---

## 2. 問題点の言語化（重要度順）

### 優先度1（最重点）：表示設定ポップアップ

#### 1-1. 適用範囲と設定行の背景色が乱立し、選択状態・境界が破綻している
- **根拠箇所**: [display_settings.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/display_settings.rs#L244-L315)
- **何が起きているか**:
  1. 「このウィンドウ」見出し（`window == true`）と、その直下の「詳細ペイン」行（`display_switch` 内の `if row == 3 { "accent.soft" }`）の両方が `accent.soft`（ライト `#2563eb1f` / ダーク `#3b82f633`）で塗られています。
  2. 「すべてのウィンドウ」見出し（`global == true`）、「隠しファイル」行（`surface.field`）、および「表示密度」セクション全体（`if row == 5 { "surface.field" }`）がすべて `surface.field` で塗られています。
  3. ところがセグメントの受け皿 `pair()` も `bg(t.c("surface.field"))` であるため、**「表示密度」行ではセグメントの枠背景がセクション背景に完全に溶けて消滅**しています。さらに `pb(px(10.))` の余白部分だけ親の `surface.raised`（白/#2c2c2c）に戻るため、ポップアップ下端に不自然な色の段差が生じます。
- **ユーザーへの不利益**:
  - 「このウィンドウ」一帯だけが青い帯になって目立ちすぎ、「詳細ペイン」が常にアクティブ・選択中・あるいは警告対象のように誤認されます。
  - 3つのセクションで「白背景」「青背景」「グレー背景＋枠消失」と見た目がバラバラになり、「適用範囲」という本来有益なメタ情報が視覚ノイズに変わっています。

#### 1-2. セグメント選択の視認性が弱く、行ごとの実装が3パターンに分裂している
- **根拠箇所**: [display_settings.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/display_settings.rs#L194-L398)
- **何が起きているか**:
  - `display_option` の選択状態が `accent.soft`＋1px枠線のみで、未選択セル（透明）と高さ・明暗のコントラストが弱いです（設定画面では `surface.chip-selected` で浮き上がる表現を使っているのに対し、ここでは使われていません）。
  - さらに、同じポップアップ内で「行のインデント」と「キーボード選択時の左青バー（`border_l_2`）」の作りが3通りに分かれています：
    1. `section()`（表示形式 / 並べ替え / 表示密度）: 外枠 `px(12.)`、左バーはラベル文字列だけ（`.pl(px(3.))`）。
    2. 昇順・降順（`row == 2`）: `section()` を使わず外枠 `div` に直接 `.border_l_2().px(px(12.))`。
    3. `display_switch()`（詳細ペイン / 隠しファイル）: 外枠に `.border_l_2().px(px(15.))`。
- **ユーザーへの不利益**: 左右の整列ラインが `12px` と `15px` でズレ、どの項目にカーソル／フォーカスがあるか一貫した法則で読み取れません。

#### 1-3. ツールチップ「表示設定」がポップアップ本体に重なる
- **根拠箇所**: [folder.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder.rs#L2686-L2724), [display_settings.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/display_settings.rs#L97-L115)
- **何が起きているか**:
  - `render_display_trigger` が呼び出す `icon_button("display-settings-trigger", ...)` に無条件で `.tooltip(...)` が付いています。
  - ポップアップはトリガー直下（`anchor.bottom() + px(6.)`）に開くため、クリック後もマウスがボタン上に留まるとツールチップ「表示設定」がポップアップ上端（タイトルや `×` ボタン付近）に重なって表示されます。
- **ユーザーへの不利益**: ポップアップを開いた直後に見出しや閉じるボタンが隠れ、操作の邪魔になります。`display_settings.open` が `true` の間はツールチップを抑止すべきです。

#### 1-4. 閉じるボタンが貧弱で、すぐ下のインスペクタの閉じるボタンと紛らわしい
- **根拠箇所**: [display_settings.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/display_settings.rs#L436-L445), [folder.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder.rs#L2051-L2056)
- **何が起きているか**:
  - ポップアップの閉じるボタン（`display-settings-close`）は `icon_button` ではなく、素の文字 `"×"` に `p(px(5.))` だけ付けた要素（ホバー背景もアイコンSVGもなし）です。
  - しかもポップアップは `anchor.right() - width` で右端に揃うため、詳細ペイン（`268px`）が開いているときは、すぐ下に位置する詳細ペイン側の `close-pane`（`icons/x.svg`、`22px`）と縦方向に近接します。
- **ユーザーへの不利益**: クリック領域が狭い上にボタンらしく見えず、さらに「ポップアップを閉じる `×`」と「詳細ペインを閉じる `×`」が同じ右端に並んで誤操作を誘発します。そもそもポップアップは外側クリック・`Esc`・トリガー再クリックで閉じられるため、タイトルバーには小さすぎる素の文字 `×` ではなく統一された `icon_button`（またはヘッダー境界線付きの標準閉じるアイコン）を置くべきです。

#### 1-5. スイッチ横の「表示／非表示」ラベルと、OFF時トラックの視認性不足
- **根拠箇所**: [display_settings.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/display_settings.rs#L248-L275)
- **何が起きているか**:
  - スイッチのすぐ左に `10px` の `"表示"` / `"非表示"` テキストが添えられています。
  - OFF時のスイッチトラックが `bg(t.c("surface.field"))`＋`border.default` ですが、「隠しファイル」行は行背景自体も `surface.field` のため、トラック内部と行背景が同色になり、極めて薄い枠線だけで輪郭を保つ状態になっています。
- **ユーザーへの不利益**: トグルスイッチ自体がON/OFFを表すUI部品なのに文字が重複しており、情報密度だけが高まって窮屈です。またOFF状態のスイッチが背景に埋もれて「スイッチが有効に押せるもの」に見えにくくなっています。

#### 1-6. 出現アニメーションの欠如と用語不統一（「詳細ペイン」vs「インスペクタ」）
- **根拠箇所**: [display_settings.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/display_settings.rs#L367-L448), [folder.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder.rs#L2050)
- **何が起きているか**:
  - 右クリックメニューには `menu-reveal`（`110ms`）があるのに、表示設定ポップアップにはアニメーションが一切なく、突然出現・消滅します。
  - ポップアップやツールチップでは「詳細ペイン」、実際の右パネル見出しは「インスペクタ」と名称が統一されていません（オーナーとユーザーが同じ機能を別名で呼ぶ原因になります）。
  - またスコープ補足文が `px(9.)` と極小（通常の最小本文は `11px`）で読みづらい点も改善余地があります。

---

### 優先度2：設定モーダル（特に「Windows連携」ページ）

#### 2-1. 「Windows連携」ページのボタンが横幅いっぱいに縦積みされ、ボタンに見えない
- **根拠箇所**: [settings_panel.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/explorer/settings_panel.rs#L202-L231)
- **何が起きているか**:
  - 3つの連携項目（`フォルダをSkylisで開く` / `Win+EでSkylisを開く` / `Explorerに切り替えメニューを追加`）それぞれのボタン「有効にする」「元の設定に戻す」が、横並びコンテナ（`.flex().gap(...)`）に入れられず、縦積み `body` の直下へ直接 `.child(...)` されています。
  - その結果、コンテンツ幅いっぱい（約640px）に引き伸ばされた細長い箱が2段縦に並び、文字も左寄せ（親のデフォルト）で `py(10)`・`px(14)` のため、**ボタンではなく「テキスト入力欄」や「リスト項目の行」にしか見えません**。
- **ユーザーへの不利益**: オーナーの「なんか微妙」の最大の原因です。アクションを起こすボタンと判別できず、さらに3項目×2ボタンの縦積みに各24pxのマージンが重なるため、たった3機能しかないのに `880×620` のモーダル内で縦スクロールを強制されます（1画面に収まる情報量なのに見切れます）。

#### 2-2. 「有効」「無効」の状態表示が素のテキストで弱く、項目間のグルーピングがない
- **根拠箇所**: [settings_panel.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/explorer/settings_panel.rs#L194-L201)
- **何が起きているか**:
  - 状態表示が説明文の10px下に色装飾なしの素のテキスト（`"有効"` / `"無効"` / `"確認・変更中…"`）として置かれているだけです（一般設定行では `accent.fg` にしているのとも不統一）。
  - 項目を囲むカード枠（`surface.card`）や区切り線がなく、見出し・説明・状態・幅100%ボタン×2が縦に羅列されています。
- **ユーザーへの不利益**: 現在の登録状態を一目で把握できず、「有効」と「無効」の差が文字列を読み比べるまで分かりません。

#### 2-3. フッターのショートカットヒント「1〜8 切替」がカテゴリと噛み合っていない
- **根拠箇所**: [settings_panel.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/explorer/settings_panel.rs#L420-L433)
- **何が起きているか**:
  - フッターに `"↑↓ カテゴリ · 1〜8 切替 · Esc 閉じる"` とありますが、左ナビゲーションのカテゴリはWindowsで **5個**（外観と表示 / フォルダと一覧 / 描画とスクロール / 開発者向け / Windows連携）です。
  - 実装上 `1〜8` は設定項目（`rows[0..8]`）への直接ジャンプ兼トグルですが、設定画面内に「1〜8」の番号表記が一切ないため、ユーザーには「カテゴリが5つしかないのに1〜8とは何か？」と混乱を与えます。しかも5番目の「Windows連携」の操作にはショートカットも `Tab` キー移動も対応していません（[settings-navigation.md](file:///E:/github_projects/Skylis-File-Manager/docs/implementation/settings-navigation.md) でもTab移動未実装と明記）。

#### 2-4. 一般設定のホバー帯クリップ・OFFトグル低コントラスト・選択アイコン色不統一
- **根拠箇所**: [settings_panel.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/explorer/settings_panel.rs#L134-L162), [settings_panel.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/explorer/settings_panel.rs#L343-L404)
- **何が起きているか**:
  - トグル行（`rows[2..=7]`）は左右パディング・角丸なしのまま `hover(|s| s.bg("state.hover"))` されるため、ホバー背景が要素の左右端でスパッと切れる角張った帯になります。
  - OFF時のトグルスイッチが `surface.field` 上に `border.default` のみで、特にダークテーマ（`#1c1c1c` on `#262626`）では背景との差が小さく見えづらいです。
  - 左ナビで選択中のカテゴリは文字だけ `accent.fg` になり、横のSVGアイコンは `text.secondary` のまま残っています。

---

### 優先度3：アプリ全体のUI/UX上の改善ポイント（効果の高いものに限定）

#### 3-1. サイドバー「ピン留め」の `×` ボタンが常時32pxで大きく、ツールチップが「テーマ切替」に化けている
- **根拠箇所**: [folder.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder.rs#L1632-L1639), [folder.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder.rs#L2700-L2723)
- **何が起きているか**:
  - ピン留め行で `icon_button("unpin", "icons/x.svg", true, t)` を呼んでおり、1行の高さ約26pxに対して `32×32px` のボタンが常時右端に置かれているため、ピン留め行だけ縦に膨らんで「よく使う場所」と行リズムがズレます。
  - さらに `icon_button` のツールチップ分岐に `"unpin"` が定義されておらず、フォールバックの `_ => "テーマ切替"` が表示されてしまう明確なバグがあります。

#### 3-2. パンくずリストのサイレント省略と、ツールバー右端2ボタンの未分離
- **根拠箇所**: [folder.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder.rs#L1203-L1275)
- **何が起きているか**:
  - フォルダ階層が6段以上になると `i == 0 || i >= n - 3` で中間階層をフィルタしていますが、省略を示す `…` チップを出さないため、例えば `C: > skylis` のように直結して見え、中間フォルダの存在が隠れます。
  - ツールバー右端の「表示設定」と「設定」はスコープ（現在のフォルダ表示 vs アプリ全体設定）が異なるにもかかわらず、同じサイズの丸ボタンが境界なしに並んでいます。

#### 3-3. スクロールバーが専用トークンを使わず角丸がない
- **根拠箇所**: [scrollbar.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/scrollbar.rs#L65-L78), [theme_tokens.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/theme_tokens.rs#L71)
- **何が起きているか**:
  - `theme_tokens.rs` に半透明の `"scrollbar.thumb"` トークンが用意されているのに、`scrollbar.rs` は不透明な `t.c("text.muted")` を `corner_radii` なしの四角形として描画しています。ファイル一覧の繊細な雰囲気に対し、右端の棒だけ無骨に見えます。

#### 3-4. インスペクタ（詳細ペイン）のメタデータラベル幅と空白時の収まり
- **根拠箇所**: [folder.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder.rs#L2118-L2127), [folder.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder.rs#L2172-L2178)
- **何が起きているか**:
  - メタデータカード内のラベル幅が `w(px(50.))` 固定のため、「更新日時」（12px×4文字≒48px）が値と非常に接近します。`px(60.)` 程度に広げるだけで余白が整います。
  - 未選択時は上部から `mt(52)` の位置に左揃えの素テキストが出るだけで、空白ペインとしての収まりが弱いです。

#### 3-5. タブの固定幅（`180px`）による早期スクロール化
- **根拠箇所**: [explorer.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/explorer.rs#L283-L327)
- **何が起きているか**:
  - タブがすべて固定 `w(px(180.))` のため、ウィンドウ幅が狭い場合や5〜6タブ開いた時点で即座に横スクロール状態になり、他のタブが見えなくなります。通常ブラウザのように最大幅 `180px`〜最小幅 `96px` 程度で柔軟に縮小する方が視認性が高いです。

---

## 3. アニメーション方針の提案

### 基本原則（Skylisの軽量思想との両立）
Skylis の強みは「一瞬で開き、引っかからず反応すること」です（[AGENTS.md](file:///E:/github_projects/Skylis-File-Manager/AGENTS.md)）。したがってアニメーションは **装飾や演出のためではなく、「状態変化・出現元・操作の着地」を最短距離で伝えるため** にのみ使います。
- **時間の上限**: マイクロインタラクションは **`70〜150ms`**。ユーザーが次の一手を待つ時間はゼロにします。
- **OS設定の尊重**: [animation.rs](file:///E:/github_projects/Skylis-File-Manager/vendor/gpui-fast/src/elements/animation.rs#L78-L82) の `with_animation` / `with_spring` は両方とも `cx.reduce_motion()` を自動検知して最終状態へ即時スナップします。独自実装を増やさずこの標準APIに乗ることで、OSの「アニメーションを無効にする／視差効果を減らす」設定へ確実に追従できます。
- **GPUI実装上の描画コスト制約**: `gpui-fast` の `Div` (`Styled`) には CSS `transform: scale()` がなく、サイズ（`w`/`h`）を毎フレームアニメートすると親や兄弟要素のテキスト再レイアウトを誘発します。そのため **「オーバーレイ内の `opacity` + `mt/left` 微移動」「固定枠内のインジケータ位置（`left`）」「背景・枠線の色補間」** に限定し、一覧リストやギャラリーの再レイアウトを絶対に起こさない設計にします。

### 入れるべきアニメーション一覧表

| 箇所 | トリガー | 種類と変化量 | 時間・イージング / Spring | GPUI実装方法と描画コスト | 入れる理由 |
|---|---|---|---|---|---|
| **表示設定ポップアップ** | ツールバー右上ボタン押下 | `opacity: 0 → 1`<br>`mt: -6px → 0px`（上から軽く降りる） | **`120ms`**<br>`cubic_out` (`1 - (1-t)^3`) | `with_animation(("display-reveal", gen), ...)`。`deferred()` 層内のため背後のファイル一覧を一切再レイアウトしない。 | 既存の `context_menu`（`110ms`）と統一感が出て、右上ボタンから生えた知覚を最小コストで得られる。 |
| **設定モーダル** | `Ctrl+,` / 歯車ボタン | 背景: `opacity: 0 → 1`<br>本体: `opacity: 0 → 1` + `mt: 8px → 0px` | 背景 **`110ms`** `linear`<br>本体 **`140ms`** `cubic_out` | モーダルはウィンドウ全面オーバーレイのため、`with_animation` で `opacity` と外側マージンのみ動かす。負荷極小。 | 唐突な画面ジャック感をなくし、設定完了後に元の画面へ戻る空間文脈を保つ。 |
| **設定カテゴリ切替** | 左ナビクリック / `↑↓` | アクティブインジケータ（背景チップ／左バー）の `top` 移動＋右ペイン軽フェード | インジケータ: **`with_spring`**（`130ms` 収束・オーバーシュートなし）<br>右ペイン: **`80ms`** `opacity: 0.7 → 1` | 左ナビの絶対配置インジケータを `with_spring` で `top` 補間。テキストは再描画するだけなので極めて軽量。 | カテゴリ移動の方向が直感的に分かり、「パチパチ」と画面が跳ねる感覚を抑える。 |
| **トグルスイッチ** | クリック / Space | ノブの `left` 移動（`12px`）＋トラック色補間 | **`with_spring`**（臨界減衰・約 `130ms` 相当、リターゲットで速度継承） | `DisplaySettings` / `Settings` のノブ `div` に `with_spring` を適用。30×18pxの単一要素のみでコスト無視可。 | 連打しても破綻せず、押し心地（タクタイル感）が劇的に良くなる。 |
| **セグメントコントロール** | 詳細/ギャラリー・並べ替え等 | 選択ピル（`surface.chip-selected`）の `left` / `w` 移動、または選択色の `bg` 補間 | **`with_spring`**（約 `120ms`）または `bg` 補間 **`100ms`** | 均等幅ボタン（`pair()`）ならインデックス比から `left` が固定計算でき、レイアウト再計算不要で滑る。 | 選択肢間の移動が線として繋がり、いまどれを選んだか迷わなくなる。 |
| **行・ボタンのホバー／押下** | マウスホバー / 押下 | 背景色・枠線色のクロスフェード（移動・拡縮なし） | **`75ms`** `ease_out` | `state.hover` の軽量色補間（必要箇所のみ）。※リスト大量行には `with_animation` を個別に貼らず既存の即時ホバーでも十分軽快。 | クリックできる要素の手触りを向上させつつ、高速カーソル移動に遅れない。 |
| **詳細ペイン（インスペクタ）開閉** | 表示設定 / `×` ボタン | パネル出現時: 中身 `opacity: 0 → 1` ＋ `ml: 10px → 0px` | **`130ms`** `cubic_out` | **注意**: `w(0→268px)` で幅をアニメートするとギャラリー列数計算とテキスト折り返しが毎フレーム走るため禁止。幅は即確保し中身だけ軽くフェードインさせる。 | 右側が突然増える違和感を和らげつつ、ファイル一覧側のレイアウト負荷をゼロに保つ。 |

### 入れるべきでない箇所とその理由

| 箇所 | 避けるべきアニメーション | 理由 |
|---|---|---|
| **フォルダ移動・進む／戻る** | 一覧のフェードイン／スライド切替 | 1日に何百回も行われる中核操作。`100ms` でも毎回の遅延は「ファイルマネージャーがもっさりする」最大の要因になる。即時描画が正義。 |
| **リスト行・ギャラリーカードの出現** | スクロール時の時間差（スタガー）フェード | `uniform_list` の高速スクロール（[scroll_motion.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/scroll_motion.rs)）において文字が後から浮かび上がると視認性が激減する。 |
| **タブ切替の中身** | タブ間の横スライド | フォルダ比較のために複数タブを素早く行き来する際、残像が邪魔になる（タブ自体のアクティブピルを軽く動かす程度に留める）。 |
| **ホバー時のカード拡大（Scale）** | ギャラリーサムネイルやボタンの `1.0 → 1.04` 拡大 | 境界の滲みや隣接要素との隙間崩れを起こしやすく、軽量ユーティリティの品格から外れる。 |
| **過剰なバネ（跳ね返り）** | ポップアップやスイッチの大きなオーバーシュート | 毎回ブルッと揺れると「騒がしいUI」になり、道具としての落ち着きが失われる。臨界減衰（跳ねない、または極小）に留める。 |

---

## 4. 表示設定ポップアップ改善案の方向性比較

オーナーが触って判断できるよう、[mock.html](file:///E:/github_projects/Skylis-File-Manager/research/ui-ux-review/mock.html) では **「現行 (Before)」「案A：構造整理・トークン統一案（推奨）」「案B：コンパクト密度・インライン範囲案」** の3つを切り替え可能にしています。

| 観点 | 現行 (Before) | 案A：構造整理・トークン統一案（**推奨**） | 案B：コンパクト密度・インライン範囲案 |
|---|---|---|---|
| **適用範囲の表現** | 見出し帯の背景色（白 / 青帯 / グレー帯）で無理に塗り分け、行側にも同色が漏れている | [display-settings.md](file:///E:/github_projects/Skylis-File-Manager/docs/implementation/display-settings.md) の3見出しを維持しつつ、背景ベタ塗りを廃止。細い区切り線＋淡いスコープバッジ（ピル）とアイコンで明瞭に区別 | 見出し行を廃止し、各設定行ラベルの右横に控えめなスコープピル（`このタブ` / `この窓` / `全窓`）を配置 |
| **縦サイズと密度** | `489px`（かなり縦長、9px極小ヒントあり） | 約 `448px`（余白と文字サイズを11〜12pxに整理、余裕あり） | 約 `340px`（見出し3行分を削減し、小さな画面でもスクロール不要） |
| **選択ピル** | `accent.soft` + 1px青枠（平面的） | `surface.chip-selected`＋微細シャドウ（設定画面と統一された立体セグメント＋Springスライド） | 同左に加え、並べ替えキーと昇降順を1つのまとまり（2段一体カード）として視覚統合 |
| **トグル行** | 「表示／非表示」文字付き、OFF枠が背景と同化 | 冗長な文字を撤去し、高コントラストなOFFトラック＋Springノブに変更 | 同左に加え、詳細ペインの名称を「インスペクタ」に統一 |
| **長所・短所** | 現状把握用 | **長所**: 既存設計（3スコープの意図）を100%尊重しつつ、見た目の粗さと誤認を完全に解消できる。<br>**短所**: 縦幅は案Bより約100px長い。 | **長所**: 最もコンパクトで日常操作が速い。小ウィンドウでも切れない。<br>**短所**: 「どの設定が全ウィンドウに効くか」の教育効果は案Aよりやや弱まる。 |

**推奨**: まずは既存設計意図（[display-settings.md](file:///E:/github_projects/Skylis-File-Manager/docs/implementation/display-settings.md)）と設定画面とのトークン一貫性を壊さずに完成度を極限まで引き上げられる **案A** を第一推奨とします。もしテスト時に「ポップアップがまだ縦に長い」と感じる場合は、案Bの「見出し廃止＋インライン範囲ピル」または「並べ替えキー＋昇降順のカード統合」を部分的に取り入れるのが最もバランスが良いです。

---

## 案B2での変更点

案B（コンパクト統合）の方向性と承認済みアニメーション仕様（時間・イージング・移動量）を維持したまま、レビュー指摘8点およびモック上の不具合を解消した **「案B2」** を [mock.html](file:///E:/github_projects/Skylis-File-Manager/research/ui-ux-review/mock.html) に追加し、案Bと並べて比較できるようにしました（既定表示は案B2）。

### 1. 各指摘への対応内容

1. **適用範囲の表記・見た目・配置の統一**
   - 表記を自然な日本語の **「このタブ」「このウィンドウ」「すべてのウィンドウ」** に改めました（「この窓」「全窓」を廃止）。
   - 枠線・背景色付きピル（特に青枠ピル）を廃止し、押せないメタ情報だと一目で分かる控えめな補助テキスト（`10.5px`・`text.muted`）に変更しました。
   - 全行の項目ラベル幅を `82px` 固定＋余白 `8px` に揃えることで、全5行（表示形式・並べ替え・詳細ペイン・隠しファイル・表示密度）で適用範囲テキストの左端が同一の垂直軸に揃うようにしました。
   - ホバーおよびフォーカス時に、ポップアップ下端の1行ヒント欄とネイティブツールチップ（`title`）で適用範囲の補足説明を表示します。

2. **箱の入れ子構造の解消**
   - 案Bの「ポップアップ外枠 ＞ カード枠（`surface.field`＋枠線） ＞ セグメント受け皿枠」という3重枠を廃止し、カード枠と背景色を撤去しました。
   - 3つの適用範囲グループ（このタブ／このウィンドウ／すべてのウィンドウ）は余白と **1本の細い区切り線（`1px`・`border.subtle`）** のみで分離し、視認負荷を下げました。

3. **ラベルと選択値の強弱（タイポグラフィ階層）**
   - 項目ラベル（表示形式・並べ替え・詳細ペイン・隠しファイル・表示密度）は通常の太さ（`400`）かつ抑えた色（`text.secondary`）に落としました。
   - 選択中のセグメント値を太字（`600`）・高コントラスト（`text.primary`）にすることで、現在の設定値が最も目立つ階層にしました。

4. **選択中セグメントのコントラスト改善（ライト／ダーク両対応）**
   - カード背景を無くしてポップアップ面を `surface.raised` に統一した上で、セグメント受け皿を沈んだ色面（ライト `#eef2f7` / ダーク `#1b1b1b`＋枠線）、選択中サムを浮き上がる色面（ライト `#ffffff` / ダーク `#3d3d3d`＋明瞭な枠線と影）へ引き離しました。
   - これにより、色覚に頼らず **明度差・枠線・影・太字** の組み合わせでライト／ダーク双方で選択状態を一目で判別できます。

5. **並べ替えの1段統合と昇降反転操作**
   - 「名前／更新日／種類／サイズ」と「昇順／降順」の2段構成を **1段のセグメント** に統合しました。
   - 選択中のキーにのみ方向矢印（`↑` / `↓`）を表示し、未選択キーを押すとそのキーの昇順（`↑`）へ切り替え、選択中のキーをもう一度押すと昇順／降順を反転します。
   - キーボードでも `←`/`→`/`Home`/`End` でキー選択、`Enter`/`Space`（フォーカス中または行選択中）で昇降反転できるようにし、矢印表示・ツールチップ・下部ヒント欄で再押し反転の操作性を明示しました。

6. **ポップアップ側の閉じる「×」ボタン撤去**
   - 右下に隣接する詳細ペインの「×」との誤操作を防ぐため、案B2ではポップアップヘッダー右上の「×」ボタンを廃止し、`Esc`・外側クリック・ツールバートリガー再クリックで閉じる構成に統一しました（案A・案Bは比較用に維持）。

7. **用語の統一（「詳細ペイン」）**
   - 案B2のポップアップ内スイッチ名称を実物の設計仕様と同じ **「詳細ペイン」** に統一しました（「インスペクタ」を廃止）。

8. **表示密度の比較スイッチ（モック操作バー）**
   - モック上部操作バーに **「B2表示密度：ポップアップに残す / 設定画面のみ」** の切替スイッチを追加しました（既定は「残す」）。ポップアップの高さ差（`398px` vs `325px`）と使い勝手を並べて比較できます。

### 2. モック自体の不具合修正と追加改善
- **ダークテーマで文字が黒（`rgb(15, 23, 42)`）のまま残る不具合**: `.app-window` にかかっていた `transition: background, color` によりテーマ切替直後に文字色継承が遅れて暗い背景に暗い文字になっていた問題を解消し、各コンテナ／見出し／ファイル名に明示的なトークン指定を追加しました。
- **メイン画面の実物再現**: 既定状態をギャラリー表示（横6列、縦横混在の枠なしサムネイル＋`DSCF3050.JPG` 等）、タブ `JPG` 1つ、パンくず `E: > camera > 2026-10-24 > JPG`、実物準拠のサイドバー（ホーム／よく使う場所5件／ストレージ4ドライブ、E: は空き `71 GB / 1.9 TB` でオレンジ使用量バー）、ステータスバー（`128 項目` / `1 項目を選択 · 11 MB`）、見出し文字なしの詳細ペイン（プレビュー・ファイル名・サイズ/更新日時/パス表・`パスをコピー`ボタン）へ合わせました。
- **キーボード操作とフォーカス表示の補完**: 案B2ポップアップ内で `↑`/`↓` による行移動、`←`/`→`・`Enter`/`Space` による操作、`Tab` 移動、`Esc` で閉じてトリガーへフォーカス復帰する挙動と `:focus-visible` フォーカスリングを追加しました。

### 3. Rust実装時に触ることになる主なファイルと関数

| 変更内容 | 対象ファイル | 主な関数・構造体・箇所 |
|---|---|---|
| 表示設定ポップアップのレイアウト刷新（区切り線・軸揃え・×撤去・「詳細ペイン」表記・1段並べ替え・表示密度有無） | [display_settings.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/display_settings.rs) | [`FolderView::render_display_settings`](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/display_settings.rs#L401), [`FolderView::display_option`](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/display_settings.rs#L277), [`FolderView::display_switch`](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/display_settings.rs#L319) |
| 1段並べ替えのクリック時反転・キーボード操作（行配列 `row == 2` 削除と `row == 1` での `Enter`/`Space` 反転） | [display_settings.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/display_settings.rs) | [`FolderView::display_rows`](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/display_settings.rs#L166), [`FolderView::display_key`](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/display_settings.rs#L176), [`FolderView::choose_display_sort`](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/display_settings.rs#L271) |
| ポップアップ展開中のトリガーツールチップ抑止・詳細ペイン見出し整理 | [folder.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder.rs) | [`FolderView::render_inspector`](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder.rs#L2004), [`icon_button`](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder.rs#L2700), [`FolderView::render_display_trigger`](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/display_settings.rs#L106) |
| 選択中セグメントのダーク時コントラスト強化用トークン調整（必要時） | [theme_tokens.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/theme_tokens.rs) | [`color`](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/theme_tokens.rs#L4)（`surface.chip-selected`・`surface.field` 周辺） |
| （比較結果で「設定画面のみ」を選ぶ場合）ポップアップ側密度行の削除と回帰テスト更新 | [display_settings.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/display_settings.rs), [tests.rs](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/folder/tests.rs) | `set_display_density` 呼び出し箇所、`display_popup_*` テスト群（設定モーダル側は [`Explorer::render_settings`](file:///E:/github_projects/Skylis-File-Manager/apps/explorer/src/explorer/settings_panel.rs#L10) に既存維持） |
