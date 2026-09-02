# 開発環境

## 必要なもの

- Rust 1.88以降
- rustfmt / Clippy
- Android Platform Tools
- 対応バージョンのscrcpy-server

FFmpeg / OpenCV / ONNX Runtimeは、それぞれを使うcrateへ着手するときに追加します。CoreとGUI骨格の開発には不要です。

## 初回確認

```bash
rustup show
cargo fmt --all --check
cargo test -p better-e7-core
cargo run -p better-e7-app
```

ADBがPATHにない場合は`better-e7.toml`の`adb_path`を変更してください。書式は [configuration.md](configuration.md) にあります。

## 実装の進め方

縦切りで動く範囲を増やします。最初の縦切りは次の流れです。

```mermaid
flowchart LR
    A[ADB端末選択] --> B[映像受信]
    B --> C[H.264 decode]
    C --> D[egui表示]
    D --> E[template検出]
    E --> F[ADB tap]
```

各段階では保存データを使う代替入力を用意し、Android端末がなくてもテストできるようにします。

## 外部依存を追加する基準

- Rustだけで保守できる実装が現実的か確認する
- 対象3OSの導入方法とCIでの扱いを確認する
- ライセンスと配布条件を確認する
- 外部型をcrateの公開APIへ漏らさない
- 採用理由と更新方針をADRへ追記する

## ブランチと変更

通常の作業はdevelopで進めます。必要に応じてdevelopから小さな機能branchを作り、完了後にdevelopへ戻します。リリース可能な状態になったらdevelopからmainへのPRを作ります。コミットにはコードと対応するテスト / 文書を含めます。

## ローカル制約

Android実機を使う結合テストは通常のCIから分けます。通常CIは保存フレーム / 模擬VideoSource / 模擬InputControllerを使い、決定的に実行できるテストだけを対象にします。

通常のpush / PRではUbuntuだけを使います。Windows / macOSのGitHub-hosted runnerは必要なときだけ`Cross-platform check`を手動実行します。特にmacOS runnerは通常CIへ追加しません。
