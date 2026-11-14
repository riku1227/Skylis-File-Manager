# GPUI Fast 基盤への切り替え

2026-11-15。採用UIをWindows上で再現する段階から、本人希望のGPUI Fastを使う。

## 固定した依存

- `gpui = { package = "gpui-fast", version = "=0.1.15" }`
- `gpui_platform = { package = "gpui-fast-platform", version = "=0.1.15" }`
- core/platformの入口はルート`Cargo.toml`に集約。アプリ側は引き続き`gpui` / `gpui_platform`というaliasだけを使う。
- `Cargo.lock`でWindows/macOS等の同系列バックエンドを含め解決済み。Fast内部の共通ユーティリティに`gpui-pre-* 0.3.12`が残るのは配布物の構成どおりで、旧GPUI coreが混在しているわけではない。

一次資料は[公式リポジトリ](https://github.com/longbridge/gpui-fast)、[0.1.15 core配布](https://crates.io/crates/gpui-fast/0.1.15)、[0.1.15 platform配布](https://crates.io/crates/gpui-fast-platform/0.1.15)。`cargo info`でも同じ版・リポジトリ・Apache-2.0を確認した。coreの配布日付は2026-11-13。

## 境界

- WebView、ネイティブビュー合成、Fast専用APIには依存しない。通常のGPUI要素・Entity・Theme・仮想リストで描画する。
- フォント・ウィンドウ・描画backendは`gpui_platform::application()`に任せる。Windows/macOS用にLinux backend featureを追加しない。
- 外部状態が変化した際は`cx.notify()`を行う。保持描画で変更を検知できる形を保つ。
- UIの意味トークンとファイル列挙等のcoreを描画基盤から分離しておく。将来のGPUI系列変更では依存差し替えとコンパイル/描画テストを必須とし、互換を無条件には保証しない。
- OSアイコン用の`image::Frame`はGPUIの`RenderImage`へ渡す。GPUIのこのAPIはBGRAバッファを受け取るため、Win32 DIBのチャネル順をそのまま維持する（通常のRGBA画像生成との混同に注意）。

## 検証と限界

- Rust 1.100.0 / Windows MSVC環境で依存解決し、`cargo tree -p explorer --depth 1`のcore/platformが0.1.15であることを確認。
- 移行直後の既存PoCに対する`cargo check -p explorer`が成功（1分38秒）。基盤変更に伴うRustソースのAPI修正は不要だった。新UIのビルド・実画面確認は親タスクで引き続き行う。
- 公式はexperimentalと明記している。公式の性能比較は特定ワークロードでの測定であり、このExplorerのWindowsでの優位を示すものではない。大量ディレクトリのスクロール・フレーム時間は別途計測する。
- macOSの実機ビルド・描画は今回のWindows環境では未検証。
