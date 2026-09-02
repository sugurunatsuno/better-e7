# better-e7

Androidゲームの映像取得 / 認識 / 操作 / タスク実行を、macOS / Windows / Linuxで共通化する自動化基盤です。

現在は実装前の土台を作る段階です。Rustとeguiを使い、Androidとの接続はADBと同梱するscrcpy-serverを使う方針です。

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
| 画像認識 | OpenCV |
| 推論 | ONNX Runtime |

FFmpeg / OpenCV / ONNX Runtimeは必要になるまでworkspaceへ追加しません。最初に境界を固定し、依存の導入と機能実装を段階的に進めます。

## 現在の構成

```text
crates/
  better-e7-adb/   ADB実行と端末情報の解析
  better-e7-core/  OSやGUIに依存しない型とtrait
  better-e7-config/ TOML設定
  better-e7-runtime/ Workerと状態管理
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

## ライセンス

現時点ではライセンスを付与していません。公開や外部配布を始める前に決定します。
