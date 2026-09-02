# better-e7

Androidゲームの映像取得 / 認識 / 操作 / タスク実行を、macOS / Windows / Linuxで共通化する自動化基盤です。

現在は映像preview / ADB入力 / 複数template認識 / 汎用AutomationProfileから入力queueまでの実行経路 / profile再読み込み / rule editor / dry-run / 保存Frame列のオフライン実行 / JSONL実行履歴とviewer / ゲーム拡張APIまで実装しています。Rustとeguiを使い、Androidとの接続はADBと固定versionのscrcpy-serverを使います。

## 目標

- Androidの画面をPCのウィンドウ経由ではなくscrcpyの映像ストリームから取得する
- 認識ロジックと端末操作をOSや通信方式から分離する
- ゲーム固有処理を小さなプラグインとして追加できるようにする
- GUIで映像 / 認識結果 / タスク状態 / ログを確認できるようにする
- 録画や保存画像でも認識処理を再現できるようにする

## 採用技術

| 用途 | 採用候補 |
|---|---|
| 言語 | Rust 2024 |
| GUI | egui / eframe |
| 非同期処理 | Tokio |
| Android接続 | ADB / scrcpy-server |
| 映像デコード | FFmpeg |
| 画像認識 | pure Rust template matcher / 将来OpenCV |
| 推論 | ONNX Runtime |

映像decodeは外部FFmpeg processを交換可能な境界の内側で使います。最初のtemplate matcherはpure Rustで実装し、OpenCV / ONNX Runtimeは必要になるまでworkspaceへ追加しません。

## 現在の構成

```text
crates/
  better-e7-adb/   ADB実行 / 端末情報 / tap / swipe / key
  better-e7-android/ scrcpy-serverと映像socket
  better-e7-automation/ 汎用profile / Rule engine
  better-e7-core/  OSやGUIに依存しない型とtrait
  better-e7-config/ TOML設定
  better-e7-game-api/ ゲーム登録 / 状態 / Trigger / Task / Dispatcher
  better-e7-runtime/ Worker / 状態管理 / 最新Frame / 入力queue
  better-e7-video/ H.264 parser / FFmpeg process decoder
  better-e7-vision/ 保存画像source / template matcher
  better-e7-app/   eguiデスクトップアプリ
docs/
  decisions/       技術判断の記録
```

## 開発

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo run -p better-e7-app
```

必要な環境と実装順は [docs/development.md](docs/development.md) を参照してください。要件は [docs/requirements.md](docs/requirements.md)、設計は [docs/architecture.md](docs/architecture.md) にあります。

設定項目は [docs/configuration.md](docs/configuration.md) にあります。

汎用profileの書き方は [docs/automation-profiles.md](docs/automation-profiles.md) にあります。

通常CIはUbuntuだけで実行します。Windows / macOSの確認は必要な場合だけ手動workflowを使います。

## ライセンス

現時点ではライセンスを付与していません。公開や外部配布を始める前に決定します。
