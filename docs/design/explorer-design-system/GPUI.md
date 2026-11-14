# GPUI へ移すためのメモ

> **位置づけ:** このデザインシステムは HTML のリファレンスです。**Rust / GPUI 側は今回変更しておらず、ネイティブ実装は済んでいません。**
> 以下は移し方の方針です。API 名は「現行コードが使っているもの」と「未確認のもの」を分けて書きます。現行コードのビルド成否も今回は確かめていません。

## 1. 前提（現行コードから読める事実）

- 依存窓口は `Cargo.toml` の `gpui = { package = "gpui-pre", version = "=0.3.12" }` の 1 か所。GPUI Fast を最初の基盤にし、のちに GPUI 系の別エディションへ替える前提。
- `apps/explorer/src/theme.rs` に平たい `Theme` 構造体（12 色）があり、`gpui::{Rgba, WindowAppearance, rgb, rgba}` を使って `Theme::for_appearance(window.appearance())` で明暗を選んでいる。外観の変化は `cx.observe_window_appearance(...)` で再描画している。
- 色は旧 PoC の値で、**採用デザイン（このトークン）とは別物**。寸法も一部が違う（下の表）。

| 項目 | 現行 Rust | トークン |
|---|---|---|
| 行高 | 28（`ROW_HEIGHT`） | `size.row` 28 / コンパクト 24 |
| 一覧ヘッダー | 30 | `size.list-header` 28 |
| 情報バー | 26・11.5px | `size.statusbar` 28・11px |
| アイコンボタン | 30 | `size.icon-button` 32 |
| 列幅 更新日時/種類/サイズ | 150 / 120 / 90 | 144 / 136 / 92 |
| サイドバー | 200 | 200（ウィンドウ幅 1140 以下で 196） |
| タブバー・インスペクタ・パネル間隔 6 | なし | 32 / 268 / 6 |

## 2. 意味トークンの対応

tokens.json の名前をそのまま Rust の入れ子構造へ写すのが最も迷いません（`.` → 構造体の階層、`-` → `_`）。

```rust
// 擬似コード（型名・フィールド名は提案。GPUI の色型は現行コードと同じ Rgba を想定）
pub struct Theme {
    pub surface: Surface,     // canvas, content, chrome, statusbar, field, field_focus, card, chip_selected, raised, dialog, backdrop
    pub text: Text,           // primary, secondary, muted, disabled, emphasis, on_accent
    pub icon: Icon,           // default
    pub accent: Accent,       // fg, soft
    pub focus: Focus,         // ring, halo
    pub state: State,         // hover, pressed
    pub nav: Nav,             // current { bg, fg, caption, meter_track, meter_fill }
    pub selection: Selection, // bg, border, fg, fg_secondary, inactive { bg, border }
    pub tab: Tab, pub button: Button, pub menu: Menu, pub badge: Badge,
    pub border: Border,       // subtle, default, content, card, statusbar, dialog
    pub r#match: Match, pub meter: Meter, pub scrollbar: Scrollbar,
    pub shadow_popover: ShadowSpec, // 1 種類だけ
}
pub struct Metrics { /* size.* / space.* / radius.* / line.* を px の定数で。テーマに依存しない */ }
```

- **色の書き方:** CSS の `rgba(37, 99, 235, 0.055)` は 8 桁 16 進 `0x2563eb0e`（α = round(0.055×255) = 14）。現行コードの `rgba(0x…)` と同じ形にそろう。手で写すとずれるため、**tokens.json から Rust の定数ファイルを生成するスクリプトを足す**ことを推奨（未作成。`build-tokens.cjs` に出力先を 1 つ加える形が最小）。
- **`light` / `dark` の 2 つだけ**を持ち、OS ごとの配色は持たない。`placeholderIcon`（`obj.*`）は**移さない**（本番は OS アイコン）。
- `currentColor`（`match.fg` のライト）は「周囲の文字色をそのまま使う」の意味。Rust では `Option<Rgba>` の `None` などで表す。
- `derived` の 2 値（ボタンのホバー）は不透明色なので、そのまま使える。

### 現行 `Theme` フィールドからの置き換え

| 現行 | 置き換え先 | 備考 |
|---|---|---|
| `bg` | `surface.content` | 一覧面 |
| `sidebar` | `surface.chrome` | ツールバー・インスペクタも同じ |
| `border` | `border.default` | 内側の区切りは `border.subtle` |
| `text` / `text_muted` / `text_faint` | `text.primary` / `text.muted` / `text.disabled` | `text.secondary` を新設 |
| `accent` | `accent.fg` | 塗りの青は `nav.current.bg`・`button.primary.bg`・`menu.highlight.bg` に分かれる |
| `folder` | 廃止 | フォルダー色は OS アイコンに任せる |
| `hover` | `state.hover` | 押下は `state.pressed` |
| `selection` / `selection_inactive` | `selection.bg`（＋`border`・`fg`） / `selection.inactive.bg` | |
| `danger` | `danger.fg` / `danger.solid` | どちらも**提案**（未承認） |

## 3. テーマの解決

```
ユーザー設定 (Light | Dark | System) ─┐
                                      ├─> ThemeMode (Light | Dark) ─> &'static Theme
OS の外観（System のときだけ参照）───┘
密度設定 (Standard | Compact) ──────────> Metrics.row = 28 | 24（色とは独立）
```

- System のときは現行と同じく外観を見て決める。`VibrantDark` / `VibrantLight` も通常の暗/明として扱い、**半透明・背景マテリアルは使わない**。
- テーマ・密度はウィンドウ共通、タブごとには持たない（モック準拠）。
- 切替時にアニメーションしない。Theme は不変値として持ち、切替は参照の差し替え＋再描画にする。
- ユーザー設定の保存先・OS 外観の取得方法（エディション差）は未確認。現行の `window.appearance()` / `observe_window_appearance` が移行先エディションにもあるかは移す時に確認する。

## 4. 部品の分け方

部品は「`&Theme` と `&Metrics` と状態を受け取って要素を返す」純粋な描画関数（またはそれに近い小さな View）にし、値を直接持たせない。状態は呼ぶ側が持つ。

| 部品 | 主なトークン | 状態の入力 |
|---|---|---|
| `TabBar` / `Tab` / `TabAdd` | `tab.*`, `size.tab*`, `radius.tab`, `radius.small` | active, hovered, focused, close_hovered |
| `Toolbar` | `surface.chrome`, `border.default`, `size.toolbar`, `radius.island` | — |
| `IconButton` / `ButtonGroup` | `icon.default`, `state.*`, `accent.soft`, `size.icon-button(-sm)` | disabled, pressed, toggled, hovered, focused |
| `Button` | `button.*`, `size.button`, `radius.pill` | primary, disabled(提案), hovered, focused |
| `AddressBar` / `Crumb` | `surface.field`, `surface.chip-selected`, `text.secondary/emphasis`, `size.field`, `size.crumb` | current, hovered, focused, collapsed |
| `SearchField` | `surface.field(-focus)`, `focus.ring/halo`, `text.muted` | focused, has_text |
| `Segmented` | `surface.field`, `surface.chip-selected`, `accent.fg` | selected index |
| `Sidebar` / `PlaceItem` / `DriveCard` / `Meter` | `surface.chrome`, `nav.current.*`, `meter.*` | current, hovered, warn |
| `ListHeader` / `HeaderCell` | `surface.chrome`, `border.subtle`, `text.muted`, `accent.fg` | sort key / 向き |
| `FileRow` / `Tile` | `selection.*`, `state.hover`, `text.*`, `match.*`, `size.row`, `radius.row/tile` | selected, list_focused, hovered, keyboard_current, hidden, match ranges |
| `Inspector`（Preview / PropsCard / Badge / CodePreview） | `surface.chrome/card`, `border.card`, `badge.*` | 選択内容 |
| `StatusBar` / `StatusButton` | `surface.statusbar`, `border.statusbar`, `text.secondary`, `accent.fg` | count, selection, notice |
| `Menu` / `MenuItem` | `surface.raised`, `menu.*`, `shadow.popover` | current, checked, disabled |
| `Dialog` | `surface.dialog`, `border.dialog`, `surface.backdrop`, `shadow.popover` | — |

## 5. 状態の優先順位（行・タイル）

```rust
// 擬似コード。塗りは上位 1 つだけ。フォーカスリングは別層で最後に重ねる。
fn row_fill(s: &RowState, t: &Theme) -> Fill {
    if s.selected && s.list_focused { Fill { bg: t.selection.bg, border: t.selection.border, name: t.selection.fg, name_weight: 600, meta: t.selection.fg_secondary } }
    else if s.selected            { Fill { bg: t.selection.inactive.bg, border: t.selection.inactive.border, ..normal(t) } }
    else if s.hovered             { Fill { bg: t.state.hover, ..normal(t) } }
    else                          { normal(t) }
}
// ring = s.keyboard_current && s.list_focused && keyboard_modality → 2px focus.ring
// hidden → 名前を text.muted、アイコン不透明度 0.48（塗りと独立）
// match ranges → match.bg（文字色は match.fg。None なら周囲の色）
```

ボタン類は「無効 > 押下中 > トグルオン > ホバー > 通常」。トグルオンをホバーより上に置くのは**提案**です（モックは CSS の詳細度でホバーが勝つ）。キーボード操作時だけリングを出す判定（キーボード操作中かどうか）は自前で持つ必要があるかもしれません（未確認）。

## 6. 避ける境界（デザイン層に持ち込まないもの）

- **仮想リストの実装に依存しない。** デザインが決めるのは「全行同じ高さ `size.row`」「行間に線なし」「ヘッダーは上に固定」まで。現行は `uniform_list` を使っているが、どの方式でも一定高さの行だけで成り立つようにし、可変高の行・行ごとの影・行をまたぐ装飾は作らない。
- **OS アイコンの非同期取得に依存しない。** デザインが決めるのは「サイズの枠（18 / 16 / 52 / 68px）」「色加工しない」「取得前は枠だけ確保してレイアウトを動かさない」まで。取得方法・キャッシュ・スレッドは実装側の判断。取得前の仮表示は**空の枠**を推奨し、仮素材 SVG は出さない（提案）。
- **エディション固有の機能に依存しない。** 使うのは単色の塗り・1px 線・角丸・文字・単色 SVG・影 1 種類だけ。背景ぼかし・ウィンドウ背景マテリアル・グラデーション・ブレンドは使わない（デザイン上も禁止）。影を描けないエディションでは、メニュー/ダイアログは `border.default` だけで成立させてよい。
- **フォントの指定方法は未確認。** 「OS の UI フォント（Segoe UI Variable / Yu Gothic UI、macOS のシステムフォント / Hiragino Sans）」という意図だけを守り、名前の指定・フォールバックの API は移す時に確認する。
- **キー割当は OS 慣習に合わせる。** Windows は Ctrl / Alt、macOS は ⌘ / ⌥。`Ctrl+T/W/Tab` などはモックではブラウザーに取られる場合があり、本番で再検証が必要。

## 7. 移す順番（提案）

1. tokens.json → Rust 定数の生成を追加（色は `Rgba`、寸法は px）。生成物はコミットし、`--check` で手編集を防ぐ。
2. `Theme` を意味トークンの構造へ置き換え、`Metrics` を追加。既存の描画は置き換え表どおりに参照先だけ変える。
3. レイアウト骨格（タブバー 32・ツールバー 44・6px 間隔・情報バー 28 全幅・サイドバー 200・インスペクタ 268）。
4. 一覧（行・ヘッダー・選択・フォーカス）→ ツールバー部品 → サイドバー → インスペクタ → メニュー/ダイアログ の順。
5. 各段階で index.html の状態見本・モックのスクリーンショットと並べて目視確認。色の数値はトークンが正、見た目の判断はモック（concept-1*.html）が正。

## 8. 未確認（移す時に確かめる）

- 移行先エディションで `Rgba` / `rgba()` / `WindowAppearance` / `observe_window_appearance` / `uniform_list` が同じ形で使えるか。
- 角丸付きの 1px 枠・2px のフォーカスリング（外側に 1px 空ける）・影 1 種類の描画方法と、その描画負荷。
- 文字の省略記号・等幅数字（tabular-nums 相当）の指定方法。
- OS アイコン取得の方法と速度、高 DPI での取得サイズ。
- ハイコントラストモード・文字拡大・スクリーンリーダーへの対応方法。
